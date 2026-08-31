//! [`Canvas`] — everything the page painter can put on a page.
//!
//! Four methods, named and shaped exactly like the [`Layer`] calls
//! `components/editor.rs` draws the same things with. That is on purpose
//! and it is the whole abstraction: `paint.rs` reads as a diff against the
//! editor's own painting, so "does the PDF match the screen" is a question
//! you can answer by reading two files side by side rather than by
//! comparing renders.
//!
//! It is not a general drawing API and should not grow into one. A new
//! element gets an arm in `paint.rs`; it does not get a method here. The
//! four below are, between them, every primitive the editor uses.
//!
//! [`Layer`]: crate::renderer::Layer

use crate::renderer::{Alignment, Color, PathPaint, Rounding};
use crate::theme::TextStyle;

/// Coordinates are logical pixels in the content column, top-left origin,
/// y down — the same space [`DocLayout`] is in. A backend maps them to
/// wherever it draws; nothing above this trait knows about that space.
///
/// [`DocLayout`]: crate::document::layout::DocLayout
pub trait Canvas {
    fn draw_rectangle(
        &mut self,
        at: (f32, f32),
        size: (f32, f32),
        color: Color,
        rounding: Rounding,
    );

    #[allow(dead_code)] // arrives with the list markers — `PDF.md` §6
    fn draw_circle(&mut self, center: (f32, f32), radius: f32, color: Color);

    /// An SVG path-data string, scaled and placed by its own origin.
    /// `rotation` is radians clockwise about the scaled path's bounding-box
    /// centre, matching `Layer::draw_path_rotated` — zero is upright.
    #[allow(dead_code)] // arrives with the task checkbox and math — `PDF.md` §6
    fn draw_path(&mut self, d: &str, at: (f32, f32), scale: f32, rotation: f32, paint: &PathPaint);

    /// `at` is the point named by `align`, as in `theme::draw` — for the
    /// editor's body text that is the left edge at the line's vertical
    /// centre, not a baseline.
    fn draw_text(&mut self, text: &str, at: (f32, f32), style: &TextStyle, align: Alignment);

    /// How wide `text` sets — `Layer::get_text_size`'s width, and the
    /// number `document::layout` wrapped on.
    ///
    /// Measuring sits on the drawing trait for the same reason it sits on
    /// `Layer`: whatever draws the glyphs is the only thing that can say
    /// how much room they take, and a painter that had to be handed a
    /// second measurer could be handed one that disagrees. It is also what
    /// lets `paint.rs` be tested against a recording canvas with no GPU
    /// behind it.
    fn measure(&self, text: &str, style: &TextStyle) -> f32;
}
