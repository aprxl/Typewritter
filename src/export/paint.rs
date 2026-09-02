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

use crate::components::editor::{
    BLOCK_PAD, CODE_ROUNDING, EQ_NUMBER_INSET, EQ_NUMBER_SIZE, VIEW_CAP,
};
use crate::components::sidenotes;
use crate::document::layout::{self, DocLayout, NUMBER_GUTTER, NUMBER_SIZE};
use crate::document::{ATOM, Block, Inline, math_layout, math_paint};
use crate::renderer::Rounding;
use crate::theme::{self, TextStyle};

use super::geometry::NOTE_COLUMN;
use super::notes::Placed;
use super::paginate::{Page, Piece};
use crate::canvas::{self, Canvas, Offset};

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

/// The margin column: every note this page carries, beside the line that
/// anchored it. Where each sits is [`super::notes::place`]'s decision; this
/// only draws them.
///
/// The screen's margin also fills a background and rules its left edge.
/// Neither is drawn here: they are the *panel's* — full height whether or
/// not a note is near, and a page has no panels. The tick beside each note
/// is the document's own cue, and that one stays.
pub fn notes(canvas: &mut dyn Canvas, notes: &[Placed]) {
    for note in notes {
        canvas::rule(
            canvas,
            (NOTE_COLUMN, note.y + sidenotes::TICK_TOP),
            sidenotes::TICK_LENGTH,
            sidenotes::TICK_THICKNESS,
            // The margin tints the focused note's tick with the accent. A
            // page has no focus, so every tick is the quiet one.
            theme::border(),
        );
        canvas.draw_text(
            &note.marker,
            (
                NOTE_COLUMN + sidenotes::MARKER_X,
                note.y + sidenotes::MARKER_TOP,
            ),
            &sidenotes::marker_style(),
            theme::LEFT,
        );

        // The body is a document in its own right — its own `DocLayout`, at
        // the margin's width and scale, starting at (0, 0) — so it draws
        // through the same painter the page does, on a canvas whose origin
        // has been moved to the note. Nothing below here knows it is in a
        // margin, which is why a note can hold anything the page can.
        let body = whole(&note.layout);
        // A heading inside a note is not a section of the document, so it
        // gets no auto-number. Notes are one paragraph today; this is what
        // stays right on the day they are not.
        let numbers = vec![None; note.layout.source.len()];
        let mut inner = Offset::new(canvas, (NOTE_COLUMN + sidenotes::NOTE_INSET, note.y));
        page(
            &mut inner,
            &note.layout,
            &body,
            &numbers,
            sidenotes::NOTE_WIDTH,
        );
    }
}

/// A whole layout as one page: every block, all of its lines, where its own
/// layout put them.
///
/// What a sidenote's body is — a small document that never paginates,
/// because the page it belongs to was settled by its anchor. Expressing it
/// as a `Page` is what lets it reuse [`page`] rather than needing a second
/// walk over `DocLayout`.
fn whole(layout: &DocLayout) -> Page {
    Page {
        pieces: layout
            .blocks
            .iter()
            .enumerate()
            .map(|(block, laid)| Piece {
                block,
                lines: 0..laid.lines.len(),
                y: laid.lines.first().map_or(laid.y, |line| line.y),
            })
            .collect(),
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
        // Display math: one slab under the block, and the equation's number
        // hung flush right inside it.
        //
        // Measured from the *piece*, not the block. `paginate` keeps a math
        // block atomic, so the two are the same slab in every ordinary case
        // — but its guard against an unfittable block splits one anyway
        // rather than hang, and a band drawn from the block's own extent
        // would then run off both pages it appears on.
        Block::Math { .. } => {
            let Some(last) = laid.lines.get(piece.lines.end.saturating_sub(1)) else {
                return;
            };
            let top = first.y + dy;
            let bottom = last.y + last.height + dy;
            canvas.draw_rectangle(
                (-BLOCK_PAD.0, top - BLOCK_PAD.1),
                (width + BLOCK_PAD.0 * 2.0, bottom - top + BLOCK_PAD.1 * 2.0),
                theme::math_surface(),
                CODE_ROUNDING,
            );
            // Virtual, like a heading's: only tagged blocks have one, the
            // caret cannot reach it, and it never reflows the math it
            // labels.
            if let Some(number) = layout.equation_numbers.get(&piece.block) {
                canvas.draw_text(
                    number,
                    (width - EQ_NUMBER_INSET, top + first.height * 0.5),
                    &TextStyle::mono(EQ_NUMBER_SIZE, theme::non_text()),
                    theme::RIGHT,
                );
            }
        }
        // Everything below draws its text and, for now, none of its own
        // furniture — the tints, the markers. Each is one arm here and one
        // row of `PDF.md` §6's table; until then a code block prints as
        // correctly-styled code without its slab, which is incomplete
        // rather than wrong.
        Block::Paragraph(_) | Block::CodeLine { .. } | Block::ListItem { .. } => {}
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
            // Notation, drawn through the painter the editor draws it with
            // — see `document::math_paint`. `slots` is false: an empty slot
            // says where the next character lands, and nothing lands on a
            // page. Its geometry is reserved either way, so an expression
            // is the same width here as on screen.
            //
            // An inline expression gets no tint of its own; the notation is
            // already distinct from the words around it. A display block
            // keeps its slab, drawn above.
            Inline::Math(list) => {
                let box_ = math_layout::layout(list, 0, layout.scale, &|text, style| {
                    canvas.measure(text, style)
                });
                math_paint::draw(canvas, &box_, (cursor, baseline), false);
            }
            // A sidenote's anchor: the raised number, matching the marker
            // beside the note itself out in the margin. Raised rather than
            // superscripted — it is the line's own text set small and
            // lifted, which is what `advance` reserved room for.
            Inline::Note(_) => {
                canvas.draw_text(
                    &text,
                    (cursor, baseline - layout::ANCHOR_RISE),
                    &layout::anchor_style(),
                    theme::LEFT,
                );
            }
            // An equation reference: `(n)` when it resolves, and the raw
            // `@eq:label` set small and muted when it does not. Never
            // silently absent — an unresolved reference on a page is a
            // typo the reader can see rather than a hole they cannot.
            Inline::EqRef(_) => {
                let style = layout::eq_ref_style(&text, layout.scale);
                canvas.draw_text(&text, (cursor, baseline), &style, theme::LEFT);
            }
        }
        cursor += width;
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::document::layout::VisLine;
    use crate::document::math::MathNode;
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

        fn draw_path(&mut self, _: &str, _: (f32, f32), _: f32, _: &PathPaint) {}

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

    /// The tagged display block `(1)`, with one symbol in it.
    fn equation() -> Block {
        Block::Math {
            list: vec![Inline::Math(vec![MathNode::Sym('x')])],
            tag: Some("eq:one".into()),
        }
    }

    #[test]
    fn display_math_gets_a_band_wider_than_the_column() {
        let layout = laid_out(vec![equation()], 500.0);
        let painted = paint(&layout, &whole(&layout), 500.0);
        let Some(Call::Rectangle { at, size }) = painted
            .calls
            .iter()
            .find(|call| matches!(call, Call::Rectangle { .. }))
        else {
            panic!("a display block draws its slab, got {:?}", painted.calls);
        };
        // The slab overhangs the column by the block padding on both sides,
        // exactly as the editor's does.
        assert_eq!(at.0, -BLOCK_PAD.0);
        assert_eq!(size.0, 500.0 + BLOCK_PAD.0 * 2.0);
    }

    #[test]
    fn a_tagged_equations_number_hangs_inside_the_band() {
        let layout = laid_out(vec![equation()], 500.0);
        let painted = paint(&layout, &whole(&layout), 500.0);
        let Some(Call::Text { at, align, .. }) = painted.saying("(1)") else {
            panic!("a tagged block is numbered, got {:?}", painted.calls);
        };
        assert_eq!(at.0, 500.0 - EQ_NUMBER_INSET);
        assert_eq!(align.horizontal, HorizontalAlign::Right);
    }

    #[test]
    fn an_untagged_equation_gets_no_number() {
        let layout = laid_out(
            vec![Block::Math {
                list: vec![Inline::Math(vec![MathNode::Sym('x')])],
                tag: None,
            }],
            500.0,
        );
        let painted = paint(&layout, &whole(&layout), 500.0);
        assert!(painted.saying("(1)").is_none());
    }

    #[test]
    fn notation_is_drawn_where_its_run_sits() {
        // An expression inline in prose: the word before it reserves its
        // own width, and the notation starts where that leaves off rather
        // than at the column's edge.
        let layout = laid_out(
            vec![Block::Paragraph(vec![
                text("ab "),
                Inline::Math(vec![MathNode::Sym('x')]),
            ])],
            500.0,
        );
        let painted = paint(&layout, &whole(&layout), 500.0);
        let notation = painted
            .texts()
            .into_iter()
            .find(|call| matches!(call, Call::Text { text, .. } if text == "x"))
            .expect("the expression's glyph is painted");
        let Call::Text { at, .. } = notation else {
            unreachable!()
        };
        assert!(
            at.0 >= 3.0 * GLYPH,
            "notation must start past the run before it, not at {}",
            at.0
        );
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

    #[test]
    fn an_anchor_draws_its_number_raised_above_the_line() {
        let layout = laid_out(
            vec![Block::Paragraph(vec![
                text("see"),
                Inline::Note("aside".into()),
            ])],
            500.0,
        );
        let painted = paint(&layout, &whole(&layout), 500.0);
        let (Some(Call::Text { at: word, .. }), Some(Call::Text { at, size, .. })) =
            (painted.saying("see"), painted.saying("1"))
        else {
            panic!("the anchor draws its number, got {:?}", painted.calls);
        };
        assert_eq!(
            at.1,
            word.1 - layout::ANCHOR_RISE,
            "an anchor is lifted off the line it sits on"
        );
        assert!(*size < 17.5, "an anchor is set smaller than the prose");
        assert_eq!(
            at.0,
            3.0 * GLYPH,
            "the anchor starts where the run before it ends"
        );
    }

    #[test]
    fn a_resolved_equation_reference_draws_its_number_in_the_prose() {
        let layout = laid_out(
            vec![
                equation(),
                Block::Paragraph(vec![text("by "), Inline::EqRef("eq:one".into())]),
            ],
            500.0,
        );
        let painted = paint(&layout, &whole(&layout), 500.0);
        // Twice: once hung in the band's margin, once in the sentence — and
        // the sentence's is the one on the baseline of the prose after it.
        let refs: Vec<&Call> = painted
            .texts()
            .into_iter()
            .filter(|call| matches!(call, Call::Text { text, .. } if text == "(1)"))
            .collect();
        assert_eq!(refs.len(), 2, "band number and inline reference");
        assert!(
            refs.iter().any(
                |call| matches!(call, Call::Text { at, align, .. } if at.0 == 3.0 * GLYPH && *align == theme::LEFT)
            ),
            "the reference sits after the words it follows, got {:?}",
            painted.calls
        );
    }

    #[test]
    fn an_unresolved_equation_reference_keeps_its_raw_spelling() {
        // Never silently absent: a typo a reader can see beats a hole they
        // cannot.
        let layout = laid_out(
            vec![Block::Paragraph(vec![Inline::EqRef("eq:nope".into())])],
            500.0,
        );
        let painted = paint(&layout, &whole(&layout), 500.0);
        assert!(
            painted.saying("@eq:nope").is_some(),
            "got {:?}",
            painted.calls
        );
    }

    /// One note in the margin, with `body` as its text, placed at `y`.
    fn placed(body: &str, y: f32) -> Placed {
        Placed {
            marker: "1".into(),
            layout: layout::layout_blocks(
                &[Block::Paragraph(vec![text(body)])],
                sidenotes::NOTE_WIDTH,
                sidenotes::SCALE,
                &|value, _| value.chars().count() as f32 * GLYPH,
            ),
            y,
        }
    }

    fn paint_notes(notes: &[Placed]) -> Recorder {
        let mut recorder = Recorder::default();
        super::notes(&mut recorder, notes);
        recorder
    }

    #[test]
    fn a_notes_body_is_drawn_out_in_the_margin_column() {
        let painted = paint_notes(&[placed("aside", 100.0)]);
        let Some(Call::Text { at, .. }) = painted.saying("aside") else {
            panic!("the note's body is painted, got {:?}", painted.calls);
        };
        assert_eq!(
            at.0,
            NOTE_COLUMN + sidenotes::NOTE_INSET,
            "the body is inset inside the margin, past the marker"
        );
        assert!(
            at.0 > crate::components::editor::MEASURE,
            "the margin is beside the text column, not inside it"
        );
    }

    #[test]
    fn a_notes_marker_and_tick_sit_at_the_margins_edge() {
        let painted = paint_notes(&[placed("aside", 100.0)]);
        let Some(Call::Text { at, .. }) = painted.saying("1") else {
            panic!("the note's marker is painted, got {:?}", painted.calls);
        };
        assert_eq!(
            at,
            &(
                NOTE_COLUMN + sidenotes::MARKER_X,
                100.0 + sidenotes::MARKER_TOP
            )
        );
        assert!(
            painted.calls.contains(&Call::Rectangle {
                at: (NOTE_COLUMN, 100.0 + sidenotes::TICK_TOP),
                size: (sidenotes::TICK_LENGTH, sidenotes::TICK_THICKNESS),
            }),
            "the tick beside the note is drawn, got {:?}",
            painted.calls
        );
    }

    #[test]
    fn a_note_draws_where_it_was_placed_and_nowhere_else() {
        // The body's own layout starts at zero; what puts it beside its
        // anchor is the offset canvas, so moving the note must move every
        // line of it by exactly the same amount.
        let high = paint_notes(&[placed("aside", 100.0)]);
        let low = paint_notes(&[placed("aside", 340.0)]);
        let (Some(Call::Text { at: a, .. }), Some(Call::Text { at: b, .. })) =
            (high.saying("aside"), low.saying("aside"))
        else {
            panic!("both are painted");
        };
        assert!((b.1 - a.1 - 240.0).abs() < 0.001, "moved by {}", b.1 - a.1);
        assert_eq!(a.0, b.0, "moving a note down must not move it across");
    }

    #[test]
    fn a_page_with_no_notes_draws_no_margin() {
        assert!(paint_notes(&[]).calls.is_empty());
    }
}
