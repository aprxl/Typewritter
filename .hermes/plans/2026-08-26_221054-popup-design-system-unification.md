# Popup Design-System Unification Implementation Plan

> **For Hermes:** This repo's AGENTS.md forbids subagent delegation — execute every task
> inline, in this session, task by task. Do not dispatch delegate_task workers.

**Goal:** Extract the format bar's elevated-popup treatment (blurred shadow, gradient
surface, rounded corners, spring/fade entrances, sliding accent pill, dismissal ghost)
into one reusable popup toolkit, then apply it to all six remaining popups so the whole
app shares a single motion-and-surface vocabulary instead of ~5 divergent copies.

**Architecture:** New module `src/components/popup.rs` owns everything popups share:
the row-pill `Slide`, entrance timings/easings (menu-slide vs modal-fade), the shadow
slab painter, the card surface painter, and dismissal-ghost bookkeeping types. The shell
renames its format-bar-specific `bar_shadow`/`format_reveal`/`format_dismiss` machinery
into popup-generic equivalents. Each popup component keeps its own layout/content logic
and consumes the toolkit — layouts deliberately stay different per the user.

**Tech Stack:** Rust, vendored Atomos renderer (`src/renderer/`) with per-layer
`ShaderEffect::Blur`; existing `theme.rs` helpers (`elevated_popup`, `rounded_outline`,
`shadow_ink`); winit event loop with needs-based redraw scheduling.

---

## User decisions locked in clarification (2026-08-26)

| Question | Decision |
|---|---|
| Scope | **All six popups**: palette, slash menu, context menu, file finder, math menu, dialog |
| Entrance | **Modals fade** (palette, dialog); **anchored menus slide** down with the format bar's gentle overshoot (slash, context, math) |
| Exit | **Menus get the real-content fade-out ghost** (slash, context, math); **modals close instantly** (palette, dialog); file finder closes instantly |
| Modal backdrop | **Keep the current dim**, elevated card floats above it |
| List rows | **Sliding accent pill everywhere**, driven by mouse hover *and* keyboard selection through the same pill |

## Current context / assumptions

- Working tree clean at `035033a` on `master`. Test count today: **572 passing**,
  clippy `-D warnings` clean.
- Already shared in `src/theme.rs`: `shadow_ink()` (~18% dark / ~12% light alpha),
  `elevated_popup(e)` vertical-gradient surface, `rounded_outline()` /
  private `rounded_rect_path()` quarter-arc stroker (unit-tested).
- Trapped inside `src/components/format_bar.rs`: `Slide` (row-pill animator,
  lines ~249–283), `RADIUS = 9.0`, `SHADOW_SPREAD = 10.0`, `SHADOW_BLUR_RADIUS = 14.0`,
  `REVEAL_EASING` CubicBezier(0.34,1.32,0.64,1.0) @220 ms, `card_anchored()`.
- Trapped inside the shell (`src/shell/mod.rs`, `src/shell/input.rs`):
  `bar_shadow` blur layer (created after all regular regions — **that ordering is the
  sidenote-clipping fix and must be preserved**), `format_reveal: Animation`,
  `WordFormatState`, `struct FormatDismiss { items, anchor, pointer_cell }`,
  `format_dismiss_clock` driving the 140 ms reverse-curve ghost fade.
- The six target popups all still draw legacy-style:
  `layer.draw_rectangle(card.pos, card.size, theme::popup(), Rounding::NONE)` +
  square `theme::outline()` (verified in palette.rs:178, slash_menu.rs:114,
  context_menu.rs:117, file_finder.rs:79, math_menu draw, dialog.rs).
  None paints a shadow; none animates entrance/exit; none has a pill highlight.
- Overlay regions are created detached in `Shell::new`
  (dialog_region, palette_region, slash_region, finder_region, menu_region,
  math_menu_region ≈ lines 514–542) and attach/detach by open state around line 900.
- Only one popup is ever open at a time (`Context::overlay_open` is a flat disjunction),
  so **one shared entrance animation and one ghost slot suffice** — no per-popup clocks.

### Constraints carried from prior sessions (hard rules)

1. **CRLF line endings.** Edit files by reading with `newline=""`, normalizing to LF in
   memory, anchoring with assert-found-and-unique replaces, writing back with `\r\n`.
   The established vehicle is an `execute_code` block defining `rep(old, new)`.
2. **`cargo fmt` reflows the whole tree.** After every fmt run, restore untouched
   components:
   `git checkout -- src/components/breadcrumb.rs src/components/context_menu.rs src/components/editor.rs src/components/file_finder.rs src/components/onboarding.rs src/components/palette.rs src/components/slash_menu.rs src/components/status_line.rs src/components/tab_strip.rs src/components/topics.rs`
   (adjust the list per task — do NOT restore the file you just edited).
3. **Four gates before every commit:** `cargo fmt` (+ restores), `cargo clippy
   --all-targets -- -D warnings`, `cargo test` (expect 572+ passing), live smoke run.
   Smoke command (MSYS-safe):
   ```
   MSYS2_ARG_CONV_EXCL="*" powershell -Command "\$p = Start-Process cargo -ArgumentList run -NoNewWindow -PassThru -RedirectStandardError \"\$env:TEMP\tw.log\"; if (\$p.WaitForExit(90000)) { \"exited early: \$(\$p.ExitCode)\" } else { Stop-Process -Id \$p.Id -Force; \"ran clean\" }"
   grep -ciE "panic|wgpu error" "$LOCALAPPDATA/Temp/tw.log"   # expect 0
   ```
4. **Read the diff before committing.** Every defect caught so far was found reading
   diffs after green suites.
5. **Commit after every task.** Small history.
6. **AGENTS.md:** simplest implementation, no compatibility layers, no speculative
   abstraction, delete obsolete paths rather than shimming them.
7. Frame-loop discipline: any new playing-state (`Animation::new` starts *playing*)
   must be gated so idle popups report `is_animating() == false` — see the
   closed-bar frame-leak fix (`self.open && self.slide.advancing()` pattern).

---

## Step-by-step plan

### Task 1: Create the popup toolkit (`src/components/popup.rs`)

**Objective:** One file owning everything popups share, extracted verbatim from
`format_bar.rs` plus the two new entrance styles.

**Files:**
- Create: `src/components/popup.rs`
- Modify: `src/lib.rs` or `src/components/mod.rs` (add `pub mod popup;`)
- Modify: `src/components/format_bar.rs` (consume toolkit, delete duplicates)

**Contents (extracted, not redesigned):**
- `pub const CARD_RADIUS: f32 = 9.0;`, `pub const SHADOW_SPREAD: f32 = 10.0;`,
  `pub const SHADOW_BLUR_RADIUS: f32 = 14.0;` (moved from format_bar; format_bar
  re-imports; update `Shell::new` blur radius reference accordingly).
- `pub const MENU_SLIDE_DURATION: Duration` (220 ms),
  `pub const MENU_SLIDE_EASING: Easing` = the CubicBezier overshoot curve
  (renamed from `REVEAL_EASING`),
  `pub const MODAL_FADE_DURATION: Duration` (180 ms),
  `pub const GHOST_DURATION: Duration` (140 ms, renamed from `DISMISS_DURATION`).
- `pub struct Slide` moved whole from format_bar (~lines 249–283) with its `park`,
  `slide_to -> bool`, `advancing`, `value/rect` accessors — pure code move plus unit
  tests (park ≠ slide, slide_to same-rect returns false, advancing goes false at end).
- `pub fn paint_card(layer: &Layer, card: Rect, e: f32)` — draws the elevated surface:
  `theme::elevated_popup(e)` fill via `draw_path`/rounded-rect fill using
  `rounded_rect_path` data (needs a filled path, not just the stroke — extend
  `theme.rs` with `rounded_rect_fill` alongside `rounded_outline` if a fill helper
  doesn't exist yet; keep it in theme.rs since it is a paint primitive) + stroke via
  `theme::rounded_outline(layer, card, CARD_RADIUS, 1.0, theme::border())`.
- `pub fn paint_shadow_slab(shadow_layer: &Layer, card: Rect, e: f32)` — the
  clear-slab-draw dance from `FormatBar::paint_shadow` (clear first, then slab at
  `theme::fade(theme::shadow_ink(), e)` with `Rounding::uniform(CARD_RADIUS +
  SHADOW_SPREAD)`, spread `SHADOW_SPREAD` on every side). Must ALWAYS clear even when
  returning early — carry the frozen-halo trap comment over.
- `pub fn fade_clamped(color: Color, e: f32) -> Color` wrapping the existing
  clamp-then-`theme::fade` behavior used for overshooting spring values.

**TDD steps:** write the `Slide` unit tests first (adapt format_bar's existing
pill-persistence test locations), watch fail (module absent), move code, watch pass.

**Verify:** `cargo test` — all previous tests pass unchanged (pure move);
`grep -c "struct Slide" src/components/format_bar.rs` → 0.

**Commit:** `refactor(ui): extract shared popup toolkit from format bar`

### Task 2: Genericize the shell plumbing

**Objective:** Rename the format-bar-owned machinery into popup-generic equivalents;
attach the shadow layer to every overlay snapshot.

**Files:**
- Modify: `src/shell/mod.rs`
- Modify: `src/shell/input.rs`

**Steps:**
1. `bar_shadow` → `popup_shadow` (field, `Shell::new` creation site — keep the
   created-after-all-regions comment; `radius` now references
   `popup::SHADOW_BLUR_RADIUS`).
2. `format_reveal` → `popup_reveal`. Advance condition becomes
   `if self.any_popup_open() || self.popup_reveal.is_playing()`. Restart
   (`restart()`) from whichever input handler opens a popup. Motion style is read by
   the component: modal components interpret the weight as pure opacity (clamped),
   menu components as the overshoot slide — the shell stays unaware of which style
   applies.
3. **Menu-type entrance flag:** add `enum PopupKind { Format, Slash, Context, Math }`;
   opening any popup records its kind. `Context::reveal` keeps its current meaning
   (dismiss clock during a flight, weight otherwise).
4. Attach `.with_shadow(self.popup_shadow.clone())` to EVERY overlay region snapshot
   built in `input.rs` (find all `set_component` sites for palette/slash/context/
   finder/math/dialog snapshots), not just the format bar's. Each component will later
   call `paint_shadow_slab` on the layer it received.
5. Keep the existing pattern: closed snapshots ALSO carry the layer so a stale halo can
   always be cleared.

**Verify:** `cargo check` fast loop first, then gates. Manual smoke: open palette —
no visible change beyond nothing (its draw isn't migrated yet), confirm format bar
still fully works (regression).

**Commit:** `refactor(shell): popup-generic reveal, shadow layer, kinds`

### Task 3: Ghost machinery for menus (slash, context, math)

**Objective:** Generalize `FormatDismiss` so any menu can fall away as a ghost of its
real content; modals opt out entirely.

**Files:**
- Modify: `src/shell/mod.rs` — `FormatDismiss` →
  ```
  enum MenuDismiss {
      Format { items: Vec<format_bar::Item>, anchor: (f32,f32), pointer_cell: Option<usize> },
      Slash { query: String-ish state clone, anchor },
      Context { entries clone, anchor, selected },
      Math { offers clone, anchor, selected },
  }
  ```
  (`input.rs` builds the variant at close time from pre-take() state — replicate the
  capture-before-take ordering lesson; format arm migrates its existing fields.)
- Modify: `src/shell/input.rs` — close handlers for the three menus spawn ghosts the
  way `close_format_bar` does; modal close paths untouched (instant).
- Each menu component gains a `dismissing(state_clone, anchor, ...)` constructor that
  renders one last faded frame from captured state — mirroring
  `FormatBar::dismissing`. Components don't tick the clock; the shell's
  `format_dismiss_clock` (rename: `menu_dismiss_clock`) drives all variants identically
  through `Context::reveal`.

**TDD:** unit-test each variant constructor captures the passed state (format bar's
real-icons-ghost bug class — placeholders are banned; a variant must carry clones of
REAL data or nothing).

**Commit:** `feat(shell): dismissal ghosts for anchored menus`

### Task 4: Command palette — modal fade + pill rows

**Objective:** Elevated floating card over the dimmed backdrop; 180 ms fade-in
(clamped, no overshoot); instant close; sliding accent pill shared by keyboard
selection and pointer hover.

**Files:**
- Modify: `src/components/palette.rs` (~340 lines): replace flat
  `draw_rectangle(theme::popup()) + outline` with `popup::paint_card`; dim pass stays;
  card grows soft shoulders; row highlight replaced by a `popup::Slide` pill fed by
  `max(hover_row, keyboard_selected)` semantics — keyboard moves set the same target;
  dirty tracking follows pill advances; `sync` reads reveal as opacity
  (`theme::fade`-style alpha applied to card paint via the clamped fade helper) and
  reports `is_animating` honestly (weight-moving OR pill-advancing AND open).
- Modify: `src/shell/input.rs` palette snapshot builders: attach shadow layer, feed
  selection changes through existing revision flow.

**Tests:** palette geometry/hit-test tests updated for radii; new test — keyboard
selection change slides the pill (target equals new row before animation completes).

**Commit:** `feat(ui): palette adopts popup toolkit — fade, elevation, pill rows`

### Task 5: Dialog — modal fade + rounded fields

**Objective:** Same modal vocabulary for rename/new-file dialogs; input fields get
matching rounded outlines instead of squares.

**Files:**
- Modify: `src/components/dialog.rs` (~254): `popup::paint_card` surface; fade entrance
  driven by `popup_reveal` opacity; field boxes drawn with
  `rounded_rect` fill + `rounded_outline` (radius ~6, smaller than card's 9 — nest
  smaller inside larger); caret/typing behavior untouched.
- Modify: `src/shell/input.rs` dialog snapshots: attach shadow layer.

**Tests:** existing dialog input tests pass; add field-rounding geometry parity test
only if cheap — otherwise visual smoke only.

**Commit:** `feat(ui): dialog adopts popup toolkit`

### Task 6: File finder — elevated list, pill rows

**Objective:** Finder panel adopts surface + pill (it is a tall anchored list but the
user grouped closes as instant; entrance = fade like other non-springing overlays —
matches "large panel, heavy slide reads sluggish").

**Files:**
- Modify: `src/components/file_finder.rs` (~242): `popup::paint_card`; `popup::Slide`
  pill for hovered/selected row; fade entrance.
- Modify: `src/shell/input.rs` finder snapshots: attach shadow layer.

**Commit:** `feat(ui): file finder adopts popup toolkit`

### Task 7: Slash menu — spring slide + ghost + pill

**Objective:** First full-vocabulary menu: slide-down entrance with overshoot, sliding
pill, real-content ghost on close.

**Files:**
- Modify: `src/components/slash_menu.rs` (~290): `popup::paint_card`; reveal weight
  drives `revealed_card`-style translate DOWN (reuse/adapt format_bar's
  `card_anchored`+offset pattern into `popup::slid_card(base, e)`); pill rows;
  `dismissing` constructor rendering from captured `Slash` ghost state.
- Modify: `src/shell/input.rs` slash open/close: restart `popup_reveal` on open; spawn
  ghost on close.

**Commit:** `feat(ui): slash menu adopts popup toolkit`

### Task 8: Context menu — spring slide + ghost + pill (checkmarks kept)

**Files:** mirror Task 7 onto `src/components/context_menu.rs` (~220); checked-item
check glyphs recolored against pill when overlapping (accent ring idiom from format
bar cells).

**Commit:** `feat(ui): context menu adopts popup toolkit`

### Task 9: Math menu — spring slide + ghost + pill

**Files:** mirror Task 7 onto `src/components/math_menu.rs` (~312). Its segmented card
(`card_anchored` with variant offsets) composes with `popup::slid_card`.

**Commit:** `feat(ui): math menu adopts popup toolkit`

### Task 10: Sweep, docs, retire stragglers

**Objective:** No orphaned names, documented conventions.

**Steps:**
1. `grep -rn "bar_shadow\|format_reveal\|FormatDismiss\|DISMISS_DURATION\|REVEAL_EASING"`
   → zero hits outside history.
2. `format_bar.rs` sheds any helper now duplicated by popup.rs (Slide import, consts).
3. Patch project skill `typewritter`: record the popup toolkit contract (components
   paint slab through the attached layer always-clear rule; ghost variants carry REAL
   state; motion table: modals fade / menus slide; pill gating rule for
   `is_animating`).
4. Full four gates + READ THE COMPLETE DIFF of tasks 4–9 surfaces for drift between
   popups (consistent radii, spacings, dim values — the point of the exercise).

**Commit:** `docs(skill): popup toolkit conventions`

---

## Files likely to change (summary)

- Create: `src/components/popup.rs`
- Heavily modify: `src/shell/mod.rs`, `src/shell/input.rs`
- Modify: `src/components/{format_bar,palette,slash_menu,context_menu,file_finder,math_menu,dialog}.rs`
- Maybe modify: `src/theme.rs` (only if a rounded FILL helper is missing),
  `src/components/mod.rs` (module registration)

## Tests / validation

- Per-task: targeted `cargo test <name>` first failing, then passing.
- Global gates after every task: fmt(+restores), clippy `-D warnings`, `cargo test`
  (572+ passing), smoke run grep `panic|wgpu error` = 0.
- Visual acceptance checklist (manual, after Tasks 4–7 and again at the end):
  each popup opens with its designated motion, floats above sidenotes margin (shadow
  not clipped — the ordering regression), ghost falls away with real content, pill
  follows BOTH mouse and arrow keys, closing leaves no frame-loop drain (idle CPU flat
  — can spot-check via frametime log or long-idle smoke).

## Risks, tradeoffs, open questions

- **Shared-reveal bottleneck:** one global `popup_reveal` assumes strictly one popup at
  a time. True today; if a future feature pops two simultaneously this needs a small
  map. Accepted (YAGNI, matches `overlay_open` disjunction).
- **Fade-with-overlap on shells:** modal fade applies alpha to text-heavy cards;
  glyphon text alpha behaves well (editor glow precedent), but check palette query text
  legibility mid-fade during review; fallback is fading the card surface while text
  uses step-end.
- **Ghost capture cost:** cloning slash/context/math entry vectors on every close is
  O(rows); rows are ≤ dozens — accepted. Never capture lazily AFTER take().
- **File finder motion classification** (faded, chosen here as call) — flag at review;
  flipping it to slide later is a one-line easing swap thanks to the toolkit.
- **Regression risk hot spots:** `with_shadow` attach points in `input.rs` (missed
  site = frozen halo for that popup — grep-audit in Task 10), and CRLF/fmt restores
  (process risk, mitigated by constraint block above).

## Execution handoff

Per AGENTS.md, all nine tasks execute inline in this session, sequentially — no
subagent dispatch despite the plan skill's default suggestion. Say the word and I
start with Task 1.
