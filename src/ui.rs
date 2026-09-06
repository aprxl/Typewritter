//! Components, and the regions that host them.
//!
//! A [`Component`] owns one piece of the interface end to end: it measures
//! itself, decides when it has changed, and draws itself into whatever
//! rectangle it is given. The layout knows none of that — it is handed a
//! minimum size and hands back a rect.
//!
//! **One layer per region.** Each [`Region`] holds its own Atomos layer in
//! [`LayerInvalidation::Manual`] mode, so a component that did not change
//! issues no draw calls at all: no tessellation, no text shaping, no GPU
//! upload, nothing but the composite of a texture that is already there.
//! A keystroke that only touches the text column rebuilds the text
//! column's layer and leaves the other eight alone.
//!
//! One layer per region is also what makes clipping possible: the scissor
//! is per layer, so a region's layer is scissored to that region's rect
//! and a component physically cannot draw over its neighbours.
//!
//! The redraw rule is the whole point, and it is two conditions:
//!
//! 1. the component says it is dirty, or
//! 2. the layout moved or resized its rect.
//!
//! Neither one is guessed at — components report the first, the layout
//! reports the second.

use std::time::Duration;

use crate::animation::{Animation, Easing};
use crate::layout::{Layout, NodeId, Rect};
use crate::renderer::Layer;

/// This frame's mouse state, for components that hit-test themselves.
#[derive(Clone, Copy, Debug)]
pub struct Mouse {
    /// Cursor position in logical pixels, same space as the draw API.
    pub position: (f32, f32),
    /// Did the left button go down this frame?
    pub left_pressed: bool,
    /// Did the right button go down this frame? Components that own a
    /// context menu need it; the shell handles its own right-clicks
    /// straight off `Input`.
    pub right_pressed: bool,
    /// Whether the cursor is over the window at all.
    pub in_window: bool,
}

impl Default for Mouse {
    fn default() -> Self {
        Self {
            position: (0.0, 0.0),
            left_pressed: false,
            right_pressed: false,
            in_window: false,
        }
    }
}

/// Per-frame shell state a component may want to draw. Components pull
/// what they care about in [`Component::sync`] and ignore the rest; this
/// keeps the shell from having to reach into a specific component's type.
#[derive(Clone, Copy, Debug, Default)]
pub struct Context {
    /// Wall-clock time of the last frame.
    pub frametime: Duration,
    /// Clamped delta for caller-owned animations.
    pub animation_dt: Duration,
    /// Layout solves since launch — cumulative, not a live count.
    pub layouts: u32,
    /// Regions that redrew on the previous frame, out of how many.
    pub redraws: (usize, usize),
    /// The tree/canvas divider is hovered or being dragged.
    pub divider_hot: bool,
    /// Caret visibility this frame — step-end, not a fade.
    pub caret_on: bool,
    /// Whether the status line shows render instrumentation.
    pub show_stats: bool,
    /// Every auxiliary panel is closed for focused writing.
    pub focus_mode: bool,
    /// Animated emphasis, independent of the editor snapshot's lifetime.
    pub focus_amount: f32,
    /// Mouse state, for components that handle clicks and hovers.
    pub mouse: Mouse,
    /// Whether THIS region owns the shared popup shadow layer this frame —
    /// exactly the region whose popup is on screen (or falling as a ghost).
    /// Owners paint/clear it; everyone else must never touch it, or a
    /// closed snapshot's unconditional clear would race a live halo (this
    /// wiped popups' shadows once the format bar had been used: its closed
    /// snapshot synced after other regions' paints and cleared their slabs).
    pub owns_shadow: bool,
    /// Lines scrolled this frame (positive is up), for scrollable regions.
    pub scroll_y: f32,
    /// The region's own solved rect. The shell sets this per region each
    /// frame so a component can hit-test against itself in `sync`, before
    /// its `draw`.
    pub self_rect: Rect,
    /// Animated opacity of the tree/canvas divider hover state.
    pub divider_hover: f32,
    /// Debug aid: draw each row's hit band so hit-testing can be checked
    /// against what is on screen.
    pub debug_rows: bool,
    /// The entrance reveal of an overlay that opened, 0..1 — 1.0 when none
    /// is in flight. The shell owns the animation so a refreshed overlay
    /// snapshot (a new `Component`) never re-triggers the pop: the value is
    /// a matter of elapsed time, not of which component instance reads it.
    /// The first consumer is the format bar, which fades and lifts as this
    /// climbs from 0.
    pub reveal: f32,
    /// Something is drawn over the whole window — the onboarding splash or
    /// a dialog. Everything underneath is a picture until it closes.
    pub overlay_open: bool,
    /// A palette swap is still running. The switch refuses clicks until it
    /// finishes, and says so by holding itself at rest.
    pub theme_locked: bool,
}

impl Context {
    /// Is the pointer over `rect` and free to act on it? False while an
    /// overlay covers the window: a region under a splash or a dialog is
    /// not clickable just because it is still on screen.
    ///
    /// Overlay components hit-test [`Context::mouse`] directly instead —
    /// they *are* the overlay.
    pub fn hovering(&self, rect: Rect) -> bool {
        self.mouse.in_window && !self.overlay_open && rect.contains(self.mouse.position)
    }

    /// Which of `rects` the pointer is over, on the same terms.
    pub fn hovered_index(&self, rects: &[Rect]) -> Option<usize> {
        if !self.mouse.in_window || self.overlay_open {
            return None;
        }
        rects
            .iter()
            .position(|rect| rect.contains(self.mouse.position))
    }

    /// Where the left button went down this frame, if it did and if this
    /// component is allowed to act on it.
    pub fn click_position(&self) -> Option<(f32, f32)> {
        (self.mouse.left_pressed && self.mouse.in_window && !self.overlay_open)
            .then_some(self.mouse.position)
    }

    /// Where the right button went down this frame, if it did and if this
    /// component is allowed to act on it.
    pub fn right_click_position(&self) -> Option<(f32, f32)> {
        (self.mouse.right_pressed && self.mouse.in_window && !self.overlay_open)
            .then_some(self.mouse.position)
    }
}

/// One reversible, linear hover transition. Components own these rather than
/// sharing a registry because each visual affordance has one owner.
#[derive(Debug)]
pub struct Hover {
    animation: Animation,
    from: f32,
    to: f32,
    target: bool,
}

impl Hover {
    pub fn new() -> Self {
        Self {
            animation: Animation::new(Duration::from_millis(150), Easing::Linear).paused(),
            from: 0.0,
            to: 0.0,
            target: false,
        }
    }

    /// Updates target and advances by already-clamped frame time. Returns
    /// whether component should redraw this frame.
    pub fn update(&mut self, over: bool, dt: Duration) -> bool {
        let mut changed = false;
        if over != self.target {
            self.from = self.value();
            self.to = if over { 1.0 } else { 0.0 };
            self.target = over;
            self.animation = Animation::new(Duration::from_millis(150), Easing::Linear);
            changed = true;
        }

        let before = self.value();
        self.animation.advance(dt);
        changed || before != self.value()
    }

    /// Follows which of several items the pointer is over, `current` being
    /// the component's record of it. The old item is held onto while the
    /// fade-out runs, so a highlight always finishes on the item it started
    /// on rather than vanishing the instant the pointer leaves. Returns
    /// whether anything visible changed.
    pub fn track<T: PartialEq>(
        &mut self,
        current: &mut Option<T>,
        next: Option<T>,
        dt: Duration,
    ) -> bool {
        let over = next.is_some();
        let mut changed = false;
        if over && next != *current {
            *current = next;
            changed = true;
        }
        changed |= self.update(over, dt);
        // Faded out and nothing under the pointer: release the old item.
        if !over && self.value() == 0.0 && !self.is_animating() && current.take().is_some() {
            changed = true;
        }
        changed
    }

    pub fn value(&self) -> f32 {
        self.animation.value(self.from, self.to)
    }

    pub fn is_animating(&self) -> bool {
        self.animation.is_playing()
    }
}

impl Default for Hover {
    fn default() -> Self {
        Self::new()
    }
}

/// A number that eases toward a target which may move while it travels.
///
/// [`Hover`]'s shape, for a value that is not a 0..1 weight — the page's
/// scroll offset. Two properties make it the right primitive for that:
///
/// - **Re-aiming picks up from wherever the value is now**, so a second
///   wheel notch during the first one's travel reads as one continuous push
///   rather than a jump back to the old resting place.
/// - **Aiming where it is already going does nothing**, so a target that is
///   recomputed and re-aimed every frame does not hold the glide at its
///   start forever.
///
/// [`Glide::advance`] reports the frame it *lands* on as a change, not just
/// the frames before it: the landing frame is the one that moves the value
/// onto the target, and a caller that stopped asking for frames a moment
/// early would leave the last pixel of travel undrawn.
#[derive(Debug)]
pub struct Glide {
    animation: Animation,
    from: f32,
    to: f32,
    duration: Duration,
    easing: Easing,
}

impl Glide {
    /// A glide resting at zero. `duration` is one full travel, whatever the
    /// distance: a scroll should take the same time to settle whether it
    /// moved a line or a screen.
    pub fn new(duration: Duration, easing: Easing) -> Self {
        Self {
            animation: Animation::new(duration, easing).paused(),
            from: 0.0,
            to: 0.0,
            duration,
            easing,
        }
    }

    /// Travel to `to`, starting from wherever the value is now.
    pub fn aim(&mut self, to: f32) {
        if to == self.to {
            return;
        }
        self.from = self.value();
        self.to = to;
        self.animation = Animation::new(self.duration, self.easing);
    }

    /// Arrive at `to` this instant. For the moves that are not travel at
    /// all: a different document under the same offset, or a thumb the
    /// reader is dragging, which must stay under the pointer holding it.
    pub fn settle(&mut self, to: f32) {
        self.from = to;
        self.to = to;
        self.animation = Animation::new(self.duration, self.easing).paused();
    }

    /// Steps the clock and reports whether anything visible changed.
    pub fn advance(&mut self, dt: Duration) -> bool {
        let before = self.value();
        let running = self.animation.advance(dt);
        running || before != self.value()
    }

    pub fn value(&self) -> f32 {
        self.animation.value(self.from, self.to)
    }

    /// Where it is heading — what to read when deciding where to go next.
    /// Accumulating onto [`Glide::value`] instead would make each notch of
    /// a fast scroll shorter than the one before it, because every one
    /// would start from a journey that had not finished.
    pub fn target(&self) -> f32 {
        self.to
    }
}

/// One piece of interface. Every method has a default except [`draw`], so
/// a static component is a single function.
///
/// [`draw`]: Component::draw
pub trait Component {
    /// The smallest `(width, height)` this component can draw itself in,
    /// in logical pixels. Measured against `layer` so text-derived
    /// minimums are real measurements rather than guesses; called only
    /// when the component is dirty, never per frame.
    fn measure(&mut self, _layer: &Layer) -> (f32, f32) {
        (0.0, 0.0)
    }

    /// Reads whatever it needs out of the frame's [`Context`], and marks
    /// itself dirty if that changed something it draws.
    fn sync(&mut self, _context: &Context) {}

    /// Draws into `rect`. The layer has already been cleared, and is
    /// scissored to `rect` — anything drawn past the edge is cut off
    /// rather than landing on a neighbour.
    ///
    /// That is a safety net, not a layout strategy: clipped content is
    /// invisible content. A component that can say something useful in
    /// less room (fewer tabs, an elided path) should still do so, and
    /// [`Component::measure`] should report the width below which it
    /// can't.
    fn draw(&mut self, layer: &Layer, rect: Rect);

    /// Has this component's own state changed since it was last drawn?
    fn is_dirty(&self) -> bool {
        false
    }

    /// Called right after a redraw.
    fn clear_dirty(&mut self) {}

    /// Whether this component owns a transition that needs another frame.
    fn is_animating(&self) -> bool {
        false
    }

    /// Type-erased access for the shell, which owns regions but not the
    /// concrete components inside them. The one use today is reading the
    /// format bar's pill position back after a refresh; implementors that
    /// never need it get the default for free.
    fn as_any(&self) -> &dyn std::any::Any {
        &()
    }
}

/// A component bound to a layout node and, usually, its own layer.
///
/// "Usually" because a layer is a full-surface render target whether or not
/// anything is drawn into it, which is a bad deal for a region that spends
/// almost all of its life closed. A detached region keeps its component and
/// its place in the stack, owns no GPU memory, and costs nothing per frame —
/// see [`Region::attach`].
pub struct Region {
    node: NodeId,
    layer: Option<Layer>,
    component: Box<dyn Component>,
    last_rect: Rect,
    measured: bool,
}

impl Region {
    pub fn new(node: NodeId, layer: Layer, component: Box<dyn Component>) -> Self {
        Self {
            node,
            layer: Some(layer),
            component,
            last_rect: Rect::default(),
            measured: false,
        }
    }

    /// A region with no layer yet. It draws nothing and measures nothing
    /// until [`Region::attach`] gives it one.
    pub fn detached(node: NodeId, component: Box<dyn Component>) -> Self {
        Self {
            node,
            layer: None,
            component,
            last_rect: Rect::default(),
            measured: false,
        }
    }

    /// The layout node this region is bound to.
    pub fn node(&self) -> NodeId {
        self.node
    }

    /// The region's layer — measuring text the same way its own draw does.
    ///
    /// # Panics
    /// If the region is detached. Only call this on regions that are
    /// attached for the whole session; for the rest use
    /// [`Region::is_attached`] first.
    pub fn layer(&self) -> &Layer {
        self.layer.as_ref().expect("region has no layer attached")
    }

    pub fn is_attached(&self) -> bool {
        self.layer.is_some()
    }

    /// Gives the region a layer to draw into, forcing a re-measure and a
    /// redraw — the layer is new, so it holds nothing.
    pub fn attach(&mut self, layer: Layer) {
        self.layer = Some(layer);
        self.measured = false;
        self.last_rect = Rect::default();
    }

    /// Drops the region's layer, and with it a full-surface render target.
    /// The renderer's stack holds only a `Weak`, so the layer leaves the
    /// composite on its own; re-attaching puts a fresh one back on top.
    pub fn detach(&mut self) {
        self.layer = None;
        self.measured = false;
        self.last_rect = Rect::default();
    }

    /// Forces a re-measure and redraw on the next update, keeping the
    /// component (unlike [`Region::set_component`]). For shell-side state
    /// (a deleted vault row) the component cannot see.
    pub fn poke(&mut self) {
        self.measured = false;
        self.last_rect = Rect::default();
    }

    /// Replaces the component, forcing a re-measure and a redraw. The layer
    /// is kept — the region's scissor and invalidation mode survive.
    pub fn set_component(&mut self, component: Box<dyn Component>) {
        self.component = component;
        self.measured = false;
        self.last_rect = Rect::default();
    }

    pub fn sync(&mut self, context: &Context) {
        self.component.sync(context);
    }

    /// The region's component, downcast to `T` — the shell's read-only
    /// window into a snapshot's state (see [`Component::as_any`]).
    pub fn component_as<T: 'static>(&self) -> Option<&T> {
        self.component.as_any().downcast_ref()
    }

    pub fn is_animating(&self) -> bool {
        self.component.is_animating()
    }

    /// Pushes the component's minimum into the tree, re-measuring only
    /// when the component has changed. `set_style` compares by value, so
    /// an unchanged minimum does not dirty the layout.
    pub fn measure_into(&mut self, layout: &mut Layout) {
        // A detached region is not on screen, so it has no claim on space —
        // and measuring text needs a layer to measure it against anyway.
        let Some(layer) = &self.layer else { return };
        if self.measured && !self.component.is_dirty() {
            return;
        }
        let min = self.component.measure(layer);
        layout.set_style(self.node, |s| s.min_size = min);
        self.measured = true;
    }

    /// Redraws this region if it needs it. Returns whether it did.
    pub fn update(&mut self, layout: &Layout) -> bool {
        let Some(layer) = &self.layer else {
            return false;
        };
        let rect = if layout.style(self.node).visible {
            layout.rect(self.node)
        } else {
            // A collapsed region keeps its layer, emptied. Re-expanding
            // gives it a different rect, which redraws it.
            Rect::default()
        };
        if rect == self.last_rect && !self.component.is_dirty() {
            return false;
        }

        // Manual layers accumulate draw calls forever; clearing first is
        // what makes a redraw a replacement rather than a pile-up.
        layer.clear();
        if rect != self.last_rect {
            // Hardware scissor, one per region. Clip state is caller-driven
            // rather than part of the drawn content, so it survives
            // `clear()` and only has to be re-set when the rect moves.
            layer.set_clip_rect(Some((rect.position(), rect.size())));
        }
        self.last_rect = rect;
        if !rect.is_empty() {
            self.component.draw(layer, rect);
            // Components may use a narrower temporary clip. Restore Region's
            // viewport after draw so later dirty redraws remain scissored.
            layer.set_clip_rect(Some((rect.position(), rect.size())));
        }
        self.component.clear_dirty();
        true
    }
}

/// A dirty flag with the one behaviour worth not rewriting per component:
/// setting it to the value it already has is not a change.
#[derive(Debug, Default)]
pub struct Dirty(bool);

impl Dirty {
    /// Starts dirty — a component that has never drawn always needs to.
    pub fn new() -> Self {
        Self(true)
    }

    pub fn set(&mut self) {
        self.0 = true;
    }

    /// Assigns `value` to `field`, marking dirty only on a real change.
    pub fn write<T: PartialEq>(&mut self, field: &mut T, value: T) {
        if *field != value {
            *field = value;
            self.0 = true;
        }
    }

    pub fn get(&self) -> bool {
        self.0
    }

    pub fn clear(&mut self) {
        self.0 = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_component_is_dirty_until_it_draws_and_only_on_real_changes() {
        let mut dirty = Dirty::new();
        assert!(dirty.get(), "never drawn");
        dirty.clear();

        let mut frametime = Duration::from_millis(5);
        dirty.write(&mut frametime, Duration::from_millis(5));
        assert!(!dirty.get(), "same value is not a change");

        dirty.write(&mut frametime, Duration::from_millis(6));
        assert!(dirty.get());
    }

    /// The scroll case, in miniature: a notch lands, a second notch arrives
    /// mid-travel, and the page must carry both without losing the distance
    /// the first one had left to run.
    #[test]
    fn a_second_aim_mid_flight_keeps_the_distance_the_first_had_left() {
        let mut glide = Glide::new(Duration::from_millis(100), Easing::Linear);
        glide.aim(100.0);
        glide.advance(Duration::from_millis(50));
        assert!((glide.value() - 50.0).abs() < 0.001, "halfway");

        // A second notch: the target grows, and travel resumes from 50 —
        // not from 0, and not by teleporting to the old target first.
        glide.aim(200.0);
        assert!((glide.value() - 50.0).abs() < 0.001, "no jump on re-aim");
        assert_eq!(glide.target(), 200.0);
        glide.advance(Duration::from_millis(50));
        assert!((glide.value() - 125.0).abs() < 0.001);
        glide.advance(Duration::from_millis(50));
        assert_eq!(glide.value(), 200.0, "arrives exactly");
    }

    /// The camera aims at the same offset every frame it is asked to follow
    /// a caret that has not moved. Restarting there would pin the value at
    /// the start of a travel it never finishes.
    #[test]
    fn aiming_where_it_is_already_going_is_not_a_restart() {
        let mut glide = Glide::new(Duration::from_millis(100), Easing::Linear);
        glide.aim(100.0);
        for _ in 0..4 {
            glide.aim(100.0);
            glide.advance(Duration::from_millis(25));
        }
        assert_eq!(glide.value(), 100.0);
    }

    /// A frame that lands the value has to be reported, or the caller stops
    /// asking for frames one short and the last pixel is never drawn.
    #[test]
    fn the_landing_frame_counts_as_a_change_and_the_rest_do_not() {
        let mut glide = Glide::new(Duration::from_millis(100), Easing::Linear);
        glide.aim(10.0);
        assert!(glide.advance(Duration::from_millis(60)), "still travelling");
        assert!(
            glide.advance(Duration::from_millis(60)),
            "landed this frame"
        );
        assert!(!glide.advance(Duration::from_millis(60)), "at rest");

        // Settling is arrival without travel: nothing to animate afterwards.
        glide.settle(400.0);
        assert_eq!(glide.value(), 400.0);
        assert_eq!(glide.target(), 400.0);
        assert!(!glide.advance(Duration::from_millis(16)));
    }

    #[test]
    fn hover_reverses_from_current_weight() {
        let mut hover = Hover::new();
        assert!(hover.update(true, Duration::ZERO));
        hover.update(true, Duration::from_millis(75));
        assert!((hover.value() - 0.5).abs() < 0.001);

        hover.update(false, Duration::ZERO);
        assert!((hover.value() - 0.5).abs() < 0.001);
        hover.update(false, Duration::from_millis(150));
        assert!(hover.value().abs() < 0.001);
    }
}
