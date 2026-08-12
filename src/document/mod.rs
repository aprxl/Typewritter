//! The document model: blocks, inline runs, the style-context caret, and
//! the editing operations on top of them. Pure model — no IO, no renderer.
//!
//! See `tasks/01-document-model.md` for the spec this implements, and
//! `docs/SPEC.md` §4.2 for the style-context machine.

use std::path::{Path, PathBuf};

pub mod layout;
pub mod markdown;

/// A document: an ordered list of blocks with a caret.
#[derive(Clone, PartialEq, Debug)]
pub struct Document {
    pub blocks: Vec<Block>, // invariant: never empty
    pub path: PathBuf,
    pub name: String,
    dirty: bool,
    pub caret: Caret,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Block {
    Paragraph(Vec<Inline>),
    /// A horizontal rule. It holds a single empty run so it satisfies the
    /// "every block has a run" invariant and the caret can rest on it like
    /// any other empty line; typing there turns it back into a paragraph
    /// (see `prune_runs`). The rule itself is drawn by the editor — this
    /// block has no content of its own.
    Divider(Vec<Inline>),
    Heading {
        level: u8,
        content: Vec<Inline>,
    }, // level 1..=4
    CodeLine {
        content: Vec<Inline>,
        first: bool,
        lang: Option<String>,
    }, // one line of a fenced code block
}

#[derive(Clone, PartialEq, Debug)]
pub enum Inline {
    Text(Text),
}

#[derive(Clone, PartialEq, Debug)]
pub struct Text {
    pub text: String,
    pub style: Style,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    /// A chip: the run's own text is the label, drawn small and boxed.
    /// Exclusive with every other flag — a badge is a whole visual unit,
    /// not a weight applied to prose.
    pub badge: bool,
    /// Marked text. Stacks with bold and italic; excluded by `code` and
    /// `badge`, which draw their own box and would fight it.
    pub highlight: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Caret {
    pub block: usize,  // index into blocks
    pub inline: usize, // index into the block's inline runs
    pub offset: usize, // char offset within the run's text
    pub style: Style,  // context: what typed text becomes (SPEC §4.2)
}

/// A character position inside one logical block.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct FlatPos {
    pub block: usize,
    pub offset: usize,
}

/// Half-open range over logical blocks. Newlines between blocks belong to the
/// range when its endpoints span blocks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FlatRange {
    pub start: FlatPos,
    pub end: FlatPos,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TextObject {
    InnerWord,
    AroundWord,
    InnerParagraph,
    AroundParagraph,
    InnerQuote,
    InnerParen,
    InnerHeading,
}

impl FlatRange {
    pub fn new(start: FlatPos, end: FlatPos) -> Self {
        Self { start, end }
    }

    pub fn normalized(self) -> Self {
        if (self.start.block, self.start.offset) <= (self.end.block, self.end.offset) {
            self
        } else {
            Self {
                start: self.end,
                end: self.start,
            }
        }
    }
}

impl Style {
    pub const PLAIN: Style = Style {
        bold: false,
        italic: false,
        code: false,
        badge: false,
        highlight: false,
    };

    /// Whether this run draws its own box, and so owns its whole extent:
    /// code spans and badges. Nothing else may be layered onto one.
    pub fn is_boxed(&self) -> bool {
        self.code || self.badge
    }

    pub fn is_plain(&self) -> bool {
        *self == Style::PLAIN
    }
}

impl Block {
    pub fn inlines(&self) -> &[Inline] {
        match self {
            Block::Paragraph(inlines)
            | Block::Divider(inlines)
            | Block::Heading {
                content: inlines, ..
            } => inlines,
            Block::CodeLine { content, .. } => content,
        }
    }

    pub fn inlines_mut(&mut self) -> &mut Vec<Inline> {
        match self {
            Block::Paragraph(inlines)
            | Block::Divider(inlines)
            | Block::Heading {
                content: inlines, ..
            } => inlines,
            Block::CodeLine { content, .. } => content,
        }
    }

    pub fn is_heading(&self) -> bool {
        matches!(self, Block::Heading { .. })
    }

    pub fn is_divider(&self) -> bool {
        matches!(self, Block::Divider(_))
    }

    pub fn is_code(&self) -> bool {
        matches!(self, Block::CodeLine { .. })
    }
}

impl Inline {
    fn text(&self) -> &str {
        match self {
            Inline::Text(t) => &t.text,
        }
    }

    fn text_mut(&mut self) -> &mut String {
        match self {
            Inline::Text(t) => &mut t.text,
        }
    }

    fn style(&self) -> Style {
        match self {
            Inline::Text(t) => t.style,
        }
    }

    fn set_style(&mut self, style: Style) {
        match self {
            Inline::Text(t) => t.style = style,
        }
    }
}

fn run_len(run: &Inline) -> usize {
    run.text().chars().count()
}

fn style_matches(style: Style, mask: Style) -> bool {
    (!mask.bold || style.bold)
        && (!mask.italic || style.italic)
        && (!mask.highlight || style.highlight)
}

/// The block the caret's placeholder-empty state lives in.
fn empty_block() -> Block {
    Block::Paragraph(vec![Inline::Text(Text {
        text: String::new(),
        style: Style::PLAIN,
    })])
}

/// A rule block, holding the one empty run every block must have.
fn divider_block() -> Block {
    Block::Divider(vec![Inline::Text(Text {
        text: String::new(),
        style: Style::PLAIN,
    })])
}

/// Inserts `s` at character offset `char_idx` of `text`.
fn insert_str(text: &mut String, char_idx: usize, s: &str) {
    let byte = text
        .char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(text.len());
    text.insert_str(byte, s);
}

/// Removes the character at `char_idx` of `text`.
fn remove_char_at(text: &mut String, char_idx: usize) {
    let (byte, ch) = text.char_indices().nth(char_idx).unwrap();
    text.replace_range(byte..byte + ch.len_utf8(), "");
}

/// Splits a run at character offset `at`, keeping the run's style on both
/// halves.
fn split_run(run: Inline, at: usize) -> (Inline, Inline) {
    match run {
        Inline::Text(t) => {
            let byte = t
                .text
                .char_indices()
                .nth(at)
                .map(|(b, _)| b)
                .unwrap_or(t.text.len());
            let (front, back) = t.text.split_at(byte);
            let style = t.style;
            (
                Inline::Text(Text {
                    text: front.to_string(),
                    style,
                }),
                Inline::Text(Text {
                    text: back.to_string(),
                    style,
                }),
            )
        }
    }
}

impl Document {
    /// A fresh document with one empty paragraph; name from the path.
    pub fn new(path: &Path) -> Document {
        Document {
            blocks: vec![empty_block()],
            path: path.to_path_buf(),
            name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string_lossy().into_owned()),
            dirty: false,
            caret: Caret {
                block: 0,
                inline: 0,
                offset: 0,
                style: Style::PLAIN,
            },
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Whitespace-split over all runs.
    pub fn word_count(&self) -> usize {
        self.blocks
            .iter()
            .flat_map(Block::inlines)
            .map(Inline::text)
            .map(|t| t.split_whitespace().count())
            .sum()
    }

    pub fn block_text(&self, block: usize) -> String {
        self.blocks[block]
            .inlines()
            .iter()
            .map(Inline::text)
            .collect()
    }

    pub fn block_len(&self, block: usize) -> usize {
        self.blocks[block].inlines().iter().map(run_len).sum()
    }

    pub fn caret_position(&self) -> FlatPos {
        FlatPos {
            block: self.caret.block,
            offset: self.caret_flat(self.caret.block),
        }
    }

    pub fn position(&self, block: usize, offset: usize) -> FlatPos {
        FlatPos {
            block: block.min(self.blocks.len().saturating_sub(1)),
            offset: offset.min(self.block_len(block.min(self.blocks.len().saturating_sub(1)))),
        }
    }

    pub fn set_flat_position(&mut self, position: FlatPos) {
        let position = self.position(position.block, position.offset);
        let (inline, offset) = self.flat_to_pos(position.block, position.offset);
        self.set_caret(position.block, inline, offset);
    }

    /// Returns text in a range, inserting logical newlines between blocks.
    pub fn range_text(&self, range: FlatRange) -> String {
        let range = range.normalized();
        let start = self.position(range.start.block, range.start.offset);
        let end = self.position(range.end.block, range.end.offset);
        if (start.block, start.offset) >= (end.block, end.offset) {
            return String::new();
        }
        let mut text = String::new();
        for block in start.block..=end.block {
            let block_text = self.block_text(block);
            let from = if block == start.block {
                start.offset
            } else {
                0
            };
            let to = if block == end.block {
                end.offset
            } else {
                block_text.chars().count()
            };
            text.extend(block_text.chars().skip(from).take(to.saturating_sub(from)));
            if block != end.block {
                text.push('\n');
            }
        }
        text
    }

    /// Toggle style flags over selected text. Boxed styles replace all other
    /// styling in the range; non-boxed styles never layer onto boxed runs.
    pub fn toggle_style_range(&mut self, range: FlatRange, mask: Style) {
        let range = range.normalized();
        let start = self.position(range.start.block, range.start.offset);
        let end = self.position(range.end.block, range.end.offset);
        if (start.block, start.offset) >= (end.block, end.offset) {
            return;
        }

        if mask.code || mask.badge {
            let boxed = Style {
                code: mask.code,
                badge: !mask.code && mask.badge,
                ..Style::PLAIN
            };
            let mut has_selected = false;
            let mut all_boxed = true;
            for block in start.block..=end.block {
                let from = if block == start.block {
                    start.offset
                } else {
                    0
                };
                let to = if block == end.block {
                    end.offset
                } else {
                    self.block_len(block)
                };
                for run in self.slice_runs(block, from, to) {
                    has_selected = true;
                    all_boxed &= run.style() == boxed;
                }
            }
            if !has_selected {
                return;
            }
            let target = if all_boxed { Style::PLAIN } else { boxed };
            for block in start.block..=end.block {
                let from = if block == start.block {
                    start.offset
                } else {
                    0
                };
                let to = if block == end.block {
                    end.offset
                } else {
                    self.block_len(block)
                };
                if from >= to {
                    continue;
                }
                let original = self.blocks[block].clone();
                let mut runs = self.slice_runs(block, 0, from);
                let mut selected = self.slice_runs(block, from, to);
                for run in &mut selected {
                    run.set_style(target);
                }
                runs.extend(selected);
                runs.extend(self.slice_runs(block, to, self.block_len(block)));
                self.blocks[block] = Self::block_with_runs(&original, runs);
            }
            self.dirty = true;
            self.enforce();
            self.refresh_context();
            return;
        }

        let mut eligible = false;
        let mut enable = false;
        for block in start.block..=end.block {
            let from = if block == start.block {
                start.offset
            } else {
                0
            };
            let to = if block == end.block {
                end.offset
            } else {
                self.block_len(block)
            };
            for run in self.slice_runs(block, from, to) {
                if !run.style().is_boxed() {
                    eligible = true;
                    enable |= !style_matches(run.style(), mask);
                }
            }
        }
        if !eligible {
            return;
        }

        for block in start.block..=end.block {
            let from = if block == start.block {
                start.offset
            } else {
                0
            };
            let to = if block == end.block {
                end.offset
            } else {
                self.block_len(block)
            };
            if from >= to {
                continue;
            }
            let original = self.blocks[block].clone();
            let mut runs = self.slice_runs(block, 0, from);
            let mut selected = self.slice_runs(block, from, to);
            for run in &mut selected {
                if !run.style().is_boxed() {
                    let mut style = run.style();
                    if mask.bold {
                        style.bold = enable;
                    }
                    if mask.italic {
                        style.italic = enable;
                    }
                    if mask.highlight {
                        style.highlight = enable;
                    }
                    run.set_style(style);
                }
            }
            runs.extend(selected);
            runs.extend(self.slice_runs(block, to, self.block_len(block)));
            self.blocks[block] = Self::block_with_runs(&original, runs);
        }
        self.dirty = true;
        self.enforce();
        self.refresh_context();
    }

    pub fn yank_range(&self, range: FlatRange) -> String {
        self.range_text(range)
    }

    pub fn line_range(&self, first: usize, last: usize) -> FlatRange {
        let first = first.min(self.blocks.len().saturating_sub(1));
        let last = last.min(self.blocks.len().saturating_sub(1)).max(first);
        FlatRange::new(
            self.position(first, 0),
            self.position(last, self.block_len(last)),
        )
    }

    fn slice_runs(&self, block: usize, start: usize, end: usize) -> Vec<Inline> {
        let mut result = Vec::new();
        let mut cursor = 0;
        for run in self.blocks[block].inlines() {
            let text = run.text();
            let run_start = cursor;
            let run_end = cursor + text.chars().count();
            let from = start.max(run_start).min(run_end);
            let to = end.max(run_start).min(run_end);
            if from < to {
                let value: String = text
                    .chars()
                    .skip(from - run_start)
                    .take(to - from)
                    .collect();
                result.push(Inline::Text(Text {
                    text: value,
                    style: run.style(),
                }));
            }
            cursor = run_end;
        }
        result
    }

    fn block_with_runs(block: &Block, mut runs: Vec<Inline>) -> Block {
        if runs.is_empty() {
            runs.push(Inline::Text(Text {
                text: String::new(),
                style: if block.is_code() {
                    Style {
                        code: true,
                        ..Style::PLAIN
                    }
                } else {
                    Style::PLAIN
                },
            }));
        }
        match block {
            Block::Paragraph(_) => Block::Paragraph(runs),
            Block::Divider(_) => Block::Divider(runs),
            Block::Heading { level, .. } => Block::Heading {
                level: *level,
                content: runs,
            },
            Block::CodeLine { first, lang, .. } => Block::CodeLine {
                content: runs,
                first: *first,
                lang: lang.clone(),
            },
        }
    }

    /// Delete range and leave caret at its start. Returns deleted text for the
    /// unnamed yank buffer.
    pub fn delete_range(&mut self, range: FlatRange) -> String {
        let range = range.normalized();
        let start = self.position(range.start.block, range.start.offset);
        let end = self.position(range.end.block, range.end.offset);
        if (start.block, start.offset) >= (end.block, end.offset) {
            return String::new();
        }
        let deleted = self.range_text(FlatRange::new(start, end));
        if start.block == end.block {
            let block = self.blocks[start.block].clone();
            let runs = self.slice_runs(start.block, 0, start.offset);
            let mut suffix = self.slice_runs(start.block, end.offset, self.block_len(start.block));
            let mut combined = runs;
            combined.append(&mut suffix);
            self.blocks[start.block] = Self::block_with_runs(&block, combined);
        } else {
            let first = self.blocks[start.block].clone();
            let mut runs = self.slice_runs(start.block, 0, start.offset);
            runs.extend(self.slice_runs(end.block, end.offset, self.block_len(end.block)));
            self.blocks.splice(
                start.block..=end.block,
                [Self::block_with_runs(&first, runs)],
            );
        }
        self.dirty = true;
        self.set_flat_position(start);
        self.enforce();
        deleted
    }

    pub fn replace_range(&mut self, range: FlatRange, text: &str) -> String {
        let deleted = self.delete_range(range);
        self.set_flat_position(range.normalized().start);
        self.insert_text(text);
        deleted
    }

    /// Delete complete logical blocks, as used by visual-line mode.
    pub fn delete_lines(&mut self, first: usize, last: usize) -> String {
        if self.blocks.is_empty() {
            return String::new();
        }
        let first = first.min(self.blocks.len() - 1);
        let last = last.min(self.blocks.len() - 1).max(first);
        let deleted = (first..=last)
            .map(|block| self.block_text(block))
            .collect::<Vec<_>>()
            .join("\n");
        self.blocks.drain(first..=last);
        if self.blocks.is_empty() {
            self.blocks.push(empty_block());
        }
        self.caret.block = first.min(self.blocks.len() - 1);
        self.caret.inline = 0;
        self.caret.offset = 0;
        self.caret.style = Style::PLAIN;
        self.dirty = true;
        self.enforce();
        deleted
    }

    pub fn open_change(&mut self, range: FlatRange) -> String {
        self.delete_range(range)
    }

    pub fn text_object_range(&self, object: TextObject) -> Option<FlatRange> {
        let caret = self.caret_position();
        let text = self.block_text(caret.block);
        let chars: Vec<char> = text.chars().collect();
        match object {
            TextObject::InnerWord | TextObject::AroundWord => {
                let is_word = |c: char| c.is_alphanumeric() || c == '_';
                if chars.is_empty() || caret.offset >= chars.len() {
                    return None;
                }
                let word = is_word(chars[caret.offset]);
                let mut start = caret.offset;
                while start > 0
                    && is_word(chars[start - 1]) == word
                    && !chars[start - 1].is_whitespace()
                {
                    start -= 1;
                }
                let mut end = caret.offset + 1;
                while end < chars.len()
                    && is_word(chars[end]) == word
                    && !chars[end].is_whitespace()
                {
                    end += 1;
                }
                if matches!(object, TextObject::AroundWord) {
                    while end < chars.len() && chars[end].is_whitespace() {
                        end += 1;
                    }
                    if end == caret.offset + 1 {
                        while start > 0 && chars[start - 1].is_whitespace() {
                            start -= 1;
                        }
                    }
                }
                Some(FlatRange::new(
                    self.position(caret.block, start),
                    self.position(caret.block, end),
                ))
            }
            TextObject::InnerQuote => {
                if chars.get(caret.offset).is_none_or(|&c| c == '"') {
                    return None;
                }
                let left = chars[..caret.offset].iter().rposition(|&c| c == '"')?;
                let right = chars[caret.offset..].iter().position(|&c| c == '"')? + caret.offset;
                Some(FlatRange::new(
                    self.position(caret.block, left + 1),
                    self.position(caret.block, right),
                ))
            }
            TextObject::InnerParen => {
                if chars
                    .get(caret.offset)
                    .is_none_or(|&c| matches!(c, '(' | ')'))
                {
                    return None;
                }
                let mut depth = 0;
                let mut left = None;
                for index in (0..=caret.offset.min(chars.len())).rev() {
                    match chars.get(index) {
                        Some(')') => depth += 1,
                        Some('(') if depth == 0 => {
                            left = Some(index);
                            break;
                        }
                        Some('(') => depth -= 1,
                        _ => {}
                    }
                }
                let left = left?;
                depth = 0;
                let mut right = None;
                for (index, ch) in chars.iter().enumerate().skip(left + 1) {
                    match ch {
                        '(' => depth += 1,
                        ')' if depth == 0 => {
                            right = Some(index);
                            break;
                        }
                        ')' => depth -= 1,
                        _ => {}
                    }
                }
                Some(FlatRange::new(
                    self.position(caret.block, left + 1),
                    self.position(caret.block, right?),
                ))
            }
            TextObject::InnerParagraph => Some(FlatRange::new(
                self.position(caret.block, 0),
                self.position(caret.block, self.block_len(caret.block)),
            )),
            TextObject::AroundParagraph => {
                if caret.block + 1 < self.blocks.len() {
                    Some(FlatRange::new(
                        self.position(caret.block, 0),
                        self.position(caret.block + 1, 0),
                    ))
                } else if caret.block > 0 {
                    Some(FlatRange::new(
                        self.position(caret.block - 1, self.block_len(caret.block - 1)),
                        self.position(caret.block, self.block_len(caret.block)),
                    ))
                } else {
                    Some(FlatRange::new(
                        self.position(caret.block, 0),
                        self.position(caret.block, self.block_len(caret.block)),
                    ))
                }
            }
            TextObject::InnerHeading => {
                let mut heading = None;
                for index in (0..=caret.block).rev() {
                    if let Block::Heading { level, .. } = self.blocks[index] {
                        heading = Some((index, level));
                        break;
                    }
                }
                let (start, level) = heading?;
                let end = (start + 1..self.blocks.len())
                    .find(|&index| matches!(self.blocks[index], Block::Heading { level: next, .. } if next <= level))
                    .unwrap_or(self.blocks.len());
                Some(FlatRange::new(
                    self.position(start, 0),
                    self.position(end.saturating_sub(1), self.block_len(end.saturating_sub(1))),
                ))
            }
        }
    }

    // ---- internal geometry helpers --------------------------------------

    fn block_flat_len(&self, block: usize) -> usize {
        self.blocks[block].inlines().iter().map(run_len).sum()
    }

    /// The caret's flat position within its block, clamped.
    fn caret_flat(&self, block: usize) -> usize {
        let runs = self.blocks[block].inlines();
        let inline = self.caret.inline.min(runs.len().saturating_sub(1));
        let prefix: usize = runs[..inline].iter().map(run_len).sum();
        let offset = if runs.is_empty() {
            0
        } else {
            self.caret.offset.min(run_len(&runs[inline]))
        };
        (prefix + offset).min(self.block_flat_len(block))
    }

    /// `(inline, offset)` for a flat position, clamped to the block's end.
    fn flat_to_pos(&self, block: usize, flat: usize) -> (usize, usize) {
        let runs = self.blocks[block].inlines();
        let mut pos = 0;
        for (i, run) in runs.iter().enumerate() {
            let len = run_len(run);
            if flat < pos + len {
                return (i, flat - pos);
            }
            pos += len;
        }
        let last = runs.len().saturating_sub(1);
        (last, run_len(&runs[last]))
    }

    /// Style of the character at `flat`, or `None` at the end of the block.
    fn style_at(&self, block: usize, flat: usize) -> Option<Style> {
        let runs = self.blocks[block].inlines();
        let mut pos = 0;
        for run in runs {
            let len = run_len(run);
            if flat < pos + len {
                return Some(run.style());
            }
            pos += len;
        }
        None
    }

    /// Style of the character before `flat`, or `None` at the block start.
    fn style_before(&self, block: usize, flat: usize) -> Option<Style> {
        if flat > 0 {
            self.style_at(block, flat - 1)
        } else {
            None
        }
    }

    /// Repairs the caret into valid bounds. `blocks` is never empty.
    fn clamp_caret(&mut self) {
        if self.blocks.is_empty() {
            self.blocks.push(empty_block());
        }
        let block = self.caret.block.min(self.blocks.len() - 1);
        self.caret.block = block;
        let runs = self.blocks[block].inlines();
        let inline = self.caret.inline.min(runs.len().saturating_sub(1));
        self.caret.inline = inline;
        let len = run_len(&runs[inline]);
        self.caret.offset = self.caret.offset.min(len);
    }

    /// Enforces the invariants: non-empty blocks, every block has ≥1 run,
    /// no empty run outside the placeholder, caret in bounds.
    fn enforce(&mut self) {
        self.prune_runs();
        self.clamp_caret();
        debug_assert!(self.invariants_hold());
    }

    /// Drops empty runs; an all-empty block becomes a placeholder of the same
    /// kind (a Heading stays a heading, so `set_heading` on an empty line
    /// survives and can be typed into — the invariant that an empty document
    /// is one empty Paragraph is only about `Document::new`).
    fn prune_runs(&mut self) {
        for block in &mut self.blocks {
            block.inlines_mut().retain(|r| !r.text().is_empty());
            // A rule holds no text. Typing on one turns it into prose —
            // enforced centrally here, so every edit path gets it without
            // a special case of its own.
            if block.is_divider() && block.inlines().iter().any(|r| !r.text().is_empty()) {
                *block = Block::Paragraph(std::mem::take(block.inlines_mut()));
            }
            if block.inlines().is_empty() {
                *block = match block {
                    Block::Heading { level, .. } => Block::Heading {
                        level: *level,
                        content: vec![Inline::Text(Text {
                            text: String::new(),
                            style: Style::PLAIN,
                        })],
                    },
                    Block::CodeLine { first, lang, .. } => Block::CodeLine {
                        content: vec![Inline::Text(Text {
                            text: String::new(),
                            style: Style {
                                code: true,
                                ..Style::PLAIN
                            },
                        })],
                        first: *first,
                        lang: lang.clone(),
                    },
                    Block::Divider(_) => divider_block(),
                    Block::Paragraph(_) => empty_block(),
                };
            }
        }
    }

    fn invariants_hold(&self) -> bool {
        if self.blocks.is_empty() {
            return false;
        }
        for block in &self.blocks {
            if block.inlines().is_empty() {
                return false;
            }
            for (i, run) in block.inlines().iter().enumerate() {
                if run.text().is_empty() && (block.inlines().len() > 1 || i != 0) {
                    return false;
                }
            }
        }
        let block = &self.blocks[self.caret.block];
        if self.caret.inline >= block.inlines().len() {
            return false;
        }
        self.caret.offset <= run_len(&block.inlines()[self.caret.inline])
    }

    // ---- caret movement (never dirt) ------------------------------------

    /// Click placement / jump. Clamps into bounds; the style context is the
    /// style-before rule (§3).
    pub fn set_caret(&mut self, block: usize, inline: usize, offset: usize) {
        self.clamp_caret();
        let b = block.min(self.blocks.len() - 1);
        self.caret.block = b;
        let runs = self.blocks[b].inlines();
        let i = inline.min(runs.len().saturating_sub(1));
        let len = run_len(&runs[i]);
        self.caret.inline = i;
        self.caret.offset = offset.min(len);
        self.caret.style = self
            .style_before(b, self.caret_flat(b))
            .unwrap_or(Style::PLAIN);
    }

    pub fn move_right(&mut self) {
        self.clamp_caret();
        let b = self.caret.block;
        // A pop is only legitimate at a run/block seam (offset at the very
        // start or end of the current run). Off-seam, `before` and `after`
        // are necessarily the same run's style, so a caret.style mismatch
        // there is a stray context (e.g. an armed-but-unused toggle) with
        // no boundary to pop — it must not swallow the keypress.
        let o = self.caret.offset;
        let run_len_i = run_len(&self.blocks[b].inlines()[self.caret.inline]);
        let at_seam = o == 0 || o == run_len_i;
        let after = self.style_at(b, self.caret_flat(b));
        let target = after.unwrap_or(Style::PLAIN);
        if at_seam && self.caret.style != target {
            self.caret.style = target; // pop out of the run — no movement
            return;
        }
        let flat = self.caret_flat(b);
        if flat < self.block_flat_len(b) {
            let (i, o) = self.flat_to_pos(b, flat + 1);
            self.caret.inline = i;
            self.caret.offset = o;
            self.caret.style = self
                .style_before(b, self.caret_flat(b))
                .unwrap_or(Style::PLAIN);
        } else if b + 1 < self.blocks.len() {
            self.caret.block = b + 1;
            self.caret.inline = 0;
            self.caret.offset = 0;
            self.caret.style = Style::PLAIN;
        }
    }

    pub fn move_left(&mut self) {
        self.clamp_caret();
        let b = self.caret.block;
        // Mirrors move_right: only pop at a genuine run/block seam.
        let o = self.caret.offset;
        let run_len_i = run_len(&self.blocks[b].inlines()[self.caret.inline]);
        let at_seam = o == 0 || o == run_len_i;
        let before = self.style_before(b, self.caret_flat(b));
        let target = before.unwrap_or(Style::PLAIN);
        if at_seam && self.caret.style != target {
            self.caret.style = target; // re-enter the run to the left
            return;
        }
        let flat = self.caret_flat(b);
        if flat > 0 {
            let (i, o) = self.flat_to_pos(b, flat - 1);
            self.caret.inline = i;
            self.caret.offset = o;
            self.caret.style = self.style_at(b, self.caret_flat(b)).unwrap_or(Style::PLAIN);
        } else if b > 0 {
            let prev = b - 1;
            let (i, o) = self.flat_to_pos(prev, self.block_flat_len(prev));
            self.caret.block = prev;
            self.caret.inline = i;
            self.caret.offset = o;
            self.caret.style = Style::PLAIN;
        }
    }

    /// Logical start of the block's flat text, context per the style-before
    /// rule (PLAIN at the start).
    pub fn move_home(&mut self) {
        self.clamp_caret();
        self.caret.inline = 0;
        self.caret.offset = 0;
        self.caret.style = Style::PLAIN;
    }

    /// Logical end of the block, context = style of the last character.
    pub fn move_end(&mut self) {
        self.clamp_caret();
        let b = self.caret.block;
        let len = self.block_flat_len(b);
        let (i, o) = self.flat_to_pos(b, len);
        self.caret.inline = i;
        self.caret.offset = o;
        self.caret.style = self.style_before(b, len).unwrap_or(Style::PLAIN);
    }

    // ---- edits (content changes set `dirty`) ----------------------------

    pub fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.clamp_caret();
        let len = text.chars().count();
        let s = self.caret.style;
        let b = self.caret.block;
        let i = self.caret.inline;
        let o = self.caret.offset;
        let flat = self.caret_flat(b);

        let runs = self.blocks[b].inlines();
        let placeholder = runs.len() == 1 && runs[0].text().is_empty();
        let li = run_len(&runs[i]);
        let left_style = if o > 0 {
            Some(runs[i].style())
        } else if i > 0 {
            Some(runs[i - 1].style())
        } else {
            None
        };
        let right_style = if o < li {
            Some(runs[i].style())
        } else if i + 1 < runs.len() {
            Some(runs[i + 1].style())
        } else {
            None
        };

        if placeholder {
            let runs = self.blocks[b].inlines_mut();
            runs[0].text_mut().push_str(text);
            runs[0].set_style(s);
        } else if left_style == Some(s) {
            let runs = self.blocks[b].inlines_mut();
            if o > 0 {
                insert_str(runs[i].text_mut(), o, text);
            } else {
                runs[i - 1].text_mut().push_str(text);
            }
        } else if right_style == Some(s) {
            let runs = self.blocks[b].inlines_mut();
            if o < li {
                insert_str(runs[i].text_mut(), o, text);
            } else {
                insert_str(runs[i + 1].text_mut(), 0, text);
            }
        } else {
            // Splice a new run at the caret, splitting the current run.
            let (prefix, suffix) = split_run(self.blocks[b].inlines_mut().remove(i), o);
            let new_run = Inline::Text(Text {
                text: text.to_string(),
                style: s,
            });
            let runs = self.blocks[b].inlines_mut();
            runs.splice(i..i, [prefix, new_run, suffix]);
        }

        // Caret moves past the inserted text; style context unchanged. The
        // caret's desired flat offset is `flat + len`; it is re-derived from
        // that *after* `enforce` drops any empty run the splice created (an
        // insert at a run edge leaves a zero-length trash run whose position
        // would otherwise corrupt the caret).
        let target = flat + len;
        self.dirty = true;
        self.enforce();
        let (ni, no) = self.flat_to_pos(b, target.min(self.block_flat_len(b)));
        self.caret.inline = ni;
        self.caret.offset = no;
    }

    pub fn backspace(&mut self) {
        self.clamp_caret();
        let b = self.caret.block;
        let i = self.caret.inline;
        let o = self.caret.offset;
        let before = self.caret_flat(b);
        if o > 0 {
            let runs = self.blocks[b].inlines_mut();
            remove_char_at(runs[i].text_mut(), o - 1);
        } else if i > 0 {
            let runs = self.blocks[b].inlines_mut();
            let prev_len = run_len(&runs[i - 1]);
            remove_char_at(runs[i - 1].text_mut(), prev_len - 1);
        } else if b > 0 {
            self.merge_into_previous();
            self.dirty = true;
            self.enforce();
            self.refresh_context();
            return;
        } else {
            return; // start of document
        }
        // One char vanished before the caret: it sits exactly one char back.
        let (ni, no) = self.flat_to_pos(b, before.saturating_sub(1));
        self.caret.inline = ni;
        self.caret.offset = no;
        self.dirty = true;
        self.enforce();
        self.refresh_context();
    }

    /// Append this block's runs onto the previous block's; caret at the
    /// junction; delete this block.
    fn merge_into_previous(&mut self) {
        let b = self.caret.block;
        let prev = b - 1;
        let junction = self.block_flat_len(prev);
        let mut taken = {
            let runs = self.blocks[b].inlines_mut();
            std::mem::take(runs)
        };
        self.blocks.remove(b);
        self.blocks[prev].inlines_mut().append(&mut taken);
        self.caret.block = prev;
        let (i, o) = self.flat_to_pos(prev, junction);
        self.caret.inline = i;
        self.caret.offset = o;
    }

    pub fn delete_forward(&mut self) {
        self.clamp_caret();
        let b = self.caret.block;
        let i = self.caret.inline;
        let o = self.caret.offset;
        let runs = self.blocks[b].inlines();
        let li = run_len(&runs[i]);
        if o < li {
            let runs = self.blocks[b].inlines_mut();
            remove_char_at(runs[i].text_mut(), o);
        } else if i + 1 < runs.len() {
            let runs = self.blocks[b].inlines_mut();
            remove_char_at(runs[i + 1].text_mut(), 0);
        } else if b + 1 < self.blocks.len() {
            self.merge_block_into_next();
        } else {
            return; // end of document
        }
        self.dirty = true;
        self.enforce();
        self.refresh_context();
    }

    /// Append the next block's runs onto this one; delete the next block.
    /// The caret stays where it is (the junction).
    fn merge_block_into_next(&mut self) {
        let b = self.caret.block;
        let next = b + 1;
        let mut taken = {
            let runs = self.blocks[next].inlines_mut();
            std::mem::take(runs)
        };
        self.blocks.remove(next);
        self.blocks[b].inlines_mut().append(&mut taken);
    }

    pub fn newline(&mut self) {
        self.clamp_caret();
        let b = self.caret.block;
        let i = self.caret.inline;
        let o = self.caret.offset;
        let is_code = self.blocks[b].is_code();
        let (prefix, suffix) = split_run(self.blocks[b].inlines_mut().remove(i), o);
        let mut taken = Vec::new();
        let runs = self.blocks[b].inlines_mut();
        taken.extend(runs.drain(i..));
        runs.push(prefix);

        let mut new_content = vec![suffix];
        new_content.extend(taken);
        let new_block = if is_code {
            Block::CodeLine {
                content: new_content,
                first: false,
                lang: None,
            }
        } else {
            Block::Paragraph(new_content)
        };
        self.blocks.insert(b + 1, new_block);

        self.dirty = true;
        self.caret.block = b + 1;
        self.caret.inline = 0;
        self.caret.offset = 0;
        // Style context preserved: you keep typing in the same style.
        self.enforce();
    }

    /// vim `x`: delete the char at the caret's flat position — the char under
    /// the cursor, or the next run's first char when the caret sits in the
    /// gap at a run's end. Nothing at a block end or in an empty block. Never
    /// merges.
    pub fn delete_char(&mut self) {
        self.clamp_caret();
        let b = self.caret.block;
        let flat = self.caret_flat(b);
        if flat >= self.block_flat_len(b) {
            return;
        }
        let (i, o) = self.flat_to_pos(b, flat);
        let runs = self.blocks[b].inlines_mut();
        remove_char_at(runs[i].text_mut(), o);
        self.dirty = true;
        self.enforce();
        self.refresh_context();
    }

    /// vim `dd`: remove the caret's whole block; the only block becomes an
    /// empty paragraph. Caret to the same-or-clamped block index, at its
    /// first non-blank char, context PLAIN.
    pub fn delete_line(&mut self) {
        self.clamp_caret();
        let b = self.caret.block;
        if self.blocks.len() == 1 {
            self.blocks[0] = empty_block();
            self.caret.block = 0;
            self.caret.inline = 0;
            self.caret.offset = 0;
        } else {
            self.blocks.remove(b);
            let landing = b.min(self.blocks.len() - 1);
            self.caret.block = landing;
            self.caret.inline = 0;
            self.caret.offset = 0;
            let mut prefix = 0;
            for run in self.blocks[landing].inlines() {
                let lead = run.text().chars().take_while(|c| c.is_whitespace()).count();
                if lead < run_len(run) {
                    let (i, o) = self.flat_to_pos(landing, prefix + lead);
                    self.caret.inline = i;
                    self.caret.offset = o;
                    break;
                }
                prefix += run_len(run);
            }
        }
        self.caret.style = Style::PLAIN;
        self.dirty = true;
        self.enforce();
    }

    /// vim `o`: an empty Paragraph below the caret's block.
    pub fn open_below(&mut self) {
        self.clamp_caret();
        let b = self.caret.block;
        self.blocks.insert(b + 1, empty_block());
        self.caret.block = b + 1;
        self.caret.inline = 0;
        self.caret.offset = 0;
        self.caret.style = Style::PLAIN;
        self.dirty = true;
        self.enforce();
    }

    /// vim `O`: an empty Paragraph above the caret's block.
    pub fn open_above(&mut self) {
        self.clamp_caret();
        let b = self.caret.block;
        self.blocks.insert(b, empty_block());
        self.caret.block = b;
        self.caret.inline = 0;
        self.caret.offset = 0;
        self.caret.style = Style::PLAIN;
        self.dirty = true;
        self.enforce();
    }

    /// Flip `bold` on the pending context. Not a content edit; no dirty.
    pub fn toggle_bold(&mut self) {
        if self.caret.style.is_boxed() {
            return;
        }
        self.caret.style.bold = !self.caret.style.bold;
    }

    /// Flip `italic` on the pending context. Not a content edit; no dirty.
    pub fn toggle_italic(&mut self) {
        if self.caret.style.is_boxed() {
            return;
        }
        self.caret.style.italic = !self.caret.style.italic;
    }

    /// Flip `code` on the pending context. Not a content edit; no dirty.
    pub fn toggle_code(&mut self) {
        let on = !self.caret.style.code;
        self.caret.style = Style {
            code: on,
            ..Style::PLAIN
        };
    }

    /// Flip `badge` on the pending context: what is typed next becomes the
    /// chip's label. Not a content edit; no dirty.
    pub fn toggle_badge(&mut self) {
        let on = !self.caret.style.badge;
        self.caret.style = Style {
            badge: on,
            ..Style::PLAIN
        };
    }

    /// Flip `highlight` on the pending context. Not a content edit; no
    /// dirty. A boxed context ignores it — see [`Style::is_boxed`].
    pub fn toggle_highlight(&mut self) {
        if self.caret.style.is_boxed() {
            return;
        }
        self.caret.style.highlight = !self.caret.style.highlight;
    }

    /// Convert the caret's block to/from a code line. Unlike `set_heading`,
    /// this collapses the block's runs into one, since a code line never has
    /// sub-formatting — no bold spans, no nested styles, matching the "no
    /// syntax highlighting" scope cut for this feature.
    pub fn set_code(&mut self, on: bool) {
        self.clamp_caret();
        let b = self.caret.block;
        let flat_text: String = self.blocks[b].inlines().iter().map(Inline::text).collect();
        let style = if on {
            Style {
                code: true,
                ..Style::PLAIN
            }
        } else {
            Style::PLAIN
        };
        let run = Inline::Text(Text {
            text: flat_text,
            style,
        });
        self.blocks[b] = if on {
            Block::CodeLine {
                content: vec![run],
                first: true,
                lang: None,
            }
        } else {
            Block::Paragraph(vec![run])
        };
        self.dirty = true;
        self.enforce();
    }

    /// Convert the caret's block: `Some(1..=4)` → Heading, `None` →
    /// Paragraph. Content runs preserved.
    pub fn set_heading(&mut self, level: Option<u8>) {
        self.clamp_caret();
        let b = self.caret.block;
        if let Some(level) = level {
            if !(1..=4).contains(&level) {
                return;
            }
            let mut inlines = std::mem::take(self.blocks[b].inlines_mut());
            if self.blocks[b].is_code() {
                for run in &mut inlines {
                    run.set_style(Style {
                        code: false,
                        ..run.style()
                    });
                }
            }
            self.blocks[b] = Block::Heading {
                level,
                content: inlines,
            };
        } else {
            let mut inlines = std::mem::take(self.blocks[b].inlines_mut());
            if self.blocks[b].is_code() {
                for run in &mut inlines {
                    run.set_style(Style {
                        code: false,
                        ..run.style()
                    });
                }
            }
            self.blocks[b] = Block::Paragraph(inlines);
        }
        self.dirty = true;
        self.enforce();
    }

    /// Puts a rule below the caret's block and leaves the caret on a fresh
    /// empty paragraph after it — a rule is a separator you keep writing
    /// past, never a place to land. An empty block is replaced rather than
    /// pushed down, so a rule on a blank line does not leave a gap above
    /// itself.
    pub fn insert_divider(&mut self) {
        self.clamp_caret();
        let b = self.caret.block;
        let at = if self.block_len(b) == 0 {
            self.blocks[b] = divider_block();
            b
        } else {
            self.blocks.insert(b + 1, divider_block());
            b + 1
        };
        self.blocks.insert(at + 1, empty_block());
        self.set_caret(at + 1, 0, 0);
        self.dirty = true;
        self.enforce();
    }

    /// After a deletion the context is the style of the char now before the
    /// caret (PLAIN at block start).
    fn refresh_context(&mut self) {
        let b = self.caret.block;
        self.caret.style = self
            .style_before(b, self.caret_flat(b))
            .unwrap_or(Style::PLAIN);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> Document {
        Document::new(Path::new("notes/lecture-01.md"))
    }

    fn runs(block: &Block) -> Vec<(String, Style)> {
        block
            .inlines()
            .iter()
            .map(|r| (r.text().to_string(), r.style()))
            .collect()
    }

    fn text_of_block(doc: &Document, block: usize) -> String {
        doc.blocks[block]
            .inlines()
            .iter()
            .map(Inline::text)
            .collect()
    }

    fn bold() -> Style {
        Style {
            bold: true,
            ..Style::PLAIN
        }
    }

    fn code_style() -> Style {
        Style {
            code: true,
            ..Style::PLAIN
        }
    }

    fn plain_run(t: &str) -> Inline {
        Inline::Text(Text {
            text: t.into(),
            style: Style::PLAIN,
        })
    }

    fn bold_run(t: &str) -> Inline {
        Inline::Text(Text {
            text: t.into(),
            style: bold(),
        })
    }

    /// The §2 invariants: blocks never empty, every block has ≥1 run, no
    /// empty run outside the placeholder, caret in bounds.
    fn assert_invariants(d: &Document) {
        assert!(!d.blocks.is_empty());
        for block in &d.blocks {
            assert!(!block.inlines().is_empty());
            for (i, run) in block.inlines().iter().enumerate() {
                assert!(!(run.text().is_empty() && (block.inlines().len() > 1 || i != 0)));
            }
        }
        assert!(d.caret.block < d.blocks.len());
        let block_runs = d.blocks[d.caret.block].inlines();
        assert!(d.caret.inline < block_runs.len());
        assert!(d.caret.offset <= run_len(&block_runs[d.caret.inline]));
    }

    #[test]
    fn new_document_is_one_empty_paragraph() {
        let d = doc();
        assert_eq!(d.blocks.len(), 1);
        assert!(matches!(d.blocks[0], Block::Paragraph(ref r) if r.len() == 1));
        assert!(d.blocks[0].inlines()[0].text().is_empty());
        assert_eq!(d.blocks[0].inlines()[0].style(), Style::PLAIN);
        assert!(!d.is_dirty());
        assert_eq!(d.name, "lecture-01.md");
        assert_eq!(
            d.caret,
            Caret {
                block: 0,
                inline: 0,
                offset: 0,
                style: Style::PLAIN
            }
        );
    }

    #[test]
    fn word_count_splits_whitespace_over_all_runs() {
        let mut d = doc();
        d.blocks = vec![
            Block::Paragraph(vec![
                Inline::Text(Text {
                    text: "hello world".into(),
                    style: Style::PLAIN,
                }),
                Inline::Text(Text {
                    text: "  our   ".into(),
                    style: Style::PLAIN,
                }),
            ]),
            Block::Heading {
                level: 1,
                content: vec![Inline::Text(Text {
                    text: "big note".into(),
                    style: Style::PLAIN,
                })],
            },
        ];
        assert_eq!(d.word_count(), 5);
    }

    #[test]
    fn style_plain_helpers() {
        assert!(Style::PLAIN.is_plain());
        assert!(
            !Style {
                bold: true,
                ..Style::PLAIN
            }
            .is_plain()
        );
        assert_eq!(Style::default(), Style::PLAIN);
    }

    #[test]
    fn block_kind_helpers() {
        assert!(!Block::Paragraph(vec![]).is_heading());
        assert!(
            Block::Heading {
                level: 2,
                content: vec![]
            }
            .is_heading()
        );
        assert!(Block::Divider(vec![]).is_divider());
    }

    #[test]
    fn insert_into_empty_block_makes_one_real_run() {
        let mut d = doc();
        d.insert_text("abc");
        let block = &d.blocks[0];
        assert_eq!(runs(block), vec![("abc".into(), Style::PLAIN)]);
        assert_eq!(d.caret.offset, 3);
        assert!(d.is_dirty());
    }

    #[test]
    fn insert_into_placeholder_uses_caret_context() {
        let mut d = doc();
        d.toggle_bold();
        d.insert_text("x");
        assert_eq!(
            runs(&d.blocks[0]),
            vec![(
                "x".into(),
                Style {
                    bold: true,
                    ..Style::PLAIN
                }
            )]
        );
    }

    #[test]
    fn insert_merges_into_left_same_style_run() {
        let mut d = doc();
        d.blocks[0] = Block::Paragraph(vec![plain_run("ab"), bold_run("cd"), plain_run("ef")]);
        // Inside the bold run at its end; set_caret's style-before rule
        // gives bold context.
        d.set_caret(0, 1, 2);
        assert_eq!(d.caret.style, bold());
        d.insert_text("X");
        assert_eq!(
            runs(&d.blocks[0]),
            vec![
                ("ab".into(), Style::PLAIN),
                ("cdX".into(), bold()),
                ("ef".into(), Style::PLAIN)
            ]
        );
        assert_eq!((d.caret.inline, d.caret.offset), (2, 0));
        assert!(d.is_dirty());
        assert_invariants(&d);
    }

    #[test]
    fn insert_merges_into_right_same_style_run() {
        let mut d = doc();
        d.blocks[0] = Block::Paragraph(vec![plain_run("ab"), bold_run("cd")]);
        d.set_caret(0, 0, 2); // after "ab", before-style is plain
        d.toggle_bold(); // context becomes the right run's bold
        d.insert_text("X");
        assert_eq!(
            runs(&d.blocks[0]),
            vec![("ab".into(), Style::PLAIN), ("Xcd".into(), bold())]
        );
        assert_eq!((d.caret.inline, d.caret.offset), (1, 1));
        assert_invariants(&d);
    }

    #[test]
    fn insert_bold_context_between_plain_runs_splices_new_run() {
        let mut d = doc();
        d.blocks[0] = Block::Paragraph(vec![plain_run("ab"), plain_run("ef")]);
        d.set_caret(0, 0, 2); // boundary between the plain runs
        d.toggle_bold();
        d.insert_text("X");
        assert_eq!(
            runs(&d.blocks[0]),
            vec![
                ("ab".into(), Style::PLAIN),
                ("X".into(), bold()),
                ("ef".into(), Style::PLAIN)
            ]
        );
        assert_eq!((d.caret.inline, d.caret.offset), (2, 0));
        assert_invariants(&d);
    }

    #[test]
    fn insert_bold_at_end_of_line_keeps_caret_after_the_character() {
        // The splice creates an empty trash run at the split; the caret must
        // land *after* the inserted bold char regardless, not at its start.
        let mut d = doc();
        d.blocks[0] = Block::Paragraph(vec![plain_run("hello")]);
        d.set_caret(0, 0, 5);
        d.toggle_bold();
        d.insert_text("X");
        assert_eq!(
            runs(&d.blocks[0]),
            vec![("hello".into(), Style::PLAIN), ("X".into(), bold())]
        );
        assert_eq!((d.caret.inline, d.caret.offset), (1, 1), "caret is after X");
        assert_eq!(
            d.caret.style,
            bold(),
            "context stays bold — keep typing bold"
        );
        assert_invariants(&d);
    }

    #[test]
    fn set_heading_on_empty_line_stays_a_heading_and_types_into_it() {
        let mut d = doc(); // one empty paragraph placeholder
        d.set_heading(Some(1));
        assert!(
            matches!(d.blocks[0], Block::Heading { level: 1, .. }),
            "an empty line can become a heading"
        );
        d.insert_text("Title");
        assert!(
            matches!(d.blocks[0], Block::Heading { level: 1, .. }),
            "typing into it must keep the heading kind"
        );
        assert_eq!(runs(&d.blocks[0]), vec![("Title".into(), Style::PLAIN)]);
        assert_eq!((d.caret.inline, d.caret.offset), (0, 5));
        assert_invariants(&d);
    }

    #[test]
    fn set_heading_on_empty_line_body_round_trips() {
        let mut d = doc();
        d.set_heading(Some(2));
        assert!(matches!(d.blocks[0], Block::Heading { level: 2, .. }));
        d.set_heading(None);
        assert!(!d.blocks[0].is_heading());
        assert_invariants(&d);
    }

    #[test]
    fn level_four_heading_is_a_heading() {
        let mut d = doc();
        d.set_heading(Some(4));
        assert!(matches!(d.blocks[0], Block::Heading { level: 4, .. }));
        assert_invariants(&d);
    }

    #[test]
    fn backspace_mid_run() {
        let mut d = doc();
        d.insert_text("abc");
        d.set_caret(0, 0, 2);
        d.backspace();
        assert_eq!(runs(&d.blocks[0]), vec![("ac".into(), Style::PLAIN)]);
        assert_eq!((d.caret.inline, d.caret.offset), (0, 1));
        assert_invariants(&d);
    }

    #[test]
    fn backspace_at_run_boundary_deletes_previous_run_last_char() {
        let mut d = doc();
        d.blocks[0] = Block::Paragraph(vec![plain_run("ab"), plain_run("cd")]);
        d.set_caret(0, 1, 0);
        d.backspace();
        assert_eq!(text_of_block(&d, 0), "acd");
        assert_eq!((d.caret.inline, d.caret.offset), (1, 0));
        assert_invariants(&d);
    }

    #[test]
    fn backspace_at_block_start_merges_blocks() {
        let mut d = doc();
        d.blocks = vec![
            Block::Paragraph(vec![plain_run("hello")]),
            Block::Paragraph(vec![bold_run("world")]),
        ];
        d.set_caret(1, 0, 0);
        d.backspace();
        assert_eq!(d.blocks.len(), 1);
        assert_eq!(text_of_block(&d, 0), "helloworld");
        // Caret at the junction: end of the first block, start of its runs.
        assert_eq!((d.caret.block, d.caret.inline, d.caret.offset), (0, 1, 0));
        assert_eq!(d.caret.style, Style::PLAIN);
        assert_invariants(&d);
    }

    #[test]
    fn backspace_at_document_start_is_noop() {
        let mut d = doc();
        d.backspace();
        assert_eq!(text_of_block(&d, 0), "");
        assert!(!d.is_dirty());
        assert_invariants(&d);
    }

    #[test]
    fn newline_mid_paragraph_splits_runs_and_styles() {
        let mut d = doc();
        d.blocks[0] = Block::Paragraph(vec![plain_run("ab"), bold_run("cd")]);
        d.set_caret(0, 1, 1); // inside the bold run
        d.newline();
        assert_eq!(
            runs(&d.blocks[0]),
            vec![("ab".into(), Style::PLAIN), ("c".into(), bold())]
        );
        assert_eq!(runs(&d.blocks[1]), vec![("d".into(), bold())]);
        assert_eq!(d.caret.block, 1);
        assert_eq!((d.caret.inline, d.caret.offset), (0, 0));
        assert_eq!(d.caret.style, bold()); // context preserved
        assert_invariants(&d);
    }

    #[test]
    fn newline_at_end_of_heading_creates_paragraph() {
        let mut d = doc();
        d.blocks[0] = Block::Heading {
            level: 1,
            content: vec![plain_run("title")],
        };
        d.set_caret(0, 0, 5);
        d.newline();
        assert!(d.blocks[0].is_heading());
        assert_eq!(text_of_block(&d, 0), "title");
        assert!(!d.blocks[1].is_heading());
        assert_eq!(text_of_block(&d, 1), "");
        assert_eq!(d.caret.block, 1);
        assert_eq!((d.caret.inline, d.caret.offset), (0, 0));
        assert_invariants(&d);
    }

    #[test]
    fn delete_line_on_only_block_leaves_empty_paragraph() {
        let mut d = doc();
        d.insert_text("abc");
        d.set_heading(Some(1));
        d.delete_line();
        assert_eq!(d.blocks.len(), 1);
        assert!(!d.blocks[0].is_heading());
        assert_eq!(text_of_block(&d, 0), "");
        assert_eq!(d.caret.block, 0);
        assert_eq!((d.caret.inline, d.caret.offset), (0, 0));
        assert!(d.caret.style.is_plain());
        assert_invariants(&d);
    }

    #[test]
    fn delete_line_lands_on_first_non_blank_char() {
        let mut d = doc();
        d.insert_text("first");
        d.newline();
        d.insert_text("  indented");
        d.set_caret(0, 0, 0); // delete block 0
        d.delete_line();
        assert_eq!(d.blocks.len(), 1);
        assert_eq!(text_of_block(&d, 0), "  indented");
        assert_eq!(d.caret.block, 0);
        assert_eq!((d.caret.inline, d.caret.offset), (0, 2));
        assert!(d.caret.style.is_plain());
        assert_invariants(&d);
    }

    #[test]
    fn style_caret_at_end_of_bold_run_pops_before_moving() {
        let mut d = doc();
        d.blocks[0] = Block::Paragraph(vec![plain_run("ab"), bold_run("cd"), plain_run("ef")]);
        d.set_caret(0, 1, 2); // end of the bold run, context bold
        assert_eq!(d.caret.style, bold());
        let pos = (d.caret.inline, d.caret.offset);
        d.move_right(); // pop out to PLAIN — no movement
        assert!(d.caret.style.is_plain());
        assert_eq!((d.caret.inline, d.caret.offset), pos);
        assert!(!d.is_dirty());
        d.move_right(); // second move_right actually moves
        assert_ne!((d.caret.inline, d.caret.offset), pos);
        assert_invariants(&d);
    }

    #[test]
    fn style_caret_reenters_previous_run_within_boundary() {
        let mut d = doc();
        d.blocks[0] = Block::Paragraph(vec![plain_run("ab"), bold_run("cd"), plain_run("ef")]);
        d.set_caret(0, 1, 2); // context bold
        d.move_right(); // pop to PLAIN, no move
        assert!(d.caret.style.is_plain());
        assert_eq!((d.caret.inline, d.caret.offset), (1, 2));
        d.move_left(); // re-enter the bold run — context only
        assert_eq!(d.caret.style, bold());
        assert_eq!((d.caret.inline, d.caret.offset), (1, 2));
        d.move_left(); // now it moves
        assert_eq!((d.caret.inline, d.caret.offset), (1, 1));
        assert_invariants(&d);
    }

    #[test]
    fn style_armed_toggle_with_nothing_typed_does_not_block_movement() {
        // Bold armed with nothing typed yet: toggle_bold only arms the caret's
        // pending context, it creates no run. Pressing Right here must move
        // the caret immediately, not "pop" a boundary that doesn't exist.
        let mut d = doc();
        d.insert_text("abc");
        d.set_caret(0, 0, 1); // interior of the one plain run, not a seam
        assert!(d.caret.style.is_plain());
        d.toggle_bold();
        assert_eq!(d.caret.style, bold());
        d.move_right();
        assert_eq!(
            (d.caret.inline, d.caret.offset),
            (0, 2),
            "the press must move the caret, not get eaten by a phantom pop"
        );
        assert!(
            d.caret.style.is_plain(),
            "style reverts to the destination's own style-before value"
        );
        assert_invariants(&d);
    }

    #[test]
    fn style_click_at_boundary_gets_before_style() {
        let mut d = doc();
        d.blocks[0] = Block::Paragraph(vec![plain_run("ab"), bold_run("cd")]);
        // Between plain and bold: the char before is plain.
        d.set_caret(0, 1, 0);
        assert_eq!(d.caret.style, Style::PLAIN);
        // Between bold and nothing: the char before is bold.
        d.set_caret(0, 1, 2);
        assert_eq!(d.caret.style, bold());
        assert_invariants(&d);
    }

    #[test]
    fn style_crossing_block_boundary_resets_to_plain() {
        let mut d = doc();
        d.blocks = vec![
            Block::Paragraph(vec![plain_run("ab")]),
            Block::Paragraph(vec![bold_run("cd")]),
        ];
        d.set_caret(0, 0, 2);
        d.toggle_bold(); // bold context sitting at the block end
        assert_eq!(d.caret.style, bold());
        d.move_right(); // pop to PLAIN at the block end — no move
        assert!(d.caret.style.is_plain());
        assert_eq!((d.caret.block, d.caret.inline, d.caret.offset), (0, 0, 2));
        d.move_right(); // cross the boundary into block 1, context PLAIN
        assert_eq!((d.caret.block, d.caret.inline, d.caret.offset), (1, 0, 0));
        assert!(d.caret.style.is_plain());
        assert_invariants(&d);
    }

    #[test]
    fn set_heading_round_trips_kind_without_touching_runs() {
        let mut d = doc();
        d.insert_text("hello");
        d.set_heading(Some(2));
        assert!(matches!(d.blocks[0], Block::Heading { level: 2, .. }));
        assert_eq!(runs(&d.blocks[0]), vec![("hello".into(), Style::PLAIN)]);
        d.set_heading(None);
        assert!(!d.blocks[0].is_heading());
        assert_eq!(runs(&d.blocks[0]), vec![("hello".into(), Style::PLAIN)]);
        assert_invariants(&d);
    }

    #[test]
    fn a_divider_inserted_on_a_blank_line_replaces_it() {
        let mut d = doc();
        d.insert_divider();
        assert_eq!(d.blocks.len(), 2);
        assert!(d.blocks[0].is_divider());
        assert_eq!(runs(&d.blocks[0]), vec![(String::new(), Style::PLAIN)]);
        assert_eq!(text_of_block(&d, 1), "");
        assert_eq!(d.caret.block, 1);
        assert_invariants(&d);
    }

    #[test]
    fn a_divider_after_text_keeps_the_text() {
        let mut d = doc();
        d.insert_text("text");
        d.insert_divider();
        assert_eq!(d.blocks.len(), 3);
        assert_eq!(text_of_block(&d, 0), "text");
        assert!(d.blocks[1].is_divider());
        assert_eq!(text_of_block(&d, 2), "");
        assert_eq!(d.caret.block, 2);
        assert_invariants(&d);
    }

    #[test]
    fn typing_on_a_rule_turns_it_back_into_a_paragraph() {
        let mut d = doc();
        d.insert_divider();
        d.set_caret(0, 0, 0);
        d.insert_text("typed");
        assert!(matches!(d.blocks[0], Block::Paragraph(_)));
        assert_eq!(text_of_block(&d, 0), "typed");
        assert_eq!(d.blocks.len(), 2);
        assert_eq!(d.caret.block, 0);
        assert_invariants(&d);
    }

    #[test]
    fn move_home_and_move_end_logical_and_style_before() {
        let mut d = doc();
        d.blocks[0] = Block::Paragraph(vec![plain_run("ab"), bold_run("cd")]);
        d.set_caret(0, 1, 1);
        d.move_home();
        assert_eq!((d.caret.inline, d.caret.offset), (0, 0));
        assert!(d.caret.style.is_plain());
        d.move_end();
        assert_eq!((d.caret.inline, d.caret.offset), (1, 2));
        assert_eq!(d.caret.style, bold()); // style-before: last char is bold
        assert_invariants(&d);
    }

    #[test]
    fn delete_char_through_run_boundary_into_bold() {
        // The caret sits in the gap at a plain run's end; vim `x` deletes
        // the char after the cursor — the first bold char. The old run-local
        // `o >= len` check turned this boundary into a silent no-op, stranding
        // the caret in front of the bold block.
        let mut d = doc();
        d.blocks[0] = Block::Paragraph(vec![plain_run("a"), bold_run("bcd")]);
        d.set_caret(0, 0, 1);
        d.delete_char();
        assert_eq!(text_of_block(&d, 0), "acd");
        assert_eq!(
            runs(&d.blocks[0]),
            vec![("a".into(), Style::PLAIN), ("cd".into(), bold())]
        );
        d.delete_char(); // still at flat 1: deletes the run's next char
        assert_eq!(text_of_block(&d, 0), "ad");
        d.delete_char();
        assert_eq!(text_of_block(&d, 0), "a");
        assert_invariants(&d);
    }

    #[test]
    fn delete_char_consumes_the_plain_run_and_lands_on_the_bold() {
        let mut d = doc();
        d.blocks[0] = Block::Paragraph(vec![plain_run("ab"), bold_run("cdef")]);
        d.set_caret(0, 0, 0);
        for _ in 0..4 {
            d.delete_char();
        }
        assert_eq!(runs(&d.blocks[0]), vec![("ef".into(), bold())]);
        assert_eq!((d.caret.inline, d.caret.offset), (0, 0));
        d.delete_char(); // caret now on the bold run, deletion continues
        assert_eq!(text_of_block(&d, 0), "f");
        assert_invariants(&d);
    }

    // ---- code-line tests ------------------------------------------------

    #[test]
    fn style_has_code_field() {
        let s = Style {
            code: true,
            ..Style::PLAIN
        };
        assert!(!s.is_plain());
        assert_eq!(s, code_style());
    }

    #[test]
    fn block_kind_code_line() {
        assert!(
            Block::CodeLine {
                content: vec![],
                first: true,
                lang: None
            }
            .is_code()
        );
        assert!(!Block::Paragraph(vec![]).is_code());
        assert!(
            !Block::Heading {
                level: 1,
                content: vec![]
            }
            .is_code()
        );
    }

    #[test]
    fn toggle_code_sets_and_unsets_caret_context() {
        let mut d = doc();
        assert!(!d.caret.style.code);
        d.toggle_code();
        assert!(d.caret.style.code);
        d.toggle_code();
        assert!(!d.caret.style.code);
        assert!(!d.is_dirty());
    }

    #[test]
    fn toggle_bold_and_italic_are_noops_when_code_context_is_active() {
        let mut d = doc();
        d.toggle_code();
        d.toggle_bold();
        assert!(d.caret.style.code);
        assert!(!d.caret.style.bold);
        d.toggle_italic();
        assert!(!d.caret.style.italic);
        d.toggle_code();
        d.toggle_bold();
        assert!(d.caret.style.bold);
    }

    #[test]
    fn a_boxed_context_replaces_the_pending_style_rather_than_stacking() {
        // A badge and a code span each draw their own box; turning one on
        // has to clear whatever was armed, or the next run carries a weight
        // its box has no way to show.
        let mut d = doc();
        d.toggle_bold();
        d.toggle_highlight();
        d.toggle_badge();
        assert_eq!(
            d.caret.style,
            Style {
                badge: true,
                ..Style::PLAIN
            }
        );
        d.toggle_code();
        assert_eq!(
            d.caret.style,
            Style {
                code: true,
                ..Style::PLAIN
            }
        );
        // And nothing stacks onto one while it is active.
        d.toggle_highlight();
        assert!(!d.caret.style.highlight);
        d.toggle_code();
        d.toggle_highlight();
        assert!(d.caret.style.highlight, "plain again, so the mark takes");
        assert!(!d.is_dirty(), "style context is not a content edit");
    }

    #[test]
    fn set_code_converts_paragraph_to_code_line_and_back() {
        let mut d = doc();
        d.insert_text("hello world");
        d.set_code(true);
        assert!(d.blocks[0].is_code());
        assert_eq!(
            runs(&d.blocks[0]),
            vec![("hello world".into(), code_style())]
        );
        assert_invariants(&d);
        d.set_code(false);
        assert!(!d.blocks[0].is_code());
        assert_eq!(
            runs(&d.blocks[0]),
            vec![("hello world".into(), Style::PLAIN)]
        );
        assert_invariants(&d);
    }

    #[test]
    fn set_code_collapses_multi_run_paragraph_into_one_run() {
        let mut d = doc();
        d.blocks[0] = Block::Paragraph(vec![plain_run("ab"), bold_run("cd")]);
        d.set_code(true);
        assert_eq!(runs(&d.blocks[0]), vec![("abcd".into(), code_style())]);
        assert_invariants(&d);
    }

    #[test]
    fn set_code_clamps_caret_mid_block() {
        let mut d = doc();
        d.insert_text("hello world");
        d.set_caret(0, 0, 5);
        d.set_code(true);
        assert!(d.blocks[0].is_code());
        // Caret clamped into the single collapsed run's bounds
        assert!(d.caret.offset <= 11);
        assert_eq!(d.caret.block, 0);
        assert_invariants(&d);
    }

    #[test]
    fn set_heading_from_code_line_strips_code_style() {
        let mut d = doc();
        d.insert_text("my code");
        d.set_code(true);
        assert!(d.blocks[0].is_code());
        d.set_heading(Some(1));
        let block = &d.blocks[0];
        assert!(block.is_heading());
        // The result heading must not carry code-styled runs
        for run in block.inlines() {
            assert!(
                !run.style().code,
                "heading from code line must not have code style"
            );
        }
        assert_eq!(runs(block), vec![("my code".into(), Style::PLAIN)]);
        assert_invariants(&d);
    }

    #[test]
    fn newline_in_code_line_creates_another_code_line() {
        let mut d = doc();
        d.insert_text("abc");
        d.set_code(true);
        d.set_caret(0, 0, 2); // mid-text
        d.newline();
        assert_eq!(d.blocks.len(), 2);
        assert!(d.blocks[0].is_code());
        assert!(d.blocks[1].is_code());
        assert_eq!(d.caret.block, 1);
        assert_eq!((d.caret.inline, d.caret.offset), (0, 0));
        // Split creates a continuation: first == false, lang == None
        assert!(
            matches!(
                d.blocks[1],
                Block::CodeLine {
                    first: false,
                    lang: None,
                    ..
                }
            ),
            "new block from split must be a continuation"
        );
        assert_invariants(&d);
    }

    #[test]
    fn empty_code_line_keeps_placeholder_run() {
        let mut d = doc();
        d.set_code(true);
        assert!(d.blocks[0].is_code());
        assert_eq!(d.blocks[0].inlines().len(), 1);
        assert!(d.blocks[0].inlines()[0].text().is_empty());
        assert_eq!(d.blocks[0].inlines()[0].style(), code_style());
        assert_invariants(&d);
    }

    #[test]
    fn prune_runs_emptied_code_line_stays_code_line() {
        let mut d = doc();
        d.insert_text("x");
        d.set_code(true);
        d.set_caret(0, 0, 0);
        d.delete_char(); // removes the only char
        assert!(d.blocks[0].is_code(), "emptied code line stays a code line");
        assert_eq!(d.blocks[0].inlines().len(), 1);
        assert_invariants(&d);
    }

    #[test]
    fn flat_range_delete_preserves_runs_and_merges_block_edges() {
        let mut d = doc();
        d.blocks = vec![
            Block::Paragraph(vec![plain_run("ab"), bold_run("cd")]),
            Block::Paragraph(vec![plain_run("ef")]),
        ];
        let deleted = d.delete_range(FlatRange::new(
            FlatPos {
                block: 0,
                offset: 1,
            },
            FlatPos {
                block: 1,
                offset: 1,
            },
        ));
        assert_eq!(deleted, "bcd\ne");
        assert_eq!(text_of_block(&d, 0), "af");
        assert_eq!(d.blocks.len(), 1);
        assert_invariants(&d);
    }

    #[test]
    fn text_objects_find_words_quotes_parens_and_nested_heading_section() {
        let mut d = doc();
        d.blocks = vec![
            Block::Heading {
                level: 1,
                content: vec![plain_run("Top")],
            },
            Block::Paragraph(vec![plain_run("body (inside) and \"quoted\"")]),
            Block::Heading {
                level: 2,
                content: vec![plain_run("Nested")],
            },
            Block::Paragraph(vec![plain_run("child")]),
            Block::Heading {
                level: 1,
                content: vec![plain_run("Next")],
            },
        ];
        d.set_caret(1, 0, 7);
        assert_eq!(
            d.range_text(d.text_object_range(TextObject::InnerParen).unwrap()),
            "inside"
        );
        d.set_caret(1, 0, 21);
        assert_eq!(
            d.range_text(d.text_object_range(TextObject::InnerQuote).unwrap()),
            "quoted"
        );
        d.set_caret(3, 0, 2);
        let heading = d.text_object_range(TextObject::InnerHeading).unwrap();
        assert_eq!(heading.start.block, 2);
        assert_eq!(heading.end.block, 3);
        assert_eq!(d.range_text(heading), "Nested\nchild");
    }

    #[test]
    fn delimiters_require_caret_inside_and_around_paragraph_includes_separator() {
        let mut d = doc();
        d.blocks = vec![
            Block::Paragraph(vec![plain_run("(inside)")]),
            Block::Paragraph(vec![plain_run("next")]),
        ];
        d.set_caret(0, 0, 0);
        assert!(d.text_object_range(TextObject::InnerParen).is_none());
        d.set_caret(0, 0, 1);
        assert_eq!(
            d.range_text(d.text_object_range(TextObject::InnerParen).unwrap()),
            "inside"
        );
        d.set_caret(0, 0, 7);
        assert!(d.text_object_range(TextObject::InnerParen).is_none());

        d.set_caret(0, 0, 2);
        assert_eq!(
            d.range_text(d.text_object_range(TextObject::InnerParagraph).unwrap()),
            "(inside)"
        );
        assert_eq!(
            d.range_text(d.text_object_range(TextObject::AroundParagraph).unwrap()),
            "(inside)\n"
        );
    }

    #[test]
    fn toggle_style_range_flips_bold_on_selected_text() {
        let mut d = doc();
        d.blocks[0] = Block::Paragraph(vec![plain_run("hello world")]);
        let range = FlatRange::new(
            FlatPos {
                block: 0,
                offset: 0,
            },
            FlatPos {
                block: 0,
                offset: 5,
            },
        );
        d.toggle_style_range(
            range,
            Style {
                bold: true,
                ..Style::PLAIN
            },
        );
        assert_eq!(
            runs(&d.blocks[0]),
            vec![("hello".into(), bold()), (" world".into(), Style::PLAIN)]
        );
        d.toggle_style_range(
            range,
            Style {
                bold: true,
                ..Style::PLAIN
            },
        );
        assert_eq!(text_of_block(&d, 0), "hello world");
        assert!(
            runs(&d.blocks[0])
                .iter()
                .all(|(_, style)| *style == Style::PLAIN)
        );
        assert_invariants(&d);
    }

    #[test]
    fn toggle_style_range_flips_inline_code_on_selected_text() {
        let mut d = doc();
        d.blocks[0] = Block::Paragraph(vec![plain_run("hello world")]);
        let range = FlatRange::new(
            FlatPos {
                block: 0,
                offset: 0,
            },
            FlatPos {
                block: 0,
                offset: 5,
            },
        );
        let code = Style {
            code: true,
            ..Style::PLAIN
        };

        d.toggle_style_range(range, code);
        assert_eq!(
            runs(&d.blocks[0]),
            vec![("hello".into(), code), (" world".into(), Style::PLAIN)]
        );
        d.toggle_style_range(range, code);
        assert_eq!(text_of_block(&d, 0), "hello world");
        assert!(
            runs(&d.blocks[0])
                .iter()
                .all(|(_, style)| *style == Style::PLAIN)
        );
        assert_invariants(&d);
    }
}
