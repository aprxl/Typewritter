//! [`ShapedText`] — the glyphs behind a measurement, for callers that have
//! to place text somewhere the GPU isn't.
//!
//! [`Layer::get_text_size`](super::Layer::get_text_size) answers "how wide",
//! which is all screen drawing needs: the same `Layer` that measured also
//! draws, so the glyphs never have to leave the renderer. An exporter is the
//! other case — it measures here and draws elsewhere, and if it re-shaped
//! the text at the far end it would be a second shaper making a second set
//! of decisions about kerning, clusters, and font fallback. This type is
//! the first shaper's answer, handed over intact.
//!
//! Positions are the *natural* shaped ones, in logical pixels. The
//! synthetic weight/width/tracking effects
//! (`super::glyph_effects`) are deliberately **not** baked in: they are
//! rasterization-time effects the shaper never sees, and the caller applies
//! them with the same
//! [`effect_advance_delta`](super::glyph_effects::effect_advance_delta) the
//! screen uses, so the two can't disagree.

/// One face in the renderer's font database. Opaque: the only thing a
/// caller can do with it is group glyphs by it and ask
/// [`Layer::face_data`](super::Layer::face_data) for its bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FaceId(pub(super) glyphon::fontdb::ID);

/// One shaped glyph, positioned in logical pixels from its run's origin.
#[derive(Clone, Debug)]
pub struct ShapedGlyph {
    /// The face it came out of — not necessarily the face that was asked
    /// for, since shaping falls back per glyph for anything the requested
    /// family doesn't cover.
    pub face: FaceId,
    /// Index within `face`, not a character.
    pub glyph: u16,
    /// Pen position, from the run's left edge.
    pub x: f32,
    /// The advance the shaper gave it, before any synthetic effect.
    pub advance: f32,
    /// Byte offset in the source string of the cluster this glyph belongs
    /// to. Runs of glyphs can share one; a caller reconstructing text from
    /// glyphs (a PDF's `ToUnicode`, say) needs it to get "ﬁ" back out of
    /// one ligature.
    pub cluster: usize,
}

/// One string's worth of shaped glyphs, plus where they sit in the box
/// [`Layer::get_text_size`](super::Layer::get_text_size) reports.
#[derive(Clone, Debug)]
pub struct ShapedText {
    pub glyphs: Vec<ShapedGlyph>,
    /// The baseline's offset below the box's top edge. Text is positioned
    /// by its box (that is what [`super::Alignment`] resolves against), but
    /// drawn from its baseline.
    pub baseline: f32,
    /// Exactly what [`Layer::get_text_size`](super::Layer::get_text_size)
    /// would return for the same arguments — synthetic effects included,
    /// even though `glyphs` excludes them.
    pub size: (f32, f32),
}
