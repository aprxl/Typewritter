# AGENTS.md
- Do not preserve backward compatibility. Remove obsolete paths instead of
  adding compatibility layers, fallbacks, or migrations.
- Choose the simplest implementation that fully meets the current
  requirements. Avoid speculative abstractions, configuration, and
  indirection.
- Grow the system in layers. Start from the smallest version that works end
  to end, and add each new capability on top of a product that already
  works. Never trade a working product for unfinished complexity.
- Keep components modular and concerns clearly separated.
- Prefer established, well-maintained libraries when they reduce overall
  complexity or improve reliability. Do not reimplement common
  functionality without a clear reason.
- Lean on the dependencies already in the project before writing your own
  implementation or adding packages. Do not assume a library lacks a
  capability without checking its documentation and types.
- Make architectural decisions for the long term. Do not accept a stopgap
  that only works for now and is meant to be replaced later.

## Who writes the code

Opus reasons and designs; it does not implement. Every coding task is
delegated to a subagent — **prefer Sonnet, fall back to Haiku only for
mechanical work**. Opus may edit directly only for changes small and
obvious enough to need no real work: a doc tweak, a renamed symbol, a
one-line fix.

A delegated task is given the design, the exact files it owns, and the
verification it must pass (`cargo fmt`, `cargo clippy --all-targets` clean,
`cargo test`). Two agents never own the same file in the same wave.

Commit after every change. Small commits make history easy to follow and
easy to bisect later.

## Project

Typewritter — keyboard-driven capture tool for math-heavy lecture notes.
Vim motions, fast custom math input (no LaTeX), structural editing with no
visible Markdown. Full design spec: [`docs/SPEC.md`](docs/SPEC.md); build
order is §14.

## UI architecture

- `src/layout.rs` — geometry only. Rows/columns, `Fixed`/`Flex`, minimums,
  visibility. It measures nothing itself; minimums are pushed in.
- `src/ui.rs` — the `Component` trait and `Region`. A component measures
  itself, decides when it is dirty, and draws itself into its rect.
- `src/components/` — **one widget per file**, each owning the metrics that
  belong to it (`title_bar::HEIGHT`, `file_tree::WIDTH`), plus the geometry
  of anything it is clicked on so the hit-test and the drawing cannot drift.
- `src/shell/` — the driver: `mod.rs` wires regions to layout nodes and
  rebuilds views, `input.rs` is the keyboard and mouse, `panel.rs` is one
  collapsible region, `commands.rs` is the one table every keybinding
  lives in and what the leader-key command palette lists.
- `src/vim/` — the vim keymap core (`mod.rs`) and its motions
  (`motion.rs`): turns one key into an `Action`, with no idea a `Buffer`
  or a renderer exists. The shell applies what comes back.
- `src/theme.rs` — every colour, font, and shared paint helper. Nothing
  else names a colour.

Split a file before it becomes a place things are hidden in. A new widget
is a new file in `components/`, not another thousand lines of shell.

Two rules that hold everywhere:

- **One Atomos layer per region, `Manual` invalidation.** A region redraws
  only when its component is dirty or its rect moved. Never put unrelated
  regions on one layer. Clipping is per layer too, so a shared layer would
  mean a shared scissor.
- **Every region is scissored to its rect** (`Layer::set_clip_rect`, set by
  `Region`). Overflow is cut off, not painted over a neighbour. That is a
  safety net: components still degrade gracefully and still report a
  truthful minimum, because clipped content is invisible content.

## Platform

Built on **Atomos** (`~/.dev/Atomos`), April's own renderer/input/animation
platform. No Electron, no browser.

`src/renderer/`, `src/input.rs`, `src/frame.rs`, `src/animation.rs` are
**vendored copies** of Atomos, taken 2026-08-09. Atomos is a separate
project with its own purpose — never edit it from here. Changes to the
platform happen in this repo, on these copies.

To pull a newer Atomos: copy those files plus `RENDERER.md`, `INPUT.md`,
and `ANIMATION.md` into `docs/atomos/`, then run `cargo fmt` before
reading the diff — upstream is not rustfmt-formatted, so without that step
every pull looks like it changed everything.

API references (read before writing code against them):
[`docs/atomos/RENDERER.md`](docs/atomos/RENDERER.md),
[`docs/atomos/INPUT.md`](docs/atomos/INPUT.md),
[`docs/atomos/ANIMATION.md`](docs/atomos/ANIMATION.md).

## Verification

`cargo check` passing does not mean the renderer works — wgpu validation and
WGSL errors only surface at runtime. After any renderer change, smoke test:

```bash
timeout 6 cargo run > /tmp/typewritter_run.log 2>&1
```

Exit 124/143 means it ran clean; then grep the log for `panic|wgpu error`.

Keep the code warning-free.
