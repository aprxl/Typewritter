# Task 01 — Document model (AST + caret + editing ops)

**Wave 1.** No dependencies on other tasks. Pure model: no IO, no renderer,
no winit. Everything here must be unit-testable with plain `cargo test`.

**You own**: `src/document/mod.rs` (new), `src/document/markdown.rs` (new
stub — just a doc comment; task 02 fills it), `src/document/layout.rs` (new
stub — same), `src/lib.rs` (add `pub mod document;`). Nothing else.

**Read first**: `tasks/00-overview.md` (invariants §3, module map §4),
`docs/SPEC.md` §4 (cursor states, Esc rule), `src/buffer.rs` (the model
being replaced — read its tests to understand which behaviors survive),
`src/tabs.rs` (how the model will be driven: `edit` vs `touch`).

---

## 1. Types

```rust
// src/document/mod.rs

pub struct Document {
    pub blocks: Vec<Block>,   // invariant: never empty
    pub path: PathBuf,
    pub name: String,         // file name, for tabs/status/breadcrumb
    dirty: bool,
    pub caret: Caret,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Block {
    Paragraph(Vec<Inline>),
    Heading { level: u8, content: Vec<Inline> },  // level 1..=3
}

#[derive(Clone, PartialEq, Debug)]
pub enum Inline {
    Text(Text),
}

#[derive(Clone, PartialEq, Debug)]
pub struct Text {
    pub text: String,
    pub style: Style,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Caret {
    pub block: usize,    // index into blocks
    pub inline: usize,   // index into the block's inline runs
    pub offset: usize,   // char offset within the run's text
    pub style: Style,    // context: what typed text becomes (SPEC §4.2)
}
```

Provide:

```rust
impl Style {
    pub const PLAIN: Style;                 // bold: false, italic: false
    pub fn is_plain(&self) -> bool;
}

impl Block {
    pub fn inlines(&self) -> &[Inline];
    pub fn inlines_mut(&mut self) -> &mut Vec<Inline>;
    pub fn is_heading(&self) -> bool;
}

impl Document {
    pub fn new(path: &Path) -> Document;    // one empty paragraph; name from path
    pub fn is_dirty(&self) -> bool;
    pub fn word_count(&self) -> usize;      // whitespace-split over all runs
}
```

## 2. Invariants (enforce in every op)

1. `blocks` is never empty. An "empty document" is
   `vec![Block::Paragraph(vec![Inline::Text(Text { text: String::new(), style: Style::PLAIN })])]`.
2. Every block has ≥1 inline run.
3. A run's `text` is empty only when it is the sole run of an empty block
   (the placeholder the caret sits on). After every content edit, drop any
   run that became empty unless it is that placeholder.
4. Adjacent runs may share a style (normalization is *not* required);
   merging adjacent same-style runs after an edit is allowed and keeps the
   model tidy.
5. The caret is always in bounds after every op. `inline`/`offset` clamp to
   the block's runs; `block` clamps to `blocks`.

## 3. The style-context machine (the subtle part — spec §4.2/Q9)

Think of a block's text as a flat char sequence where each char has a style
(its run's style). The caret sits *between* chars and carries a context
style. At a run boundary there are two positions with the same x: *inside
the run at its end* (context = that run's style) and *outside it* (context =
the following run's style). Right-arrow walks from the first to the second.

Definitions (block-scoped; blocks never share context):

- `style_at(block, flat_pos)`: style of the char at flat position, or
  `None` if `flat_pos == block_len` (end of block).
- `style_before(block, flat_pos)`: style of the char at `flat_pos - 1`,
  or `None` if `flat_pos == 0` (start of block).
- `None` context behaves as `Style::PLAIN`.

`move_right()`:

```
let after = style_at(caret);
if caret.style != after.unwrap_or(PLAIN) {
    caret.style = after.unwrap_or(PLAIN);   // pop out of the run — no movement
} else if flat_pos < block_len {
    advance one char (crossing run boundaries within the block);
    caret.style = style_before(new flat_pos).unwrap_or(PLAIN);
} else if caret.block + 1 < blocks.len() {
    caret.block += 1; caret.inline = 0; caret.offset = 0;
    caret.style = PLAIN;                    // block start: outside everything
} // else: end of document, nothing
```

`move_left()` is the mirror:

```
let before = style_before(caret);
if caret.style != before.unwrap_or(PLAIN) {
    caret.style = before.unwrap_or(PLAIN);  // re-enter the run to the left
} else if flat_pos > 0 {
    retreat one char; caret.style = style_at(new flat_pos).unwrap_or(PLAIN);
} else if caret.block > 0 {
    caret to end of previous block; caret.style = PLAIN;  // outside-at-end
} // else: start of document
```

`set_caret(block, inline, offset)` (click placement / jump): clamp into
bounds; `caret.style = style_before(...)unwrap_or(PLAIN)`.

Helper conversions between `(inline, offset)` and flat positions are
internal; expose what the ops need, not more.

## 4. Editing ops (`&mut self`; content edits set `dirty = true`, movement never does)

| Op | Behavior |
|---|---|
| `insert_text(&mut self, text: &str)` | Insert at caret with `caret.style`. If the run to the left has that style, append there; else if the run to the right has it, prepend there; else splice in a new run at the caret (splitting the current run if mid-run). Typing into an empty block replaces the placeholder run with a real run of `caret.style`. Caret moves past the inserted text; style context unchanged. |
| `backspace()` | **Always deletes a character (§12.3), never formatting.** If `offset > 0`: delete the char before the caret in the current run. Else if `inline > 0`: delete the last char of the previous run. Else if `block > 0`: append this block's runs onto the previous block's runs (heading or paragraph — no kind check), caret at the junction, delete this block. Else: nothing. After the delete, caret.style = style of the char now before the caret (PLAIN at block start). |
| `delete_forward()` | Mirror of `backspace`: char at the caret; at run end → first char of next run; at block end → merge next block into this one. |
| `newline()` | Split the current run at the caret; everything after (with its styles) moves to a new **Paragraph** inserted after the current block (splitting a Heading never continues the heading). Caret to start of the new block, style context preserved (you keep typing in the same style). |
| `delete_char()` | vim `x`: delete the char under the caret only. Nothing at run end / block end / empty block. Never merges. |
| `delete_line()` | vim `dd`: remove the caret's whole block. If it was the only block, replace with an empty paragraph. Caret lands on the same-or-clamped block index, at its first non-blank char, context PLAIN. |
| `open_below()` / `open_above()` | vim `o`/`O`: insert an empty Paragraph below/above the caret's block; caret on it, context PLAIN. |
| `toggle_bold()` / `toggle_italic()` | Flip the flag on `caret.style` (the pending context). Not a content edit; does **not** set dirty. |
| `set_heading(level: Option<u8>)` | Convert the caret's block: `Some(1..=3)` → Heading, `None` → Paragraph. Content runs preserved. Dirty. |
| `move_home()` / `move_end()` | Logical: start / end of the current block's flat text. Context per the style-before rule. (Visual-line Home/End is a layout concern, not here.) |

`Tabs` will wrap these in `edit` (content changes) vs `touch` (movement).
For this task the distinction is only: **movement ops must not set dirty**.

## 5. Tests (write them all; they are the spec's teeth)

- Insert into empty block → one real run, caret after the text.
- Insert with bold context between plain runs → new bold run spliced in.
- Insert merging into left / right same-style run (no duplicate runs).
- Backspace: mid-run, at run boundary (deletes previous run's last char),
  at block start (merges blocks, caret at junction), at document start
  (no-op).
- `newline` mid-paragraph splits runs and styles; at end of Heading
  creates a Paragraph.
- `delete_line` on the only block leaves one empty paragraph.
- The style machine, explicitly:
  - caret at end of a bold run: `move_right` changes context to PLAIN
    without moving; a second `move_right` moves.
  - mirror for `move_left` re-entering the run.
  - clicking at a boundary gets the before-style.
  - crossing a block boundary resets context to PLAIN.
- `set_heading` round-trips a block's kind without touching its runs.
- Invariants hold after every op above (assert blocks non-empty, caret in
  bounds, no empty non-placeholder runs).

## 6. Verification

```
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

`src/document/markdown.rs` and `layout.rs` exist as doc-comment stubs so
wave-2 tasks own exactly one file each. `src/lib.rs` gains
`pub mod document;` — nothing else changes anywhere in the crate, and the
old `src/buffer.rs` stays put for now (task 05 deletes it).

## 7. Non-goals

Parsing/serialization (02), any pixel/layout concept (04), shell or vim
wiring (05), undo, selections, Normal-mode operators.
