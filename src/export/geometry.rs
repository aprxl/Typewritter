//! Where the editor's column lands on a sheet of paper.
//!
//! The only file in the pipeline that says `pt`. Everything upstream —
//! pagination, painting, every constant in `paint.rs` — is in the editor's
//! logical pixels with the editor's own top-left origin and y pointing
//! down; [`PageGeometry::point`] is the single place that changes, and it
//! is called by the backend and nobody else.

use crate::components::editor::{MEASURE, RIGHT_MARGIN};
use crate::components::sidenotes;

/// A sheet, in points. Portrait; a landscape variant is `flipped`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Paper {
    pub width: f32,
    pub height: f32,
}

impl Paper {
    pub const A4: Self = Self {
        width: 595.276,
        height: 841.890,
    };

    /// The same sheet turned on its side — the escape hatch when a layout
    /// has to fit more across than a portrait page can hold at a readable
    /// size.
    pub fn flipped(self) -> Self {
        Self {
            width: self.height,
            height: self.width,
        }
    }
}

/// Points per logical pixel for a document without sidenotes.
///
/// The editor's column is a fixed [`MEASURE`] logical pixels wide, so the
/// export lays out at exactly that width and inherits the editor's line
/// breaks character for character. This number is only how large that
/// column prints: 634 px × 0.63 is a 399 pt column with 11 pt body text,
/// which reads as a book rather than as a screenshot of one.
const PLAIN_SCALE: f32 = 0.63;

/// Space above and below the text column.
const VERTICAL_MARGIN: f32 = 72.0;

/// Space either side of the *spread* — text column plus margin column — on a
/// page that carries sidenotes. Tighter than the plain page's, because the
/// spread is 38% wider than the column and the margins are what pays for it.
const NOTED_SIDE: f32 = 40.0;

/// Where the margin column starts, in content-column pixels from the text
/// column's left edge — the same offset the screen's margin sits at, since
/// it is the editor's own column width plus the gap it wraps short of.
///
/// Only a [`PageGeometry::noted`] page has one. On a plain page nothing is
/// drawn out here, so nothing asks.
pub const NOTE_COLUMN: f32 = MEASURE + RIGHT_MARGIN;

/// One page's frame: the sheet, the margins, and the scale that turns the
/// editor's pixels into its points.
#[derive(Clone, Copy, Debug)]
pub struct PageGeometry {
    pub paper: Paper,
    /// Points per logical pixel. Uniform — it scales type, leading, and
    /// every gap together, so the page is the editor's proportions at a
    /// different size and never a different layout.
    pub scale: f32,
    /// The content column's top-left on the sheet, in points.
    pub origin: (f32, f32),
    /// How tall the content column is, in *pixels* — the unit pagination
    /// works in.
    pub content_height: f32,
    /// How wide the content column is, in pixels. What the document is laid
    /// out at.
    pub content_width: f32,
}

impl PageGeometry {
    /// A document with no sidenotes: the text column centred on the sheet.
    pub fn plain(paper: Paper) -> Self {
        let column = MEASURE * PLAIN_SCALE;
        Self {
            paper,
            scale: PLAIN_SCALE,
            origin: ((paper.width - column) * 0.5, VERTICAL_MARGIN),
            content_height: (paper.height - VERTICAL_MARGIN * 2.0) / PLAIN_SCALE,
            content_width: MEASURE,
        }
    }

    /// A document with sidenotes: the text column pushed left to make room
    /// for the margin column beside it, and the pair fitted to the sheet.
    ///
    /// The scale here is *derived*, not chosen — the spread is a fixed number
    /// of pixels wide and the paper is a fixed number of points, so there is
    /// exactly one number that makes them meet. It comes out around `0.59`
    /// against `plain`'s `0.63`: a page carrying sidenotes carries more, and
    /// prints smaller for it. If that reads too small the escape hatch is the
    /// paper, not the scale — `Paper::flipped`, or a larger sheet.
    ///
    /// `content_width` stays [`MEASURE`]. The spread is wider, but the text
    /// column is not: it is the editor's, character for character, and the
    /// margin hangs off its right edge rather than eating into it.
    pub fn noted(paper: Paper) -> Self {
        let spread = NOTE_COLUMN + sidenotes::WIDTH;
        let scale = (paper.width - NOTED_SIDE * 2.0) / spread;
        Self {
            paper,
            scale,
            origin: (NOTED_SIDE, VERTICAL_MARGIN),
            content_height: (paper.height - VERTICAL_MARGIN * 2.0) / scale,
            content_width: MEASURE,
        }
    }

    /// A point on the page, from a point in the content column. The one
    /// conversion in the pipeline.
    pub fn point(&self, at: (f32, f32)) -> (f32, f32) {
        (
            self.origin.0 + at.0 * self.scale,
            self.origin.1 + at.1 * self.scale,
        )
    }

    /// A length on the page, from a length in the content column.
    pub fn length(&self, px: f32) -> f32 {
        px * self.scale
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_column_is_centred_on_the_sheet() {
        let geometry = PageGeometry::plain(Paper::A4);
        let left = geometry.point((0.0, 0.0)).0;
        let right = geometry.point((MEASURE, 0.0)).0;
        assert!(
            (left - (Paper::A4.width - right)).abs() < 0.01,
            "margins differ: {left} vs {}",
            Paper::A4.width - right
        );
    }

    #[test]
    fn the_content_box_fits_between_the_vertical_margins() {
        let geometry = PageGeometry::plain(Paper::A4);
        let bottom = geometry.point((0.0, geometry.content_height)).1;
        assert!(
            (bottom - (Paper::A4.height - VERTICAL_MARGIN)).abs() < 0.01,
            "content runs to {bottom}, not the bottom margin"
        );
    }

    #[test]
    fn a_noted_spread_is_centred_on_the_sheet() {
        let geometry = PageGeometry::noted(Paper::A4);
        let left = geometry.point((0.0, 0.0)).0;
        let right = geometry.point((NOTE_COLUMN + sidenotes::WIDTH, 0.0)).0;
        assert!(
            (left - (Paper::A4.width - right)).abs() < 0.01,
            "the margin column must be inside the sheet, not hanging off it: \
             {left} vs {}",
            Paper::A4.width - right
        );
    }

    #[test]
    fn a_noted_page_prints_smaller_but_at_the_same_measure() {
        let plain = PageGeometry::plain(Paper::A4);
        let noted = PageGeometry::noted(Paper::A4);
        assert!(
            noted.scale < plain.scale,
            "a spread carries more and must print smaller for it"
        );
        assert_eq!(
            noted.content_width, plain.content_width,
            "the text column is the editor's either way — only the sheet changes"
        );
        assert!(
            noted.content_height > plain.content_height,
            "smaller type puts more lines on a page"
        );
    }

    #[test]
    fn a_flipped_sheet_swaps_its_sides() {
        assert_eq!(Paper::A4.flipped().width, Paper::A4.height);
        assert_eq!(Paper::A4.flipped().height, Paper::A4.width);
    }
}
