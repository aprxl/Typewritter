# Work log

State at handoff: `cargo fmt --check` clean, `cargo clippy --all-targets -D
warnings` clean, 185 tests passing, `timeout 6 ./target/release/typewritter`
exits 124 with no panic or wgpu error. Nothing is committed yet — the repo has
no commits at all, and `git commit` is blocked until a git identity is set
(`git config --global user.name`/`user.email`).

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
