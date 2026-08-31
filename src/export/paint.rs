//! [`DocLayout`] → [`Canvas`]. The extension point: a new element is an arm
//! in [`block`], and nothing else.
//!
//! Every function here is the corresponding stretch of
//! `components/editor.rs` with the session taken out — no caret, no
//! selection, no current-line band, no hover, no scroll culling, no fold
//! chevron. Those are statements about someone editing, and a page has
//! nobody editing it. What is left is the document, and it is drawn from
//! the same `DocLayout`, with the same constants, at the same coordinates.
//!
//! Read `PDF.md` §6 before adding to this file; it is four steps and the
//! first one is "don't touch `canvas.rs`".

use crate::components::editor::VIEW_CAP;
use crate::document::layout::{self, DocLayout, NUMBER_GUTTER, NUMBER_SIZE};
use crate::document::{ATOM, Block, Inline};
use crate::renderer::Rounding;
use crate::theme::{self, TextStyle};

use super::canvas::Canvas;
use super::paginate::{Page, Piece};

/// A divider's hairline. Not rounded to a whole pixel the way the editor
/// rounds it: that is a defence against a 1px rule smearing across two rows
/// of a screen's grid, and a page has no such grid to land on.
const RULE_THICKNESS: f32 = 1.0;

/// Draw one page's worth of pieces. `width` is the content column's, which
/// is what a rule spans and what a right-aligned annotation hangs off.
pub fn page(
    canvas: &mut dyn Canvas,
    layout: &DocLayout,
    page: &Page,
    numbers: &[Option<String>],
    width: f32,
) {
    for piece in &page.pieces {
        block(canvas, layout, piece, numbers, width);
    }
}

/// One block's decorations, then its text.
///
/// `piece.y` is where the block's first *visible on this page* line lands,
/// so everything below works in that line's frame rather than the
/// document's — a paragraph split across a break draws its second half at
/// the top of page two with no special case.
fn block(
    canvas: &mut dyn Canvas,
    layout: &DocLayout,
    piece: &Piece,
    numbers: &[Option<String>],
    width: f32,
) {
    let kind = &layout.source[piece.block];
    let laid = &layout.blocks[piece.block];
    let Some(first) = laid.lines.get(piece.lines.start) else {
        return;
    };
    // Document y → page y, for every line in this piece.
    let dy = piece.y - first.y;

    match kind {
        // The auto-number hung in the margin: virtual, so it is drawn
        // rather than laid out, and only beside the heading's first line.
        // The fold chevron beside it is not drawn — it is a click target,
        // and a page cannot be clicked.
        Block::Heading { .. } => {
            if piece.lines.start == 0
                && let Some(number) = numbers.get(piece.block).and_then(Option::as_ref)
            {
                let baseline = first.y + dy + first.height * 0.5;
                canvas.draw_text(
                    number,
                    (-NUMBER_GUTTER, baseline),
                    &TextStyle::mono(NUMBER_SIZE, theme::non_text()),
                    theme::RIGHT,
                );
            }
        }
        // A rule has no runs to paint, so the block *is* its decoration: a
        // hairline centred in its own short band, spanning the text column
        // and nothing more.
        Block::Divider(_) => {
            let y = first.y + dy + first.height * 0.5;
            canvas.draw_rectangle(
                (0.0, y),
                (width, RULE_THICKNESS),
                theme::border(),
                Rounding::NONE,
            );
        }
        // Everything below draws its text and, for now, none of its own
        // furniture — the tints, the markers, the bands. Each is one arm
        // here and one row of `PDF.md` §6's table; until then a code block
        // prints as correctly-styled code without its slab, which is
        // incomplete rather than wrong.
        Block::Paragraph(_)
        | Block::Math { .. }
        | Block::CodeLine { .. }
        | Block::ListItem { .. } => {}
    }

    for index in piece.lines.clone() {
        let Some(line) = laid.lines.get(index) else {
            continue;
        };
        self::line(canvas, layout, piece.block, line, dy);
    }
}

/// One visual line's runs. The shared path every block's text takes, and
/// where a new [`Inline`] variant gets its arm.
fn line(
    canvas: &mut dyn Canvas,
    layout: &DocLayout,
    index: usize,
    line: &crate::document::layout::VisLine,
    dy: f32,
) {
    let kind = &layout.source[index];
    // Not a typographic baseline: the editor draws text vertically centred
    // on the line, and this is that centre. `Canvas::draw_text` recovers the
    // real baseline from the shaped run.
    let baseline = line.y + dy + line.height * 0.5;
    let mut cursor = line.x;

    for segment in &line.segments {
        let run = &kind.inlines()[segment.inline];
        let text: String = match run {
            Inline::Text(t) => t
                .text
                .chars()
                .skip(segment.start)
                .take(segment.len.min(VIEW_CAP))
                .collect(),
            Inline::Math(_) => ATOM.to_string(),
            // An anchor and a reference each draw their derived number, not
            // what the author stored — carried on the segment so the drawing
            // and `advance` read one value.
            Inline::Note(_) | Inline::EqRef(_) => segment.number.clone().unwrap_or_default(),
        };
        let width = layout::advance(
            run,
            &text,
            kind,
            segment.style,
            segment.number.as_deref(),
            layout.scale,
            &|text, style| canvas.measure(text, style),
        );

        match run {
            Inline::Text(_) => {
                let style = layout::text_style(kind, segment.style, layout.scale);
                canvas.draw_text(&text, (cursor, baseline), &style, theme::LEFT);
            }
            // Notation, anchors and references each have their own drawing
            // and their own decorations — see `PDF.md` §6. Their width is
            // already reserved above, so the runs around them sit where the
            // editor puts them even before they draw anything.
            Inline::Math(_) | Inline::Note(_) | Inline::EqRef(_) => {}
        }
        cursor += width;
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::document::layout::VisLine;
    use crate::document::{Document, Style, Text};
    use crate::renderer::{Alignment, Color, HorizontalAlign, PathPaint, VerticalAlign};

    /// Every glyph is ten wide — the same fake `components/editor.rs`'s own
    /// layout tests measure with, so a line's arithmetic stays arithmetic
    /// rather than a font's opinion.
    const GLYPH: f32 = 10.0;

    #[derive(Debug, PartialEq)]
    enum Call {
        Text {
            text: String,
            at: (f32, f32),
            size: f32,
            bold: bool,
            align: Alignment,
        },
        Rectangle {
            at: (f32, f32),
            size: (f32, f32),
        },
    }

    /// A canvas that draws nothing and remembers everything. What makes the
    /// painter testable with no GPU under it.
    #[derive(Default)]
    struct Recorder {
        calls: Vec<Call>,
    }

    impl Recorder {
        fn texts(&self) -> Vec<&Call> {
            self.calls
                .iter()
                .filter(|call| matches!(call, Call::Text { .. }))
                .collect()
        }

        fn saying(&self, wanted: &str) -> Option<&Call> {
            self.calls
                .iter()
                .find(|call| matches!(call, Call::Text { text, .. } if text == wanted))
        }
    }

    impl Canvas for Recorder {
        fn draw_rectangle(&mut self, at: (f32, f32), size: (f32, f32), _: Color, _: Rounding) {
            self.calls.push(Call::Rectangle { at, size });
        }

        fn draw_circle(&mut self, _: (f32, f32), _: f32, _: Color) {}

        fn draw_path(&mut self, _: &str, _: (f32, f32), _: f32, _: f32, _: &PathPaint) {}

        fn draw_text(&mut self, text: &str, at: (f32, f32), style: &TextStyle, align: Alignment) {
            self.calls.push(Call::Text {
                text: text.to_string(),
                at,
                size: style.size,
                bold: style.weight > 0.0,
                align,
            });
        }

        fn measure(&self, text: &str, _: &TextStyle) -> f32 {
            text.chars().count() as f32 * GLYPH
        }
    }

    fn text(value: &str) -> Inline {
        Inline::Text(Text {
            text: value.into(),
            style: Style::PLAIN,
        })
    }

    fn laid_out(blocks: Vec<Block>, width: f32) -> DocLayout {
        let mut document = Document::new(Path::new("notes/test.md"));
        *document.body_mut() = blocks;
        layout::layout(&document, width, &|value, _| {
            value.chars().count() as f32 * GLYPH
        })
    }

    /// Every block on one page, the way `paginate` hands them over when the
    /// whole document fits.
    fn whole(layout: &DocLayout) -> Page {
        Page {
            pieces: layout
                .blocks
                .iter()
                .enumerate()
                .map(|(block, laid)| Piece {
                    block,
                    lines: 0..laid.lines.len(),
                    y: laid.lines.first().map_or(0.0, |line| line.y),
                })
                .collect(),
        }
    }

    fn paint(layout: &DocLayout, page: &Page, width: f32) -> Recorder {
        let mut recorder = Recorder::default();
        let numbers = super::super::heading_numbers(layout);
        super::page(&mut recorder, layout, page, &numbers, width);
        recorder
    }

    #[test]
    fn a_paragraph_starts_at_the_columns_left_edge() {
        let layout = laid_out(vec![Block::Paragraph(vec![text("hello")])], 500.0);
        let painted = paint(&layout, &whole(&layout), 500.0);
        let [Call::Text { text, at, .. }] = painted.texts()[..] else {
            panic!("expected one run, got {:?}", painted.calls);
        };
        assert_eq!(text, "hello");
        assert_eq!(at.0, 0.0, "the content column is the origin, not the pane");
    }

    #[test]
    fn a_heading_is_set_bold_and_larger_than_its_body() {
        let layout = laid_out(
            vec![
                Block::Heading {
                    level: 1,
                    folded: false,
                    content: vec![text("Title")],
                },
                Block::Paragraph(vec![text("body")]),
            ],
            500.0,
        );
        let painted = paint(&layout, &whole(&layout), 500.0);
        let (Some(Call::Text { size: h, bold, .. }), Some(Call::Text { size: b, .. })) =
            (painted.saying("Title"), painted.saying("body"))
        else {
            panic!("both runs are painted, got {:?}", painted.calls);
        };
        assert!(*bold, "a heading is always bold");
        assert!(h > b, "a heading sets larger than body text: {h} vs {b}");
    }

    #[test]
    fn a_headings_number_hangs_in_the_margin_right_aligned() {
        let layout = laid_out(
            vec![Block::Heading {
                level: 1,
                folded: false,
                content: vec![text("Title")],
            }],
            500.0,
        );
        let painted = paint(&layout, &whole(&layout), 500.0);
        let Some(Call::Text { at, align, .. }) = painted.saying("1") else {
            panic!("a heading carries its auto-number, got {:?}", painted.calls);
        };
        assert!(
            at.0 < 0.0,
            "the number hangs left of the column, not at {}",
            at.0
        );
        assert_eq!(align.horizontal, HorizontalAlign::Right);
    }

    #[test]
    fn a_number_is_drawn_once_however_far_its_heading_wraps() {
        let layout = laid_out(
            vec![Block::Heading {
                level: 1,
                folded: false,
                content: vec![text("aaaa bbbb cccc")],
            }],
            60.0,
        );
        assert!(
            layout.blocks[0].lines.len() > 1,
            "the fixture must actually wrap"
        );
        let painted = paint(&layout, &whole(&layout), 60.0);
        let numbers = painted
            .texts()
            .into_iter()
            .filter(|call| matches!(call, Call::Text { text, .. } if text == "1"))
            .count();
        assert_eq!(numbers, 1);
    }

    #[test]
    fn text_is_centred_on_its_line_the_way_the_editor_centres_it() {
        let layout = laid_out(vec![Block::Paragraph(vec![text("hello")])], 500.0);
        let line: &VisLine = &layout.blocks[0].lines[0];
        let painted = paint(&layout, &whole(&layout), 500.0);
        let [Call::Text { at, align, .. }] = painted.texts()[..] else {
            panic!("expected one run, got {:?}", painted.calls);
        };
        assert_eq!(at.1, line.y + line.height * 0.5);
        assert_eq!(align.vertical, VerticalAlign::Center);
    }

    #[test]
    fn a_divider_rules_the_full_column() {
        let layout = laid_out(vec![Block::Divider(vec![text("")])], 500.0);
        let painted = paint(&layout, &whole(&layout), 500.0);
        let Some(Call::Rectangle { at, size }) = painted
            .calls
            .iter()
            .find(|call| matches!(call, Call::Rectangle { .. }))
        else {
            panic!("a divider draws its rule, got {:?}", painted.calls);
        };
        assert_eq!(at.0, 0.0);
        assert_eq!(*size, (500.0, RULE_THICKNESS));
    }

    #[test]
    fn a_split_paragraph_draws_its_second_half_at_the_top_of_the_page() {
        let layout = laid_out(
            vec![Block::Paragraph(vec![text("aaaa bbbb cccc dddd")])],
            60.0,
        );
        let lines = layout.blocks[0].lines.len();
        assert!(lines >= 3, "the fixture must wrap at least three ways");

        // Everything from line two on, landing flush at its own page's top:
        // the document y those lines were laid out at is irrelevant here.
        let page = Page {
            pieces: vec![Piece {
                block: 0,
                lines: 1..lines,
                y: 0.0,
            }],
        };
        let painted = paint(&layout, &page, 60.0);
        let Some(Call::Text { at, .. }) = painted.texts().into_iter().next() else {
            panic!("a run is painted, got {:?}", painted.calls);
        };
        assert_eq!(
            at.1,
            layout.blocks[0].lines[1].height * 0.5,
            "the piece's first line sits at the page's top, not the document's"
        );
    }
}
