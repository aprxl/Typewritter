//! Where the scroll gets cut into pages.
//!
//! The editor has one continuous column and no notion of a page; this is
//! the only stage in the pipeline that adds one. It reads a [`DocLayout`]
//! and produces, per page, a list of [`Piece`]s — *a block, a slice of its
//! lines, and where that slice lands on the sheet*.
//!
//! A piece is deliberately not "a block": a paragraph longer than a page
//! has to split, and making the split case a different type would give the
//! painter two paths through the same work. One type, one path, and a whole
//! block is just the piece whose range is all of its lines.

use std::ops::Range;

use crate::document::layout::DocLayout;

use super::geometry::PageGeometry;

/// A run of one block's lines, placed on a page.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    /// Index into [`DocLayout::blocks`] and [`DocLayout::source`] alike.
    pub block: usize,
    /// Which of that block's lines are on this page.
    pub lines: Range<usize>,
    /// Where the first of them sits, in content-column pixels from the top
    /// of *this page* — not of the document.
    pub y: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub pieces: Vec<Piece>,
}

/// Cut `layout` into pages that fit `geometry`.
///
/// Two rules today, and both are about height:
///
/// 1. A block goes on the current page if it fits, and starts a new one if
///    it doesn't.
/// 2. A block too tall for any page splits between its visual lines.
///
/// The gap between blocks is swallowed at a break, so a page never opens
/// with leading whitespace. The rules that keep display math, code blocks
/// and headings from being separated from what they belong to arrive with
/// those elements — see `PDF.md` §5.
///
/// Folded blocks are not consulted: export lays out its own `DocLayout`
/// from a fold-cleared document, so `BlockLayout::hidden` is always `None`
/// here and a collapsed section paginates like any other.
pub fn paginate(layout: &DocLayout, geometry: &PageGeometry) -> Vec<Page> {
    let height = geometry.content_height;
    let mut pages = vec![Page::default()];
    // Where the current page's top sits in document coordinates. A block
    // lands at `block.y - page_top`.
    let mut page_top = 0.0;

    for (index, block) in layout.blocks.iter().enumerate() {
        let mut line = 0;
        while line < block.lines.len() {
            // The top of the first line still to place — the block's own top
            // only until a split moves it — and the bottom of the last.
            let start_y = block.lines[line].y;
            let bottom = block
                .lines
                .last()
                .map_or(start_y, |last| last.y + last.height);

            if bottom - page_top <= height {
                pages
                    .last_mut()
                    .expect("a page always exists")
                    .pieces
                    .push(Piece {
                        block: index,
                        lines: line..block.lines.len(),
                        y: start_y - page_top,
                    });
                break;
            }

            // Doesn't fit whole. How many of its lines do? Never all of
            // them: the last line's bottom is the block's, which is what
            // just failed to fit.
            let mut fits = block.lines[line..]
                .iter()
                .take_while(|l| l.y + l.height - page_top <= height)
                .count();

            // A line taller than an entire page fits nowhere, and breaking
            // to a fresh one would come straight back here — the export
            // would never finish. Once this line already has a page to
            // itself, it takes it and overflows the bottom margin: one
            // overset line, rather than a hang.
            if fits == 0 && page_top >= start_y {
                fits = 1;
            }

            if fits > 0 {
                pages
                    .last_mut()
                    .expect("a page always exists")
                    .pieces
                    .push(Piece {
                        block: index,
                        lines: line..line + fits,
                        y: start_y - page_top,
                    });
                line += fits;
            }

            // Break. The next page opens flush at the next line's top — the
            // gap that would have preceded it is what a page break replaces.
            if line < block.lines.len() {
                page_top = block.lines[line].y;
                pages.push(Page::default());
            }
        }
    }
    pages
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::document::Block;
    use crate::document::layout::{BlockLayout, VisLine};

    /// A layout of `heights`-tall single-line blocks stacked flush.
    fn stacked(heights: &[f32]) -> DocLayout {
        let mut y = 0.0;
        let mut blocks = Vec::new();
        for &height in heights {
            blocks.push(BlockLayout {
                y,
                lines: vec![VisLine {
                    y,
                    x: 0.0,
                    height,
                    segments: Vec::new(),
                }],
                height,
                hidden: None,
                indicator: None,
            });
            y += height;
        }
        DocLayout {
            blocks,
            height: y,
            scale: 1.0,
            source: heights
                .iter()
                .map(|_| Block::Paragraph(Vec::new()))
                .collect(),
            anchors: Vec::new(),
            equation_numbers: HashMap::new(),
        }
    }

    /// One block of `lines` lines, each `height` tall.
    fn tall(lines: usize, height: f32) -> DocLayout {
        let lines: Vec<VisLine> = (0..lines)
            .map(|index| VisLine {
                y: index as f32 * height,
                x: 0.0,
                height,
                segments: Vec::new(),
            })
            .collect();
        let total = lines.len() as f32 * height;
        DocLayout {
            blocks: vec![BlockLayout {
                y: 0.0,
                lines,
                height: total,
                hidden: None,
                indicator: None,
            }],
            height: total,
            scale: 1.0,
            source: vec![Block::Paragraph(Vec::new())],
            anchors: Vec::new(),
            equation_numbers: HashMap::new(),
        }
    }

    fn geometry(height: f32) -> PageGeometry {
        let mut geometry = PageGeometry::plain(super::super::geometry::Paper::A4);
        geometry.content_height = height;
        geometry
    }

    #[test]
    fn a_document_that_fits_is_one_page() {
        let pages = paginate(&stacked(&[30.0, 30.0, 30.0]), &geometry(100.0));
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].pieces.len(), 3);
    }

    #[test]
    fn a_block_that_does_not_fit_starts_the_next_page() {
        let pages = paginate(&stacked(&[30.0, 30.0, 30.0, 30.0]), &geometry(70.0));
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].pieces.len(), 2);
        assert_eq!(pages[1].pieces.len(), 2);
    }

    #[test]
    fn a_page_opens_flush_at_its_first_block() {
        let pages = paginate(&stacked(&[30.0, 30.0, 30.0, 30.0]), &geometry(70.0));
        assert_eq!(
            pages[1].pieces[0].y, 0.0,
            "a page must not open with the gap the break replaced"
        );
    }

    #[test]
    fn a_block_taller_than_a_page_splits_between_its_lines() {
        let pages = paginate(&tall(10, 30.0), &geometry(100.0));
        assert_eq!(pages.len(), 4);
        assert_eq!(pages[0].pieces[0].lines, 0..3);
        assert_eq!(pages[1].pieces[0].lines, 3..6);
        assert_eq!(pages[3].pieces[0].lines, 9..10);
    }

    #[test]
    fn every_line_lands_on_exactly_one_page() {
        let pages = paginate(&tall(10, 30.0), &geometry(100.0));
        let placed: Vec<usize> = pages
            .iter()
            .flat_map(|page| page.pieces.iter())
            .flat_map(|piece| piece.lines.clone())
            .collect();
        assert_eq!(placed, (0..10).collect::<Vec<_>>());
    }

    #[test]
    fn a_break_at_the_last_line_leaves_no_empty_page() {
        // Nine 30px lines into 90px pages: three full pages, the last of
        // them ending exactly on a break.
        let pages = paginate(&tall(9, 30.0), &geometry(90.0));
        assert_eq!(pages.len(), 3);
        assert!(pages.iter().all(|page| !page.pieces.is_empty()));
    }

    #[test]
    fn a_line_taller_than_the_page_gets_one_and_does_not_hang() {
        // Would loop forever if an unfittable line kept asking for a fresh
        // page: it takes the page it is on and overflows.
        let pages = paginate(&tall(3, 200.0), &geometry(100.0));
        assert_eq!(pages.len(), 3);
        for (index, page) in pages.iter().enumerate() {
            assert_eq!(page.pieces[0].lines, index..index + 1);
            assert_eq!(page.pieces[0].y, 0.0);
        }
    }
}
