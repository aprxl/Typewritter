//! The shared popup vocabulary: everything a floating card needs that has
//! nothing to do with *what* floats.
//!
//! Every popup in this app — the format bar, the command palette, the slash
//! menu, the context menu, the file finder, the math menu, dialogs — draws
//! the same way at its edges and differs only in what it puts inside:
//! an elevated surface with soft shoulders, a blurred drop shadow painted
//! into a shell-owned layer, one motion for entrances (menus slide with a
//! gentle overshoot, large modals fade), and a hover pill that slides
//! between rows or cells rather than teleporting. This module owns those
//! shared pieces; each component keeps its own layout, hit-testing, and
//! content drawing.
//!
//! Two rules hold everywhere, learned the hard way:
//!
//! - **The shadow slab is always cleared first.** Shader effects apply per
//!   layer, so shadows live in an extra blurred layer the shell creates
//!   once and hands to every snapshot. A snapshot replaced mid-flight must
//!   still clear it, or the last halo freezes on screen forever.
//! - **A never-advanced animation is a playing animation.** `Animation::new`
//!   starts playing; any component whose sync returns early on some state
//!   (a closed popup) must not report `is_animating` while in that state,
//!   or it pins the frame loop open with a redraw every frame.

use std::time::Duration;

use crate::animation::{Animation, Easing};
use crate::layout::Rect;
use crate::renderer::Layer;
use crate::theme;

/// Corner radius of every floating card — a popup reads as modern when its
/// shoulders are soft rather than square. One number across all popups, so
/// two cards on screen never disagree about how round they are.
pub const CARD_RADIUS: f32 = 14.0;

/// How far the shadow slab spreads past the resting card on every side,
/// before its blur. Generous on purpose: the halo must fade to nothing
/// before the slab's own edge arrives, or the blur prints that edge as a
/// visible ring — a shadow with a border reads as a second card.
pub const SHADOW_SPREAD: f32 = 4.0;

/// The blur radius the shell sets on the shadow layer at creation and
/// never touches again — see `Shell::new`.
pub const SHADOW_BLUR_RADIUS: f32 = 20.0;

/// How long an anchored menu's entrance spring takes to settle, and its
/// curve: a quick rise with a small overshoot and a long tail — past 200ms
/// the pop stops reading as snappy and starts reading as lag.
/// `CubicBezier`'s y is unclamped, which is exactly what an overshoot needs.
///
/// Modals do not use this curve; see [`MODAL_FADE_DURATION`].
pub const MENU_SLIDE_DURATION: Duration = Duration::from_millis(170);
pub const MENU_SLIDE_EASING: Easing = Easing::CubicBezier(0.3, 1.25, 0.5, 1.0);

/// How long a large modal takes to fade up. No overshoot: a palette-sized
/// surface scaling past 100% reads as a wobble, not a spring. Clamped by
/// the components applying it.
pub const MODAL_FADE_DURATION: Duration = Duration::from_millis(180);

/// How long a dismissal ghost's fall takes. There is no dismiss *curve*:
/// the shell feeds the same reveal weight falling 1→0, so whatever easing
/// carried the entrance — run backwards by a falling input — is the exit,
/// and the ghost leaves with exactly the motion it arrived with. Faster
/// than any entrance, because leaving should never hold the eye longer
/// than arriving.
pub const GHOST_DURATION: Duration = Duration::from_millis(140);

/// The pill's hover chase: nearly the same spring as the entrance, shorter
/// and with a subtler overshoot — fast enough to feel attached to the
/// pointer. A function rather than a constant: `Animation::new` is not
/// `const`, and each caller needs a fresh timer anyway.
pub fn pill_animation() -> Animation {
    Animation::new(
        Duration::from_millis(110),
        Easing::CubicBezier(0.3, 1.18, 0.5, 1.0),
    )
}

/// A rounded rect that slides between its `from` and `to` targets as its
/// [`Animation`] plays — the highlight pill chasing the pointer (or the
/// keyboard) across a popup's rows or cells.
pub struct Slide {
    animation: Animation,
    from: Rect,
    to: Rect,
}

impl Default for Slide {
    fn default() -> Self {
        Self::new()
    }
}

impl Slide {
    /// A fresh slide that has never been parked: everything at zero, timer
    /// running, so the first `park` starts from a sane state.
    pub fn new() -> Self {
        Self {
            animation: pill_animation(),
            from: Rect::default(),
            to: Rect::default(),
        }
    }

    /// A fresh slide parked on top of `rect`. Snappy on purpose: a hover
    /// chase should feel immediate, not laggy — an ease-out with a whisper
    /// of overshoot lands like a magnet, not like a fade.
    pub fn park(&mut self, rect: Rect) {
        self.from = rect;
        self.to = rect;
        self.animation = pill_animation();
    }

    /// Point the slide at `rect`, leaving from wherever it currently is so
    /// the pill travels rather than teleports. Returns whether it moved.
    pub fn slide_to(&mut self, rect: Rect) -> bool {
        if self.to == rect {
            return false;
        }
        self.from = self.rect();
        self.to = rect;
        self.animation.restart();
        true
    }

    pub fn rect(&self) -> Rect {
        lerp_rect(self.from, self.to, self.animation.weight())
    }

    /// Advances the pill's clock by `dt`; returns whether it is still
    /// playing. The component owning the slide calls this from sync — the
    /// animation never leaks out of here.
    pub fn advance(&mut self, dt: std::time::Duration) -> bool {
        self.animation.advance(dt)
    }

    pub fn advancing(&self) -> bool {
        self.animation.is_playing()
    }
}

fn lerp_rect(from: Rect, to: Rect, t: f32) -> Rect {
    Rect::new(
        from.x + (to.x - from.x) * t,
        from.y + (to.y - from.y) * t,
        from.width + (to.width - from.width) * t,
        from.height + (to.height - from.height) * t,
    )
}

/// Which row/cell a sliding pill should sit on. The pointer's last touch
/// wins, for good — the highlight is persistent, whether the pointer now
/// sits in a gap or has left the card for the document entirely. `None`
/// only when there is nothing to sit on.
pub fn slide_target(touched: Option<usize>, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    touched.map(|h| h.min(len - 1))
}

/// A floating card grown out of `anchor`: scaled from 97% up to full size
/// around the point it serves while sliding down a few pixels into place —
/// the card grows out of what was clicked and settles downward, gravity
/// agreeing with the direction it opens in. Clamped, so an overshooting
/// weight beyond 1 holds at rest.
///
/// Shared by drawing code and the shell's shadow painting — one function,
/// so the two can never disagree about where the floating surface sits.
pub fn revealed_card(card: Rect, anchor: (f32, f32), e: f32) -> Rect {
    let e = e.clamp(0.0, 1.0);
    let scale = 0.97 + 0.03 * e;
    let travel = (1.0 - e) * -5.0; // starts 5px above, slides down into place
    let cx = anchor.0;
    let cy = anchor.1;
    let x = cx + (card.x - cx) * scale;
    let y = cy + (card.y - cy) * scale + travel;
    let width = card.width * scale;
    let height = card.height * scale;
    Rect::new(x, y, width, height)
}

/// Paints the drop-shadow slab for a resting card onto the shell's blurred
/// layer: one fat, faint rounded rect spread past the card, from which the
/// layer's blur makes a soft halo. The caller runs this from
/// [`Component::sync`](crate::ui::Component::sync) — which runs whether or
/// not `draw` will — and passes the revealed weight already eased.
///
/// Clears before anything else. Components are recreated on every shell
/// refresh; whatever a dead snapshot drew stays in the Manual-mode layer
/// until someone clears here, which is why the clear is unconditional even
/// when the card below would early-return: skipping it freezes the last
/// halo on screen forever.
pub fn paint_shadow_slab(shadow: &Layer, card: Rect, e: f32) {
    shadow.clear();
    if card.is_empty() || !(0.0..=1.0).contains(&e) {
        return;
    }
    shadow.draw_rectangle(
        (card.x - SHADOW_SPREAD, card.y - SHADOW_SPREAD),
        (
            card.width + SHADOW_SPREAD * 2.0,
            card.height + SHADOW_SPREAD * 2.0,
        ),
        // scale_alpha, not fade: the ink's own alpha is an authored TUNABLE
        // (see `shadow_ink`), and the reveal weight must multiply it rather
        // than replace it. `fade(shadow_ink(), e)` overwrote the byte with
        // e×255 every frame — a resting popup's e sits at 1.0, so the slab
        // rendered at full opacity and every alpha tuned into `shadow_ink`
        // was silently discarded.
        theme::scale_alpha(theme::shadow_ink(), e),
        crate::renderer::Rounding::uniform(CARD_RADIUS + SHADOW_SPREAD),
    );
}
