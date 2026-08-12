# Tasks 00 — AST-structural editing: architecture and build order

This is the milestone where Typewritter stops being a plain-text file editor
and becomes a structural editor. Scope: **paragraphs, headings (levels 1–3),
bold, italic**, fully rendered, saving to Markdown. Everything else (math,
chips, sidenotes, tables, code blocks) plugs into the same seams later, so
the seams are the point of this document.

Read first: `docs/SPEC.md` (§1, §4, §9), `AGENTS.md`, `src/buffer.rs`,
`src/tabs.rs`, `src/shell/mod.rs`, `src/shell/input.rs`, `src/shell/commands.rs`,
`src/vim/mod.rs`, `src/vim/motion.rs`, `src/components/editor.rs`,
`src/prose.rs`, `src/theme.rs`, `src/renderer/glyph_effects.rs`.

---

## 1. Decisions already made (do not revisit)

1. **The rope-backed `Buffer` is replaced, not wrapped.** The new
   `Document` AST owns text as plain `String`s inside inline runs. The
   rope's bounded-chunk inserts never pay off at paragraph scale, and
   AGENTS.md forbids compatibility layers. `src/buffer.rs` is deleted in
   the integration task (05). User-approved.
2. **Normal-mode formatting (`<leader>biw`) is deferred** to the operator
   milestone (operator-pending `d`/`c`/`y` × motion is already a known gap
   in `WORK.md`; bold-by-operator lands with it). This milestone's
   formatting entry points are Insert-mode `Ctrl-F b`/`Ctrl-F i` and palette
   commands for block styles (headings/body). User-approved.
3. **The editor pane's filename title is dropped.** The document's own
   `# Heading` is the title; the name lives in the tab strip and
   breadcrumb. User-approved.
4. **Italic is synthesized by the renderer.** Only Georgia Regular is
   installed (`fc-match "Georgia:italic"` → Regular). The vendored
   renderer already synthesizes faux-bold and faux-condense through swash
   (`src/renderer/glyph_effects.rs`); italic joins as a third synthetic
   effect (a shear transform). Per AGENTS.md, editing the vendored copies
   *in this repo* is allowed — upstream `~/.dev/Atomos` is never touched.

## 2. The pipeline

```
disk (.md) ──parse──▶ Document (AST) ◀──ops── vim Actions / mouse / palette
                         │
                         ▼ layout(width, measure)
                      DocLayout (visual lines, styled segments)
                         │
                         ▼ draw into Editor's Region layer
```

- **Load**: `fs::read` → binary check (NUL byte) → `markdown::parse` →
  `Document`.
- **Edit**: `Input` → shell precedence (palette → dialog → onboarding →
  command table → vim) → `vim::Action` → `Document` op → `Tabs::edit`
  (content change; promotes preview tab) or `Tabs::touch` (caret-only).
- **Save** (`Ctrl+S`): `markdown::serialize` → `fs::write`. Deterministic
  and canonical (§9): one true way to write any document.
- **Render**: `Tabs::revision` bumps → `Shell::rebuild_views` rebuilds a
  cached `DocLayout` (keyed by revision + content width) → `Editor` draws
  it. The model never knows about pixels; the layout never mutates the
  model.

## 3. Invariants (the foundation's load-bearing rules)

1. **Markup is never visible** (§4.1). Nothing the user types is a format
   trigger; `#5` stays `#5`. Structure comes from commands, not syntax.
2. **The AST is the source of truth; Markdown is a file format** (§1).
   No editing operation ever manipulates Markdown text directly.
3. **Serialization is deterministic.** `parse ∘ print` is identity on ASTs;
   `print ∘ parse ∘ print` is byte-identical to `print`. Property-tested.
4. **`Document.blocks` is never empty** (an empty document is one empty
   paragraph). Every block has ≥1 inline run; a run's text is empty only
   when it is the sole placeholder run of an empty block.
5. **The caret carries a style context** (`Caret.style`) — what typed text
   becomes (§4.2). The caret renders its context.
6. **Esc pops one level** (§4.3): active style context → plain; then
   Insert → Normal. First slice of the rule; more levels (list item, math
   slot) attach later.
7. **Backspace always deletes a character** (§12.3), never formatting.
8. **One Atomos layer per region, `Manual` invalidation; every region
   scissored to its rect** — unchanged from the existing shell rules.

## 4. Target module map

```
src/document/
  mod.rs       — Document, Block, Inline, Style, Caret; editing ops; style
                 context machine. Pure: no IO, no renderer.  (task 01)
  markdown.rs  — parse + serialize; Document::load / Document::save. (task 02)
  layout.rs    — DocLayout: wrap + caret↔visual mapping, fake-measure tests.
                 (task 04)
src/renderer/  — FontParameters.slant + rasterize shear        (task 03)
src/theme.rs   — TextStyle.slant + TextStyle::italic()         (task 03)
src/components/editor.rs — renders DocLayout; styled caret     (task 05)
src/vim/motion.rs        — motions ported to Document          (task 05)
src/tabs.rs              — Tab holds Document                  (task 05)
src/buffer.rs            — DELETED                             (task 05)
```

`src/prose.rs` stays as-is (sidenotes mock uses it). The document layout
does *not* reuse `prose::Paragraph`: prose collapses whitespace and its
placements carry no source offsets, both fatal to caret mapping. If
sidenotes later render document content, converge then.

## 5. Extensibility seams (why these shapes)

- `Inline` is an enum with one variant today (`Text`). Chips, footnote
  refs, and inline math become new variants; layout, serializer, and caret
  machine each gain one match arm.
- `Block` is an enum (`Paragraph`, `Heading`). Lists, code blocks, math
  blocks, tables become new variants with their own layout/serialization
  arms.
- `Style` is a struct of bools (`bold`, `italic`). Strikethrough, code,
  highlight add fields. `Style::PLAIN` is the zero value.
- `Caret { block, inline, offset, style }` is the embryo of the spec's
  node-path cursor (§3.3). When the math tree lands, `inline` becomes a
  path; the style-context machine is unchanged.
- The command table (`src/shell/commands.rs`) is where every new command
  goes; every chord carries Ctrl, so commands can never collide with vim.

## 6. Build order (waves; no two tasks in a wave own the same file)

| Wave | Task | Owns |
|---|---|---|
| 1 | `01-document-model.md` | `src/document/mod.rs`, `src/lib.rs` (+ stub `markdown.rs`, `layout.rs` so wave 2 never touches `mod.rs`) |
| 1 ‖ | `03-faux-italic-renderer.md` | `src/renderer/font_parameters.rs`, `src/renderer/glyph_effects.rs`, `src/renderer/text_stack.rs`, `src/theme.rs` |
| 2 | `02-markdown-io.md` | `src/document/markdown.rs` |
| 2 ‖ | `04-document-layout.md` | `src/document/layout.rs` |
| 3 | `05-editor-integration.md` | `src/tabs.rs`, `src/shell/{mod,input,commands}.rs`, `src/vim/motion.rs`, `src/components/editor.rs`, `src/lib.rs`, deletes `src/buffer.rs` |

Each task ends with: `cargo fmt`, `cargo clippy --all-targets -- -D warnings`,
`cargo test` all clean. Tasks touching the renderer (03, 05) also run the
smoke test:

```bash
timeout 6 cargo run > /tmp/typewritter_run.log 2>&1
```

Exit 124/143 = ran clean; then grep the log for `panic|wgpu error`.

## 7. Non-goals for this milestone

- Normal-mode formatting operators (`d`/`c`/`y`/`<leader>` × motion),
  dot-repeat, selections, visual mode.
- Undo/redo (`u`/`Ctrl-r`).
- Inline math, chips, sidenote content, code blocks, tables, folds, images.
- Search, peek, git sync, PDF export.
- Incremental/virtualized layout for huge documents (layout is O(document)
  per change — fine for notes; the visible-range optimization is its own
  task when it becomes measurable).
- Undo of the `Tabs`-level preview semantics — unchanged, keep working.
