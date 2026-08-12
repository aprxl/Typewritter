# Typewritter — Design Specification

**Source:** 23 decisions you made directly (3 before the interview, 20 during it). Every line below traces to one of them.

**Provenance markers:**
- `[Qn]` — your decision, verbatim.
- `[derived]` — my inference from your decisions. Overridable, and I've said why each one follows.
- `[flagged]` — a conflict between two of your own answers. Listed in full in §12.

**Out of scope, per your brief:** tech stack, dependencies, colour and typography themes.

---

## 1. What Typewritter is

A keyboard-driven capture tool for math-heavy lecture notes, with a structural editor that saves Markdown files.

That sentence is narrower than "an Obsidian replacement," and it's what your answers describe. You write fast, live, under time pressure `[Q2]`; you rarely go back to clean up `[Q16]`; you return occasionally to hunt one fact `[Q17]`; and linked reference material is the least of what you write `[Q1]`. The product optimises for the first minute of contact with a note, not the fiftieth.

**One consequence up front:** you never see or type Markdown `[Q8, Q10]`. Markdown is the file format, the way `.docx` is for Word. Typewritter is a structural editor that happens to serialize to `.md` — not a Markdown editor. Everything in §4 follows from this.

---

## 2. Decision register

| # | Decision | Source |
|---|---|---|
| 1 | Math is rendered, never evaluated | pre-interview |
| 2 | Vim: basic motions only, dot-repeat kept | pre-interview |
| 3 | Math stored as canonical tree notation | pre-interview |
| 4 | Primary use: lecture notes; wiki-style linking last | Q1 |
| 5 | Math is typed live, racing the lecturer | Q2 |
| 6 | One note per lecture; many small files | Q3 |
| 7 | Launch restores the last note and cursor | Q4 |
| 8 | Four collapsible regions + sidenote margin | Q5, Q12 |
| 9 | Tabs primary, peek secondary | Q6 |
| 10 | New note: one keystroke, then type a name | Q7 |
| 11 | Fully rendered; markup never visible | Q8 |
| 12 | Right-arrow / Esc exits a formatting run | Q9 |
| 13 | Formatting via leader menu, not typed markup | Q10 |
| 14 | Formatting applied both after and mid-sentence | Q11 |
| 15 | Blocks: badges, tables, folds, sidenotes | Q12 |
| 16 | Math entered via menu binding | Q13 |
| 17 | Math structure: linear triggers + menu | Q14 |
| 18 | Bail-out: raw text node + placeholder marker | Q15 |
| 19 | Notes stay rough; no cleanup pass | Q16 |
| 20 | Retrieval is search, not browsing | Q17 |
| 21 | Two machines, automated git sync | Q18 |
| 22 | PDF export, 1:1 with the editor | Q19 |
| 23 | Failure ranking: math speed > lag > findability > loss | Q20 |

---

## 3. Layout

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ opamps ×│ lecture-04 ×│ +                                                    │ ← tabs
│ vault › analog › opamps › Non-inverting amp                                  │ ← breadcrumb
├───────────────┬─────────────────────────────┬──────────────┬─────────────────┤
│ ANALOG      ▾ │  Non-inverting amplifier    │              │ TOPICS          │
│   opamps      │                             │              │  Topology       │
│   filters     │  Closed-loop gain:          │ ¹ ideal case │   · Ideal       │
│   bias-notes  │                             │   A → ∞      │   · Bias        │
│ DAILY       ▾ │          R₂                 │              │  Gain           │
│   2026-08-09  │   A = 1 + ──         (2)    │              │   · Closed-loop │
│               │          R₁                 │              │   · Open-loop   │
│               │                             │              │                 │
│               │  With R₂ = 10k …¹           │              │                 │
├───────────────┴─────────────────────────────┴──────────────┴─────────────────┤
│ INSERT   opamps   math › frac › denom          ⟳ synced   1,204 w            │
└──────────────────────────────────────────────────────────────────────────────┘
  working dir       text column          sidenotes       topics & subtopics
```

### 3.1 Regions `[Q5]`

| Region | Position | Contents |
|---|---|---|
| Tab strip | top row 1 | open notes, preview tabs italic |
| Breadcrumb | top row 2 | *document location* — vault › folder › note › heading, clickable |
| Working directory | left | file tree |
| Text column | centre | the note |
| Sidenote margin | centre-right, **inside the canvas** | footnotes aligned to their anchors |
| Topics & subtopics | right | live outline, two levels |
| Statusline | bottom | *editor state* — mode, math node path, sync state, counts |

All four outer regions collapsible `[Q5]`.

### 3.2 Rules

- **Breadcrumb and statusline have split duties** `[derived]`. Top answers "where is this note"; bottom answers "what is the editor doing". Without the split they duplicate each other.
- **The sidenote column is part of the canvas, not a panel** `[derived]`. Q5 put topics on the right and Q12 put sidenotes on the right; they can't both be the right panel. Text column and margin scroll together as one unit; the topics panel is separate and further right.
- **Sidenotes push down on collision** `[derived]`. Two anchors three lines apart both want the same vertical position. Resolution must be instant, with no visible settling during typing.
- **Capture mode** `[derived]` — one key collapses all four regions to bare text, one key restores. Given Q2, this is the state you're in whenever you're actually typing.
- **Topics panel updates live** `[derived]`. In a lecture you're building the heading structure in real time; it's a progress indicator, not just navigation.
- **Left tree auto-collapses under width pressure** `[derived]`. Tree + text + margin + topics does not fit a laptop screen, and the tree is the least load-bearing of the four given Q17.

### 3.3 Startup `[Q4]`

Last note, cursor exactly where you left it. No splash, no dashboard, no home screen. Indexing happens in the background and never blocks the canvas.

- Session state persists **continuously**, not on quit `[derived]` — open tabs, cursor, scroll, folds, panel widths. A crash or force-quit still lands you back in place.
- Cursor position is stored as a **node path, not line:col** `[derived]`. It survives reformatting and external edits. Needs a documented fallback when the node no longer exists — nearest surviving ancestor, then start of document.

---

## 4. Editing model

### 4.1 Fully rendered `[Q8]`

Markup is never displayed. There is no live-preview toggle, no reveal-on-cursor-line, no source view as a normal mode of operation.

- **Raw view exists as an escape hatch only** `[derived]`. When something renders wrong you need to see the file, and with no other inspection path you'd be stuck. Per-document and per-block, never the default.
- **No accidental formatting, ever** `[derived]`. Since nothing is a trigger, `#5` stays `#5` and `*` stays `*`. No escaping rules exist because none are needed. This matters more than it sounds for lab logs full of pointers, footnote marks, and multiplication.

### 4.2 Cursor states `[Q9]`

At the end of a bold run there are two distinct cursor positions with the same x-coordinate: *inside the run at its end*, and *outside the run*. Right-arrow (or `l` in Normal) walks from the first to the second.

- **The caret renders its own state** `[derived]`. It takes the styling of whatever you'd type next — a bold caret means you're still inside. Without this the two positions are indistinguishable and the model fails.
- **Backspace at a boundary needs an explicit rule** `[flagged, §12.3]`. Currently unspecified.

### 4.3 Esc = pop one context level `[derived]`

One rule, everywhere: leave the formatting run, then the list item, then the math slot, then Insert mode. Derived from Q9, and it's exactly what math editing needs for escaping a fraction denominator.

### 4.4 Applying formatting `[Q10, Q11]`

Two entry points, one per mode:

| Mode | Entry | Example |
|---|---|---|
| Normal / Visual | `<leader>` | `<leader>b` |
| Insert | `/` menu | `/bold` |

- **Insert routes through the `/` menu, not a chord** `[revised]`. `Ctrl-H`, `Ctrl-I`, `Ctrl-M`, `Ctrl-W`, `Ctrl-R` and `Ctrl-[` are all spoken for by terminal and vim convention, and a `Ctrl-F` prefix was one more thing to memorise. A bare `/` on the typing path lists every editor-only action by name; `//` types a literal slash.
- **In Normal mode, formatting is an operator** `[derived]`: `<leader>biw` bolds the inner word, and dot-repeat then applies it to the next one. Consistent with keeping dot-repeat.
- **Insert-mode formatting opens an empty run**, indicated by the styled caret from §4.2.
- **Which-key hints appear in Normal mode only** `[derived]`. Q2 forbids popups on the typing path; that rule is absolute.

### 4.5 Vim surface

Basic motions, per the pre-interview decision. In: Normal/Insert/Visual(char+line)/Command, `h j k l w b e 0 ^ $ gg G { } f t F T`, `/ ? n N`, `d y c` × motions, `x s p P o O a i A I`, counts, `u`/`Ctrl-r`, `iw aw ip ap i" i(`, and `ih` (heading + everything nested under it). Out: macros, named registers, marks, visual block, undo tree, `:g`, `:cdo`, Vimscript.

**Dot-repeat is retained** despite being the most expensive item on the list. It's the difference between a vim mode and a vim costume.

---

## 5. Blocks

**In** `[Q12]`: badges and inline chips · tables · foldable sections under any heading · sidenotes in the margin · code blocks with syntax highlighting · image embeds.

**Out** `[Q12]`: callout boxes. You asked for them in your opening brief and then didn't select them. Structure comes from headings, folds, and badges instead.

Because you never see syntax `[Q8, Q10]`, the on-disk representation of these blocks is purely a serialization concern with no user-facing consequence. Any consistent directive-style encoding works; pick one and make it deterministic (§9).

---

## 6. Math

The core of the product, and the thing whose failure kills it `[Q20]`.

### 6.1 Entry `[Q13]`

`<leader>m` in Normal, `/math` in Insert. Math is just another block in the same menu as badges and tables — no special case. Exit via §4.3.

`[flagged, §12.1]` — this is three keystrokes for your highest-frequency gesture.

### 6.2 Building structure `[Q14]`

Linear triggers for common structures, menu for rare ones.

**Linear set** `[derived]` — the rule is *more than once per lecture*:

| Trigger | Produces |
|---|---|
| `/` | fraction; preceding operand becomes the numerator |
| `^` `_` | superscript / subscript slot |
| `(` `[` `{` | auto-paired, stretchy delimiters |
| `sqrt` `sum` `int` `lim` + space | radical, Σ, ∫, lim with their slots |

**Menu-only:** matrices, cases, over/underbrace, tensors, anything exotic.

**Two rules that make this survivable at speed** `[derived]`:

1. **Backspace immediately after a trigger reverts it** to the literal characters. You will fire triggers accidentally while racing; without this you're fighting the editor mid-lecture.
2. **Triggers fire on standalone tokens only**, so a variable named `sum` survives.

`Tab` / `Shift-Tab` move between slots. Empty slots are permanently legal and never nag — nothing in the system needs an expression to be complete, since nothing evaluates it.

### 6.3 The bail-out `[Q15]`

When the notation outruns you, two mechanisms:

1. **Raw text node** — type anything, no validation, no parsing. **This needs the fastest gesture in the app** `[derived]`: a single key inside math, `Esc` to leave. It's the panic button, and right now it would be quicker to reach than math entry itself.
2. **Placeholder marker** — drop a mark and keep moving.

**Raw nodes render plainly** `[derived]` — normal weight, on the math baseline, unobtrusive. Not dim, not monospace, not flagged. Q16 says rough *is* the final state, so styling raw nodes as debt would make every note you own look permanently broken.

**No repair workflow** `[Q16]`. No unresolved queue, no statusline counter, no vault-wide fix-it view. Cleanup rarely happens, so tooling for it is wasted.

### 6.4 What math does not do

No evaluation, units, dimensional analysis, symbol bindings, plots, or ghost results (pre-interview decision). No ad-hoc symbol nodes `[Q15]` — unknown notation goes into a raw text node instead.

The tree is **presentational-semantic**: it knows a fraction is a fraction and `≤` is a relation, because layout requires that, but it knows nothing about meaning.

**One forward-compatibility rule, and it costs nothing today** `[derived]`: name node kinds by meaning, never geometry. `frac(num, den)`, not `stack(top, bottom)`. If you ever want evaluation, that's an additive pass over an unchanged tree. Skip it and it's a re-parse of your whole vault.

---

## 7. Navigation and retrieval

### 7.1 Tabs and peek `[Q6]`

- **Preview tabs** `[derived]` — opening a note gives an italic preview tab, replaced by the next open. It becomes permanent only if you edit or pin it. Without this, 120 notes a semester `[Q3]` means tab pileup by week six.
- **Peek never creates a tab** `[derived]`. That's the entire reason to have both. Separate trigger from open; `Esc` dismisses.

### 7.2 Note creation `[Q7]`

One keystroke, then type the name. Consequences `[derived]`:

- **The date leaves the filename**, so it's auto-stamped into frontmatter as `created`. Your top two uses are both dated `[Q1]`; this can't be optional.
- **The left tree can't sort alphabetically.** A folder of hand-named lecture notes sorts into nonsense. Default is created-date descending, with a toggle.
- **New notes land in the current note's folder.** No picker, no prompt.
- **Rename is cheap**, since almost nothing links to these files `[Q1]`.

### 7.3 Search `[Q17]`

Search is the whole retrieval story. Not the tree, not the outline, not tabs, not links.

Requirements `[derived]`:

1. **Math is searchable.** Hunting one fact in an EE notebook usually means hunting an equation. Canonical tree notation on disk means `R_2 / R_1` is a plain text search that works.
2. **Results answer without opening the note.** Full-height panel, real surrounding context, and math **rendered as math** in the results — not shown as notation.
3. **Fuzzy tolerance is mandatory.** Your notes are rough by your own account `[Q16]`, typos included. Exact match over rough notes finds nothing.

### 7.4 Not built `[Q1]`

Graph view, backlinks, unlinked mentions, link autocomplete as a primary feature. Wiki-style linking ranked last; Obsidian's centre of gravity is not yours.

---

## 8. Sync `[Q18]`

Two machines, git, configured once and then invisible.

- **Auto-commit on idle plus on close** `[derived]`, message derived from which notes changed. Per-keystroke history is unusable; per-session loses too much.
- **Conflicts are handled in-app** `[derived]`. Git writes conflict markers into `.md`, and a conflicted file may not parse into a tree at all — meaning the app can't open it. Required: detect on open, side-by-side resolution, and if resolution fails, **keep both versions as separate notes rather than blocking.** Never drop a student into `git mergetool` before a lecture.
- No accounts, no telemetry, no vendor sync service.

---

## 9. Serialization

Canonical tree notation in fenced blocks, versioned:

````markdown
```tw-math v1 #eq:gain
A = 1 + R_2 / R_1
```
````

- **Determinism is a correctness requirement, not a diff-quality nicety** `[derived]`. Non-deterministic printing plus auto-commit `[Q18]` means a phantom commit every time you open a note, and an unreadable history.
- **Round-trip is property-tested from day one**: print∘parse is identity on trees; parse∘print is byte-identical on any file we wrote.
- **Version marker in the info string.** You will change this format; make that a detectable migration rather than silent corruption.
- **The format is yours alone**, so PDF export (§10) is the only way content leaves. There is no LaTeX interchange requirement, since you never said you needed one — see §13.2.

---

## 10. PDF export `[Q19]`

1:1 with the editor. Stronger than it sounds:

- **One layout engine, not two** `[derived]`. Screen and PDF render through the same path or the promise quietly breaks. This is the most constraining thing in the spec.
- **Sidenotes repaginate** `[derived]`. The margin column is anchored to scroll position on screen; a PDF has fixed pages, so a sidenote near a break travels with its anchor.
- **Keep-together rules** `[derived]` for display equations, tables, and code blocks, which currently have no reason to know pages exist.
- **Collapsed folds expand on export**, with a toggle `[derived]` — see §13.1.

---

## 11. Performance budget

Your failure ranking `[Q20]` puts input speed first and lag second, so these are requirements, not aspirations:

| Path | Budget |
|---|---|
| Keystroke → glyph on screen | < 16 ms, no exceptions, nothing async on this path |
| Cold launch → editable cursor | < 500 ms |
| Note switch (tab or peek) | < 50 ms |
| Search → first results | < 100 ms |
| Reflow after a structural change | one frame, never animated |

Reflow deserves emphasis: fully-rendered editing `[Q8]` means text moves under you as you type. Instant and unanimated is the difference between acceptable and unusable at lecture speed.

---

## 12. Flagged tensions

Conflicts between your own answers. Each needs your call.

### 12.1 Math entry cost vs. your top failure mode
You ranked "typing math is slower than Obsidian + LaTeX" as the #1 thing that sends you back `[Q20]`, then chose a three-keystroke menu binding for math entry `[Q13]` — your highest-frequency gesture in a math-heavy lecture. In Obsidian, `$` is one key.

**Recommendation:** keep the menu binding as the canonical, discoverable path *and* alias one bare key to it. You already accepted this asymmetry in math itself, where you chose typed triggers for frequent structures and the menu for rare ones `[Q14]`. The same frequency argument applies here.

### 12.2 Render fidelity vs. lag
Fully rendered `[Q8]` + sidenotes with collision resolution `[Q12]` + 1:1 PDF `[Q19]` is a large amount of layout work on the hot path, and lag is your #2 failure mode `[Q20]`. These are not incompatible, but sidenote repositioning during typing is the specific thing most likely to blow the 16 ms budget.

**Recommendation:** sidenote layout runs off the typing path and is allowed to lag one frame behind the text.

### 12.3 Backspace at a formatting boundary
Unspecified. At the edge of a bold run, does Backspace delete a character or remove the formatting? Every WYSIWYG editor gets this wrong somewhere.

**Recommendation:** always delete a character. Removing formatting is `<leader>b` again, explicitly. Never make a destructive key do two different things based on invisible state.

### 12.4 Rough notes vs. findability
Notes stay rough `[Q16]`, but "can't find something I know I wrote" is your #3 failure mode `[Q20]`. Rough notes are harder to search: raw text nodes contain unstructured garbage, placeholders mark gaps, headings may be half-written.

**Recommendation:** index raw node contents as searchable text, not as opaque blobs. It's the only way a rough note stays findable.

---

## 13. Open questions

1. **Collapsed folds in PDF export.** Strict 1:1 says a collapsed section doesn't print. Spec currently says expand-with-toggle. Confirm.
2. **LaTeX import.** You never asked for it, so it's not in the spec — but it's the only route your existing Obsidian equations could take into Typewritter. If you plan to migrate rather than start fresh, this becomes a v1 blocker.
3. **Images.** Assumed in `[§5]` for lecture slides and lab logs, but you rejected board photos as a bail-out `[Q15]` and never specified an insertion gesture. Paste-from-clipboard is the presumed default.
4. **Callout boxes.** In your brief, absent from your selection `[Q12]`. Confirm the cut.

---

## 14. Build order

1. **Canonical notation spec, printer, parser, round-trip property tests.** Nothing else is testable until this exists, and §9 makes determinism a correctness requirement.
2. **Math tree and layout engine** — spacing classes, script levels, stretchy delimiters. Same engine will serve PDF (§10), so build it once and build it right.
3. **Linear triggers, slot navigation, raw text node.** This is the §11 budget's hardest test and your #1 failure mode.
4. **Editor core:** fully-rendered prose, cursor states, Esc rule, grouped undo.
5. **Formatting bindings, blocks, sidenotes.**
6. **Tabs, peek, note creation, search.**
7. **Git sync and conflict handling.**
8. **PDF export.**

Structure mode — tree navigation for editing existing expressions — is **v2** `[Q16]`. It's a cleanup tool, and you don't clean up.
