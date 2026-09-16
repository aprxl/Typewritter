# Math mode — design

Structural math editing, in the spirit of corca.app: an AST-driven input
system where every structure is a node with typed-into slots, autocompletion
is the default typing experience, and editing never evaluates. This
document records the design decisions and the build plan; SPEC.md §6, §9,
§11 and §14 are the requirements it answers to.

## 1. What the reference does (corca.app)

From the screenshots and session notes:

- **Everything is a node with slots.** A fraction is a bar with two empty
  placeholder boxes; an integral is bounds + integrand + differential slots.
  Empty slots are drawn as small outlined boxes, are permanently legal, and
  are typed into directly.
- **Tab / Shift-Tab walk the slots.** The caret jumps placeholder to
  placeholder in document order; escaping a slot pops one level.
- **Autocomplete is the typing experience, not a feature.** Every word you
  type opens a completion card: variables you already used, constants,
  functions, named operators (`der` → derivative, `frtr` → Fourier
  transform), then a symbol grid. Numbered shortcuts (`Ctrl+1..9`) pick a
  row; Enter takes the top hit.
- **Identifiers are typed entities.** A variable renders as a colored chip,
  a function as `f(□)` with its argument slot; constants, frequencies, etc.
  are distinct kinds. The palette offers "Variable", "Constant · New",
  "Function" readings of the same typed word.
- **It is not a calculator.** Nothing computes; the tree exists for layout,
  search, and speed.

What we deliberately do *not* copy: corca's persistent definition store
(variables with global identity across a workspace). Our identifiers are
per-document display entities — SPEC §6.4 forbids symbol bindings.

## 2. What the codebase already gives us

- **Pure model / measure-closure layout / component draw.** `document::layout`
  is pure and measured through `&dyn Fn(&str, &TextStyle) -> f32`, so every
  layout rule is testable headless. Math layout must follow the same shape.
- **The block invariant.** Every block holds ≥1 inline run, enforced
  centrally in `prune_runs`/`enforce()`. Divider already shows the pattern
  for a block that is "not text but holds the mandatory run".
- **Theme groundwork already laid.** `theme::math()` (Iosevka) exists and is
  documented as the math stack's font; `ALT`'s comment names it "the math
  slot fill"; `icons::NEXT_SLOT` is "the you-are-inside-a-structure marker
  in the status line". The status line mock in SPEC.md shows
  `math › frac › denom`.
- **Overlay pattern for popups.** Palette, slash menu, context menu are all
  detached regions anchored to a point, with `card_anchored` flip/clamp
  logic. The in-math completion card is the same pattern with a different
  row model.
- **SPEC commitments** that bind this design:
  - §6.2: linear triggers (`/`, `^`, `_`, brackets, `sqrt sum int lim` +
    space) for common structures, menu for rare ones; backspace right after
    a trigger reverts it; triggers fire on standalone tokens only;
    Tab/Shift-Tab between slots; empty slots never nag.
  - §4.3: Esc pops one context level — slot, then math, then Insert.
  - §6.3: raw text node as the panic button; renders plainly.
  - §6.4: node kinds named by meaning (`frac(num, den)`, never
    `stack(top, bottom)`).
  - §9: canonical, deterministic, versioned serialization in
    ` ```tw-math v1` fences; round-trip property-tested from day one.
  - §11: keystroke → glyph < 16 ms, reflow in one frame, nothing async on
    the typing path.

## 3. Data model

### 3.1 The tree

New pure module `src/document/math.rs`:

```rust
/// One atom in a horizontal list. Kinds are named by meaning (SPEC §6.4).
pub enum MathNode {
    /// One typed character: a digit, letter, operator, comma...
    /// Chars are individual atoms — the TeX hlist model — so the cursor is
    /// always a plain index into a list and never an offset into a string.
    Sym(char),
    /// A fraction. Slots may be empty; empty is permanently legal.
    Frac { num: MathList, den: MathList },
    // Later, same shape: Sup/Sub, Sqrt, Group (stretchy delimiters),
    // BigOp (sum/int/lim), Func, Var/Const (typed identifiers), Raw (§6.3).
}

pub type MathList = Vec<MathNode>;
```

Single-char atoms cost more nodes than string runs but buy three things:
cursor arithmetic is an index (no run/offset pair), trigger recognition is
a backward scan over chars, and later tokenization (recognizing `sum`,
recognizing a multi-char variable) is a display/recognition pass that never
restructures storage.

### 3.2 The cursor

```rust
/// A slot of a structural node, named, not numbered — the status line
/// prints these names ("math › frac › denom").
pub enum Slot { Num, Den /* later: Sup, Sub, Body, ... */ }

/// A step down the tree: into node `index` of the current list, through
/// one of its slots.
pub struct Step { pub index: usize, pub slot: Slot }

/// Where typing goes inside one math expression: follow `path` from the
/// root list, then sit at `index` atoms in — between atoms, like a text
/// caret sits between chars.
pub struct MathCursor { pub path: Vec<Step>, pub index: usize }
```

All edit operations live beside the types as pure functions over
`(&mut MathList, &mut MathCursor)`: insert char, backspace, insert
fraction, slot next/prev, move left/right, ascend (Esc). Everything
testable headless.

### 3.3 Two containers, one payload

- **Inline math** — `Inline::Math(MathList)`, a run variant beside
  `Inline::Text`. In the block's flat-text coordinate space the whole atom
  has **length 1**: one caret position covers it, exactly like one char.
  The caret can sit before or after it; entering it is an explicit gesture
  (see §5). It never splits, never merges, and word motions treat it as
  one word.
- **Math block** — `Block::Math(Vec<Inline>)` whose invariant is *exactly
  one run, and that run is `Inline::Math`* (enforced in `prune_runs`, the
  same central place Divider uses). It reuses every block mechanism —
  caret can rest on it, `dd` deletes it, blocks split/join around it — and
  is drawn centered and display-sized rather than inline-sized.

The flat-length-1 protocol is the entire integration contract with the
existing model: `run_len`, layout segments, `FlatPos` arithmetic, vim
motions, selections all keep working with no new coordinate system. Text
editing ops (`insert_str`, `split_run`, `remove_char_at`) apply only to
`Text` runs; on a `Math` run, insert-before/after resolves to the adjacent
text run (creating one if needed) and delete removes the whole atom — an
atom is never partially deleted from the outside.

### 3.4 Focus

`Document` gains `math: Option<MathCursor>` beside `caret`. When `Some`,
the document caret is parked on the math run it refers to (`caret.block` +
`caret.inline` name the run; the cursor names the position inside it) and
all typing routes into the math ops. One source of truth: the math cursor
is *part of the document state*, so undo, tab switching, and session
restore carry it for free.

## 4. Layout

New pure module `src/document/math_layout.rs`, mirroring
`document::layout`'s discipline: pure functions, measurement through the
same closure type, tests with a fake measure.

```rust
/// A laid-out node: its box and where its children sit, in coordinates
/// relative to the expression's origin (baseline-left).
pub struct MathBox {
    pub width: f32,
    /// Height above the baseline.
    pub ascent: f32,
    /// Depth below the baseline.
    pub descent: f32,
    pub kind: BoxKind, // glyph / rule / slot / list of positioned children
}
```

Rules, all from the TeX playbook but only as much of it as fractions need:

- **A list** lays out left to right on a shared baseline; its ascent and
  descent are the maxima of its children's. Width is the sum (inter-atom
  spacing classes come later with operators; PoC uses the glyphs' natural
  advances).
- **A fraction** centers numerator over denominator; the bar sits on the
  **math axis** (approximately half the x-height above the baseline, a
  constant `AXIS_RISE` derived from the font size for now); constant gaps
  `FRAC_GAP` above and below the bar; the bar overhangs the wider operand
  by `FRAC_PAD` each side. The fraction's ascent/descent grow with its
  operands — this is what makes arbitrary nesting "just work": a nested
  fraction is simply a tall child, and every ancestor's box grows to hold
  it.
- **Script levels.** Layout carries a `depth` (0 display, 1 script,
  2+ scriptscript) and scales the font size by `1.0 / 0.78 / 0.62`,
  clamped at level 2. Fraction operands in inline math go up one level;
  in a math block the top-level fraction keeps level 0 (display style).
- **An empty list is a slot box**: a fixed-size rounded placeholder
  (`SLOT_W × SLOT_H`, scaled by level), filled `theme::ALT`, outlined
  `theme::NON_TEXT` — the outline the theme already reserved for "empty
  slot outlines". The focused slot outlines `theme::ACCENT` instead.
- **The cursor position** resolves the same way the text caret does:
  `cursor_pos(&MathBox, &MathCursor) -> (x, baseline, height)` walks the
  positioned children. The editor draws the standard accent bar there.

Integration with `document::layout`:

- A math atom is one **unbreakable piece** in the wrap: its width is its
  laid-out box's width. It never hyphenates; an over-wide expression
  overhangs like an over-long word does.
- **Per-line height.** Today every visual line of a block has the block's
  constant line height. A line containing a math atom needs
  `max(base, ascent + descent + leading)`. `VisLine` already carries its
  own `height`; the change is computing it per line instead of copying a
  constant. `caret_pos`, `caret_band`, hit-testing already read the
  per-line value, so they follow automatically.
- A `Block::Math` block's height is its expression's box height plus
  padding; the expression is drawn centered in the measure.

Cost: math layout runs inside `layout()` (which is O(document) per edit
already) and is O(expression). Expressions are lecture-note sized; if a
profile ever says otherwise, the box tree memoizes per block — noted, not
built.

## 5. Input model

### 5.1 Entering math

- `/math` in the slash menu → **inline math**: insert an empty
  `Inline::Math` atom at the caret and focus it (`format.math`).
- `/math block` → **math block**: new `Block::Math` below (or converting an
  empty paragraph, the divider pattern), focused (`format.mathblock`).
- `<leader>m` in Normal aliases `format.math` (SPEC §6.1).
- SPEC §12.1 wants a one-key alias eventually; the command layer makes that
  a binding, not a design change. Deferred.
- **Click** on a rendered expression focuses it and seats the cursor at the
  nearest atom boundary. (PoC: click focuses at the end; precise seating
  when hit-testing lands.)
- With the caret adjacent to an atom, `Enter` (Insert) or `i` on it
  (Normal) re-enters it.

### 5.2 Inside math

All keys route to the math ops while `document.math` is `Some`:

| Key | Action |
|---|---|
| printable char | insert `Sym` at cursor (triggers checked after, §5.3) |
| `←` / `→` | move by atom; at a slot edge, climb out to after/before the parent; at the expression edge, exit math onto the adjacent flat position |
| `Tab` / `Shift-Tab` | next / previous slot in depth-first document order, cycling; prefers empty slots when any exist, otherwise all slots |
| `Esc` | pop one level: inside a slot → after the parent node; at the root list → exit math, caret after the atom, back to Insert (§4.3, one rule everywhere) |
| `Backspace` | at index > 0: if the atom before is a `Sym`, delete it; if structural, **revert it** (§5.3). At index 0 of a slot: climb out before the parent (never deletes through a slot wall) |
| `Enter` in a math block | leave the block: new paragraph below (matching prose Enter) |

Up/down between numerator and denominator (corca's arrow behavior) is a
nicety once cursor x-resolution exists; Tab is the contract, arrows are
polish. Deferred, noted.

### 5.3 Triggers (SPEC §6.2)

On `/` typed inside math:

1. Scan backward from the cursor for the **preceding operand**: one
   structural node (a fraction, later a group), else the longest trailing
   run of `Sym` atoms that are not operators (letters, digits, `.`, `_`).
2. Non-empty operand → it becomes the numerator; cursor lands in the empty
   denominator. This is what makes `1/2` then `/3` build `(1/2)/3` — the
   whole preceding fraction is the operand — and typing `a`, `/`, `b` read
   as write-what-you-say.
3. Empty operand (cursor at slot start or after an operator) → empty
   fraction, cursor in the numerator.

**Revert**: `Backspace` when the atom immediately before the cursor is a
structure it just created flattens it back to the literal atoms
(`frac(num, den)` → `num / den` as plain atoms, cursor after the `/`).
Implemented structurally — reverting is a function of the node, not of an
undo journal — so it also serves as "dissolve this fraction" at any later
time, which is more useful than a time-window special case and simpler
than one.

Later triggers (`^`, `_`, brackets, `sqrt`+space...) reuse the same two
mechanics: operand capture and structural revert. The word-triggers fire
on standalone tokens only — the backward scan stops at a non-identifier
atom, so a variable named `sum` survives (SPEC §6.2 rule 2).

### 5.4 The in-math palette

Deferred past the PoC, designed now: identifier autocompletion à la corca
— typing letters inside math opens the anchored completion card
(slash-menu pattern, same `card_anchored`), offering: identifiers already
used in this document, Greek letters by name, operator words, structure
commands (everything the trigger set covers plus menu-only structures).
Enter/Ctrl+N picks; typing a trigger char takes the literal path. This is
a shell concern (the shell owns keystrokes and popups); the math module
only exposes "the word before the cursor".

### 5.5 Typed identifiers

Variables, constants, functions as distinct node kinds with distinct
rendering (italic serif variables, upright constants, chips only where
corca uses them — restrained; this is a page of notes, not a form), and a
scalar/vector distinction rendered as weight (bold upright vectors, the
standard convention). All are display-semantic only: no bindings, no
store, no evaluation (SPEC §6.4). These are `MathNode` kinds added later —
the `Sym` stream stays the source they are recognized from, so no
migration.

## 6. Serialization (SPEC §9)

- **Math block:**

  ````markdown
  ```tw-math v1
  (1 + x) / 2
  ```
  ````

- **Inline math:** `` $…$ `` with the same notation inside. The format is
  ours alone (SPEC §9); `$` never triggers anything when typed in prose
  (§4.1 — no accidental formatting), it is purely the serializer's
  delimiter, escaped as `\$` in prose text on write.

- **Notation v1**, covering `Sym` + `Frac`:
  - `Sym` chars print as themselves;
  - `Frac` prints `num/den`. The rule, stated precisely: parens wrap an
    operand iff its list holds more than one atom. So `R_2/R_1` stays
    bare (each operand is one atom once identifiers are typed entities;
    until then, `(R_2)/(R_1)`), `(1 + x)/2` wraps its sum, and a nested
    fraction — one atom — needs no parens of its own. Parens are
    **structural**: the parser treats `(`…`)` as grouping, not as atoms.
    Literal brackets become `Group` nodes when those arrive; v1 documents
    cover only what v1 can express.
  - Whitespace: single spaces around the top-level `/`? No — determinism
    demands one rule: **no spaces are printed**; `Sym(' ')` is preserved
    as-is. Print∘parse is identity on trees; parse∘print is byte-identity
    on files we wrote (property-tested, per SPEC).
- Search consequence: `R_2/R_1` in a file is findable by typing exactly
  that (SPEC §7.3), which is the point of the linear notation.
- Unknown versions or unparsable content load as a raw math text node
  (§6.3's bail-out doubles as the corruption path — never block the file).

## 7. Rendering

In `components/editor.rs`:

- A math atom draws its `MathBox` tree at the pen position: glyphs through
  `theme::draw` with `TextStyle::math(size, theme::INK)`, fraction bars
  through `theme::rule` (whole-pixel y, same as the divider), slots as §4
  describes.
- The focused expression draws the math cursor (accent bar) instead of the
  document caret, and its active slot gets the accent outline. The
  document caret is suppressed while `math` focus is active — two blinking
  bars would be two claims about where typing goes.
- The status line shows the cursor path — `math › frac › denom` — via the
  existing status machinery and `icons::NEXT_SLOT` (wired when the status
  line task lands; the path string is a one-liner off `MathCursor`).

## 8. What is deliberately not built

- No units or symbol bindings (SPEC §6.4). Evaluation exists only as a
  read of one expression for plotting (`document::math_eval`, through
  exmex): it never changes the tree, stores nothing, and binds no symbol
  across a note. Plain letters are free variables; only symbols resolved
  as functions or constants carry a meaning. Notation without a numeric
  reading yet (big operators, accents, undefined functions) is reported,
  never guessed.
- No LaTeX import/export (SPEC §13.2).
- No repair workflow for raw nodes (§6.3).
- No matrix/cases/decorations until the slot machinery is proven — they
  are menu-only structures over the same `MathList` slots.
- No per-keystroke incremental math layout — whole-expression relayout is
  within budget at note scale.

## 9. Build plan

Sequential waves, each delegated, each verified before the next:

1. **`document/math.rs`** — tree, cursor, edit ops, fraction trigger,
   revert, slot navigation. Pure, exhaustively unit-tested. No other file
   touched.
2. **`document/math_layout.rs`** — box layout for lists, fractions, slots,
   script levels; cursor position resolution. Pure, tested with fake
   measure.
3. **Model integration** — `Inline::Math`, `Block::Math`, flat-length-1
   protocol, invariants in `prune_runs`, focus field; compile-driven sweep
   of every `Inline::Text` match site (motions, layout segments, editor,
   markdown treat the atom as opaque length-1). Markdown: `$…$` and
   ` ```tw-math v1` fences, printer + parser + round-trip property tests.
4. **Per-line heights + editor drawing** — layout computes line heights
   from content; editor draws math boxes, slots, math cursor.
5. **Shell wiring** — `format.math` / `format.mathblock` commands, slash
   entries, key routing into math ops, Esc/Tab handling, focus enter/exit.
6. **Runtime verification** — harness script: create note, `/math block`,
   type `1/2`, Tab, nest a fraction into the denominator, screenshot;
   assert the saved file's fence content and reload it.

Proof of concept exit criteria: any number of fractions, arbitrarily
nested, each slot typable, Tab/Shift-Tab cycling, Esc popping levels,
backspace reverting, everything resizing and repositioning correctly, and
the file round-tripping through save/load.

After the PoC, in order of daily value: `^`/`_` scripts, brackets/groups,
the in-math palette (§5.4), word triggers (`sqrt sum int lim`), typed
identifiers (§5.5), raw text node, status-line path.
