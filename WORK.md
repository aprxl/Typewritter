# Work log

State at handoff: `cargo fmt --check` clean, `cargo clippy --all-targets -D
warnings` clean, 205 tests passing, and the tree committed as `d5caaa1`
(`Initial commit: Typewritter, plus today's bug-fix wave`).

---

## Session log — current work

**`src/document/mod.rs`, `src/document/layout.rs`, `src/document/markdown.rs`**
— line dividers. `Block::Divider` holds one always-empty run, satisfying the
"every block has a run" invariant. Typing on a rule turns it back into a
paragraph, enforced centrally in `prune_runs` rather than at each edit site.
Divider metrics are deliberately tighter than a blank line: a big hole
disrupts reading more than the separation is worth. Markdown writes `---`,
with an escape guard for a paragraph that would re-read as a rule.

**`arboard`, `src/shell/`** — OS clipboard. `arboard` was chosen over pulling
in a second UI toolkit. All clipboard I/O lives in the shell, so document and
tab models stay pure and their tests stay headless. The yank register exports
once per frame by comparing state, covering every yank path without a hook at
each one.

**`src/shell/`, `src/components/`** — right-click menus. Research first:
winit has no menu API, and `muda` (the standard cross-platform menu crate)
needs GTK3 and libxdo on Linux, which would link a second UI toolkit into an
app that draws its own title bar. The menu is drawn in-app on the same
detached-overlay pattern as the palette and slash menu; the clipboard goes
through the OS, which is the part that genuinely has to interoperate. Menus
now cover the editor, a file-tree row, and empty tree space.

**`src/document/outline.rs`, `src/components/editor.rs`, `src/components/topics.rs`**
— document outline. One pure function has three consumers: editor margin
numbers, Topics panel, and breadcrumb trail. All derive from the same blocks,
so a number can never disagree with the heading beside it. Headings nest by
insertion order rather than level arithmetic, so a document that opens at H3
or skips a level still comes out sensible. Auto-numbers are virtual: hung in
the margin and drawn rather than laid out, so the caret can never reach one
and it never shifts the heading it labels.

**`MATH.md`, `src/document/math.rs`, `src/document/math_layout.rs`,
`src/document/math_notation.rs`, `src/document/layout.rs`, `src/shell/`** —
math mode. The design lives in `MATH.md`; this log does not restate it. The
math tree uses single-character atoms in the TeX hlist model, so the cursor is
an index and never a string offset. It has cursor and edit operations, `/`
trigger operand capture, and structural revert on backspace. Recursive box
layout makes nesting work because a nested fraction is just a tall child its
ancestors grow to hold. Canonical linear notation is deterministic and
property-tested both ways. In containers, an inline atom costs exactly one
character in flat-text space, so existing motion and selection work unchanged.
Real drawing and content-driven line heights are in place. Shell wiring makes
`/math`, the fraction trigger, Tab between slots, and Esc popping levels work
in the running app. JuliaMono is the math font, chosen for glyph coverage.
Verified end to end in the app: typing `1 / 2 / 3` builds a nested fraction,
saves as `gain is $(19)/(2/3)$`, and reopens as the same tree.

**Next for math** — `^` and `_` scripts, brackets and groups, the in-math
completion palette, word triggers (`sqrt`, `sum`, `int`, `lim`), typed
identifiers, and the raw text node.

---

## Session log

**`src/markdown.rs`, `src/document/mod.rs`, `src/document/layout.rs`,
`src/theme.rs`** — `####` headings now parse as level-4 headings (`#{1,4}`),
render at 30px with a 17.0 font size. `#####` stays a paragraph.

**`src/theme.rs`** — faux-bold weight cut from `size * 0.045` to `size * 0.025`
(`TextStyle::bold()` and the resize path). Bold text reads as heavier without
the ~1.6px outline growth that looked like a font-size jump.

**`src/document/mod.rs`** — `prune_runs` no longer collapses an emptied
Heading back to a plain Paragraph placeholder; `set_heading` on an empty line
survives and typing into it stays a heading.

**`src/document/mod.rs`** — fixed `insert_text` computing the caret against a
zero-length trash run created by the splice at a run edge. The caret is now
re-derived from the flat `flat + len` *after* `enforce()` prunes, so bold/italic
insert no longer shifts the caret back a whole run.

**`src/shell/input.rs`** — Esc popping a pending B/I style context now routes
through `Tabs::touch` (bumps revision), so the B/I marker disappears the same
frame instead of lingering until the next keystroke.

**`src/vim/mod.rs`, `src/vim/motion.rs`, `src/shell/input.rs`** — new
`Motion::Append` clamps to the current block's end; `a`/`A` no longer jump to
the next line at EOL.

**`src/input.rs`** — Ctrl/Alt/Super + key no longer generates typed text;
Shift alone still types (`SUPPRESS_TEXT_MODS` = CONTROL|ALT|SUPER).

**`src/document/mod.rs`** — `delete_char` (vim `x`) rewritten from run-local
(`o >= run_len` no-op at a run's end) to flat-position based, so it deletes the
char after the caret across a plain→bold run boundary. The caret now eats into
a bold block normally instead of being stranded at the boundary.

New regression tests added alongside each fix.

---

## Session log — badges and highlights

**`src/document/mod.rs`** — `Style` gained `badge` and `highlight` flags plus
`Style::is_boxed()` (true for `code` and `badge`: runs that draw their own box
and so own their whole extent). `toggle_bold`/`toggle_italic` early-return on a
boxed context; `toggle_code`/`toggle_badge` replace the whole pending style
rather than stacking onto it; `toggle_highlight` is ignored on a boxed context.
Badges are a style on a text run, not a new `Inline` variant, so the run's own
text is the label and typing, caret motion, wrapping, deletion and vim motions
all work unchanged.

**`src/document/markdown.rs`** — grammar for `[[BADGE]]` (verbatim label, like
a code span) and `==highlight==`. `==` is the one recursive marker: `parse_inline`
recurses into the body and distributes `highlight` over the returned runs,
skipping boxed ones, so emphasis nests inside a highlight instead of being
swallowed. `serialize_runs` groups adjacent highlighted runs under one `==`
pair — re-opening at every emphasis change both reads badly on disk and parses
back as several marks. Escaping is narrow: only the doubled forms `==` and `[[`
are markers, so only those get escaped.

**`src/document/layout.rs`** — new public `advance(text, block, style, measure)`,
the single seam that puts a badge's box into the flow (adds `BADGE_PAD * 2`).
Used by `piece_width` (wrap) and `x_of_flat` (caret); the left pad is also added
in `x_of_flat`'s prefix branch and in `caret_for_click` either side of the
per-char loop. Fixes badges clipping into the text after them — the box was
originally excluded from the advance on purpose, which was wrong.

**`src/theme.rs`** — `BADGE_SIZE`/`BADGE_INK`/`BADGE_PAD`/`BADGE_HEIGHT` and
`HIGHLIGHT` live here rather than in the editor component, since the document
layout needs the metrics too.

**`src/components/editor.rs`** — badge outlines, the highlight underline bar,
and the glow halo. `spans()` merges adjacent decorated pieces so a mark split
across runs reads as one. The editor mirrors `layout::advance`'s arithmetic on
its own draw cursor, so box, caret and following text cannot drift.

**`src/shell/mod.rs`** — the highlight glow is a bare `Layer` (not a `Region`)
carrying `ShaderEffect::Blur`, composited directly above the editor. `Editor`
holds a cloned handle and clears/clips it itself, so it can never hold a halo
for a bar that is no longer there.

**`src/shell/commands.rs`, `src/tabs.rs`** — `format.badge` and
`format.highlight` commands (auto-appear in the `/` menu), plus caret-only
`Tabs` wrappers that never promote a preview tab.

---

## Session log — memory audit and fixes

Audit first: a 40-line control app (winit window + wgpu surface + device, zero
textures, zero draws) measures **161 MB RSS / 209 MiB VRAM**. That is the
NVIDIA/Vulkan driver floor, and it is most of what the process reports.
Typewritter's own heap is ~19 MB. Two useful facts fell out: `nvidia-smi`
reports VRAM in **256 MiB granules**, so it is a poor benchmark for small
changes; and `FontSystem::new()` costs **1.3 MB, not 100 MB** — fontdb 0.23
drops the mmap after parsing. Full write-up in `~/.dev/Atomos/FEEDBACK.md`.

Measured effect of the three fixes below, idle, release, same window:
VRAM 471 MiB → 215 MiB, RSS 240 MB → 230 MB, idle CPU 8.6% → 2.1%.

**`src/ui.rs`** — `Region::layer` is now `Option<Layer>`, with `detached()`,
`attach()`, `detach()` and `is_attached()`. `update` and `measure_into` no-op
without a layer. Every layer is a full-surface render target whether or not
anything is drawn into it, so a permanently-closed overlay was pure dead VRAM.

**`src/shell/mod.rs`** — the four overlay regions (onboarding, dialog, palette,
slash menu) start detached; `sync_overlay_layers` attaches on open and drops on
close, each frame after input. Dropping the handle is the whole deallocation —
the renderer's stack holds only a `Weak`. Note the behaviour change: a
re-attached layer goes back on *top*, so overlay stacking now follows what was
opened last rather than construction order.

**`src/shell/stepped.rs`** (new) — `Stepped` wraps `Animation`, quantises the
weight into N steps, and `advance()` reports a *step change* instead of "still
running". `Animation::advance` returns `self.playing`, which for `Repeat::Loop`
and `Repeat::PingPong` is permanently true, so the looping caret and pulse meant
`ControlFlow::Wait` was unreachable and the app rendered continuously forever.
`wake_in()` says when the next step is due, since an animation that falls silent
must supply its own wake or it freezes. It keeps its own wall clock rather than
taking the frame `dt`: `FrameScheduler::animation_delta` clamps to 100 ms, and
something that sleeps 525 ms on purpose cannot be driven by clamped time.

**`src/shell/mod.rs`** — caret is `Stepped` with 2 steps (on/off), pulse with 16
(`Topics` already rounds it to sixteenths, so that is every value that reaches
the screen). The pulse had to be converted too, otherwise `animating` never goes
false and the caret change is unmeasurable. Pulse is why idle is 2.1% and not
~0%: it still wakes ~13×/sec. New `Shell::wake_at()`.

**`src/main.rs`** — `about_to_wait` turns `Shell::wake_at()` into a
`ControlFlow::WaitUntil`, but only in place of `Wait`; anything owed now must
not be delayed behind an animation. When the deadline passes it requests the
redraw and clears the wake — without that, `WaitUntil` gets a past instant every
iteration and the loop spins at 100% CPU (it did, first try). Also
`trim_vulkan_drivers()`, which sets `VK_LOADER_DRIVERS_DISABLE` before wgpu
builds its instance: the loader opens every ICD manifest on the machine, not
just the one it uses. Only drivers that can never be the right answer here are
disabled (`lvp`, `virtio`, `gfxstream`, `hasvk`, `asahi`) — worth ~8 MB. An
explicit choice in the environment always wins.

Open, deliberately not done: the highlight glow layer still costs three
full-surface textures for a 2.5px halo (`ponytail:` note at its `set_effect`
call); pausing the pulse when nothing is live would take idle CPU near zero, but
that is a product decision. The larger renderer-side items — rect-sized layer
textures, the measure-only shape cache, `EffectGlyphs` eviction — are Atomos's
and are ranked in `~/.dev/Atomos/FEEDBACK.md`.

---

## Session log — bug fixes from classroom testing

**`src/vault.rs`, `src/components/dialog.rs`, `src/shell/commands.rs`,
`src/shell/input.rs`** — folders. There was no way to create one and no way to
put a note inside one: `create_note` refused any name containing `/` and always
wrote to the vault root. Now `Ctrl+Shift+N` opens a `Prompt::NewFolder`, a new
note lands in whatever folder is selected in the tree (the parent folder when a
file is selected, the vault root when nothing is), and a typed name may be a
relative path such as `math/lecture 3` whose parents are created on the way.
Absolute paths and `..` are refused in `resolve_new_path`. `Vault::refresh` used
to re-read only the top level and throw away every expanded folder, so a file
created inside one was invisible; it now merges the fresh listing with the old
expansion state, and the new `Vault::reveal` expands every ancestor of whatever
was just created.

**`src/components/editor.rs`** — the current-line band was drawn inside
`if self.caret_on`, so the whole line highlight blinked along with the caret.
The band is the line indicator and is now drawn every frame; only the caret bar
stays blink-gated.

**`src/components/editor.rs`, `src/shell/mod.rs`** — the editor could not
scroll past the end of the file, which left the last line stuck against the
bottom edge of the window. New `editor::max_scroll` adds `OVERSCROLL` (half the
visible height) of empty space past the last line, and `Shell::editor_max_scroll`
calls it.

**`src/main.rs`** — random black flashes lasting one to three frames. A
compositor can report a zero-sized window mid-resize; solving the layout against
it collapses every region's rect, and `Region::update` then clears every layer
without drawing into it, leaving the renderer's clear colour on screen.
`App::frame` now returns early on a degenerate viewport, keeping the last good
frame.

**`src/main.rs`, `src/input.rs`** — dead keys and system input methods produced
nothing on macOS. winit only sends `WindowEvent::Ime` when IME is allowed and it
is off by default; its own documentation notes that on macOS IME must be enabled
for dead-key sequences to combine at all. The window now calls
`set_ime_allowed(true)` and `Ime::Commit` folds into the same per-frame `text()`
buffer. winit delivers no `KeyboardInput` during a preedit, so a commit cannot
double up with typed text. Not yet drawn: the preedit itself, so a sequence in
flight is invisible until it commits.

**`src/shell/mod.rs`, `src/shell/input.rs`** — only the file tree could be
drag-resized. `Shell::divider()` became `dividers()`/`divider_at()` over a new
`Divider` enum, and `dragging` went from `bool` to `Option<Divider>`. The topics
outline and the sidenote margin are measured from their own right edge
(`dragged_width`), since they grow leftwards, while the tree still grows
rightwards. `divider_hot` still means the tree's divider specifically, because
that is what the file tree draws its accent rule from.

Every change is covered by `cargo test` (205 passing), and the folder creation,
the non-blinking band and the IME path were additionally verified against the
running app by injecting real key events through `/dev/uinput` and reading the
saved Markdown back off disk — but the two mouse-driven paths (wheel overscroll
and divider dragging) rest on unit tests only, because pointer injection could
not be made to reach the app under this compositor.
