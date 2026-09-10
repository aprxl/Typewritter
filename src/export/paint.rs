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
use crate::document::decoration::{self, Painted};
use crate::document::layout::{self, DocLayout, NUMBER_GUTTER, NUMBER_SIZE, TABLE_CELL_PAD};
use crate::document::{ATOM, Block, Inline, code, math_layout, math_paint};
use crate::layout::Rect;
use crate::renderer::{Color, Rounding};
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
    fences(canvas, layout, page, width);
    for piece in &page.pieces {
        block(canvas, layout, piece, numbers, width);
    }
}

/// The page's own ground, edge to edge, under everything else.
///
/// The editor fills its pane with [`theme::background`] and every colour
/// above is chosen against it, so a sheet left to whatever the reader's
/// viewer paints is not the editor's page — it is the editor's ink on
/// somebody else's paper. On the light palette that is a two percent
/// difference nobody would notice; on the dark one it is near-white text
/// on white, which is to say nothing at all. One rule covers both, so
/// there is no branch here asking how dark a colour has to be before it
/// counts.
pub fn ground(canvas: &mut dyn Canvas, sheet: Rect) {
    canvas.draw_rectangle(
        sheet.position(),
        sheet.size(),
        theme::background(),
        Rounding::NONE,
    );
}

/// The tint under every fenced block on this page, one slab per fence.
///
/// The one decoration a [`Piece`] cannot draw for itself: a fence is a
/// *run* of `CodeLine` blocks, its lines sit flush against each other, and
/// a slab per line would notch the rounded corners at every join. So the
/// run is found here — among the pieces this page actually carries, not
/// the blocks the document has, so a fence that `paginate` had to split
/// gets a slab on each side of the break rather than one drawn off the
/// bottom of the first sheet.
fn fences(canvas: &mut dyn Canvas, layout: &DocLayout, page: &Page, width: f32) {
    let mut index = 0;
    while index < page.pieces.len() {
        if !layout.source[page.pieces[index].block].is_code() {
            index += 1;
            continue;
        }
        // Every continuation line of this fence that is also on this page.
        // A `first: true` block opens a new fence and ends this one, and so
        // does a gap in the block numbering.
        let mut end = index;
        while end + 1 < page.pieces.len()
            && page.pieces[end + 1].block == page.pieces[end].block + 1
            && matches!(
                layout.source[page.pieces[end + 1].block],
                Block::CodeLine { first: false, .. }
            )
        {
            end += 1;
        }
        if let (Some((top, _)), Some((_, bottom))) = (
            extent(layout, &page.pieces[index]),
            extent(layout, &page.pieces[end]),
        ) {
            slab(canvas, top, bottom, width, theme::code());
        }
        index = end + 1;
    }
}

/// Where a piece's lines start and end, in page coordinates.
fn extent(layout: &DocLayout, piece: &Piece) -> Option<(f32, f32)> {
    let laid = &layout.blocks[piece.block];
    let first = laid.lines.get(piece.lines.start)?;
    let last = laid.lines.get(piece.lines.end.checked_sub(1)?)?;
    // Document y → page y, the same shift every line of this piece takes.
    let dy = piece.y - first.y;
    Some((first.y + dy, last.y + last.height + dy))
}

/// The ground under a block that owns its whole band — a fence's tint, a
/// display equation's. It overhangs the column by [`BLOCK_PAD`] on every
/// side, which is what makes it read as ground under the text rather than
/// as a box drawn around it.
fn slab(canvas: &mut dyn Canvas, top: f32, bottom: f32, width: f32, color: Color) {
    canvas.draw_rectangle(
        (-BLOCK_PAD.0, top - BLOCK_PAD.1),
        (width + BLOCK_PAD.0 * 2.0, bottom - top + BLOCK_PAD.1 * 2.0),
        color,
        CODE_ROUNDING,
    );
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
        // block whole, so the two are the same slab in every ordinary case
        // — but a block taller than a sheet packs line by line anyway
        // rather than run off it, and a band drawn from the block's own
        // extent would then overhang both pages it appears on.
        Block::Math { .. } => {
            if let Some((top, bottom)) = extent(layout, piece) {
                slab(canvas, top, bottom, width, theme::math_surface());
                // Virtual, like a heading's: only tagged blocks have one,
                // the caret cannot reach it, and it never reflows the math
                // it labels.
                if let Some(number) = layout.equation_numbers.get(&piece.block) {
                    canvas.draw_text(
                        number,
                        (width - EQ_NUMBER_INSET, top + first.height * 0.5),
                        &TextStyle::mono(EQ_NUMBER_SIZE, theme::non_text()),
                        theme::RIGHT,
                    );
                }
            }
        }
        // The marker in a list item's gutter: a bullet, a number, or a task
        // box. Virtual like a heading's auto-number — drawn, never laid
        // out — and hung beside the item's *first* line only, so a piece
        // that opens mid-item is a continuation and carries none. Its
        // hanging indent is already in `line.x`, which is why the marker
        // hangs off that rather than off the column.
        Block::ListItem { marker, .. } => {
            if piece.lines.start == 0 {
                decoration::marker(
                    canvas,
                    marker,
                    first.x,
                    first.y + dy + first.height * 0.5,
                    layout.scale,
                );
            }
        }
        Block::TableRow { .. } => table_row(canvas, layout, piece, first, dy),
        // A fence's tint is drawn by `fences`, which sees the whole run;
        // a paragraph has no furniture of its own at all.
        Block::Paragraph(_) | Block::CodeLine { .. } => {}
    }

    for index in piece.lines.clone() {
        let Some(line) = laid.lines.get(index) else {
            continue;
        };
        self::line(canvas, layout, piece.block, line, dy);
    }
}

/// The PDF counterpart of `Editor::draw_table`: one row at a time because
/// pagination may put neighbouring rows on different sheets. Track geometry
/// still comes from the same layout snapshot as the editor.
fn table_row(
    canvas: &mut dyn Canvas,
    layout: &DocLayout,
    piece: &Piece,
    line: &crate::document::layout::VisLine,
    dy: f32,
) {
    let Some(table) = layout.tables.get(piece.block).and_then(Option::as_ref) else {
        return;
    };
    let top = line.y + dy;
    let height = line.height;
    let width: f32 = table.columns.iter().sum();
    canvas.draw_rectangle(
        (0.0, top),
        (width, height),
        theme::fade(theme::alt(), 0.78),
        Rounding::NONE,
    );
    let mut left = 0.0;
    let cell_source = &layout.source[piece.block];
    for (column, cell) in cell_source.cells().iter().enumerate() {
        let inset = TABLE_CELL_PAD * layout.scale;
        let lines = &table.cells[column];
        let content_height: f32 = lines.iter().map(|line| line.height).sum();
        let content_top = top + (height - content_height).max(0.0) * 0.5;
        for line in lines {
            let logical = &cell.lines()[line.cell_line.min(cell.lines().len() - 1)];
            let cell_block = Block::Paragraph(logical.to_vec());
            let line_top = content_top + line.y;
            let baseline = line_top + line.height * 0.5;
            let mut cursor = left + inset + line.x;
            let mut pieces = Vec::with_capacity(line.segments.len());
            for segment in &line.segments {
                let run = &logical[segment.inline];
                let text: String = match run {
                    Inline::Text(text) => text
                        .text
                        .chars()
                        .skip(segment.start)
                        .take(segment.len.min(VIEW_CAP))
                        .collect(),
                    Inline::Math(_) => ATOM.to_string(),
                    Inline::Note(_) | Inline::EqRef(_) => {
                        segment.number.clone().unwrap_or_default()
                    }
                };
                let advance =
                    segment.advance(run, &text, &cell_block, layout.scale, &|text, style| {
                        canvas.measure(text, style)
                    });
                match run {
                    Inline::Math(list) => {
                        let expression =
                            math_layout::layout(list, 0, layout.scale, &|text, style| {
                                canvas.measure(text, style)
                            });
                        math_paint::draw(canvas, &expression, (cursor, baseline), false);
                    }
                    Inline::Note(_) => canvas.draw_text(
                        &text,
                        (cursor, baseline - layout::ANCHOR_RISE),
                        &layout::anchor_style(),
                        theme::LEFT,
                    ),
                    Inline::EqRef(_) => canvas.draw_text(
                        &text,
                        (cursor, baseline),
                        &layout::eq_ref_style(&text, layout.scale),
                        theme::LEFT,
                    ),
                    Inline::Text(_) => {}
                }
                pieces.push(decoration::piece(
                    text,
                    segment.style,
                    cursor,
                    advance,
                    layout.scale,
                    segment.padding,
                ));
                cursor += advance;
            }
            decoration::runs(
                canvas,
                &pieces,
                &cell_block,
                line_top,
                line.height,
                baseline,
                layout.scale,
            );
            for ((text, _, at, _), segment) in pieces.iter().zip(&line.segments) {
                if matches!(logical[segment.inline], Inline::Text(_)) {
                    canvas.draw_text(
                        text,
                        (*at, baseline),
                        &layout::table_text_style(segment.style, layout.scale),
                        theme::LEFT,
                    );
                }
            }
        }
        left += table.columns[column];
    }
    let line_color = theme::border();
    if table.lines.left {
        canvas.draw_rectangle(
            (0.0, top),
            (1.0, height),
            line_color.clone(),
            Rounding::NONE,
        );
    }
    if table.lines.right {
        canvas.draw_rectangle(
            (width, top),
            (1.0, height),
            line_color.clone(),
            Rounding::NONE,
        );
    }
    if table.lines.vertical {
        let mut edge = 0.0;
        for track in table
            .columns
            .iter()
            .take(table.columns.len().saturating_sub(1))
        {
            edge += track;
            canvas.draw_rectangle(
                (edge, top),
                (1.0, height),
                line_color.clone(),
                Rounding::NONE,
            );
        }
    }
    if table.row == 0 && table.lines.top {
        canvas.draw_rectangle((0.0, top), (width, 1.0), line_color.clone(), Rounding::NONE);
    }
    let rows = (table.first..layout.source.len())
        .take_while(|&row| {
            row == table.first
                || matches!(
                    layout.source.get(row),
                    Some(Block::TableRow { first: false, .. })
                )
        })
        .count();
    if table.row + 1 == rows {
        if table.lines.bottom {
            canvas.draw_rectangle(
                (0.0, top + height),
                (width, 1.0),
                line_color,
                Rounding::NONE,
            );
        }
    } else if table.lines.horizontal {
        canvas.draw_rectangle(
            (0.0, top + height),
            (width, 1.0),
            line_color,
            Rounding::NONE,
        );
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
    let top = line.y + dy;
    // Not a typographic baseline: the editor draws text vertically centred
    // on the line, and this is that centre. `Canvas::draw_text` recovers the
    // real baseline from the shaped run.
    let baseline = top + line.height * 0.5;
    let mut cursor = line.x;
    // Measured first, drawn second, exactly as the editor does it: the
    // marks these runs carry are read off their extents, so they cannot be
    // drawn until every run on the line has been measured — and they go
    // *under* the text, so the text cannot be drawn until they are down.
    let mut pieces: Vec<Painted> = Vec::new();

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
        let width = segment.advance(run, &text, kind, layout.scale, &|text, style| {
            canvas.measure(text, style)
        });

        match run {
            // Text waits for the pass below: it is drawn over every mark
            // this line carries, and none of those is placed yet.
            Inline::Text(_) => {}
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
        pieces.push(decoration::piece(
            text,
            segment.style,
            cursor,
            width,
            layout.scale,
            segment.padding,
        ));
        cursor += width;
    }

    // The box behind a code span, a badge's chip, a highlight's bar, a done
    // task's strike — the editor's own painter, so a page cannot disagree
    // with the screen about where any of them lands.
    decoration::runs(
        canvas,
        &pieces,
        kind,
        top,
        line.height,
        baseline,
        layout.scale,
    );

    // The prose, last and on top of every mark. Drawn from the *piece's* x
    // rather than the pen's: a badge's label sits one pad inside the chip
    // drawn around it, and the piece is what knows that.
    for ((text, _, at, _), segment) in pieces.iter().zip(&line.segments) {
        if !matches!(kind.inlines()[segment.inline], Inline::Text(_)) {
            continue;
        }
        let mut prefix = String::new();
        for (part, style) in code::painted(layout, index, segment, text) {
            let x = at + canvas.measure(&prefix, &style);
            canvas.draw_text(part, (x, baseline), &style, theme::LEFT);
            prefix.push_str(part);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::document::layout::VisLine;
    use crate::document::math::MathNode;
    use crate::document::{Document, ListMarker, Style, Text};
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
            color: Color,
        },
        Circle {
            center: (f32, f32),
            radius: f32,
        },
        Path {
            at: (f32, f32),
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

        /// Every rectangle drawn in `color` — which is what tells a fence's
        /// tint from an equation's band and a chip from a highlight's bar.
        fn rectangles(&self, color: Color) -> Vec<((f32, f32), (f32, f32))> {
            self.calls
                .iter()
                .filter_map(|call| match call {
                    Call::Rectangle { at, size, color: c } if *c == color => Some((*at, *size)),
                    _ => None,
                })
                .collect()
        }

        fn first_rectangle(&self) -> ((f32, f32), (f32, f32)) {
            self.calls
                .iter()
                .find_map(|call| match call {
                    Call::Rectangle { at, size, .. } => Some((*at, *size)),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("a rectangle is drawn, got {:?}", self.calls))
        }
    }

    impl Canvas for Recorder {
        fn draw_rectangle(&mut self, at: (f32, f32), size: (f32, f32), color: Color, _: Rounding) {
            self.calls.push(Call::Rectangle { at, size, color });
        }

        fn draw_circle(&mut self, center: (f32, f32), radius: f32, _: Color) {
            self.calls.push(Call::Circle { center, radius });
        }

        fn draw_path(&mut self, _: &str, at: (f32, f32), _: f32, _: &PathPaint) {
            self.calls.push(Call::Path { at });
        }

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
    fn the_ground_covers_the_whole_sheet_in_the_pages_own_colour() {
        let geometry =
            super::super::geometry::PageGeometry::plain(super::super::geometry::Paper::A4);
        let sheet = geometry.sheet();
        let mut recorder = Recorder::default();
        super::ground(&mut recorder, sheet);
        assert_eq!(
            recorder.rectangles(theme::background()),
            vec![(sheet.position(), sheet.size())],
            "one rectangle, the sheet's, in the editor's page colour"
        );
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
        let (at, size) = painted.first_rectangle();
        assert_eq!(at.0, 0.0);
        assert_eq!(size, (500.0, RULE_THICKNESS));
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
        let (at, size) = painted.first_rectangle();
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

    /// One line of a fenced block. `first` opens the fence; the rest
    /// continue it, which is what makes a fence a run rather than a block.
    fn code_line(value: &str, first: bool) -> Block {
        Block::CodeLine {
            content: vec![text(value)],
            first,
            lang: None,
        }
    }

    #[test]
    fn a_fence_is_tinted_as_one_slab_over_all_its_lines() {
        // Three blocks, one tint. Per-line boxes would notch the rounded
        // corner at every join, which is why the run is found first.
        let layout = laid_out(
            vec![
                code_line("one", true),
                code_line("two", false),
                code_line("three", false),
            ],
            500.0,
        );
        let painted = paint(&layout, &whole(&layout), 500.0);
        let tints = painted.rectangles(theme::code());
        let [(at, size)] = tints[..] else {
            panic!("one slab under the whole fence, got {tints:?}");
        };
        let first = &layout.blocks[0].lines[0];
        let last = &layout.blocks[2].lines[0];
        assert_eq!(at, (-BLOCK_PAD.0, first.y - BLOCK_PAD.1));
        assert_eq!(size.0, 500.0 + BLOCK_PAD.0 * 2.0);
        assert_eq!(
            size.1,
            last.y + last.height - first.y + BLOCK_PAD.1 * 2.0,
            "the slab reaches the bottom of the last line of the fence"
        );
    }

    #[test]
    fn two_fences_are_two_slabs() {
        // A `first: true` line opens a new fence and closes the one above
        // it, however flush the two sit.
        let layout = laid_out(
            vec![
                code_line("one", true),
                code_line("two", true),
                code_line("three", false),
            ],
            500.0,
        );
        let painted = paint(&layout, &whole(&layout), 500.0);
        assert_eq!(painted.rectangles(theme::code()).len(), 2);
    }

    #[test]
    fn a_fence_split_across_a_break_is_tinted_on_both_pages() {
        // The slab is drawn from the pieces this page carries, not from the
        // blocks the document has, or the first sheet would carry a slab
        // long enough for lines that are not on it.
        let layout = laid_out(
            vec![
                code_line("one", true),
                code_line("two", false),
                code_line("three", false),
            ],
            500.0,
        );
        let split = Page {
            pieces: vec![Piece {
                block: 0,
                lines: 0..1,
                y: 0.0,
            }],
        };
        let painted = paint(&layout, &split, 500.0);
        let tints = painted.rectangles(theme::code());
        let [(_, size)] = tints[..] else {
            panic!("the half on this page is tinted, got {tints:?}");
        };
        let line = &layout.blocks[0].lines[0];
        assert_eq!(size.1, line.height + BLOCK_PAD.1 * 2.0, "one line's worth");
    }

    #[test]
    fn a_code_span_in_prose_gets_a_box_of_its_own() {
        let code_run = Inline::Text(Text {
            text: "run".into(),
            style: Style {
                code: true,
                ..Style::PLAIN
            },
        });
        let layout = laid_out(
            vec![Block::Paragraph(vec![text("a"), code_run, text("b")])],
            500.0,
        );
        let painted = paint(&layout, &whole(&layout), 500.0);
        let boxes = painted.rectangles(theme::code());
        let [(at, size)] = boxes[..] else {
            panic!("a code span is boxed, got {:?}", painted.calls);
        };
        assert!(
            at.0 > GLYPH,
            "the background must leave space after the preceding prose"
        );
        let Some(Call::Text { at: after, .. }) = painted.saying("b") else {
            panic!("following prose must be drawn")
        };
        assert!(
            at.0 + size.0 < after.0,
            "the background must stop before following prose"
        );
    }

    #[test]
    fn a_run_inside_a_fence_gets_no_box_of_its_own() {
        // It is already on the fence's slab; a second box per line would
        // draw a ladder down the block.
        let layout = laid_out(vec![code_line("one", true)], 500.0);
        let painted = paint(&layout, &whole(&layout), 500.0);
        assert_eq!(
            painted.rectangles(theme::code()).len(),
            1,
            "the slab and nothing else, got {:?}",
            painted.calls
        );
    }

    fn item(marker: ListMarker, value: &str) -> Block {
        Block::ListItem {
            marker,
            content: vec![text(value)],
        }
    }

    #[test]
    fn a_bullet_is_drawn_in_the_gutter_beside_its_item() {
        let layout = laid_out(vec![item(ListMarker::Bullet, "milk")], 500.0);
        let painted = paint(&layout, &whole(&layout), 500.0);
        let line = &layout.blocks[0].lines[0];
        let Some(Call::Circle { center, .. }) = painted
            .calls
            .iter()
            .find(|call| matches!(call, Call::Circle { .. }))
        else {
            panic!("a bullet is drawn, got {:?}", painted.calls);
        };
        assert!(
            center.0 < line.x,
            "the dot hangs left of the item's text at {}, not {}",
            line.x,
            center.0
        );
        assert!((center.1 - (line.y + line.height * 0.5)).abs() <= 1.0);
    }

    #[test]
    fn a_numbered_item_draws_its_ordinal_right_aligned() {
        let layout = laid_out(vec![item(ListMarker::Number(7), "seventh")], 500.0);
        let painted = paint(&layout, &whole(&layout), 500.0);
        let Some(Call::Text { at, align, .. }) = painted.saying("7") else {
            panic!("the ordinal is drawn, got {:?}", painted.calls);
        };
        assert_eq!(at.0, layout.blocks[0].lines[0].x - NUMBER_GUTTER);
        assert_eq!(align.horizontal, HorizontalAlign::Right);
    }

    #[test]
    fn an_open_task_draws_an_empty_box_and_a_done_one_a_tick() {
        let open = laid_out(vec![item(ListMarker::Task { done: false }, "todo")], 500.0);
        let open = paint(&open, &whole(&open), 500.0);
        let done = laid_out(vec![item(ListMarker::Task { done: true }, "todo")], 500.0);
        let done = paint(&done, &whole(&done), 500.0);

        let paths = |painted: &Recorder| {
            painted
                .calls
                .iter()
                .filter(|call| matches!(call, Call::Path { .. }))
                .count()
        };
        assert_eq!(paths(&open), 1, "just the outline, got {:?}", open.calls);
        assert_eq!(
            paths(&done),
            2,
            "the outline and the tick, got {:?}",
            done.calls
        );
    }

    #[test]
    fn a_done_task_is_struck_through_its_own_text() {
        let layout = laid_out(vec![item(ListMarker::Task { done: true }, "done")], 500.0);
        let painted = paint(&layout, &whole(&layout), 500.0);
        let strikes = painted.rectangles(theme::dim());
        let [(at, size)] = strikes[..] else {
            panic!("a done task is struck, got {:?}", painted.calls);
        };
        let line = &layout.blocks[0].lines[0];
        assert!((at.1 - (line.y + line.height * 0.5)).abs() < 1.0);
        assert!(
            size.0 >= 4.0 * GLYPH,
            "the rule crosses the whole word, not {}",
            size.0
        );
    }

    #[test]
    fn a_marker_is_drawn_once_however_far_its_item_wraps() {
        let layout = laid_out(vec![item(ListMarker::Number(1), "aaaa bbbb cccc")], 90.0);
        assert!(
            layout.blocks[0].lines.len() > 1,
            "the fixture must actually wrap"
        );
        let painted = paint(&layout, &whole(&layout), 90.0);
        let markers = painted
            .texts()
            .into_iter()
            .filter(|call| matches!(call, Call::Text { text, .. } if text == "1"))
            .count();
        assert_eq!(markers, 1);
    }

    #[test]
    fn a_continuation_piece_carries_no_marker() {
        // The second half of an item split across a break keeps the indent
        // and leaves the bullet on the page its first line went to.
        let layout = laid_out(vec![item(ListMarker::Bullet, "aaaa bbbb cccc")], 90.0);
        let lines = layout.blocks[0].lines.len();
        assert!(lines > 1, "the fixture must actually wrap");
        let rest = Page {
            pieces: vec![Piece {
                block: 0,
                lines: 1..lines,
                y: 0.0,
            }],
        };
        let painted = paint(&layout, &rest, 90.0);
        assert!(
            !painted
                .calls
                .iter()
                .any(|call| matches!(call, Call::Circle { .. })),
            "got {:?}",
            painted.calls
        );
    }

    fn styled(value: &str, style: Style) -> Inline {
        Inline::Text(Text {
            text: value.into(),
            style,
        })
    }

    #[test]
    fn a_badge_draws_its_chip_around_an_inset_label() {
        let badge = Style {
            badge: true,
            ..Style::PLAIN
        };
        let layout = laid_out(vec![Block::Paragraph(vec![styled("TODO", badge)])], 500.0);
        let painted = paint(&layout, &whole(&layout), 500.0);
        let Some(Call::Text { at: label, .. }) = painted.saying("TODO") else {
            panic!("the chip's label is painted, got {:?}", painted.calls);
        };
        assert_eq!(
            label.0,
            theme::BADGE_PAD,
            "the label sits one pad inside the chip"
        );
        let (chip, size) = painted.first_rectangle();
        assert_eq!(chip.0, 0.0, "the chip starts where the run does");
        assert_eq!(size.1, theme::BADGE_HEIGHT);
        assert_eq!(
            size.0,
            4.0 * GLYPH + theme::BADGE_PAD * 2.0,
            "the chip is the label plus a pad each side"
        );
    }

    #[test]
    fn a_highlight_draws_its_bar_under_the_run_it_marks() {
        let mark = Style {
            highlight: true,
            ..Style::PLAIN
        };
        let layout = laid_out(
            vec![Block::Paragraph(vec![text("a "), styled("marked", mark)])],
            500.0,
        );
        let painted = paint(&layout, &whole(&layout), 500.0);
        let bars = painted.rectangles(theme::highlight());
        let [(at, size)] = bars[..] else {
            panic!("a marked run is barred, got {:?}", painted.calls);
        };
        let Some(Call::Text { at: word, .. }) = painted.saying("marked") else {
            panic!("the marked word is painted");
        };
        assert!(at.1 > word.1, "the bar hangs below the line it marks");
        assert!(
            size.0 >= 6.0 * GLYPH,
            "the bar spans the marked run, not {}",
            size.0
        );
    }

    #[test]
    fn text_is_drawn_over_the_marks_beneath_it() {
        // Order is the whole promise here: a bar drawn after its glyphs
        // would strike them out.
        let mark = Style {
            highlight: true,
            ..Style::PLAIN
        };
        let layout = laid_out(vec![Block::Paragraph(vec![styled("marked", mark)])], 500.0);
        let painted = paint(&layout, &whole(&layout), 500.0);
        let bar = painted
            .calls
            .iter()
            .position(|call| matches!(call, Call::Rectangle { .. }))
            .expect("the bar is drawn");
        let word = painted
            .calls
            .iter()
            .position(|call| matches!(call, Call::Text { text, .. } if text == "marked"))
            .expect("the word is drawn");
        assert!(bar < word, "the mark goes under the glyphs, not over them");
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
            painted.rectangles(theme::border()).contains(&(
                (NOTE_COLUMN, 100.0 + sidenotes::TICK_TOP),
                (sidenotes::TICK_LENGTH, sidenotes::TICK_THICKNESS)
            )),
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
