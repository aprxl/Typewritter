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

use crate::document::math::{MathCursor, NodeAddress};
use crate::document::{Block, Caret, Document, FlatPos, FlatRange, Inline, Style, math_layout};
use crate::theme::{self, TextStyle};

/// Visual line heights, in logical pixels.
pub const LINE_BODY: f32 = 30.0;
pub const LINE_H1: f32 = 42.0;
pub const LINE_H2: f32 = 36.0;
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
/// Clear space above *and* below a display expression — the **total**, not
/// an addition to whatever the neighbouring block already contributes, so an
/// equation is inset by the same amount top and bottom whatever it sits
/// between. Wider than a paragraph's gap because a displayed equation is set
/// apart from the prose around it rather than wrapped into it. This is on
/// top of [`MATH_PAD`], which is the breathing room *inside* the block.
pub const GAP_MATH: f32 = 24.0;
/// Space *above* a heading (the first block gets none).
pub const GAP_HEADING: f32 = 26.0;
/// Space below a heading.
pub const GAP_AFTER_HEADING: f32 = 8.0;
/// Space below a rule — tighter than a paragraph's, for the same reason.
pub const GAP_DIVIDER: f32 = 8.0;
/// The size of an anchor's raised number. Matches the heading's auto-number:
/// both are margin annotations, not part of the prose they annotate.
pub const ANCHOR_SIZE: f32 = 11.0;
/// How far an anchor's number sits above the line baseline — high enough to
/// read as a footnote marker, low enough not to collide with the line above.
pub const ANCHOR_RISE: f32 = 6.0;

/// A run of a visual line that came from one source run, covering exactly
/// `[start, start + len)` chars of it. Ranges on a line are contiguous and
/// exact — no gaps, no overlaps — which is what makes caret math trivial.
pub struct Segment {
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

/// One visual line: a contiguous slice of the source block's flat text.
pub struct VisLine {
    /// Top of the line, relative to content top.
    pub y: f32,
    /// Actual line height: the block's floor or its tallest content.
    pub height: f32,
    pub segments: Vec<Segment>,
}

/// One block's laid-out lines plus where it sits.
pub struct BlockLayout {
    /// Top of the block, relative to content top.
    pub y: f32,
    pub lines: Vec<VisLine>, // never empty — an empty block has one empty line
    pub height: f32,
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
    /// The y of the anchor's visual line, in document coordinates.
    pub y: f32,
}

/// A whole document's layout.
pub struct DocLayout {
    /// The visual blocks, one per source block.
    pub blocks: Vec<BlockLayout>,
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
    /// Each sidenote anchor, in document order, with its derived number and
    /// the y of the line it sits on. Nothing stores these — like the heading
    /// outline, they are derived from position so a number can never
    /// disagree with the anchor beside it.
    pub anchors: Vec<Anchor>,
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
        node: Option<NodeAddress>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeKind {
    Word,
    Badge,
    InlineCode,
    CodeBlock,
}

/// The font a run renders with. Body is serif 17.5; headings are serif at
/// 24/21/18.5 and always bold; a run's `bold`/`italic` stack on top. `scale`
/// multiplies every size, so a note laid out at the margin's scale reads from
/// here rather than from a second copy of the size table.
pub fn text_style(kind: &Block, style: Style, scale: f32) -> TextStyle {
    if style.code {
        return TextStyle::mono(17.5 * scale, theme::INK);
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
        Block::Heading { level, .. } => TextStyle::serif(
            match level {
                1 => 24.0,
                2 => 21.0,
                3 => 18.5,
                _ => 17.5,
            } * scale,
            theme::INK,
        )
        .bold(),
        Block::Paragraph(_) | Block::Divider(_) | Block::Math(_) => {
            TextStyle::serif(17.5 * scale, theme::INK)
        }
        Block::CodeLine { .. } => TextStyle::mono(17.5 * scale, theme::INK),
    };
    if style.bold {
        base = base.bold();
    }
    if style.italic {
        base = base.italic();
    }
    base
}

/// The raised number an anchor draws with. The size matches the heading's
/// auto-number and the colour is the accent, so an anchor reads as "this
/// opens something" rather than as a word in the sentence.
pub fn anchor_style() -> TextStyle {
    TextStyle::serif(ANCHOR_SIZE, theme::ACCENT)
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
    block: &Block,
    scale: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> f32 {
    advance(
        &block.inlines()[piece.inline],
        &piece.text,
        block,
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
fn wrap(
    pieces: &[Piece],
    block: &Block,
    width: f32,
    scale: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> Vec<Vec<usize>> {
    let mut lines = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    let mut cursor = 0.0f32;
    let mut first = true;

    for (i, piece) in pieces.iter().enumerate() {
        if piece.space {
            cursor += piece_width(piece, block, scale, measure);
            current.push(i);
            continue;
        }
        let word_width = piece_width(piece, block, scale, measure);
        if first {
            // First piece starts the line at the left, whatever it is.
        } else if cursor + word_width <= width {
            // Fits.
        } else {
            // The trailing space stays on the line we are closing.
            lines.push(std::mem::take(&mut current));
            cursor = 0.0;
        }
        current.push(i);
        cursor += word_width;
        first = false;
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(Vec::new());
    }
    lines
}

fn tokens(
    block: &Block,
    block_index: usize,
    numbers: &HashMap<(usize, usize), String>,
) -> Vec<Piece> {
    let mut pieces = Vec::new();
    for (inline, run) in block.inlines().iter().enumerate() {
        let text = run.text();
        let number = numbers.get(&(block_index, inline)).cloned();
        let chars: Vec<(usize, char)> = text.char_indices().collect();
        let mut k = 0;
        while k < chars.len() {
            let space = chars[k].1.is_whitespace();
            let mut j = k;
            while j < chars.len() && chars[j].1.is_whitespace() == space {
                j += 1;
            }
            pieces.push(Piece {
                text: chars[k..j].iter().map(|(_, c)| *c).collect(),
                style: run.style(),
                inline,
                start: k,
                len: j - k,
                space,
                number: number.clone(),
            });
            // Math text is one opaque ATOM character, so this already creates
            // one unsplittable non-space piece for the whole expression.
            k = j;
        }
    }
    pieces
}

/// Merge a line's piece indices into segments, folding together adjacent
/// pieces from the same run so the output is one segment per source stretch.
fn segments_for(pieces: &[Piece], line: &[usize]) -> Vec<Segment> {
    let mut segs: Vec<Segment> = Vec::new();
    for &i in line {
        let p = &pieces[i];
        match segs.last_mut() {
            Some(s) if s.inline == p.inline && s.start + s.len == p.start => {
                s.len += p.len;
            }
            _ => segs.push(Segment {
                inline: p.inline,
                start: p.start,
                len: p.len,
                style: p.style,
                number: p.number.clone(),
            }),
        }
    }
    segs
}

/// One pass over `doc`'s own blocks at full scale; `measure(text, style) ->
/// width` is the only rendering input. The thin wrapper around
/// [`layout_blocks`] — a whole document is just its body at scale `1.0`.
pub fn layout(doc: &Document, width: f32, measure: &dyn Fn(&str, &TextStyle) -> f32) -> DocLayout {
    layout_blocks(doc.body(), width, 1.0, measure)
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
                    y: 0.0,
                });
            }
        }
    }

    let mut laid = Vec::with_capacity(blocks.len());
    let mut y = 0.0f32;
    let mut first_block = true;
    // The gap the previous block already contributed below itself. A display
    // expression tops this up to `GAP_MATH` rather than adding to it, which
    // is what keeps its space above equal to its space below.
    let mut gap_below_previous = 0.0f32;

    for (source_index, block) in blocks.iter().enumerate() {
        let gap_above = if first_block {
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

        let base_line_height = match block {
            Block::Heading { level: 1, .. } => LINE_H1 * scale,
            Block::Heading { level: 2, .. } => LINE_H2 * scale,
            Block::Heading { level: 3, .. } => LINE_H3 * scale,
            Block::Heading { level: 4, .. } => LINE_H4 * scale,
            Block::Divider(_) => LINE_DIVIDER * scale,
            Block::Math(runs) => {
                let Inline::Math(list) = &runs[0] else {
                    unreachable!("math block must contain one math atom")
                };
                let expression = math_layout::layout(list, 0, scale, measure);
                expression.ascent + expression.descent + MATH_PAD * 2.0 * scale
            }
            Block::Heading { .. } | Block::Paragraph(_) | Block::CodeLine { .. } => {
                LINE_BODY * scale
            }
        };

        let pieces = tokens(block, source_index, &number_of);
        let grouped = wrap(&pieces, block, width, scale, measure);
        let mut line_y = y;
        let lines = grouped
            .iter()
            .map(|line_pieces| {
                let content_height = line_pieces
                    .iter()
                    .filter_map(
                        |&piece_index| match &block.inlines()[pieces[piece_index].inline] {
                            Inline::Math(list) => {
                                let expression = math_layout::layout(list, 0, scale, measure);
                                Some(expression.ascent + expression.descent + MATH_LEADING * scale)
                            }
                            Inline::Text(_) | Inline::Note(_) => None,
                        },
                    )
                    .fold(0.0, f32::max);
                let height = base_line_height.max(content_height);
                let line = VisLine {
                    y: line_y,
                    height,
                    segments: segments_for(&pieces, line_pieces),
                };
                line_y += height;
                line
            })
            .collect::<Vec<_>>();

        // VisLine heights are content-driven, so later lines start after the
        // actual height of every earlier line rather than a copied constant.
        let height = lines.iter().map(|line| line.height).sum();
        laid.push(BlockLayout { y, lines, height });
        y += height;

        let gap_after = if block.is_code()
            && matches!(
                blocks.get(source_index + 1),
                Some(Block::CodeLine { first: false, .. })
            ) {
            0.0
        } else if block.is_math() {
            GAP_MATH * scale
        } else if block.is_heading() {
            GAP_AFTER_HEADING * scale
        } else if block.is_divider() {
            GAP_DIVIDER * scale
        } else {
            GAP_PARAGRAPH * scale
        };
        y += gap_after;
        gap_below_previous = gap_after;
    }

    // A note sits beside the line its anchor is on, and that line's y is only
    // known after the block is laid out — fill it in now that the lines exist.
    for anchor in &mut anchors {
        anchor.y = anchor_y(&laid[anchor.block], anchor.inline);
    }

    DocLayout {
        blocks: laid,
        source: blocks.to_vec(),
        height: y,
        scale,
        anchors,
    }
}

/// The y of the visual line the anchor `inline` sits on, in document
/// coordinates — the same space the editor scrolls in. A note in the margin
/// starts at this y, so it sits beside the sentence that anchored it.
fn anchor_y(block: &BlockLayout, inline: usize) -> f32 {
    for line in &block.lines {
        if line.segments.iter().any(|segment| segment.inline == inline) {
            return line.y;
        }
    }
    block.y
}

fn run_text(run: &Inline) -> &str {
    match run {
        Inline::Text(t) => &t.text,
        Inline::Math(_) => "\u{FFFC}",
        Inline::Note(_) => "\u{FFFC}",
    }
}

fn run_style(run: &Inline) -> Style {
    match run {
        Inline::Text(t) => t.style,
        Inline::Math(_) => Style::PLAIN,
        Inline::Note(_) => Style::PLAIN,
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

/// The char index (block-flat) that a segment covers up to, exclusive.
fn block_flat_len(block: &Block) -> usize {
    block
        .inlines()
        .iter()
        .map(|r| run_text(r).chars().count())
        .sum()
}

fn flat_of_caret(source: &[Block], caret: Caret) -> usize {
    source.get(caret.block).map_or(0, |block| {
        let runs = block.inlines();
        let inline = caret.inline.min(runs.len().saturating_sub(1));
        let prefix: usize = runs[..inline]
            .iter()
            .map(|r| run_text(r).chars().count())
            .sum();
        let offset = caret.offset.min(run_text(&runs[inline]).chars().count());
        (prefix + offset).min(block_flat_len(block))
    })
}

/// Style of the char at block-flat `pos`, or `None` at the block's end.
fn style_at(source: &[Block], block: usize, pos: usize) -> Option<Style> {
    let runs = source.get(block)?.inlines();
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
    block: &Block,
    line: &VisLine,
    line_start: usize,
    flat: usize,
    scale: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> f32 {
    let mut x = 0.0;
    let mut seg_flat = line_start;
    for segment in &line.segments {
        let run = &block.inlines()[segment.inline];
        let text = segment_text(run, segment);
        let seg_len = segment.len;
        if flat >= seg_flat + seg_len {
            x += advance(
                run,
                &text,
                block,
                segment.style,
                segment.number.as_deref(),
                scale,
                measure,
            );
            seg_flat += seg_len;
        } else {
            // The caret is inside this segment: measure its prefix, past
            // the badge box's left edge if there is one — the label starts
            // inside the box, so the caret has to as well.
            let up_to = flat - seg_flat;
            let prefix: String = text.chars().take(up_to).collect();
            if segment.style.badge {
                x += theme::BADGE_PAD * scale;
            }
            x += measure(&prefix, &text_style(block, segment.style, scale));
            break;
        }
    }
    x
}

/// The visual line that flat offset falls in. At the block's very end it is
/// the last line; at a wrap boundary it is the *next* line's start.
fn line_of_flat(layout_block: &BlockLayout, flat: usize) -> usize {
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
    let start = line_flat_start(layout_block, line_idx);

    // The clicked char's offset within the line's flat text, defaulting to
    // the line end. Segments are contiguous, so `pos` advances through them.
    let mut pos = line.segments.iter().map(|s| s.len).sum();
    let mut cum = 0.0;
    let mut seg_flat = 0;

    'segments: for segment in &line.segments {
        let run = &block.inlines()[segment.inline];
        let text = segment_text(run, segment);
        let width = advance(
            run,
            &text,
            block,
            segment.style,
            segment.number.as_deref(),
            scale,
            measure,
        );
        // The label sits inside its box; skip the left edge so a click
        // lands on the character the user aimed at.
        if matches!(run, Inline::Math(_) | Inline::Note(_)) {
            if x <= cum + width / 2.0 {
                pos = seg_flat;
                break 'segments;
            }
        } else {
            let style = text_style(block, segment.style, scale);
            let text_x = cum
                + if segment.style.badge {
                    theme::BADGE_PAD * scale
                } else {
                    0.0
                };
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
    /// Reports the label and y of each anchor in the document, in document
    /// order, so the margin can put a note beside the sentence that made it.
    /// The y is in document coordinates, the same space the editor scrolls in.
    pub fn note_anchors(&self) -> Vec<(String, f32)> {
        self.anchors
            .iter()
            .map(|anchor| (anchor.label.clone(), anchor.y))
            .collect()
    }

    /// (x, baseline-y, line-height) of a model caret, relative to content top.
    pub fn caret_pos(
        &self,
        caret: Caret,
        measure: &dyn Fn(&str, &TextStyle) -> f32,
    ) -> (f32, f32, f32) {
        let flat = flat_of_caret(&self.source, caret);
        let block = &self.source[caret.block];
        let layout_block = &self.blocks[caret.block];
        let line_idx = line_of_flat(layout_block, flat);
        let line = &layout_block.lines[line_idx];
        let line_start = line_flat_start(layout_block, line_idx);
        let x = x_of_flat(block, line, line_start, flat, self.scale, measure);
        (x, line.y + line.height / 2.0, line.height)
    }

    /// Nearest caret position for a click at (x, y) — y relative to content
    /// top. Style context follows the style-before rule.
    pub fn hit(&self, x: f32, y: f32, measure: &dyn Fn(&str, &TextStyle) -> f32) -> Caret {
        let block_idx = block_of_y(self, y);
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
        let block_idx = block_of_y(self, y);
        let layout_block = &self.blocks[block_idx];
        let line_idx = line_of_y(layout_block, y);
        if let Some((block, inline, node)) =
            self.hit_math_node_on_line(block_idx, line_idx, x, y, measure)
        {
            return Some(ContextHit::Math {
                block,
                inline,
                node,
            });
        }

        let line = &layout_block.lines[line_idx];
        let block = &self.source[block_idx];

        if block.is_code() {
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
            let width = advance(
                run,
                &text,
                block,
                segment.style,
                segment.number.as_deref(),
                self.scale,
                measure,
            );
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
                if segment.style.code {
                    return Some(ContextHit::Range {
                        range: whole_run,
                        kind: RangeKind::InlineCode,
                    });
                }

                let style = text_style(block, segment.style, self.scale);
                let text_x = advance_x
                    + if segment.style.badge {
                        theme::BADGE_PAD * self.scale
                    } else {
                        0.0
                    };
                let within_segment = context_char_for_x(&text, text_x, x, &style, measure);
                let clicked = run_start + segment.start + within_segment;
                let ch = text
                    .chars()
                    .nth(within_segment)
                    .expect("segment length matches segment text");
                if !ch.is_alphanumeric() && ch != '_' {
                    return self.hit_hidden_math_node(block_idx, line_idx, x, y, measure);
                }
                let chars: Vec<char> = block
                    .inlines()
                    .iter()
                    .flat_map(|run| run_text(run).chars())
                    .collect();
                let mut start = clicked;
                while start > 0 && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_') {
                    start -= 1;
                }
                let mut end = clicked + 1;
                while end < chars.len() && (chars[end].is_alphanumeric() || chars[end] == '_') {
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
                    kind: RangeKind::Word,
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
            if block.is_code() {
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

            let chars: Vec<char> = block
                .inlines()
                .iter()
                .flat_map(|run| run_text(run).chars())
                .collect();
            for line in &layout_block.lines {
                let baseline = line.y + line.height * 0.5;
                let mut advance_x = 0.0;
                for segment in &line.segments {
                    let run = &block.inlines()[segment.inline];
                    let text = segment_text(run, segment);
                    let width = advance(
                        run,
                        &text,
                        block,
                        segment.style,
                        segment.number.as_deref(),
                        self.scale,
                        measure,
                    );
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
                        } else if segment.style.code {
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
                            let mut char_x = advance_x;
                            for (within_segment, ch) in text.chars().enumerate() {
                                let char_width = measure(
                                    &ch.to_string(),
                                    &text_style(block, segment.style, self.scale),
                                );
                                if (ch.is_alphanumeric() || ch == '_')
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
                                    let mut start = clicked;
                                    while start > 0
                                        && (chars[start - 1].is_alphanumeric()
                                            || chars[start - 1] == '_')
                                    {
                                        start -= 1;
                                    }
                                    let mut end = clicked + 1;
                                    while end < chars.len()
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
                                            kind: RangeKind::Word,
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
                        node: hit.2,
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
    ) -> Option<(usize, usize, Option<NodeAddress>)> {
        let line = &self.blocks[block_idx].lines[line_idx];
        let block = &self.source[block_idx];
        let mut advance_x = 0.0;
        for segment in &line.segments {
            let run = &block.inlines()[segment.inline];
            let text = segment_text(run, segment);
            let width = advance(
                run,
                &text,
                block,
                segment.style,
                segment.number.as_deref(),
                self.scale,
                measure,
            );
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
    ) -> Option<(usize, usize, MathCursor)> {
        let block_idx = block_of_y(self, y);
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
    ) -> Option<(usize, usize, MathCursor)> {
        let line = &self.blocks[block_idx].lines[line_idx];
        let block = &self.source[block_idx];
        let mut advance_x = 0.0;

        for segment in &line.segments {
            let run = &block.inlines()[segment.inline];
            let text = segment_text(run, segment);
            let width = advance(
                run,
                &text,
                block,
                segment.style,
                segment.number.as_deref(),
                self.scale,
                measure,
            );
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
        let prev = block_idx - 1;
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

    /// One visual line down from `caret`, aiming at `goal_x` pixels.
    pub fn line_down(
        &self,
        caret: Caret,
        goal_x: f32,
        measure: &dyn Fn(&str, &TextStyle) -> f32,
    ) -> Option<Caret> {
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
        if block_idx + 1 >= self.blocks.len() {
            return None;
        }
        Some(caret_for_click(
            &self.source,
            &self.blocks[block_idx + 1],
            block_idx + 1,
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
        let line_idx = line_of_flat(layout_block, flat);
        let line = &layout_block.lines[line_idx];
        (line.y, line.y + line.height)
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
    use crate::document::{Focus, Inline, Sidenote, Text};
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
        let math = Block::Math(vec![Inline::Math(vec![MathNode::Sym('x')])]);
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
                content: vec![Inline::Text(Text {
                    text: "h".into(),
                    style: Style::PLAIN,
                })],
            },
            Block::Math(vec![Inline::Math(vec![MathNode::Sym('x')])]),
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
        assert_eq!(seen.get(), Some(24.0), "H1 text is measured at size 24");
        // The heading weight should be bold.
        let h1_style = text_style(&d.body()[0], Style::PLAIN, 1.0);
        assert!(h1_style.weight > 0.0, "headings are always bold");

        // H4 matches body size (17.5) — bold is what distinguishes it.
        let h4 = Block::Heading {
            level: 4,
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
        assert!(matches!(inside, Some((0, 1, _))));

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

        let (_, _, cursor) = laid
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
            let (_, _, cursor) = laid
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
        let (_, inline, cursor) = laid
            .hit_math(first_width, line.y + line.height * 0.5, &fake_measure)
            .expect("shared boundary must delegate to math hit-testing");
        assert_eq!(inline, 0);
        assert_eq!(cursor.index, 1);
    }

    #[test]
    fn a_display_math_block_is_as_tall_as_its_expression() {
        let single = Block::Math(vec![Inline::Math(vec![MathNode::Sym('x')])]);
        let nested = Block::Math(vec![Inline::Math(vec![MathNode::Frac {
            num: vec![MathNode::Frac {
                num: vec![MathNode::Sym('1')],
                den: vec![MathNode::Sym('2')],
            }],
            den: vec![MathNode::Sym('3')],
        }])]);
        let single_height = layout(&doc_with(vec![single]), 300.0, &fake_measure).blocks[0].height;
        let nested_height = layout(&doc_with(vec![nested]), 300.0, &fake_measure).blocks[0].height;
        assert!(nested_height > single_height);
    }
}
