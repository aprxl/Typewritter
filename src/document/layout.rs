//! Document layout: greedy word wrap and the caret↔visual mapping.
//!
//! This is the bridge between the model and pixels. Given a [`Document`], a
//! column width, and a way to measure text, [`layout`] produces the visual
//! lines the editor draws plus the inverse mappings the caret, clicks, and
//! `j`/`k` need. Everything is pure and measured through a closure, so every
//! rule is testable headless.
//!
//! Layout is O(document) per call — the milestone ceiling; visible-range
//! layout is a later optimization. Nothing here is reused from `prose`:
//! prose collapses whitespace and carries no source offsets, both fatal to
//! caret mapping.

use std::collections::HashMap;
use std::sync::Arc;

use super::widget_layout::{WidgetRowLayout, widget_layout};
use crate::document::math::{MathCursor, NodeAddress};
use crate::document::{
    Block, Caret, Document, FlatPos, FlatRange, Inline, ListMarker, Style, fold_region_end,
    math_layout, outline, table,
};
use crate::layout::Rect;
use crate::theme::{self, TextStyle};

/// Visual line heights, in logical pixels.
pub const LINE_BODY: f32 = 30.0;
/// Horizontal inset of inline code's background, plus clear space outside it.
pub const INLINE_CODE_INSET: f32 = 6.0;
pub const INLINE_CODE_GAP: f32 = 3.0;
const INLINE_CODE_SPACE: f32 = INLINE_CODE_INSET + INLINE_CODE_GAP;
pub const LINE_H1: f32 = 50.0;
pub const LINE_H2: f32 = 38.0;
pub const LINE_H3: f32 = 32.0;
pub const LINE_H4: f32 = 30.0;
/// The band a rule occupies. Deliberately shorter than a body line: a rule
/// separates, and a big hole around it disrupts reading more than the
/// separation it buys is worth.
pub const LINE_DIVIDER: f32 = 20.0;
/// Vertical breathing room above and below a display expression, each
/// side. A display block is its own paragraph; it should not sit tighter
/// than one.
pub const MATH_PAD: f32 = 10.0;
/// Extra room above and below an inline expression inside a text line,
/// so a tall fraction does not touch the lines around it.
pub const MATH_LEADING: f32 = 4.0;
/// Space below a paragraph.
pub const GAP_PARAGRAPH: f32 = 14.0;
/// A widget wall is furniture, not a line of prose. Give the text flowing
/// beside it a little air before its first baseline, while keeping every
/// subsequent line on the normal rhythm.
pub const WIDGET_FLOW_TOP_GAP: f32 = 10.0;
/// Clear space above *and* below a display expression — the **total**, not
/// an addition to whatever the neighbouring block already contributes, so an
/// equation is inset by the same amount top and bottom whatever it sits
/// between. Wider than a paragraph's gap because a displayed equation is set
/// apart from the prose around it rather than wrapped into it. This is on
/// top of [`MATH_PAD`], which is the breathing room *inside* the block.
pub const GAP_MATH: f32 = 24.0;
/// Space *above* a heading (the first block gets none).
pub const GAP_HEADING: f32 = 30.0;
/// Space below a heading.
pub const GAP_AFTER_HEADING: f32 = 12.0;
/// Space below a rule — tighter than a paragraph's, for the same reason.
pub const GAP_DIVIDER: f32 = 8.0;
/// Insets shared by text, the cell caret, and table hit testing.
pub const TABLE_CELL_PAD: f32 = 12.0;
/// Vertical breathing room inside a table cell. Horizontal padding is larger
/// because columns need an obvious editable gutter; vertical padding simply
/// keeps wrapped lines from grazing a grid stroke.
pub const TABLE_CELL_VERTICAL_PAD: f32 = 6.0;
/// The baseline-to-baseline rhythm for wrapped table text. It is tighter than
/// prose's block band while still leaving an easy-to-hit line in a layout
/// table.
pub const TABLE_TEXT_LINE_HEIGHT: f32 = 24.0;
/// Space below a list item that has another item right after it — tighter
/// than a paragraph's, because the items are one thought. The last item of
/// a run keeps the full paragraph gap below it.
pub const LIST_GAP: f32 = 6.0;
/// A list item's content column, hung once for the whole block: the marker
/// lives in the gutter to its left, and every wrapped line aligns here —
/// under the content, never under the marker (the hanging indent).
pub const LIST_INDENT: f32 = 26.0;
/// A task checkbox's edge, and the gap between its right side and the
/// item's content column. One geometry for the drawing and the click
/// hit-test, both reading [`DocLayout::task_box`].
pub const CHECK_SIZE: f32 = 15.0;
pub const CHECK_GAP: f32 = 9.0;
/// The collapsed-body indicator's band height: one quiet line, sized to the
/// small type it carries rather than to the prose around it.
pub const FOLD_INDICATOR_HEIGHT: f32 = 22.0;
/// The gutter the heading auto-number and the fold chevron share, measured
/// back from the text column's left edge in content coordinates. The number
/// is drawn right-aligned against it; the chevron hangs to its left. Lives
/// here (not in the editor) because the chevron's hit-test and its drawing
/// must read one number.
pub const NUMBER_GUTTER: f32 = 12.0;
/// The auto-number's size. Constant rather than scaled per heading level:
/// it is a margin annotation, not part of the heading's own typography.
pub const NUMBER_SIZE: f32 = 11.0;
/// The chevron triangle: this wide and this tall, pointing down when the
/// section is open and right when it is folded.
pub const CHEVRON_WIDTH: f32 = 7.0;
pub const CHEVRON_HEIGHT: f32 = 6.0;
/// Gap between the chevron's right edge and the auto-number's left edge, so
/// the two gutter marks read as one aligned column, not one collision.
pub const CHEVRON_GAP: f32 = 9.0;
/// Slack around the chevron a click is granted: a 5px triangle is a cruel
/// click target on its own.
pub const CHEVRON_HIT_PAD: f32 = 8.0;
/// Floor for the chevron's right edge: a deeply numbered heading must not
/// push its chevron out of the page gutter where a click cannot reach it.
pub const CHEVRON_RIGHT_FLOOR: f32 = -44.0;

/// Content x of a fold chevron's right edge, given the width of the
/// auto-number it must clear — the one formula the editor's drawing and the
/// layout's hit-test share, so they cannot drift.
pub fn chevron_right(number_width: f32, scale: f32) -> f32 {
    (-(NUMBER_GUTTER + number_width + CHEVRON_GAP) * scale).max(CHEVRON_RIGHT_FLOOR * scale)
}
/// The size of an anchor's raised number. Matches the heading's auto-number:
/// both are margin annotations, not part of the prose they annotate.
pub const ANCHOR_SIZE: f32 = 11.0;
/// How far an anchor's number sits above the line baseline — high enough to
/// read as a footnote marker, low enough not to collide with the line above.
pub const ANCHOR_RISE: f32 = 6.0;

/// A run of a visual line that came from one source run, covering exactly
/// `[start, start + len)` chars of it. Ranges on a line are contiguous and
/// exact — no gaps, no overlaps — which is what makes caret math trivial.
#[derive(Clone, Debug)]
pub struct Segment {
    /// Space before/after the label; only the outside edges of inline code reserve it.
    pub padding: (f32, f32),
    /// Index of the source run in the block's `inlines`.
    pub inline: usize,
    /// Char offset within that run's text.
    pub start: usize,
    /// Char count.
    pub len: usize,
    /// The run's style — what to draw with.
    pub style: Style,
    /// The raised number a sidenote anchor draws, and `None` for every other
    /// run. Carried here so the caret, the hit tests, and the editor's
    /// drawing all read the same number instead of deriving it each.
    pub number: Option<String>,
}

impl Segment {
    pub fn advance(
        &self,
        run: &Inline,
        text: &str,
        block: &Block,
        scale: f32,
        measure: &dyn Fn(&str, &TextStyle) -> f32,
    ) -> f32 {
        advance(
            run,
            text,
            block,
            self.style,
            self.number.as_deref(),
            scale,
            measure,
        ) + (self.padding.0 + self.padding.1) * scale
    }
    pub fn text_inset(&self, scale: f32) -> f32 {
        (self.padding.0
            + if self.style.badge {
                theme::BADGE_PAD
            } else {
                0.0
            })
            * scale
    }
}

/// One visual line: a contiguous slice of the source block's flat text.
#[derive(Clone, Debug)]
pub struct VisLine {
    /// Top of the line, relative to content top.
    pub y: f32,
    /// Left edge of the line's text, relative to the content column —
    /// nonzero only for a list item's content, which is indented once for
    /// every line of the block. The caret, the click hit-tests, and the
    /// drawing all add it, so they cannot drift apart.
    pub x: f32,
    /// Width available to this line. It is the full column for ordinary
    /// blocks, or the free lane beside a widget row until that lane ends.
    pub width: f32,
    /// Actual line height: the block's floor or its tallest content.
    pub height: f32,
    pub segments: Vec<Segment>,
    /// Which logical line of its table cell this line belongs to, so a
    /// wrapped visual line can recover the cell's own runs and line
    /// arithmetic (0 outside a table).
    pub cell_line: usize,
}

/// The collapsed-body indicator under a folded heading: where it sits and
/// how many lines it hides. The count lands after the body blocks are laid
/// out, so it is patched in once the main pass is done.
pub struct FoldIndicator {
    /// Top of the indicator band, relative to content top.
    pub y: f32,
    pub lines: usize,
}

/// One block's laid-out lines plus where it sits.
pub struct BlockLayout {
    /// Top of the block, relative to content top.
    pub y: f32,
    /// Empty exactly when the block is folded away: a hidden block occupies
    /// no height and draws nothing, but its lines were counted for the
    /// folded heading's indicator before being dropped.
    pub lines: Vec<VisLine>,
    pub height: f32,
    /// The folded heading that hides this block, if it is hidden.
    pub hidden: Option<usize>,
    /// Set on a folded heading whose body is non-empty.
    pub indicator: Option<FoldIndicator>,
}

/// The measured geometry of one table row. One entry per row in the layout
/// snapshot, so painting, hit-testing, drag handles, and the caret all read
/// one geometry without reaching back into the live document. Everything
/// that is the same for every row — the column tracks, the row count, the
/// grid lines — is measured once per table and shared, so a table costs
/// per-cell work rather than per-row work.
#[derive(Clone, Debug)]
pub struct TableLayout {
    pub first: usize,
    pub row: usize,
    /// The table's measured column tracks, shared with every other row.
    pub columns: Arc<[f32]>,
    /// How many rows the table has, so no consumer has to re-count them.
    pub rows: usize,
    pub row_height: f32,
    /// Wrapped inline layout for each cell, with y coordinates local to the
    /// cell's content box and segment indices addressing its line's own runs.
    pub cells: Vec<Vec<VisLine>>,
    pub lines: super::table::TableLines,
}

/// A draggable internal table divider, named by the tracks it separates.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TableResize {
    Column { first: usize, divider: usize },
    Row { first: usize, divider: usize },
}

/// The stable coordinate of one table cell. It is named from the table's
/// first row, not from a transient caret, so a Normal-mode table card can
/// safely add or remove tracks after the click that opened it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableCell {
    pub first: usize,
    pub row: usize,
    pub column: usize,
}

/// One sidenote anchor: where it is, and the number the reader sees.
pub struct Anchor {
    /// The block and inline run the anchor occupies.
    pub block: usize,
    pub inline: usize,
    /// The author's label on disk — not shown, only used to find the note.
    pub label: String,
    /// The raised number shown, "1".."n" in document order.
    pub number: String,
    /// Which of the block's visual lines the anchor sits on, and `None` when
    /// the block is folded away and has no lines to sit on. The export reads
    /// this to decide which page a note travels to, because a page break
    /// falls between lines and the y alone cannot say which side of one an
    /// anchor is on.
    pub line: Option<usize>,
    /// The y of the anchor's visual line, in document coordinates.
    pub y: f32,
}

/// A whole document's layout.
pub struct DocLayout {
    /// The visual blocks, one per source block.
    pub blocks: Vec<BlockLayout>,
    /// Table geometry aligned with [`Self::blocks`].
    pub tables: Vec<Option<TableLayout>>,
    /// Widget geometry aligned with `blocks` and `source`.
    pub widget_rows: Vec<Option<WidgetRowLayout>>,
    /// Total content height, for scroll bounds.
    pub height: f32,
    /// How much smaller than the page this layout is set: every text size
    /// and vertical measure is multiplied by it. The page is `1.0`; a margin
    /// note is the page's own layout at a smaller scale, so its measuring and
    /// its drawing read one number and can never disagree about a line break.
    pub scale: f32,
    /// A snapshot of the source blocks, so the editor can index a visual
    /// block back to its kind and runs without holding the live document.
    pub source: Vec<Block>,
    pub code_colors: Vec<Vec<Option<super::code::Ink>>>,
    /// Each sidenote anchor, in document order, with its derived number and
    /// the y of the line it sits on. Nothing stores these — like the heading
    /// outline, they are derived from position so a number can never
    /// disagree with the anchor beside it.
    pub anchors: Vec<Anchor>,
    /// Each tagged equation block's `(n)`, indexed like `source`. Derived in
    /// the same pre-pass as the anchor numbers, so the number hung in a
    /// band's margin can never disagree with what a `@eq:…` reference
    /// resolves to.
    pub equation_numbers: HashMap<usize, String>,
}

/// The smallest editable document node under a Normal-mode click.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContextHit {
    /// A prose node represented by an ordinary flat-text range.
    Range { range: FlatRange, kind: RangeKind },
    /// A whole math atom, or its deepest structural child when `node` is set.
    Math {
        block: usize,
        inline: usize,
        /// Block-flat start of the atom. In a table `inline` names the cell,
        /// so this also identifies one of several expressions in that cell.
        offset: usize,
        node: Option<NodeAddress>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeKind {
    Word,
    Badge,
    InlineCode,
    CodeBlock,
    CodeWord,
}

/// The font a run renders with. Body is serif 17.5; headings are serif at
/// 24/21/18.5 and always bold; a run's `bold`/`italic` stack on top. `scale`
/// multiplies every size, so a note laid out at the margin's scale reads from
/// here rather than from a second copy of the size table.
pub fn text_style(kind: &Block, style: Style, scale: f32) -> TextStyle {
    if style.code {
        return TextStyle::mono(17.5 * scale, theme::ink());
    }
    if style.badge {
        // A chip is set far smaller than the prose it sits in, tracked out
        // the way the design's labels are — it reads as machinery, not as
        // a word in the sentence.
        return TextStyle::mono(
            theme::BADGE_SIZE * scale,
            theme::badge_ink(style.badge_color),
        )
        .tracked(0.1);
    }
    let mut base = match kind {
        Block::Heading { level, .. } => TextStyle::sans(
            match level {
                1 => 30.0,
                2 => 23.0,
                3 => 19.0,
                _ => 17.5,
            } * scale,
            theme::ink(),
        )
        .bold(),
        Block::Paragraph(_) | Block::Divider(_) | Block::Math { .. } | Block::TableRow { .. } => {
            TextStyle::sans(17.5 * scale, theme::ink())
        }
        // A done task's text is dimmed: the drawn strike through it says
        // "done", the muted ink says "past tense". Together they quiet the
        // item without hiding it.
        Block::ListItem {
            marker: ListMarker::Task { done: true },
            ..
        } => TextStyle::sans(17.5 * scale, theme::dim()),
        Block::ListItem { .. } => TextStyle::sans(17.5 * scale, theme::ink()),
        Block::CodeLine { .. } => TextStyle::mono(17.5 * scale, theme::ink()),
        Block::WidgetRow(_) => TextStyle::sans(17.5 * scale, theme::ink()),
    };
    if style.bold {
        base = base.bold();
    }
    if style.italic {
        base = base.italic();
    }
    base
}

/// A visible portion of one table cell's text, with its source offset. Table
/// rows are structural blocks rather than prose runs, so this small layout
/// primitive gives painting, hit-testing, and the caret one wrapped geometry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableTextLine {
    pub start: usize,
    pub text: String,
}

/// The text style for a table cell. Unlike a prose block it has no heading
/// context, but it still honours the reader's normal formatting controls.
pub fn table_text_style(style: Style, scale: f32) -> TextStyle {
    if style.badge {
        return TextStyle::mono(
            theme::BADGE_SIZE * scale,
            theme::badge_ink(style.badge_color),
        )
        .tracked(0.1);
    }
    let mut result = if style.code {
        TextStyle::mono(17.5 * scale, theme::ink())
    } else {
        TextStyle::sans(17.5 * scale, theme::ink())
    };
    if style.bold {
        result = result.bold();
    }
    if style.italic {
        result = result.italic();
    }
    result
}

/// Character-wrap one cell to its usable track width. It deliberately breaks
/// long unspaced text too: a spreadsheet-like cell must grow vertically,
/// never silently paint past its right edge or discard what the reader typed.
pub fn table_text_lines(
    text: &str,
    style: &TextStyle,
    max_width: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> Vec<TableTextLine> {
    if text.is_empty() {
        return vec![TableTextLine {
            start: 0,
            text: String::new(),
        }];
    }
    let max_width = max_width.max(1.0);
    let mut lines = Vec::new();
    let mut start = 0;
    let mut current = String::new();
    for ch in text.chars() {
        let mut next = current.clone();
        next.push(ch);
        if !current.is_empty() && measure(&next, style) > max_width {
            lines.push(TableTextLine {
                start,
                text: std::mem::take(&mut current),
            });
            start += lines
                .last()
                .expect("line was just pushed")
                .text
                .chars()
                .count();
        }
        current.push(ch);
    }
    lines.push(TableTextLine {
        start,
        text: current,
    });
    lines
}

/// The raised number an anchor draws with. The size matches the heading's
/// auto-number and the colour is the accent, so an anchor reads as "this
/// opens something" rather than as a word in the sentence.
pub fn anchor_style() -> TextStyle {
    TextStyle::sans(ANCHOR_SIZE, theme::accent())
}

/// The ink an inline equation reference draws with. Resolved, it takes the
/// accent — a jump target, the way an anchor reads as "this opens
/// something". Unresolved, its raw `@eq:label` text stays on the page, set
/// small and muted: never vanishing, never an error, and never mistaken for
/// a number.
pub fn eq_ref_style(display: &str, scale: f32) -> TextStyle {
    if display.starts_with('(') {
        TextStyle::sans(17.5 * scale, theme::accent())
    } else {
        TextStyle::mono(theme::BADGE_SIZE * scale, theme::comment())
    }
}

/// One word or whitespace stretch, with its source coordinates.
struct Piece {
    text: String,
    style: Style,
    inline: usize,
    start: usize,
    len: usize,
    space: bool,
    number: Option<String>,
}

/// How far a run moves the cursor: its shaped width, plus the box
/// a badge draws around its label. A chip's box is part of the flow, not
/// decoration on top of it — measured as bare glyphs, a badge would sit
/// under the word after it.
///
/// The unit is one segment (a run's slice of one visual line), which is
/// what the caret and the editor both walk. Wrapping asks per whitespace-
/// split piece instead, so a multi-word label is measured a few pixels
/// wide there; it wraps a shade early and nothing drifts, since the caret
/// and the drawing agree with each other. The run is a parameter because an
/// atom cannot be measured from its opaque placeholder text; its math tree is
/// the thing that determines its width.
pub fn advance(
    run: &Inline,
    text: &str,
    block: &Block,
    style: Style,
    number: Option<&str>,
    scale: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> f32 {
    let width = match run {
        Inline::Math(list) => {
            // Atom placement is always body level; level is for its nested
            // fraction operands, not for making the atom itself larger.
            math_layout::layout(list, 0, scale, measure).width
        }
        Inline::Text(_) => measure(text, &text_style(block, style, scale)),
        // An anchor reserves the width of the number it actually draws, so
        // the caret, the hit tests, and the editor's drawing — all of which
        // ask `advance` — agree about where the character after the anchor
        // begins. The number is derived from position once, in `layout`, and
        // stamped onto the segment this run flows through; nothing measures
        // it from a guess.
        Inline::Note(_) => measure(number.unwrap_or("0"), &anchor_style()),
        // A reference's display — `(n)` resolved, `@eq:label` not — is
        // stamped by the numbering pass and carried on the segment, so the
        // caret, the hit tests, and the drawing all measure the same text.
        Inline::EqRef(_) => {
            let display = number.unwrap_or_default();
            measure(display, &eq_ref_style(display, scale))
        }
    };
    let box_pad = if style.badge {
        theme::BADGE_PAD * 2.0 * scale
    } else {
        0.0
    };
    width + box_pad
}

fn piece_width(
    piece: &Piece,
    kind: &Block,
    runs: &[Inline],
    scale: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> f32 {
    advance(
        &runs[piece.inline],
        &piece.text,
        kind,
        piece.style,
        piece.number.as_deref(),
        scale,
        measure,
    )
}

/// Greedy wrap over a block's pieces. A word goes on the current line if it
/// fits (`cursor + space + word <= width`); else the line breaks *before*
/// the space — the space belongs to the previous line (drawn at its right
/// edge; invisible and scissor-clipped). Ranges stay contiguous, which is
/// what keeps caret math exact. A word wider than the column overhangs on
/// its own line rather than vanishing.
/// The part of one piece a wrapped line covers. A piece that fits is covered
/// whole; a piece wider than the measure on its own is covered in as many
/// chunks as it takes, so nothing is ever painted through the edge of the
/// line (or the column) it was wrapped into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chunk {
    /// Index into the piece list.
    pub piece: usize,
    /// Char offset within the run the piece came from, so a chunk maps
    /// straight onto a [`Segment`].
    pub start: usize,
    /// Char count.
    pub len: usize,
}

impl Chunk {
    /// Whether the chunk covers its whole piece.
    fn whole(&self, pieces: &[Piece]) -> bool {
        let piece = &pieces[self.piece];
        self.start == piece.start && self.len == piece.len
    }

    /// The chunk's own text, sliced out of the piece's text.
    fn text(&self, pieces: &[Piece]) -> String {
        let piece = &pieces[self.piece];
        piece
            .text
            .chars()
            .skip(self.start - piece.start)
            .take(self.len)
            .collect()
    }
}

/// The widest prefix of `text` that fits `budget`, counted in characters.
/// Always at least one character — a glyph wider than the line it is wrapped
/// into still has to land somewhere — and never more than the text itself.
fn fitting_prefix(
    text: &str,
    budget: f32,
    style: &TextStyle,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> usize {
    let mut best = 0;
    let mut prefix = String::new();
    for (count, ch) in text.chars().enumerate() {
        prefix.push(ch);
        if measure(&prefix, style) > budget {
            break;
        }
        best = count + 1;
    }
    best.max(1).min(text.chars().count())
}

fn wrap(
    pieces: &[Piece],
    kind: &Block,
    runs: &[Inline],
    width: f32,
    scale: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
    split_overlong: bool,
) -> Vec<Vec<Chunk>> {
    let mut lines: Vec<Vec<Chunk>> = Vec::new();
    let mut current: Vec<Chunk> = Vec::new();
    let mut cursor = 0.0f32;
    let mut first = true;

    for (i, piece) in pieces.iter().enumerate() {
        let code = piece.style.code && !kind.is_code();
        let continues_code = current
            .last()
            .is_some_and(|chunk| pieces[chunk.piece].style.code);
        let style = text_style(kind, piece.style, scale);
        let base = piece_width(piece, kind, runs, scale, measure);
        // Inline code reserves its chip's margin on the outside edge only, so
        // a piece that follows code on the same line measures narrower.
        let padding = |continues: bool| {
            if code && !continues {
                INLINE_CODE_SPACE * 2.0 * scale
            } else {
                0.0
            }
        };
        let mut word_width = base + padding(continues_code);
        if !piece.space && !first && cursor + word_width > width {
            lines.push(std::mem::take(&mut current));
            cursor = 0.0;
            word_width = base + padding(false);
        }
        let mut local = 0;
        while local < piece.len {
            // Prose keeps an over-wide word whole on its own line; a table
            // cell cannot — a track is narrower than a word often enough that
            // the text would paint over its neighbour. Every line a split word
            // lands on is filled from the measure again, or the tail of the
            // word would spill across the rest of the cell in one chunk.
            let take = if !split_overlong || word_width <= width - cursor {
                piece.len - local
            } else {
                fitting_prefix(
                    &piece.text.chars().skip(local).collect::<String>(),
                    width - cursor,
                    &style,
                    measure,
                )
            };
            let len = take.min(piece.len - local).max(1);
            let chunk = Chunk {
                piece: i,
                start: piece.start + local,
                len,
            };
            let chunk_width = chunk_width(&chunk, pieces, kind, runs, scale, measure);
            cursor += if local == 0 {
                chunk_width + padding(continues_code)
            } else {
                chunk_width
            };
            current.push(chunk);
            local += len;
            if local < piece.len {
                lines.push(std::mem::take(&mut current));
                cursor = 0.0;
                first = false;
            }
        }
        if !piece.space {
            first = false;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(Vec::new());
    }
    lines
}

/// Width of one chunk: the piece's own advance when the chunk covers all of
/// it, otherwise the measured width of the slice it covers.
fn chunk_width(
    chunk: &Chunk,
    pieces: &[Piece],
    kind: &Block,
    runs: &[Inline],
    scale: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> f32 {
    let piece = &pieces[chunk.piece];
    if chunk.whole(pieces) {
        return piece_width(piece, kind, runs, scale, measure);
    }
    measure(&chunk.text(pieces), &text_style(kind, piece.style, scale))
}

/// One piece per whitespace-bounded stretch of `runs`, in run order. `runs`
/// is the exact run list the pieces address: a block's own inlines, or one
/// logical line of a table cell, whose segments therefore index that line and
/// only that line.
fn tokens(
    runs: &[Inline],
    block_index: usize,
    numbers: &HashMap<(usize, usize), String>,
) -> Vec<Piece> {
    let mut pieces = Vec::new();
    for (inline, run) in runs.iter().enumerate() {
        let text = run.text();
        let number = numbers.get(&(block_index, inline)).cloned();
        // One walk of the run's characters in char indices — the unit a
        // `Piece`'s `start`/`len` use. Collecting the indices into a `Vec`
        // first allocated once per run for nothing.
        let mut chars = text.chars().peekable();
        let mut start = 0;
        while let Some(&first) = chars.peek() {
            let space = first.is_whitespace();
            let mut piece = String::new();
            let mut len = 0;
            while let Some(&ch) = chars.peek() {
                if ch.is_whitespace() != space {
                    break;
                }
                chars.next();
                piece.push(ch);
                len += 1;
            }
            pieces.push(Piece {
                text: piece,
                style: run.style(),
                inline,
                start,
                len,
                space,
                number: number.clone(),
            });
            // Math text is one opaque ATOM character, so this already creates
            // one unsplittable non-space piece for the whole expression.
            start += len;
        }
    }
    pieces
}

/// Merge a line's piece indices into segments, folding together adjacent
/// pieces from the same run so the output is one segment per source stretch.
fn segments_for(pieces: &[Piece], line: &[Chunk], kind: &Block) -> Vec<Segment> {
    let mut segs: Vec<Segment> = Vec::new();
    for chunk in line {
        let p = &pieces[chunk.piece];
        match segs.last_mut() {
            Some(s) if s.inline == p.inline && s.start + s.len == chunk.start => {
                s.len += chunk.len;
            }
            _ => segs.push(Segment {
                padding: (0.0, 0.0),
                inline: p.inline,
                start: chunk.start,
                len: chunk.len,
                style: p.style,
                number: p.number.clone(),
            }),
        }
    }
    if !kind.is_code() {
        for i in 0..segs.len() {
            if segs[i].style.code {
                let left = i == 0 || !segs[i - 1].style.code;
                let right = i + 1 == segs.len() || !segs[i + 1].style.code;
                segs[i].padding = (
                    if left { INLINE_CODE_SPACE } else { 0.0 },
                    if right { INLINE_CODE_SPACE } else { 0.0 },
                );
            }
        }
    }
    segs
}

fn wrapped_line_content_height(
    line: &[Chunk],
    pieces: &[Piece],
    block: &Block,
    scale: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> f32 {
    line.iter()
        .filter_map(
            |&chunk| match &block.inlines()[pieces[chunk.piece].inline] {
                Inline::Math(list) => {
                    let expression = math_layout::layout(list, 0, scale, measure);
                    Some(expression.ascent + expression.descent + MATH_LEADING * scale)
                }
                Inline::Text(_) | Inline::Note(_) | Inline::EqRef(_) => None,
            },
        )
        .fold(0.0, f32::max)
}

fn layout_wrapped_lines(
    grouped: &[Vec<Chunk>],
    pieces: &[Piece],
    block: &Block,
    origin: Rect,
    base_line_height: f32,
    scale: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> Vec<VisLine> {
    let mut line_y = origin.y;
    grouped
        .iter()
        .map(|line_pieces| {
            let height = base_line_height.max(wrapped_line_content_height(
                line_pieces,
                pieces,
                block,
                scale,
                measure,
            ));
            let line = VisLine {
                y: line_y,
                x: origin.x,
                width: origin.width,
                height,
                segments: segments_for(pieces, line_pieces, block),
                cell_line: 0,
            };
            line_y += height;
            line
        })
        .collect()
}

/// One pass over `doc`'s own blocks at full scale; `measure(text, style) ->
/// width` is the only rendering input. The thin wrapper around
/// [`layout_blocks`] — a whole document is just its body at scale `1.0`.
pub fn layout(doc: &Document, width: f32, measure: &dyn Fn(&str, &TextStyle) -> f32) -> DocLayout {
    layout_blocks(doc.body(), width, 1.0, measure)
}

fn table_cell_layout(
    cell: &table::Cell,
    kind: &Block,
    width: f32,
    scale: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> Vec<VisLine> {
    let mut lines = Vec::new();
    let mut y = 0.0;
    // One piece list per logical line, so a segment's `inline` addresses the
    // line's own runs — the reading every consumer of a cell's segments makes.
    // `kind` is the row block: it picks the base text style and nothing else.
    for (index, logical) in cell.lines().iter().enumerate() {
        let pieces = tokens(logical, 0, &HashMap::new());
        // A line whose only run is an expression is a display atom in a
        // cell: it leads like a display block and centres in its column
        // instead of sitting on the text baseline.
        let display = matches!(logical.as_slice(), [Inline::Math(_)]);
        let mut display_box = None;
        if let [Inline::Math(list)] = logical.as_slice() {
            display_box = Some(math_layout::layout(list, 0, scale, measure));
        }
        for chunks in wrap(&pieces, kind, logical, width, scale, measure, true) {
            let (height, x) = if display {
                match &display_box {
                    Some(expression) => (
                        expression.ascent + expression.descent + MATH_PAD * 2.0 * scale,
                        ((width - expression.width) * 0.5).max(0.0),
                    ),
                    None => (TABLE_TEXT_LINE_HEIGHT * scale, 0.0),
                }
            } else {
                let content_height = chunks
                    .iter()
                    .filter_map(|chunk| match &logical[pieces[chunk.piece].inline] {
                        Inline::Math(list) => {
                            let expression = math_layout::layout(list, 0, scale, measure);
                            Some(expression.ascent + expression.descent + MATH_LEADING * scale)
                        }
                        Inline::Text(_) | Inline::Note(_) | Inline::EqRef(_) => None,
                    })
                    .fold(0.0, f32::max);
                ((TABLE_TEXT_LINE_HEIGHT * scale).max(content_height), 0.0)
            };
            lines.push(VisLine {
                y,
                x,
                width,
                height,
                segments: segments_for(&pieces, &chunks, kind),
                cell_line: index,
            });
            y += height;
        }
    }
    lines
}

/// The runs one logical line of a cell lays out as. Segment `inline` indices
/// address this run list — the line's own, never the whole cell — so no
/// consumer has to build (and clone) a block to read a cell line's runs.
fn table_cell_line_runs(cell: &table::Cell, line: usize) -> &[Inline] {
    let index = line.min(cell.lines().len().saturating_sub(1));
    &cell.lines()[index]
}

fn table_cell_height(lines: &[VisLine]) -> f32 {
    lines.iter().map(|line| line.height).sum()
}

fn table_cell_line_start(lines: &[VisLine], line: usize) -> usize {
    let chars: usize = lines[..line]
        .iter()
        .flat_map(|line| &line.segments)
        .map(|segment| segment.len)
        .sum();
    // One break per logical line already passed: `cell_line` counts them.
    chars + lines.get(line).map_or(0, |line| line.cell_line)
}

fn table_cell_line_of_flat(lines: &[VisLine], flat: usize) -> usize {
    let mut start = 0;
    for (index, line) in lines.iter().enumerate() {
        let end = start
            + line
                .segments
                .iter()
                .map(|segment| segment.len)
                .sum::<usize>();
        // A cell line's break sits at the end of its line, so the caret
        // stays there; a wrap boundary hands it to the next line.
        let breaks = lines
            .get(index + 1)
            .is_some_and(|next| next.cell_line != line.cell_line);
        if flat < end || (breaks && flat == end) || index + 1 == lines.len() {
            return index;
        }
        start = end + usize::from(breaks);
    }
    0
}

/// A table's geometry that every one of its rows shares: the measured
/// tracks, the row count, and the settings already normalized for this
/// shape. Measured once, at the table's first row, and handed down the rows —
/// a table's tracks do not depend on which row is being laid out.
struct TableShared {
    first: usize,
    rows: usize,
    settings: table::TableSettings,
    columns: Arc<[f32]>,
}

/// Measure a table's shared geometry once, from any of its rows: back-scan to
/// the row that owns the Markdown header, count the table's rows, normalize
/// the settings for that shape, and turn the column shares into pixel tracks.
/// `None` when `index` is not a row of a well-formed table.
fn table_shared(blocks: &[Block], index: usize, width: f32) -> Option<TableShared> {
    let Block::TableRow { settings, .. } = blocks.get(index)? else {
        return None;
    };
    let mut first = index;
    while first > 0
        && matches!(
            blocks.get(first),
            Some(Block::TableRow { first: false, .. })
        )
    {
        first -= 1;
    }
    if !blocks.get(first).is_some_and(Block::table_first) {
        return None;
    }
    let rows = (first..blocks.len())
        .take_while(|&row| {
            row == first || matches!(blocks.get(row), Some(Block::TableRow { first: false, .. }))
        })
        .count();
    let columns = blocks[first].cells().len();
    let settings = (**settings).clone().normalized(columns, rows);
    let mut tracks = Vec::with_capacity(columns);
    let mut used = 0.0;
    for (column, share) in settings.column_shares.iter().enumerate() {
        let track = if column + 1 == columns {
            (width - used).max(0.0)
        } else {
            (width * *share).max(0.0)
        };
        tracks.push(track);
        used += track;
    }
    Some(TableShared {
        first,
        rows,
        settings,
        columns: Arc::from(tracks),
    })
}

/// One row's entry in the layout snapshot. `shared` is re-measured only when
/// the row is outside the table it currently holds, so a table's tracks, row
/// count and settings are derived exactly once per pass instead of once per
/// row.
fn table_layout_for(
    blocks: &[Block],
    index: usize,
    width: f32,
    scale: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
    shared: &mut Option<TableShared>,
) -> Option<TableLayout> {
    let block = blocks.get(index)?;
    if !block.is_table() {
        return None;
    }
    let covers = shared
        .as_ref()
        .is_some_and(|table| index >= table.first && index < table.first + table.rows);
    if !covers {
        *shared = table_shared(blocks, index, width);
    }
    let shared = shared.as_ref()?;
    let cells = block
        .cells()
        .iter()
        .enumerate()
        .map(|(column, cell)| {
            table_cell_layout(
                cell,
                block,
                (shared.columns[column] - TABLE_CELL_PAD * 2.0 * scale).max(1.0),
                scale,
                measure,
            )
        })
        .collect::<Vec<_>>();
    let row = index - shared.first;
    let row_height = cells
        .iter()
        .map(|lines| {
            lines.iter().map(|line| line.height).sum::<f32>()
                + TABLE_CELL_VERTICAL_PAD * 2.0 * scale
        })
        .fold(shared.settings.row_heights[row] * scale, f32::max);
    Some(TableLayout {
        first: shared.first,
        row,
        columns: Arc::clone(&shared.columns),
        rows: shared.rows,
        row_height,
        cells,
        lines: shared.settings.lines,
    })
}

fn widget_block_fits_lane(
    block: &Block,
    lane: Rect,
    scale: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> bool {
    // Tables and display math own their full-width furniture in the existing
    // painters. Keep them below a widget band until those painters gain an
    // explicit lane origin; prose, headings, lists, dividers, and code still
    // flow through the selected free run.
    if block.is_table() || block.is_math() {
        return false;
    }
    let _ = (scale, measure, lane);
    true
}

/// A vertical run of rows whose free tracks make the same text lane. Widget
/// markers remain in source order, but the cards form one visual wall and the
/// Markdown between them may use its complete height.
#[derive(Clone, Debug)]
struct WidgetWall {
    /// The Markdown lane, from its deliberately inset first line through the
    /// bottom of the final card in the wall.
    lane: Rect,
    /// `(source block, card-top)` for every row that belongs to this wall.
    members: Vec<(usize, f32)>,
    /// Set once adjacent Markdown claims the lane. Rows added later inherit
    /// it, so their edit affordance truthfully protects the wall's text.
    has_content: bool,
    /// The final widget marker has been placed. Keep the plan through the
    /// following Markdown pass so a wall with contiguous markers can still
    /// learn that its lane is occupied.
    last_member_placed: bool,
}

impl WidgetWall {
    fn member_y(&self, source_index: usize) -> Option<f32> {
        self.members
            .iter()
            .find(|(index, _)| *index == source_index)
            .map(|(_, y)| *y)
    }

    fn final_member(&self, source_index: usize) -> bool {
        self.members
            .last()
            .is_some_and(|(index, _)| *index == source_index)
    }

    fn bounds(&self) -> Rect {
        let top = self.members[0].1;
        Rect::new(self.lane.x, top, self.lane.width, self.lane.bottom() - top)
    }
}

fn same_lane(a: Rect, b: Rect) -> bool {
    (a.x - b.x).abs() < f32::EPSILON && (a.width - b.width).abs() < f32::EPSILON
}

/// Plans every compatible widget row before adjacent prose is wrapped. The
/// look-ahead is what lets the first paragraph use space beside a later row,
/// rather than discovering the extra height after it has already reflowed.
fn widget_wall(
    blocks: &[Block],
    start: usize,
    first: &super::widget::WidgetRow,
    width: f32,
    y: f32,
    scale: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> Option<WidgetWall> {
    let first_layout = widget_layout(first, width, y, scale);
    let first_lane = first_layout.lane?;
    let mut members = vec![(start, y)];
    let mut bottom = y + first_layout.height;

    for (index, block) in blocks.iter().enumerate().skip(start + 1) {
        // The fold walk below hides the body of a collapsed heading. Do not
        // let an invisible future widget extend this visual wall.
        if block.is_folded() {
            break;
        }
        match block {
            Block::WidgetRow(row) => {
                let row_y = bottom + GAP_PARAGRAPH * scale;
                let layout = widget_layout(row, width, row_y, scale);
                let Some(lane) = layout.lane else {
                    break;
                };
                if !same_lane(first_lane, lane) {
                    break;
                }
                members.push((index, row_y));
                bottom = row_y + layout.height;
            }
            block if widget_block_fits_lane(block, first_lane, scale, measure) => {}
            _ => break,
        }
    }

    let text_top = y + WIDGET_FLOW_TOP_GAP * scale;
    Some(WidgetWall {
        lane: Rect::new(
            first_lane.x,
            text_top,
            first_lane.width,
            (bottom - text_top).max(0.0),
        ),
        members,
        has_content: false,
        last_member_placed: false,
    })
}

/// Lays out a slice of blocks — a whole document's body, or one note's body —
/// at `scale`, which multiplies every text size and vertical measure the
/// layout produces. `measure(text, style) -> width` is the only rendering
/// input. The scale is stored on the result so the measuring pass and the
/// editor's drawing pass read one number and can never disagree about where a
/// line breaks.
pub fn layout_blocks(
    blocks: &[Block],
    width: f32,
    scale: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> DocLayout {
    // Number every anchor before measuring anything, so an anchor can reserve
    // the width of the number it will actually draw. One counter, one source
    // of truth: the ordinal lands on the `Anchor` and is stamped onto the
    // pieces and segments the measuring and the drawing walks share, never
    // recomputed in either.
    let mut anchors = Vec::new();
    let mut number_of = HashMap::new();
    let mut number = 0usize;
    for (block, source) in blocks.iter().enumerate() {
        for (inline, run) in source.inlines().iter().enumerate() {
            if let Inline::Note(label) = run {
                number += 1;
                let ordinal = number.to_string();
                number_of.insert((block, inline), ordinal.clone());
                anchors.push(Anchor {
                    block,
                    inline,
                    label: label.clone(),
                    number: ordinal,
                    line: None,
                    y: 0.0,
                });
            }
        }
    }

    // Number the tagged equations in document order — one counter, one
    // source of truth, exactly the anchor scheme above. Untagged math gets
    // no number; a repeated tag takes the first occurrence's number, since
    // one label naming two equations must still resolve somewhere.
    let mut eq_of_label: HashMap<&str, usize> = HashMap::new();
    let mut equation_numbers: HashMap<usize, String> = HashMap::new();
    let mut equations = 0usize;
    for (block, source) in blocks.iter().enumerate() {
        if let Block::Math {
            tag: Some(label), ..
        } = source
        {
            eq_of_label.entry(label.as_str()).or_insert_with(|| {
                equations += 1;
                equations
            });
            if let Some(n) = eq_of_label.get(label.as_str()) {
                equation_numbers.insert(block, format!("({n})"));
            }
        }
    }
    // Resolve every reference against that map now and stamp the display it
    // will draw, the way an anchor's number is stamped onto its pieces: an
    // unresolved label keeps its raw spelling, dimmed at draw time.
    for (block, source) in blocks.iter().enumerate() {
        for (inline, run) in source.inlines().iter().enumerate() {
            if let Inline::EqRef(label) = run {
                let display = match eq_of_label.get(label.as_str()) {
                    Some(n) => format!("({n})"),
                    // The label already carries its `eq:` prefix; the raw
                    // spelling just puts the `@` back.
                    None => format!("@{label}"),
                };
                number_of.insert((block, inline), display);
            }
        }
    }

    let mut laid = Vec::with_capacity(blocks.len());
    let mut tables = Vec::with_capacity(blocks.len());
    let mut widget_rows: Vec<Option<WidgetRowLayout>> = Vec::with_capacity(blocks.len());
    let mut y = 0.0f32;
    let mut first_block = true;
    // The gap the previous block already contributed below itself. A display
    // expression tops this up to `GAP_MATH` rather than adding to it, which
    // is what keeps its space above equal to its space below.
    let mut gap_below_previous = 0.0f32;

    // The fold walk. A folded heading hides its body — everything up to the
    // next heading of level <= its own — in one instant pass (§11: reflow is
    // never animated, only recomputed). Hidden blocks are laid out for their
    // line count — the folded heading's indicator reports it — and then
    // dropped: a zero-height stub with no lines. `fold_cover` names the
    // folded heading whose body we are inside; a heading of level <= its own
    // closes the region, which makes nested folds fall out for free: a
    // folded H3 under a folded H2 is a hidden block like any other, and
    // keeps its own fold for when the H2 is unfolded.
    let mut fold_cover: Option<(usize, u8)> = None;
    let mut hidden_lines: HashMap<usize, usize> = HashMap::new();
    // The table currently being laid out. Its tracks, row count and settings
    // are measured once at its first row and reused by every row after it.
    let mut table_shared: Option<TableShared> = None;
    // A following run of ordinary Markdown may use a widget wall's free lane
    // until its cursor passes the final card's bottom edge. The plan stays
    // around after the text fills it, until every later widget marker has
    // been placed at its pre-reserved vertical position.
    let mut widget_flow: Option<Rect> = None;
    let mut wall_plan: Option<WidgetWall> = None;

    for (source_index, block) in blocks.iter().enumerate() {
        let mut hidden = false;
        if let Some((owner, level)) = fold_cover {
            match block {
                Block::Heading { level: next, .. } if *next <= level => fold_cover = None,
                _ => {
                    hidden = true;
                    *hidden_lines.entry(owner).or_insert(0) += wrap(
                        &tokens(block.inlines(), source_index, &number_of),
                        block,
                        block.inlines(),
                        width,
                        scale,
                        measure,
                        false,
                    )
                    .len();
                    laid.push(BlockLayout {
                        y,
                        lines: Vec::new(),
                        height: 0.0,
                        hidden: Some(owner),
                        indicator: None,
                    });
                    tables.push(None);
                    widget_rows.push(None);
                }
            }
        }
        if hidden {
            continue;
        }
        let planned_widget_y = wall_plan
            .as_ref()
            .and_then(|wall| wall.member_y(source_index));
        if widget_flow.is_none()
            && planned_widget_y.is_none()
            && wall_plan
                .as_ref()
                .is_some_and(|wall| wall.last_member_placed)
        {
            wall_plan = None;
        }
        let mut lane = None;
        let mut resumed_below = false;
        if let Some(flow) = widget_flow {
            if planned_widget_y.is_some() {
                // This marker belongs to the wall we already measured. It
                // must not interrupt the prose using the wall's shared lane.
            } else if block.is_widget() {
                y = flow.bottom() + GAP_PARAGRAPH * scale;
                widget_flow = None;
                wall_plan = None;
                resumed_below = true;
            } else if widget_block_fits_lane(block, flow, scale, measure) {
                lane = Some(flow);
                y = y.max(flow.y);
                if let Some(wall) = &mut wall_plan {
                    wall.has_content = true;
                    for (widget_block, _) in &wall.members {
                        if let Some(Some(row_layout)) = widget_rows.get_mut(*widget_block) {
                            row_layout.lane_has_content = true;
                        }
                    }
                }
            } else {
                y = flow.bottom() + GAP_PARAGRAPH * scale;
                widget_flow = None;
                wall_plan = None;
                resumed_below = true;
            }
        }
        let gap_above = if lane.is_some() || resumed_below || first_block {
            0.0
        } else if block.is_math() {
            (GAP_MATH * scale - gap_below_previous).max(0.0)
        } else if block.is_heading() {
            GAP_HEADING * scale
        } else {
            0.0
        };
        y += gap_above;
        first_block = false;

        if let Block::WidgetRow(row) = block {
            let row_y = planned_widget_y.unwrap_or(y);
            let mut row_layout = widget_layout(row, width, row_y, scale);
            let planned_wall =
                (planned_widget_y.is_none() && row_layout.lane.is_some()).then(|| {
                    widget_wall(blocks, source_index, row, width, row_y, scale, measure)
                        .expect("a row with a lane plans a widget wall")
                });
            row_layout.wall = if let Some(wall) = &planned_wall {
                Some(wall.bounds())
            } else {
                wall_plan.as_ref().map(WidgetWall::bounds)
            };
            if planned_widget_y.is_some() && wall_plan.as_ref().is_some_and(|wall| wall.has_content)
            {
                row_layout.lane_has_content = true;
            }
            let height = row_layout.height;
            let has_lane = row_layout.lane;
            laid.push(BlockLayout {
                y: row_y,
                lines: vec![VisLine {
                    y: row_y,
                    x: 0.0,
                    width,
                    height,
                    segments: Vec::new(),
                    cell_line: 0,
                }],
                height,
                hidden: None,
                indicator: None,
            });
            tables.push(None);
            widget_rows.push(Some(row_layout));
            if planned_widget_y.is_some() {
                if let Some(wall) = &mut wall_plan
                    && wall.final_member(source_index)
                {
                    wall.last_member_placed = true;
                }
                continue;
            }
            let gap_after = GAP_PARAGRAPH * scale;
            if let Some(mut wall) = planned_wall {
                wall.last_member_placed = wall.final_member(source_index);
                widget_flow = Some(wall.lane);
                wall_plan = Some(wall);
            } else if has_lane.is_none() {
                widget_flow = None;
                wall_plan = None;
                y += height + gap_after;
            } else {
                unreachable!("a row with a lane always has a widget wall plan");
            }
            gap_below_previous = gap_after;
            continue;
        }

        let table = table_layout_for(
            blocks,
            source_index,
            width,
            scale,
            measure,
            &mut table_shared,
        );

        let base_line_height = match block {
            Block::Heading { level: 1, .. } => LINE_H1 * scale,
            Block::Heading { level: 2, .. } => LINE_H2 * scale,
            Block::Heading { level: 3, .. } => LINE_H3 * scale,
            Block::Heading { level: 4, .. } => LINE_H4 * scale,
            Block::Divider(_) => LINE_DIVIDER * scale,
            Block::Math { list: runs, .. } => {
                let Inline::Math(list) = &runs[0] else {
                    unreachable!("math block must contain one math atom")
                };
                let expression = math_layout::layout(list, 0, scale, measure);
                expression.ascent + expression.descent + MATH_PAD * 2.0 * scale
            }
            Block::TableRow { .. } => {
                table
                    .as_ref()
                    .expect("table row must have table layout")
                    .row_height
            }
            Block::Heading { .. } | Block::Paragraph(_) | Block::CodeLine { .. } => {
                LINE_BODY * scale
            }
            Block::ListItem { .. } => LINE_BODY * scale,
            Block::WidgetRow(_) => 0.0,
        };

        // A list item's content column is indented once for the whole
        // block — first line and wraps alike — so the marker hangs in the
        // gutter and wrapped lines align under the content, never under
        // the marker.
        let indent = match block {
            Block::ListItem { .. } => LIST_INDENT * scale,
            _ => 0.0,
        };
        let available_width = lane.map_or(width, |lane| lane.width);
        let lane_x = lane.map_or(0.0, |lane| lane.x);
        let lines = if table.is_some() {
            vec![VisLine {
                y,
                x: 0.0,
                width: available_width,
                height: base_line_height,
                segments: Vec::new(),
                cell_line: 0,
            }]
        } else {
            let pieces = tokens(block.inlines(), source_index, &number_of);
            let full_width = width - indent;
            if let Some(flow) = widget_flow.filter(|_| lane.is_some()) {
                let lane_grouped = wrap(
                    &pieces,
                    block,
                    block.inlines(),
                    flow.width - indent,
                    scale,
                    measure,
                    false,
                );
                let lane_lines = layout_wrapped_lines(
                    &lane_grouped,
                    &pieces,
                    block,
                    Rect::new(lane_x + indent, y, flow.width - indent, 0.0),
                    base_line_height,
                    scale,
                    measure,
                );
                let lane_count = lane_lines
                    .iter()
                    .take_while(|line| line.y + line.height <= flow.bottom())
                    .count();
                if lane_count == 0 {
                    widget_flow = None;
                    y = flow.bottom() + GAP_PARAGRAPH * scale;
                    let grouped = wrap(
                        &pieces,
                        block,
                        block.inlines(),
                        full_width,
                        scale,
                        measure,
                        false,
                    );
                    layout_wrapped_lines(
                        &grouped,
                        &pieces,
                        block,
                        Rect::new(indent, y, full_width, 0.0),
                        base_line_height,
                        scale,
                        measure,
                    )
                } else if lane_count == lane_lines.len() {
                    lane_lines
                } else {
                    let remaining_piece = lane_grouped[lane_count]
                        .first()
                        .map_or(pieces.len(), |chunk| chunk.piece);
                    let mut lines = lane_lines[..lane_count].to_vec();
                    let suffix = &pieces[remaining_piece..];
                    let grouped = wrap(
                        suffix,
                        block,
                        block.inlines(),
                        full_width,
                        scale,
                        measure,
                        false,
                    );
                    let suffix_y = flow.bottom();
                    lines.extend(layout_wrapped_lines(
                        &grouped,
                        suffix,
                        block,
                        Rect::new(indent, suffix_y, full_width, 0.0),
                        base_line_height,
                        scale,
                        measure,
                    ));
                    lines
                }
            } else {
                let grouped = wrap(
                    &pieces,
                    block,
                    block.inlines(),
                    available_width - indent,
                    scale,
                    measure,
                    false,
                );
                layout_wrapped_lines(
                    &grouped,
                    &pieces,
                    block,
                    Rect::new(lane_x + indent, y, available_width - indent, 0.0),
                    base_line_height,
                    scale,
                    measure,
                )
            }
        };

        // VisLine heights are content-driven, so later lines start after the
        // actual height of every earlier line rather than a copied constant.
        let mut height = lines.last().map_or(0.0, |line| line.y + line.height - y);
        // A folded heading reserves its indicator's band right here, in the
        // flow, so everything below it moves in the same pass — the reflow
        // is one layout, not a settle.
        let mut indicator = None;
        if block.is_folded() && fold_region_end(blocks, source_index) > source_index + 1 {
            indicator = Some(FoldIndicator {
                y: y + height,
                lines: 0,
            });
            height += FOLD_INDICATOR_HEIGHT * scale;
            fold_cover = Some((source_index, heading_level(block)));
        }
        laid.push(BlockLayout {
            y,
            lines,
            height,
            hidden: None,
            indicator,
        });
        tables.push(table);
        widget_rows.push(None);
        let content_bottom = y + height;
        y += height;

        let grouped_with_next = (block.is_table()
            && matches!(
                blocks.get(source_index + 1),
                Some(Block::TableRow { first: false, .. })
            ))
            || (block.is_code()
                && matches!(
                    blocks.get(source_index + 1),
                    Some(Block::CodeLine { first: false, .. })
                ));
        let gap_after = if grouped_with_next {
            0.0
        } else if block.is_math() {
            GAP_MATH * scale
        } else if block.is_heading() {
            GAP_AFTER_HEADING * scale
        } else if block.is_divider() {
            GAP_DIVIDER * scale
        } else if matches!(block, Block::ListItem { .. }) {
            if matches!(blocks.get(source_index + 1), Some(Block::ListItem { .. })) {
                LIST_GAP * scale
            } else {
                GAP_PARAGRAPH * scale
            }
        } else {
            GAP_PARAGRAPH * scale
        };
        y += gap_after;
        if let Some(flow) = widget_flow
            && content_bottom >= flow.bottom()
        {
            widget_flow = None;
        }
        gap_below_previous = gap_after;
    }

    // The folded headings' indicators can only be counted while their body
    // blocks are laid out — stamp the tallies in now that the pass is done.
    for (owner, count) in &hidden_lines {
        if let Some(indicator) = &mut laid[*owner].indicator {
            indicator.lines = *count;
        }
    }

    // A row with a free lane deliberately leaves `y` at the row's top while
    // following Markdown flows beside it. If the row is the final block (or
    // every following block is hidden), there was no ordinary block pass to
    // advance the cursor past the cards themselves.
    if let Some(flow) = widget_flow {
        y = flow.bottom() + GAP_PARAGRAPH * scale;
    }

    // A note sits beside the line its anchor is on, and that line is only
    // known after the block is laid out — fill it in now that the lines
    // exist. The index and the y come from the same lookup, so they cannot
    // name two different lines.
    for anchor in &mut anchors {
        let block = &laid[anchor.block];
        anchor.line = anchor_line(block, anchor.inline);
        anchor.y = anchor.line.map_or(block.y, |index| block.lines[index].y);
    }

    DocLayout {
        blocks: laid,
        tables,
        widget_rows,
        source: blocks.to_vec(),
        code_colors: super::code::colors(blocks),
        height: y,
        scale,
        anchors,
        equation_numbers,
    }
}

/// Which visual line the anchor `inline` sits on. A note in the margin starts
/// at that line's y, so it sits beside the sentence that anchored it.
///
/// `None` when no line carries the run, which today means the block is folded
/// away and has none. The margin does not draw a note for a hidden anchor and
/// the export expands every fold before it lays out, so neither ever asks.
fn anchor_line(block: &BlockLayout, inline: usize) -> Option<usize> {
    block
        .lines
        .iter()
        .position(|line| line.segments.iter().any(|segment| segment.inline == inline))
}

/// A heading block's level; every caller has already matched the variant.
fn heading_level(block: &Block) -> u8 {
    match block {
        Block::Heading { level, .. } => *level,
        _ => 0,
    }
}

fn run_text(run: &Inline) -> &str {
    match run {
        Inline::Text(t) => &t.text,
        Inline::Math(_) => "\u{FFFC}",
        Inline::Note(_) => "\u{FFFC}",
        Inline::EqRef(_) => "\u{FFFC}",
    }
}

/// A block's characters the way a [`FlatRange`] offset indexes them: every
/// run's text end to end, with a math atom, a note, or an equation
/// reference standing in as the one object-replacement character it
/// occupies.
pub fn flat_chars(block: &Block) -> Vec<char> {
    flat_chars_of(block.inlines())
}

/// [`flat_chars`] over a bare run list — the runs of one table cell line.
fn flat_chars_of(runs: &[Inline]) -> Vec<char> {
    runs.iter().flat_map(|run| run_text(run).chars()).collect()
}

fn run_style(run: &Inline) -> Style {
    match run {
        Inline::Text(t) => t.style,
        Inline::Math(_) => Style::PLAIN,
        Inline::Note(_) => Style::PLAIN,
        Inline::EqRef(_) => Style::PLAIN,
    }
}

/// The segment's slice of its run's text.
fn segment_text(run: &Inline, segment: &Segment) -> String {
    run_text(run)
        .chars()
        .skip(segment.start)
        .take(segment.len)
        .collect()
}

/// Bounds of the contiguous code span, independent of color-run boundaries.
/// `kind` is the block whose text style the span is measured in — the block
/// itself for prose, the row block for a table cell line.
fn code_span_bounds(kind: &Block, runs: &[Inline], inline: usize) -> (usize, usize) {
    if kind.is_code() {
        return (0, runs_flat_len(runs));
    }
    let mut first = inline;
    let mut last = inline + 1;
    while first > 0 && runs[first - 1].style().code {
        first -= 1;
    }
    while last < runs.len() && runs[last].style().code {
        last += 1;
    }
    let start = runs[..first]
        .iter()
        .map(|run| run_text(run).chars().count())
        .sum();
    let end = runs[..last]
        .iter()
        .map(|run| run_text(run).chars().count())
        .sum();
    (start, end)
}

/// Flat length of a block, counting an opaque atom as one position and a table
/// row as its cells. `Block::flat_len` is the model's own rule; this exists so
/// the callers that read a block's own space go through it instead of a run
/// list, which a table row does not have.
fn block_flat_len(block: &Block) -> usize {
    block.flat_len()
}

/// Character count of a run list, counting opaque atoms as one.
fn runs_flat_len(runs: &[Inline]) -> usize {
    runs.iter().map(|run| run_text(run).chars().count()).sum()
}

/// Block-flat offset of a caret.
///
/// The unit is a run in prose and a CELL in a table row — the same arithmetic
/// `Document::caret_flat` uses, and the reason it cannot read a row's run list:
/// a table row's content is its cells, so `Block::inlines` is empty for one and
/// indexing it panicked (`caret_band` on a freshly loaded table).
fn flat_of_caret(source: &[Block], caret: Caret) -> usize {
    source.get(caret.block).map_or(0, |block| {
        let unit = caret.inline.min(block.unit_count().saturating_sub(1));
        let prefix: usize = (0..unit).map(|unit| block.unit_len(unit)).sum();
        let offset = caret.offset.min(block.unit_len(unit));
        (prefix + offset).min(block.flat_len())
    })
}

/// Style of the char at `pos` in `runs`, or `None` at the end.
fn run_style_at(runs: &[Inline], pos: usize) -> Option<Style> {
    let mut acc = 0;
    for run in runs {
        let len = run_text(run).chars().count();
        if pos < acc + len {
            return Some(run_style(run));
        }
        acc += len;
    }
    None
}

/// Style of the char before `pos` in `runs`.
fn run_style_before(runs: &[Inline], pos: usize) -> Option<Style> {
    pos.checked_sub(1)
        .and_then(|position| run_style_at(runs, position))
}

/// Style of the char at block-flat `pos`, or `None` at the block's end.
fn style_at(source: &[Block], block: usize, pos: usize) -> Option<Style> {
    run_style_at(source.get(block)?.inlines(), pos)
}

/// Style of the char before block-flat `pos`, or `None` at the block start.
fn style_before(source: &[Block], block: usize, pos: usize) -> Option<Style> {
    if pos > 0 {
        style_at(source, block, pos - 1)
    } else {
        None
    }
}

/// X of a block-flat offset resting on `line`, in pixels. `line_start` is
/// the block-flat offset where the line begins; the segments cover that
/// range contiguously, so the position is a running width over them.
fn x_of_flat(
    kind: &Block,
    runs: &[Inline],
    line: &VisLine,
    line_start: usize,
    flat: usize,
    scale: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> f32 {
    let mut x = 0.0;
    let mut seg_flat = line_start;
    for segment in &line.segments {
        let run = &runs[segment.inline];
        let text = segment_text(run, segment);
        let seg_len = segment.len;
        if flat >= seg_flat + seg_len {
            x += segment.advance(run, &text, kind, scale, measure);
            seg_flat += seg_len;
        } else {
            // The caret is inside this segment: measure its prefix, past
            // the badge box's left edge if there is one — the label starts
            // inside the box, so the caret has to as well.
            let up_to = flat - seg_flat;
            let prefix: String = text.chars().take(up_to).collect();
            x += segment.text_inset(scale);
            x += measure(&prefix, &text_style(kind, segment.style, scale));
            break;
        }
    }
    x
}

/// The visual line that flat offset falls in. At the block's very end it is
/// the last line; at a wrap boundary it is the *next* line's start.
fn line_of_flat(layout_block: &BlockLayout, flat: usize) -> usize {
    if layout_block.lines.is_empty() {
        return 0;
    }
    for (i, _) in layout_block.lines.iter().enumerate() {
        if flat < line_flat_start(layout_block, i + 1) {
            return i;
        }
    }
    layout_block.lines.len() - 1
}

/// The block the y-band falls in; past the last block → the last block.
fn block_of_y(layout: &DocLayout, y: f32) -> usize {
    for (i, block) in layout.blocks.iter().enumerate() {
        if y < block.y + block.height || i + 1 == layout.blocks.len() {
            return i;
        }
    }
    layout.blocks.len() - 1
}

/// The visual line the y-band falls in inside a block.
fn line_of_y(block: &BlockLayout, y: f32) -> usize {
    if block.lines.is_empty() {
        return 0;
    }
    for (i, line) in block.lines.iter().enumerate() {
        if y < line.y + line.height || i + 1 == block.lines.len() {
            return i;
        }
    }
    block.lines.len() - 1
}

/// The block-flat offset where a visual line starts (sum of the segment
/// lengths of every earlier line in the block). Lines cover the block's
/// flat text contiguously, so this is exact.
fn line_flat_start(layout_block: &BlockLayout, line_idx: usize) -> usize {
    let mut flat = 0;
    for line in &layout_block.lines[..line_idx] {
        for segment in &line.segments {
            flat += segment.len;
        }
    }
    flat
}

fn prefix_width(
    text: &str,
    chars: usize,
    style: &TextStyle,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> f32 {
    let end = text
        .char_indices()
        .nth(chars)
        .map_or(text.len(), |(start, _)| start);
    measure(&text[..end], style)
}

/// First character whose right edge is strictly past x, or the segment length.
fn first_char_past_x(
    text: &str,
    text_x: f32,
    x: f32,
    style: &TextStyle,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> (usize, Option<f32>) {
    let len = text.chars().count();
    let mut low = 0;
    let mut high = len;
    let mut right_width = None;
    while low < high {
        let mid = low + (high - low) / 2;
        let width = prefix_width(text, mid + 1, style, measure);
        if x < text_x + width {
            high = mid;
            right_width = Some(width);
        } else {
            low = mid + 1;
        }
    }
    if low == len {
        (len, None)
    } else {
        // Lower-bound search's final true probe is this character's right edge.
        (
            low,
            right_width.or_else(|| Some(prefix_width(text, low + 1, style, measure))),
        )
    }
}

/// Character selected by caret placement: split at each character midpoint.
fn caret_char_for_x(
    text: &str,
    text_x: f32,
    x: f32,
    style: &TextStyle,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> usize {
    let len = text.chars().count();
    let (right, right_width) = first_char_past_x(text, text_x, x, style, measure);
    if right == len {
        return len;
    }
    let left_width = if right == 0 {
        0.0
    } else {
        prefix_width(text, right, style, measure)
    };
    let right_width = right_width.expect("right-edge search found a character");
    if x <= text_x + left_width + (right_width - left_width) / 2.0 {
        right
    } else {
        right + 1
    }
}

/// Character selected by contextual hit testing: split at the right edge,
/// with the last character owning x beyond the segment.
fn context_char_for_x(
    text: &str,
    text_x: f32,
    x: f32,
    style: &TextStyle,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> usize {
    first_char_past_x(text, text_x, x, style, measure)
        .0
        .min(text.chars().count() - 1)
}

/// A caret for a click at (x, y) on a specific visual line, per the
/// style-before rule (§4.2) — split at each char's midpoint.
///
/// `scan_source` indexes against the layout's source snapshot.
fn caret_for_click(
    scan_source: &[Block],
    layout_block: &BlockLayout,
    block_idx: usize,
    line_idx: usize,
    x: f32,
    scale: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> Caret {
    let block = &scan_source[block_idx];
    let line = &layout_block.lines[line_idx];
    // Clicks arrive in content coordinates; a list item's text starts at
    // its own indent, so the aim is against the line's left edge.
    let x = x - line.x;
    let start = line_flat_start(layout_block, line_idx);

    // The clicked char's offset within the line's flat text, defaulting to
    // the line end. Segments are contiguous, so `pos` advances through them.
    let mut pos = line.segments.iter().map(|s| s.len).sum();
    let mut cum = 0.0;
    let mut seg_flat = 0;

    'segments: for segment in &line.segments {
        let run = &block.inlines()[segment.inline];
        let text = segment_text(run, segment);
        let width = segment.advance(run, &text, block, scale, measure);
        // The label sits inside its box; skip the left edge so a click
        // lands on the character the user aimed at.
        if matches!(run, Inline::Math(_) | Inline::Note(_) | Inline::EqRef(_)) {
            if x <= cum + width / 2.0 {
                pos = seg_flat;
                break 'segments;
            }
        } else {
            let style = text_style(block, segment.style, scale);
            let text_x = cum + segment.text_inset(scale);
            let ci = caret_char_for_x(&text, text_x, x, &style, measure);
            if ci < segment.len {
                pos = seg_flat + ci;
                break 'segments;
            }
        }
        cum += width;
        seg_flat += segment.len;
    }

    let flat = (start + pos).min(block_flat_len(block));
    let (inline, offset) = flat_to_pos(block, flat);
    let style = style_before(scan_source, block_idx, flat).unwrap_or(Style::PLAIN);
    Caret {
        block: block_idx,
        inline,
        offset,
        style,
    }
}

/// `(inline, offset)` for a block-flat position, clamped to the block's end.
fn flat_to_pos(block: &Block, flat: usize) -> (usize, usize) {
    let runs = block.inlines();
    if runs.is_empty() {
        return (0, 0);
    }
    let mut pos = 0;
    for (i, run) in runs.iter().enumerate() {
        let len = run_text(run).chars().count();
        if flat < pos + len {
            return (i, flat - pos);
        }
        pos += len;
    }
    let last = runs.len().saturating_sub(1);
    (last, run_text(&runs[last]).chars().count())
}

impl DocLayout {
    fn widget_caret(&self, block: usize, x: f32) -> Caret {
        let slot = self
            .widget_rows
            .get(block)
            .and_then(Option::as_ref)
            .and_then(|row| row.tracks.iter().position(|track| x < track.right()))
            .unwrap_or(super::widget::TRACKS - 1);
        Caret {
            block,
            inline: slot,
            offset: 0,
            style: Style::PLAIN,
        }
    }

    /// Resolve an overlapping widget row before ordinary block hit testing.
    /// A card claims its own rectangle; the free lane belongs to the Markdown
    /// block laid out beside it, which is the rule that keeps widgets from
    /// becoming invisible paragraph blockers.
    fn block_at(&self, x: f32, y: f32) -> usize {
        if let Some((widget_block, _)) = self.widget_at(x, y) {
            return widget_block;
        }
        if let Some(widget_block) = self.widget_lane_at(x, y) {
            if let Some((text_block, _)) = self.blocks.iter().enumerate().find(|(index, block)| {
                !self.source[*index].is_widget()
                    && block.hidden.is_none()
                    && block.lines.iter().any(|line| {
                        y >= line.y
                            && y < line.y + line.height
                            && x >= line.x
                            && x < line.x + line.width
                    })
            }) {
                return text_block;
            }
            return widget_block;
        }
        let block_idx = block_of_y(self, y);
        if !self.source[block_idx].is_widget() {
            return block_idx;
        }
        let Some(row) = self.widget_rows.get(block_idx).and_then(Option::as_ref) else {
            return block_idx;
        };
        if row.cards.iter().any(|card| card.rect.contains((x, y))) {
            return block_idx;
        }
        let Some(lane) = row.lane else {
            return block_idx;
        };
        if !lane.contains((x, y)) {
            return block_idx;
        }
        for index in block_idx + 1..self.source.len() {
            let block = &self.blocks[index];
            if block.hidden.is_some() {
                continue;
            }
            if block.y > y {
                break;
            }
            if row.lane_has_content && y < block.y + block.height {
                return index;
            }
            if block.lines.iter().any(|line| {
                y >= line.y && y < line.y + line.height && x >= line.x && x < line.x + line.width
            }) {
                return index;
            }
        }
        block_idx
    }

    /// The placed card at a point, before the Markdown lane is considered.
    pub fn widget_at(&self, x: f32, y: f32) -> Option<(usize, usize)> {
        self.widget_rows
            .iter()
            .enumerate()
            .find_map(|(block, row)| {
                row.as_ref()?.cards.iter().find_map(|card| {
                    card.rect
                        .contains((x, y))
                        .then_some((block, card.placement))
                })
            })
    }

    /// The row whose Markdown lane contains this point. Unlike source-order
    /// lookup, this remains correct when a later compatible row is part of a
    /// widget wall and shares vertical space with earlier prose.
    fn widget_lane_at(&self, x: f32, y: f32) -> Option<usize> {
        self.widget_rows
            .iter()
            .enumerate()
            .find_map(|(block, row)| {
                let row = row.as_ref()?;
                (row.lane_has_content && row.lane.is_some_and(|lane| lane.contains((x, y))))
                    .then_some(block)
            })
    }

    /// The track under a point. A free track is returned too, so the editor
    /// can open its widget chooser without adding a placeholder model object.
    pub fn widget_slot_at(&self, x: f32, y: f32) -> Option<(usize, usize)> {
        self.widget_rows
            .iter()
            .enumerate()
            .find_map(|(block, row)| {
                row.as_ref()?
                    .tracks
                    .iter()
                    .position(|track| track.contains((x, y)))
                    .map(|slot| (block, slot))
            })
    }

    /// The card close button at a point. Its rectangle is part of the layout
    /// snapshot, so painting and pointer routing cannot drift apart.
    pub fn widget_close_at(&self, x: f32, y: f32) -> Option<(usize, usize)> {
        let (block, placement) = self.widget_at(x, y)?;
        let row = self.widget_rows.get(block)?.as_ref()?;
        row.cards
            .get(placement)?
            .close
            .contains((x, y))
            .then_some((block, placement))
    }

    /// Whether a point is in a free track lane already occupied by flowing
    /// Markdown. This keeps a plain click in prose from opening the widget
    /// chooser; dragging an existing card deliberately has separate,
    /// reflow-aware acceptance.
    pub fn widget_lane_blocked_at(&self, x: f32, y: f32) -> bool {
        self.widget_rows
            .iter()
            .flatten()
            .any(|row| row.lane_has_content && row.lane.is_some_and(|lane| lane.contains((x, y))))
    }

    /// A calendar day at a point, if the point is inside a placed calendar.
    pub fn widget_day_at(&self, x: f32, y: f32) -> Option<(usize, usize, u8)> {
        let (block, placement) = self.widget_at(x, y)?;
        let row = self.widget_rows[block].as_ref()?;
        let day = row.cards[placement]
            .days
            .iter()
            .find(|day| day.rect.contains((x, y)))?;
        Some((block, placement, day.day))
    }

    /// The previous/next month control at a point, with the month delta to
    /// apply. Controls are part of the card snapshot so pointer routing and
    /// painting use the same geometry.
    pub fn widget_calendar_control_at(&self, x: f32, y: f32) -> Option<(usize, usize, i32)> {
        let (block, placement) = self.widget_at(x, y)?;
        let row = self.widget_rows.get(block)?.as_ref()?;
        let card = row.cards.get(placement)?;
        if card.previous.is_some_and(|rect| rect.contains((x, y))) {
            return Some((block, placement, -1));
        }
        if card.next.is_some_and(|rect| rect.contains((x, y))) {
            return Some((block, placement, 1));
        }
        None
    }

    /// Reports the label and y of each anchor in the document, in document
    /// order, so the margin can put a note beside the sentence that made it.
    /// The y is in document coordinates, the same space the editor scrolls in.
    pub fn note_anchors(&self) -> Vec<(String, f32)> {
        self.anchors
            .iter()
            .map(|anchor| (anchor.label.clone(), anchor.y))
            .collect()
    }

    /// A task item's checkbox, in content coordinates: `(x, y, w, h)` with
    /// x from the content column's left edge and y in document coordinates.
    /// `None` for every block that is not a task item. The drawing and the
    /// click hit-test both read this one geometry, so they cannot drift.
    pub fn task_box(&self, block_idx: usize) -> Option<(f32, f32, f32, f32)> {
        if !matches!(
            self.source.get(block_idx),
            Some(Block::ListItem {
                marker: ListMarker::Task { .. },
                ..
            })
        ) {
            return None;
        }
        let line = self.blocks.get(block_idx)?.lines.first()?;
        let mid = line.y + line.height * 0.5;
        let size = CHECK_SIZE * self.scale;
        let gap = CHECK_GAP * self.scale;
        let indent = LIST_INDENT * self.scale;
        Some((indent - gap - size, mid - size * 0.5, size, size))
    }

    /// The task item whose checkbox contains the point, if any. The frame
    /// is the same one [`Self::hit`] reads: x from the content column's
    /// left edge, y in document coordinates.
    pub fn task_at(&self, x: f32, y: f32) -> Option<usize> {
        let block_idx = self.block_at(x, y);
        let (bx, by, w, h) = self.task_box(block_idx)?;
        (x >= bx && x <= bx + w && y >= by && y <= by + h).then_some(block_idx)
    }

    /// The first row of the table containing this point. Normal-mode clicks
    /// use this to open the grid-line selector rather than a text menu.
    pub fn table_at(&self, x: f32, y: f32) -> Option<usize> {
        self.table_cell_at(x, y).map(|cell| cell.first)
    }

    /// The table cell containing a point, if any. Its geometry is shared by
    /// the context click routing and the structural controls in the card.
    pub fn table_cell_at(&self, x: f32, y: f32) -> Option<TableCell> {
        let block = self.block_at(x, y);
        let table = self.tables.get(block)?.as_ref()?;
        let layout = self.blocks.get(block)?;
        if x < 0.0
            || x > table.columns.iter().sum::<f32>()
            || y < layout.y
            || y > layout.y + layout.height
        {
            return None;
        }
        let mut edge = 0.0;
        let mut column = table.columns.len().saturating_sub(1);
        for (index, width) in table.columns.iter().enumerate() {
            edge += width;
            if x <= edge {
                column = index;
                break;
            }
        }
        Some(TableCell {
            first: table.first,
            row: table.row,
            column,
        })
    }

    /// The internal track divider under a point. Outer borders are styling,
    /// so only dividers with a neighbour resize a row or column.
    pub fn table_resize_at(&self, x: f32, y: f32, tolerance: f32) -> Option<TableResize> {
        // A boundary belongs to the following block in `block_of_y`, but a
        // row divider belongs to the row above it. Check both neighbours so
        // its hit target stays continuous across that half-open seam.
        let below = block_of_y(self, y);
        for block in [below, below.saturating_sub(1)] {
            let Some(table) = self.tables.get(block).and_then(Option::as_ref) else {
                continue;
            };
            let Some(layout) = self.blocks.get(block) else {
                continue;
            };
            if y < layout.y - tolerance || y > layout.y + layout.height + tolerance {
                continue;
            }
            if table.row + 1 < table.rows && (y - (layout.y + layout.height)).abs() <= tolerance {
                return Some(TableResize::Row {
                    first: table.first,
                    divider: table.row,
                });
            }
            let mut edge = 0.0;
            for (divider, width) in table
                .columns
                .iter()
                .enumerate()
                .take(table.columns.len() - 1)
            {
                edge += width;
                if (x - edge).abs() <= tolerance {
                    return Some(TableResize::Column {
                        first: table.first,
                        divider,
                    });
                }
            }
        }
        None
    }

    /// (x, baseline-y, line-height) of a model caret, relative to content top.
    pub fn caret_pos(
        &self,
        caret: Caret,
        measure: &dyn Fn(&str, &TextStyle) -> f32,
    ) -> (f32, f32, f32) {
        if let Some(table) = self.tables.get(caret.block).and_then(Option::as_ref) {
            let cell = caret.inline.min(table.columns.len().saturating_sub(1));
            let left: f32 = table.columns.iter().take(cell).sum();
            let inset = TABLE_CELL_PAD * self.scale;
            let lines = &table.cells[cell];
            let line_index = table_cell_line_of_flat(lines, caret.offset);
            let line = &lines[line_index];
            let kind = &self.source[caret.block];
            let runs = table_cell_line_runs(&kind.cells()[cell], line.cell_line);
            let line_start = table_cell_line_start(lines, line_index);
            let x = x_of_flat(
                kind,
                runs,
                line,
                line_start,
                caret.offset,
                self.scale,
                measure,
            );
            let content_top = self.blocks[caret.block].y
                + (table.row_height - table_cell_height(lines)).max(0.0) * 0.5;
            return (
                left + inset + line.x + x,
                content_top + line.y + line.height * 0.5,
                line.height,
            );
        }
        if self.source[caret.block].is_widget()
            && let Some(row) = self.widget_rows[caret.block].as_ref()
        {
            let track = &row.tracks[caret.inline.min(row.tracks.len().saturating_sub(1))];
            return (
                track.x + track.width * 0.5,
                track.y + track.height * 0.5,
                track.height,
            );
        }
        let flat = flat_of_caret(&self.source, caret);
        let block = &self.source[caret.block];
        let layout_block = &self.blocks[caret.block];
        if layout_block.lines.is_empty() {
            // A caret inside a folded region (an undo can restore one) has
            // no line to sit on; park it on the fold instead of panicking.
            return (
                0.0,
                layout_block.y + layout_block.height * 0.5,
                layout_block.height.max(LINE_BODY),
            );
        }
        let line_idx = line_of_flat(layout_block, flat);
        let line = &layout_block.lines[line_idx];
        let line_start = line_flat_start(layout_block, line_idx);
        let x = x_of_flat(
            block,
            block.inlines(),
            line,
            line_start,
            flat,
            self.scale,
            measure,
        );
        (x + line.x, line.y + line.height / 2.0, line.height)
    }

    /// Nearest caret position for a click at (x, y) — y relative to content
    /// top. Style context follows the style-before rule.
    pub fn hit(&self, x: f32, y: f32, measure: &dyn Fn(&str, &TextStyle) -> f32) -> Caret {
        let block_idx = self.block_at(x, y);
        if let Some(owner) = self.blocks[block_idx].hidden {
            // Folded ground: the nearest legal caret is the end of the
            // folded heading that owns the hidden block.
            let block = &self.source[owner];
            let (inline, offset) = flat_to_pos(block, block_flat_len(block));
            return Caret {
                block: owner,
                inline,
                offset,
                style: Style::PLAIN,
            };
        }
        if self.source[block_idx].is_widget() {
            let slot = self.widget_slot_at(x, y).map_or(0, |(_, slot)| slot);
            return Caret {
                block: block_idx,
                inline: slot,
                offset: 0,
                style: Style::PLAIN,
            };
        }
        if let Some(table) = self.tables.get(block_idx).and_then(Option::as_ref) {
            let mut left = 0.0;
            let mut cell = table.columns.len().saturating_sub(1);
            for (index, width) in table.columns.iter().enumerate() {
                if x < left + *width {
                    cell = index;
                    break;
                }
                left += width;
            }
            let lines = &table.cells[cell];
            let content_top = self.blocks[block_idx].y
                + (table.row_height - table_cell_height(lines)).max(0.0) * 0.5;
            let line_index = lines
                .iter()
                .position(|line| y < content_top + line.y + line.height)
                .unwrap_or(lines.len().saturating_sub(1));
            let line = &lines[line_index];
            let kind = &self.source[block_idx];
            let runs = table_cell_line_runs(&kind.cells()[cell], line.cell_line);
            let target = (x - left - TABLE_CELL_PAD * self.scale - line.x).max(0.0);
            let line_start = table_cell_line_start(lines, line_index);
            let mut position = line.segments.iter().map(|segment| segment.len).sum();
            let mut advance_x = 0.0;
            let mut segment_flat = 0;
            for segment in &line.segments {
                let run = &runs[segment.inline];
                let text = segment_text(run, segment);
                let width = segment.advance(run, &text, kind, self.scale, measure);
                if matches!(run, Inline::Math(_) | Inline::Note(_) | Inline::EqRef(_)) {
                    if target <= advance_x + width * 0.5 {
                        position = segment_flat;
                        break;
                    }
                } else {
                    let style = text_style(kind, segment.style, self.scale);
                    let text_x = advance_x + segment.text_inset(self.scale);
                    let char_index = caret_char_for_x(&text, text_x, target, &style, measure);
                    if char_index < segment.len {
                        position = segment_flat + char_index;
                        break;
                    }
                }
                advance_x += width;
                segment_flat += segment.len;
            }
            // The style belongs to the character before the caret *on this
            // line*: `position` is the line-local index, `offset` the cell-flat
            // one, and the two differ by this line's own start.
            let cell_len = self.source[block_idx].cells()[cell].flat_len();
            let offset = (line_start + position).min(cell_len);
            let style = run_style_before(runs, position).unwrap_or(Style::PLAIN);
            return Caret {
                block: block_idx,
                inline: cell,
                offset,
                style,
            };
        }
        let layout_block = &self.blocks[block_idx];
        let line_idx = line_of_y(layout_block, y);
        caret_for_click(
            &self.source,
            layout_block,
            block_idx,
            line_idx,
            x,
            self.scale,
            measure,
        )
    }

    /// The smallest contextual node under a Normal-mode click. Unlike
    /// [`Self::hit`], whitespace deliberately has no target.
    pub fn hit_context(
        &self,
        x: f32,
        y: f32,
        measure: &dyn Fn(&str, &TextStyle) -> f32,
    ) -> Option<ContextHit> {
        let block_idx = self.block_at(x, y);
        if self.blocks[block_idx].hidden.is_some() {
            return None;
        }
        if self.source[block_idx].is_widget() {
            return None;
        }
        if let Some(table) = self.tables.get(block_idx).and_then(Option::as_ref) {
            let cell = self.table_cell_at(x, y)?;
            let left: f32 = table.columns.iter().take(cell.column).sum();
            let inset = TABLE_CELL_PAD * self.scale;
            let source_cell = &self.source[block_idx].cells()[cell.column];
            let lines = &table.cells[cell.column];
            let content_top = self.blocks[block_idx].y
                + (table.row_height - table_cell_height(lines)).max(0.0) * 0.5;
            let line_index = lines
                .iter()
                .position(|line| y < content_top + line.y + line.height)?;
            let line = &lines[line_index];
            let kind = &self.source[block_idx];
            let runs = table_cell_line_runs(source_cell, line.cell_line);
            let local_x = x - left - inset - line.x;
            let baseline = content_top + line.y + line.height * 0.5;
            let cell_base: usize = self.source[block_idx].cells()[..cell.column]
                .iter()
                .map(table::Cell::flat_len)
                .sum();
            // Where this logical line starts in the cell's flat space. A word
            // found on the line is line-local; the range it names is not.
            let logical_start = source_cell.run_start(line.cell_line, 0);
            let mut advance_x = 0.0;
            for segment in &line.segments {
                let run = &runs[segment.inline];
                let run_start = source_cell.run_start(line.cell_line, segment.inline);
                let text = segment_text(run, segment);
                let width = segment.advance(run, &text, kind, self.scale, measure);
                if local_x < advance_x || local_x > advance_x + width {
                    advance_x += width;
                    continue;
                }
                if let Inline::Math(list) = run {
                    let local = (local_x - advance_x, baseline - y);
                    let expression = math_layout::layout(list, 0, self.scale, measure);
                    let bounds = math_layout::interaction_bounds(&expression);
                    return (local.0 >= bounds.left
                        && local.0 <= bounds.right
                        && local.1 >= -bounds.descent
                        && local.1 <= bounds.ascent)
                        .then(|| ContextHit::Math {
                            block: block_idx,
                            inline: cell.column,
                            offset: cell_base + run_start,
                            node: math_layout::hit_node(list, local, 0, self.scale, measure),
                        });
                }
                let run_len = run_text(run).chars().count();
                let whole_run = FlatRange::new(
                    FlatPos {
                        block: block_idx,
                        offset: cell_base + run_start,
                    },
                    FlatPos {
                        block: block_idx,
                        offset: cell_base + run_start + run_len,
                    },
                );
                if segment.style.badge {
                    return Some(ContextHit::Range {
                        range: whole_run,
                        kind: RangeKind::Badge,
                    });
                }
                if segment.style.code && !segment.style.syntax.manual {
                    return Some(ContextHit::Range {
                        range: whole_run,
                        kind: RangeKind::InlineCode,
                    });
                }
                let style = text_style(kind, segment.style, self.scale);
                let text_x = advance_x + segment.text_inset(self.scale);
                let within = context_char_for_x(&text, text_x, local_x, &style, measure);
                let ch = text.chars().nth(within)?;
                if !ch.is_alphanumeric() && ch != '_' {
                    return None;
                }
                let chars = flat_chars_of(runs);
                let clicked = run_start - logical_start + segment.start + within;
                let (lower, upper) = if segment.style.code {
                    code_span_bounds(kind, runs, segment.inline)
                } else {
                    (0, chars.len())
                };
                let mut start = clicked;
                while start > lower
                    && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_')
                {
                    start -= 1;
                }
                let mut end = clicked + 1;
                while end < upper && (chars[end].is_alphanumeric() || chars[end] == '_') {
                    end += 1;
                }
                return Some(ContextHit::Range {
                    range: FlatRange::new(
                        FlatPos {
                            block: block_idx,
                            offset: cell_base + logical_start + start,
                        },
                        FlatPos {
                            block: block_idx,
                            offset: cell_base + logical_start + end,
                        },
                    ),
                    kind: if segment.style.code {
                        RangeKind::CodeWord
                    } else {
                        RangeKind::Word
                    },
                });
            }
            return None;
        }
        let layout_block = &self.blocks[block_idx];
        let line_idx = line_of_y(layout_block, y);
        if let Some((block, inline, offset, node)) =
            self.hit_math_node_on_line(block_idx, line_idx, x, y, measure)
        {
            return Some(ContextHit::Math {
                block,
                inline,
                offset,
                node,
            });
        }

        let line = &layout_block.lines[line_idx];
        let block = &self.source[block_idx];
        let x = x - line.x;
        if block.is_code() && block_flat_len(block) == 0 {
            return Some(self.code_block_target(block_idx));
        }

        if block.is_code() && !block.inlines()[0].style().syntax.manual {
            let mut first = block_idx;
            while first > 0
                && matches!(
                    self.source.get(first),
                    Some(Block::CodeLine { first: false, .. })
                )
            {
                first -= 1;
            }
            let mut last = block_idx;
            while matches!(
                self.source.get(last + 1),
                Some(Block::CodeLine { first: false, .. })
            ) {
                last += 1;
            }
            return Some(ContextHit::Range {
                range: FlatRange::new(
                    FlatPos {
                        block: first,
                        offset: 0,
                    },
                    FlatPos {
                        block: last,
                        offset: block_flat_len(&self.source[last]),
                    },
                ),
                kind: RangeKind::CodeBlock,
            });
        }

        let mut advance_x = 0.0;
        for segment in &line.segments {
            let run = &block.inlines()[segment.inline];
            let run_start: usize = block.inlines()[..segment.inline]
                .iter()
                .map(|run| run_text(run).chars().count())
                .sum();
            let text = segment_text(run, segment);
            let width = segment.advance(run, &text, block, self.scale, measure);
            if x >= advance_x && x <= advance_x + width {
                let run_len = run_text(run).chars().count();
                let whole_run = FlatRange::new(
                    FlatPos {
                        block: block_idx,
                        offset: run_start,
                    },
                    FlatPos {
                        block: block_idx,
                        offset: run_start + run_len,
                    },
                );
                if segment.style.badge {
                    return Some(ContextHit::Range {
                        range: whole_run,
                        kind: RangeKind::Badge,
                    });
                }
                if segment.style.code && !segment.style.syntax.manual {
                    return Some(ContextHit::Range {
                        range: whole_run,
                        kind: RangeKind::InlineCode,
                    });
                }

                let style = text_style(block, segment.style, self.scale);
                let text_x = advance_x + segment.text_inset(self.scale);
                let within_segment = context_char_for_x(&text, text_x, x, &style, measure);
                let clicked = run_start + segment.start + within_segment;
                let ch = text
                    .chars()
                    .nth(within_segment)
                    .expect("segment length matches segment text");
                if ch.is_whitespace() && segment.style.code && segment.style.syntax.manual {
                    return Some(if block.is_code() {
                        self.code_block_target(block_idx)
                    } else {
                        ContextHit::Range {
                            range: whole_run,
                            kind: RangeKind::InlineCode,
                        }
                    });
                }
                if !ch.is_alphanumeric()
                    && ch != '_'
                    && !(segment.style.code && segment.style.syntax.manual && !ch.is_whitespace())
                {
                    return self.hit_hidden_math_node(block_idx, line_idx, x, y, measure);
                }
                let chars = flat_chars(block);
                let (lower, upper) = if segment.style.code {
                    code_span_bounds(block, block.inlines(), segment.inline)
                } else {
                    (0, chars.len())
                };
                let mut start = clicked;
                while (ch.is_alphanumeric() || ch == '_')
                    && start > lower
                    && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_')
                {
                    start -= 1;
                }
                let mut end = clicked + 1;
                while (ch.is_alphanumeric() || ch == '_')
                    && end < upper
                    && (chars[end].is_alphanumeric() || chars[end] == '_')
                {
                    end += 1;
                }
                return Some(ContextHit::Range {
                    range: FlatRange::new(
                        FlatPos {
                            block: block_idx,
                            offset: start,
                        },
                        FlatPos {
                            block: block_idx,
                            offset: end,
                        },
                    ),
                    kind: if segment.style.code {
                        RangeKind::CodeWord
                    } else {
                        RangeKind::Word
                    },
                });
            }
            advance_x += width;
        }
        self.hit_hidden_math_node(block_idx, line_idx, x, y, measure)
    }

    /// Every Normal-mode target touched by a circular selection brush.
    ///
    /// The query uses the same measured boxes as drawing and contextual
    /// clicks. A target is returned once even when it spans runs or lines.
    pub fn hit_contexts_in_circle(
        &self,
        x: f32,
        y: f32,
        radius: f32,
        content_width: f32,
        measure: &dyn Fn(&str, &TextStyle) -> f32,
    ) -> Vec<ContextHit> {
        let point = (x, y);
        let radius = radius.max(0.0);
        let mut hits = Vec::new();

        for (block_idx, layout_block) in self.blocks.iter().enumerate() {
            let block = &self.source[block_idx];
            if block.is_code() && !block.inlines()[0].style().syntax.manual {
                let target = self.code_block_target(block_idx);
                for line in &layout_block.lines {
                    if circle_intersects_rect(
                        point,
                        radius,
                        0.0,
                        line.y,
                        content_width,
                        line.y + line.height,
                    ) {
                        push_unique(&mut hits, target.clone());
                        break;
                    }
                }
                continue;
            }

            let chars = flat_chars(block);
            for line in &layout_block.lines {
                let point = (x - line.x, y);
                let baseline = line.y + line.height * 0.5;
                let mut advance_x = 0.0;
                for segment in &line.segments {
                    let run = &block.inlines()[segment.inline];
                    let text = segment_text(run, segment);
                    let width = segment.advance(run, &text, block, self.scale, measure);
                    let run_start: usize = block.inlines()[..segment.inline]
                        .iter()
                        .map(|run| run_text(run).chars().count())
                        .sum();
                    let run_len = run_text(run).chars().count();

                    if let Inline::Math(list) = run {
                        let local = (x - advance_x, baseline - y);
                        let expression = math_layout::layout(list, 0, self.scale, measure);
                        let bounds = math_layout::interaction_bounds(&expression);
                        if circle_intersects_rect(
                            local,
                            radius,
                            bounds.left,
                            -bounds.descent,
                            bounds.right,
                            bounds.ascent,
                        ) {
                            if list.is_empty() {
                                push_unique(
                                    &mut hits,
                                    ContextHit::Math {
                                        block: block_idx,
                                        inline: segment.inline,
                                        offset: run_start,
                                        node: None,
                                    },
                                );
                            } else {
                                for node in math_layout::hit_nodes_in_circle(
                                    list, local, radius, 0, self.scale, measure,
                                ) {
                                    push_unique(
                                        &mut hits,
                                        ContextHit::Math {
                                            block: block_idx,
                                            inline: segment.inline,
                                            offset: run_start,
                                            node: Some(node),
                                        },
                                    );
                                }
                            }
                        }
                    } else {
                        let whole_run = FlatRange::new(
                            FlatPos {
                                block: block_idx,
                                offset: run_start,
                            },
                            FlatPos {
                                block: block_idx,
                                offset: run_start + run_len,
                            },
                        );
                        if segment.style.badge {
                            if circle_intersects_rect(
                                point,
                                radius,
                                advance_x,
                                baseline - theme::BADGE_HEIGHT * 0.5,
                                advance_x + width,
                                baseline + theme::BADGE_HEIGHT * 0.5,
                            ) {
                                push_unique(
                                    &mut hits,
                                    ContextHit::Range {
                                        range: whole_run,
                                        kind: RangeKind::Badge,
                                    },
                                );
                            }
                        } else if segment.style.code && !segment.style.syntax.manual {
                            if circle_intersects_rect(
                                point,
                                radius,
                                advance_x,
                                line.y,
                                advance_x + width,
                                line.y + line.height,
                            ) {
                                push_unique(
                                    &mut hits,
                                    ContextHit::Range {
                                        range: whole_run,
                                        kind: RangeKind::InlineCode,
                                    },
                                );
                            }
                        } else {
                            let mut char_x = advance_x + segment.text_inset(self.scale);
                            for (within_segment, ch) in text.chars().enumerate() {
                                let char_width = measure(
                                    &ch.to_string(),
                                    &text_style(block, segment.style, self.scale),
                                );
                                if (ch.is_alphanumeric()
                                    || ch == '_'
                                    || (segment.style.code && !ch.is_whitespace()))
                                    && circle_intersects_rect(
                                        point,
                                        radius,
                                        char_x,
                                        line.y,
                                        char_x + char_width,
                                        line.y + line.height,
                                    )
                                {
                                    let clicked = run_start + segment.start + within_segment;
                                    let (lower, upper) = if segment.style.code {
                                        code_span_bounds(block, block.inlines(), segment.inline)
                                    } else {
                                        (0, chars.len())
                                    };
                                    let mut start = clicked;
                                    while (ch.is_alphanumeric() || ch == '_')
                                        && start > lower
                                        && (chars[start - 1].is_alphanumeric()
                                            || chars[start - 1] == '_')
                                    {
                                        start -= 1;
                                    }
                                    let mut end = clicked + 1;
                                    while (ch.is_alphanumeric() || ch == '_')
                                        && end < upper
                                        && (chars[end].is_alphanumeric() || chars[end] == '_')
                                    {
                                        end += 1;
                                    }
                                    push_unique(
                                        &mut hits,
                                        ContextHit::Range {
                                            range: FlatRange::new(
                                                FlatPos {
                                                    block: block_idx,
                                                    offset: start,
                                                },
                                                FlatPos {
                                                    block: block_idx,
                                                    offset: end,
                                                },
                                            ),
                                            kind: if segment.style.code {
                                                RangeKind::CodeWord
                                            } else {
                                                RangeKind::Word
                                            },
                                        },
                                    );
                                }
                                char_x += char_width;
                            }
                        }
                    }
                    advance_x += width;
                }
            }
        }
        hits
    }

    fn code_block_target(&self, block_idx: usize) -> ContextHit {
        let mut first = block_idx;
        while first > 0
            && matches!(
                self.source.get(first),
                Some(Block::CodeLine { first: false, .. })
            )
        {
            first -= 1;
        }
        let mut last = block_idx;
        while matches!(
            self.source.get(last + 1),
            Some(Block::CodeLine { first: false, .. })
        ) {
            last += 1;
        }
        ContextHit::Range {
            range: FlatRange::new(
                FlatPos {
                    block: first,
                    offset: 0,
                },
                FlatPos {
                    block: last,
                    offset: block_flat_len(&self.source[last]),
                },
            ),
            kind: RangeKind::CodeBlock,
        }
    }

    fn hit_hidden_math_node(
        &self,
        current_block: usize,
        current_line: usize,
        x: f32,
        y: f32,
        measure: &dyn Fn(&str, &TextStyle) -> f32,
    ) -> Option<ContextHit> {
        for (other_block, layout_block) in self.blocks.iter().enumerate() {
            for other_line in 0..layout_block.lines.len() {
                if (other_block, other_line) == (current_block, current_line) {
                    continue;
                }
                if let Some(hit) =
                    self.hit_math_node_on_line(other_block, other_line, x, y, measure)
                {
                    return Some(ContextHit::Math {
                        block: hit.0,
                        inline: hit.1,
                        offset: hit.2,
                        node: hit.3,
                    });
                }
            }
        }
        None
    }

    fn hit_math_node_on_line(
        &self,
        block_idx: usize,
        line_idx: usize,
        x: f32,
        y: f32,
        measure: &dyn Fn(&str, &TextStyle) -> f32,
    ) -> Option<(usize, usize, usize, Option<NodeAddress>)> {
        let line = &self.blocks[block_idx].lines[line_idx];
        let block = &self.source[block_idx];
        let x = x - line.x;
        let mut advance_x = 0.0;
        for segment in &line.segments {
            let run = &block.inlines()[segment.inline];
            let text = segment_text(run, segment);
            let width = segment.advance(run, &text, block, self.scale, measure);
            if let Inline::Math(list) = run {
                let local = (x - advance_x, line.y + line.height / 2.0 - y);
                let expression = math_layout::layout(list, 0, self.scale, measure);
                let bounds = math_layout::interaction_bounds(&expression);
                if local.0 >= bounds.left
                    && local.0 <= bounds.right
                    && local.1 >= -bounds.descent
                    && local.1 <= bounds.ascent
                {
                    return Some((
                        block_idx,
                        segment.inline,
                        block.inlines()[..segment.inline]
                            .iter()
                            .map(|run| run_text(run).chars().count())
                            .sum(),
                        math_layout::hit_node(list, local, 0, self.scale, measure),
                    ));
                }
            }
            advance_x += width;
        }
        None
    }

    /// The atom a point landed inside, with the cursor seated where the
    /// point falls in it. `None` when the point is not inside an atom's box.
    pub fn hit_math(
        &self,
        x: f32,
        y: f32,
        measure: &dyn Fn(&str, &TextStyle) -> f32,
    ) -> Option<(usize, usize, usize, MathCursor)> {
        let block_idx = self.block_at(x, y);
        if self.blocks[block_idx].hidden.is_some() {
            return None;
        }
        if let Some(table) = self.tables.get(block_idx).and_then(Option::as_ref) {
            let cell = self.table_cell_at(x, y)?;
            let left: f32 = table.columns.iter().take(cell.column).sum();
            let inset = TABLE_CELL_PAD * self.scale;
            let source_cell = &self.source[block_idx].cells()[cell.column];
            let lines = &table.cells[cell.column];
            let content_top = self.blocks[block_idx].y
                + (table.row_height - table_cell_height(lines)).max(0.0) * 0.5;
            let line = lines.iter().find(|line| {
                y >= content_top + line.y && y <= content_top + line.y + line.height
            })?;
            let kind = &self.source[block_idx];
            let runs = table_cell_line_runs(source_cell, line.cell_line);
            let baseline = content_top + line.y + line.height * 0.5;
            let local_x = x - left - inset - line.x;
            let mut advance_x = 0.0;
            for segment in &line.segments {
                let run = &runs[segment.inline];
                let text = segment_text(run, segment);
                let width = segment.advance(run, &text, kind, self.scale, measure);
                if let Inline::Math(list) = run {
                    let local = (local_x - advance_x, baseline - y);
                    let expression = math_layout::layout(list, 0, self.scale, measure);
                    let bounds = math_layout::interaction_bounds(&expression);
                    if local.0 >= bounds.left
                        && local.0 <= bounds.right
                        && local.1 >= -bounds.descent
                        && local.1 <= bounds.ascent
                    {
                        let offset = self.source[block_idx].cells()[..cell.column]
                            .iter()
                            .map(table::Cell::flat_len)
                            .sum::<usize>()
                            + source_cell.run_start(line.cell_line, segment.inline);
                        return Some((
                            block_idx,
                            cell.column,
                            offset,
                            math_layout::hit(list, local, 0, self.scale, measure),
                        ));
                    }
                }
                advance_x += width;
            }
            return None;
        }
        let layout_block = &self.blocks[block_idx];
        let line_idx = line_of_y(layout_block, y);
        if let Some(hit) = self.hit_math_on_line(block_idx, line_idx, x, y, measure) {
            return Some(hit);
        }

        // Non-painting structural slots deliberately do not grow their line.
        // Search the other lines only after the visual line under the pointer,
        // so ordinary content keeps priority where interaction envelopes meet.
        for (other_block, layout_block) in self.blocks.iter().enumerate() {
            for other_line in 0..layout_block.lines.len() {
                if (other_block, other_line) == (block_idx, line_idx) {
                    continue;
                }
                if let Some(hit) = self.hit_math_on_line(other_block, other_line, x, y, measure) {
                    return Some(hit);
                }
            }
        }
        None
    }

    fn hit_math_on_line(
        &self,
        block_idx: usize,
        line_idx: usize,
        x: f32,
        y: f32,
        measure: &dyn Fn(&str, &TextStyle) -> f32,
    ) -> Option<(usize, usize, usize, MathCursor)> {
        let line = &self.blocks[block_idx].lines[line_idx];
        let block = &self.source[block_idx];
        let x = x - line.x;
        let mut advance_x = 0.0;

        for segment in &line.segments {
            let run = &block.inlines()[segment.inline];
            let text = segment_text(run, segment);
            let width = segment.advance(run, &text, block, self.scale, measure);
            if let Inline::Math(list) = run {
                let local_x = x - advance_x;
                // Math boxes use positive-up y; the line baseline is its centre.
                let local_y = line.y + line.height / 2.0 - y;
                let expression = math_layout::layout(list, 0, self.scale, measure);
                let bounds = math_layout::interaction_bounds(&expression);
                if local_x >= bounds.left
                    && local_x <= bounds.right
                    && local_y >= -bounds.descent
                    && local_y <= bounds.ascent
                {
                    return Some((
                        block_idx,
                        segment.inline,
                        block.inlines()[..segment.inline]
                            .iter()
                            .map(|run| run_text(run).chars().count())
                            .sum(),
                        math_layout::hit(list, (local_x, local_y), 0, self.scale, measure),
                    ));
                }
            }
            advance_x += width;
        }
        None
    }

    /// One visual line up from `caret`, aiming at `goal_x` pixels.
    pub fn line_up(
        &self,
        caret: Caret,
        goal_x: f32,
        measure: &dyn Fn(&str, &TextStyle) -> f32,
    ) -> Option<Caret> {
        if self.tables.get(caret.block).is_some_and(Option::is_some) {
            if caret.block == 0 {
                return None;
            }
            let previous = self.visible_before(caret.block)?;
            let target = &self.blocks[previous];
            return if self.tables.get(previous).is_some_and(Option::is_some) {
                Some(self.hit(goal_x, target.y + target.height * 0.5, measure))
            } else if self.source[previous].is_widget() {
                Some(self.widget_caret(previous, goal_x))
            } else {
                let last_line = target.lines.len() - 1;
                Some(caret_for_click(
                    &self.source,
                    target,
                    previous,
                    last_line,
                    goal_x,
                    self.scale,
                    measure,
                ))
            };
        }
        let flat = flat_of_caret(&self.source, caret);
        let block_idx = caret.block;
        let layout_block = &self.blocks[block_idx];
        let line_idx = line_of_flat(layout_block, flat);

        if line_idx > 0 {
            return Some(caret_for_click(
                &self.source,
                &self.blocks[block_idx],
                block_idx,
                line_idx - 1,
                goal_x,
                self.scale,
                measure,
            ));
        }
        if block_idx == 0 {
            return None;
        }
        let prev = self.visible_before(block_idx)?;
        if self.tables.get(prev).is_some_and(Option::is_some) {
            let target = &self.blocks[prev];
            return Some(self.hit(goal_x, target.y + target.height * 0.5, measure));
        }
        if self.source[prev].is_widget() {
            return Some(self.widget_caret(prev, goal_x));
        }
        let last_line = self.blocks[prev].lines.len() - 1;
        Some(caret_for_click(
            &self.source,
            &self.blocks[prev],
            prev,
            last_line,
            goal_x,
            self.scale,
            measure,
        ))
    }

    /// The nearest visible block above `block_idx`, skipping folded ground —
    /// vertical motion and folded sections agree that hidden blocks are not
    /// places a caret can rest.
    fn visible_before(&self, block_idx: usize) -> Option<usize> {
        (0..block_idx)
            .rev()
            .find(|&index| self.blocks[index].hidden.is_none())
    }

    fn visible_after(&self, block_idx: usize) -> Option<usize> {
        (block_idx + 1..self.blocks.len()).find(|&index| self.blocks[index].hidden.is_none())
    }

    /// One visual line down from `caret`, aiming at `goal_x` pixels.
    pub fn line_down(
        &self,
        caret: Caret,
        goal_x: f32,
        measure: &dyn Fn(&str, &TextStyle) -> f32,
    ) -> Option<Caret> {
        if self.tables.get(caret.block).is_some_and(Option::is_some) {
            let next = self.visible_after(caret.block)?;
            let target = &self.blocks[next];
            return if self.tables.get(next).is_some_and(Option::is_some) {
                Some(self.hit(goal_x, target.y + target.height * 0.5, measure))
            } else if self.source[next].is_widget() {
                Some(self.widget_caret(next, goal_x))
            } else {
                Some(caret_for_click(
                    &self.source,
                    target,
                    next,
                    0,
                    goal_x,
                    self.scale,
                    measure,
                ))
            };
        }
        let flat = flat_of_caret(&self.source, caret);
        let block_idx = caret.block;
        let layout_block = &self.blocks[block_idx];
        let line_idx = line_of_flat(layout_block, flat);

        if line_idx + 1 < layout_block.lines.len() {
            return Some(caret_for_click(
                &self.source,
                &self.blocks[block_idx],
                block_idx,
                line_idx + 1,
                goal_x,
                self.scale,
                measure,
            ));
        }
        let next = self.visible_after(block_idx)?;
        if self.tables.get(next).is_some_and(Option::is_some) {
            let target = &self.blocks[next];
            return Some(self.hit(goal_x, target.y + target.height * 0.5, measure));
        }
        if self.source[next].is_widget() {
            return Some(self.widget_caret(next, goal_x));
        }
        Some(caret_for_click(
            &self.source,
            &self.blocks[next],
            next,
            0,
            goal_x,
            self.scale,
            measure,
        ))
    }

    /// The caret's visual line's [top, bottom) — scroll-follow reads this.
    pub fn caret_band(&self, caret: Caret) -> (f32, f32) {
        let flat = flat_of_caret(&self.source, caret);
        let layout_block = &self.blocks[caret.block];
        if layout_block.lines.is_empty() {
            return (layout_block.y, layout_block.y + layout_block.height);
        }
        let line_idx = line_of_flat(layout_block, flat);
        let line = &layout_block.lines[line_idx];
        (line.y, line.y + line.height)
    }

    /// The folded heading whose gutter chevron a click at `(x, y)` landed
    /// on, in content coordinates. The band is the chevron's own hit pad —
    /// a 5px triangle is a cruel target — but nothing more, so a click on
    /// the heading's number or text stays a caret placement.
    pub fn fold_chevron_at(
        &self,
        x: f32,
        y: f32,
        measure: &dyn Fn(&str, &TextStyle) -> f32,
    ) -> Option<usize> {
        let block_idx = block_of_y(self, y);
        if !self.source[block_idx].is_heading() {
            return None;
        }
        let layout_block = &self.blocks[block_idx];
        let line = layout_block.lines.first()?;
        if y < line.y || y >= line.y + line.height {
            return None;
        }
        let right = self.chevron_right(block_idx, measure);
        let width = CHEVRON_WIDTH * self.scale;
        let left = right - width - CHEVRON_HIT_PAD * self.scale;
        (x >= left && x <= right + CHEVRON_HIT_PAD * self.scale).then_some(block_idx)
    }

    /// The folded heading whose collapsed-body indicator a click at `y`
    /// landed on. The whole column is the target: the indicator is the one
    /// quiet line standing in for everything hidden, and clicking it
    /// unfolds the fold.
    pub fn fold_indicator_at(&self, y: f32) -> Option<usize> {
        self.blocks.iter().position(|block| {
            block.indicator.as_ref().is_some_and(|indicator| {
                y >= indicator.y && y < indicator.y + FOLD_INDICATOR_HEIGHT * self.scale
            })
        })
    }

    /// Content x of the chevron's right edge on `block`'s first line — the
    /// auto-number's left edge minus [`CHEVRON_GAP`], clamped so a deeply
    /// numbered heading cannot push the chevron out of the gutter.
    fn chevron_right(&self, block: usize, measure: &dyn Fn(&str, &TextStyle) -> f32) -> f32 {
        let width = outline::outline(&self.source)
            .iter()
            .find(|node| node.block == block)
            .map(|node| measure(&node.number, &TextStyle::mono(NUMBER_SIZE, theme::ink())))
            .unwrap_or(0.0);
        chevron_right(width, self.scale)
    }
}

fn circle_intersects_rect(
    point: (f32, f32),
    radius: f32,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
) -> bool {
    let nearest_x = point.0.clamp(left.min(right), left.max(right));
    let nearest_y = point.1.clamp(top.min(bottom), top.max(bottom));
    let dx = point.0 - nearest_x;
    let dy = point.1 - nearest_y;
    dx * dx + dy * dy <= radius * radius
}

fn push_unique(hits: &mut Vec<ContextHit>, hit: ContextHit) {
    if !hits.contains(&hit) {
        hits.push(hit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::math::{BigOp, MathNode, Slot, Step};
    use crate::document::math_layout::BoxKind;
    use crate::document::widget::{
        CalendarWidget, ClarityWidget, Widget, WidgetPlacement, WidgetRow,
    };
    use crate::document::{Caret, Focus, Inline, Sidenote, Text};
    use std::cell::Cell;

    /// Every glyph 10 wide, so line breaks are countable by hand.
    fn fake_measure(text: &str, style: &TextStyle) -> f32 {
        let _ = style;
        text.chars().count() as f32 * 10.0
    }

    /// A display expression is inset by `GAP_MATH` above *and* below, whoever
    /// its neighbours are — the gap tops the previous block's own up rather
    /// than stacking on it, so the equation is not pushed off-centre by the
    /// paragraph that happens to precede it.
    #[test]
    fn a_display_expression_is_inset_equally_above_and_below() {
        let math = Block::Math {
            list: vec![Inline::Math(vec![MathNode::Sym('x')])],
            tag: None,
        };
        let text = || {
            Block::Paragraph(vec![Inline::Text(Text {
                text: "a".into(),
                style: Style::PLAIN,
            })])
        };
        let blocks = vec![text(), math, text()];
        let laid = layout_blocks(&blocks, 400.0, 1.0, &fake_measure);

        let above = laid.blocks[1].y - (laid.blocks[0].y + laid.blocks[0].height);
        let below = laid.blocks[2].y - (laid.blocks[1].y + laid.blocks[1].height);
        assert!(
            (above - GAP_MATH).abs() < 0.0001,
            "space above was {above}, want {GAP_MATH}"
        );
        assert!(
            (below - GAP_MATH).abs() < 0.0001,
            "space below was {below}, want {GAP_MATH}"
        );
        const {
            assert!(
                GAP_MATH > GAP_PARAGRAPH,
                "a display expression breathes more than a paragraph"
            )
        };
    }

    /// A heading's `gap_after` is tighter than a paragraph's, so the top-up
    /// has to be larger there. The result is the same either way.
    #[test]
    fn a_display_expression_after_a_heading_keeps_the_same_inset() {
        let blocks = vec![
            Block::Heading {
                level: 1,
                folded: false,
                content: vec![Inline::Text(Text {
                    text: "h".into(),
                    style: Style::PLAIN,
                })],
            },
            Block::Math {
                list: vec![Inline::Math(vec![MathNode::Sym('x')])],
                tag: None,
            },
        ];
        let laid = layout_blocks(&blocks, 400.0, 1.0, &fake_measure);
        let above = laid.blocks[1].y - (laid.blocks[0].y + laid.blocks[0].height);
        assert!((above - GAP_MATH).abs() < 0.0001, "space above was {above}");
    }

    fn shaped_measure(text: &str, style: &TextStyle) -> f32 {
        let _ = style;
        let chars = text.chars().count() as f32;
        if chars > 1.0 {
            chars * 10.0 - (chars - 1.0) * 2.0
        } else {
            chars * 10.0
        }
    }

    fn linear_caret_char(
        text: &str,
        x: f32,
        style: &TextStyle,
        measure: &dyn Fn(&str, &TextStyle) -> f32,
    ) -> usize {
        let mut left = 0.0;
        for (index, _) in text.chars().enumerate() {
            let prefix: String = text.chars().take(index + 1).collect();
            let right = measure(&prefix, style);
            if x <= left + (right - left) / 2.0 {
                return index;
            }
            left = right;
        }
        text.chars().count()
    }

    fn linear_context_char(
        text: &str,
        x: f32,
        style: &TextStyle,
        measure: &dyn Fn(&str, &TextStyle) -> f32,
    ) -> usize {
        let len = text.chars().count();
        for (index, _) in text.chars().enumerate() {
            let prefix: String = text.chars().take(index + 1).collect();
            if x < measure(&prefix, style) || index + 1 == len {
                return index;
            }
        }
        len - 1
    }

    fn doc_with(blocks: Vec<Block>) -> Document {
        let mut d = Document::new(std::path::Path::new("notes/t.md"));
        *d.body_mut() = blocks;
        d
    }

    fn para(text: &str) -> Block {
        Block::Paragraph(vec![Inline::Text(Text {
            text: text.into(),
            style: Style::PLAIN,
        })])
    }

    #[test]
    fn widgets_keep_prose_in_the_largest_free_lane() {
        let row = WidgetRow {
            placements: vec![
                WidgetPlacement {
                    slot: 0,
                    span: 1,
                    widget: Widget::Calendar(CalendarWidget {
                        year: 2026,
                        month: 9,
                        heading: crate::document::widget::CalendarHeading::Both,
                        selected: vec![12],
                    }),
                },
                WidgetPlacement {
                    slot: 2,
                    span: 1,
                    widget: Widget::Clarity(ClarityWidget::default()),
                },
            ],
        };
        let laid = layout_blocks(
            &[Block::WidgetRow(row), para("hello world")],
            400.0,
            1.0,
            &fake_measure,
        );
        let widget = laid.widget_rows[0].as_ref().expect("widget geometry");
        let lane = widget.lane.expect("one free track remains");
        assert!(widget.lane_has_content);
        assert_eq!(widget.cards.len(), 2);
        assert_eq!(widget.cards[0].days.len(), 30);
        assert_eq!(laid.blocks[1].lines[0].x, lane.x);
        assert_eq!(laid.blocks[1].lines[0].width, lane.width);
        assert!(laid.height > widget.height);

        let card_point = widget.cards[0].rect.position();
        assert_eq!(
            laid.widget_at(card_point.0 + 4.0, card_point.1 + 4.0),
            Some((0, 0))
        );
        let prose_point = (lane.x + 5.0, laid.blocks[1].lines[0].y + 5.0);
        assert!(laid.widget_lane_blocked_at(prose_point.0, prose_point.1));
        assert_eq!(
            laid.hit(prose_point.0, prose_point.1, &fake_measure).block,
            1
        );
        assert_eq!(
            laid.widget_slot_at(prose_point.0, prose_point.1),
            Some((0, 1))
        );
        let previous = widget.cards[0].previous.expect("previous month control");
        let next = widget.cards[0].next.expect("next month control");
        assert!(widget.cards[0].rect.contains(previous.position()));
        assert!(widget.cards[0].rect.contains((
            previous.right() - f32::EPSILON,
            previous.bottom() - f32::EPSILON,
        )));
        assert!(widget.cards[0].rect.contains(next.position()));
        assert!(
            widget.cards[0]
                .rect
                .contains((next.right() - f32::EPSILON, next.bottom() - f32::EPSILON,))
        );
        assert_eq!(
            laid.widget_calendar_control_at(previous.x + 2.0, previous.y + 2.0),
            Some((0, 0, -1))
        );
        assert_eq!(
            laid.widget_calendar_control_at(next.x + 2.0, next.y + 2.0),
            Some((0, 0, 1))
        );
    }

    #[test]
    fn moving_a_widget_reflows_the_adjacent_prose() {
        let mut row = WidgetRow::new(Widget::Empty);
        let text = "These words begin beside the widget and should follow it when the card moves.";
        let before = layout_blocks(
            &[Block::WidgetRow(row.clone()), para(text)],
            400.0,
            1.0,
            &fake_measure,
        );
        let before_lane = before.widget_rows[0].as_ref().unwrap().lane.unwrap();

        assert!(before.widget_rows[0].as_ref().unwrap().lane_has_content);
        assert_eq!(before.blocks[1].lines[0].x, before_lane.x);
        assert!(before_lane.x > 0.0);

        assert!(row.move_at(0, 3));
        let after = layout_blocks(
            &[Block::WidgetRow(row), para(text)],
            400.0,
            1.0,
            &fake_measure,
        );
        let after_lane = after.widget_rows[0].as_ref().unwrap().lane.unwrap();

        assert!(after.widget_rows[0].as_ref().unwrap().lane_has_content);
        assert_eq!(after.blocks[1].lines[0].x, after_lane.x);
        assert_eq!(after_lane.x, 0.0);
        assert_ne!(before_lane, after_lane);
    }

    #[test]
    fn prose_rewraps_at_the_bottom_of_a_widget_lane() {
        let row = WidgetRow::new(Widget::Calendar(CalendarWidget {
            year: 2026,
            month: 9,
            heading: crate::document::widget::CalendarHeading::Both,
            selected: Vec::new(),
        }));
        let text = (0..50).map(|_| "word").collect::<Vec<_>>().join(" ");
        let laid = layout_blocks(
            &[Block::WidgetRow(row), para(&text)],
            400.0,
            1.0,
            &fake_measure,
        );
        let widget = laid.widget_rows[0].as_ref().expect("widget geometry");
        let lane = widget.lane.expect("one free track remains");
        let paragraph = &laid.blocks[1];
        let first_full_width = paragraph
            .lines
            .iter()
            .position(|line| line.y >= widget.height)
            .expect("long prose continues below the widget");

        assert!(first_full_width > 0);
        assert!(
            paragraph.lines[..first_full_width]
                .iter()
                .all(|line| line.x == lane.x && line.width == lane.width)
        );
        assert!(
            paragraph.lines[first_full_width..]
                .iter()
                .all(|line| line.x == 0.0 && line.width == 400.0)
        );
        assert_eq!(paragraph.lines[first_full_width].y, widget.height);
        assert_eq!(
            laid.height,
            paragraph.lines.last().unwrap().y
                + paragraph.lines.last().unwrap().height
                + GAP_PARAGRAPH
        );
    }

    #[test]
    fn compatible_widget_rows_make_one_tall_markdown_wall() {
        let calendar = WidgetRow::new(Widget::Calendar(CalendarWidget {
            year: 2026,
            month: 9,
            heading: crate::document::widget::CalendarHeading::Both,
            selected: Vec::new(),
        }));
        let clarity = WidgetRow::new(Widget::Clarity(ClarityWidget::default()));
        let text = std::iter::repeat_n("word", 100)
            .collect::<Vec<_>>()
            .join(" ");
        let laid = layout_blocks(
            &[
                Block::WidgetRow(calendar),
                para(&text),
                Block::WidgetRow(clarity),
            ],
            400.0,
            1.0,
            &fake_measure,
        );
        let first = laid.widget_rows[0].as_ref().unwrap();
        let second = laid.widget_rows[2].as_ref().unwrap();
        let lane = first.lane.unwrap();
        let paragraph = &laid.blocks[1];
        let wall_bottom = second.height + second.cards[0].rect.y;

        assert_eq!(paragraph.lines[0].y, WIDGET_FLOW_TOP_GAP);
        assert_eq!(second.cards[0].rect.y, first.height + GAP_PARAGRAPH);
        assert_eq!(first.wall, second.wall);
        assert!(first.lane_has_content && second.lane_has_content);
        assert!(paragraph.lines.iter().any(|line| {
            line.y >= second.cards[0].rect.y
                && line.y < wall_bottom
                && line.x == lane.x
                && line.width == lane.width
        }));
        let first_full_width = paragraph
            .lines
            .iter()
            .position(|line| line.x == 0.0)
            .expect("long prose eventually leaves the wall");
        assert_eq!(paragraph.lines[first_full_width].y, wall_bottom);

        let prose = paragraph
            .lines
            .iter()
            .find(|line| line.y >= second.cards[0].rect.y && line.y < wall_bottom)
            .unwrap();
        assert_eq!(
            laid.hit(lane.x + 5.0, prose.y + 5.0, &fake_measure).block,
            1
        );
        let card = second.cards[0].rect;
        assert_eq!(laid.widget_at(card.x + 4.0, card.y + 4.0), Some((2, 0)));
        assert_eq!(laid.hit(card.x + 4.0, card.y + 4.0, &fake_measure).block, 2);
    }

    #[test]
    fn contiguous_widget_rows_reserve_their_shared_text_lane() {
        let calendar = WidgetRow::new(Widget::Calendar(CalendarWidget {
            year: 2026,
            month: 9,
            heading: crate::document::widget::CalendarHeading::Both,
            selected: Vec::new(),
        }));
        let clarity = WidgetRow::new(Widget::Clarity(ClarityWidget::default()));
        let text = std::iter::repeat_n("word", 60)
            .collect::<Vec<_>>()
            .join(" ");
        let laid = layout_blocks(
            &[
                Block::WidgetRow(calendar),
                Block::WidgetRow(clarity),
                para(&text),
            ],
            400.0,
            1.0,
            &fake_measure,
        );
        let first = laid.widget_rows[0].as_ref().unwrap();
        let second = laid.widget_rows[1].as_ref().unwrap();
        let paragraph = &laid.blocks[2];

        assert!(first.lane_has_content && second.lane_has_content);
        assert_eq!(paragraph.lines[0].y, WIDGET_FLOW_TOP_GAP);
        assert!(paragraph.lines.iter().any(|line| {
            line.y >= second.cards[0].rect.y
                && line.y < second.cards[0].rect.bottom()
                && line.x == first.lane.unwrap().x
        }));
    }

    #[test]
    fn a_changed_widget_distribution_ends_the_markdown_wall() {
        let first = WidgetRow::new(Widget::Empty);
        let second = WidgetRow {
            placements: vec![WidgetPlacement {
                slot: 1,
                span: 1,
                widget: Widget::Empty,
            }],
        };
        let text = std::iter::repeat_n("word", 100)
            .collect::<Vec<_>>()
            .join(" ");
        let laid = layout_blocks(
            &[
                Block::WidgetRow(first),
                para(&text),
                Block::WidgetRow(second),
            ],
            400.0,
            1.0,
            &fake_measure,
        );
        let first = laid.widget_rows[0].as_ref().unwrap();
        let second = laid.widget_rows[2].as_ref().unwrap();
        let paragraph = &laid.blocks[1];

        assert_eq!(
            paragraph.lines.iter().find(|line| line.x == 0.0).unwrap().y,
            first.height
        );
        assert!(second.cards[0].rect.y >= paragraph.y + paragraph.height + GAP_PARAGRAPH);
    }

    #[test]
    fn a_folded_heading_does_not_extend_a_widget_wall_with_hidden_rows() {
        let laid = layout_blocks(
            &[
                Block::WidgetRow(WidgetRow::new(Widget::Empty)),
                Block::Heading {
                    level: 1,
                    folded: true,
                    content: vec![Inline::Text(Text {
                        text: "Collapsed".into(),
                        style: Style::PLAIN,
                    })],
                },
                Block::WidgetRow(WidgetRow::new(Widget::Clarity(ClarityWidget::default()))),
            ],
            400.0,
            1.0,
            &fake_measure,
        );

        let first = laid.widget_rows[0].as_ref().unwrap();
        assert_eq!(first.wall.unwrap().bottom(), first.height);
        assert!(laid.widget_rows[2].is_none());
    }

    #[test]
    fn wide_blocks_resume_below_a_widget_row() {
        let row = WidgetRow::new(Widget::Empty);
        let math = Block::Math {
            list: vec![Inline::Math(vec![MathNode::Sym('x')])],
            tag: None,
        };
        let laid = layout_blocks(&[Block::WidgetRow(row), math], 400.0, 1.0, &fake_measure);
        let widgets = laid.widget_rows[0].as_ref().unwrap();
        assert!(laid.blocks[1].y >= widgets.height + GAP_PARAGRAPH);
        assert_eq!(laid.blocks[1].lines[0].x, 0.0);
        assert_eq!(laid.blocks[1].lines[0].width, 400.0);
    }

    #[test]
    fn vertical_motion_between_tables_and_widgets_keeps_the_track_caret() {
        let table = crate::document::table_row(
            2,
            true,
            std::sync::Arc::new(crate::document::table::TableSettings::new(2, 1)),
        );
        let row = Block::WidgetRow(WidgetRow::new(Widget::Empty));
        let laid = layout_blocks(&[table, row], 400.0, 1.0, &fake_measure);
        let table_caret = Caret {
            block: 0,
            inline: 0,
            offset: 0,
            style: Style::PLAIN,
        };
        let widget_caret = laid
            .line_down(table_caret, 0.0, &fake_measure)
            .expect("table moves into the widget row");
        assert_eq!((widget_caret.block, widget_caret.inline), (1, 0));
        let third_track = laid.widget_rows[1].as_ref().unwrap().tracks[2];
        let aligned_widget_caret = laid
            .line_down(
                table_caret,
                third_track.x + third_track.width * 0.5,
                &fake_measure,
            )
            .expect("table keeps its horizontal goal entering a widget row");
        assert_eq!(
            (aligned_widget_caret.block, aligned_widget_caret.inline),
            (1, 2)
        );
        let table_caret = laid
            .line_up(widget_caret, 0.0, &fake_measure)
            .expect("widget moves back into the table");
        assert_eq!((table_caret.block, table_caret.inline), (0, 0));
    }

    const BOLD: Style = Style {
        bold: true,
        ..Style::PLAIN
    };

    #[test]
    fn a_scaled_layout_is_narrower_and_shorter_and_scales_every_text_size() {
        let blocks = vec![
            para("word word word word word word"),
            Block::Heading {
                level: 1,
                folded: false,
                content: vec![Inline::Text(Text {
                    text: "Title".into(),
                    style: Style::PLAIN,
                })],
            },
        ];
        let full = layout_blocks(&blocks, 200.0, 1.0, &fake_measure);
        let half = layout_blocks(&blocks, 200.0, 0.5, &fake_measure);

        // Smaller text fits more words per line, so the same blocks come out
        // shorter at half scale.
        assert!(half.height < full.height);

        // Every text size the layout produces is scaled by the same factor.
        assert_eq!(
            text_style(&blocks[0], Style::PLAIN, 0.5).size,
            text_style(&blocks[0], Style::PLAIN, 1.0).size * 0.5
        );
        assert_eq!(
            text_style(&blocks[1], Style::PLAIN, 0.5).size,
            text_style(&blocks[1], Style::PLAIN, 1.0).size * 0.5
        );

        // A single word laid out at half scale measures half as wide — the
        // "narrower" half of the claim, read off the caret's resting x.
        let measure = |text: &str, style: &TextStyle| text.chars().count() as f32 * style.size;
        let one = vec![para("word")];
        let at_end = |layout: &DocLayout| {
            layout
                .caret_pos(
                    Caret {
                        block: 0,
                        inline: 0,
                        offset: 4,
                        style: Style::PLAIN,
                    },
                    &measure,
                )
                .0
        };
        let full_x = at_end(&layout_blocks(&one, 1000.0, 1.0, &measure));
        let half_x = at_end(&layout_blocks(&one, 1000.0, 0.5, &measure));
        assert!(half_x < full_x);
        assert!((half_x - full_x * 0.5).abs() < 0.001);
    }

    #[test]
    fn scaled_math_fits_the_sidenote_column() {
        let measure =
            |text: &str, style: &TextStyle| text.chars().count() as f32 * style.size * 0.5;
        let list: Vec<MathNode> = "abcdefghijkl".chars().map(MathNode::Sym).collect();
        let blocks = vec![Block::Paragraph(vec![Inline::Math(list.clone())])];
        let width = crate::components::sidenotes::NOTE_WIDTH;
        let scale = crate::components::sidenotes::SCALE;
        let full = math_layout::layout(&list, 0, 1.0, &measure);
        let note = layout_blocks(&blocks, width, scale, &measure);
        let small = math_layout::layout(&list, 0, scale, &measure);

        assert!(full.width > width);
        assert!(small.width <= width);
        assert_eq!(note.blocks[0].lines.len(), 1);
        assert_eq!(note.scale, scale);
    }

    #[test]
    fn layout_blocks_at_scale_1_matches_layout_for_the_same_document() {
        let d = doc_with(vec![
            Block::Heading {
                level: 1,
                folded: false,
                content: vec![Inline::Text(Text {
                    text: "Title".into(),
                    style: Style::PLAIN,
                })],
            },
            Block::Paragraph(vec![
                Inline::Text(Text {
                    text: "body ".into(),
                    style: Style::PLAIN,
                }),
                Inline::Note("1".into()),
                Inline::Text(Text {
                    text: " tail".into(),
                    style: Style::PLAIN,
                }),
            ]),
        ]);
        let whole = layout(&d, 300.0, &fake_measure);
        let by_blocks = layout_blocks(d.body(), 300.0, 1.0, &fake_measure);

        assert_eq!(whole.height, by_blocks.height);
        assert_eq!(whole.scale, by_blocks.scale);
        assert_eq!(whole.source, by_blocks.source);

        assert_eq!(whole.anchors.len(), by_blocks.anchors.len());
        for (a, b) in whole.anchors.iter().zip(&by_blocks.anchors) {
            assert_eq!(a.block, b.block);
            assert_eq!(a.inline, b.inline);
            assert_eq!(a.label, b.label);
            assert_eq!(a.number, b.number);
            assert_eq!(a.y, b.y);
        }

        assert_eq!(whole.blocks.len(), by_blocks.blocks.len());
        for (a, b) in whole.blocks.iter().zip(&by_blocks.blocks) {
            assert_eq!(a.y, b.y);
            assert_eq!(a.height, b.height);
            assert_eq!(a.lines.len(), b.lines.len());
            for (al, bl) in a.lines.iter().zip(&b.lines) {
                assert_eq!(al.y, bl.y);
                assert_eq!(al.height, bl.height);
                assert_eq!(al.segments.len(), bl.segments.len());
                for (as_, bs) in al.segments.iter().zip(&bl.segments) {
                    assert_eq!(as_.inline, bs.inline);
                    assert_eq!(as_.start, bs.start);
                    assert_eq!(as_.len, bs.len);
                    assert_eq!(as_.style, bs.style);
                    assert_eq!(as_.number, bs.number);
                }
            }
        }
    }

    #[test]
    fn layout_and_outline_are_unchanged_by_which_scope_is_focused() {
        let mut d = doc_with(vec![
            Block::Heading {
                level: 1,
                folded: false,
                content: vec![Inline::Text(Text {
                    text: "Title".into(),
                    style: Style::PLAIN,
                })],
            },
            Block::Paragraph(vec![
                Inline::Text(Text {
                    text: "body ".into(),
                    style: Style::PLAIN,
                }),
                Inline::Note("1".into()),
                Inline::Text(Text {
                    text: " tail".into(),
                    style: Style::PLAIN,
                }),
            ]),
        ]);
        d.notes.push(Sidenote {
            label: "1".into(),
            body: vec![para("note text")],
            anchored: true,
        });

        let laid = layout(&d, 300.0, &fake_measure);
        let nodes = crate::document::outline::outline(d.body());

        d.focus = Focus::Note(0);

        // Both describe the file, not the caret, so focusing a note must not
        // move a single block or heading.
        assert_eq!(layout(&d, 300.0, &fake_measure).source, laid.source);
        assert_eq!(crate::document::outline::outline(d.body()), nodes);
    }

    #[test]
    fn greedy_wrap_breaks_exactly_and_reecovers_source() {
        let d = doc_with(vec![para("aaa bbb ccc ddd eee")]);
        let laid = layout(&d, 140.0, &fake_measure);
        // "aaa bbb" = 70, + "ccc" = 110 <= 140, + " ddd" = 150 > 140.
        // "aaa bbb" = 70, + "ccc" = 110 <= 140, + " ddd" = 150 > 140.
        // The space before "ddd" belongs to line 0 ("aaa bbb ccc ").
        // Line 1: "ddd eee".
        assert_eq!(laid.blocks[0].lines.len(), 2);

        let block = &d.body()[0];
        let all_text: String = laid.blocks[0]
            .lines
            .iter()
            .flat_map(|l| l.segments.iter())
            .map(|s| segment_text(&block.inlines()[s.inline], s))
            .collect();
        assert_eq!(all_text, "aaa bbb ccc ddd eee");
    }

    #[test]
    fn overlong_word_gets_own_line() {
        let d = doc_with(vec![para("short supercalifragilistic")]);
        let laid = layout(&d, 100.0, &fake_measure);
        // " supercalifragilistic" = 22 chars > 100, so it wraps onto its own
        // unbroken line.
        assert_eq!(laid.blocks[0].lines.len(), 2);
        let last = laid.blocks[0].lines.last().unwrap();
        let block = &d.body()[0];
        assert_eq!(
            segment_text(&block.inlines()[last.segments[0].inline], &last.segments[0]),
            "supercalifragilistic"
        );
        assert_eq!(
            last.segments.len(),
            1,
            "the word is not broken across lines"
        );
    }

    #[test]
    fn style_segmentation_survives_wrap() {
        // run0 "aaa " plain, run1 "bbb" bold, run2 " ccc" plain.
        // Width 100: "aaa " = 40; + "bbb" = 70; + " " + "ccc" = 110 > 100,
        // so the space splits with the break and the line is "aaa bbb ".
        let d = doc_with(vec![Block::Paragraph(vec![
            Inline::Text(Text {
                text: "aaa ".into(),
                style: Style::PLAIN,
            }),
            Inline::Text(Text {
                text: "bbb".into(),
                style: BOLD,
            }),
            Inline::Text(Text {
                text: " ccc".into(),
                style: Style::PLAIN,
            }),
        ])]);
        let laid = layout(&d, 100.0, &fake_measure);
        assert_eq!(laid.blocks[0].lines.len(), 2);

        let block = &d.body()[0];
        let line0 = &laid.blocks[0].lines[0];
        // "aaa " from run 0, "bbb" from run 1 (bold), then the space from
        // run 2 that hangs at the line's right edge (invisible, clipped).
        assert_eq!(
            line0
                .segments
                .iter()
                .map(|s| (s.inline, s.start, s.len, s.style))
                .collect::<Vec<_>>(),
            vec![
                (0, 0, 4, Style::PLAIN),
                (1, 0, 3, BOLD),
                (2, 0, 1, Style::PLAIN),
            ]
        );

        let line1 = &laid.blocks[0].lines[1];
        assert_eq!(
            line1
                .segments
                .iter()
                .map(|s| (s.inline, s.start, s.len))
                .collect::<Vec<_>>(),
            vec![(2, 1, 3)]
        );

        // And the whole block's text is re-covered exactly.
        let all: String = laid.blocks[0]
            .lines
            .iter()
            .flat_map(|l| l.segments.iter())
            .map(|s| segment_text(&block.inlines()[s.inline], s))
            .collect();
        assert_eq!(all, "aaa bbb ccc");
    }

    #[test]
    fn caret_at_wrap_boundary_lands_on_next_line_at_x0() {
        let mut d = doc_with(vec![para("aaa bbb ccc ddd")]);
        let laid = layout(&d, 100.0, &fake_measure);
        // Line 0 holds "aaa bbb " — the space before "ccc" is the trailing
        // space that belongs to the wrapped line. Flat 8 is the boundary:
        // right after that space, the caret sits on line 1 at x = 0.
        assert_eq!(
            flat_of_caret(
                d.body(),
                Caret {
                    block: 0,
                    inline: 0,
                    offset: 8,
                    style: Style::PLAIN
                }
            ),
            8
        );
        d.caret = Caret {
            block: 0,
            inline: 0,
            offset: 8,
            style: Style::PLAIN,
        };
        let (x, _, _) = laid.caret_pos(d.caret, &fake_measure);
        assert_eq!(x, 0.0, "the caret past a wrap break starts the next line");
    }

    #[test]
    fn caret_at_block_end_at_last_line_end() {
        let mut d = doc_with(vec![para("abcdef")]);
        let laid = layout(&d, 300.0, &fake_measure);
        d.caret = Caret {
            block: 0,
            inline: 0,
            offset: 6,
            style: Style::PLAIN,
        };
        let (x, _, _) = laid.caret_pos(d.caret, &fake_measure);
        assert_eq!(x, 60.0);
    }

    #[test]
    fn hit_round_trips_caret() {
        let d = doc_with(vec![para("aaa bbb ccc ddd")]);
        let laid = layout(&d, 100.0, &fake_measure);
        // Flat offsets 0..14 over two wrapped lines, including the boundary.
        for offset in 0..15 {
            let (x, y, _) = laid.caret_pos(
                Caret {
                    block: 0,
                    inline: 0,
                    offset,
                    style: Style::PLAIN,
                },
                &fake_measure,
            );
            let hit = laid.hit(x, y, &fake_measure);
            let hit_flat = flat_of_caret(d.body(), hit);
            assert_eq!(hit_flat, offset, "flat {offset} -> x {x}");
        }
    }

    #[test]
    fn line_down_keeps_goal_x() {
        let d = doc_with(vec![para("aaa bbb ccc ddd eee")]);
        let laid = layout(&d, 140.0, &fake_measure);
        // Line 0: "aaa bbb ccc " (110 + trailing space), line 1: "ddd eee".
        assert_eq!(laid.blocks[0].lines.len(), 2);

        // Caret after "aa" (x = 20). Moving down should land near x = 20 on
        // the second line ("dd d" ...), not at the line start or a full
        // remeasure from the left.
        let caret = Caret {
            block: 0,
            inline: 0,
            offset: 3,
            style: Style::PLAIN,
        };
        let (x, y, _) = laid.caret_pos(caret, &fake_measure);
        assert_eq!(x, 30.0);
        let down = laid.line_down(caret, x, &fake_measure).unwrap();
        assert_eq!(down.block, 0);
        assert!(down.offset > 0, "goal x is inside the second line");
        let (dx, dy, _) = laid.caret_pos(down, &fake_measure);
        assert!(dy > y, "moved to a lower line");
        assert!((dx - x).abs() < 10.0, "kept the goal x: {} vs {}", dx, x);
    }

    #[test]
    fn empty_document_is_one_empty_line() {
        let d = Document::new(std::path::Path::new("notes/t.md"));
        let laid = layout(&d, 200.0, &fake_measure);
        assert_eq!(laid.blocks.len(), 1);
        assert_eq!(laid.blocks[0].lines.len(), 1);
        assert!(laid.blocks[0].lines[0].segments.is_empty());

        let (x, y, h) = laid.caret_pos(d.caret, &fake_measure);
        assert_eq!(x, 0.0);
        assert_eq!(h, LINE_BODY);
        assert_eq!(y, LINE_BODY / 2.0);

        let hit = laid.hit(40.0, 5.0, &fake_measure);
        assert_eq!(hit.block, 0);
        assert_eq!(hit.offset, 0);
        assert_eq!(hit.style, Style::PLAIN);
    }

    // ---- list item tests -------------------------------------------------

    fn list_item(marker: ListMarker, t: &str) -> Block {
        Block::ListItem {
            marker,
            content: vec![Inline::Text(Text {
                text: t.into(),
                style: Style::PLAIN,
            })],
        }
    }

    /// The hanging indent: every line of a list item's content starts at the
    /// same left edge, so a wrapped line never slides back under the marker.
    #[test]
    fn a_list_item_wraps_with_a_true_hanging_indent() {
        let d = doc_with(vec![list_item(
            ListMarker::Bullet,
            "aaaa bbbb cccc dddd eeee ffff gggg hhhh",
        )]);
        // Narrow enough to force at least one wrap at 10px/char.
        let layout = layout(&d, 220.0, &fake_measure);
        let block = &layout.blocks[0];
        assert!(
            block.lines.len() >= 2,
            "expected a wrap: {}",
            block.lines.len()
        );
        for line in &block.lines {
            assert_eq!(
                line.x, LIST_INDENT,
                "every line hangs at the content column"
            );
        }
        // The first wrapped line continues exactly where the flat text says:
        // a click at the content column lands on its first char.
        let second = line_flat_start(block, 1);
        let caret = layout.hit(LIST_INDENT, block.lines[1].y + 1.0, &fake_measure);
        assert_eq!(flat_of_caret(&layout.source, caret), second);
        // The caret there sits at the content column, not at the gutter.
        let (x, _, _) = layout.caret_pos(caret, &fake_measure);
        assert_eq!(x, LIST_INDENT);
    }

    #[test]
    fn non_list_lines_start_at_the_column_edge() {
        let d = doc_with(vec![para("prose"), list_item(ListMarker::Bullet, "item")]);
        let layout = layout(&d, 400.0, &fake_measure);
        assert_eq!(layout.blocks[0].lines[0].x, 0.0);
        assert_eq!(layout.blocks[1].lines[0].x, LIST_INDENT);
    }

    /// The checkbox is a control: one geometry answers both the drawing and
    /// the click, sitting in the gutter left of the content column.
    #[test]
    fn a_task_checkbox_is_clickable_where_it_is_drawn() {
        let d = doc_with(vec![
            list_item(ListMarker::Task { done: false }, "buy milk"),
            list_item(ListMarker::Bullet, "tea"),
            list_item(ListMarker::Task { done: true }, "done thing"),
        ]);
        let layout = layout(&d, 400.0, &fake_measure);

        let (x, y, w, h) = layout.task_box(0).expect("task item has a box");
        assert_eq!(w, CHECK_SIZE);
        assert_eq!(
            x + w,
            LIST_INDENT - CHECK_GAP,
            "right edge clears the column"
        );
        let mid = layout.blocks[0].lines[0].y + layout.blocks[0].lines[0].height * 0.5;
        assert_eq!(y + h * 0.5, mid, "centred on the line");

        assert_eq!(
            layout.task_at(x + w * 0.5, y + h * 0.5),
            Some(0),
            "centre of the box toggles item 0"
        );
        assert_eq!(layout.task_at(1.0, 1.0), None, "outside the box");
        // A bullet is not a control, wherever the click lands.
        let bullet_mid = layout.blocks[1].lines[0].y + layout.blocks[1].lines[0].height * 0.5;
        assert_eq!(layout.task_at(2.0, bullet_mid), None);
        assert!(layout.task_box(2).is_some(), "the done item keeps its box");
    }

    #[test]
    fn list_items_sit_tighter_than_paragraphs() {
        let gap = |blocks: Vec<Block>| {
            let d = doc_with(blocks);
            let layout = layout(&d, 400.0, &fake_measure);
            layout.blocks[1].y - (layout.blocks[0].y + layout.blocks[0].height)
        };
        let items = gap(vec![
            list_item(ListMarker::Bullet, "a"),
            list_item(ListMarker::Bullet, "b"),
        ]);
        let paragraphs = gap(vec![para("a"), para("b")]);
        assert_eq!(items, LIST_GAP);
        assert_eq!(paragraphs, GAP_PARAGRAPH);
        assert!(items < paragraphs);
        // The last item of a run keeps the full paragraph gap below it.
        let d = doc_with(vec![
            list_item(ListMarker::Bullet, "a"),
            list_item(ListMarker::Bullet, "b"),
            para("after"),
        ]);
        let layout = layout(&d, 400.0, &fake_measure);
        let after_items = layout.blocks[2].y - (layout.blocks[1].y + layout.blocks[1].height);
        assert_eq!(after_items, GAP_PARAGRAPH);
    }

    #[test]
    fn a_done_task_reads_dim_through_text_style() {
        let done = list_item(ListMarker::Task { done: true }, "x");
        let open = list_item(ListMarker::Task { done: false }, "x");
        let dim = text_style(&done, Style::PLAIN, 1.0);
        let plain = text_style(&open, Style::PLAIN, 1.0);
        assert_ne!(dim.color, plain.color, "done tasks are dimmed");
        assert_eq!(dim.size, plain.size, "same size — quiet, not smaller");
    }

    #[test]
    fn a_rule_costs_less_vertical_space_than_a_blank_line() {
        let d = doc_with(vec![
            para("a"),
            Block::Divider(vec![Inline::Text(Text {
                text: String::new(),
                style: Style::PLAIN,
            })]),
        ]);
        let laid = layout(&d, 200.0, &fake_measure);
        assert_eq!(laid.blocks[1].height, LINE_DIVIDER);
        assert_eq!(laid.blocks[1].lines.len(), 1);
        assert!(laid.blocks[1].lines[0].segments.is_empty());

        let blank = doc_with(vec![para("a"), para("")]);
        assert!(laid.height < layout(&blank, 200.0, &fake_measure).height);
    }

    #[test]
    fn headings_use_bold_heading_style_and_their_gaps() {
        let d = doc_with(vec![
            Block::Heading {
                level: 1,
                folded: false,
                content: vec![Inline::Text(Text {
                    text: "Title".into(),
                    style: Style::PLAIN,
                })],
            },
            para("body text"),
        ]);
        // Assert the measure sees the right TextStyle for the heading.
        let seen: std::cell::Cell<Option<f32>> = std::cell::Cell::new(None);
        let check = |text: &str, style: &TextStyle| {
            if text == "Title" {
                seen.set(Some(style.size));
            }
            fake_measure(text, style)
        };
        layout(&d, 300.0, &check);
        assert_eq!(seen.get(), Some(30.0), "H1 text is measured at size 30");
        // The heading weight should be bold.
        let h1_style = text_style(&d.body()[0], Style::PLAIN, 1.0);
        assert!(h1_style.weight > 0.0, "headings are always bold");

        // H4 matches body size (17.5) — bold is what distinguishes it.
        let h4 = Block::Heading {
            level: 4,
            folded: false,
            content: vec![Inline::Text(Text {
                text: "Sub".into(),
                style: Style::PLAIN,
            })],
        };
        let h4_style = text_style(&h4, Style::PLAIN, 1.0);
        assert_eq!(
            h4_style.size, 17.5,
            "H4 is body-sized, distinguished by bold"
        );

        let laid = layout(&d, 300.0, &fake_measure);
        assert_eq!(laid.blocks[0].y, 0.0, "first block has no gap above");
        assert_eq!(laid.blocks[0].lines[0].height, LINE_H1);
        assert!(
            laid.blocks[1].y >= laid.blocks[0].height,
            "gap above a body block"
        );
    }

    #[test]
    fn code_style_renders_monospace() {
        let code = Style {
            code: true,
            ..Style::PLAIN
        };
        let para = Block::Paragraph(vec![]);
        let code_block = Block::CodeLine {
            content: vec![],
            first: true,
            lang: None,
        };
        let ts_para = text_style(&para, code, 1.0);
        let ts_code = text_style(&code_block, code, 1.0);
        assert_eq!(ts_para.font, theme::mono());
        assert_eq!(ts_code.font, theme::mono());
        assert_eq!(ts_para.size, 17.5);
        assert_eq!(ts_code.size, 17.5);
        // weight and slant stay zero — no faux-bold/italic for code
        assert_eq!(ts_para.weight, 0.0);
        assert_eq!(ts_code.slant, 0.0);
    }

    #[test]
    fn a_badge_flows_as_its_box_and_the_caret_agrees() {
        let badge = Style {
            badge: true,
            ..Style::PLAIN
        };
        let block = Block::Paragraph(vec![
            Inline::Text(Text {
                text: "PS".into(),
                style: badge,
            }),
            Inline::Math(vec![MathNode::Sym('x')]),
        ]);
        // The box counts: bare glyphs would put `x` under the chip.
        let bare = fake_measure("PS", &text_style(&block, badge, 1.0));
        assert_eq!(
            advance(
                &block.inlines()[0],
                "PS",
                &block,
                badge,
                None,
                1.0,
                &fake_measure
            ),
            bare + theme::BADGE_PAD * 2.0
        );
        assert_eq!(
            advance(
                &block.inlines()[0],
                "PS",
                &block,
                Style::PLAIN,
                None,
                1.0,
                &fake_measure,
            ),
            bare,
            "only a badge pays for a box"
        );

        // And the caret walks the same distance the drawing does: past the
        // whole chip at its end, inside the left edge at its start.
        let doc = doc_with(vec![block]);
        let laid = layout(&doc, 1000.0, &fake_measure);
        let at = |inline, offset| {
            let caret = Caret {
                block: 0,
                inline,
                offset,
                style: Style::PLAIN,
            };
            laid.caret_pos(caret, &fake_measure).0
        };
        // Inside the box at the label's start, not on its left edge — the
        // caret marks where the next glyph goes, and that is past the pad.
        assert_eq!(at(0, 0), theme::BADGE_PAD);
        assert_eq!(at(0, 1), theme::BADGE_PAD + 10.0, "inside the label");
        assert_eq!(at(1, 0), bare + theme::BADGE_PAD * 2.0, "past the box");

        let line = &laid.blocks[0].lines[0];
        let hit = laid
            .hit_math(
                bare + theme::BADGE_PAD * 2.0 + 5.0,
                line.y + line.height * 0.5,
                &fake_measure,
            )
            .expect("math immediately after a badge remains hittable");
        assert_eq!((hit.0, hit.1), (0, 1));
    }

    #[test]
    fn note_anchors_are_reported_in_document_order() {
        let d = doc_with(vec![
            Block::Paragraph(vec![
                Inline::Text(Text {
                    text: "a".into(),
                    style: Style::PLAIN,
                }),
                Inline::Note("second".into()),
                Inline::Text(Text {
                    text: " b".into(),
                    style: Style::PLAIN,
                }),
            ]),
            Block::Paragraph(vec![
                Inline::Note("first".into()),
                Inline::Text(Text {
                    text: " text".into(),
                    style: Style::PLAIN,
                }),
            ]),
        ]);
        let laid = layout(&d, 1000.0, &fake_measure);
        // Labels follow document order, not the author's own label ordering —
        // the anchor named "second" appears in the prose before "first".
        assert_eq!(
            laid.note_anchors(),
            vec![
                ("second".to_string(), laid.blocks[0].lines[0].y),
                ("first".to_string(), laid.blocks[1].lines[0].y),
            ]
        );
    }

    #[test]
    fn an_anchor_advances_the_line_by_its_own_width() {
        let block = Block::Paragraph(vec![
            Inline::Note("a".into()),
            Inline::Text(Text {
                text: "word".into(),
                style: Style::PLAIN,
            }),
        ]);
        let d = doc_with(vec![block]);
        let laid = layout(&d, 1000.0, &fake_measure);
        let number = laid.anchors[0].number.clone();
        let anchor_width = advance(
            &d.body()[0].inlines()[0],
            "\u{FFFC}",
            &d.body()[0],
            Style::PLAIN,
            Some(&number),
            1.0,
            &fake_measure,
        );
        // The anchor measures as its raised number, and the text after it
        // starts at that width rather than on top of the number.
        assert_eq!(anchor_width, fake_measure(&number, &anchor_style()));
        let (x, _, _) = laid.caret_pos(
            Caret {
                block: 0,
                inline: 1,
                offset: 0,
                style: Style::PLAIN,
            },
            &fake_measure,
        );
        assert_eq!(x, anchor_width);
    }

    #[test]
    fn an_anchor_reserves_the_width_of_its_own_number() {
        // Eleven anchors: the eleventh draws "11", two digits wide, and must
        // reserve both — never the one-digit box the first anchor got.
        let d = doc_with(
            (0..11)
                .map(|i| Block::Paragraph(vec![Inline::Note(format!("note {i}"))]))
                .collect(),
        );
        let laid = layout(&d, 1000.0, &fake_measure);
        assert_eq!(laid.anchors.len(), 11);
        for anchor in &laid.anchors {
            let run = &d.body()[anchor.block].inlines()[anchor.inline];
            let width = advance(
                run,
                "\u{FFFC}",
                &d.body()[anchor.block],
                Style::PLAIN,
                Some(&anchor.number),
                1.0,
                &fake_measure,
            );
            assert_eq!(
                width,
                fake_measure(&anchor.number, &anchor_style()),
                "anchor {} reserves the width of the number it draws",
                anchor.number
            );
        }
        let first = advance(
            &d.body()[0].inlines()[0],
            "\u{FFFC}",
            &d.body()[0],
            Style::PLAIN,
            Some(&laid.anchors[0].number),
            1.0,
            &fake_measure,
        );
        let eleventh = advance(
            &d.body()[10].inlines()[0],
            "\u{FFFC}",
            &d.body()[10],
            Style::PLAIN,
            Some(&laid.anchors[10].number),
            1.0,
            &fake_measure,
        );
        assert!(eleventh > first);
    }

    #[test]
    fn a_single_digit_anchor_is_not_padded_to_two() {
        let d = doc_with(vec![Block::Paragraph(vec![Inline::Note("a".into())])]);
        let laid = layout(&d, 1000.0, &fake_measure);
        let number = laid.anchors[0].number.clone();
        let width = advance(
            &d.body()[0].inlines()[0],
            "\u{FFFC}",
            &d.body()[0],
            Style::PLAIN,
            Some(&number),
            1.0,
            &fake_measure,
        );
        assert_eq!(number, "1");
        // One digit wide — no reserve for a second digit that is not there.
        assert_eq!(width, fake_measure("1", &anchor_style()));
        assert_ne!(width, fake_measure("11", &anchor_style()));
    }

    fn code_line_run(text: &str, first: bool) -> Block {
        Block::CodeLine {
            content: vec![Inline::Text(Text {
                text: text.into(),
                style: Style {
                    code: true,
                    ..Style::PLAIN
                },
            })],
            first,
            lang: None,
        }
    }

    #[test]
    fn code_block_gap_packing() {
        let d = doc_with(vec![
            code_line_run("line1", true),
            code_line_run("line2", false), // continuation: zero gap
            para("after"),
        ]);
        let laid = layout(&d, 300.0, &fake_measure);
        assert!(laid.blocks.len() >= 3);
        // Adjacent CodeLine blocks in the same group pack tight.
        assert_eq!(
            laid.blocks[1].y,
            laid.blocks[0].y + laid.blocks[0].height,
            "continuation CodeLine blocks must have zero gap"
        );
        // CodeLine followed by Paragraph gets normal gap.
        let gap = laid.blocks[2].y - (laid.blocks[1].y + laid.blocks[1].height);
        assert!(
            (gap - GAP_PARAGRAPH).abs() < f32::EPSILON,
            "CodeLine→Paragraph gap should be GAP_PARAGRAPH, got {gap}"
        );
    }

    #[test]
    fn code_block_separate_groups_get_normal_gap() {
        // Two separate fence groups (second block first:true) get GAP_PARAGRAPH.
        let d = doc_with(vec![
            code_line_run("line1", true),
            code_line_run("line2", true), // separate group: normal gap
        ]);
        let laid = layout(&d, 300.0, &fake_measure);
        assert!(laid.blocks.len() >= 2);
        let gap = laid.blocks[1].y - (laid.blocks[0].y + laid.blocks[0].height);
        assert!(
            (gap - GAP_PARAGRAPH).abs() < f32::EPSILON,
            "separate CodeLine groups should have GAP_PARAGRAPH, got {gap}"
        );
    }

    #[test]
    fn a_line_with_a_tall_atom_grows_and_pushes_the_next_line_down() {
        let fraction = Inline::Math(vec![MathNode::Frac {
            num: vec![MathNode::Sym('1')],
            den: vec![MathNode::Sym('2')],
        }]);
        let d = doc_with(vec![Block::Paragraph(vec![
            fraction,
            Inline::Text(Text {
                text: " next".into(),
                style: Style::PLAIN,
            }),
        ])]);
        let laid = layout(&d, 30.0, &fake_measure);
        let block = &laid.blocks[0];
        assert_eq!(block.lines.len(), 2);
        assert!(block.lines[0].height > LINE_BODY);
        assert_eq!(block.lines[1].y, block.lines[0].height);
        assert_eq!(block.height, block.lines[0].height + block.lines[1].height);
    }

    #[test]
    fn a_click_past_an_expression_lands_after_it() {
        let block = Block::Paragraph(vec![
            Inline::Math(vec![MathNode::Frac {
                num: vec![MathNode::Sym('1')],
                den: vec![MathNode::Sym('2')],
            }]),
            Inline::Text(Text {
                text: " text".into(),
                style: Style::PLAIN,
            }),
        ]);
        let d = doc_with(vec![block]);
        let laid = layout(&d, 300.0, &fake_measure);
        let atom_width = advance(
            &d.body()[0].inlines()[0],
            "\u{FFFC}",
            &d.body()[0],
            Style::PLAIN,
            None,
            1.0,
            &fake_measure,
        );

        let past = laid.hit(atom_width + 1.0, LINE_BODY / 2.0, &fake_measure);
        assert_eq!(flat_of_caret(d.body(), past), 1);

        let inside_left = laid.hit(atom_width / 4.0, LINE_BODY / 2.0, &fake_measure);
        assert_eq!(flat_of_caret(d.body(), inside_left), 0);
    }

    #[test]
    fn a_click_inside_an_expression_finds_its_atom() {
        let block = Block::Paragraph(vec![
            Inline::Text(Text {
                text: "before ".into(),
                style: Style::PLAIN,
            }),
            Inline::Math(vec![MathNode::Frac {
                num: vec![MathNode::Sym('1')],
                den: vec![MathNode::Sym('2')],
            }]),
            Inline::Text(Text {
                text: " after".into(),
                style: Style::PLAIN,
            }),
        ]);
        let d = doc_with(vec![block]);
        let laid = layout(&d, 300.0, &fake_measure);
        let atom_width = advance(
            &d.body()[0].inlines()[1],
            "\u{FFFC}",
            &d.body()[0],
            Style::PLAIN,
            None,
            1.0,
            &fake_measure,
        );
        let line = &laid.blocks[0].lines[0];
        let inside = laid.hit_math(
            70.0 + atom_width / 2.0,
            line.y + line.height / 2.0,
            &fake_measure,
        );
        assert!(matches!(inside, Some((0, 1, _, _))));

        let past = laid.hit_math(
            70.0 + atom_width + 20.0,
            line.y + line.height / 2.0,
            &fake_measure,
        );
        assert_eq!(past, None);
    }

    #[test]
    fn a_click_descends_to_the_requested_fraction_position() {
        let list = vec![MathNode::Frac {
            num: vec![MathNode::Sym('n')],
            den: vec![MathNode::Sym('x'), MathNode::Sym('y'), MathNode::Sym('z')],
        }];
        let d = doc_with(vec![Block::Paragraph(vec![Inline::Math(list.clone())])]);
        let laid = layout(&d, 300.0, &fake_measure);
        let expression = math_layout::layout(&list, 0, 1.0, &fake_measure);
        let BoxKind::Row { children } = &expression.kind else {
            panic!("expression must be a row");
        };
        let BoxKind::Row { children: fraction } = &children[0].2.kind else {
            panic!("fraction must be a row");
        };
        let denominator = &fraction[2];
        let x = children[0].0 + denominator.0 + denominator.2.width * 0.95;
        let baseline = laid.blocks[0].lines[0].height * 0.5;
        let y = baseline - children[0].1 - denominator.1;

        let (_, _, _, cursor) = laid
            .hit_math(x, y, &fake_measure)
            .expect("denominator must be interactive");
        assert_eq!(
            cursor.path,
            vec![Step {
                index: 0,
                slot: Slot::Den,
            }]
        );
        assert_eq!(cursor.index, 3);
    }

    #[test]
    fn normal_context_click_selects_words_and_leaves_whitespace_empty() {
        let d = doc_with(vec![para("hello world")]);
        let laid = layout(&d, 300.0, &fake_measure);
        let y = laid.blocks[0].lines[0].height * 0.5;
        assert_eq!(
            laid.hit_context(5.0, y, &fake_measure),
            Some(ContextHit::Range {
                range: FlatRange::new(
                    FlatPos {
                        block: 0,
                        offset: 0
                    },
                    FlatPos {
                        block: 0,
                        offset: 5
                    }
                ),
                kind: RangeKind::Word,
            })
        );
        assert_eq!(laid.hit_context(55.0, y, &fake_measure), None);
    }

    #[test]
    fn binary_search_finds_the_same_character_as_a_linear_walk_over_shaped_text_at_every_x() {
        let text = "a".repeat(80);
        let style = text_style(&Block::Paragraph(vec![]), Style::PLAIN, 1.0);
        let width = shaped_measure(&text, &style);

        for x in 0..=width.ceil() as usize {
            let x = x as f32;
            assert_eq!(
                caret_char_for_x(&text, 0.0, x, &style, &shaped_measure),
                linear_caret_char(&text, x, &style, &shaped_measure),
                "caret mismatch at x={x}"
            );
            assert_eq!(
                context_char_for_x(&text, 0.0, x, &style, &shaped_measure),
                linear_context_char(&text, x, &style, &shaped_measure),
                "context mismatch at x={x}"
            );
        }

        let measures = Cell::new(0);
        let counting_measure = |text: &str, style: &TextStyle| {
            measures.set(measures.get() + 1);
            shaped_measure(text, style)
        };
        let mut maximum = 0;
        for x in 0..=width.ceil() as usize {
            measures.set(0);
            let _ = caret_char_for_x(&text, 0.0, x as f32, &style, &counting_measure);
            maximum = maximum.max(measures.get());
        }
        assert_eq!(maximum, 8);
    }

    #[test]
    fn a_word_italicised_un_italicised_and_italicised_again_through_same_click_position_ends_up_italic()
     {
        let mut d = doc_with(vec![para("left target right")]);
        let italic = Style {
            italic: true,
            ..Style::PLAIN
        };
        let y = LINE_BODY / 2.0;
        let x = 43.0;

        for expected in [true, false, true] {
            let laid = layout(&d, 300.0, &shaped_measure);
            let ContextHit::Range { range, kind } = laid
                .hit_context(x, y, &shaped_measure)
                .expect("click on target must find a word")
            else {
                panic!("click on target must find a prose range")
            };
            assert_eq!(kind, RangeKind::Word);
            assert_eq!(
                range,
                FlatRange::new(
                    FlatPos {
                        block: 0,
                        offset: 5
                    },
                    FlatPos {
                        block: 0,
                        offset: 11
                    }
                )
            );
            d.toggle_style_range(range, italic);
            assert_eq!(
                style_at(d.body(), 0, 6)
                    .expect("target must remain present")
                    .italic,
                expected
            );
        }
    }

    #[test]
    fn a_click_at_the_x_of_a_word_returns_exactly_that_word_with_shaped_text() {
        let d = doc_with(vec![para("left target right")]);
        let y = LINE_BODY / 2.0;
        assert_eq!(d.body()[0].inlines()[0].text(), "left target right");
        assert_eq!(
            layout(&d, 300.0, &shaped_measure).hit_context(x_of_target(), y, &shaped_measure),
            Some(ContextHit::Range {
                range: FlatRange::new(
                    FlatPos {
                        block: 0,
                        offset: 5,
                    },
                    FlatPos {
                        block: 0,
                        offset: 11,
                    },
                ),
                kind: RangeKind::Word,
            })
        );

        fn x_of_target() -> f32 {
            shaped_measure(
                "left ",
                &text_style(&Block::Paragraph(vec![]), Style::PLAIN, 1.0),
            ) + 1.0
        }
    }

    #[test]
    fn normal_context_click_prefers_a_whole_badge_run() {
        let badge = Style {
            badge: true,
            ..Style::PLAIN
        };
        let d = doc_with(vec![Block::Paragraph(vec![
            Inline::Text(Text {
                text: "TAG".into(),
                style: badge,
            }),
            Inline::Text(Text {
                text: " word".into(),
                style: Style::PLAIN,
            }),
        ])]);
        let laid = layout(&d, 300.0, &fake_measure);
        let y = laid.blocks[0].lines[0].height * 0.5;
        assert_eq!(
            laid.hit_context(theme::BADGE_PAD * 0.5, y, &fake_measure),
            Some(ContextHit::Range {
                range: FlatRange::new(
                    FlatPos {
                        block: 0,
                        offset: 0
                    },
                    FlatPos {
                        block: 0,
                        offset: 3
                    }
                ),
                kind: RangeKind::Badge,
            })
        );
    }

    #[test]
    fn normal_context_click_prefers_a_deep_math_symbol() {
        let list = vec![MathNode::Frac {
            num: vec![MathNode::Sym('n')],
            den: vec![MathNode::Sym('d')],
        }];
        let d = doc_with(vec![Block::Paragraph(vec![Inline::Math(list.clone())])]);
        let laid = layout(&d, 300.0, &fake_measure);
        let expression = math_layout::layout(&list, 0, 1.0, &fake_measure);
        let BoxKind::Row { children } = &expression.kind else {
            panic!("expression must be a row");
        };
        let BoxKind::Row { children: fraction } = &children[0].2.kind else {
            panic!("fraction must be a row");
        };
        let numerator = &fraction[0];
        let x = children[0].0 + numerator.0 + numerator.2.width * 0.5;
        let baseline = laid.blocks[0].lines[0].height * 0.5;
        let y = baseline - children[0].1 - numerator.1;
        assert_eq!(
            laid.hit_context(x, y, &fake_measure),
            Some(ContextHit::Math {
                block: 0,
                inline: 0,
                offset: 0,
                node: Some(NodeAddress {
                    path: vec![Step {
                        index: 0,
                        slot: Slot::Num,
                    }],
                    index: 0,
                }),
            })
        );
    }

    #[test]
    fn circular_context_hit_includes_a_tangent_corner() {
        let d = doc_with(vec![para("a")]);
        let laid = layout(&d, 300.0, &fake_measure);
        let expected = ContextHit::Range {
            range: FlatRange::new(
                FlatPos {
                    block: 0,
                    offset: 0,
                },
                FlatPos {
                    block: 0,
                    offset: 1,
                },
            ),
            kind: RangeKind::Word,
        };

        assert_eq!(
            laid.hit_contexts_in_circle(-3.0, -4.0, 5.0, 300.0, &fake_measure),
            vec![expected]
        );
        assert!(
            laid.hit_contexts_in_circle(-3.0, -4.0, 4.99, 300.0, &fake_measure)
                .is_empty()
        );
    }

    #[test]
    fn circular_context_hit_mixes_targets_and_deduplicates_runs() {
        let badge = Style {
            badge: true,
            ..Style::PLAIN
        };
        let d = doc_with(vec![Block::Paragraph(vec![
            Inline::Text(Text {
                text: "A B".into(),
                style: badge,
            }),
            Inline::Text(Text {
                text: " word ".into(),
                style: Style::PLAIN,
            }),
            Inline::Math(Vec::new()),
        ])]);
        let laid = layout(&d, 35.0, &fake_measure);
        let hits = laid.hit_contexts_in_circle(0.0, 0.0, 1_000.0, 35.0, &fake_measure);

        assert_eq!(hits.len(), 3);
        assert!(hits.contains(&ContextHit::Range {
            range: FlatRange::new(
                FlatPos {
                    block: 0,
                    offset: 0,
                },
                FlatPos {
                    block: 0,
                    offset: 3,
                },
            ),
            kind: RangeKind::Badge,
        }));
        assert!(hits.contains(&ContextHit::Range {
            range: FlatRange::new(
                FlatPos {
                    block: 0,
                    offset: 4,
                },
                FlatPos {
                    block: 0,
                    offset: 8,
                },
            ),
            kind: RangeKind::Word,
        }));
        assert!(hits.contains(&ContextHit::Math {
            block: 0,
            inline: 2,
            offset: 9,
            node: None,
        }));
    }

    #[test]
    fn circular_context_hit_uses_the_full_code_block_width() {
        let d = doc_with(vec![code_line_run("x", true)]);
        let laid = layout(&d, 300.0, &fake_measure);
        let y = laid.blocks[0].lines[0].height * 0.5;

        assert_eq!(
            laid.hit_contexts_in_circle(299.0, y, 1.0, 300.0, &fake_measure),
            vec![ContextHit::Range {
                range: FlatRange::new(
                    FlatPos {
                        block: 0,
                        offset: 0,
                    },
                    FlatPos {
                        block: 0,
                        offset: 1,
                    },
                ),
                kind: RangeKind::CodeBlock,
            }]
        );
    }

    #[test]
    fn hidden_integral_limits_are_clickable_without_growing_the_line() {
        let list = vec![MathNode::BigOp {
            kind: BigOp::ContourIntegral,
            lower: vec![],
            upper: vec![],
        }];
        let d = doc_with(vec![Block::Paragraph(vec![
            Inline::Math(list.clone()),
            Inline::Text(Text {
                text: " next".into(),
                style: Style::PLAIN,
            }),
        ])]);
        let laid = layout(&d, 20.0, &fake_measure);
        assert_eq!(laid.blocks[0].lines.len(), 2);
        let line = &laid.blocks[0].lines[0];
        let expression = math_layout::layout(&list, 0, 1.0, &fake_measure);
        assert_eq!(
            line.height,
            LINE_BODY.max(expression.ascent + expression.descent + MATH_LEADING)
        );
        let bounds = math_layout::interaction_bounds(&expression);
        assert!(bounds.ascent > expression.ascent);
        assert!(bounds.descent > expression.descent);

        let BoxKind::Row { children } = &expression.kind else {
            panic!("expression must be a row");
        };
        let BoxKind::Row { children: operator } = &children[0].2.kind else {
            panic!("operator must be a row");
        };
        for (slot, child) in [(Slot::Lower, &operator[1]), (Slot::Upper, &operator[2])] {
            let x = children[0].0 + child.0 + child.2.width * 0.5;
            let local_y = children[0].1 + child.1;
            let y = line.y + line.height * 0.5 - local_y;
            let (_, _, _, cursor) = laid
                .hit_math(x, y, &fake_measure)
                .expect("hidden limit must remain interactive");
            assert_eq!(cursor.path, vec![Step { index: 0, slot }]);
            assert_eq!(cursor.index, 0);
        }
    }

    #[test]
    fn the_boundary_between_math_atoms_enters_the_left_atom_at_its_end() {
        let first = Inline::Math(vec![MathNode::Sym('x')]);
        let second = Inline::Math(vec![MathNode::Sym('y')]);
        let d = doc_with(vec![Block::Paragraph(vec![first.clone(), second])]);
        let laid = layout(&d, 300.0, &fake_measure);
        let first_width = advance(
            &first,
            "\u{FFFC}",
            &d.body()[0],
            Style::PLAIN,
            None,
            1.0,
            &fake_measure,
        );
        let line = &laid.blocks[0].lines[0];
        let (_, inline, _, cursor) = laid
            .hit_math(first_width, line.y + line.height * 0.5, &fake_measure)
            .expect("shared boundary must delegate to math hit-testing");
        assert_eq!(inline, 0);
        assert_eq!(cursor.index, 1);
    }

    #[test]
    fn a_display_math_block_is_as_tall_as_its_expression() {
        let single = Block::Math {
            list: vec![Inline::Math(vec![MathNode::Sym('x')])],
            tag: None,
        };
        let nested = Block::Math {
            list: vec![Inline::Math(vec![MathNode::Frac {
                num: vec![MathNode::Frac {
                    num: vec![MathNode::Sym('1')],
                    den: vec![MathNode::Sym('2')],
                }],
                den: vec![MathNode::Sym('3')],
            }])],
            tag: None,
        };
        let single_height = layout(&doc_with(vec![single]), 300.0, &fake_measure).blocks[0].height;
        let nested_height = layout(&doc_with(vec![nested]), 300.0, &fake_measure).blocks[0].height;
        assert!(nested_height > single_height);
    }

    fn tagged_eq(tag: Option<&str>) -> Block {
        Block::Math {
            list: vec![Inline::Math(vec![MathNode::Sym('x')])],
            tag: tag.map(str::to_string),
        }
    }

    fn plain_run(t: &str) -> Inline {
        Inline::Text(Text {
            text: t.into(),
            style: Style::PLAIN,
        })
    }

    #[test]
    fn tagged_equations_number_in_document_order() {
        let d = doc_with(vec![
            tagged_eq(Some("eq:a")),
            tagged_eq(None),
            tagged_eq(Some("eq:b")),
            Block::Paragraph(vec![
                plain_run("see "),
                Inline::EqRef("eq:b".into()),
                plain_run(" and "),
                Inline::EqRef("eq:missing".into()),
            ]),
        ]);
        let laid = layout(&d, 600.0, &fake_measure);
        // Untagged math takes no number; the tagged ones count 1, 2.
        assert_eq!(
            laid.equation_numbers,
            HashMap::from([(0, "(1)".to_string()), (2, "(2)".to_string())])
        );
        // A resolved reference draws the number it points at; an unresolved
        // one keeps its raw spelling.
        let segments = &laid.blocks[3].lines[0].segments;
        let resolved = segments.iter().find(|s| s.inline == 1).unwrap();
        assert_eq!(resolved.number.as_deref(), Some("(2)"));
        let unresolved = segments.iter().find(|s| s.inline == 3).unwrap();
        assert_eq!(unresolved.number.as_deref(), Some("@eq:missing"));
    }

    #[test]
    fn a_repeated_tag_resolves_to_its_first_equation() {
        let d = doc_with(vec![
            tagged_eq(Some("eq:a")),
            tagged_eq(Some("eq:a")),
            Block::Paragraph(vec![plain_run("see "), Inline::EqRef("eq:a".into())]),
        ]);
        let laid = layout(&d, 600.0, &fake_measure);
        assert_eq!(laid.equation_numbers[&0], "(1)");
        assert_eq!(laid.equation_numbers[&1], "(1)");
        let segments = &laid.blocks[2].lines[0].segments;
        let reference = segments.iter().find(|s| s.inline == 1).unwrap();
        assert_eq!(reference.number.as_deref(), Some("(1)"));
    }

    #[test]
    fn a_reference_measures_as_the_number_it_draws() {
        let d = doc_with(vec![
            tagged_eq(Some("eq:a")),
            Block::Paragraph(vec![plain_run("see "), Inline::EqRef("eq:a".into())]),
        ]);
        let plain_width = {
            let run = plain_run("see ");
            advance(
                &run,
                "see ",
                &d.body()[1],
                Style::PLAIN,
                None,
                1.0,
                &fake_measure,
            )
        };
        // "see " is four glyphs; the reference is one flat position whose
        // width is the `(1)` it draws, not the placeholder atom.
        let reference_width = {
            let run = Inline::EqRef("eq:a".into());
            advance(
                &run,
                "\u{FFFC}",
                &d.body()[1],
                Style::PLAIN,
                Some("(1)"),
                1.0,
                &fake_measure,
            )
        };
        assert_eq!(plain_width, 40.0);
        assert_eq!(reference_width, 30.0);
    }

    fn heading(level: u8, text: &str) -> Block {
        Block::Heading {
            level,
            folded: false,
            content: vec![Inline::Text(Text {
                text: text.into(),
                style: Style::PLAIN,
            })],
        }
    }

    fn folded_heading(level: u8, text: &str) -> Block {
        let mut block = heading(level, text);
        if let Block::Heading { folded, .. } = &mut block {
            *folded = true;
        }
        block
    }

    const INDICATOR: f32 = FOLD_INDICATOR_HEIGHT;

    #[test]
    fn a_folded_heading_hides_its_body_up_to_the_next_heading_of_its_level() {
        let blocks = vec![
            folded_heading(1, "Top"),
            para("body one"),
            para("body two"),
            heading(1, "Next"),
            para("visible again"),
        ];
        let laid = layout_blocks(&blocks, 400.0, 1.0, &fake_measure);
        let full = layout_blocks(
            &[
                heading(1, "Top"),
                para("body one"),
                para("body two"),
                heading(1, "Next"),
                para("visible again"),
            ],
            400.0,
            1.0,
            &fake_measure,
        );

        // Blocks 1 and 2 are hidden: zero height, no lines, owned by block 0.
        assert_eq!(laid.blocks[1].hidden, Some(0));
        assert_eq!(laid.blocks[2].hidden, Some(0));
        assert!(laid.blocks[1].lines.is_empty());
        assert_eq!(laid.blocks[1].height, 0.0);
        // Everything from "Next" on is visible and sits exactly one heading's
        // spacing below the folded heading — the hidden body contributes
        // nothing, so the reflow is instant and leaves no residue. The folded
        // heading itself is taller by exactly its indicator band.
        assert_eq!(laid.blocks[0].height, full.blocks[0].height + INDICATOR);
        assert_eq!(
            laid.blocks[3].y - (laid.blocks[0].y + laid.blocks[0].height),
            GAP_AFTER_HEADING + GAP_HEADING,
        );
        assert!(laid.blocks[3].hidden.is_none());
        assert!(laid.blocks[4].hidden.is_none());

        // The folded heading's indicator carries the hidden line count.
        let indicator = laid.blocks[0].indicator.as_ref().unwrap();
        assert_eq!(indicator.lines, 2);
    }

    #[test]
    fn a_folded_h3_inside_a_folded_h1_hides_with_it_and_keeps_its_own_fold() {
        let blocks = vec![
            folded_heading(1, "Top"),
            para("inner body"),
            folded_heading(3, "Sub"),
            para("sub body"),
            heading(1, "Next"),
        ];
        let laid = layout_blocks(&blocks, 400.0, 1.0, &fake_measure);
        // Everything between the H1 and the next H1 is hidden by the H1.
        for index in 1..4 {
            assert_eq!(laid.blocks[index].hidden, Some(0));
        }
        // The hidden H3 emits no indicator of its own.
        assert!(laid.blocks[2].indicator.is_none());
        assert_eq!(laid.blocks[0].indicator.as_ref().unwrap().lines, 3);
    }

    #[test]
    fn an_unfolded_h1_leaves_its_nested_folded_h3_folded_with_its_own_indicator() {
        let blocks = vec![
            heading(1, "Top"),
            para("inner body"),
            folded_heading(3, "Sub"),
            para("sub body"),
            heading(1, "Next"),
        ];
        let laid = layout_blocks(&blocks, 400.0, 1.0, &fake_measure);
        assert!(laid.blocks[0].indicator.is_none());
        assert!(laid.blocks[2].indicator.is_some());
        assert!(laid.blocks[1].hidden.is_none());
        assert_eq!(laid.blocks[3].hidden, Some(2));
    }

    #[test]
    fn a_fold_with_no_body_gets_no_indicator() {
        let blocks = vec![folded_heading(1, "Empty"), heading(1, "Next")];
        let laid = layout_blocks(&blocks, 400.0, 1.0, &fake_measure);
        assert!(laid.blocks[0].indicator.is_none());
    }

    #[test]
    fn the_indicator_band_and_hidden_ground_answer_clicks() {
        let blocks = vec![
            folded_heading(1, "Top"),
            para("body one"),
            para("body two"),
            heading(1, "Next"),
        ];
        let laid = layout_blocks(&blocks, 400.0, 1.0, &fake_measure);
        let indicator = laid.blocks[0].indicator.as_ref().unwrap();

        // A click in the indicator band (any x — the whole column) unfolds.
        let mid = indicator.y + INDICATOR * 0.5;
        assert_eq!(laid.fold_indicator_at(mid), Some(0));
        assert_eq!(laid.fold_indicator_at(indicator.y - 0.5), None);

        // A click on the folded ground below lands on the owning heading.
        let caret = laid.hit(100.0, indicator.y + 5.0, &fake_measure);
        assert_eq!(caret.block, 0);

        // The chevron hit band sits in the gutter and nowhere else.
        let right = chevron_right(0.0, 1.0);
        let band_mid_y = laid.blocks[0].lines[0].y + laid.blocks[0].lines[0].height * 0.5;
        assert_eq!(
            laid.fold_chevron_at(right - CHEVRON_WIDTH / 2.0, band_mid_y, &fake_measure),
            Some(0)
        );
        // …but a click right of the gutter is a caret placement, not a fold.
        assert_eq!(laid.fold_chevron_at(50.0, band_mid_y, &fake_measure), None);
    }

    #[test]
    fn vertical_motion_skips_hidden_blocks() {
        let blocks = vec![
            folded_heading(1, "Top"),
            para("hidden one"),
            para("hidden two"),
            heading(1, "Next"),
        ];
        let laid = layout_blocks(&blocks, 400.0, 1.0, &fake_measure);
        let caret = Caret {
            block: 0,
            inline: 0,
            offset: 0,
            style: Style::PLAIN,
        };
        // Down from the folded heading lands past the hidden body.
        let down = laid.line_down(caret, 0.0, &fake_measure).unwrap();
        assert_eq!(down.block, 3);
        // And back up returns to the heading, never to a hidden block.
        let up = laid.line_up(down, 0.0, &fake_measure).unwrap();
        assert_eq!(up.block, 0);
    }

    #[test]
    fn chevron_right_clears_the_number_and_stays_in_the_gutter() {
        // Wide auto-numbers push the chevron left, but never out of reach.
        let wide = chevron_right(40.0, 1.0);
        let bare = chevron_right(0.0, 1.0);
        assert_eq!(bare, -(NUMBER_GUTTER + CHEVRON_GAP));
        assert_eq!(wide, CHEVRON_RIGHT_FLOOR);
        assert!(wide < bare);
    }

    #[test]
    fn a_table_fills_the_measure_and_exposes_cells_and_track_dividers() {
        let mut document = Document::new(std::path::Path::new("notes/table.md"));
        document.insert_table();
        let laid = layout(&document, 400.0, &fake_measure);
        let table = laid.tables[0].as_ref().unwrap();
        let middle = laid.blocks[0].y + table.row_height * 0.5;

        assert_eq!(table.columns.iter().sum::<f32>(), 400.0);
        assert_eq!(laid.hit(300.0, middle, &fake_measure).inline, 1);
        let second_column = Caret {
            block: 0,
            inline: 1,
            offset: 0,
            style: Style::PLAIN,
        };
        let goal_x = laid.caret_pos(second_column, &fake_measure).0;
        let next_row = laid
            .line_down(second_column, goal_x, &fake_measure)
            .expect("the second table row is visible");
        assert_eq!((next_row.block, next_row.inline), (1, 1));
        assert!(matches!(
            laid.table_resize_at(table.columns[0], middle, 1.0),
            Some(TableResize::Column {
                first: 0,
                divider: 0
            })
        ));
        assert!(matches!(
            laid.table_resize_at(100.0, laid.blocks[0].y + laid.blocks[0].height, 1.0),
            Some(TableResize::Row {
                first: 0,
                divider: 0
            })
        ));
    }

    #[test]
    fn long_table_text_wraps_into_a_taller_editable_cell() {
        let mut document = Document::new(std::path::Path::new("notes/table.md"));
        document.insert_table();
        document.insert_text("abcdefghij");
        let laid = layout(&document, 100.0, &fake_measure);
        let table = laid.tables[0].as_ref().unwrap();
        assert!(
            table.row_height > crate::document::table::DEFAULT_ROW_HEIGHT,
            "cell text grows the row instead of painting through its edge"
        );
        let caret = Caret {
            block: 0,
            inline: 0,
            offset: 6,
            style: Style::PLAIN,
        };
        let (x, y, _) = laid.caret_pos(caret, &fake_measure);
        assert_eq!(laid.hit(x, y, &fake_measure), caret);
        assert!(matches!(
            laid.hit_context(x, y, &fake_measure),
            Some(ContextHit::Range {
                kind: RangeKind::Word,
                ..
            })
        ));
    }

    #[test]
    fn a_math_table_cell_uses_the_table_layout_and_remains_interactive() {
        let mut document = Document::new(std::path::Path::new("notes/table.md"));
        document.insert_table();
        document.insert_inline_math();
        document.math_insert_char('x');
        let laid = layout(&document, 400.0, &fake_measure);
        let table = laid.tables[0]
            .as_ref()
            .expect("the table layout survives math");
        let y = laid.blocks[0].y + table.row_height * 0.5;
        // A cell line whose only run is an expression is centred in its
        // column, so the middle of the first track is the middle of the
        // expression.
        let x = table.columns[0] * 0.5;
        let hit = laid
            .hit_math(x, y, &fake_measure)
            .expect("the expression remains directly editable");
        assert_eq!((hit.0, hit.1), (0, 0));
        assert!(matches!(
            laid.hit_context(x, y, &fake_measure),
            Some(ContextHit::Math {
                block: 0,
                inline: 0,
                ..
            })
        ));
    }

    #[test]
    fn a_tables_rows_share_one_track_vector_and_row_count() {
        // The tracks and the row count are the table's, not the row's: every
        // row's entry points at the same allocation, so a consumer can read
        // the row count without re-counting the table.
        let mut document = Document::new(std::path::Path::new("notes/table.md"));
        document.insert_table();
        let laid = layout(&document, 400.0, &fake_measure);
        let first = laid.tables[0].as_ref().expect("the header row lays out");
        let second = laid.tables[1].as_ref().expect("the body row lays out");
        assert_eq!((first.rows, second.rows), (2, 2));
        assert!(std::sync::Arc::ptr_eq(&first.columns, &second.columns));
        assert_eq!(first.columns.iter().sum::<f32>(), 400.0);
    }

    #[test]
    fn two_tables_in_one_document_do_not_share_tracks() {
        // The shared tracks are cached per table, keyed on the rows it
        // covers: a second table must measure its own rather than inherit
        // the first one's column count or widths.
        let markdown = "| a | b |\n| --- | --- |\n| c | d |\n\n\
                        | x | y | z |\n| --- | --- | --- |\n| p | q | r |\n";
        let document =
            crate::document::markdown::parse(std::path::Path::new("notes/two.md"), markdown);
        let laid = layout(&document, 600.0, &fake_measure);
        let first = laid.tables[0].as_ref().expect("the first table lays out");
        let second = laid.tables[2].as_ref().expect("the second table lays out");
        assert_eq!((first.rows, first.columns.len()), (2, 2));
        assert_eq!((second.rows, second.columns.len()), (2, 3));
        assert!(!std::sync::Arc::ptr_eq(&first.columns, &second.columns));
    }

    #[test]
    fn a_two_line_cell_places_the_caret_on_both_its_lines() {
        // A cell line's segments address that line's own runs, never the
        // cell's whole run list. The second line's list is one run long, so
        // a cell-wide index is out of bounds here.
        let mut document = Document::new(std::path::Path::new("notes/table.md"));
        document.insert_table();
        document.insert_text("a");
        assert!(document.split_cell_line());
        document.insert_text("b");
        let laid = layout(&document, 400.0, &fake_measure);
        let table = laid.tables[0].as_ref().expect("the table lays out");
        assert_eq!(table.cells[0].len(), 2);
        assert_eq!(
            (table.cells[0][0].cell_line, table.cells[0][1].cell_line),
            (0, 1)
        );

        // Offset 2 is the start of the cell's second line; 3 is its end. Both
        // survive a place-then-click round trip, and neither panics.
        for offset in [2usize, 3] {
            let caret = Caret {
                block: 0,
                inline: 0,
                offset,
                style: Style::PLAIN,
            };
            let (x, y, _) = laid.caret_pos(caret, &fake_measure);
            assert_eq!(laid.hit(x, y, &fake_measure), caret);
        }
    }

    /// Every caret reader asked about a table row.
    ///
    /// A table row's content is its cells, so `Block::inlines` is empty for one
    /// and any reader that indexes a block's run list panics. This is the
    /// regression for the crash on opening a note whose FIRST block is a table
    /// (`~/Notes/tables.md`, a math cell beside an empty one): `caret_band` —
    /// the scroll-follow the shell runs on the first frame of a freshly opened
    /// tab — read the row's run list and indexed it.
    #[test]
    fn every_caret_reader_survives_a_table_document() {
        let d = crate::document::markdown::parse(
            std::path::Path::new("notes/tables.md"),
            "| $1/2$ |  |\n| --- | --- |\n|  |  |\n",
        );
        let laid = layout(&d, 800.0, &fake_measure);
        assert_eq!(d.body().len(), 2);
        for block in 0..d.body().len() {
            assert!(
                laid.tables[block].is_some(),
                "every table row has table layout"
            );
            let row_top = laid.blocks[block].y;
            let row_bottom = row_top + laid.blocks[block].height;
            for cell in 0..d.body()[block].cells().len() {
                for offset in [0usize, 1] {
                    let caret = Caret {
                        block,
                        inline: cell,
                        offset,
                        style: Style::PLAIN,
                    };
                    // The row's own band, not a line of some run list.
                    assert_eq!(laid.caret_band(caret), (row_top, row_bottom));
                    let (x, y, height) = laid.caret_pos(caret, &fake_measure);
                    assert!(height > 0.0 && x >= 0.0 && y >= row_top);
                    if offset == 0 {
                        assert_eq!(laid.hit(x, y, &fake_measure), caret);
                    } else {
                        let _ = laid.hit(x, y, &fake_measure);
                    }
                    let _ = laid.hit_context(x, y, &fake_measure);
                    let _ = laid.line_up(caret, x, &fake_measure);
                    let _ = laid.line_down(caret, x, &fake_measure);
                    let _ = laid.table_cell_at(x, y);
                    let _ = laid.table_resize_at(x, y, 2.0);
                    let _ = laid.fold_chevron_at(x, y, &fake_measure);
                    let _ = laid.fold_indicator_at(y);
                }
            }
        }
    }
}
