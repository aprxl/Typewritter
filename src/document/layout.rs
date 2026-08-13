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

use crate::document::{Block, Caret, Document, Inline, Style};
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
/// Space below a paragraph.
pub const GAP_PARAGRAPH: f32 = 14.0;
/// Space *above* a heading (the first block gets none).
pub const GAP_HEADING: f32 = 26.0;
/// Space below a heading.
pub const GAP_AFTER_HEADING: f32 = 8.0;
/// Space below a rule — tighter than a paragraph's, for the same reason.
pub const GAP_DIVIDER: f32 = 8.0;

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
}

/// One visual line: a contiguous slice of the source block's flat text.
pub struct VisLine {
    /// Top of the line, relative to content top.
    pub y: f32,
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

/// A whole document's layout.
pub struct DocLayout {
    /// The visual blocks, one per source block.
    pub blocks: Vec<BlockLayout>,
    /// Total content height, for scroll bounds.
    pub height: f32,
    /// A snapshot of the source blocks, so the editor can index a visual
    /// block back to its kind and runs without holding the live document.
    pub source: Vec<Block>,
}

/// The font a run renders with. Body is serif 17.5; headings are serif at
/// 24/21/18.5 and always bold; a run's `bold`/`italic` stack on top.
pub fn text_style(kind: &Block, style: Style) -> TextStyle {
    if kind.is_math() {
        // Placeholder only; the math layout task will replace body prose here.
        return TextStyle::serif(17.5, theme::INK);
    }
    if style.code {
        return TextStyle::mono(17.5, theme::INK);
    }
    if style.badge {
        // A chip is set far smaller than the prose it sits in, tracked out
        // the way the design's labels are — it reads as machinery, not as
        // a word in the sentence.
        return TextStyle::mono(theme::BADGE_SIZE, theme::BADGE_INK).tracked(0.1);
    }
    let mut base = match kind {
        Block::Heading { level, .. } => TextStyle::serif(
            match level {
                1 => 24.0,
                2 => 21.0,
                3 => 18.5,
                _ => 17.5,
            },
            theme::INK,
        )
        .bold(),
        Block::Paragraph(_) | Block::Divider(_) | Block::Math(_) => {
            TextStyle::serif(17.5, theme::INK)
        }
        Block::CodeLine { .. } => TextStyle::mono(17.5, theme::INK),
    };
    if style.bold {
        base = base.bold();
    }
    if style.italic {
        base = base.italic();
    }
    base
}

/// One word or whitespace stretch, with its source coordinates.
struct Piece {
    text: String,
    style: Style,
    inline: usize,
    start: usize,
    len: usize,
    space: bool,
}

/// How far a run of text moves the cursor: its shaped width, plus the box
/// a badge draws around its label. A chip's box is part of the flow, not
/// decoration on top of it — measured as bare glyphs, a badge would sit
/// under the word after it.
///
/// The unit is one segment (a run's slice of one visual line), which is
/// what the caret and the editor both walk. Wrapping asks per whitespace-
/// split piece instead, so a multi-word label is measured a few pixels
/// wide there; it wraps a shade early and nothing drifts, since the caret
/// and the drawing agree with each other.
pub fn advance(
    text: &str,
    block: &Block,
    style: Style,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> f32 {
    let box_pad = if style.badge {
        theme::BADGE_PAD * 2.0
    } else {
        0.0
    };
    measure(text, &text_style(block, style)) + box_pad
}

fn piece_width(piece: &Piece, block: &Block, measure: &dyn Fn(&str, &TextStyle) -> f32) -> f32 {
    advance(&piece.text, block, piece.style, measure)
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
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> Vec<Vec<usize>> {
    let mut lines = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    let mut cursor = 0.0f32;
    let mut first = true;

    for (i, piece) in pieces.iter().enumerate() {
        if piece.space {
            cursor += piece_width(piece, block, measure);
            current.push(i);
            continue;
        }
        let word_width = piece_width(piece, block, measure);
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

fn tokens(block: &Block) -> Vec<Piece> {
    let mut pieces = Vec::new();
    for (inline, run) in block.inlines().iter().enumerate() {
        let text = run.text();
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
            });
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
            }),
        }
    }
    segs
}

/// One pass; `measure(text, style) -> width` is the only rendering input.
pub fn layout(doc: &Document, width: f32, measure: &dyn Fn(&str, &TextStyle) -> f32) -> DocLayout {
    let mut blocks = Vec::with_capacity(doc.blocks.len());
    let mut y = 0.0f32;
    let mut first_block = true;

    for (source_index, block) in doc.blocks.iter().enumerate() {
        let gap_above = if first_block {
            0.0
        } else if block.is_heading() {
            GAP_HEADING
        } else {
            0.0
        };
        y += gap_above;
        first_block = false;

        let line_height = match block {
            Block::Heading { level: 1, .. } => LINE_H1,
            Block::Heading { level: 2, .. } => LINE_H2,
            Block::Heading { level: 3, .. } => LINE_H3,
            Block::Heading { level: 4, .. } => LINE_H4,
            Block::Divider(_) => LINE_DIVIDER,
            // Placeholder only; display math gets real sizing later.
            Block::Math(_) => LINE_BODY,
            Block::Heading { .. } | Block::Paragraph(_) | Block::CodeLine { .. } => LINE_BODY,
        };

        let pieces = tokens(block);
        let grouped = wrap(&pieces, block, width, measure);
        let line_count = grouped.len() as f32;

        let lines = grouped
            .iter()
            .enumerate()
            .map(|(i, line_pieces)| VisLine {
                y: y + i as f32 * line_height,
                height: line_height,
                segments: segments_for(&pieces, line_pieces),
            })
            .collect::<Vec<_>>();

        let height = line_count * line_height;
        blocks.push(BlockLayout { y, lines, height });
        y += height;

        let gap_after = if block.is_code()
            && matches!(
                doc.blocks.get(source_index + 1),
                Some(Block::CodeLine { first: false, .. })
            ) {
            0.0
        } else if block.is_heading() {
            GAP_AFTER_HEADING
        } else if block.is_divider() {
            GAP_DIVIDER
        } else {
            GAP_PARAGRAPH
        };
        y += gap_after;
    }

    DocLayout {
        blocks,
        source: doc.blocks.clone(),
        height: y,
    }
}

fn run_text(run: &Inline) -> &str {
    match run {
        Inline::Text(t) => &t.text,
        Inline::Math(_) => "\u{FFFC}",
    }
}

fn run_style(run: &Inline) -> Style {
    match run {
        Inline::Text(t) => t.style,
        Inline::Math(_) => Style::PLAIN,
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
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> f32 {
    let mut x = 0.0;
    let mut seg_flat = line_start;
    for segment in &line.segments {
        let run = &block.inlines()[segment.inline];
        let text = segment_text(run, segment);
        let seg_len = segment.len;
        if flat >= seg_flat + seg_len {
            x += advance(&text, block, segment.style, measure);
            seg_flat += seg_len;
        } else {
            // The caret is inside this segment: measure its prefix, past
            // the badge box's left edge if there is one — the label starts
            // inside the box, so the caret has to as well.
            let up_to = flat - seg_flat;
            let prefix: String = text.chars().take(up_to).collect();
            if segment.style.badge {
                x += theme::BADGE_PAD;
            }
            x += measure(&prefix, &text_style(block, segment.style));
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
        // The label sits inside its box; skip the left edge so a click
        // lands on the character the user aimed at.
        if segment.style.badge {
            cum += theme::BADGE_PAD;
        }
        for (ci, ch) in text.chars().enumerate() {
            let w = measure(&ch.to_string(), &text_style(block, segment.style));
            if x <= cum + w / 2.0 {
                pos = seg_flat + ci;
                break 'segments;
            }
            cum += w;
        }
        if segment.style.badge {
            cum += theme::BADGE_PAD;
        }
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
        let x = x_of_flat(block, line, line_start, flat, measure);
        (x, line.y + line.height / 2.0, line.height)
    }

    /// Nearest caret position for a click at (x, y) — y relative to content
    /// top. Style context follows the style-before rule.
    pub fn hit(&self, x: f32, y: f32, measure: &dyn Fn(&str, &TextStyle) -> f32) -> Caret {
        let block_idx = block_of_y(self, y);
        let layout_block = &self.blocks[block_idx];
        let line_idx = line_of_y(layout_block, y);
        caret_for_click(&self.source, layout_block, block_idx, line_idx, x, measure)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Inline, Text};

    /// Every glyph 10 wide, so line breaks are countable by hand.
    fn fake_measure(text: &str, style: &TextStyle) -> f32 {
        let _ = style;
        text.chars().count() as f32 * 10.0
    }

    fn doc_with(blocks: Vec<Block>) -> Document {
        let mut d = Document::new(std::path::Path::new("notes/t.md"));
        d.blocks = blocks;
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
    fn greedy_wrap_breaks_exactly_and_reecovers_source() {
        let d = doc_with(vec![para("aaa bbb ccc ddd eee")]);
        let laid = layout(&d, 140.0, &fake_measure);
        // "aaa bbb" = 70, + "ccc" = 110 <= 140, + " ddd" = 150 > 140.
        // The space before "ddd" belongs to line 0 ("aaa bbb ccc ").
        // Line 1: "ddd eee".
        assert_eq!(laid.blocks[0].lines.len(), 2);

        let block = &d.blocks[0];
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
        let block = &d.blocks[0];
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

        let block = &d.blocks[0];
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
                &d.blocks,
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
            let hit_flat = flat_of_caret(&d.blocks, hit);
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
        let h1_style = text_style(&d.blocks[0], Style::PLAIN);
        assert!(h1_style.weight > 0.0, "headings are always bold");

        // H4 matches body size (17.5) — bold is what distinguishes it.
        let h4 = Block::Heading {
            level: 4,
            content: vec![Inline::Text(Text {
                text: "Sub".into(),
                style: Style::PLAIN,
            })],
        };
        let h4_style = text_style(&h4, Style::PLAIN);
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
        let ts_para = text_style(&para, code);
        let ts_code = text_style(&code_block, code);
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
            Inline::Text(Text {
                text: "x".into(),
                style: Style::PLAIN,
            }),
        ]);
        // The box counts: bare glyphs would put `x` under the chip.
        let bare = fake_measure("PS", &text_style(&block, badge));
        assert_eq!(
            advance("PS", &block, badge, &fake_measure),
            bare + theme::BADGE_PAD * 2.0
        );
        assert_eq!(
            advance("PS", &block, Style::PLAIN, &fake_measure),
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
}
