//! Which note lands on which page, and where on it.
//!
//! On screen the margin is one column as long as the document, stacked once
//! and scrolled past. A page has neither of those things, so a note has to
//! be told which sheet it belongs to and re-stacked against that sheet's
//! bottom — `docs/SPEC.md` §10: *a sidenote near a break travels with its
//! anchor*.
//!
//! Travelling with the anchor is the whole rule, and it makes this a pure
//! consequence of pagination rather than an input to it. The page break is
//! decided by the prose alone; a note follows whichever side of it its
//! anchor's line ended up on. Nothing here can move a line of text.

use crate::components::sidenotes;
use crate::document::layout::{self, DocLayout};
use crate::document::{Block, Document};
use crate::theme::TextStyle;

use super::paginate::Page;

/// One note, placed on a page.
pub struct Placed {
    /// The raised number its anchor draws. One string, derived once by
    /// `layout`, so the marker in the margin and the number in the prose
    /// cannot disagree.
    pub marker: String,
    /// The note's body, laid out at the margin's width and scale. A small
    /// document of its own, starting at `(0, 0)` like any other.
    pub layout: DocLayout,
    /// The note's top, in content-column pixels from the top of *this page*
    /// — the same frame a [`Piece`](super::paginate::Piece)'s `y` is in.
    pub y: f32,
}

/// Whether this document prints with a margin column at all.
///
/// An anchor whose note is missing does not count, and neither does a note
/// whose anchor the reader deleted: what earns the wider spread is a note
/// that will actually be drawn. Asked before the geometry is chosen, which
/// is why it reads the anchors rather than the placement — the placement
/// needs pages, and the pages need the geometry.
pub fn present(document: &Document, layout: &DocLayout) -> bool {
    layout
        .anchors
        .iter()
        .any(|anchor| body(document, &anchor.label).is_some())
}

/// The note body a label names, if the document still has one.
fn body<'a>(document: &'a Document, label: &str) -> Option<&'a [Block]> {
    document
        .notes
        .iter()
        .find(|note| note.label == label)
        .map(|note| note.body.as_slice())
}

/// Place every note beside the anchor that made it, one list per page.
///
/// `height` is the content column's, in pixels — what a note stack has to
/// fit inside. `measure` is the export's, the same one the prose was laid
/// out with, so a note's height is its own layout's rather than a guess.
pub fn place(
    document: &Document,
    layout: &DocLayout,
    pages: &[Page],
    height: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> Vec<Vec<Placed>> {
    pages
        .iter()
        .map(|page| on_page(document, layout, page, height, measure))
        .collect()
}

/// The notes one page carries, in document order and already stacked.
fn on_page(
    document: &Document,
    layout: &DocLayout,
    page: &Page,
    height: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> Vec<Placed> {
    let mut notes = Vec::new();
    // `(wanted y, height)` per note, in the same order — what `settle` takes.
    let mut wanted = Vec::new();

    for piece in &page.pieces {
        let laid = &layout.blocks[piece.block];
        let Some(first) = laid.lines.get(piece.lines.start) else {
            continue;
        };
        // Document y → page y. The same shift `paint` draws this piece's
        // own lines with, so a note starts level with the line it belongs
        // to and not with the line that line used to be.
        let dy = piece.y - first.y;

        for anchor in &layout.anchors {
            // An anchor belongs to this piece when it is on one of the lines
            // the piece actually carries — not merely in the same block. A
            // paragraph split across a break has anchors on both sides.
            let Some(line) = anchor.line else { continue };
            if anchor.block != piece.block || !piece.lines.contains(&line) {
                continue;
            }
            let Some(body) = body(document, &anchor.label) else {
                continue;
            };
            let laid =
                layout::layout_blocks(body, sidenotes::NOTE_WIDTH, sidenotes::SCALE, measure);
            wanted.push((anchor.y + dy, laid.height));
            notes.push(Placed {
                marker: anchor.number.clone(),
                layout: laid,
                y: 0.0,
            });
        }
    }

    for (note, y) in notes.iter_mut().zip(settle(&wanted, height)) {
        note.y = y;
    }
    notes
}

/// Stack notes wanting `(y, height)` into a column `height` tall.
///
/// The forward pass is the margin's own [`sidenotes::stack`] — each note at
/// its anchor, pushed down past the one above it, never up. Shared rather
/// than reimplemented, because two notes crowding each other must crowd the
/// same way on paper as on screen.
///
/// The backward pass is the page's addition. A screen scrolls, so a note
/// pushed past the bottom is still readable; a sheet ends, so the same note
/// is simply gone. When the stack overruns, it is pulled back up until it
/// fits — which can leave a note slightly *above* its anchor, the one place
/// the margin's rule is broken and the only alternative to losing it.
///
/// More notes than a page can hold is the last case: the column starts
/// flush at the top and the surplus overflows the bottom, visibly, the way
/// `paginate` lets an unfittable line overflow rather than hang.
fn settle(wanted: &[(f32, f32)], height: f32) -> Vec<f32> {
    let mut ys = sidenotes::stack(wanted, sidenotes::GAP);
    let Some(last) = ys.len().checked_sub(1) else {
        return ys;
    };
    if ys[last] + wanted[last].1 <= height {
        return ys;
    }

    ys[last] = height - wanted[last].1;
    for index in (0..last).rev() {
        ys[index] = ys[index].min(ys[index + 1] - wanted[index].1 - sidenotes::GAP);
    }

    // `ys` is increasing, so the first note is the only one that can have
    // been pushed off the top — and if it has, the column is overfull.
    if ys[0] < 0.0 {
        let lift = ys[0];
        for y in &mut ys {
            *y -= lift;
        }
    }
    ys
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::components::editor::MEASURE;
    use crate::document::{Inline, Sidenote, Style, Text};
    use crate::export::geometry::{PageGeometry, Paper};
    use crate::export::paginate::paginate;

    /// Every glyph ten wide, as everywhere else in the export's tests.
    fn measure(text: &str, _: &TextStyle) -> f32 {
        text.chars().count() as f32 * 10.0
    }

    fn text(s: &str) -> Inline {
        Inline::Text(Text {
            text: s.into(),
            style: Style::PLAIN,
        })
    }

    fn para(s: &str) -> Block {
        Block::Paragraph(vec![text(s)])
    }

    /// A document of `blocks`, with a note per `(label, body)`.
    fn document(blocks: Vec<Block>, notes: &[(&str, &str)]) -> Document {
        let mut d = Document::new(Path::new("notes/t.md"));
        *d.body_mut() = blocks;
        for (label, body) in notes {
            d.notes.push(Sidenote {
                label: (*label).into(),
                body: vec![para(body)],
                anchored: true,
            });
        }
        d
    }

    fn lay_out(document: &Document) -> DocLayout {
        layout::layout(document, MEASURE, &measure)
    }

    #[test]
    fn a_document_without_notes_prints_plain() {
        let d = document(vec![para("just prose")], &[]);
        assert!(!present(&d, &lay_out(&d)));
    }

    #[test]
    fn an_anchor_with_a_note_earns_the_margin() {
        let d = document(
            vec![Block::Paragraph(vec![
                text("see"),
                Inline::Note("a".into()),
            ])],
            &[("a", "the note")],
        );
        assert!(present(&d, &lay_out(&d)));
    }

    #[test]
    fn an_anchor_whose_note_is_gone_does_not() {
        // A stray anchor is not worth narrowing the text column for: there
        // is nothing to put beside it.
        let d = document(
            vec![Block::Paragraph(vec![
                text("see"),
                Inline::Note("a".into()),
            ])],
            &[],
        );
        assert!(!present(&d, &lay_out(&d)));
        let layout = lay_out(&d);
        let pages = paginate(&layout, &PageGeometry::noted(Paper::A4));
        let placed = place(&d, &layout, &pages, 1000.0, &measure);
        assert!(placed[0].is_empty());
    }

    #[test]
    fn a_note_travels_to_the_page_its_anchor_landed_on() {
        // Twelve paragraphs into a page that holds five of them: the anchor
        // on the ninth is on page two, and so is its note.
        let mut blocks: Vec<Block> = (0..12).map(|_| para("line")).collect();
        blocks[8] = Block::Paragraph(vec![text("here"), Inline::Note("a".into())]);
        let d = document(blocks, &[("a", "the note")]);
        let layout = lay_out(&d);

        let mut geometry = PageGeometry::noted(Paper::A4);
        geometry.content_height = layout.blocks[5].y;
        let pages = paginate(&layout, &geometry);
        assert!(pages.len() > 1, "the test needs a break to happen");

        let placed = place(&d, &layout, &pages, geometry.content_height, &measure);
        let carrying: Vec<usize> = placed
            .iter()
            .enumerate()
            .filter(|(_, notes)| !notes.is_empty())
            .map(|(page, _)| page)
            .collect();
        assert_eq!(carrying, vec![1], "the note must follow its anchor");
    }

    #[test]
    fn a_note_starts_level_with_its_anchor_on_the_page_it_landed_on() {
        // The y a note is placed at is in the *page's* frame, not the
        // document's — the same shift the painter draws that piece with.
        let mut blocks: Vec<Block> = (0..12).map(|_| para("line")).collect();
        blocks[8] = Block::Paragraph(vec![text("here"), Inline::Note("a".into())]);
        let d = document(blocks, &[("a", "the note")]);
        let layout = lay_out(&d);

        let mut geometry = PageGeometry::noted(Paper::A4);
        geometry.content_height = layout.blocks[5].y;
        let pages = paginate(&layout, &geometry);
        let placed = place(&d, &layout, &pages, geometry.content_height, &measure);

        let anchor = &layout.anchors[0];
        let piece = pages[1]
            .pieces
            .iter()
            .find(|piece| piece.block == 8)
            .expect("the anchored block is on page two");
        let dy = piece.y - layout.blocks[8].lines[piece.lines.start].y;
        assert_eq!(placed[1][0].y, anchor.y + dy);
        assert!(
            placed[1][0].y < layout.blocks[8].y,
            "a page-two note must not carry page one's offset"
        );
    }

    #[test]
    fn two_notes_on_one_page_stack_the_way_the_margin_stacks_them() {
        let d = document(
            vec![Block::Paragraph(vec![
                text("a"),
                Inline::Note("one".into()),
                text("b"),
                Inline::Note("two".into()),
            ])],
            &[("one", "first note"), ("two", "second note")],
        );
        let layout = lay_out(&d);
        let pages = paginate(&layout, &PageGeometry::noted(Paper::A4));
        let placed = place(&d, &layout, &pages, 1000.0, &measure);

        assert_eq!(placed[0].len(), 2);
        assert_eq!(placed[0][0].marker, "1");
        assert_eq!(placed[0][1].marker, "2");
        // Both anchors are on one line, so the second is pushed clear of the
        // first by exactly the margin's gap.
        assert_eq!(
            placed[0][1].y,
            placed[0][0].y + placed[0][0].layout.height + sidenotes::GAP
        );
    }

    #[test]
    fn a_lone_note_keeps_its_anchors_y() {
        assert_eq!(settle(&[(100.0, 20.0)], 1000.0), vec![100.0]);
    }

    #[test]
    fn an_empty_column_settles_to_nothing() {
        assert_eq!(settle(&[], 1000.0), Vec::<f32>::new());
    }

    #[test]
    fn a_stack_that_would_run_off_the_bottom_is_pulled_up() {
        // Two 40-tall notes anchored near the foot of a 100-tall column.
        // The forward pass would end at 130; the backward pass lands the
        // last flush on the bottom and lifts the first clear of it.
        let ys = settle(&[(80.0, 40.0), (90.0, 40.0)], 100.0);
        assert_eq!(ys, vec![8.0, 60.0]);
        assert!(
            ys[0] < 80.0,
            "the first note is pulled above its anchor — the price of not losing the second"
        );
    }

    #[test]
    fn a_stack_that_fits_is_left_alone() {
        // The mirror of the test above with room to spare: nothing moves,
        // and every note stays exactly at its own anchor.
        assert_eq!(
            settle(&[(10.0, 40.0), (80.0, 40.0)], 1000.0),
            vec![10.0, 80.0]
        );
    }

    #[test]
    fn an_overfull_column_starts_at_the_top_and_overflows_the_bottom() {
        // Three 50-tall notes will not fit a 100-tall column however they
        // are stacked. They start flush at the top rather than hanging off
        // it, and the surplus runs past the bottom where it can be seen.
        let ys = settle(&[(0.0, 50.0), (10.0, 50.0), (20.0, 50.0)], 100.0);
        assert_eq!(ys[0], 0.0);
        assert!(ys[2] + 50.0 > 100.0);
        for pair in ys.windows(2) {
            assert!(pair[1] >= pair[0] + 50.0, "notes must not overlap");
        }
    }
}
