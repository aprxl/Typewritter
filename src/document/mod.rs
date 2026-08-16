//! The document model: blocks, inline runs, the style-context caret, and
//! the editing operations on top of them. Pure model — no IO, no renderer.
//!
//! See `tasks/01-document-model.md` for the spec this implements, and
//! `docs/SPEC.md` §4.2 for the style-context machine.

use std::path::{Path, PathBuf};

pub mod layout;
pub mod markdown;
pub mod math;
pub mod math_conversion;
pub mod math_layout;
pub mod math_notation;
pub mod math_symbols;
pub mod outline;

/// Flat-text stand-in for one opaque math atom.
pub const ATOM: char = '\u{FFFC}';

/// A margin note body, keyed by the label its anchor carries. Held beside the
/// blocks rather than in them, because a note belongs to a position in the
/// prose and not to the flow of it.
#[derive(Clone, PartialEq, Debug)]
pub struct Sidenote {
    pub label: String,
    /// The note's own block list, carrying the same invariant the document's
    /// blocks carry: never empty, every block holds at least one run. Today a
    /// note is exactly one `Block::Paragraph`; the type is `Vec<Block>`
    /// because that is what the editing primitives take, and multi-paragraph
    /// notes are the natural next step.
    pub body: Vec<Block>,
    /// Whether an anchor for this label exists in the prose. A note loaded
    /// from disk with no anchor is `false` and is left alone by `prune_runs`;
    /// only a note the reader's edit actually orphaned (an anchor they once
    /// had and then deleted) is dropped.
    pub anchored: bool,
}

/// Where the caret lives: in the document body, or inside one note's body.
/// Every editing primitive operates on whichever scope is focused, so the
/// keymap and the shell never have to know notes exist.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Focus {
    /// The document's own blocks.
    #[default]
    Body,
    /// Index into [`Document::notes`].
    Note(usize),
}

/// A document: an ordered list of blocks with a caret.
#[derive(Clone, PartialEq, Debug)]
pub struct Document {
    pub blocks: Vec<Block>, // invariant: never empty
    pub path: PathBuf,
    pub name: String,
    dirty: bool,
    pub caret: Caret,
    /// Cursor inside the math atom named by `caret`, or prose focus when None.
    pub math: Option<math::MathCursor>,
    /// Margin note bodies, keyed by the label their anchors carry. Held
    /// beside the blocks rather than in them, because a note belongs to a
    /// position in the prose and not to the flow of it.
    pub notes: Vec<Sidenote>,
    /// Which scope the caret is in: the document body, or one note's body.
    pub focus: Focus,
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
    /// A display math block: exactly one opaque math atom until math rendering lands.
    Math(Vec<Inline>),
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
    /// One opaque math expression; flat prose coordinates count it as one.
    Math(math::MathList),
    /// An anchor for a margin note. Opaque like an expression: flat prose
    /// coordinates count it as exactly one position, so every motion,
    /// selection and offset in the document keeps working unchanged.
    Note(String),
}

#[derive(Clone, PartialEq, Debug)]
pub struct Text {
    pub text: String,
    pub style: Style,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum BadgeColor {
    #[default]
    Orange,
    Blue,
    Green,
    Purple,
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
    pub badge_color: BadgeColor,
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
        badge_color: BadgeColor::Orange,
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
            | Block::Math(inlines)
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
            | Block::Math(inlines)
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

    pub fn is_math(&self) -> bool {
        matches!(self, Block::Math(_))
    }

    pub fn is_code(&self) -> bool {
        matches!(self, Block::CodeLine { .. })
    }
}

impl Inline {
    fn text(&self) -> &str {
        match self {
            Inline::Text(t) => &t.text,
            // U+FFFC gives every flat-text consumer exactly one position.
            Inline::Math(_) => "\u{FFFC}",
            Inline::Note(_) => "\u{FFFC}",
        }
    }

    fn text_mut(&mut self) -> Option<&mut String> {
        match self {
            Inline::Text(t) => Some(&mut t.text),
            // Math is edited through its tree, never as prose.
            Inline::Math(_) => None,
            // An anchor's label is fixed by the note it points at; the
            // reader edits the note body, not the anchor.
            Inline::Note(_) => None,
        }
    }

    fn style(&self) -> Style {
        match self {
            Inline::Text(t) => t.style,
            Inline::Math(_) => Style::PLAIN,
            Inline::Note(_) => Style::PLAIN,
        }
    }

    fn set_style(&mut self, style: Style) {
        match self {
            Inline::Text(t) => t.style = style,
            Inline::Math(_) => {}
            Inline::Note(_) => {}
        }
    }
}

fn run_len(run: &Inline) -> usize {
    match run {
        Inline::Text(t) => t.text.chars().count(),
        // Opaque math always costs one flat position.
        Inline::Math(_) => 1,
        Inline::Note(_) => 1,
    }
}

fn merge_style(run: &Inline) -> Option<Style> {
    match run {
        Inline::Text(t) => Some(t.style),
        // An atom is never a prose merge target.
        Inline::Math(_) => None,
        Inline::Note(_) => None,
    }
}

/// Whether a run is opaque — costs one flat position and is deleted as a
/// whole rather than char by char. Math atoms and sidenote anchors both are.
fn is_opaque(run: &Inline) -> bool {
    matches!(run, Inline::Math(_) | Inline::Note(_))
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

/// A display math block starts with one empty opaque atom.
fn math_block() -> Block {
    Block::Math(vec![Inline::Math(Vec::new())])
}

/// Repair one block's runs: drop empty runs, demote a rule or display atom
/// that gained prose, and turn an emptied block into a placeholder of its
/// own kind. Shared by the document's blocks and every note body, so the
/// invariants hold in every scope rather than only where the caret is.
fn prune_block(block: &mut Block) {
    block.inlines_mut().retain(|r| !r.text().is_empty());
    // A rule holds no text. Typing on one turns it into prose — enforced
    // centrally here, so every edit path gets it without a special case of
    // its own.
    if block.is_divider() && block.inlines().iter().any(|r| !r.text().is_empty()) {
        *block = Block::Paragraph(std::mem::take(block.inlines_mut()));
    }
    if block.is_math() {
        if block.inlines().is_empty() {
            *block = math_block();
        } else if !matches!(block.inlines(), [Inline::Math(_)]) {
            // Prose in a display atom means this is prose now; keep the
            // atom inline rather than discarding its tree.
            *block = Block::Paragraph(std::mem::take(block.inlines_mut()));
        }
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
            Block::Math(_) => math_block(),
            Block::Paragraph(_) => empty_block(),
        };
    }
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
        Inline::Math(list) => {
            // An atom cannot split; the empty side is pruned after the edit.
            if at == 0 {
                (
                    Inline::Text(Text {
                        text: String::new(),
                        style: Style::PLAIN,
                    }),
                    Inline::Math(list),
                )
            } else {
                (
                    Inline::Math(list),
                    Inline::Text(Text {
                        text: String::new(),
                        style: Style::PLAIN,
                    }),
                )
            }
        }
        Inline::Note(label) => {
            // An anchor cannot split either; the empty side is pruned.
            if at == 0 {
                (
                    Inline::Text(Text {
                        text: String::new(),
                        style: Style::PLAIN,
                    }),
                    Inline::Note(label),
                )
            } else {
                (
                    Inline::Note(label),
                    Inline::Text(Text {
                        text: String::new(),
                        style: Style::PLAIN,
                    }),
                )
            }
        }
    }
}

/// Split inserted text into inline runs, reading `$…$` as a math expression
/// exactly as `markdown::parse_inline` does, so pasting copied notation
/// reconstructs the expression instead of leaving literal dollars. A `\`
/// collapses onto the next character the way markdown reads it; everything
/// else is one text run in `style`.
fn inline_runs(text: &str, style: Style) -> Vec<Inline> {
    let chars: Vec<char> = text.chars().collect();
    let mut runs: Vec<Inline> = Vec::new();
    let mut buf = String::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' => {
                if i + 1 < chars.len() {
                    buf.push(chars[i + 1]);
                    i += 2;
                } else {
                    buf.push('\\');
                    i += 1;
                }
            }
            '$' => {
                if let Some(close) = dollar_closer(&chars, i + 1) {
                    push_text(&mut runs, &mut buf, style);
                    let inner: String = chars[i + 1..close].iter().collect();
                    runs.push(Inline::Math(math_notation::parse(&inner)));
                    i = close + 1;
                } else {
                    buf.push('$');
                    i += 1;
                }
            }
            c => {
                buf.push(c);
                i += 1;
            }
        }
    }
    push_text(&mut runs, &mut buf, style);
    runs
}

/// Char index of the next unescaped `$` at or after `start`, mirroring the
/// closer scan `markdown` uses for inline math.
fn dollar_closer(chars: &[char], start: usize) -> Option<usize> {
    let mut i = start;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += if i + 1 < chars.len() { 2 } else { 1 };
        } else if chars[i] == '$' {
            return Some(i);
        } else {
            i += 1;
        }
    }
    None
}

/// Append a pending text buffer as one text run, if it is non-empty.
fn push_text(runs: &mut Vec<Inline>, buf: &mut String, style: Style) {
    if !buf.is_empty() {
        runs.push(Inline::Text(Text {
            text: std::mem::take(buf),
            style,
        }));
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
            math: None,
            notes: Vec::new(),
            focus: Focus::Body,
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// The block list the caret lives in: the document's own blocks, or one
    /// note's body. Every caret-relative read resolves through this.
    fn focused(&self) -> &[Block] {
        match self.focus {
            Focus::Body => &self.blocks,
            Focus::Note(i) => &self.notes[i].body,
        }
    }

    /// The mutable block list the caret lives in. Every caret-relative edit
    /// resolves through this, so it lands in the note body when a note is
    /// focused and in the prose when it is not.
    fn focused_mut(&mut self) -> &mut Vec<Block> {
        match self.focus {
            Focus::Body => &mut self.blocks,
            Focus::Note(i) => &mut self.notes[i].body,
        }
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
    /// A math atom is written as its `$…$` notation and a literal `$` in
    /// prose is escaped as `\$` — the same bytes `markdown::serialize` writes
    /// to disk — so copy, cut, and the yank register all carry a re-readable
    /// form of the block, and pasting it back reconstructs the same content.
    pub fn range_text(&self, range: FlatRange) -> String {
        let range = range.normalized();
        let start = self.position(range.start.block, range.start.offset);
        let end = self.position(range.end.block, range.end.offset);
        if (start.block, start.offset) >= (end.block, end.offset) {
            return String::new();
        }
        let mut text = String::new();
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
            text.push_str(&self.block_range_text(block, from, to));
            if block != end.block {
                text.push('\n');
            }
        }
        text
    }

    /// Flat-text form of `[from, to)` within one block. A math atom costs
    /// exactly one flat position, so it is either wholly inside the range or
    /// wholly outside it; the `$…$` form is emitted whole, never sliced, so
    /// `from`/`to` stay aligned with every other flat offset. Prose is escaped
    /// so the result is the same notation `markdown::serialize` writes to
    /// disk: a literal `$` becomes `\$`, which cannot reopen math on the way
    /// back in.
    fn block_range_text(&self, block: usize, from: usize, to: usize) -> String {
        let mut out = String::new();
        let mut cursor = 0;
        for run in self.blocks[block].inlines() {
            let run_start = cursor;
            let run_end = cursor + run_len(run);
            let slice_from = from.max(run_start);
            let slice_to = to.min(run_end);
            if slice_from < slice_to {
                match run {
                    Inline::Text(t) => {
                        let slice: String = t
                            .text
                            .chars()
                            .skip(slice_from - run_start)
                            .take(slice_to - slice_from)
                            .collect();
                        out.push_str(&Self::escape_prose(&slice));
                    }
                    Inline::Math(list) => {
                        out.push('$');
                        out.push_str(&math_notation::print(list));
                        out.push('$');
                    }
                    Inline::Note(label) => {
                        out.push_str("[^");
                        out.push_str(label);
                        out.push(']');
                    }
                }
            }
            cursor = run_end;
        }
        out
    }

    /// Escape the prose characters `insert_notation` would otherwise read as
    /// notation, exactly as `markdown::serialize_runs` does: a literal `$`
    /// becomes `\$` (so it cannot open math on the way back) and a literal
    /// `\` becomes `\\` (so it cannot collapse onto the next character).
    fn escape_prose(text: &str) -> String {
        let mut out = String::new();
        for c in text.chars() {
            match c {
                '\\' => out.push_str("\\\\"),
                '$' => out.push_str("\\$"),
                c => out.push(c),
            }
        }
        out
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
                    all_boxed &= if boxed.badge {
                        run.style().badge
                    } else {
                        run.style() == boxed
                    };
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

    /// Change the colour of badges covered by `range`, leaving prose and
    /// other boxed runs untouched.
    pub fn set_badge_color(&mut self, range: FlatRange, color: BadgeColor) {
        let range = range.normalized();
        let start = self.position(range.start.block, range.start.offset);
        let end = self.position(range.end.block, range.end.offset);
        if (start.block, start.offset) >= (end.block, end.offset) {
            return;
        }

        let mut changed = false;
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
            let mut selected = self.slice_runs(block, from, to);
            let block_changed = selected.iter_mut().fold(false, |changed, run| {
                let mut style = run.style();
                if style.badge && style.badge_color != color {
                    style.badge_color = color;
                    run.set_style(style);
                    true
                } else {
                    changed
                }
            });
            if !block_changed {
                continue;
            }
            let mut runs = self.slice_runs(block, 0, from);
            runs.extend(selected);
            runs.extend(self.slice_runs(block, to, self.block_len(block)));
            self.blocks[block] = Self::block_with_runs(&original, runs);
            changed = true;
        }
        if changed {
            self.dirty = true;
            self.enforce();
            self.refresh_context();
        }
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
            let run_start = cursor;
            let run_end = cursor + run_len(run);
            let from = start.max(run_start).min(run_end);
            let to = end.max(run_start).min(run_end);
            if from < to {
                match run {
                    Inline::Text(t) => {
                        let value: String = t
                            .text
                            .chars()
                            .skip(from - run_start)
                            .take(to - from)
                            .collect();
                        result.push(Inline::Text(Text {
                            text: value,
                            style: t.style,
                        }));
                    }
                    // Slices cover the whole opaque atom or none of it.
                    Inline::Math(_) => result.push(run.clone()),
                    Inline::Note(_) => result.push(run.clone()),
                }
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
            Block::Math(_) => Block::Math(runs),
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
        self.focused()[block].inlines().iter().map(run_len).sum()
    }

    /// The caret's flat position within its block, clamped.
    fn caret_flat(&self, block: usize) -> usize {
        let runs = self.focused()[block].inlines();
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
        let runs = self.focused()[block].inlines();
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
        let runs = self.focused()[block].inlines();
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

    /// Repairs the caret into valid bounds. `blocks` is never empty. When
    /// focus names a note that no longer exists — its anchor was deleted, so
    /// `prune_runs` dropped the note — focus falls back to the body and the
    /// caret is clamped there, because a caret pointing into a dropped note
    /// must never be reachable.
    fn clamp_caret(&mut self) {
        if self.blocks.is_empty() {
            self.blocks.push(empty_block());
        }
        if let Focus::Note(i) = self.focus
            && i >= self.notes.len()
        {
            self.focus = Focus::Body;
        }
        let block = self.caret.block.min(self.focused().len() - 1);
        self.caret.block = block;
        // Read the clamped inline/offset and whether the run is math before
        // mutating: the `focused` borrow would otherwise outlive `self.math`.
        let (inline, offset, is_math) = {
            let runs = self.focused()[block].inlines();
            let inline = self.caret.inline.min(runs.len().saturating_sub(1));
            let len = run_len(&runs[inline]);
            let offset = self.caret.offset.min(len);
            let is_math = matches!(runs[inline], Inline::Math(_));
            (inline, offset, is_math)
        };
        self.caret.inline = inline;
        self.caret.offset = offset;
        if !is_math {
            self.math = None;
        }
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
        // The invariant holds in *every* scope, not just where the caret
        // happens to be: a note body left with an empty run would be an
        // invariant only until focus moved elsewhere.
        for block in &mut self.blocks {
            prune_block(block);
        }
        for note in &mut self.notes {
            for block in &mut note.body {
                prune_block(block);
            }
        }

        // An anchor the reader deleted leaves its note unreachable — a body
        // nothing points at cannot be seen or reached, so keep it only for
        // notes that never had an anchor to begin with (loaded from disk as
        // a stray definition), which an edit is not allowed to discard.
        let anchored: Vec<&str> = self
            .blocks
            .iter()
            .flat_map(Block::inlines)
            .filter_map(|run| match run {
                Inline::Note(label) => Some(label.as_str()),
                _ => None,
            })
            .collect();
        self.notes
            .retain(|note| !note.anchored || anchored.contains(&note.label.as_str()));
    }

    fn invariants_hold(&self) -> bool {
        if self.blocks.is_empty() {
            return false;
        }
        for block in self
            .blocks
            .iter()
            .chain(self.notes.iter().flat_map(|note| note.body.iter()))
        {
            if block.inlines().is_empty() {
                return false;
            }
            for (i, run) in block.inlines().iter().enumerate() {
                if run.text().is_empty() && (block.inlines().len() > 1 || i != 0) {
                    return false;
                }
            }
        }
        for note in &self.notes {
            if note.body.is_empty() {
                return false;
            }
        }
        let block = &self.focused()[self.caret.block];
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
        let b = block.min(self.focused().len() - 1);
        self.caret.block = b;
        let runs = self.focused()[b].inlines();
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
        let run_len_i = run_len(&self.focused()[b].inlines()[self.caret.inline]);
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
        } else if b + 1 < self.focused().len() {
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
        let run_len_i = run_len(&self.focused()[b].inlines()[self.caret.inline]);
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

    /// Inserts `text` exactly as given. Every keystroke and every paste from
    /// another application arrives here, so nothing in it is interpreted: a
    /// `$` is a dollar sign, a `\` is a backslash. Only text this app itself
    /// produced takes [`Self::insert_notation`], where `$…$` and `\$` carry
    /// meaning.
    pub fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.clamp_caret();
        let s = self.caret.style;
        let b = self.caret.block;
        let i = self.caret.inline;
        let o = self.caret.offset;
        let flat = self.caret_flat(b);

        let len = text.chars().count();
        let runs = self.focused()[b].inlines();
        let placeholder = runs.len() == 1 && runs[0].text().is_empty();
        let li = run_len(&runs[i]);
        let left_style = if o > 0 {
            merge_style(&runs[i])
        } else if i > 0 {
            merge_style(&runs[i - 1])
        } else {
            None
        };
        let right_style = if o < li {
            merge_style(&runs[i])
        } else if i + 1 < runs.len() {
            merge_style(&runs[i + 1])
        } else {
            None
        };

        if placeholder {
            let runs = self.focused_mut()[b].inlines_mut();
            runs[0]
                .text_mut()
                .expect("placeholder is always a text run")
                .push_str(text);
            runs[0].set_style(s);
        } else if left_style == Some(s) {
            let runs = self.focused_mut()[b].inlines_mut();
            if o > 0 {
                insert_str(
                    runs[i]
                        .text_mut()
                        .expect("merge target is always a text run"),
                    o,
                    text,
                );
            } else {
                runs[i - 1]
                    .text_mut()
                    .expect("merge target is always a text run")
                    .push_str(text);
            }
        } else if right_style == Some(s) {
            let runs = self.focused_mut()[b].inlines_mut();
            if o < li {
                insert_str(
                    runs[i]
                        .text_mut()
                        .expect("merge target is always a text run"),
                    o,
                    text,
                );
            } else {
                insert_str(
                    runs[i + 1]
                        .text_mut()
                        .expect("merge target is always a text run"),
                    0,
                    text,
                );
            }
        } else {
            // Splice a new run at the caret, splitting the current run.
            let (prefix, suffix) = split_run(self.focused_mut()[b].inlines_mut().remove(i), o);
            let new_run = Inline::Text(Text {
                text: text.to_string(),
                style: s,
            });
            let runs = self.focused_mut()[b].inlines_mut();
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

    /// Inserts inline Markdown that this app produced, reading `$…$` back as
    /// an expression and `\$` as a literal dollar — the same notation
    /// `block_range_text` writes on copy and `markdown::serialize` writes on
    /// save. Only ever called with text Typewritter itself wrote; foreign
    /// text goes through [`Self::insert_text`], which never interprets it.
    pub fn insert_notation(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.clamp_caret();
        let s = self.caret.style;
        let b = self.caret.block;
        let i = self.caret.inline;
        let o = self.caret.offset;
        let flat = self.caret_flat(b);

        let parsed = inline_runs(text, s);
        let inserted: usize = parsed.iter().map(run_len).sum();
        let (prefix, suffix) = split_run(self.focused_mut()[b].inlines_mut().remove(i), o);
        let mut runs = vec![prefix];
        runs.extend(parsed);
        runs.push(suffix);
        self.focused_mut()[b].inlines_mut().splice(i..i, runs);
        let target = flat + inserted;
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
        let math_inline = if o > 0 && matches!(self.focused()[b].inlines()[i], Inline::Math(_)) {
            Some(i)
        } else if o == 0 && i > 0 && matches!(self.focused()[b].inlines()[i - 1], Inline::Math(_)) {
            Some(i - 1)
        } else {
            None
        };
        if let Some(inline) = math_inline {
            let len = match &self.focused()[b].inlines()[inline] {
                Inline::Math(list) => list.len(),
                Inline::Text(_) => unreachable!("math target was checked above"),
                Inline::Note(_) => unreachable!("math target was checked above"),
            };
            if len > 0 || self.focused()[b].is_math() {
                self.set_caret(b, inline, 0);
                self.math = Some(math::MathCursor {
                    path: Vec::new(),
                    index: len,
                });
                let _ = self.math_backspace();
                return;
            }
        }
        if o > 0 {
            let runs = self.focused_mut()[b].inlines_mut();
            if is_opaque(&runs[i]) {
                runs.remove(i);
            } else {
                remove_char_at(runs[i].text_mut().expect("non-math run is text"), o - 1);
            }
        } else if i > 0 {
            let runs = self.focused_mut()[b].inlines_mut();
            if is_opaque(&runs[i - 1]) {
                runs.remove(i - 1);
            } else {
                let prev_len = run_len(&runs[i - 1]);
                remove_char_at(
                    runs[i - 1].text_mut().expect("non-math run is text"),
                    prev_len - 1,
                );
            }
        } else if b > 0 && self.focused()[b - 1].is_math() {
            let previous = b - 1;
            let len = match self.focused()[previous].inlines() {
                [Inline::Math(list)] => list.len(),
                _ => unreachable!("display math must contain exactly one math atom"),
            };
            self.set_caret(previous, 0, 0);
            self.math = Some(math::MathCursor {
                path: Vec::new(),
                index: len,
            });
            let _ = self.math_backspace();
            return;
        } else if b > 0 && self.focused()[b].is_math() {
            self.set_caret(b, 0, 0);
            self.math = Some(math::MathCursor::default());
            return;
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
        let target = before.saturating_sub(1);
        self.dirty = true;
        self.enforce();
        let (ni, no) = self.flat_to_pos(b, target.min(self.block_flat_len(b)));
        self.caret.inline = ni;
        self.caret.offset = no;
        self.refresh_context();
    }

    /// Append this block's runs onto the previous block's; caret at the
    /// junction; delete this block.
    fn merge_into_previous(&mut self) {
        let b = self.caret.block;
        let prev = b - 1;
        let junction = self.block_flat_len(prev);
        let mut taken = {
            let runs = self.focused_mut()[b].inlines_mut();
            std::mem::take(runs)
        };
        self.focused_mut().remove(b);
        self.focused_mut()[prev].inlines_mut().append(&mut taken);
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
        let runs = self.focused()[b].inlines();
        let li = run_len(&runs[i]);
        let math_inline = if o < li && matches!(runs[i], Inline::Math(_)) {
            Some(i)
        } else if o == li && i + 1 < runs.len() && matches!(runs[i + 1], Inline::Math(_)) {
            Some(i + 1)
        } else {
            None
        };
        if let Some(inline) = math_inline {
            let empty = match &self.focused()[b].inlines()[inline] {
                Inline::Math(list) => list.is_empty(),
                Inline::Text(_) => unreachable!("math target was checked above"),
                Inline::Note(_) => unreachable!("math target was checked above"),
            };
            if !empty || self.focused()[b].is_math() {
                self.set_caret(b, inline, 0);
                self.math = Some(math::MathCursor::default());
                let _ = self.math_delete_forward();
                return;
            }
        }
        if o < li {
            let runs = self.focused_mut()[b].inlines_mut();
            if is_opaque(&runs[i]) {
                runs.remove(i);
            } else {
                remove_char_at(runs[i].text_mut().expect("non-math run is text"), o);
            }
        } else if i + 1 < runs.len() {
            let runs = self.focused_mut()[b].inlines_mut();
            if is_opaque(&runs[i + 1]) {
                runs.remove(i + 1);
            } else {
                remove_char_at(runs[i + 1].text_mut().expect("non-math run is text"), 0);
            }
        } else if b + 1 < self.focused().len() && self.focused()[b].is_math() {
            return;
        } else if b + 1 < self.focused().len() && self.focused()[b + 1].is_math() {
            let next = b + 1;
            self.set_caret(next, 0, 0);
            self.math = Some(math::MathCursor::default());
            let _ = self.math_delete_forward();
            return;
        } else if b + 1 < self.focused().len() {
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
            let runs = self.focused_mut()[next].inlines_mut();
            std::mem::take(runs)
        };
        self.focused_mut().remove(next);
        self.focused_mut()[b].inlines_mut().append(&mut taken);
    }

    pub fn newline(&mut self) {
        self.clamp_caret();
        // ponytail: a note is one paragraph, so splitting its block is a
        // no-op — the caret stays put and the body keeps its single
        // paragraph, which is all the one-line footnote definition on disk
        // can hold. Multi-paragraph notes would need indented continuation
        // lines in the Markdown, which the format has no room for yet.
        if matches!(self.focus, Focus::Note(_)) {
            return;
        }
        let b = self.caret.block;
        let i = self.caret.inline;
        let o = self.caret.offset;
        let is_code = self.focused()[b].is_code();
        let (prefix, suffix) = split_run(self.focused_mut()[b].inlines_mut().remove(i), o);
        let mut taken = Vec::new();
        let runs = self.focused_mut()[b].inlines_mut();
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
        self.focused_mut().insert(b + 1, new_block);

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
        if let Inline::Math(list) = &self.focused()[b].inlines()[i]
            && (!list.is_empty() || self.focused()[b].is_math())
        {
            self.set_caret(b, i, 0);
            self.math = Some(math::MathCursor::default());
            let _ = self.math_delete_forward();
            return;
        }
        let runs = self.focused_mut()[b].inlines_mut();
        if is_opaque(&runs[i]) {
            runs.remove(i);
        } else {
            remove_char_at(runs[i].text_mut().expect("non-math run is text"), o);
        }
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
        if self.focused().len() == 1 {
            self.focused_mut()[0] = empty_block();
            self.caret.block = 0;
            self.caret.inline = 0;
            self.caret.offset = 0;
        } else {
            self.focused_mut().remove(b);
            let landing = b.min(self.focused().len() - 1);
            self.caret.block = landing;
            self.caret.inline = 0;
            self.caret.offset = 0;
            let mut prefix = 0;
            for run in self.focused()[landing].inlines() {
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
        self.focused_mut().insert(b + 1, empty_block());
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
        self.focused_mut().insert(b, empty_block());
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
        let flat_text: String = self.focused()[b]
            .inlines()
            .iter()
            .map(Inline::text)
            .collect();
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
        self.focused_mut()[b] = if on {
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

    /// Convert one exact block to/from a code line without moving the caret.
    pub fn set_block_code_at(&mut self, block: usize, on: bool) -> bool {
        let Some(current) = self.blocks.get(block) else {
            return false;
        };
        if current.is_code() == on {
            return false;
        }

        let mut inlines = std::mem::take(self.blocks[block].inlines_mut());
        for run in &mut inlines {
            if let Inline::Text(text) = run {
                text.style = if on {
                    Style {
                        code: true,
                        ..Style::PLAIN
                    }
                } else {
                    Style::PLAIN
                };
            }
        }
        self.blocks[block] = if on {
            Block::CodeLine {
                content: inlines,
                first: true,
                lang: None,
            }
        } else {
            Block::Paragraph(inlines)
        };
        self.dirty = true;
        true
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
            let mut inlines = std::mem::take(self.focused_mut()[b].inlines_mut());
            if self.focused()[b].is_code() {
                for run in &mut inlines {
                    run.set_style(Style {
                        code: false,
                        ..run.style()
                    });
                }
            }
            self.focused_mut()[b] = Block::Heading {
                level,
                content: inlines,
            };
        } else {
            let mut inlines = std::mem::take(self.focused_mut()[b].inlines_mut());
            if self.focused()[b].is_code() {
                for run in &mut inlines {
                    run.set_style(Style {
                        code: false,
                        ..run.style()
                    });
                }
            }
            self.focused_mut()[b] = Block::Paragraph(inlines);
        }
        self.dirty = true;
        self.enforce();
    }

    /// Convert one exact block to Body or H1-H4 without moving the caret.
    pub fn set_block_heading_at(&mut self, block: usize, level: Option<u8>) -> bool {
        if level.is_some_and(|level| !(1..=4).contains(&level)) {
            return false;
        }
        let Some(current) = self.blocks.get(block) else {
            return false;
        };
        if matches!((current, level), (Block::Paragraph(_), None))
            || matches!((current, level), (Block::Heading { level: current, .. }, Some(next)) if *current == next)
        {
            return false;
        }

        let was_code = current.is_code();
        let mut inlines = std::mem::take(self.blocks[block].inlines_mut());
        if was_code {
            for run in &mut inlines {
                if let Inline::Text(text) = run {
                    text.style = Style::PLAIN;
                }
            }
        }
        self.blocks[block] = match level {
            Some(level) => Block::Heading {
                level,
                content: inlines,
            },
            None => Block::Paragraph(inlines),
        };
        self.dirty = true;
        true
    }

    /// Puts a rule below the caret's block and leaves the caret on a fresh
    /// empty paragraph after it — a rule is a separator you keep writing
    /// past, never a place to land. An empty block is replaced rather than
    /// pushed down, so a rule on a blank line does not leave a gap above
    /// itself.
    pub fn insert_divider(&mut self) {
        self.clamp_caret();
        let b = self.caret.block;
        let at = if self.block_flat_len(b) == 0 {
            self.focused_mut()[b] = divider_block();
            b
        } else {
            self.focused_mut().insert(b + 1, divider_block());
            b + 1
        };
        self.focused_mut().insert(at + 1, empty_block());
        self.set_caret(at + 1, 0, 0);
        self.dirty = true;
        self.enforce();
    }

    /// Inserts and focuses one inline math atom at the current flat caret.
    pub fn insert_inline_math(&mut self) {
        self.clamp_caret();
        let b = self.caret.block;
        let i = self.caret.inline;
        let o = self.caret.offset;
        let flat = self.caret_flat(b);
        let (prefix, suffix) = split_run(self.focused_mut()[b].inlines_mut().remove(i), o);
        self.focused_mut()[b]
            .inlines_mut()
            .splice(i..i, [prefix, Inline::Math(Vec::new()), suffix]);
        self.dirty = true;
        self.enforce();
        let (inline, offset) = self.flat_to_pos(b, flat);
        self.set_caret(b, inline, offset);
        self.math = Some(math::MathCursor::default());
    }

    /// Puts an anchor at the caret and opens an empty note for it, choosing
    /// the lowest label not already in use. Returns the new note's label, or
    /// `None` when no anchor was placed.
    pub fn insert_sidenote(&mut self) -> Option<String> {
        self.clamp_caret();
        // No note inside a note: the anchor would sit in the note's body but
        // the definition lives in the file's block stream, so there is
        // nowhere honest to place it. `None` reports "did not insert".
        if matches!(self.focus, Focus::Note(_)) {
            return None;
        }
        let label = self.next_free_label();
        let b = self.caret.block;
        let i = self.caret.inline;
        let o = self.caret.offset;
        let flat = self.caret_flat(b);
        let (prefix, suffix) = split_run(self.focused_mut()[b].inlines_mut().remove(i), o);
        self.focused_mut()[b]
            .inlines_mut()
            .splice(i..i, [prefix, Inline::Note(label.clone()), suffix]);
        self.notes.push(Sidenote {
            label: label.clone(),
            body: vec![empty_block()],
            anchored: true,
        });
        self.dirty = true;
        self.enforce();
        let (inline, offset) = self.flat_to_pos(b, (flat + 1).min(self.block_flat_len(b)));
        self.set_caret(b, inline, offset);
        Some(label)
    }

    /// The lowest integer label not already carried by an anchor or a note.
    fn next_free_label(&self) -> String {
        let mut n = 1usize;
        loop {
            let candidate = n.to_string();
            let used = self.notes.iter().any(|note| note.label == candidate)
                || self
                    .blocks
                    .iter()
                    .flat_map(Block::inlines)
                    .any(|run| matches!(run, Inline::Note(label) if label == &candidate));
            if !used {
                return candidate;
            }
            n += 1;
        }
    }

    /// Inserts a focused display math block, replacing an empty block.
    pub fn insert_math_block(&mut self) {
        self.clamp_caret();
        let b = self.caret.block;
        let at = if self.block_flat_len(b) == 0 {
            self.focused_mut()[b] = math_block();
            b
        } else {
            self.focused_mut().insert(b + 1, math_block());
            b + 1
        };
        self.set_caret(at, 0, 0);
        self.math = Some(math::MathCursor::default());
        self.dirty = true;
        self.enforce();
    }

    /// The focused atom's tree and cursor, or None when focus is stale.
    fn focused_math(&mut self) -> Option<(&mut math::MathList, &mut math::MathCursor)> {
        self.clamp_caret();
        let block = self.caret.block;
        let inline = self.caret.inline;
        // Borrow the atom and the cursor from disjoint fields: `focused_mut`
        // holds all of `self`, so the cursor borrow would not fit alongside.
        let list = match self.focus {
            Focus::Body => match self.blocks[block].inlines_mut().get_mut(inline) {
                Some(Inline::Math(list)) => list,
                _ => {
                    self.math = None;
                    return None;
                }
            },
            Focus::Note(i) => match self.notes[i].body[block].inlines_mut().get_mut(inline) {
                Some(Inline::Math(list)) => list,
                _ => {
                    self.math = None;
                    return None;
                }
            },
        };
        let cursor = self.math.as_mut()?;
        Some((list, cursor))
    }

    pub fn math_insert_char(&mut self, c: char) {
        if let Some((list, cursor)) = self.focused_math() {
            math::insert_char(list, cursor, c);
            self.dirty = true;
        }
    }

    pub fn math_insert_fraction(&mut self) {
        if let Some((list, cursor)) = self.focused_math() {
            math::insert_fraction(list, cursor);
            self.dirty = true;
        }
    }

    pub fn math_insert_script(&mut self, which: math::Slot) {
        if let Some((list, cursor)) = self.focused_math() {
            math::insert_script(list, cursor, which);
            self.dirty = true;
        }
    }

    /// Opens a bracket group. `false` when `c` is not an opener, so the
    /// caller can type it literally instead.
    pub fn math_open_group(&mut self, c: char) -> bool {
        let opened = self
            .focused_math()
            .is_some_and(|(list, cursor)| math::insert_group(list, cursor, c));
        if opened {
            self.dirty = true;
        }
        opened
    }

    /// Steps out of the group `c` closes, if the cursor is in one.
    pub fn math_close_group(&mut self, c: char) -> bool {
        let closed = self
            .focused_math()
            .is_some_and(|(list, cursor)| math::close_group(list, cursor, c));
        if closed {
            self.dirty = true;
        }
        closed
    }

    /// A space was typed: turns a trigger word before the cursor into its
    /// structure. `false` means the space is an ordinary space.
    pub fn math_insert_word(&mut self) -> bool {
        let inserted = self
            .focused_math()
            .is_some_and(|(list, cursor)| math::insert_word(list, cursor));
        if inserted {
            self.dirty = true;
        }
        inserted
    }

    /// The focused expression and cursor, read-only, for the shell's
    /// geometry and completion queries.
    pub fn focused_math_view(&self) -> Option<(&math::MathList, &math::MathCursor)> {
        let cursor = self.math.as_ref()?;
        let list = match self.focused()[self.caret.block]
            .inlines()
            .get(self.caret.inline)
        {
            Some(Inline::Math(list)) => list,
            _ => return None,
        };
        Some((list, cursor))
    }

    /// The precise token before the math cursor, including its tree location.
    pub fn math_conversion_query(&self) -> Option<math_conversion::Query> {
        let (list, cursor) = self.focused_math_view()?;
        math_conversion::query_before(list, cursor)
    }

    /// Applies a still-current completion or compact-input rewrite.
    pub fn math_accept_conversion(
        &mut self,
        query: &math_conversion::Query,
        offer: &math_conversion::Offer,
    ) -> bool {
        let accepted = self
            .focused_math()
            .is_some_and(|(list, cursor)| math_conversion::accept(list, cursor, query, offer));
        if accepted {
            self.dirty = true;
        }
        accepted
    }

    fn mutate_math_at(
        &mut self,
        block: usize,
        inline: usize,
        mutation: impl FnOnce(&mut math::MathList) -> bool,
    ) -> bool {
        let Some(Inline::Math(list)) = self
            .blocks
            .get_mut(block)
            .and_then(|block| block.inlines_mut().get_mut(inline))
        else {
            return false;
        };
        let before = list.clone();
        if !mutation(list) || *list == before {
            *list = before;
            return false;
        }
        self.dirty = true;
        true
    }

    pub fn set_math_node_role_at(
        &mut self,
        block: usize,
        inline: usize,
        address: &math::NodeAddress,
        role: math::SymbolRole,
    ) -> bool {
        self.mutate_math_at(block, inline, |list| {
            math::set_node_role(list, address, role)
        })
    }

    pub fn set_math_node_variant_at(
        &mut self,
        block: usize,
        inline: usize,
        address: &math::NodeAddress,
        variant: &str,
    ) -> bool {
        self.mutate_math_at(block, inline, |list| {
            if math::set_node_variant(list, address, variant) {
                return true;
            }
            matches!(math::node_at(list, address), Some(math::MathNode::Sym(c)) if c.is_alphabetic())
                && math::set_node_role(list, address, math::SymbolRole::Variable)
                && math::set_node_variant(list, address, variant)
        })
    }

    pub fn set_math_group_delimiter_at(
        &mut self,
        block: usize,
        inline: usize,
        address: &math::NodeAddress,
        open: char,
    ) -> bool {
        self.mutate_math_at(block, inline, |list| {
            math::set_group_delimiter(list, address, open)
        })
    }

    pub fn set_math_accent_kind_at(
        &mut self,
        block: usize,
        inline: usize,
        address: &math::NodeAddress,
        kind: math::AccentKind,
    ) -> bool {
        self.mutate_math_at(block, inline, |list| {
            math::set_accent_kind(list, address, kind)
        })
    }

    pub fn set_math_big_op_kind_at(
        &mut self,
        block: usize,
        inline: usize,
        address: &math::NodeAddress,
        kind: math::BigOp,
    ) -> bool {
        self.mutate_math_at(block, inline, |list| {
            math::set_big_op_kind(list, address, kind)
        })
    }

    pub fn math_backspace(&mut self) -> Option<math::Removed> {
        let result = self
            .focused_math()
            .map(|(list, cursor)| math::backspace(list, cursor));
        if result == Some(math::Removed::Edited) {
            self.dirty = true;
        }
        result
    }

    pub fn math_delete_forward(&mut self) -> Option<math::Removed> {
        let result = self
            .focused_math()
            .map(|(list, cursor)| math::delete_forward(list, cursor));
        if result == Some(math::Removed::Edited) {
            self.dirty = true;
        }
        result
    }

    pub fn math_move_left(&mut self) -> bool {
        self.focused_math()
            .is_some_and(|(list, cursor)| math::move_left(list, cursor))
    }

    pub fn math_move_right(&mut self) -> bool {
        self.focused_math()
            .is_some_and(|(list, cursor)| math::move_right(list, cursor))
    }

    pub fn math_slot_next(&mut self) -> bool {
        self.focused_math()
            .is_some_and(|(list, cursor)| math::slot_next(list, cursor))
    }

    pub fn math_slot_prev(&mut self) -> bool {
        self.focused_math()
            .is_some_and(|(list, cursor)| math::slot_prev(list, cursor))
    }

    pub fn math_pop(&mut self) -> bool {
        self.focused_math()
            .is_some_and(|(list, cursor)| math::pop_level(list, cursor))
    }

    /// Enters the atom immediately before the caret, if there is one,
    /// putting the math cursor at its end — arrowing left into an
    /// expression continues into its content rather than stepping over
    /// it, which is the mirror of how the cursor steps out at its edges.
    pub fn enter_math_before(&mut self) -> bool {
        self.clamp_caret();
        let block = self.caret.block;
        let flat = self.caret_flat(block);
        if flat == 0 {
            return false;
        }
        let (inline, _) = self.flat_to_pos(block, flat - 1);
        let list_len = match self.focused()[block].inlines().get(inline) {
            Some(Inline::Math(list)) => list.len(),
            _ => return false,
        };
        // Offset 0 identifies the atom; offset 1 is past it, as math_exit uses.
        self.set_caret(block, inline, 0);
        self.math = Some(math::MathCursor {
            path: Vec::new(),
            index: list_len,
        });
        true
    }

    /// Enters the atom immediately after the caret, cursor at its start.
    pub fn enter_math_after(&mut self) -> bool {
        self.clamp_caret();
        let block = self.caret.block;
        let flat = self.caret_flat(block);
        if flat >= self.block_flat_len(block) {
            return false;
        }
        let (inline, _) = self.flat_to_pos(block, flat);
        if !matches!(
            self.focused()[block].inlines().get(inline),
            Some(Inline::Math(_))
        ) {
            return false;
        }
        self.set_caret(block, inline, 0);
        self.math = Some(math::MathCursor::default());
        true
    }

    /// Enters the atom at `block`/`inline` with an already-resolved
    /// cursor — the click path, where the geometry decided where inside the
    /// expression the cursor goes.
    pub fn enter_math_at(&mut self, block: usize, inline: usize, mut cursor: math::MathCursor) {
        if !matches!(
            self.blocks
                .get(block)
                .and_then(|block| block.inlines().get(inline)),
            Some(Inline::Math(_))
        ) {
            return;
        }
        self.set_caret(block, inline, 0);
        let list = match self.blocks[self.caret.block]
            .inlines()
            .get(self.caret.inline)
        {
            Some(Inline::Math(list)) => list,
            _ => return,
        };
        math::clamp(list, &mut cursor);
        self.math = Some(cursor);
    }

    fn math_exit_at(&mut self, offset: usize) {
        self.clamp_caret();
        let block = self.caret.block;
        let inline = self.caret.inline;
        if !matches!(
            self.focused()[block].inlines().get(inline),
            Some(Inline::Math(_))
        ) {
            self.math = None;
            return;
        }
        self.math = None;
        self.caret.offset = offset;
        self.refresh_context();
    }

    pub fn math_exit_before(&mut self) {
        self.math_exit_at(0);
    }

    pub fn math_exit_after(&mut self) {
        self.math_exit_at(1);
    }

    pub fn math_path_names(&mut self) -> Vec<&'static str> {
        self.focused_math()
            .map_or_else(Vec::new, |(_, cursor)| math::path_names(cursor))
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
    /// empty run outside the placeholder, caret in bounds. Checked over the
    /// document's blocks and every note body, with the caret read against
    /// whichever scope is focused.
    fn assert_invariants(d: &Document) {
        assert!(!d.blocks.is_empty());
        for block in d
            .blocks
            .iter()
            .chain(d.notes.iter().flat_map(|note| note.body.iter()))
        {
            assert!(!block.inlines().is_empty());
            for (i, run) in block.inlines().iter().enumerate() {
                assert!(!(run.text().is_empty() && (block.inlines().len() > 1 || i != 0)));
            }
        }
        for note in &d.notes {
            assert!(!note.body.is_empty());
        }
        assert!(d.caret.block < d.focused().len());
        let block_runs = d.focused()[d.caret.block].inlines();
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

    #[test]
    fn an_atom_counts_as_one_char() {
        let mut d = doc();
        d.blocks[0] = Block::Paragraph(vec![
            plain_run("ab"),
            Inline::Math(vec![math::MathNode::Sym('x')]),
            plain_run("cd"),
        ]);
        assert_eq!(d.block_len(0), 5);
        assert_eq!(d.block_text(0), format!("ab{ATOM}cd"));
        d.set_caret(0, 0, 2);
        d.move_right();
        assert_eq!(d.caret_position().offset, 3);
        d.move_left();
        assert!(matches!(
            d.blocks[0].inlines()[d.caret.inline],
            Inline::Math(_)
        ));
    }

    #[test]
    fn inserting_an_inline_atom_splits_the_run() {
        let mut d = doc();
        d.insert_text("abcd");
        d.set_caret(0, 0, 2);
        d.insert_inline_math();
        assert!(matches!(
            d.blocks[0].inlines(),
            [Inline::Text(Text { text: left, .. }), Inline::Math(_), Inline::Text(Text { text: right, .. })]
                if left == "ab" && right == "cd"
        ));
        assert_eq!((d.caret.inline, d.caret.offset), (1, 0));
        assert!(d.math.is_some());
    }

    #[test]
    fn typing_beside_an_atom_never_merges_into_it() {
        let mut d = doc();
        d.insert_inline_math();
        d.set_caret(0, 0, 1);
        d.insert_text("x");
        assert!(matches!(d.blocks[0].inlines()[0], Inline::Math(_)));
        assert_eq!(d.block_text(0), format!("{ATOM}x"));
        assert!(
            matches!(d.blocks[0].inlines()[1], Inline::Text(Text { ref text, .. }) if text == "x")
        );
    }

    #[test]
    fn backspace_enters_a_nonempty_atom_then_removes_it_only_when_empty() {
        let mut d = doc();
        d.blocks[0] = Block::Paragraph(vec![
            plain_run("before"),
            Inline::Math(vec![math::MathNode::Sym('x')]),
            plain_run("after"),
        ]);
        d.set_caret(0, 2, 0);

        d.backspace();
        assert!(matches!(d.blocks[0].inlines()[1], Inline::Math(ref list) if list.is_empty()));
        assert_eq!(d.math, Some(math::MathCursor::default()));

        d.math_exit_after();
        d.backspace();
        assert_eq!(d.block_text(0), "beforeafter");
        assert!(
            d.blocks[0]
                .inlines()
                .iter()
                .all(|inline| !matches!(inline, Inline::Math(_)))
        );
    }

    #[test]
    fn delete_forward_enters_a_nonempty_atom_then_removes_it_only_when_empty() {
        let mut d = doc();
        d.blocks[0] = Block::Paragraph(vec![
            plain_run("before"),
            Inline::Math(vec![math::MathNode::Sym('x')]),
            plain_run("after"),
        ]);
        d.set_caret(0, 0, 6);

        d.delete_forward();
        assert!(matches!(d.blocks[0].inlines()[1], Inline::Math(ref list) if list.is_empty()));
        assert_eq!(d.math, Some(math::MathCursor::default()));

        d.math_exit_before();
        d.delete_forward();
        assert_eq!(d.block_text(0), "beforeafter");
    }

    #[test]
    fn normal_delete_starts_inside_a_math_atom() {
        let mut d = doc();
        d.blocks[0] = Block::Paragraph(vec![Inline::Math(vec![math::MathNode::Sym('x')])]);
        d.set_caret(0, 0, 0);

        d.delete_char();

        assert!(matches!(d.blocks[0].inlines()[0], Inline::Math(ref list) if list.is_empty()));
        assert_eq!(d.math, Some(math::MathCursor::default()));
    }

    #[test]
    fn deleting_around_an_empty_display_atom_preserves_the_math_block() {
        let mut d = doc();
        d.insert_math_block();
        d.math_exit_after();

        d.backspace();
        assert!(d.blocks[0].is_math());
        assert!(matches!(d.blocks[0].inlines(), [Inline::Math(list)] if list.is_empty()));
        assert_eq!(d.block_text(0), ATOM.to_string());
    }

    #[test]
    fn backspace_from_prose_after_display_math_enters_without_merging_blocks() {
        let mut d = doc();
        d.blocks = vec![
            Block::Math(vec![Inline::Math(vec![math::MathNode::Sym('x')])]),
            Block::Paragraph(vec![plain_run("after")]),
        ];
        d.set_caret(1, 0, 0);

        d.backspace();

        assert_eq!(d.blocks.len(), 2);
        assert!(matches!(d.blocks[0], Block::Math(ref inlines)
            if matches!(inlines.as_slice(), [Inline::Math(list)] if list.is_empty())));
        assert_eq!(d.block_text(1), "after");
        assert_eq!(d.caret.block, 0);
        assert_eq!(d.math, Some(math::MathCursor::default()));
    }

    #[test]
    fn delete_from_prose_before_display_math_enters_without_merging_blocks() {
        let mut d = doc();
        d.blocks = vec![
            Block::Paragraph(vec![plain_run("before")]),
            Block::Math(vec![Inline::Math(vec![math::MathNode::Sym('x')])]),
        ];
        d.set_caret(0, 0, 6);

        d.delete_forward();

        assert_eq!(d.blocks.len(), 2);
        assert_eq!(d.block_text(0), "before");
        assert!(matches!(d.blocks[1], Block::Math(ref inlines)
            if matches!(inlines.as_slice(), [Inline::Math(list)] if list.is_empty())));
        assert_eq!(d.caret.block, 1);
        assert_eq!(d.math, Some(math::MathCursor::default()));
    }

    #[test]
    fn delete_after_display_math_retains_the_block_boundary() {
        let mut d = doc();
        d.blocks = vec![
            Block::Math(vec![Inline::Math(vec![math::MathNode::Sym('x')])]),
            Block::Paragraph(vec![plain_run("after")]),
        ];
        d.set_caret(0, 0, 1);

        d.delete_forward();

        assert_eq!(d.blocks.len(), 2);
        assert!(d.blocks[0].is_math());
        assert_eq!(d.block_text(1), "after");
        assert_eq!(d.caret.block, 0);
        assert_eq!(d.caret.offset, 1);
        assert!(d.math.is_none());
    }

    #[test]
    fn a_math_block_that_gains_prose_demotes_to_a_paragraph() {
        let mut d = doc();
        d.blocks[0] = Block::Math(vec![
            Inline::Math(vec![math::MathNode::Sym('x')]),
            plain_run(" prose"),
        ]);
        d.enforce();
        assert!(matches!(d.blocks[0], Block::Paragraph(_)));
        assert!(matches!(d.blocks[0].inlines()[0], Inline::Math(_)));
    }

    #[test]
    fn insert_math_block_replaces_an_empty_block() {
        let mut d = doc();
        d.insert_math_block();
        assert_eq!(d.blocks.len(), 1);
        assert!(d.blocks[0].is_math());
        assert_eq!((d.caret.block, d.caret.inline, d.caret.offset), (0, 0, 0));
        assert!(d.math.is_some());
    }

    #[test]
    fn math_ops_route_into_the_focused_atom() {
        let mut d = doc();
        d.insert_inline_math();
        d.math_insert_char('1');
        d.math_insert_fraction();
        d.math_insert_char('2');
        assert_eq!(
            d.blocks[0].inlines()[0],
            Inline::Math(vec![math::MathNode::Frac {
                num: vec![math::MathNode::Sym('1')],
                den: vec![math::MathNode::Sym('2')],
            }])
        );
        d.math_exit_after();
        assert!(d.math.is_none());
        assert_eq!(d.caret.offset, 1);
    }

    #[test]
    fn an_opener_routes_into_a_group_and_a_stray_closer_does_not() {
        let mut d = doc();
        d.insert_inline_math();

        assert!(d.math_open_group('('));
        assert_eq!(
            d.blocks[0].inlines()[0],
            Inline::Math(vec![math::MathNode::Group {
                open: '(',
                close: ')',
                body: Vec::new(),
            }])
        );
        assert!(!d.math_close_group(']'));
    }

    #[test]
    fn a_space_after_a_trigger_word_builds_its_structure() {
        let mut d = doc();
        d.insert_inline_math();
        for c in ['s', 'q', 'r', 't'] {
            d.math_insert_char(c);
        }

        assert!(d.math_insert_word());
        assert_eq!(
            d.blocks[0].inlines()[0],
            Inline::Math(vec![math::MathNode::Sqrt { body: Vec::new() }])
        );
    }

    #[test]
    fn arrowing_back_over_an_atom_enters_it() {
        let mut d = doc();
        d.blocks[0] = Block::Paragraph(vec![
            plain_run("before"),
            Inline::Math(vec![math::MathNode::Sym('x'), math::MathNode::Sym('y')]),
            plain_run("after"),
        ]);
        d.set_caret(0, 1, 1);

        assert!(d.enter_math_before());
        assert_eq!(
            d.caret,
            Caret {
                block: 0,
                inline: 1,
                offset: 0,
                style: Style::PLAIN
            }
        );
        assert_eq!(
            d.math,
            Some(math::MathCursor {
                path: Vec::new(),
                index: 2,
            })
        );
    }

    #[test]
    fn click_entry_clamps_against_a_deep_denominator_not_the_root() {
        let mut d = doc();
        d.blocks[0] = Block::Paragraph(vec![Inline::Math(vec![math::MathNode::Frac {
            num: Vec::new(),
            den: vec![math::MathNode::Frac {
                num: Vec::new(),
                den: vec![
                    math::MathNode::Sym('a'),
                    math::MathNode::Sym('b'),
                    math::MathNode::Sym('c'),
                ],
            }],
        }])]);
        let cursor = math::MathCursor {
            path: vec![
                math::Step {
                    index: 0,
                    slot: math::Slot::Den,
                },
                math::Step {
                    index: 0,
                    slot: math::Slot::Den,
                },
            ],
            index: usize::MAX,
        };

        d.enter_math_at(0, 0, cursor.clone());

        assert_eq!(
            d.math,
            Some(math::MathCursor {
                path: cursor.path,
                index: 3,
            })
        );
    }

    #[test]
    fn document_accepts_an_explicit_compact_conversion_offer() {
        let mut d = doc();
        d.insert_inline_math();
        for c in "e0".chars() {
            d.math_insert_char(c);
        }
        let query = d.math_conversion_query().unwrap();
        let offer = math_conversion::offers(&query).offers[0].clone();

        assert!(d.math_accept_conversion(&query, &offer));
        assert!(matches!(
            d.blocks[0].inlines()[0],
            Inline::Math(ref list)
                if matches!(list.as_slice(), [math::MathNode::Resolved {
                    id,
                    role: math::SymbolRole::Constant,
                    variant,
                    body,
                }] if id == "vacuum_permittivity"
                    && variant == "plain"
                    && matches!(body.as_slice(), [math::MathNode::Script { sub: Some(sub), .. }]
                        if sub == &vec![math::MathNode::Sym('0')]))
        ));
    }

    #[test]
    fn exact_math_node_mutations_are_dirty_without_moving_the_caret() {
        let mut d = doc();
        d.blocks = vec![
            Block::Paragraph(vec![
                plain_run("before"),
                Inline::Math(vec![
                    math::MathNode::Sym('x'),
                    math::MathNode::Group {
                        open: '(',
                        close: ')',
                        body: vec![math::MathNode::Sym('y')],
                    },
                    math::MathNode::Accent {
                        kind: math::AccentKind::Vector,
                        body: vec![math::MathNode::Sym('z')],
                    },
                    math::MathNode::BigOp {
                        kind: math::BigOp::Integral,
                        lower: Vec::new(),
                        upper: Vec::new(),
                    },
                    math::MathNode::Sym('q'),
                ]),
                plain_run("after"),
            ]),
            Block::Paragraph(vec![plain_run("caret")]),
        ];
        d.set_caret(1, 0, 2);
        let caret = d.caret;
        let at = |index| math::NodeAddress {
            path: Vec::new(),
            index,
        };

        assert!(d.set_math_node_variant_at(0, 1, &at(0), "bold"));
        assert!(matches!(
            &d.blocks[0].inlines()[1],
            Inline::Math(list)
                if matches!(&list[0], math::MathNode::Resolved {
                    role: math::SymbolRole::Variable,
                    variant,
                    ..
                } if variant == "bold")
        ));
        assert!(d.set_math_node_role_at(0, 1, &at(0), math::SymbolRole::Constant));
        assert!(d.set_math_group_delimiter_at(0, 1, &at(1), '['));
        assert!(d.set_math_accent_kind_at(0, 1, &at(2), math::AccentKind::Dot));
        assert!(d.set_math_big_op_kind_at(0, 1, &at(3), math::BigOp::ContourIntegral));
        assert!(d.is_dirty());
        assert_eq!(d.caret, caret);
        assert!(d.math.is_none());
        assert!(matches!(
            &d.blocks[0].inlines()[1],
            Inline::Math(list)
                if matches!(&list[0], math::MathNode::Resolved {
                    role: math::SymbolRole::Constant,
                    variant,
                    ..
                } if variant == "bold")
                    && matches!(&list[1], math::MathNode::Group { open: '[', close: ']', .. })
                    && matches!(&list[2], math::MathNode::Accent { kind: math::AccentKind::Dot, .. })
                    && matches!(&list[3], math::MathNode::BigOp { kind: math::BigOp::ContourIntegral, .. })
        ));

        d.dirty = false;
        assert!(!d.set_math_node_role_at(0, 1, &at(0), math::SymbolRole::Constant));
        assert!(!d.set_math_node_variant_at(0, 1, &at(0), "bold"));
        assert!(!d.set_math_group_delimiter_at(0, 1, &at(1), '['));
        assert!(!d.set_math_accent_kind_at(0, 1, &at(2), math::AccentKind::Dot));
        assert!(!d.set_math_big_op_kind_at(0, 1, &at(3), math::BigOp::ContourIntegral));
        assert!(!d.set_math_node_variant_at(0, 1, &at(4), "missing"));
        assert!(!d.set_math_node_role_at(9, 9, &at(0), math::SymbolRole::Variable));
        assert!(!d.is_dirty());
        assert_eq!(d.caret, caret);
        assert!(matches!(
            &d.blocks[0].inlines()[1],
            Inline::Math(list) if matches!(list.get(4), Some(math::MathNode::Sym('q')))
        ));
    }

    #[test]
    fn a_raw_greek_letter_accepts_and_persists_a_variant() {
        let mut d = doc();
        d.blocks[0] = Block::Paragraph(vec![Inline::Math(vec![math::MathNode::Sym('α')])]);
        let address = math::NodeAddress {
            path: Vec::new(),
            index: 0,
        };

        assert!(d.set_math_node_variant_at(0, 0, &address, "bold"));
        let Inline::Math(list) = &d.blocks[0].inlines()[0] else {
            panic!("the targeted inline stays math");
        };
        assert!(matches!(
            list.as_slice(),
            [math::MathNode::Resolved {
                id,
                role: math::SymbolRole::Variable,
                variant,
                body,
            }] if id == "α"
                && variant == "bold"
                && body == &vec![math::MathNode::Sym('𝛂')]
        ));
        assert_eq!(
            math_notation::parse(&math_notation::print(list)),
            list.clone(),
            "the promoted identity and variant survive canonical notation"
        );
    }

    #[test]
    fn exact_block_type_changes_preserve_runs_and_caret() {
        let mut d = doc();
        let content = vec![
            plain_run("a"),
            Inline::Math(vec![math::MathNode::Sym('x')]),
            bold_run("b"),
        ];
        d.blocks = vec![
            Block::Paragraph(content.clone()),
            Block::Paragraph(vec![plain_run("caret")]),
        ];
        d.set_caret(1, 0, 3);
        let caret = d.caret;

        assert!(d.set_block_heading_at(0, Some(2)));
        assert!(matches!(
            &d.blocks[0],
            Block::Heading { level: 2, content: actual } if actual == &content
        ));
        assert_eq!(d.caret, caret);

        d.dirty = false;
        assert!(!d.set_block_heading_at(0, Some(2)));
        assert!(!d.is_dirty());
        assert!(d.set_block_code_at(0, true));
        assert!(matches!(
            &d.blocks[0],
            Block::CodeLine { content, first: true, lang: None }
                if content.len() == 3 && matches!(content[1], Inline::Math(_))
        ));
        assert_eq!(d.caret, caret);

        assert!(d.set_block_code_at(0, false));
        assert!(matches!(&d.blocks[0], Block::Paragraph(content) if content.len() == 3));
        assert_eq!(d.caret, caret);
    }

    #[test]
    fn enter_math_before_ignores_prose() {
        let mut d = doc();
        d.blocks[0] = Block::Paragraph(vec![plain_run("text")]);
        d.set_caret(0, 0, 1);

        assert!(!d.enter_math_before());
        assert!(d.math.is_none());
    }

    #[test]
    fn a_stale_focus_is_dropped_by_clamp() {
        let mut d = doc();
        d.insert_inline_math();
        d.blocks.remove(0);
        d.move_right();
        assert!(d.math.is_none());
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

    #[test]
    fn badge_color_changes_only_the_selected_badge() {
        let badge = Style {
            badge: true,
            ..Style::PLAIN
        };
        let mut d = doc();
        d.blocks[0] = Block::Paragraph(vec![
            Inline::Text(Text {
                text: "A".into(),
                style: badge,
            }),
            Inline::Text(Text {
                text: "B".into(),
                style: badge,
            }),
        ]);
        let first = FlatRange::new(
            FlatPos {
                block: 0,
                offset: 0,
            },
            FlatPos {
                block: 0,
                offset: 1,
            },
        );

        d.set_badge_color(first, BadgeColor::Blue);
        let styled = runs(&d.blocks[0]);
        assert_eq!(styled.len(), 2);
        assert_eq!(styled[0].1.badge_color, BadgeColor::Blue);
        assert_eq!(styled[1].1.badge_color, BadgeColor::Orange);

        d.toggle_style_range(first, badge);
        assert_eq!(runs(&d.blocks[0])[0].1, Style::PLAIN);
        assert_invariants(&d);
    }

    #[test]
    fn copying_a_range_with_an_expression_yields_its_notation() {
        let mut d = doc();
        d.blocks = vec![Block::Paragraph(vec![
            plain_run("The value is "),
            Inline::Math(math_notation::parse("a/b")),
            plain_run(" and here."),
        ])];
        let copied = d.range_text(d.line_range(0, 0));
        assert!(copied.contains("$a/b$"), "copied: {copied:?}");
        assert!(!copied.contains(ATOM), "no object-replacement character");
        assert_eq!(copied, "The value is $a/b$ and here.");
    }

    #[test]
    fn an_expression_survives_a_copy_and_a_paste() {
        let original = math_notation::parse("a/b");
        let mut source = doc();
        source.blocks = vec![Block::Paragraph(vec![
            plain_run("before "),
            Inline::Math(original.clone()),
            plain_run(" after"),
        ])];
        let copied = source.range_text(source.line_range(0, 0));
        assert_eq!(copied, "before $a/b$ after");

        // Paste back the way the shell does for text this app itself produced:
        // `insert_notation` reads `$…$` as math again, so the copied notation
        // comes back as an expression.
        let mut target = doc();
        target.insert_notation(&copied);
        assert_eq!(
            target.blocks[0].inlines(),
            &[
                Inline::Text(Text {
                    text: "before ".into(),
                    style: Style::PLAIN,
                }),
                Inline::Math(original),
                Inline::Text(Text {
                    text: " after".into(),
                    style: Style::PLAIN,
                }),
            ]
        );
    }

    #[test]
    fn a_selection_that_ends_before_an_expression_does_not_copy_it() {
        let mut d = doc();
        d.blocks = vec![Block::Paragraph(vec![
            plain_run("abc"),
            Inline::Math(math_notation::parse("a/b")),
            plain_run("def"),
        ])];
        // "abc" occupies flat offsets 0..=2; the atom sits at offset 3, so a
        // range ending at offset 3 must not pull it in.
        let copied = d.range_text(FlatRange::new(d.position(0, 0), d.position(0, 3)));
        assert_eq!(copied, "abc");
    }

    #[test]
    fn copying_prose_escapes_a_literal_dollar() {
        let mut d = doc();
        d.blocks = vec![Block::Paragraph(vec![plain_run("costs $40 and $12")])];
        let copied = d.range_text(d.line_range(0, 0));
        // A literal `$` must not look like math on the way back, so it is
        // escaped exactly as `markdown::serialize` writes it to disk.
        assert_eq!(copied, "costs \\$40 and \\$12");
    }

    #[test]
    fn pasting_a_price_list_stays_literal() {
        let mut d = doc();
        d.insert_text("costs $40 and $12");
        // Foreign text is inserted byte-for-byte: one text run, no expression.
        let block = &d.blocks[0];
        assert_eq!(
            runs(block),
            vec![("costs $40 and $12".into(), Style::PLAIN)]
        );
        assert!(!block.inlines().iter().any(|r| matches!(r, Inline::Math(_))));
    }

    #[test]
    fn notation_reads_an_escaped_dollar_as_a_dollar() {
        let mut d = doc();
        d.insert_notation("costs \\$40 and \\$12");
        // `\$` collapses to `$` with no expression and no backslash left over.
        let block = &d.blocks[0];
        assert_eq!(
            runs(block),
            vec![("costs $40 and $12".into(), Style::PLAIN)]
        );
        assert!(!block.inlines().iter().any(|r| matches!(r, Inline::Math(_))));
    }

    // ---- sidenote tests ------------------------------------------------

    #[test]
    fn an_anchor_costs_one_flat_position() {
        let mut d = doc();
        d.blocks[0] = Block::Paragraph(vec![
            plain_run("ab"),
            Inline::Note("1".into()),
            plain_run("cd"),
        ]);
        assert_eq!(d.block_len(0), 5);
        assert_eq!(d.block_text(0), format!("ab{ATOM}cd"));
        d.set_caret(0, 0, 2);
        d.move_right();
        assert_eq!(d.caret_position().offset, 3);
        d.move_left();
        assert!(matches!(
            d.blocks[0].inlines()[d.caret.inline],
            Inline::Note(_)
        ));
    }

    #[test]
    fn inserting_a_sidenote_picks_the_lowest_free_label() {
        let mut d = doc();
        assert_eq!(d.insert_sidenote(), Some("1".into()));
        assert_eq!(d.insert_sidenote(), Some("2".into()));
        assert_eq!(d.insert_sidenote(), Some("3".into()));
        assert_eq!(
            d.notes.iter().map(|n| n.label.as_str()).collect::<Vec<_>>(),
            vec!["1", "2", "3"]
        );

        // Delete the anchor labelled "1" — its note is dropped with it, so
        // "1" is free again and the next insert reclaims it.
        d.delete_range(FlatRange::new(d.position(0, 0), d.position(0, 1)));
        assert_eq!(d.notes.len(), 2);
        assert_eq!(d.insert_sidenote(), Some("1".into()));
    }

    #[test]
    fn deleting_an_anchor_drops_its_note() {
        let mut d = doc();
        d.insert_text("note");
        d.set_caret(0, 0, 0);
        let _ = d.insert_sidenote();
        assert_eq!(d.notes.len(), 1);
        assert_eq!(d.block_text(0), format!("{ATOM}note"));

        d.delete_range(FlatRange::new(d.position(0, 0), d.position(0, 1)));
        assert!(d.notes.is_empty(), "deleting the anchor drops the note");
        assert_eq!(d.block_text(0), "note");
    }

    #[test]
    fn a_note_that_never_had_an_anchor_is_left_alone() {
        let mut d = doc();
        d.notes.push(Sidenote {
            label: "1".into(),
            body: vec![Block::Paragraph(vec![plain_run("orphan")])],
            anchored: false,
        });
        d.insert_text("hello");
        d.enforce();
        assert_eq!(d.notes.len(), 1, "a stray note is not an edit's to drop");
        assert_eq!(d.notes[0].label, "1");
        assert_eq!(
            d.notes[0].body,
            vec![Block::Paragraph(vec![plain_run("orphan")])]
        );
    }

    #[test]
    fn typing_into_a_focused_note_edits_that_notes_body_and_leaves_the_documents_blocks_untouched()
    {
        let mut d = doc();
        d.insert_text("body");
        d.set_caret(0, 0, 0);
        let _ = d.insert_sidenote();
        let blocks_before = d.blocks.clone();
        d.focus = Focus::Note(0);
        d.insert_text("a note");
        assert_eq!(
            d.notes[0].body,
            vec![Block::Paragraph(vec![plain_run("a note")])]
        );
        assert_eq!(d.blocks, blocks_before, "the body is untouched");
        assert_invariants(&d);
    }

    #[test]
    fn typing_into_the_body_with_focus_on_body_leaves_every_note_untouched() {
        let mut d = doc();
        d.insert_text("body");
        d.set_caret(0, 0, 0);
        let _ = d.insert_sidenote();
        let notes_before = d.notes.clone();
        d.insert_text("more ");
        assert_eq!(d.notes, notes_before, "no note changes");
        assert_eq!(d.block_text(0), format!("{ATOM}more body"));
        assert_invariants(&d);
    }

    #[test]
    fn deleting_the_anchor_of_the_focused_note_drops_the_note_resets_focus_to_body_and_leaves_the_caret_in_bounds()
     {
        let mut d = doc();
        d.insert_text("note");
        d.set_caret(0, 0, 0);
        let _ = d.insert_sidenote();
        d.focus = Focus::Note(0);
        // Remove the anchor from the body; the next enforce drops the note
        // and the clamp must walk focus back to the body.
        d.blocks[0]
            .inlines_mut()
            .retain(|run| !matches!(run, Inline::Note(_)));
        d.enforce();
        assert!(d.notes.is_empty(), "dropping the anchor drops the note");
        assert_eq!(d.focus, Focus::Body, "focus falls back to the body");
        assert!(d.caret.block < d.blocks.len(), "caret stays in bounds");
        assert_invariants(&d);
    }

    #[test]
    fn pressing_enter_inside_a_note_leaves_the_body_at_one_paragraph() {
        let mut d = doc();
        d.insert_text("body");
        d.set_caret(0, 0, 0);
        let _ = d.insert_sidenote();
        d.focus = Focus::Note(0);
        d.insert_text("note text");
        d.set_caret(0, 0, 4);
        d.newline();
        assert_eq!(d.notes[0].body.len(), 1, "a note stays one paragraph");
        assert_eq!(
            runs(&d.notes[0].body[0]),
            vec![("note text".into(), Style::PLAIN)]
        );
        assert_invariants(&d);
    }

    #[test]
    fn insert_sidenote_while_focused_in_a_note_inserts_nothing() {
        let mut d = doc();
        d.insert_text("body");
        d.set_caret(0, 0, 0);
        let _ = d.insert_sidenote();
        d.focus = Focus::Note(0);
        assert_eq!(d.insert_sidenote(), None);
        assert_eq!(d.notes.len(), 1, "no note inside a note");
        assert_eq!(d.block_text(0), format!("{ATOM}body"));
    }
}
