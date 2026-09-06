//! The page's scroll indicator: a thumb in a reserved column at the
//! canvas's right edge.
//!
//! It takes its own strip rather than floating over the text. An overlay bar
//! has to fade out to stop covering words, and a bar that fades out is gone
//! exactly when a reader glances at it to ask how much of the document is
//! left — which, on a paper long enough to need one, is the question it
//! exists to answer. Twelve reserved pixels cost one break's worth of
//! measure, and nothing on the page moves when a document grows past a
//! screenful.
//!
//! The shell owns the numbers and shares one [`Cell`] of them, so scrolling
//! updates the bar in place instead of rebuilding it. That is not a
//! micro-optimisation: a snapshot rebuilt on every scrolled pixel would
//! restart the hover transition on every scrolled pixel, and the thumb under
//! the pointer would never finish widening.
//!
//! Geometry lives here as free functions because two callers need the same
//! answer — this component draws the thumb, and the shell hit-tests a drag
//! against it. A scrollbar whose thumb is not where its drag thinks it is
//! would be a scrollbar that fights the hand holding it.

use std::cell::Cell;
use std::rc::Rc;

use crate::layout::Rect;
use crate::renderer::{Layer, Rounding};
use crate::theme;
use crate::ui::{Component, Context, Dirty, Hover};

/// The reserved column's width. Wide enough to be an easy grab target on
/// its own — the thumb is drawn much narrower, but the whole strip is
/// clickable, so aim is never a matter of pixels.
pub const WIDTH: f32 = 12.0;

/// The thumb's drawn width, at rest and under the pointer. The rest state
/// is a hairline that reports position; the hot state is a control.
const THUMB: f32 = 4.0;
const THUMB_HOT: f32 = 7.0;

/// The shortest the thumb may get. Proportional length is what makes a
/// scrollbar tell you how long a document is, but taken literally it
/// vanishes on a long one and takes the grab target with it.
const MIN_LENGTH: f32 = 36.0;

/// Space kept above and below the track, so the thumb never runs into the
/// document sheet's rounded corners.
const INSET: f32 = 10.0;

/// What the bar reports: where the page sits, the offsets it may take, and
/// how much of the document is on screen at once.
///
/// `Copy` and shared through a [`Cell`] — the shell writes it on every
/// frame the page moves, and never rebuilds the component to do it.
#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub struct Span {
    /// The page's current offset, somewhere within `range`.
    pub scroll: f32,
    /// The offsets the page may take, low to high.
    pub range: (f32, f32),
    /// The visible height of the content. With `range` this is the whole
    /// shape of the document: the viewport shows `view` out of a document
    /// that is `reach` plus `view` tall.
    pub view: f32,
    /// Whether the reader has hold of the thumb. Held is drawn hot however
    /// far the pointer has since wandered from the strip.
    pub held: bool,
}

impl Span {
    /// How far the page can travel. Zero means the document fits and there
    /// is no bar to draw.
    pub fn reach(&self) -> f32 {
        self.range.1 - self.range.0
    }
}

/// The strip the thumb travels in: `rect`, less the space kept at each end.
pub fn track(rect: Rect) -> Rect {
    Rect::new(
        rect.x,
        rect.y + INSET,
        rect.width,
        (rect.height - INSET * 2.0).max(0.0),
    )
}

/// The thumb's rect inside `track`, or `None` when the document fits.
///
/// Full track width: this is the *grab* rect, and the drawn bar is centred
/// inside it. A reader aiming at a four-pixel line would miss.
pub fn thumb(track: Rect, span: Span) -> Option<Rect> {
    let reach = span.reach();
    if reach <= 0.5 || track.height <= 0.0 || span.view <= 0.0 {
        return None;
    }
    // `view` out of the whole document, which is the travel plus the
    // viewport itself — the ratio a reader reads as "how much of this am I
    // looking at".
    let length = (track.height * span.view / (reach + span.view))
        .clamp(MIN_LENGTH.min(track.height), track.height);
    let at = ((span.scroll - span.range.0) / reach).clamp(0.0, 1.0);
    Some(Rect::new(
        track.x,
        track.y + (track.height - length) * at,
        track.width,
        length,
    ))
}

/// The scroll offset that puts the thumb's *top edge* at `y`.
///
/// The exact inverse of [`thumb`], which is what keeps a drag from creeping:
/// the pointer grabs the thumb at some distance from its top and holds it
/// there, so the same distance has to come back out.
pub fn scroll_at(track: Rect, y: f32, span: Span) -> f32 {
    let Some(thumb) = thumb(track, span) else {
        return span.range.0;
    };
    let travel = track.height - thumb.height;
    if travel <= 0.0 {
        return span.range.0;
    }
    span.range.0 + ((y - track.y) / travel).clamp(0.0, 1.0) * span.reach()
}

pub struct Scrollbar {
    span: Rc<Cell<Span>>,
    /// The span as of the last sync — the value drawn, and what the dirty
    /// flag compares against.
    shown: Span,
    hover: Hover,
    dirty: Dirty,
}

impl Scrollbar {
    pub fn new(span: Rc<Cell<Span>>) -> Self {
        Self {
            span,
            shown: Span::default(),
            hover: Hover::new(),
            dirty: Dirty::new(),
        }
    }
}

impl Component for Scrollbar {
    fn measure(&mut self, _: &Layer) -> (f32, f32) {
        (WIDTH, 0.0)
    }

    fn sync(&mut self, context: &Context) {
        self.dirty.write(&mut self.shown, self.span.get());
        // Held beats hovered: a thumb dragged past the strip's edge is
        // still the thing the pointer is doing.
        let hot = self.shown.held || context.hovering(context.self_rect);
        if self.hover.update(hot, context.animation_dt) {
            self.dirty.set();
        }
    }

    fn is_dirty(&self) -> bool {
        self.dirty.get()
    }

    fn clear_dirty(&mut self) {
        self.dirty.clear();
    }

    fn is_animating(&self) -> bool {
        self.hover.is_animating()
    }

    fn draw(&mut self, layer: &Layer, rect: Rect) {
        let Some(grab) = thumb(track(rect), self.shown) else {
            return;
        };
        // Two named inks and no invented alpha: at rest the bar sits in the
        // palette's non-text register, with the fold chevrons and equation
        // numbers, and warms to `faint` as it widens into a control.
        let weight = self.hover.value();
        let width = THUMB + (THUMB_HOT - THUMB) * weight;
        layer.draw_rectangle(
            (grab.x + (grab.width - width) * 0.5, grab.y),
            (width, grab.height),
            theme::mix(theme::non_text(), theme::faint(), weight),
            Rounding::uniform(width * 0.5),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(scroll: f32, max: f32, view: f32) -> Span {
        Span {
            scroll,
            range: (0.0, max),
            view,
            held: false,
        }
    }

    const STRIP: Rect = Rect {
        x: 900.0,
        y: 100.0,
        width: WIDTH,
        height: 620.0,
    };

    #[test]
    fn a_document_that_fits_has_no_thumb() {
        assert_eq!(thumb(track(STRIP), span(0.0, 0.0, 600.0)), None);
        // A view taller than nothing, but with no travel: still nothing.
        assert_eq!(thumb(track(STRIP), span(0.0, 0.4, 600.0)), None);
    }

    /// The thumb's length is the fraction of the document on screen, and
    /// its position is the fraction of the travel used up. Both ends are
    /// exact — an indicator that stops short of the bottom would say the
    /// reader had not reached the end when they had.
    #[test]
    fn the_thumb_is_as_long_as_the_view_and_travels_the_whole_track() {
        let track = track(STRIP);
        // Half the document on screen: half the track, parked at the top.
        let top = thumb(track, span(0.0, 600.0, 600.0)).expect("scrollable");
        assert!((top.height - track.height / 2.0).abs() < 0.001);
        assert_eq!(top.y, track.y);

        let bottom = thumb(track, span(600.0, 600.0, 600.0)).expect("scrollable");
        assert!((bottom.bottom() - track.bottom()).abs() < 0.001);

        let middle = thumb(track, span(300.0, 600.0, 600.0)).expect("scrollable");
        assert!((middle.y - (track.y + track.height / 4.0)).abs() < 0.001);
    }

    /// Proportional length taken literally disappears; the floor is what
    /// keeps a very long document draggable.
    #[test]
    fn a_very_long_document_still_has_something_to_grab() {
        let thumb = thumb(track(STRIP), span(0.0, 90_000.0, 600.0)).expect("scrollable");
        assert_eq!(thumb.height, MIN_LENGTH);
        assert_eq!(thumb.width, WIDTH, "the whole strip is the grab target");
    }

    /// A drag reads the thumb's top back out of the pointer, so the two
    /// have to be exact inverses or the thumb creeps away from the hand.
    #[test]
    fn dragging_lands_the_thumb_where_it_was_taken_from() {
        let track = track(STRIP);
        for scroll in [0.0, 137.0, 900.0, 2400.0] {
            let span = span(scroll, 2400.0, 600.0);
            let thumb = thumb(track, span).expect("scrollable");
            let back = scroll_at(track, thumb.y, span);
            assert!((back - scroll).abs() < 0.01, "{scroll} came back as {back}");
        }
    }

    /// Past either end the answer is the end, not an offset off the page.
    #[test]
    fn a_drag_past_the_ends_clamps_to_them() {
        let track = track(STRIP);
        let span = span(0.0, 2400.0, 600.0);
        assert_eq!(scroll_at(track, track.y - 400.0, span), 0.0);
        assert_eq!(scroll_at(track, track.bottom() + 400.0, span), 2400.0);
    }

    /// Focus mode scrolls through negative offsets — the camera starts
    /// above the first line. The bar is drawn from a range, not from zero,
    /// so it must survive one.
    #[test]
    fn a_range_that_starts_below_zero_still_maps_end_to_end() {
        let track = track(STRIP);
        let span = Span {
            scroll: -200.0,
            range: (-200.0, 1000.0),
            view: 600.0,
            held: false,
        };
        let thumb = thumb(track, span).expect("scrollable");
        assert_eq!(thumb.y, track.y, "the low end of the range is the top");
        assert_eq!(scroll_at(track, track.y, span), -200.0);
    }
}
