# Task 04 — Document layout (wrap + caret mapping)

**Wave 2.** Depends on task 01 (the `Document` model). You own **only**
`src/document/layout.rs` (currently a doc-comment stub — fill it; do not
touch `mod.rs`).

**Read first**: `tasks/00-overview.md` (§2 pipeline, §5 seams),
`src/document/mod.rs` as merged from 01, `src/theme.rs` (`TextStyle`,
`bold()`, `italic()`), `src/prose.rs` — *study* its greedy wrap and its
fake-measure tests, but **do not reuse it**: prose collapses whitespace
and carries no source offsets, both fatal here. Your segments must map
exactly back to the model.

---

## 1. What this is

The bridge between the model and pixels. Given a `Document`, a column
width, and a way to measure text, produce the visual lines the editor
draws — and the inverse mappings the caret, clicks, and `j`/`k` need.
Everything is pure and measured through a closure, so every rule is
testable headless.

## 2. API

```rust
pub struct DocLayout {
    pub blocks: Vec<BlockLayout>,
    pub height: f32,            // total content height, for scroll bounds
}

pub struct BlockLayout {
    pub y: f32,                 // top of the block, relative to content top
    pub lines: Vec<VisLine>,    // never empty — an empty block has one empty line
    pub height: f32,
}

pub struct VisLine {
    pub y: f32,                 // top of the line, relative to content top
    pub height: f32,
    pub segments: Vec<Segment>, // contiguous, exact source coverage of the line
}

pub struct Segment {
    pub inline: usize,          // run index in the source block
    pub start: usize,           // char offset within the run
    pub len: usize,             // char count
    pub style: Style,           // copied from the run — what to draw with
}

/// One pass; `measure(text, style) -> width` is the only rendering input.
pub fn layout(
    doc: &Document,
    width: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> DocLayout;
```

Style → font (export it; the editor and shell both need it):

```rust
pub fn text_style(kind: &Block, style: Style) -> TextStyle;
// body:     theme serif 17.5 INK
// heading:  serif {1: 24.0, 2: 21.0, 3: 18.5} INK, always .bold()
// then:     style.bold → .bold() (stacks with heading bold — same factor,
//           so it is idempotent), style.italic → .italic()
```

Rhythm constants (export; editor + shell use them):

| Constant | Value | Meaning |
|---|---|---|
| `LINE_BODY` | 30.0 | body line height |
| `LINE_H1` / `LINE_H2` / `LINE_H3` | 40.0 / 34.0 / 30.0 | heading line heights |
| `GAP_PARAGRAPH` | 14.0 | space below a paragraph |
| `GAP_HEADING` | 26.0 | space *above* a heading (first block: 0) |
| `GAP_AFTER_HEADING` | 8.0 | space below a heading |

## 3. Wrapping rules (exactness beats typography)

Greedy word wrap per block, like `prose.rs`, but with **source-exact
coverage**:

- Split each run into word pieces on spaces; each space is its own piece.
- A visual line's segments must be contiguous and cover exactly the source
  chars the line claims: `[start, start+len)` ranges in run order, no gaps,
  no overlaps.
- Wrap decision: a word goes on the current line if it fits
  (`cursor + space_width + word_width <= width`), else the line breaks
  **before the space** — the space belongs to the previous line (drawn at
  its right edge; invisible and scissor-clipped; this keeps ranges
  contiguous, which is what makes caret math trivial).
- A word wider than the whole column overhangs (never dropped), same as
  prose.rs.
- Merge adjacent pieces from the same run into one segment.
- An empty block (placeholder run) produces one empty `VisLine`.

## 4. Mappings (the editor and shell call these)

```rust
impl DocLayout {
    /// (x, baseline-y, line-height) of a model caret, relative to content top.
    fn caret_pos(&self, doc: &Document, caret: Caret,
                 measure: &dyn Fn(&str, &TextStyle) -> f32) -> (f32, f32, f32);

    /// Nearest caret position for a click at (x, y) — y relative to content top.
    fn hit(&self, doc: &Document, x: f32, y: f32,
           measure: &dyn Fn(&str, &TextStyle) -> f32) -> Caret;

    /// One visual line up/down from `caret`, aiming at `goal_x` pixels.
    /// Returns None when already at the first/last visual line.
    fn line_up(&self, doc: &Document, caret: Caret, goal_x: f32, measure: ...) -> Option<Caret>;
    fn line_down(&self, ...) -> Option<Caret>;

    /// The caret's visual line's [top, bottom) — scroll-follow reads this.
    fn caret_band(&self, doc: &Document, caret: Caret) -> (f32, f32);
}
```

Rules:

- **caret → position**: the caret belongs to the line whose range contains
  its flat offset with `line_start <= offset < line_end`, except at a
  block's very end, which belongs to the last line's end. (At a wrap
  boundary the caret goes to the *next* line's start — after typing a
  space that wraps, the caret must appear on the fresh line.) x = width of
  the line's text before the caret (walk segments; measure each; the
  segment containing the caret contributes its prefix). Style context
  never affects position.
- **hit**: block by `y` (past the last block → end of last block), line by
  `y` inside it, then walk segments accumulating width; split at each
  char's midpoint (compare the pattern in `components/editor.rs
  ::column_at`). The resulting caret's `style` context follows the
  style-before rule from task 01 (`set_caret` semantics).
- **line_up/down**: find the caret's current visual line and x; move to
  the previous/next visual line — crossing into the adjacent block at
  edges — and `hit`-test `goal_x` on that line.
- All of these take `measure` — the same one `layout` was built with.

## 5. Tests (fake measure: 10px per char, like prose.rs)

- Wrapping: a 15-word paragraph at width 100 breaks exactly where greedy
  says; segments are contiguous and re-cover the source exactly
  (concatenate segment source ranges → the whole block text).
- A word wider than the column lands on its own line, unbroken.
- Style segmentation survives wrapping: `plain **bold** plain` wraps to
  segments with correct `(inline, start, len, style)`.
- caret at a wrap boundary appears at the next line's x=0.
- caret at block end appears at the last line's end.
- `hit` round-trips: `hit(caret_pos(c))` == `c` for a spread of positions
  (mid-word, boundaries, empty block, block start/end).
- line_down through a short line keeps `goal_x` (test at the layout
  level; the shell owns the state).
- Empty document → one empty VisLine; caret_pos == (0, baseline); hit
  anywhere in it → caret (0,0,0, PLAIN).
- Headings: correct line heights and gaps; heading text uses the bold
  heading style (assert via the fake measure seeing the right
  `TextStyle.size`).

## 6. Verification

```
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

## 7. Non-goals

- Drawing anything (that's the Editor component, task 05).
- Incremental or visible-range layout. O(document) per call is the
  milestone ceiling; note it in a module doc comment.
- `prose.rs` changes. Sidenotes keep their own path for now.
