# Task 05 — Editor integration (wire the Document in, delete the Buffer)

**Wave 3.** Depends on tasks 01–04 being merged. This is the payoff: the
app opens `.md` files, renders them styled and wrapped, edits them
structurally, and saves canonical Markdown.

**You own**: `src/tabs.rs`, `src/shell/mod.rs`, `src/shell/input.rs`,
`src/shell/commands.rs`, `src/vim/motion.rs`, `src/components/editor.rs`,
`src/lib.rs`, and you **delete `src/buffer.rs`**. Nothing else.

**Read first**: `tasks/00-overview.md` (pipeline §2, invariants §3),
`src/document/{mod,markdown,layout}.rs` as merged, `src/tabs.rs`,
`src/shell/mod.rs`, `src/shell/input.rs`, `src/shell/commands.rs`,
`src/vim/{mod,motion}.rs`, `src/components/editor.rs`. `WORK.md` lists
behaviors that must keep working — treat it as the regression spec.

---

## 1. Model swap

- `Tab` holds `Document` instead of `Buffer`
  (`Tab { document: Document, preview: bool }`). `Tab::load` →
  `Document::load` (binary rejection rides along).
- `Tabs`' delegation methods retarget to the `Document` ops from task 01.
  Keep the `edit` (content; promotes preview, §7.1) vs `touch`
  (caret-only) split exactly — the test
  `moving_the_caret_in_a_preview_tab_does_not_promote_it_but_typing_does`
  must still pass (rewritten against `Document`).
- `src/buffer.rs` is deleted; `src/lib.rs` drops `pub mod buffer;`.
  Any surviving behavior from its tests should already be covered by
  task 01's tests — check before deleting, port what's missing.

## 2. Editor rendering (`src/components/editor.rs`)

Rebuild the component around `DocLayout`:

- `Editor::new(layout: Rc<DocLayout>, caret: Caret, scroll: f32,
  block_caret: bool, caret_style: Style)` and `Editor::placeholder()`
  (unchanged placeholder text/behavior).
- **The filename title header is gone** (decision 3 in the overview).
  Content starts at a small top padding (`editor::TOP = 28.0`); keep
  `INSET = 56.0`; content width = `min(MEASURE, rect.width - INSET - 24.0)`
  with `MEASURE = 634.0` — the design's measure rule is unchanged.
- Draw: for each block, each visual line, each segment —
  `theme::draw` with `layout::text_style(block_kind, segment.style)`.
  Current-line highlight (`theme::ALT` band, full region width) on the
  caret's *visual* line.
- The caret reads its context (SPEC §4.2):
  - Insert mode: 2px `theme::ACCENT` bar. If `caret_style` is not plain,
    draw a tiny mono 9px marker to the bar's top-right: `B`, `I`, or `BI`
    (ACCENT). That is the entire affordance for "what you'd type next".
  - Normal mode (`block_caret`): the existing faded block over the glyph
    under the caret — keep, including the space-width fallback past EOL.
- `sync`/dirty behavior unchanged (caret blink only redraws when a file is
  open).

## 3. Shell wiring (`src/shell/mod.rs`, `input.rs`)

- **Layout cache**: the shell holds
  `doc_layout: Option<(u64 /*revision*/, f32 /*width*/, Rc<DocLayout>)>`.
  (Re)build it in `rebuild_views` with the text region's layer as the
  measure source (`theme::width(region.layer(), …)`). The Editor gets a
  clone of the same `Rc`. Width comes from the region rect (see §2).
- **Scroll-follow**: `ensure_caret_visible` now uses
  `layout.caret_band(caret)` (absolute y) instead of line-index math —
  generalize `follow_scroll` (currently in `components/editor.rs`) to take
  a y-band instead of `(line, line_height)`, and move/keep it wherever it
  stays pure and tested. `editor_max_scroll` = `layout.height - visible`.
- **Click-to-place**: `caret_at` becomes `layout.hit(...)`.
- **Vertical motion**: `j`/`k` and `ArrowUp`/`ArrowDown` go through
  `layout.line_up/line_down` with a `goal_x: Option<f32>` owned by the
  shell (set from the caret's x on the first vertical move, cleared by any
  non-vertical caret change — mirror the old `goal_col` semantics, now in
  pixels).
- **`Tabs.editor_scroll`/`set_editor_scroll`** unchanged.

### Insert mode (`edit_frame_insert`)

- Typing/backspace/delete/enter → the corresponding `Tabs` ops.
- **`Ctrl-F` prefix** (SPEC §4.4): `is_shortcut_pressed(CONTROL, KeyF)`
  arms a one-key chord (`ctrl_f: bool` on the shell). While armed, the next
  frame's chars: `b` → `toggle_bold`, `i` → `toggle_italic` (both via
  `Tabs::touch` — context isn't content), then disarm; **any other char
  inserts normally** and disarms; `Escape` disarms.
- **Esc pops one level** (§4.3): if `caret.style` is non-plain → set it
  plain (via `touch`), stay in Insert; else → Normal (existing
  `vim.key(Key::Escape)` path).
- Left/right/Home/End arrows: `move_left`/`move_right` from task 01 (the
  style-context machine — this is where §4.2 becomes real);
  Home/End = visual line start/end via the layout (`hit` at x=0 / x=∞ of
  the caret's visual line).

### Normal mode (`edit_frame_normal` / `apply`)

- `src/vim/motion.rs` is ported to `Document`: `h l w b e 0 ^ $ gg G`
  with counts, same semantics as today (see its tests — port them against
  `Document`, preserving expectations). Word motions keep their
  char-class rules; treat a block boundary as whitespace. `w`/`b`/`e`
  cross blocks. `h`/`l` cross blocks (the style machine's block-crossing
  already handles boundaries — `motion::apply` for Left/Right delegates to
  `Document::move_left/move_right`).
- `Motion::Up`/`Down`: the shell applies them via `line_up/line_down`
  (layout), not via motion.rs — motion.rs has no pixels.
- `i a I A o O x dd` map to the task-01 ops; `o`/`O` still enter Insert.
- Leader (Space) → palette: unchanged.

### Commands (`src/shell/commands.rs`)

New palette entries, group `"Format"`, `chord: None` (chords must carry
Ctrl per the table's invariant and `Ctrl+1..4` are taken by panel toggles
— so these are palette-only):

| id | title | run |
|---|---|---|
| `format.body` | "Body text" | `set_heading(None)` on the caret's block |
| `format.h1` | "Heading 1" | `set_heading(Some(1))` |
| `format.h2` | "Heading 2" | `set_heading(Some(2))` |
| `format.h3` | "Heading 3" | `set_heading(Some(3))` |

All four work in Normal and Insert. Keep the existing tests green (they
iterate the table; new rows must satisfy them).

## 4. Chrome

- Status line: word count and saved/unsaved from `Document` — unchanged
  behavior.
- Breadcrumb / tab strip: unchanged (they read `path`/`name`).
- Create (`Ctrl+N`) → empty `Document` at the new path; delete-confirm
  flow unchanged; save (`Ctrl+S`) → `Document::save` (canonical Markdown
  out).

## 5. Tests

- Port `src/vim/motion.rs` tests to `Document`; same expectations.
- Port `src/tabs.rs` tests (temp files now contain Markdown).
- New: preview-tab promotion still works through the Document path
  (content edit promotes; motion doesn't).
- Update anything that referenced `Buffer` — after this task,
  `rg "buffer::" src/` finds nothing and `src/buffer.rs` is gone.

## 6. Verification

```
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
timeout 6 cargo run > /tmp/typewritter_run.log 2>&1   # renderer-adjacent
```

Exit 124/143 = clean; grep the log for `panic|wgpu error`.

Manual acceptance (the user will run this):

1. Open a note containing headings and `**bold**`/`*italic*` → renders
   styled, wrapped, no markup visible.
2. Type inside a paragraph; bold caret appears after `Ctrl-F b`; typed
   text is bold; `Ctrl-F i` adds italic.
3. Right-arrow at the end of a bold run pops the caret out (marker
   disappears) before moving on.
4. Esc: first press clears the style marker, second enters Normal.
5. Palette → "Heading 1/2/3" / "Body text" converts the block.
6. `Ctrl+S` → the file on disk is canonical Markdown; reopening shows the
   same styled document (round-trip).
7. Vim: `h j k l w b e 0 ^ $ gg G`, counts, `x dd o O i a I A` all behave
   per the motion tests.
8. A big file still scrolls smoothly; a binary file still refuses to open.

## 7. Non-goals

Normal-mode formatting operators, dot-repeat, undo, selections, math,
chips, sidenote content, incremental layout. If a behavior not listed here
regresses versus `WORK.md`, that is a bug in this task, not a scope call.
