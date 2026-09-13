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

use crate::document::Block;
use crate::document::layout::{DocLayout, VisLine};

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
/// The document is one column of visual lines, and the only question this
/// asks is where that column may be cut. Between two adjacent lines there
/// is either a *break opportunity* or there is not ([`breakable`]); the run
/// of lines between two opportunities is a chunk, and a chunk lands whole
/// on one page or starts the next. Every keep-rule in `PDF.md` §5 is a
/// statement about where an opportunity is missing, so all four live in one
/// predicate rather than in four special cases here.
///
/// The gap between blocks is swallowed at a break, so a page never opens
/// with leading whitespace.
///
/// Folded blocks are not consulted: export lays out its own `DocLayout`
/// from a fold-cleared document, so `BlockLayout::hidden` is always `None`
/// here and a collapsed section paginates like any other.
pub fn paginate(layout: &DocLayout, geometry: &PageGeometry) -> Vec<Page> {
    let height = geometry.content_height;
    let lines = self::lines(layout);
    let mut pages = vec![Page::default()];
    // Where the current page's top sits in document coordinates. A line
    // lands at `line.y - page_top`.
    let mut page_top = 0.0;

    for (index, &at) in lines.iter().enumerate() {
        let top = line(layout, at).y;
        // Break before this line only where a break is allowed — which is
        // to say only when it opens a chunk, since `demand` measures from
        // here to the next opportunity and a mid-chunk line's demand was
        // already paid for by the line that opened it.
        //
        // The second half of the test is the hang guard: a chunk taller
        // than a whole sheet fits nowhere, and breaking to a fresh page
        // would come straight back here. Once it has a page to itself it
        // takes it, and `demand` drops to one line so the rest of it packs
        // line by line rather than running off the bottom of the sheet.
        if demand(layout, &lines, index, height) - page_top > height && page_top < top {
            page_top = top;
            pages.push(Page::default());
        }

        let page = pages.last_mut().expect("a page always exists");
        place(page, layout, at, page_top);
    }
    pages
}

/// How far down the document must fit on this page for the line at `index`
/// to start here: the bottom of its chunk, which is everything up to the
/// next place a break is allowed.
///
/// A chunk taller than the whole sheet is the exception, and asks only for
/// its own first line. Nothing can keep it together, and a chunk that
/// insisted anyway would take one page and overflow it — one lost equation
/// against one that prints in two halves.
fn demand(layout: &DocLayout, lines: &[(usize, usize)], index: usize, height: f32) -> f32 {
    let mut end = index + 1;
    while end < lines.len() && !breakable(layout, lines[end - 1], lines[end]) {
        end += 1;
    }
    let first = line(layout, lines[index]);
    let last = line(layout, lines[end - 1]);
    if last.y + last.height - first.y <= height {
        last.y + last.height
    } else {
        first.y + first.height
    }
}

/// Every visual line in visual order, as `(block, line)`. Widget walls may
/// place a later source marker beside prose that started after the first row,
/// so source order alone is no longer the document's paint order.
fn lines(layout: &DocLayout) -> Vec<(usize, usize)> {
    let mut lines = layout
        .blocks
        .iter()
        .enumerate()
        .flat_map(|(block, laid)| (0..laid.lines.len()).map(move |line| (block, line)))
        .collect::<Vec<_>>();
    lines.sort_by(|left, right| {
        line(layout, *left)
            .y
            .total_cmp(&line(layout, *right).y)
            .then_with(|| left.cmp(right))
    });
    lines
}

fn line(layout: &DocLayout, at: (usize, usize)) -> &VisLine {
    &layout.blocks[at.0].lines[at.1]
}

/// Whether a page may break between two adjacent lines — `PDF.md` §5's
/// keep-rules, all four of them, said once.
fn breakable(layout: &DocLayout, previous: (usize, usize), next: (usize, usize)) -> bool {
    if layout.widget_rows.iter().flatten().any(|row| {
        row.wall.is_some_and(|wall| {
            let previous = line(layout, previous).y;
            let next = line(layout, next).y;
            wall.y <= previous && previous < wall.bottom() && wall.y <= next && next < wall.bottom()
        })
    }) {
        return false;
    }
    let source = &layout.source;
    if previous.0 == next.0 {
        // Inside one block, only prose splits. Notation broken across a
        // sheet is not a smaller equation but two wrong ones; a fence cut
        // in half loses the slab that says it is one; and a heading split
        // across a break ends a page, which is what rule 4 forbids.
        return matches!(
            source[previous.0],
            Block::Paragraph(_) | Block::ListItem { .. }
        );
    }
    // Between blocks: a heading keeps with whatever follows it, and a
    // fenced block's continuation lines keep with the line that opened it —
    // a fence is a *run* of `CodeLine` blocks rather than one block, and
    // this is where the run is found.
    !source[previous.0].is_heading()
        && !matches!(source[next.0], Block::CodeLine { first: false, .. })
}

/// Put one line on `page`, extending the piece it continues rather than
/// starting a second one.
///
/// A block's lines on one page are one piece, which is what lets a painter
/// draw a slab under all of them at once instead of notching a rounded
/// corner at every line join.
fn place(page: &mut Page, layout: &DocLayout, at: (usize, usize), page_top: f32) {
    let (block, row) = at;
    if let Some(piece) = page.pieces.last_mut()
        && piece.block == block
        && piece.lines.end == row
    {
        piece.lines.end = row + 1;
        return;
    }
    page.pieces.push(Piece {
        block,
        lines: row..row + 1,
        y: line(layout, at).y - page_top,
    });
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::document::layout::{BlockLayout, VisLine};
    use crate::document::widget_layout::WidgetRowLayout;
    use crate::layout::Rect;

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
                    width: 500.0,
                    height,
                    segments: Vec::new(),
                    cell_line: 0,
                }],
                height,
                hidden: None,
                indicator: None,
            });
            y += height;
        }
        DocLayout {
            blocks,
            tables: vec![None; heights.len()],
            widget_rows: vec![None; heights.len()],
            height: y,
            scale: 1.0,
            code_colors: vec![Vec::new(); heights.len()],
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
                width: 500.0,
                height,
                segments: Vec::new(),
                cell_line: 0,
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
            tables: vec![None],
            widget_rows: vec![None],
            height: total,
            scale: 1.0,
            source: vec![Block::Paragraph(Vec::new())],
            code_colors: vec![Vec::new()],
            anchors: Vec::new(),
            equation_numbers: HashMap::new(),
        }
    }

    #[test]
    fn lines_follow_visual_order_when_a_widget_wall_overlaps_source_blocks() {
        let mut layout = stacked(&[240.0, 210.0, 112.0]);
        layout.blocks[1].y = 10.0;
        layout.blocks[1].lines[0].y = 10.0;
        layout.blocks[2].y = 254.0;
        layout.blocks[2].lines[0].y = 254.0;
        assert_eq!(lines(&layout), vec![(0, 0), (1, 0), (2, 0)]);

        layout.blocks[1].lines.push(VisLine {
            y: 280.0,
            x: 100.0,
            width: 300.0,
            height: 30.0,
            segments: Vec::new(),
            cell_line: 0,
        });
        assert_eq!(lines(&layout), vec![(0, 0), (1, 0), (2, 0), (1, 1)]);
    }

    #[test]
    fn a_widget_wall_moves_to_the_next_page_whole() {
        let mut layout = stacked(&[60.0, 90.0, 90.0]);
        let wall = Rect::new(0.0, 60.0, 300.0, 180.0);
        for index in [1, 2] {
            layout.widget_rows[index] = Some(WidgetRowLayout {
                tracks: Vec::new(),
                cards: Vec::new(),
                lane: None,
                wall: Some(wall),
                lane_has_content: true,
                height: 90.0,
                scale: 1.0,
            });
        }

        let pages = paginate(&layout, &geometry(200.0));
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].pieces[0].block, 0);
        assert_eq!(
            pages[1]
                .pieces
                .iter()
                .map(|piece| piece.block)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    fn geometry(height: f32) -> PageGeometry {
        let mut geometry = PageGeometry::plain(super::super::geometry::Paper::A4);
        geometry.content_height = height;
        geometry
    }

    /// [`stacked`] with each block's kind said out loud: one 30px line
    /// apiece, which is all the keep-rules read.
    fn of(source: Vec<Block>) -> DocLayout {
        let mut layout = stacked(&vec![30.0; source.len()]);
        layout.source = source;
        layout
    }

    fn para() -> Block {
        Block::Paragraph(Vec::new())
    }

    fn heading() -> Block {
        Block::Heading {
            level: 1,
            content: Vec::new(),
            folded: false,
        }
    }

    fn code(first: bool) -> Block {
        Block::CodeLine {
            content: Vec::new(),
            first,
            lang: None,
        }
    }

    #[test]
    fn a_fenced_block_moves_to_the_next_page_whole() {
        // A paragraph, then a three-line fence, with room for three lines
        // on the sheet: two of the fence would fit, and half a slab is not
        // a code block.
        let layout = of(vec![para(), code(true), code(false), code(false)]);
        let pages = paginate(&layout, &geometry(100.0));
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].pieces.len(), 1, "only the paragraph stays behind");
        assert_eq!(pages[1].pieces.len(), 3, "the fence travels whole");
    }

    #[test]
    fn a_fence_taller_than_a_page_splits_rather_than_running_off_it() {
        // Nothing can keep a five-line fence together on a three-line
        // sheet. It packs line by line instead: two halves of a slab beat
        // two lines drawn past the bottom edge of the paper.
        let layout = of(vec![
            code(true),
            code(false),
            code(false),
            code(false),
            code(false),
        ]);
        let pages = paginate(&layout, &geometry(100.0));
        let placed: Vec<usize> = pages
            .iter()
            .flat_map(|page| page.pieces.iter())
            .map(|piece| piece.block)
            .collect();
        assert_eq!(pages.len(), 2);
        assert_eq!(placed, vec![0, 1, 2, 3, 4], "no line of it is lost");
    }

    #[test]
    fn a_heading_never_ends_a_page() {
        // Two paragraphs, a heading, then its body, with room for three
        // lines: the heading fits at the bottom and the first line under it
        // does not, so both move.
        let layout = of(vec![para(), para(), heading(), para()]);
        let pages = paginate(&layout, &geometry(100.0));
        assert_eq!(pages.len(), 2);
        assert_eq!(
            pages[0].pieces.len(),
            2,
            "the heading travels with its body"
        );
        assert_eq!(pages[1].pieces.len(), 2);
    }

    #[test]
    fn a_heading_with_nothing_after_it_asks_for_nothing_more() {
        // Keep-with-next has nothing to keep the last block with, and must
        // not read past the end of the document looking for it.
        let layout = of(vec![para(), para(), heading()]);
        let pages = paginate(&layout, &geometry(100.0));
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].pieces.len(), 3);
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
    fn display_math_moves_to_the_next_page_whole() {
        // Three 30px lines of notation, with 40px of the page left under
        // them: two lines would fit, and a split equation is two wrong ones.
        let mut layout = tall(3, 30.0);
        layout.source = vec![Block::Math {
            list: Vec::new(),
            tag: None,
        }];
        for (index, line) in layout.blocks[0].lines.iter_mut().enumerate() {
            line.y = 40.0 + index as f32 * 30.0;
        }
        layout.blocks[0].y = 40.0;

        let pages = paginate(&layout, &geometry(100.0));
        assert_eq!(pages.len(), 2);
        assert!(
            pages[0].pieces.is_empty(),
            "the equation must not leave two of its lines behind"
        );
        assert_eq!(pages[1].pieces[0].lines, 0..3);
    }

    #[test]
    fn a_paragraph_in_the_same_place_does_split() {
        // The mirror of the test above, differing only in the block kind:
        // what keeps the equation together is the rule, not the geometry.
        let mut layout = tall(3, 30.0);
        for (index, line) in layout.blocks[0].lines.iter_mut().enumerate() {
            line.y = 40.0 + index as f32 * 30.0;
        }
        layout.blocks[0].y = 40.0;

        let pages = paginate(&layout, &geometry(100.0));
        assert_eq!(pages[0].pieces[0].lines, 0..2);
        assert_eq!(pages[1].pieces[0].lines, 2..3);
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
