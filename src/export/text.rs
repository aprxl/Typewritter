//! [`Shaper`] — the app's own text stack, borrowed for the export.
//!
//! Export measures and export draws, and both have to be the screen's
//! answers or the 1:1 promise is broken at the first bold heading. So
//! neither is computed here: [`Shaper::width`] is `theme::width`, the
//! function the editor wraps on, and [`Shaper::runs`] hands over the glyphs
//! that same shaping produced.
//!
//! What this file *does* own is the one thing the renderer has no reason to
//! know: how the synthetic weight/width/tracking effects turn shaped
//! positions into drawn ones. `renderer/glyph_effects.rs` does that for the
//! screen at rasterization time; [`Shaper::runs`] does the identical
//! arithmetic here, through the same `effect_advance_delta`, for a backend
//! that places glyphs itself.

use crate::renderer::{FaceId, FontParameters, Layer, effect_advance_delta};
use crate::theme::TextStyle;

/// A stretch of glyphs from one face, positioned and ready to draw.
/// Shaping falls back per glyph, so one string can be several runs.
pub struct GlyphRun {
    pub face: FaceId,
    /// Glyph indices within `face`, in visual order.
    pub glyphs: Vec<u16>,
    /// Each glyph's pen position, in logical pixels from the string's left
    /// edge — synthetic effects already applied.
    pub positions: Vec<f32>,
    /// Each glyph's advance, likewise corrected. A backend places by
    /// `positions`; this is what it reports as the glyph's own width, which
    /// is what a reader's selection and word spacing follow.
    pub advances: Vec<f32>,
    /// The source text these glyphs came from, so a backend can write a
    /// character mapping. Not necessarily one char per glyph.
    pub text: String,
}

/// One string's glyphs, grouped into runs, plus where its baseline sits.
pub struct ShapedLine {
    pub runs: Vec<GlyphRun>,
    /// Below the box's top edge. Alignment resolves against the box; glyphs
    /// are drawn from the baseline.
    pub baseline: f32,
    pub size: (f32, f32),
}

/// Borrows the editor's layer for its text stack. Cheap to make, cheap to
/// pass around — everything expensive is the layer's, already warm.
pub struct Shaper<'a> {
    layer: &'a Layer,
}

impl<'a> Shaper<'a> {
    pub fn new(layer: &'a Layer) -> Self {
        Self { layer }
    }

    /// What `document::layout` wraps on. Byte-for-byte the editor's own
    /// measurement — see `theme::width`.
    pub fn width(&self, text: &str, style: &TextStyle) -> f32 {
        crate::theme::width(self.layer, text, style)
    }

    /// The glyphs, placed. Positions carry the synthetic effects the shaper
    /// itself never sees:
    ///
    /// - **tracking** shifts every glyph right by half a track, which is
    ///   how cosmic-text's CSS-style letter-spacing gets recentred into
    ///   space *between* letters rather than after them;
    /// - **faux bold** dilates the outline by `weight` px on every side, so
    ///   each glyph is nudged right by that much to recentre its ink, and
    ///   the pen grows by twice it;
    /// - **faux condense** scales ink toward the glyph origin, and the pen
    ///   with it.
    ///
    /// The last two are `effect_advance_delta`, shared with the screen. The
    /// outline transform itself (condense and slant) is not applied here —
    /// it is a property of every glyph in the run alike, so a backend
    /// applies it once as a transform. See `PDF.md` §4.
    pub fn runs(&self, text: &str, style: &TextStyle) -> ShapedLine {
        let parameters = style.parameters();
        let shaped = self.layer.shape_text(text, &style.font, &parameters);
        let FontParameters {
            size,
            weight,
            width,
            tracking,
            ..
        } = parameters;
        let embolden = weight.max(0.0);
        let track_shift = 0.5 * tracking * size * width;

        let mut runs: Vec<GlyphRun> = Vec::new();
        let mut pen_delta = 0.0;
        for glyph in &shaped.glyphs {
            let x = glyph.x + pen_delta + embolden + track_shift;
            let delta = effect_advance_delta(embolden, width, glyph.advance);
            pen_delta += delta;

            // A run ends where the face changes. Fallback puts a handful of
            // glyphs from another face mid-string; everything else is one
            // run per call.
            match runs.last_mut() {
                Some(run) if run.face == glyph.face => {
                    run.glyphs.push(glyph.glyph);
                    run.positions.push(x);
                    run.advances.push(glyph.advance + delta);
                }
                _ => runs.push(GlyphRun {
                    face: glyph.face,
                    glyphs: vec![glyph.glyph],
                    positions: vec![x],
                    advances: vec![glyph.advance + delta],
                    text: String::new(),
                }),
            }
        }

        // Each run claims the source text from its first glyph's cluster up
        // to the next run's, so a ligature or a mark cluster stays whole and
        // the runs' texts concatenate back to the original string. Slicing
        // is guarded rather than trusted: a cluster is a byte offset the
        // shaper chose, and a run whose bounds don't come back in order (or
        // don't land on char boundaries) gives up its text instead of
        // panicking — a backend loses that run's character mapping, not the
        // export.
        let starts: Vec<usize> = runs
            .iter()
            .scan(0usize, |glyph_index, run| {
                let start = shaped.glyphs[*glyph_index].cluster;
                *glyph_index += run.glyphs.len();
                Some(start)
            })
            .collect();
        for (index, run) in runs.iter_mut().enumerate() {
            let start = starts[index];
            let end = starts.get(index + 1).copied().unwrap_or(text.len());
            if start < end && text.is_char_boundary(start) && text.is_char_boundary(end) {
                run.text = text[start..end].to_string();
            }
        }

        ShapedLine {
            runs,
            baseline: shaped.baseline,
            size: shaped.size,
        }
    }
}
