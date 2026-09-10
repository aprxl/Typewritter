//! The document model: blocks, inline runs, the style-context caret, and
//! the editing operations on top of them. Pure model — no IO, no renderer.
//!
//! See `tasks/01-document-model.md` for the spec this implements, and
//! `docs/SPEC.md` §4.2 for the style-context machine.

use std::path::{Path, PathBuf};

pub mod code;
mod code_metadata;
pub mod decoration;
pub mod layout;
pub mod markdown;
pub mod math;
pub mod math_conversion;
pub mod math_layout;
pub mod math_notation;
pub mod math_paint;
pub mod math_style;
pub mod math_symbols;
pub mod outline;
pub mod table;
pub mod table_edit;

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
    /// The document's own blocks — the file's body. Never empty. Private:
    /// reach it through [`Self::body`]/[`Self::body_mut`] (what is in the
    /// file) or [`Self::scope`]/[`Self::scope_mut`] (where the caret is).
    body: Vec<Block>, // invariant: never empty
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
    /// Display math. `tag` is the author's equation label (`eq:gain`, spelled
    /// `#eq:gain` in the fence info string), used for numbering and for
    /// `@eq:…` references. `list` holds exactly one math atom.
    Math {
        list: Vec<Inline>,
        tag: Option<String>,
    },
    Heading {
        level: u8,
        content: Vec<Inline>,
        /// Editor state, not content: whether the heading's body — every
        /// block up to the next heading of level <= its own — is folded
        /// away. `parse` sets it false and `serialize` ignores it; it lives
        /// on the block so fold state survives edits above it and rides the
        /// same undo snapshots as everything else.
        folded: bool,
    }, // level 1..=4
    CodeLine {
        content: Vec<Inline>,
        first: bool,
        lang: Option<String>,
    }, // one line of a fenced code block
    /// One visual row of a table. Each entry is a [`table::Cell`] — a
    /// container of lines of inline runs, so a cell holds rich prose and math
    /// without the run list having to know a grid exists. All rows share one
    /// [`std::sync::Arc`] of the table's settings, so re-shaping a table
    /// re-clones nothing per row.
    TableRow {
        cells: Vec<table::Cell>,
        first: bool,
        settings: std::sync::Arc<table::TableSettings>,
    },
    /// One list item. Items are flat blocks; nested lists are out of scope.
    ListItem {
        marker: ListMarker,
        content: Vec<Inline>,
    },
}

/// What kind of list an item belongs to. `Number` carries the item's
/// 1-based ordinal within its run of ordered items — layout never counts,
/// and serialization renumbers, so the stored number and the printed one
/// agree after a round trip. A task's `done` is content, not UI state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ListMarker {
    Bullet,
    Number(u32),
    Task { done: bool },
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
    /// A reference to a tagged display equation: `@eq:gain` on disk, the
    /// referenced equation's number on the page. Opaque like an anchor: one
    /// flat position, deleted as a whole, never split mid-label.
    EqRef(String),
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
    pub syntax: code::CodeStyle,
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
        syntax: code::CodeStyle::PLAIN,
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

    /// Saved code annotations do not add style-context stops to arrow motions.
    fn same_context(self, other: Self) -> bool {
        Self {
            syntax: code::CodeStyle::PLAIN,
            ..self
        } == Self {
            syntax: code::CodeStyle::PLAIN,
            ..other
        }
    }

    pub fn is_plain(&self) -> bool {
        *self == Style::PLAIN
    }
}

impl Block {
    /// The block's own runs. A table row has none — its content is its cells,
    /// reached through [`Block::cells`] — so this is empty for one. Every
    /// reader that has to see a row's content uses [`Block::flat_len`],
    /// `self.unit_*`, or `block_runs`.
    pub fn inlines(&self) -> &[Inline] {
        match self {
            Block::Paragraph(inlines)
            | Block::Divider(inlines)
            | Block::Math { list: inlines, .. }
            | Block::Heading {
                content: inlines, ..
            }
            | Block::ListItem {
                content: inlines, ..
            } => inlines,
            Block::CodeLine { content, .. } => content,
            Block::TableRow { .. } => &[],
        }
    }

    /// The block's own runs, mutable. A table row has no run list to hand
    /// back: its content is edited through [`Block::cells_mut`].
    pub fn inlines_mut(&mut self) -> &mut Vec<Inline> {
        match self {
            Block::Paragraph(inlines)
            | Block::Divider(inlines)
            | Block::Math { list: inlines, .. }
            | Block::Heading {
                content: inlines, ..
            }
            | Block::ListItem {
                content: inlines, ..
            } => inlines,
            Block::CodeLine { content, .. } => content,
            Block::TableRow { .. } => {
                panic!("a table row's content is its cells, not a run list")
            }
        }
    }

    /// A table row's cells; empty for every other block.
    pub fn cells(&self) -> &[table::Cell] {
        match self {
            Block::TableRow { cells, .. } => cells,
            _ => &[],
        }
    }

    pub fn cells_mut(&mut self) -> Option<&mut Vec<table::Cell>> {
        match self {
            Block::TableRow { cells, .. } => Some(cells),
            _ => None,
        }
    }

    /// Flat length of the block's contents, a table row included.
    pub fn flat_len(&self) -> usize {
        match self {
            Block::TableRow { cells, .. } => cells.iter().map(table::Cell::flat_len).sum(),
            block => block.inlines().iter().map(flat_len).sum(),
        }
    }

    /// Positions the block's `unit` costs. A unit is a run, or — in a table
    /// row — a cell, so one arithmetic addresses both.
    pub fn unit_len(&self, unit: usize) -> usize {
        match self {
            Block::TableRow { cells, .. } => cells.get(unit).map_or(0, table::Cell::flat_len),
            block => block.inlines().get(unit).map_or(0, flat_len),
        }
    }

    pub fn unit_count(&self) -> usize {
        match self {
            Block::TableRow { cells, .. } => cells.len(),
            block => block.inlines().len(),
        }
    }

    /// `(unit, offset within that unit)` for a flat position.
    pub fn flat_to_unit(&self, flat: usize) -> (usize, usize) {
        let mut position = 0;
        for unit in 0..self.unit_count() {
            let len = self.unit_len(unit);
            if flat < position + len {
                return (unit, flat - position);
            }
            position += len;
        }
        let last = self.unit_count().saturating_sub(1);
        (last, self.unit_len(last))
    }

    pub fn is_heading(&self) -> bool {
        matches!(self, Block::Heading { .. })
    }

    pub fn is_folded(&self) -> bool {
        matches!(self, Block::Heading { folded: true, .. })
    }

    pub fn is_divider(&self) -> bool {
        matches!(self, Block::Divider(_))
    }

    pub fn is_math(&self) -> bool {
        matches!(self, Block::Math { .. })
    }

    pub fn is_code(&self) -> bool {
        matches!(self, Block::CodeLine { .. })
    }

    pub fn is_table(&self) -> bool {
        matches!(self, Block::TableRow { .. })
    }

    pub fn table_first(&self) -> bool {
        matches!(self, Block::TableRow { first: true, .. })
    }

    pub fn table_settings(&self) -> Option<&table::TableSettings> {
        match self {
            Block::TableRow { settings, .. } => Some(&**settings),
            _ => None,
        }
    }

    pub fn table_settings_arc(&self) -> Option<&std::sync::Arc<table::TableSettings>> {
        match self {
            Block::TableRow { settings, .. } => Some(settings),
            _ => None,
        }
    }
}

/// The extent of a folded heading's body: every block up to — but not
/// including — the next heading of level <= its own, or the end of the
/// document. The same extent `TextObject::InnerHeading` gives, so folding
/// and `ih` can never disagree about what a section is.
pub fn fold_region_end(blocks: &[Block], heading: usize) -> usize {
    let Some(Block::Heading { level, .. }) = blocks.get(heading) else {
        return heading + 1;
    };
    let level = *level;
    (heading + 1..blocks.len())
        .find(|&index| matches!(blocks[index], Block::Heading { level: next, .. } if next <= level))
        .unwrap_or(blocks.len())
}

/// The folded heading whose body hides `block`, if any — the innermost
/// *visible* fold covering it. A heading folded inside an already-hidden
/// region is not reported: it draws nothing, so nothing can point at it;
/// unfolding its visible ancestor exposes it with its own fold intact.
pub fn fold_owner_of(blocks: &[Block], target: usize) -> Option<usize> {
    let mut owner = None;
    let mut end = 0;
    for index in 0..target {
        if index == end {
            owner = None;
        }
        if blocks[index].is_folded() && index >= end {
            owner = Some(index);
            end = fold_region_end(blocks, index);
        }
    }
    if target < end { owner } else { None }
}

impl Inline {
    fn text(&self) -> &str {
        match self {
            Inline::Text(t) => &t.text,
            // U+FFFC gives every flat-text consumer exactly one position.
            Inline::Math(_) => "\u{FFFC}",
            Inline::Note(_) => "\u{FFFC}",
            Inline::EqRef(_) => "\u{FFFC}",
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
            // A reference's label is fixed by the equation it points at.
            Inline::EqRef(_) => None,
        }
    }

    fn style(&self) -> Style {
        match self {
            Inline::Text(t) => t.style,
            Inline::Math(_) => Style::PLAIN,
            Inline::Note(_) => Style::PLAIN,
            Inline::EqRef(_) => Style::PLAIN,
        }
    }

    fn set_style(&mut self, style: Style) {
        match self {
            Inline::Text(t) => t.style = style,
            Inline::Math(_) => {}
            Inline::Note(_) => {}
            Inline::EqRef(_) => {}
        }
    }
}

/// The one flat-length rule in the app: prose is char-counted, every atom
/// costs exactly one position. A table cell is not a run — a row's flat space
/// is its cells', and `Cell::flat_len` adds a position per line break.
pub fn flat_len(run: &Inline) -> usize {
    match run {
        Inline::Text(t) => t.text.chars().count(),
        // Opaque math always costs one flat position.
        Inline::Math(_) => 1,
        Inline::Note(_) => 1,
        Inline::EqRef(_) => 1,
    }
}

/// The text of a table cell, lines joined the way a reader reads them.
pub fn cell_text(cell: &table::Cell) -> String {
    cell.lines()
        .iter()
        .map(|line| line.iter().map(Inline::text).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every run of a block in block order, table rows included. The generic
/// readers (search, colours, the clipboard) walk this instead of `inlines`,
/// which a table row has none of.
pub fn block_runs(block: &Block) -> Vec<&Inline> {
    match block {
        Block::TableRow { cells, .. } => cells.iter().flat_map(|cell| cell.all_runs()).collect(),
        block => block.inlines().iter().collect(),
    }
}

fn merge_style(run: &Inline) -> Option<Style> {
    match run {
        Inline::Text(t) => Some(t.style),
        // An atom is never a prose merge target.
        Inline::Math(_) => None,
        Inline::Note(_) => None,
        Inline::EqRef(_) => None,
    }
}

/// Whether a run is opaque — costs one flat position and is deleted as a
/// whole rather than char by char. Math atoms and sidenote anchors both are.
fn is_opaque(run: &Inline) -> bool {
    matches!(run, Inline::Math(_) | Inline::Note(_) | Inline::EqRef(_))
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
    Block::Math {
        list: vec![Inline::Math(Vec::new())],
        tag: None,
    }
}

/// A physical arrow direction while a table cell owns the cursor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TableDirection {
    Up,
    Down,
    Left,
    Right,
}

/// One empty table cell: an empty text run on one line, so `Tab` can name a
/// cell even when two adjacent ones are empty, and a caret always has a run
/// to sit in. Cells also hold math; see `insert_inline_math`.
fn table_cell() -> table::Cell {
    table::Cell::new()
}

pub(crate) fn table_row(
    columns: usize,
    first: bool,
    settings: std::sync::Arc<table::TableSettings>,
) -> Block {
    Block::TableRow {
        cells: (0..columns.max(1)).map(|_| table_cell()).collect(),
        first,
        settings,
    }
}

/// A list item's placeholder: one empty run under its own marker, so an
/// item you emptied stays an item until Enter (or Backspace) takes it out
/// of the list.
fn list_block(marker: ListMarker) -> Block {
    Block::ListItem {
        marker,
        content: vec![Inline::Text(Text {
            text: String::new(),
            style: Style::PLAIN,
        })],
    }
}

/// A line always holds at least one run. An empty side of a cell-line split
/// keeps the same placeholder every other empty line uses.
pub(crate) fn placeholder_if_empty(mut runs: Vec<Inline>) -> Vec<Inline> {
    if runs.is_empty() {
        runs.push(Inline::Text(Text {
            text: String::new(),
            style: Style::PLAIN,
        }));
    }
    runs
}

/// The marker an item created below `marker` carries: bullets and tasks
/// repeat themselves; an ordered item takes the next ordinal (the
/// renumbering pass keeps the run canonical).
fn continued(marker: ListMarker) -> ListMarker {
    match marker {
        ListMarker::Number(n) => ListMarker::Number(n + 1),
        ListMarker::Task { .. } => ListMarker::Task { done: false },
        other => other,
    }
}

/// Whether a line already satisfies every run invariant: no empty run outside
/// a sole placeholder, and no two adjacent text runs of one style. Checked
/// before the rebuild so the common case — an untouched line on a keystroke
/// somewhere else — costs no allocation at all.
fn line_is_canonical(runs: &[Inline]) -> bool {
    if runs.is_empty() {
        return false;
    }
    let placeholder = runs.len() == 1 && runs[0].text().is_empty();
    if !placeholder && runs.iter().any(|run| run.text().is_empty()) {
        return false;
    }
    !runs.windows(2).any(|pair| match pair {
        [Inline::Text(left), Inline::Text(right)] => left.style == right.style,
        _ => false,
    })
}

/// Repairs one line of a cell: drops empty runs and merges equal adjacent
/// prose runs.
fn prune_line(runs: &mut Vec<Inline>) {
    if line_is_canonical(runs) {
        return;
    }
    let taken = std::mem::take(runs);
    let mut merged = Vec::with_capacity(taken.len());
    for run in taken {
        if run.text().is_empty() {
            continue;
        }
        match (merged.last_mut(), run) {
            (Some(Inline::Text(previous)), Inline::Text(current))
                if previous.style == current.style =>
            {
                previous.text.push_str(&current.text);
            }
            (_, run) => merged.push(run),
        }
    }
    if merged.is_empty() {
        merged.push(Inline::Text(Text {
            text: String::new(),
            style: Style::PLAIN,
        }));
    }
    *runs = merged;
}

/// Repairs one cell: every line canonical, at least one line, and every line
/// holding at least one run.
fn prune_cell(cell: &mut table::Cell) {
    if cell.lines().is_empty() {
        cell.lines_mut().push(Vec::new());
    }
    for line in cell.lines_mut() {
        prune_line(line);
        if line.is_empty() {
            line.push(Inline::Text(Text {
                text: String::new(),
                style: Style::PLAIN,
            }));
        }
    }
}

/// Repair one block's runs: drop empty runs, merge equal adjacent prose runs,
/// demote a rule or display atom that gained prose, and turn an emptied block
/// into a placeholder of its own kind. Shared by the document's blocks and
/// every note body, so invariants hold in every scope rather than only where
/// the caret is.
fn prune_block(block: &mut Block) {
    // Cell wrappers are structural positions and are never merged or
    // discarded. Their contents, however, use the same run cleanup as prose.
    if let Block::TableRow { cells, .. } = block {
        if cells.is_empty() {
            cells.push(table::Cell::new());
        }
        for cell in cells {
            prune_cell(cell);
        }
        return;
    }
    let syntax = block
        .inlines()
        .first()
        .map(|run| run.style().syntax)
        .unwrap_or_default();
    let runs = std::mem::take(block.inlines_mut());
    let mut merged = Vec::with_capacity(runs.len());
    for run in runs {
        if run.text().is_empty() {
            continue;
        }
        match (merged.last_mut(), run) {
            (Some(Inline::Text(previous)), Inline::Text(current))
                if previous.style == current.style =>
            {
                previous.text.push_str(&current.text);
            }
            (_, run) => merged.push(run),
        }
    }
    *block.inlines_mut() = merged;
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
            Block::Heading { level, folded, .. } => Block::Heading {
                level: *level,
                folded: *folded,
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
                        syntax,
                        ..Style::PLAIN
                    },
                })],
                first: *first,
                lang: lang.clone(),
            },
            // A row is repaired through its cells and returns above; this
            // arm only exists so the match stays exhaustive.
            Block::TableRow { .. } => unreachable!("a table row prunes through its cells"),
            Block::Divider(_) => divider_block(),
            Block::Math { .. } => math_block(),
            Block::ListItem { marker, .. } => list_block(*marker),
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

pub(crate) fn slice_inline_runs(runs: &[Inline], start: usize, end: usize) -> Vec<Inline> {
    let mut result = Vec::new();
    let mut cursor = 0;
    for run in runs {
        let run_start = cursor;
        let run_end = cursor + flat_len(run);
        let from = start.max(run_start).min(run_end);
        let to = end.max(run_start).min(run_end);
        if from < to {
            match run {
                Inline::Text(text) => {
                    let value: String = text
                        .text
                        .chars()
                        .skip(from - run_start)
                        .take(to - from)
                        .collect();
                    result.push(Inline::Text(Text {
                        text: value,
                        style: text.style,
                    }));
                }
                Inline::Math(_) | Inline::Note(_) | Inline::EqRef(_) => result.push(run.clone()),
            }
        }
        cursor = run_end;
    }
    result
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
        Inline::EqRef(label) => {
            // A reference cannot split either; the empty side is pruned.
            if at == 0 {
                (
                    Inline::Text(Text {
                        text: String::new(),
                        style: Style::PLAIN,
                    }),
                    Inline::EqRef(label),
                )
            } else {
                (
                    Inline::EqRef(label),
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
            '@' => {
                // `@eq:<name>` pasted back becomes a reference again, the
                // same way `$…$` does. A reference the reader *typed* goes
                // in one character at a time and never matches here.
                if let Some(len) = eq_ref_at(&chars[i..]) {
                    push_text(&mut runs, &mut buf, style);
                    // The label is everything after the `@`: `eq:<name>`, the
                    // tag without its `#`.
                    let label: String = chars[i + 1..i + len].iter().collect();
                    runs.push(Inline::EqRef(label));
                    i += len;
                } else {
                    buf.push('@');
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

/// The length of an equation reference at the start of `chars`: `@eq:` plus
/// a `[A-Za-z0-9_-]+` name, or `None` when this `@` is ordinary text.
fn eq_ref_at(chars: &[char]) -> Option<usize> {
    if chars.len() < 5 || chars[1] != 'e' || chars[2] != 'q' || chars[3] != ':' {
        return None;
    }
    let mut n = 4;
    while n < chars.len()
        && (chars[n].is_ascii_alphanumeric() || chars[n] == '_' || chars[n] == '-')
    {
        n += 1;
    }
    if n > 4 { Some(n) } else { None }
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
            body: vec![empty_block()],
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

    /// Updates the persistence marker without touching document content.
    /// Tabs use this when restoring an undo snapshot and comparing it with
    /// the last successfully serialized content.
    pub(crate) fn set_dirty(&mut self, dirty: bool) {
        self.dirty = dirty;
    }

    /// The blocks the caret is in — the body, or the focused note's body.
    /// Anything that edits at the caret, moves the caret, or answers a
    /// question about where the caret is, goes through here.
    pub fn scope(&self) -> &[Block] {
        match self.focus {
            Focus::Body => &self.body,
            Focus::Note(i) => &self.notes[i].body,
        }
    }

    /// The mutable blocks the caret is in; the mirror of [`Self::scope`].
    pub fn scope_mut(&mut self) -> &mut Vec<Block> {
        match self.focus {
            Focus::Body => &mut self.body,
            Focus::Note(i) => &mut self.notes[i].body,
        }
    }

    /// The document's own blocks, whatever is focused. Anything that
    /// describes the file — its layout, its outline, its word count, what
    /// gets written to disk — goes through here.
    pub fn body(&self) -> &[Block] {
        &self.body
    }

    /// The document's own blocks, mutable; the mirror of [`Self::body`].
    pub fn body_mut(&mut self) -> &mut Vec<Block> {
        &mut self.body
    }

    /// Whitespace-split over all runs.
    pub fn word_count(&self) -> usize {
        self.body
            .iter()
            .flat_map(block_runs)
            .map(Inline::text)
            .map(|t| t.split_whitespace().count())
            .sum()
    }

    pub fn block_text(&self, block: usize) -> String {
        block_runs(&self.scope()[block])
            .into_iter()
            .map(Inline::text)
            .collect()
    }

    pub fn block_len(&self, block: usize) -> usize {
        self.scope()[block].flat_len()
    }

    pub fn caret_position(&self) -> FlatPos {
        FlatPos {
            block: self.caret.block,
            offset: self.caret_flat(self.caret.block),
        }
    }

    pub fn position(&self, block: usize, offset: usize) -> FlatPos {
        FlatPos {
            block: block.min(self.scope().len().saturating_sub(1)),
            offset: offset.min(self.block_len(block.min(self.scope().len().saturating_sub(1)))),
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
        for run in block_runs(&self.scope()[block]) {
            let run_start = cursor;
            let run_end = cursor + flat_len(run);
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
                    Inline::EqRef(label) => {
                        // The label carries its `eq:` prefix; the `@` is the
                        // only spelling added back.
                        out.push('@');
                        out.push_str(label);
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
    fn table_cells_in_range(
        &self,
        block: usize,
        from: usize,
        to: usize,
    ) -> Vec<(usize, usize, usize)> {
        if !self.scope().get(block).is_some_and(Block::is_table) || from >= to {
            return Vec::new();
        }
        let mut start = 0;
        self.scope()[block]
            .cells()
            .iter()
            .enumerate()
            .filter_map(|(index, cell)| {
                let end = start + cell.flat_len();
                let selected = (from < end && to > start).then(|| {
                    (
                        index,
                        from.saturating_sub(start).min(end - start),
                        to.saturating_sub(start).min(end - start),
                    )
                });
                start = end;
                selected
            })
            .collect()
    }

    fn style_range_status(&self, range: FlatRange, mask: Style) -> Option<bool> {
        let range = range.normalized();
        let start = self.position(range.start.block, range.start.offset);
        let end = self.position(range.end.block, range.end.offset);
        if (start.block, start.offset) >= (end.block, end.offset) {
            return None;
        }

        if start.block == end.block && self.scope()[start.block].is_table() {
            let cells = self.table_cells_in_range(start.block, start.offset, end.offset);
            if cells.is_empty() {
                return None;
            }
            if mask.code || mask.badge {
                let boxed = Style {
                    code: mask.code,
                    badge: !mask.code && mask.badge,
                    ..Style::PLAIN
                };
                return Some(cells.iter().all(|&(cell, from, to)| {
                    slice_inline_runs(self.scope()[start.block].cells()[cell].runs(), from, to)
                        .iter()
                        .all(|run| {
                            let style = run.style();
                            if boxed.badge {
                                style.badge
                            } else {
                                style == boxed
                            }
                        })
                }));
            }
            let eligible = cells
                .iter()
                .flat_map(|&(cell, from, to)| {
                    slice_inline_runs(self.scope()[start.block].cells()[cell].runs(), from, to)
                })
                .filter(|run| !run.style().is_boxed())
                .collect::<Vec<_>>();
            return (!eligible.is_empty())
                .then(|| eligible.iter().all(|run| style_matches(run.style(), mask)));
        }

        if mask.code || mask.badge {
            let boxed = Style {
                code: mask.code,
                badge: !mask.code && mask.badge,
                ..Style::PLAIN
            };
            let mut selected = false;
            let mut active = true;
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
                    selected = true;
                    active &= if boxed.badge {
                        run.style().badge
                    } else {
                        run.style() == boxed
                    };
                }
            }
            return selected.then_some(active);
        }

        let mut eligible = false;
        let mut active = true;
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
                    active &= style_matches(run.style(), mask);
                }
            }
        }
        eligible.then_some(active)
    }

    /// Whether every eligible run covered by `range` already has `mask`.
    /// This shares the exact state calculation used by `toggle_style_range`.
    pub fn style_range_is_active(&self, range: FlatRange, mask: Style) -> bool {
        self.style_range_status(range, mask) == Some(true)
    }

    /// Whether every run covered by `range` is a badge of `color`.
    pub fn badge_color_is_active(&self, range: FlatRange, color: BadgeColor) -> bool {
        let range = range.normalized();
        let start = self.position(range.start.block, range.start.offset);
        let end = self.position(range.end.block, range.end.offset);
        if (start.block, start.offset) >= (end.block, end.offset) {
            return false;
        }

        let mut selected = false;
        let mut active = true;
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
                selected = true;
                active &= run.style().badge && run.style().badge_color == color;
            }
        }
        selected && active
    }

    pub fn toggle_style_range(&mut self, range: FlatRange, mask: Style) {
        self.clamp_caret();
        let caret_block = self.caret.block;
        let caret_flat = self.caret_flat(caret_block);
        let range = range.normalized();
        let start = self.position(range.start.block, range.start.offset);
        let end = self.position(range.end.block, range.end.offset);
        if (start.block, start.offset) >= (end.block, end.offset) {
            return;
        }

        if start.block == end.block && self.scope()[start.block].is_table() {
            let cells = self.table_cells_in_range(start.block, start.offset, end.offset);
            let Some(active) = self.style_range_status(range, mask) else {
                return;
            };
            let boxed = mask.code || mask.badge;
            for (cell, from, to) in cells {
                let original = self.scope()[start.block].cells()[cell].runs();
                let len: usize = original.iter().map(flat_len).sum();
                let mut contents = slice_inline_runs(original, 0, from);
                let mut selected = slice_inline_runs(original, from, to);
                for run in &mut selected {
                    let style = run.style();
                    let target = if boxed {
                        if active {
                            Style::PLAIN
                        } else {
                            Style {
                                code: mask.code,
                                badge: !mask.code && mask.badge,
                                ..Style::PLAIN
                            }
                        }
                    } else if style.is_boxed() {
                        style
                    } else {
                        Style {
                            bold: if mask.bold { !active } else { style.bold },
                            italic: if mask.italic { !active } else { style.italic },
                            highlight: if mask.highlight {
                                !active
                            } else {
                                style.highlight
                            },
                            ..style
                        }
                    };
                    run.set_style(target);
                }
                contents.extend(selected);
                contents.extend(slice_inline_runs(original, to, len));
                self.scope_mut()[start.block]
                    .cells_mut()
                    .expect("table row")[cell] = table::Cell::from_runs(contents);
            }
            self.dirty = true;
            self.enforce();
            self.refresh_context();
            return;
        }

        if mask.code || mask.badge {
            let Some(all_boxed) = self.style_range_status(range, mask) else {
                return;
            };
            let boxed = Style {
                code: mask.code,
                badge: !mask.code && mask.badge,
                ..Style::PLAIN
            };
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
                let original = self.scope()[block].clone();
                let mut runs = self.slice_runs(block, 0, from);
                let mut selected = self.slice_runs(block, from, to);
                for run in &mut selected {
                    run.set_style(target);
                }
                runs.extend(selected);
                runs.extend(self.slice_runs(block, to, self.block_len(block)));
                self.scope_mut()[block] = Self::block_with_runs(&original, runs);
            }
            self.dirty = true;
            self.enforce();
            self.restore_caret_flat(caret_block, caret_flat);
            self.refresh_context();
            return;
        }

        let Some(active) = self.style_range_status(range, mask) else {
            return;
        };
        let enable = !active;

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
            let original = self.scope()[block].clone();
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
            self.scope_mut()[block] = Self::block_with_runs(&original, runs);
        }
        self.dirty = true;
        self.enforce();
        self.restore_caret_flat(caret_block, caret_flat);
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

        if start.block == end.block && self.scope()[start.block].is_table() {
            let cells = self.table_cells_in_range(start.block, start.offset, end.offset);
            let mut changed = false;
            for (cell, from, to) in cells {
                let original = self.scope()[start.block].cells()[cell].runs();
                let len: usize = original.iter().map(flat_len).sum();
                let mut contents = slice_inline_runs(original, 0, from);
                let mut selected = slice_inline_runs(original, from, to);
                for run in &mut selected {
                    let mut style = run.style();
                    if style.badge && style.badge_color != color {
                        style.badge_color = color;
                        run.set_style(style);
                        changed = true;
                    }
                }
                contents.extend(selected);
                contents.extend(slice_inline_runs(original, to, len));
                self.scope_mut()[start.block]
                    .cells_mut()
                    .expect("table row")[cell] = table::Cell::from_runs(contents);
            }
            if changed {
                self.dirty = true;
                self.enforce();
                self.refresh_context();
            }
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
            let original = self.scope()[block].clone();
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
            self.scope_mut()[block] = Self::block_with_runs(&original, runs);
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
        let first = first.min(self.scope().len().saturating_sub(1));
        let last = last.min(self.scope().len().saturating_sub(1)).max(first);
        FlatRange::new(
            self.position(first, 0),
            self.position(last, self.block_len(last)),
        )
    }

    /// Runs covered by a block-flat range. A table row's runs live in its
    /// cells, which a block-flat range cannot name, so a row yields nothing:
    /// the same-block table paths (`table_cells_in_range` and its callers) are
    /// the only way a range reaches a table. Returning nothing keeps every
    /// cross-block style pass total — it leaves the grid intact rather than
    /// splicing prose into a row.
    fn slice_runs(&self, block: usize, start: usize, end: usize) -> Vec<Inline> {
        let block = &self.scope()[block];
        if block.is_table() {
            return Vec::new();
        }
        slice_inline_runs(block.inlines(), start, end)
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
            Block::Math { tag, .. } => Block::Math {
                list: runs,
                tag: tag.clone(),
            },
            Block::Heading { level, folded, .. } => Block::Heading {
                level: *level,
                folded: *folded,
                content: runs,
            },
            Block::CodeLine { first, lang, .. } => Block::CodeLine {
                content: runs,
                first: *first,
                lang: lang.clone(),
            },
            // A row is rebuilt through its cells, never through a run list;
            // `slice_runs` yields nothing for one, so a row is handed back
            // exactly as it was rather than spliced into prose.
            Block::TableRow { .. } => (*block).clone(),
            Block::ListItem { marker, .. } => Block::ListItem {
                marker: *marker,
                content: runs,
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
        if start.block == end.block && self.scope()[start.block].is_table() {
            let (landing_cell, landing_offset) = self.flat_to_pos(start.block, start.offset);
            for (cell, from, to) in self.table_cells_in_range(start.block, start.offset, end.offset)
            {
                let original = self.scope()[start.block].cells()[cell].runs();
                let len: usize = original.iter().map(flat_len).sum();
                let mut contents = slice_inline_runs(original, 0, from);
                contents.extend(slice_inline_runs(original, to, len));
                self.scope_mut()[start.block]
                    .cells_mut()
                    .expect("table row")[cell] = table::Cell::from_runs(contents);
            }
            self.enforce();
            let landing_offset =
                landing_offset.min(self.scope()[start.block].cells()[landing_cell].flat_len());
            self.set_caret(start.block, landing_cell, landing_offset);
            self.dirty = true;
            return deleted;
        }
        if (start.block..=end.block).any(|block| self.scope()[block].is_table()) {
            // A cross-row character range cannot collapse table blocks into
            // each other without deleting tracks. Keep the grid intact; row
            // removal remains an explicit popup action.
            return String::new();
        }
        if start.block == end.block {
            let block = self.scope()[start.block].clone();
            let runs = self.slice_runs(start.block, 0, start.offset);
            let mut suffix = self.slice_runs(start.block, end.offset, self.block_len(start.block));
            let mut combined = runs;
            combined.append(&mut suffix);
            self.scope_mut()[start.block] = Self::block_with_runs(&block, combined);
        } else {
            let first = self.scope()[start.block].clone();
            let mut runs = self.slice_runs(start.block, 0, start.offset);
            runs.extend(self.slice_runs(end.block, end.offset, self.block_len(end.block)));
            self.scope_mut().splice(
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
        if self.scope().is_empty() {
            return String::new();
        }
        let first = first.min(self.scope().len() - 1);
        let last = last.min(self.scope().len() - 1).max(first);
        if (first..=last).any(|block| self.scope()[block].is_table()) {
            return String::new();
        }
        let deleted = (first..=last)
            .map(|block| self.block_text(block))
            .collect::<Vec<_>>()
            .join("\n");
        self.scope_mut().drain(first..=last);
        if self.scope().is_empty() {
            self.scope_mut().push(empty_block());
        }
        self.caret.block = first.min(self.scope().len() - 1);
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
        if self.in_table() && matches!(object, TextObject::InnerWord | TextObject::AroundWord) {
            let cell = self
                .caret
                .inline
                .min(self.scope()[caret.block].cells().len().saturating_sub(1));
            let chars: Vec<char> = cell_text(&self.scope()[caret.block].cells()[cell])
                .chars()
                .collect();
            let local = self.caret.offset;
            if chars.is_empty() || local >= chars.len() {
                return None;
            }
            let is_word = |c: char| c.is_alphanumeric() || c == '_';
            let word = is_word(chars[local]);
            let mut start = local;
            while start > 0
                && is_word(chars[start - 1]) == word
                && !chars[start - 1].is_whitespace()
            {
                start -= 1;
            }
            let mut end = local + 1;
            while end < chars.len() && is_word(chars[end]) == word && !chars[end].is_whitespace() {
                end += 1;
            }
            if matches!(object, TextObject::AroundWord) {
                while end < chars.len() && chars[end].is_whitespace() {
                    end += 1;
                }
                if end == local + 1 {
                    while start > 0 && chars[start - 1].is_whitespace() {
                        start -= 1;
                    }
                }
            }
            let base: usize = self.scope()[caret.block].cells()[..cell]
                .iter()
                .map(table::Cell::flat_len)
                .sum();
            return Some(FlatRange::new(
                self.position(caret.block, base + start),
                self.position(caret.block, base + end),
            ));
        }
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
                if caret.block + 1 < self.scope().len() {
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
                    if let Block::Heading { level, .. } = self.scope()[index] {
                        heading = Some((index, level));
                        break;
                    }
                }
                let (start, level) = heading?;
                let end = (start + 1..self.scope().len())
                    .find(|&index| matches!(self.scope()[index], Block::Heading { level: next, .. } if next <= level))
                    .unwrap_or(self.scope().len());
                Some(FlatRange::new(
                    self.position(start, 0),
                    self.position(end.saturating_sub(1), self.block_len(end.saturating_sub(1))),
                ))
            }
        }
    }

    // ---- internal geometry helpers --------------------------------------

    pub(crate) fn block_flat_len(&self, block: usize) -> usize {
        self.scope()[block].flat_len()
    }

    /// The caret's flat position within its block, clamped. In a table row the
    /// unit is the cell, so the same arithmetic addresses one.
    fn caret_flat(&self, block: usize) -> usize {
        let source = &self.scope()[block];
        let unit = self.caret.inline.min(source.unit_count().saturating_sub(1));
        let prefix: usize = (0..unit).map(|unit| source.unit_len(unit)).sum();
        let offset = self.caret.offset.min(source.unit_len(unit));
        (prefix + offset).min(source.flat_len())
    }

    /// `(unit, offset)` for a flat position, clamped to the block's end.
    fn flat_to_pos(&self, block: usize, flat: usize) -> (usize, usize) {
        self.scope()[block].flat_to_unit(flat)
    }

    /// Style of the character at `flat`, or `None` at the end of the block.
    fn style_at(&self, block: usize, flat: usize) -> Option<Style> {
        let source = &self.scope()[block];
        if source.is_table() {
            let (cell, offset) = source.flat_to_unit(flat);
            return source
                .cells()
                .get(cell)
                .and_then(|cell| cell.style_at(offset));
        }
        let mut pos = 0;
        for run in source.inlines() {
            let len = flat_len(run);
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
    pub(crate) fn clamp_caret(&mut self) {
        if self.body.is_empty() {
            self.body.push(empty_block());
        }
        if let Focus::Note(i) = self.focus
            && i >= self.notes.len()
        {
            self.focus = Focus::Body;
        }
        let block = self.caret.block.min(self.scope().len() - 1);
        self.caret.block = block;
        // Read the clamped inline/offset and whether the run is math before
        // mutating: the `focused` borrow would otherwise outlive `self.math`.
        let (inline, offset, is_math) = {
            let source = &self.scope()[block];
            let inline = self.caret.inline.min(source.unit_count().saturating_sub(1));
            let len = source.unit_len(inline);
            let offset = self.caret.offset.min(len);
            let is_math = match source {
                Block::TableRow { cells, .. } => match cells.get(inline) {
                    Some(cell) => {
                        let at = cell.run_at(offset);
                        matches!(
                            cell.lines().get(at.line).and_then(|line| line.get(at.run)),
                            Some(Inline::Math(_))
                        )
                    }
                    None => false,
                },
                block => matches!(block.inlines().get(inline), Some(Inline::Math(_))),
            };
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
    pub(crate) fn enforce(&mut self) {
        self.prune_runs();
        self.renumber_ordered_runs();
        self.clamp_caret();
        debug_assert!(self.invariants_hold());
    }

    /// Ordered items are numbered 1,2,3… across each consecutive run; a
    /// non-list block (or a bullet/task item) ends the run. Kept true after
    /// every edit, so the marker drawn agrees with what serialization
    /// writes — `markdown::serialize` renumbers the same way.
    fn renumber_ordered_runs(&mut self) {
        let mut ord = 0u32;
        for block in &mut self.body {
            match block {
                Block::ListItem {
                    marker: ListMarker::Number(n),
                    ..
                } => {
                    ord += 1;
                    *n = ord;
                }
                _ => ord = 0,
            }
        }
    }

    /// Drops empty runs; an all-empty block becomes a placeholder of the same
    /// kind (a Heading stays a heading, so `set_heading` on an empty line
    /// survives and can be typed into — the invariant that an empty document
    /// is one empty Paragraph is only about `Document::new`).
    fn prune_runs(&mut self) {
        let caret_block = self.caret.block.min(self.scope().len().saturating_sub(1));
        let caret_offset = self.caret_flat(caret_block);
        let table_caret = self.scope()[caret_block]
            .is_table()
            .then_some((self.caret.inline, self.caret.offset));
        // The invariant holds in *every* scope, not just where the caret
        // happens to be: a note body left with an empty run would be an
        // invariant only until focus moved elsewhere.
        for block in &mut self.body {
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
            .body
            .iter()
            .flat_map(Block::inlines)
            .filter_map(|run| match run {
                Inline::Note(label) => Some(label.as_str()),
                _ => None,
            })
            .collect();
        self.notes
            .retain(|note| !note.anchored || anchored.contains(&note.label.as_str()));

        if match self.focus {
            Focus::Body => true,
            Focus::Note(index) => index < self.notes.len(),
        } {
            self.caret.block = caret_block;
            if let Some((inline, offset)) = table_caret
                && self.scope()[caret_block].is_table()
            {
                let (cell, offset) = {
                    let cells = self.scope()[caret_block].cells();
                    let cell = inline.min(cells.len() - 1);
                    (cell, offset.min(cells[cell].flat_len()))
                };
                self.caret.inline = cell;
                self.caret.offset = offset;
            } else {
                let (inline, offset) = self.flat_to_pos(caret_block, caret_offset);
                self.caret.inline = inline;
                self.caret.offset = offset;
            }
        }
    }

    fn invariants_hold(&self) -> bool {
        if self.body.is_empty() {
            return false;
        }
        for block in self
            .body
            .iter()
            .chain(self.notes.iter().flat_map(|note| note.body.iter()))
        {
            if block.unit_count() == 0 {
                return false;
            }
            if block.is_table() {
                for cell in block.cells() {
                    if cell.lines().is_empty() {
                        return false;
                    }
                    for line in cell.lines() {
                        if !line_is_canonical(line) {
                            return false;
                        }
                    }
                }
            } else {
                for (i, run) in block.inlines().iter().enumerate() {
                    if run.text().is_empty() && (block.inlines().len() > 1 || i != 0) {
                        return false;
                    }
                }
            }
        }
        for note in &self.notes {
            if note.body.is_empty() {
                return false;
            }
        }
        let block = &self.scope()[self.caret.block];
        if self.caret.inline >= block.unit_count() {
            return false;
        }
        self.caret.offset <= block.unit_len(self.caret.inline)
    }

    // ---- caret movement (never dirt) ------------------------------------

    /// Click placement / jump. Clamps into bounds; the style context is the
    /// style-before rule (§3). A block hidden by a fold holds no caret: the
    /// placement lands on the fold's heading instead, the ground the reader
    /// can actually see.
    pub fn set_caret(&mut self, block: usize, inline: usize, offset: usize) {
        self.clamp_caret();
        let mut block = block.min(self.scope().len() - 1);
        if let Some(owner) = fold_owner_of(self.scope(), block) {
            block = owner;
        }
        self.caret.block = block;
        let source = &self.scope()[block];
        let i = inline.min(source.unit_count().saturating_sub(1));
        let len = source.unit_len(i);
        let table_style = self.scope()[block].is_table().then(|| {
            self.scope()[block]
                .cells()
                .get(i)
                .and_then(|cell| cell.style_before(offset))
                .unwrap_or(Style::PLAIN)
        });
        self.caret.inline = i;
        self.caret.offset = offset.min(len);
        self.caret.style = if let Some(style) = table_style {
            style
        } else {
            self.style_before(block, self.caret_flat(block))
                .unwrap_or(Style::PLAIN)
        };
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
        let run_len_i = self.scope()[b].unit_len(self.caret.inline);
        let at_seam = o == 0 || o == run_len_i;
        let after = self.style_at(b, self.caret_flat(b));
        let target = after.unwrap_or(Style::PLAIN);
        if at_seam && !self.caret.style.same_context(target) {
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
        } else if b + 1 < self.scope().len() {
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
        let run_len_i = self.scope()[b].unit_len(self.caret.inline);
        let at_seam = o == 0 || o == run_len_i;
        let before = self.style_before(b, self.caret_flat(b));
        let target = before.unwrap_or(Style::PLAIN);
        if at_seam && !self.caret.style.same_context(target) {
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

    /// Whether the caret is currently in a table row.
    pub fn in_table(&self) -> bool {
        self.scope()
            .get(self.caret.block)
            .is_some_and(Block::is_table)
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
        let b = self.caret.block;
        // A display block is one atom: there is no prose beside it to type
        // into, so typed characters enter the tree at the side of the atom
        // the caret rests on. Splicing prose here would demote the whole
        // block in `prune_block` — the expression would vanish under the
        // next keystroke.
        if matches!(self.scope()[b], Block::Math { .. }) {
            let len = match self.scope()[b].inlines() {
                [Inline::Math(list)] => list.len(),
                _ => unreachable!("display math must contain exactly one math atom"),
            };
            let index = if self.caret.offset > 0 { len } else { 0 };
            self.set_caret(b, 0, 0);
            self.math = Some(math::MathCursor {
                path: Vec::new(),
                index,
            });
            for c in text.chars() {
                self.math_insert_char(c);
            }
            return;
        }
        if self.scope()[b].is_table() {
            let cell = self.caret.inline;
            let flat = self.caret.offset;
            let style = self.caret.style;
            let at = self.scope()[b].cells()[cell].run_at(flat);
            let (run, offset) = (at.run, at.offset);
            let contents =
                &mut self.scope_mut()[b].cells_mut().expect("table row")[cell].lines_mut()[at.line];
            let placeholder = contents.len() == 1 && contents[0].text().is_empty();
            let current_len = flat_len(&contents[run]);
            let left_style = if offset > 0 {
                merge_style(&contents[run])
            } else if run > 0 {
                merge_style(&contents[run - 1])
            } else {
                None
            };
            let right_style = if offset < current_len {
                merge_style(&contents[run])
            } else if run + 1 < contents.len() {
                merge_style(&contents[run + 1])
            } else {
                None
            };
            if placeholder {
                let value = contents[0].text_mut().expect("a table placeholder is text");
                value.push_str(text);
                contents[0].set_style(style);
            } else if left_style == Some(style) {
                if offset > 0 {
                    insert_str(
                        contents[run]
                            .text_mut()
                            .expect("a prose merge target is text"),
                        offset,
                        text,
                    );
                } else {
                    contents[run - 1]
                        .text_mut()
                        .expect("a prose merge target is text")
                        .push_str(text);
                }
            } else if right_style == Some(style) {
                if offset < current_len {
                    insert_str(
                        contents[run]
                            .text_mut()
                            .expect("a prose merge target is text"),
                        offset,
                        text,
                    );
                } else {
                    insert_str(
                        contents[run + 1]
                            .text_mut()
                            .expect("a prose merge target is text"),
                        0,
                        text,
                    );
                }
            } else {
                let (prefix, suffix) = split_run(contents.remove(run), offset);
                contents.splice(
                    run..run,
                    [
                        prefix,
                        Inline::Text(Text {
                            text: text.to_string(),
                            style,
                        }),
                        suffix,
                    ],
                );
            }
            self.caret.offset = flat + text.chars().count();
            self.dirty = true;
            self.enforce();
            self.caret.inline = cell;
            self.caret.offset = self
                .caret
                .offset
                .min(self.scope()[b].cells()[cell].flat_len());
            return;
        }
        let s = if self.scope()[b].is_code() {
            Style {
                code: true,
                syntax: self.scope()[b].inlines()[self.caret.inline].style().syntax,
                ..Style::PLAIN
            }
        } else {
            self.caret.style
        };
        let i = self.caret.inline;
        let o = self.caret.offset;
        let flat = self.caret_flat(b);

        let len = text.chars().count();
        let runs = self.scope()[b].inlines();
        let placeholder = runs.len() == 1 && runs[0].text().is_empty();
        let li = flat_len(&runs[i]);
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
            let runs = self.scope_mut()[b].inlines_mut();
            runs[0]
                .text_mut()
                .expect("placeholder is always a text run")
                .push_str(text);
            runs[0].set_style(s);
        } else if left_style == Some(s) {
            let runs = self.scope_mut()[b].inlines_mut();
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
            let runs = self.scope_mut()[b].inlines_mut();
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
            let (prefix, suffix) = split_run(self.scope_mut()[b].inlines_mut().remove(i), o);
            let new_run = Inline::Text(Text {
                text: text.to_string(),
                style: s,
            });
            let runs = self.scope_mut()[b].inlines_mut();
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
        // Notation markers carry no meaning inside a display atom; the
        // characters join the tree like any typed ones.
        if matches!(self.scope()[self.caret.block], Block::Math { .. }) {
            self.insert_text(text);
            return;
        }
        if self.scope()[self.caret.block].is_table() {
            let parsed = inline_runs(text, self.caret.style);
            let block = self.caret.block;
            let cell = self.caret.inline;
            let flat = self.caret.offset;
            let inserted: usize = parsed.iter().map(flat_len).sum();
            let at = self.scope()[block].cells()[cell].run_at(flat);
            let contents = &mut self.scope_mut()[block].cells_mut().expect("table row")[cell]
                .lines_mut()[at.line];
            let (prefix, suffix) = split_run(contents.remove(at.run), at.offset);
            let mut replacement = vec![prefix];
            replacement.extend(parsed);
            replacement.push(suffix);
            contents.splice(at.run..at.run, replacement);
            self.caret.offset = flat + inserted;
            self.dirty = true;
            self.enforce();
            return;
        }
        let s = self.caret.style;
        let b = self.caret.block;
        let i = self.caret.inline;
        let o = self.caret.offset;
        let flat = self.caret_flat(b);

        let parsed = inline_runs(text, s);
        let inserted: usize = parsed.iter().map(flat_len).sum();
        let (prefix, suffix) = split_run(self.scope_mut()[b].inlines_mut().remove(i), o);
        let mut runs = vec![prefix];
        runs.extend(parsed);
        runs.push(suffix);
        self.scope_mut()[b].inlines_mut().splice(i..i, runs);
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
        if self.scope()[b].is_table() {
            if self.join_cell_line() {
                return;
            }
            if o == 0 {
                return;
            }
            let at = self.scope()[b].cells()[i].run_at(o - 1);
            let run_start = self.scope()[b].cells()[i].run_start(at.line, at.run);
            if let Inline::Math(list) = &self.scope()[b].cells()[i].lines()[at.line][at.run] {
                let end = list.len();
                self.caret.offset = run_start;
                self.math = Some(math::MathCursor {
                    path: Vec::new(),
                    index: end,
                });
                let _ = self.math_backspace();
                return;
            }
            let contents =
                &mut self.scope_mut()[b].cells_mut().expect("table row")[i].lines_mut()[at.line];
            if is_opaque(&contents[at.run]) {
                contents.remove(at.run);
            } else {
                remove_char_at(
                    contents[at.run].text_mut().expect("a prose run is text"),
                    at.offset,
                );
            }
            self.caret.offset -= 1;
            self.dirty = true;
            self.enforce();
            return;
        }
        let math_inline = if o > 0 && matches!(self.scope()[b].inlines()[i], Inline::Math(_)) {
            Some(i)
        } else if o == 0 && i > 0 && matches!(self.scope()[b].inlines()[i - 1], Inline::Math(_)) {
            Some(i - 1)
        } else {
            None
        };
        if let Some(inline) = math_inline {
            let len = match &self.scope()[b].inlines()[inline] {
                Inline::Math(list) => list.len(),
                Inline::Text(_) => unreachable!("math target was checked above"),
                Inline::Note(_) | Inline::EqRef(_) => {
                    unreachable!("math target was checked above")
                }
            };
            if len > 0 || self.scope()[b].is_math() {
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
            let runs = self.scope_mut()[b].inlines_mut();
            if is_opaque(&runs[i]) {
                runs.remove(i);
            } else {
                remove_char_at(runs[i].text_mut().expect("non-math run is text"), o - 1);
            }
        } else if i > 0 {
            let runs = self.scope_mut()[b].inlines_mut();
            if is_opaque(&runs[i - 1]) {
                runs.remove(i - 1);
            } else {
                let prev_len = flat_len(&runs[i - 1]);
                remove_char_at(
                    runs[i - 1].text_mut().expect("non-math run is text"),
                    prev_len - 1,
                );
            }
        } else if b > 0 && self.scope()[b - 1].is_math() {
            let previous = b - 1;
            let len = match self.scope()[previous].inlines() {
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
        } else if b > 0 && self.scope()[b].is_math() {
            self.set_caret(b, 0, 0);
            self.math = Some(math::MathCursor::default());
            return;
        } else if matches!(self.scope()[b], Block::ListItem { .. }) {
            // Backspace at an item's start removes the marker first — one
            // gesture, one meaning. The next one merges like any paragraph.
            let content = std::mem::take(self.scope_mut()[b].inlines_mut());
            self.scope_mut()[b] = Block::Paragraph(content);
            self.dirty = true;
            self.enforce();
            self.refresh_context();
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
            let runs = self.scope_mut()[b].inlines_mut();
            std::mem::take(runs)
        };
        self.scope_mut().remove(b);
        self.scope_mut()[prev].inlines_mut().append(&mut taken);
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
        if self.scope()[b].is_table() {
            if self.join_cell_line_forward() {
                return;
            }
            let li = self.scope()[b].cells()[i].flat_len();
            if o >= li {
                return;
            }
            let at = self.scope()[b].cells()[i].run_at(o);
            if matches!(
                self.scope()[b].cells()[i].lines()[at.line][at.run],
                Inline::Math(_)
            ) {
                self.math = Some(math::MathCursor::default());
                let _ = self.math_delete_forward();
                return;
            }
            let contents =
                &mut self.scope_mut()[b].cells_mut().expect("table row")[i].lines_mut()[at.line];
            if is_opaque(&contents[at.run]) {
                contents.remove(at.run);
            } else {
                remove_char_at(
                    contents[at.run].text_mut().expect("a prose run is text"),
                    at.offset,
                );
            }
            self.dirty = true;
            self.enforce();
            return;
        }
        let runs = self.scope()[b].inlines();
        let li = flat_len(&runs[i]);
        let math_inline = if o < li && matches!(runs[i], Inline::Math(_)) {
            Some(i)
        } else if o == li && i + 1 < runs.len() && matches!(runs[i + 1], Inline::Math(_)) {
            Some(i + 1)
        } else {
            None
        };
        if let Some(inline) = math_inline {
            let empty = match &self.scope()[b].inlines()[inline] {
                Inline::Math(list) => list.is_empty(),
                Inline::Text(_) => unreachable!("math target was checked above"),
                Inline::Note(_) | Inline::EqRef(_) => {
                    unreachable!("math target was checked above")
                }
            };
            if !empty || self.scope()[b].is_math() {
                self.set_caret(b, inline, 0);
                self.math = Some(math::MathCursor::default());
                let _ = self.math_delete_forward();
                return;
            }
        }
        if o < li {
            let runs = self.scope_mut()[b].inlines_mut();
            if is_opaque(&runs[i]) {
                runs.remove(i);
            } else {
                remove_char_at(runs[i].text_mut().expect("non-math run is text"), o);
            }
        } else if i + 1 < runs.len() {
            let runs = self.scope_mut()[b].inlines_mut();
            if is_opaque(&runs[i + 1]) {
                runs.remove(i + 1);
            } else {
                remove_char_at(runs[i + 1].text_mut().expect("non-math run is text"), 0);
            }
        } else if b + 1 < self.scope().len() && self.scope()[b].is_math() {
            return;
        } else if b + 1 < self.scope().len() && self.scope()[b + 1].is_math() {
            let next = b + 1;
            self.set_caret(next, 0, 0);
            self.math = Some(math::MathCursor::default());
            let _ = self.math_delete_forward();
            return;
        } else if b + 1 < self.scope().len() {
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
            let runs = self.scope_mut()[next].inlines_mut();
            std::mem::take(runs)
        };
        self.scope_mut().remove(next);
        self.scope_mut()[b].inlines_mut().append(&mut taken);
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
        if self.scope()[b].is_table() {
            // Enter breaks the cell's line, never the row: table structure
            // belongs to the explicit structural commands.
            self.split_cell_line();
            return;
        }
        // A list item continues itself: Enter below an item opens the next
        // one of the same kind, and Enter on an empty item is the way out —
        // the empty paragraph you asked for replaces the item.
        let list_marker = match self.scope()[b] {
            Block::ListItem { marker, .. } => {
                if self.block_flat_len(b) == 0 {
                    self.scope_mut()[b] = empty_block();
                    self.dirty = true;
                    self.enforce();
                    return;
                }
                Some(continued(marker))
            }
            _ => None,
        };
        let is_code = self.scope()[b].is_code();
        let (prefix, suffix) = split_run(self.scope_mut()[b].inlines_mut().remove(i), o);
        let mut taken = Vec::new();
        let runs = self.scope_mut()[b].inlines_mut();
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
        } else if let Some(marker) = list_marker {
            Block::ListItem {
                marker,
                content: new_content,
            }
        } else {
            Block::Paragraph(new_content)
        };
        self.scope_mut().insert(b + 1, new_block);

        self.dirty = true;
        self.caret.block = b + 1;
        self.caret.inline = 0;
        self.caret.offset = 0;
        // A paragraph born from Enter wants to be typed into: a fold that
        // would hide it opens instead of swallowing the caret.
        self.reveal_block(b + 1);
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
        if self.scope()[b].is_table() {
            let cell = self.caret.inline;
            let offset = self.caret.offset;
            if offset >= self.scope()[b].cells()[cell].flat_len() {
                return;
            }
            let at = self.scope()[b].cells()[cell].run_at(offset);
            if matches!(
                self.scope()[b].cells()[cell].lines()[at.line][at.run],
                Inline::Math(_)
            ) {
                self.math = Some(math::MathCursor::default());
                let _ = self.math_delete_forward();
                return;
            }
            let contents =
                &mut self.scope_mut()[b].cells_mut().expect("table row")[cell].lines_mut()[at.line];
            if is_opaque(&contents[at.run]) {
                contents.remove(at.run);
            } else {
                remove_char_at(
                    contents[at.run].text_mut().expect("a prose run is text"),
                    at.offset,
                );
            }
            self.dirty = true;
            self.enforce();
            return;
        }
        let flat = self.caret_flat(b);
        if flat >= self.block_flat_len(b) {
            return;
        }
        let (i, o) = self.flat_to_pos(b, flat);
        if let Inline::Math(list) = &self.scope()[b].inlines()[i]
            && (!list.is_empty() || self.scope()[b].is_math())
        {
            self.set_caret(b, i, 0);
            self.math = Some(math::MathCursor::default());
            let _ = self.math_delete_forward();
            return;
        }
        let runs = self.scope_mut()[b].inlines_mut();
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
        if self.scope()[b].is_table() {
            // `dd` clears the current cell; deleting a row is a structural
            // table edit and is deliberately available only from the popup.
            let cell = self.caret.inline;
            if let Some(cells) = self.scope_mut()[b].cells_mut() {
                cells[cell] = table::Cell::new();
            }
            self.caret.offset = 0;
            self.caret.style = Style::PLAIN;
            self.math = None;
            self.dirty = true;
            return;
        }
        if self.scope().len() == 1 {
            self.scope_mut()[0] = empty_block();
            self.caret.block = 0;
            self.caret.inline = 0;
            self.caret.offset = 0;
        } else {
            self.scope_mut().remove(b);
            let landing = b.min(self.scope().len() - 1);
            self.caret.block = landing;
            self.caret.inline = 0;
            self.caret.offset = 0;
            let mut prefix = 0;
            for run in self.scope()[landing].inlines() {
                let lead = run.text().chars().take_while(|c| c.is_whitespace()).count();
                if lead < flat_len(run) {
                    let (i, o) = self.flat_to_pos(landing, prefix + lead);
                    self.caret.inline = i;
                    self.caret.offset = o;
                    break;
                }
                prefix += flat_len(run);
            }
        }
        self.caret.style = Style::PLAIN;
        self.dirty = true;
        self.enforce();
    }

    /// Unfold every fold covering `block`: an edit about to place the caret
    /// there wants visible ground. The heading's own flag is the only state
    /// touched; the content never changes.
    pub fn reveal_block(&mut self, block: usize) {
        while let Some(owner) = fold_owner_of(&self.body, block) {
            match self.body.get_mut(owner) {
                Some(Block::Heading { folded, .. }) if *folded => *folded = false,
                _ => break,
            }
        }
    }

    /// vim `o`: an empty Paragraph below the caret's block — or, on a list
    /// item, the next item of the same kind.
    pub fn open_below(&mut self) {
        self.clamp_caret();
        let b = self.caret.block;
        if self.scope()[b].is_table() {
            let Some(range) = self.table_bounds(b) else {
                return;
            };
            self.scope_mut().insert(range.end, empty_block());
            self.set_caret(range.end, 0, 0);
            self.dirty = true;
            return;
        }
        self.reveal_block(b + 1);
        let marker = match self.scope()[b] {
            Block::ListItem { marker, .. } => Some(continued(marker)),
            _ => None,
        };
        self.scope_mut()
            .insert(b + 1, marker.map_or_else(empty_block, list_block));
        self.caret.block = b + 1;
        self.caret.inline = 0;
        self.caret.offset = 0;
        self.caret.style = Style::PLAIN;
        self.dirty = true;
        self.enforce();
    }

    /// vim `O`: an empty Paragraph above the caret's block — or, on a list
    /// item, an item of the same kind above it (renumbering fixes ordinals).
    pub fn open_above(&mut self) {
        self.clamp_caret();
        let b = self.caret.block;
        if self.scope()[b].is_table() {
            let Some(range) = self.table_bounds(b) else {
                return;
            };
            self.scope_mut().insert(range.first, empty_block());
            self.set_caret(range.first, 0, 0);
            self.dirty = true;
            return;
        }
        self.reveal_block(b);
        let marker = match self.scope()[b] {
            Block::ListItem { marker, .. } => Some(marker),
            _ => None,
        };
        self.scope_mut()
            .insert(b, marker.map_or_else(empty_block, list_block));
        self.caret.block = b;
        self.caret.inline = 0;
        self.caret.offset = 0;
        self.caret.style = Style::PLAIN;
        self.dirty = true;
        self.enforce();
    }

    /// Set the pending bold/italic combination used by the `*`, `**`, and
    /// `***` insert gestures.
    pub fn set_emphasis(&mut self, bold: bool, italic: bool) {
        if self.caret.style.is_boxed() {
            return;
        }
        self.caret.style.bold = bold;
        self.caret.style.italic = italic;
    }

    /// Flip `bold` on the pending context.
    pub fn toggle_bold(&mut self) {
        if self.caret.style.is_boxed() {
            return;
        }
        self.caret.style.bold = !self.caret.style.bold;
    }

    /// Flip `italic` on the pending context.
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
        if self.scope()[b].is_table() {
            return;
        }
        if self.scope()[b].is_code() == on {
            return;
        }
        let flat_text: String = self.scope()[b].inlines().iter().map(Inline::text).collect();
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
        self.scope_mut()[b] = if on {
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
        let Some(current) = self.body.get(block) else {
            return false;
        };
        if current.is_table() {
            return false;
        }
        if current.is_code() == on {
            return false;
        }

        let mut inlines = std::mem::take(self.body[block].inlines_mut());
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
        self.body[block] = if on {
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
        if self.scope()[b].is_table() {
            return;
        }
        if let Some(level) = level {
            if !(1..=4).contains(&level) {
                return;
            }
            if matches!(
                self.scope()[b],
                Block::Heading {
                    level: current, ..
                } if current == level
            ) {
                return;
            }
            let mut inlines = std::mem::take(self.scope_mut()[b].inlines_mut());
            if self.scope()[b].is_code() {
                for run in &mut inlines {
                    run.set_style(Style {
                        code: false,
                        syntax: code::CodeStyle::PLAIN,
                        ..run.style()
                    });
                }
            }
            self.scope_mut()[b] = Block::Heading {
                level,
                folded: false,
                content: inlines,
            };
        } else {
            if matches!(self.scope()[b], Block::Paragraph(_)) {
                return;
            }
            let mut inlines = std::mem::take(self.scope_mut()[b].inlines_mut());
            if self.scope()[b].is_code() {
                for run in &mut inlines {
                    run.set_style(Style {
                        code: false,
                        syntax: code::CodeStyle::PLAIN,
                        ..run.style()
                    });
                }
            }
            self.scope_mut()[b] = Block::Paragraph(inlines);
        }
        self.dirty = true;
        self.enforce();
    }

    /// Convert one exact block to Body or H1-H4 without moving the caret.
    pub fn set_block_heading_at(&mut self, block: usize, level: Option<u8>) -> bool {
        if level.is_some_and(|level| !(1..=4).contains(&level)) {
            return false;
        }
        let Some(current) = self.body.get(block) else {
            return false;
        };
        if current.is_table() {
            return false;
        }
        if matches!((current, level), (Block::Paragraph(_), None))
            || matches!((current, level), (Block::Heading { level: current, .. }, Some(next)) if *current == next)
        {
            return false;
        }

        let was_code = current.is_code();
        let mut inlines = std::mem::take(self.body[block].inlines_mut());
        if was_code {
            for run in &mut inlines {
                if let Inline::Text(text) = run {
                    text.style = Style::PLAIN;
                }
            }
        }
        self.body[block] = match level {
            Some(level) => Block::Heading {
                level,
                folded: false,
                content: inlines,
            },
            None => Block::Paragraph(inlines),
        };
        self.dirty = true;
        true
    }

    /// Convert the caret's block to a list item of `marker`, or back to a
    /// paragraph with `None`. The command that carries the kind the block
    /// already has (bullets match bullets, ordered matches ordered whatever
    /// its ordinal, tasks match tasks) demotes it — the same gesture is the
    /// way back out.
    pub fn set_list(&mut self, marker: Option<ListMarker>) {
        if matches!(self.focus, Focus::Note(_)) {
            return;
        }
        self.clamp_caret();
        let b = self.caret.block;
        if self.scope()[b].is_table() {
            return;
        }
        if self.scope()[b].is_code() {
            return;
        }
        if marker.is_none() && matches!(self.scope()[b], Block::Paragraph(_)) {
            return;
        }
        let same_kind = matches!(
            (&marker, &self.scope()[b]),
            (
                Some(ListMarker::Bullet),
                Block::ListItem {
                    marker: ListMarker::Bullet,
                    ..
                }
            ) | (
                Some(ListMarker::Number(_)),
                Block::ListItem {
                    marker: ListMarker::Number(_),
                    ..
                }
            ) | (
                Some(ListMarker::Task { .. }),
                Block::ListItem {
                    marker: ListMarker::Task { .. },
                    ..
                }
            )
        );
        let inlines = std::mem::take(self.scope_mut()[b].inlines_mut());
        self.scope_mut()[b] = match (marker, same_kind) {
            (Some(marker), false) => Block::ListItem {
                marker,
                content: inlines,
            },
            _ => Block::Paragraph(inlines),
        };
        self.dirty = true;
        self.enforce();
    }

    /// Flip the checkbox of the task item the caret sits on. `false` when
    /// the caret is not on a task item (or is inside a note, which cannot
    /// hold one).
    pub fn toggle_task_here(&mut self) -> bool {
        if matches!(self.focus, Focus::Note(_)) {
            return false;
        }
        let b = self.caret.block;
        self.toggle_task_at(b)
    }

    /// Flip one task item's checkbox. The mark is content, not UI state: it
    /// saves with the file and the struck-through dim rendering follows it.
    pub fn toggle_task_at(&mut self, block: usize) -> bool {
        let Some(Block::ListItem {
            marker: ListMarker::Task { done },
            ..
        }) = self.body.get_mut(block)
        else {
            return false;
        };
        *done = !*done;
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
        if self.scope()[b].is_table() {
            return;
        }
        let at = if self.block_flat_len(b) == 0 {
            self.scope_mut()[b] = divider_block();
            b
        } else {
            self.scope_mut().insert(b + 1, divider_block());
            b + 1
        };
        self.scope_mut().insert(at + 1, empty_block());
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
        if self.scope()[b].is_table() {
            let at = self.scope()[b].cells()[i].run_at(o);
            let contents =
                &mut self.scope_mut()[b].cells_mut().expect("table row")[i].lines_mut()[at.line];
            let (prefix, suffix) = split_run(contents.remove(at.run), at.offset);
            contents.splice(at.run..at.run, [prefix, Inline::Math(Vec::new()), suffix]);
            self.caret.offset = o;
            self.caret.style = Style::PLAIN;
            self.math = Some(math::MathCursor::default());
            self.dirty = true;
            self.enforce();
            return;
        }
        let flat = self.caret_flat(b);
        let (prefix, suffix) = split_run(self.scope_mut()[b].inlines_mut().remove(i), o);
        self.scope_mut()[b]
            .inlines_mut()
            .splice(i..i, [prefix, Inline::Math(Vec::new()), suffix]);
        self.dirty = true;
        self.enforce();
        let (inline, offset) = self.flat_to_pos(b, flat);
        self.set_caret(b, inline, offset);
        self.math = Some(math::MathCursor::default());
    }

    /// Drops the focused inline expression if nothing has been typed into it
    /// yet, leaving the caret exactly where the atom was. Returns whether it
    /// went.
    ///
    /// This is what makes `$` its own escape hatch: `$` opens an expression,
    /// and a second `$` before typing anything takes it back, so the shell
    /// can put a literal dollar in its place. The same shape as `//` for the
    /// slash menu.
    ///
    /// A display block is never discarded. `Block::Math` *is* its atom —
    /// `prune_block` rebuilds one the moment it is missing — so removing it
    /// here would be undone before anyone saw it.
    pub fn math_discard_if_empty(&mut self) -> bool {
        if self.math.is_none() {
            return false;
        }
        self.clamp_caret();
        let b = self.caret.block;
        let i = self.caret.inline;
        if self.scope()[b].is_math() {
            return false;
        }
        if self.scope()[b].is_table() {
            let at = self.scope()[b].cells()[i].run_at(self.caret.offset);
            if !matches!(
                self.scope()[b].cells()[i]
                    .lines()
                    .get(at.line)
                    .and_then(|line| line.get(at.run)),
                Some(Inline::Math(list)) if list.is_empty()
            ) {
                return false;
            }
            self.scope_mut()[b].cells_mut().expect("table row")[i].lines_mut()[at.line]
                .remove(at.run);
            self.math = None;
            self.caret.style = Style::PLAIN;
            self.dirty = true;
            self.enforce();
            return true;
        }
        match self.scope()[b].inlines().get(i) {
            Some(Inline::Math(list)) if list.is_empty() => {}
            _ => return false,
        }
        // The atom occupies exactly one flat position, and the caret sits at
        // its start; that position is where the text after it now begins.
        let flat = self.caret_flat(b);
        self.scope_mut()[b].inlines_mut().remove(i);
        self.math = None;
        self.dirty = true;
        self.enforce();
        let (inline, offset) = self.flat_to_pos(b, flat);
        self.set_caret(b, inline, offset);
        true
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
        if self.scope()[self.caret.block].is_table() {
            return None;
        }
        let label = self.next_free_label();
        let b = self.caret.block;
        let i = self.caret.inline;
        let o = self.caret.offset;
        let flat = self.caret_flat(b);
        let (prefix, suffix) = split_run(self.scope_mut()[b].inlines_mut().remove(i), o);
        self.scope_mut()[b]
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
                    .body
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
        if self.scope()[b].is_table() {
            return;
        }
        let at = if self.block_flat_len(b) == 0 {
            self.scope_mut()[b] = math_block();
            b
        } else {
            self.scope_mut().insert(b + 1, math_block());
            b + 1
        };
        self.set_caret(at, 0, 0);
        self.math = Some(math::MathCursor::default());
        self.dirty = true;
        self.enforce();
    }

    /// Tags the display math block the caret sits in with the next free
    /// `#eq:N` label, or removes its tag when it already carries one. The
    /// numbers themselves are derived at layout time in document order, so
    /// what is stored here can never disagree with what the page shows.
    /// Returns whether the caret was on a math block.
    pub fn toggle_math_tag(&mut self) -> bool {
        self.clamp_caret();
        let b = self.caret.block;
        if !matches!(self.scope()[b], Block::Math { .. }) {
            return false;
        }
        let had_tag = matches!(&self.scope()[b], Block::Math { tag: Some(_), .. });
        if had_tag {
            if let Block::Math { tag, .. } = &mut self.scope_mut()[b] {
                *tag = None;
            }
        } else {
            let label = self.next_free_eq_label();
            if let Block::Math { tag, .. } = &mut self.scope_mut()[b] {
                *tag = Some(label);
            }
        }
        self.dirty = true;
        self.enforce();
        true
    }

    /// The lowest `eq:N` label no equation block in the focused scope
    /// carries yet.
    fn next_free_eq_label(&self) -> String {
        let mut used: Vec<u32> = self
            .scope()
            .iter()
            .filter_map(|block| match block {
                Block::Math {
                    tag: Some(label), ..
                } => label.strip_prefix("eq:").and_then(|n| n.parse().ok()),
                _ => None,
            })
            .collect();
        used.sort_unstable();
        used.dedup();
        let mut n = 1;
        while used.binary_search(&n).is_ok() {
            n += 1;
        }
        format!("eq:{n}")
    }

    /// The focused atom's tree and cursor, or None when focus is stale.
    fn focused_math(&mut self) -> Option<(&mut math::MathList, &mut math::MathCursor)> {
        self.clamp_caret();
        let block = self.caret.block;
        let inline = self.caret.inline;
        // In a table row the caret's `inline` names a cell, so the atom is
        // located through the cell's own line/run arithmetic.
        let cell_at = self.scope()[block]
            .cells()
            .get(inline)
            .map(|cell| cell.run_at(self.caret.offset));
        // Borrow the atom and the cursor from disjoint fields: a method on
        // `self` holds all of it, so the cursor borrow would not fit
        // alongside.
        let blocks = match self.focus {
            Focus::Body => &mut self.body,
            Focus::Note(i) => &mut self.notes[i].body,
        };
        let Some(source) = blocks.get_mut(block) else {
            self.math = None;
            return None;
        };
        let list = if let Block::TableRow { cells, .. } = source {
            let Some(at) = cell_at else {
                self.math = None;
                return None;
            };
            match cells
                .get_mut(inline)
                .and_then(|cell| cell.lines_mut().get_mut(at.line))
                .and_then(|line| line.get_mut(at.run))
            {
                Some(Inline::Math(list)) => list,
                _ => {
                    self.math = None;
                    return None;
                }
            }
        } else {
            match source.inlines_mut().get_mut(inline) {
                Some(Inline::Math(list)) => list,
                _ => {
                    self.math = None;
                    return None;
                }
            }
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
        let block = &self.scope()[self.caret.block];
        let list = if block.is_table() {
            let cell = block.cells().get(self.caret.inline)?;
            let at = cell.run_at(self.caret.offset);
            match cell.lines().get(at.line).and_then(|line| line.get(at.run)) {
                Some(Inline::Math(list)) => list,
                _ => return None,
            }
        } else {
            match block.inlines().get(self.caret.inline)? {
                Inline::Math(list) => list,
                _ => return None,
            }
        };
        Some((list, cursor))
    }

    /// The precise token before the math cursor, including its tree location.
    pub fn math_conversion_query(&self) -> Option<math_conversion::Query> {
        let (list, cursor) = self.focused_math_view()?;
        math_conversion::query_before(list, cursor)
    }

    /// The addressed node in a rendered expression. `offset` is the
    /// expression's block-flat start, which disambiguates multiple math
    /// atoms living in one table cell.
    pub fn math_node_at(
        &self,
        block: usize,
        inline: usize,
        offset: usize,
        address: &math::NodeAddress,
    ) -> Option<&math::MathNode> {
        let block_ref = self.body.get(block)?;
        let list = if block_ref.is_table() {
            let cell = block_ref.cells().get(inline)?;
            let cell_base: usize = block_ref.cells()[..inline]
                .iter()
                .map(table::Cell::flat_len)
                .sum();
            let local = offset.checked_sub(cell_base)?;
            let at = cell.run_at(local);
            if at.offset != 0 || cell.run_start(at.line, at.run) != local {
                return None;
            }
            match cell.lines().get(at.line).and_then(|line| line.get(at.run)) {
                Some(Inline::Math(list)) => list,
                _ => return None,
            }
        } else {
            match block_ref.inlines().get(inline)? {
                Inline::Math(list) => list,
                _ => return None,
            }
        };
        math::node_at(list, address)
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
        offset: usize,
        mutation: impl FnOnce(&mut math::MathList) -> bool,
    ) -> bool {
        let Some(block_ref) = self.body.get(block) else {
            return false;
        };
        let table_at = if block_ref.is_table() {
            let Some(cell) = block_ref.cells().get(inline) else {
                return false;
            };
            let cell_base: usize = block_ref.cells()[..inline]
                .iter()
                .map(table::Cell::flat_len)
                .sum();
            let Some(local) = offset.checked_sub(cell_base) else {
                return false;
            };
            let at = cell.run_at(local);
            if at.offset != 0 || cell.run_start(at.line, at.run) != local {
                return false;
            }
            Some((at.line, at.run))
        } else {
            None
        };
        let Some(target) = self.body.get_mut(block) else {
            return false;
        };
        let list = if target.is_table() {
            let Some((line, run)) = table_at else {
                return false;
            };
            match target
                .cells_mut()
                .and_then(|cells| cells.get_mut(inline))
                .and_then(|cell| cell.lines_mut().get_mut(line))
                .and_then(|line| line.get_mut(run))
            {
                Some(Inline::Math(list)) => list,
                _ => return false,
            }
        } else {
            match target.inlines_mut().get_mut(inline) {
                Some(Inline::Math(list)) => list,
                _ => return false,
            }
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
        offset: usize,
        address: &math::NodeAddress,
        role: math::SymbolRole,
    ) -> bool {
        self.mutate_math_at(block, inline, offset, |list| {
            math::set_node_role(list, address, role)
        })
    }

    pub fn set_math_node_variant_at(
        &mut self,
        block: usize,
        inline: usize,
        offset: usize,
        address: &math::NodeAddress,
        variant: &str,
    ) -> bool {
        self.mutate_math_at(block, inline, offset, |list| {
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
        offset: usize,
        address: &math::NodeAddress,
        open: char,
    ) -> bool {
        self.mutate_math_at(block, inline, offset, |list| {
            math::set_group_delimiter(list, address, open)
        })
    }

    pub fn set_math_accent_kind_at(
        &mut self,
        block: usize,
        inline: usize,
        offset: usize,
        address: &math::NodeAddress,
        kind: math::AccentKind,
    ) -> bool {
        self.mutate_math_at(block, inline, offset, |list| {
            math::set_accent_kind(list, address, kind)
        })
    }

    pub fn set_math_big_op_kind_at(
        &mut self,
        block: usize,
        inline: usize,
        offset: usize,
        address: &math::NodeAddress,
        kind: math::BigOp,
    ) -> bool {
        self.mutate_math_at(block, inline, offset, |list| {
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
        if self.scope()[block].is_table() {
            let cell = self.caret.inline;
            let flat = self.caret.offset;
            if flat == 0 {
                return false;
            }
            let cell_ref = &self.scope()[block].cells()[cell];
            let at = cell_ref.run_at(flat - 1);
            let list_len = match cell_ref
                .lines()
                .get(at.line)
                .and_then(|line| line.get(at.run))
            {
                Some(Inline::Math(list)) => list.len(),
                _ => return false,
            };
            self.caret.offset = cell_ref.run_start(at.line, at.run);
            self.caret.style = Style::PLAIN;
            self.math = Some(math::MathCursor {
                path: Vec::new(),
                index: list_len,
            });
            return true;
        }
        let flat = self.caret_flat(block);
        if flat == 0 {
            return false;
        }
        let (inline, _) = self.flat_to_pos(block, flat - 1);
        let list_len = match self.scope()[block].inlines().get(inline) {
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
        if self.scope()[block].is_table() {
            let cell = self.caret.inline;
            let flat = self.caret.offset;
            let cell_ref = &self.scope()[block].cells()[cell];
            if flat >= cell_ref.flat_len() {
                return false;
            }
            let at = cell_ref.run_at(flat);
            if !matches!(
                cell_ref
                    .lines()
                    .get(at.line)
                    .and_then(|line| line.get(at.run)),
                Some(Inline::Math(_))
            ) {
                return false;
            }
            self.caret.offset = cell_ref.run_start(at.line, at.run);
            self.caret.style = Style::PLAIN;
            self.math = Some(math::MathCursor::default());
            return true;
        }
        let flat = self.caret_flat(block);
        if flat >= self.block_flat_len(block) {
            return false;
        }
        let (inline, _) = self.flat_to_pos(block, flat);
        if !matches!(
            self.scope()[block].inlines().get(inline),
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
    pub fn enter_math_at(
        &mut self,
        block: usize,
        inline: usize,
        offset: usize,
        mut cursor: math::MathCursor,
    ) {
        let Some(block_ref) = self.body.get(block) else {
            return;
        };
        let local_offset = if block_ref.is_table() {
            if block_ref.cells().get(inline).is_none() {
                return;
            }
            let cell_base: usize = block_ref.cells()[..inline]
                .iter()
                .map(table::Cell::flat_len)
                .sum();
            let Some(local) = offset.checked_sub(cell_base) else {
                return;
            };
            local
        } else {
            0
        };
        self.set_caret(block, inline, local_offset);
        let target_block = &self.body[self.caret.block];
        let list = if target_block.is_table() {
            let Some(cell) = target_block.cells().get(self.caret.inline) else {
                return;
            };
            let at = cell.run_at(local_offset);
            if at.offset != 0 || cell.run_start(at.line, at.run) != local_offset {
                return;
            }
            match cell.lines().get(at.line).and_then(|line| line.get(at.run)) {
                Some(Inline::Math(list)) => list,
                _ => return,
            }
        } else {
            match target_block.inlines().get(self.caret.inline) {
                Some(Inline::Math(list)) => list,
                _ => return,
            }
        };
        math::clamp(list, &mut cursor);
        self.math = Some(cursor);
    }

    fn math_exit_at(&mut self, offset: usize) {
        self.clamp_caret();
        let block = self.caret.block;
        let inline = self.caret.inline;
        if self.scope()[block].is_table() {
            let is_math = {
                let cell = &self.scope()[block].cells()[inline];
                let at = cell.run_at(self.caret.offset);
                matches!(
                    cell.lines().get(at.line).and_then(|line| line.get(at.run)),
                    Some(Inline::Math(_))
                )
            };
            if !is_math {
                self.math = None;
                return;
            }
            self.math = None;
            self.caret.offset += offset;
            self.refresh_context();
            return;
        } else if !matches!(
            self.scope()[block].inlines().get(inline),
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

    /// Restore a caret from its block-flat position after run structure changes.
    fn restore_caret_flat(&mut self, block: usize, flat: usize) {
        if block >= self.scope().len() {
            return;
        }
        self.caret.block = block;
        let (inline, offset) = self.flat_to_pos(block, flat);
        self.caret.inline = inline;
        self.caret.offset = offset;
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
        doc.body()[block]
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
        assert!(!d.body().is_empty());
        for block in d
            .body()
            .iter()
            .chain(d.notes.iter().flat_map(|note| note.body.iter()))
        {
            if block.is_table() {
                assert!(!block.cells().is_empty(), "a table row keeps a cell");
                for cell in block.cells() {
                    assert!(!cell.lines().is_empty(), "a cell keeps a line");
                    for line in cell.lines() {
                        assert!(!line.is_empty(), "a line keeps a run");
                        for (i, run) in line.iter().enumerate() {
                            assert!(
                                !(run.text().is_empty() && (line.len() > 1 || i != 0)),
                                "no empty run outside a sole placeholder"
                            );
                        }
                    }
                }
                continue;
            }
            assert!(!block.inlines().is_empty());
            for (i, run) in block.inlines().iter().enumerate() {
                assert!(!(run.text().is_empty() && (block.inlines().len() > 1 || i != 0)));
            }
        }
        for note in &d.notes {
            assert!(!note.body.is_empty());
        }
        assert!(d.caret.block < d.scope().len());
        // The unit arithmetic addresses a run or a table cell alike.
        let caret_block = &d.scope()[d.caret.block];
        assert!(d.caret.inline < caret_block.unit_count());
        assert!(d.caret.offset <= caret_block.unit_len(d.caret.inline));
    }

    #[test]
    fn new_document_is_one_empty_paragraph() {
        let d = doc();
        assert_eq!(d.body().len(), 1);
        assert!(matches!(d.body()[0], Block::Paragraph(ref r) if r.len() == 1));
        assert!(d.body()[0].inlines()[0].text().is_empty());
        assert_eq!(d.body()[0].inlines()[0].style(), Style::PLAIN);
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
        *d.body_mut() = vec![
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
                folded: false,
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
                folded: false,
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
        let block = &d.body()[0];
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
            runs(&d.body()[0]),
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
        d.body_mut()[0] = Block::Paragraph(vec![plain_run("ab"), bold_run("cd"), plain_run("ef")]);
        // Inside the bold run at its end; set_caret's style-before rule
        // gives bold context.
        d.set_caret(0, 1, 2);
        assert_eq!(d.caret.style, bold());
        d.insert_text("X");
        assert_eq!(
            runs(&d.body()[0]),
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
        d.body_mut()[0] = Block::Paragraph(vec![plain_run("ab"), bold_run("cd")]);
        d.set_caret(0, 0, 2); // after "ab", before-style is plain
        d.toggle_bold(); // context becomes the right run's bold
        d.insert_text("X");
        assert_eq!(
            runs(&d.body()[0]),
            vec![("ab".into(), Style::PLAIN), ("Xcd".into(), bold())]
        );
        assert_eq!((d.caret.inline, d.caret.offset), (1, 1));
        assert_invariants(&d);
    }

    #[test]
    fn insert_bold_context_between_plain_runs_splices_new_run() {
        let mut d = doc();
        d.body_mut()[0] = Block::Paragraph(vec![plain_run("ab"), plain_run("ef")]);
        d.set_caret(0, 0, 2); // boundary between the plain runs
        d.toggle_bold();
        d.insert_text("X");
        assert_eq!(
            runs(&d.body()[0]),
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
        d.body_mut()[0] = Block::Paragraph(vec![plain_run("hello")]);
        d.set_caret(0, 0, 5);
        d.toggle_bold();
        d.insert_text("X");
        assert_eq!(
            runs(&d.body()[0]),
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
            matches!(d.body()[0], Block::Heading { level: 1, .. }),
            "an empty line can become a heading"
        );
        d.insert_text("Title");
        assert!(
            matches!(d.body()[0], Block::Heading { level: 1, .. }),
            "typing into it must keep the heading kind"
        );
        assert_eq!(runs(&d.body()[0]), vec![("Title".into(), Style::PLAIN)]);
        assert_eq!((d.caret.inline, d.caret.offset), (0, 5));
        assert_invariants(&d);
    }

    #[test]
    fn set_heading_on_empty_line_body_round_trips() {
        let mut d = doc();
        d.set_heading(Some(2));
        assert!(matches!(d.body()[0], Block::Heading { level: 2, .. }));
        d.set_heading(None);
        assert!(!d.body()[0].is_heading());
        assert_invariants(&d);
    }

    #[test]
    fn level_four_heading_is_a_heading() {
        let mut d = doc();
        d.set_heading(Some(4));
        assert!(matches!(d.body()[0], Block::Heading { level: 4, .. }));
        assert_invariants(&d);
    }

    #[test]
    fn backspace_mid_run() {
        let mut d = doc();
        d.insert_text("abc");
        d.set_caret(0, 0, 2);
        d.backspace();
        assert_eq!(runs(&d.body()[0]), vec![("ac".into(), Style::PLAIN)]);
        assert_eq!((d.caret.inline, d.caret.offset), (0, 1));
        assert_invariants(&d);
    }

    #[test]
    fn backspace_at_run_boundary_deletes_previous_run_last_char() {
        let mut d = doc();
        d.body_mut()[0] = Block::Paragraph(vec![plain_run("ab"), plain_run("cd")]);
        d.set_caret(0, 1, 0);
        d.backspace();
        assert_eq!(text_of_block(&d, 0), "acd");
        assert_eq!(
            d.caret_position(),
            FlatPos {
                block: 0,
                offset: 1
            }
        );
        assert_invariants(&d);
    }

    #[test]
    fn the_caret_keeps_its_position_in_the_text_when_pruning_merges_the_run_it_was_in() {
        let mut d = doc();
        d.body_mut()[0] = Block::Paragraph(vec![plain_run("ab"), plain_run("cd")]);
        d.set_caret(0, 1, 1);

        d.enforce();

        assert_eq!(runs(&d.body()[0]), vec![("abcd".into(), Style::PLAIN)]);
        assert_eq!(
            d.caret_position(),
            FlatPos {
                block: 0,
                offset: 3
            }
        );
        assert_eq!((d.caret.inline, d.caret.offset), (0, 3));
        assert_invariants(&d);
    }

    #[test]
    fn backspace_at_block_start_merges_blocks() {
        let mut d = doc();
        *d.body_mut() = vec![
            Block::Paragraph(vec![plain_run("hello")]),
            Block::Paragraph(vec![bold_run("world")]),
        ];
        d.set_caret(1, 0, 0);
        d.backspace();
        assert_eq!(d.body().len(), 1);
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
        d.body_mut()[0] = Block::Paragraph(vec![plain_run("ab"), bold_run("cd")]);
        d.set_caret(0, 1, 1); // inside the bold run
        d.newline();
        assert_eq!(
            runs(&d.body()[0]),
            vec![("ab".into(), Style::PLAIN), ("c".into(), bold())]
        );
        assert_eq!(runs(&d.body()[1]), vec![("d".into(), bold())]);
        assert_eq!(d.caret.block, 1);
        assert_eq!((d.caret.inline, d.caret.offset), (0, 0));
        assert_eq!(d.caret.style, bold()); // context preserved
        assert_invariants(&d);
    }

    #[test]
    fn newline_at_end_of_heading_creates_paragraph() {
        let mut d = doc();
        d.body_mut()[0] = Block::Heading {
            level: 1,
            folded: false,
            content: vec![plain_run("title")],
        };
        d.set_caret(0, 0, 5);
        d.newline();
        assert!(d.body()[0].is_heading());
        assert_eq!(text_of_block(&d, 0), "title");
        assert!(!d.body()[1].is_heading());
        assert_eq!(text_of_block(&d, 1), "");
        assert_eq!(d.caret.block, 1);
        assert_eq!((d.caret.inline, d.caret.offset), (0, 0));
        assert_invariants(&d);
    }

    // ---- list item tests -------------------------------------------------

    fn bullet_item(t: &str) -> Block {
        Block::ListItem {
            marker: ListMarker::Bullet,
            content: vec![plain_run(t)],
        }
    }

    fn ordered_item(n: u32, t: &str) -> Block {
        Block::ListItem {
            marker: ListMarker::Number(n),
            content: vec![plain_run(t)],
        }
    }

    fn task_item(done: bool, t: &str) -> Block {
        Block::ListItem {
            marker: ListMarker::Task { done },
            content: vec![plain_run(t)],
        }
    }

    fn marker_of(d: &Document, block: usize) -> ListMarker {
        match &d.body()[block] {
            Block::ListItem { marker, .. } => *marker,
            other => panic!("block {block} is not a list item: {other:?}"),
        }
    }

    #[test]
    fn enter_continues_a_list_item_of_the_same_kind() {
        let mut d = doc();
        *d.body_mut() = vec![ordered_item(1, "first"), ordered_item(2, "second")];
        d.set_caret(1, 0, 6);
        d.newline();
        assert_eq!(d.body().len(), 3);
        assert_eq!(marker_of(&d, 2), ListMarker::Number(3));
        assert_eq!(text_of_block(&d, 2), "");
        assert_eq!(d.caret.block, 2);
        assert_invariants(&d);
    }

    #[test]
    fn enter_on_an_empty_list_item_leaves_the_list() {
        let mut d = doc();
        *d.body_mut() = vec![bullet_item("tea"), bullet_item("")];
        d.set_caret(1, 0, 0);
        d.newline();
        // The empty item becomes the empty paragraph the user asked for;
        // the list above it is untouched.
        assert_eq!(d.body().len(), 2);
        assert!(matches!(d.body()[1], Block::Paragraph(_)));
        assert!(matches!(d.body()[0], Block::ListItem { .. }));
        assert_eq!(d.caret.block, 1);
        assert_invariants(&d);
    }

    #[test]
    fn enter_below_a_task_opens_an_unticked_one() {
        let mut d = doc();
        *d.body_mut() = vec![task_item(true, "done thing")];
        d.set_caret(0, 0, 10);
        d.newline();
        assert_eq!(marker_of(&d, 1), ListMarker::Task { done: false });
        assert_invariants(&d);
    }

    #[test]
    fn backspace_at_an_item_start_demotes_it_to_a_paragraph() {
        let mut d = doc();
        *d.body_mut() = vec![bullet_item("tea")];
        d.set_caret(0, 0, 0);
        d.backspace();
        assert!(matches!(d.body()[0], Block::Paragraph(_)));
        assert_eq!(text_of_block(&d, 0), "tea");
        assert_eq!((d.caret.block, d.caret.inline, d.caret.offset), (0, 0, 0));
        assert_invariants(&d);
    }

    #[test]
    fn backspace_again_merges_like_any_paragraph() {
        let mut d = doc();
        *d.body_mut() = vec![
            Block::Paragraph(vec![plain_run("keep")]),
            bullet_item("tea"),
        ];
        d.set_caret(1, 0, 0);
        d.backspace(); // demote
        d.backspace(); // merge
        assert_eq!(d.body().len(), 1);
        assert!(matches!(d.body()[0], Block::Paragraph(_)));
        assert_eq!(text_of_block(&d, 0), "keeptea");
        assert_invariants(&d);
    }

    #[test]
    fn backspace_twice_at_an_item_start_merges_into_the_item_above() {
        let mut d = doc();
        *d.body_mut() = vec![bullet_item("a"), bullet_item("b")];
        d.set_caret(1, 0, 0);
        d.backspace(); // the marker goes first
        assert!(matches!(d.body()[1], Block::Paragraph(_)));
        d.backspace(); // then the demoted paragraph merges like any other
        assert!(matches!(d.body()[0], Block::ListItem { .. }));
        assert_eq!(text_of_block(&d, 0), "ab");
        assert_invariants(&d);
    }

    #[test]
    fn o_and_o_continue_a_list_run() {
        let mut d = doc();
        *d.body_mut() = vec![ordered_item(1, "one"), ordered_item(2, "two")];
        d.set_caret(1, 0, 3);
        d.open_below();
        assert_eq!(marker_of(&d, 2), ListMarker::Number(3));
        d.open_above();
        // `O` inserts above the caret: the new item takes the caret block's
        // ordinal, and the renumbering pass keeps the run canonical.
        assert_eq!(marker_of(&d, 2), ListMarker::Number(3));
        assert_eq!(marker_of(&d, 3), ListMarker::Number(4));
        assert_invariants(&d);
    }

    #[test]
    fn ordered_runs_renumber_after_deletions() {
        let mut d = doc();
        *d.body_mut() = vec![
            ordered_item(1, "a"),
            ordered_item(2, "b"),
            ordered_item(3, "c"),
        ];
        d.delete_line(); // removes "a"; caret lands on the old "b"
        assert_eq!(marker_of(&d, 0), ListMarker::Number(1));
        assert_eq!(marker_of(&d, 1), ListMarker::Number(2));
        assert_invariants(&d);
    }

    #[test]
    fn set_list_converts_and_toggles_off() {
        let mut d = doc();
        d.insert_text("step one");
        d.set_list(Some(ListMarker::Number(1)));
        assert!(matches!(
            d.body()[0],
            Block::ListItem {
                marker: ListMarker::Number(1),
                ..
            }
        ));
        // Same kind again: back to prose.
        d.set_list(Some(ListMarker::Number(1)));
        assert!(matches!(d.body()[0], Block::Paragraph(_)));
        assert_eq!(text_of_block(&d, 0), "step one");
        assert_invariants(&d);
    }

    #[test]
    fn a_converted_paragraph_keeps_its_runs() {
        let mut d = doc();
        d.body_mut()[0] = Block::Paragraph(vec![bold_run("bold"), plain_run(" rest")]);
        d.set_list(Some(ListMarker::Bullet));
        assert_eq!(
            runs(&d.body()[0]),
            vec![("bold".into(), bold()), (" rest".into(), Style::PLAIN)]
        );
        assert_invariants(&d);
    }

    #[test]
    fn toggling_a_task_flips_its_checkbox_in_place() {
        let mut d = doc();
        *d.body_mut() = vec![task_item(false, "buy milk")];
        assert!(d.toggle_task_at(0));
        assert_eq!(marker_of(&d, 0), ListMarker::Task { done: true });
        assert!(d.is_dirty());
        assert!(d.toggle_task_at(0));
        assert_eq!(marker_of(&d, 0), ListMarker::Task { done: false });
        // A non-task block says no.
        *d.body_mut() = vec![Block::Paragraph(vec![plain_run("prose")])];
        assert!(!d.toggle_task_at(0));
        assert_invariants(&d);
    }

    #[test]
    fn set_list_is_a_noop_inside_a_note() {
        let mut d = doc();
        d.insert_text("anchored");
        d.insert_sidenote().expect("note created");
        d.focus = Focus::Note(0);
        d.set_caret(0, 0, 0);
        d.set_list(Some(ListMarker::Bullet));
        // A note body is one paragraph on disk; it cannot hold a list.
        assert!(matches!(d.scope()[0], Block::Paragraph(_)));
        assert_invariants(&d);
    }

    #[test]
    fn delete_line_on_only_block_leaves_empty_paragraph() {
        let mut d = doc();
        d.insert_text("abc");
        d.set_heading(Some(1));
        d.delete_line();
        assert_eq!(d.body().len(), 1);
        assert!(!d.body()[0].is_heading());
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
        assert_eq!(d.body().len(), 1);
        assert_eq!(text_of_block(&d, 0), "  indented");
        assert_eq!(d.caret.block, 0);
        assert_eq!((d.caret.inline, d.caret.offset), (0, 2));
        assert!(d.caret.style.is_plain());
        assert_invariants(&d);
    }

    #[test]
    fn style_caret_at_end_of_bold_run_pops_before_moving() {
        let mut d = doc();
        d.body_mut()[0] = Block::Paragraph(vec![plain_run("ab"), bold_run("cd"), plain_run("ef")]);
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
        d.body_mut()[0] = Block::Paragraph(vec![plain_run("ab"), bold_run("cd"), plain_run("ef")]);
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
        d.body_mut()[0] = Block::Paragraph(vec![plain_run("ab"), bold_run("cd")]);
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
        *d.body_mut() = vec![
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
        assert!(matches!(d.body()[0], Block::Heading { level: 2, .. }));
        assert_eq!(runs(&d.body()[0]), vec![("hello".into(), Style::PLAIN)]);
        d.set_heading(None);
        assert!(!d.body()[0].is_heading());
        assert_eq!(runs(&d.body()[0]), vec![("hello".into(), Style::PLAIN)]);
        assert_invariants(&d);
    }

    #[test]
    fn a_divider_inserted_on_a_blank_line_replaces_it() {
        let mut d = doc();
        d.insert_divider();
        assert_eq!(d.body().len(), 2);
        assert!(d.body()[0].is_divider());
        assert_eq!(runs(&d.body()[0]), vec![(String::new(), Style::PLAIN)]);
        assert_eq!(text_of_block(&d, 1), "");
        assert_eq!(d.caret.block, 1);
        assert_invariants(&d);
    }

    #[test]
    fn a_divider_after_text_keeps_the_text() {
        let mut d = doc();
        d.insert_text("text");
        d.insert_divider();
        assert_eq!(d.body().len(), 3);
        assert_eq!(text_of_block(&d, 0), "text");
        assert!(d.body()[1].is_divider());
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
        assert!(matches!(d.body()[0], Block::Paragraph(_)));
        assert_eq!(text_of_block(&d, 0), "typed");
        assert_eq!(d.body().len(), 2);
        assert_eq!(d.caret.block, 0);
        assert_invariants(&d);
    }

    #[test]
    fn move_home_and_move_end_logical_and_style_before() {
        let mut d = doc();
        d.body_mut()[0] = Block::Paragraph(vec![plain_run("ab"), bold_run("cd")]);
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
        d.body_mut()[0] = Block::Paragraph(vec![plain_run("a"), bold_run("bcd")]);
        d.set_caret(0, 0, 1);
        d.delete_char();
        assert_eq!(text_of_block(&d, 0), "acd");
        assert_eq!(
            runs(&d.body()[0]),
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
        d.body_mut()[0] = Block::Paragraph(vec![plain_run("ab"), bold_run("cdef")]);
        d.set_caret(0, 0, 0);
        for _ in 0..4 {
            d.delete_char();
        }
        assert_eq!(runs(&d.body()[0]), vec![("ef".into(), bold())]);
        assert_eq!((d.caret.inline, d.caret.offset), (0, 0));
        d.delete_char(); // caret now on the bold run, deletion continues
        assert_eq!(text_of_block(&d, 0), "f");
        assert_invariants(&d);
    }

    #[test]
    fn an_atom_counts_as_one_char() {
        let mut d = doc();
        d.body_mut()[0] = Block::Paragraph(vec![
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
            d.body()[0].inlines()[d.caret.inline],
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
            d.body()[0].inlines(),
            [Inline::Text(Text { text: left, .. }), Inline::Math(_), Inline::Text(Text { text: right, .. })]
                if left == "ab" && right == "cd"
        ));
        assert_eq!((d.caret.inline, d.caret.offset), (1, 0));
        assert!(d.math.is_some());
    }

    /// `$` opens an expression and a second `$` takes it back, so a literal
    /// dollar is still typeable. The atom must leave no trace: the run it
    /// split rejoins, and the caret lands where the dollar goes.
    #[test]
    fn an_untouched_inline_expression_is_discarded_whole() {
        let mut d = doc();
        d.insert_text("abcd");
        d.set_caret(0, 0, 2);
        d.insert_inline_math();
        assert!(d.math.is_some());

        assert!(d.math_discard_if_empty());
        assert!(d.math.is_none());
        assert!(
            matches!(d.body()[0].inlines(), [Inline::Text(Text { text, .. })] if text == "abcd"),
            "the split run rejoins: {:?}",
            d.body()[0].inlines()
        );
        d.insert_text("$");
        assert!(
            matches!(d.body()[0].inlines(), [Inline::Text(Text { text, .. })] if text == "ab$cd")
        );
    }

    #[test]
    fn an_expression_with_anything_in_it_is_never_discarded() {
        let mut d = doc();
        d.insert_inline_math();
        d.math_insert_char('x');
        assert!(!d.math_discard_if_empty());
        assert!(d.math.is_some(), "focus stays put when nothing was removed");
        assert!(matches!(d.body()[0].inlines(), [Inline::Math(list)] if list.len() == 1));
    }

    /// A display block *is* its atom — `prune_block` rebuilds one the moment
    /// it is missing — so discarding it would be undone before it was seen.
    #[test]
    fn an_empty_display_block_is_not_discarded() {
        let mut d = doc();
        d.insert_math_block();
        assert!(d.math.is_some());
        assert!(!d.math_discard_if_empty());
        assert!(d.body().iter().any(Block::is_math));
    }

    #[test]
    fn typing_beside_an_atom_never_merges_into_it() {
        let mut d = doc();
        d.insert_inline_math();
        d.set_caret(0, 0, 1);
        d.insert_text("x");
        assert!(matches!(d.body()[0].inlines()[0], Inline::Math(_)));
        assert_eq!(d.block_text(0), format!("{ATOM}x"));
        assert!(
            matches!(d.body()[0].inlines()[1], Inline::Text(Text { ref text, .. }) if text == "x")
        );
    }

    #[test]
    fn backspace_enters_a_nonempty_atom_then_removes_it_only_when_empty() {
        let mut d = doc();
        d.body_mut()[0] = Block::Paragraph(vec![
            plain_run("before"),
            Inline::Math(vec![math::MathNode::Sym('x')]),
            plain_run("after"),
        ]);
        d.set_caret(0, 2, 0);

        d.backspace();
        assert!(matches!(d.body()[0].inlines()[1], Inline::Math(ref list) if list.is_empty()));
        assert_eq!(d.math, Some(math::MathCursor::default()));

        d.math_exit_after();
        d.backspace();
        assert_eq!(d.block_text(0), "beforeafter");
        assert!(
            d.body()[0]
                .inlines()
                .iter()
                .all(|inline| !matches!(inline, Inline::Math(_)))
        );
    }

    #[test]
    fn delete_forward_enters_a_nonempty_atom_then_removes_it_only_when_empty() {
        let mut d = doc();
        d.body_mut()[0] = Block::Paragraph(vec![
            plain_run("before"),
            Inline::Math(vec![math::MathNode::Sym('x')]),
            plain_run("after"),
        ]);
        d.set_caret(0, 0, 6);

        d.delete_forward();
        assert!(matches!(d.body()[0].inlines()[1], Inline::Math(ref list) if list.is_empty()));
        assert_eq!(d.math, Some(math::MathCursor::default()));

        d.math_exit_before();
        d.delete_forward();
        assert_eq!(d.block_text(0), "beforeafter");
    }

    #[test]
    fn normal_delete_starts_inside_a_math_atom() {
        let mut d = doc();
        d.body_mut()[0] = Block::Paragraph(vec![Inline::Math(vec![math::MathNode::Sym('x')])]);
        d.set_caret(0, 0, 0);

        d.delete_char();

        assert!(matches!(d.body()[0].inlines()[0], Inline::Math(ref list) if list.is_empty()));
        assert_eq!(d.math, Some(math::MathCursor::default()));
    }

    #[test]
    fn deleting_around_an_empty_display_atom_preserves_the_math_block() {
        let mut d = doc();
        d.insert_math_block();
        d.math_exit_after();

        d.backspace();
        assert!(d.body()[0].is_math());
        assert!(matches!(d.body()[0].inlines(), [Inline::Math(list)] if list.is_empty()));
        assert_eq!(d.block_text(0), ATOM.to_string());
    }

    #[test]
    fn backspace_from_prose_after_display_math_enters_without_merging_blocks() {
        let mut d = doc();
        *d.body_mut() = vec![
            Block::Math {
                list: vec![Inline::Math(vec![math::MathNode::Sym('x')])],
                tag: None,
            },
            Block::Paragraph(vec![plain_run("after")]),
        ];
        d.set_caret(1, 0, 0);

        d.backspace();

        assert_eq!(d.body().len(), 2);
        assert!(matches!(d.body()[0], Block::Math {
                list: ref inlines, ..
            } if matches!(inlines.as_slice(), [Inline::Math(list)] if list.is_empty())));
        assert_eq!(d.block_text(1), "after");
        assert_eq!(d.caret.block, 0);
        assert_eq!(d.math, Some(math::MathCursor::default()));
    }

    #[test]
    fn delete_from_prose_before_display_math_enters_without_merging_blocks() {
        let mut d = doc();
        *d.body_mut() = vec![
            Block::Paragraph(vec![plain_run("before")]),
            Block::Math {
                list: vec![Inline::Math(vec![math::MathNode::Sym('x')])],
                tag: None,
            },
        ];
        d.set_caret(0, 0, 6);

        d.delete_forward();

        assert_eq!(d.body().len(), 2);
        assert_eq!(d.block_text(0), "before");
        assert!(matches!(d.body()[1], Block::Math {
                list: ref inlines, ..
            } if matches!(inlines.as_slice(), [Inline::Math(list)] if list.is_empty())));
        assert_eq!(d.caret.block, 1);
        assert_eq!(d.math, Some(math::MathCursor::default()));
    }

    #[test]
    fn delete_after_display_math_retains_the_block_boundary() {
        let mut d = doc();
        *d.body_mut() = vec![
            Block::Math {
                list: vec![Inline::Math(vec![math::MathNode::Sym('x')])],
                tag: None,
            },
            Block::Paragraph(vec![plain_run("after")]),
        ];
        d.set_caret(0, 0, 1);

        d.delete_forward();

        assert_eq!(d.body().len(), 2);
        assert!(d.body()[0].is_math());
        assert_eq!(d.block_text(1), "after");
        assert_eq!(d.caret.block, 0);
        assert_eq!(d.caret.offset, 1);
        assert!(d.math.is_none());
    }

    #[test]
    fn a_math_block_that_gains_prose_demotes_to_a_paragraph() {
        let mut d = doc();
        d.body_mut()[0] = Block::Math {
            list: vec![
                Inline::Math(vec![math::MathNode::Sym('x')]),
                plain_run(" prose"),
            ],
            tag: None,
        };
        d.enforce();
        assert!(matches!(d.body()[0], Block::Paragraph(_)));
        assert!(matches!(d.body()[0].inlines()[0], Inline::Math(_)));
    }

    #[test]
    fn a_caret_placed_on_folded_ground_lands_on_the_fold() {
        let mut d = doc();
        d.insert_text("intro");
        d.newline();
        d.set_heading(Some(1));
        d.insert_text("Section");
        d.newline();
        d.insert_text("body one");
        d.newline();
        d.insert_text("body two");
        // Fold the heading (block 1); blocks 2 and 3 become hidden ground.
        if let Block::Heading { folded, .. } = &mut d.body_mut()[1] {
            *folded = true;
        }
        let owner = fold_owner_of(d.body(), 3).expect("block 3 is folded ground");
        assert_eq!(owner, 1);
        // A jump (vim G, gg, {, } all land here) must not park the caret
        // where nothing is visible.
        d.set_caret(3, 0, 0);
        assert_eq!(d.caret.block, 1, "the caret lands on the fold's heading");
    }

    #[test]
    fn opening_below_a_folded_heading_reveals_the_body() {
        let mut d = doc();
        d.set_heading(Some(1));
        d.insert_text("Section");
        d.newline();
        d.insert_text("body");
        if let Block::Heading { folded, .. } = &mut d.body_mut()[0] {
            *folded = true;
        }
        // vim `o` ON the folded heading (the reviewer's repro): the caret
        // sits on the heading, so the new paragraph would be born at block
        // 1 — inside the fold. The fold must open instead of hiding it.
        d.set_caret(0, 0, 0);
        d.open_below();
        assert!(
            !d.body()[0].is_folded(),
            "the fold opened to give the new paragraph light"
        );
        assert!(!fold_owner_of(d.body(), d.caret.block).is_some());
    }

    #[test]
    fn typing_at_a_display_atom_joins_the_tree_instead_of_demoting_it() {
        let mut d = doc();
        d.insert_math_block();
        d.math_insert_char('x');
        // Exit to the right, then type: the characters must enter the atom
        // after `x`, not splice prose beside it and demote the block.
        d.math_exit_after();
        d.insert_text("+y");
        assert!(
            matches!(d.body()[0], Block::Math { .. }),
            "the display block must survive typing at its edge"
        );
        let list = match &d.body()[0].inlines()[0] {
            Inline::Math(list) => list,
            other => panic!("expected math, got {other:?}"),
        };
        let printed = math_notation::print(list);
        assert!(printed.contains('x') && printed.contains('y'), "{printed}");
    }

    #[test]
    fn typing_at_a_display_atom_rests_the_caret_on_the_side_it_had() {
        let mut d = doc();
        d.insert_math_block();
        d.math_insert_char('a');
        d.math_exit_before();
        // Caret before the atom: new characters go to the front.
        d.insert_text("b");
        match &d.body()[0].inlines()[0] {
            Inline::Math(list) => {
                let printed = math_notation::print(list);
                assert!(printed.starts_with('b'), "{printed}");
            }
            other => panic!("expected math, got {other:?}"),
        }
    }

    #[test]
    fn toggle_math_tag_assigns_the_next_free_label() {
        let mut d = doc();
        d.insert_math_block();
        assert!(d.toggle_math_tag());
        assert!(matches!(
            &d.body()[0],
            Block::Math { tag: Some(tag), .. } if tag == "eq:1"
        ));

        // A second tagged block never reuses the label; an untagged one in
        // between takes no number and claims nothing.
        d.insert_math_block();
        d.toggle_math_tag();
        assert!(matches!(
            &d.body()[1],
            Block::Math { tag: Some(tag), .. } if tag == "eq:2"
        ));

        // Toggling again removes the tag.
        assert!(d.toggle_math_tag());
        assert!(matches!(&d.body()[1], Block::Math { tag: None, .. }));

        // The freed label is the next one handed out.
        d.toggle_math_tag();
        assert!(matches!(
            &d.body()[1],
            Block::Math { tag: Some(tag), .. } if tag == "eq:2"
        ));
    }

    #[test]
    fn toggle_math_tag_is_a_no_op_off_a_math_block() {
        let mut d = doc();
        assert!(!d.toggle_math_tag());
        assert!(matches!(d.body()[0], Block::Paragraph(_)));
    }

    #[test]
    fn a_pasted_reference_reopens_as_a_reference() {
        let mut d = doc();
        // Paste takes `insert_notation` — the copy path wrote this text.
        d.insert_notation("by @eq:gain above");
        assert!(
            d.body()[0]
                .inlines()
                .iter()
                .any(|run| matches!(run, Inline::EqRef(l) if l == "eq:gain"))
        );
    }

    #[test]
    fn insert_math_block_replaces_an_empty_block() {
        let mut d = doc();
        d.insert_math_block();
        assert_eq!(d.body().len(), 1);
        assert!(d.body()[0].is_math());
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
            d.body()[0].inlines()[0],
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
            d.body()[0].inlines()[0],
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
            d.body()[0].inlines()[0],
            Inline::Math(vec![math::MathNode::Sqrt { body: Vec::new() }])
        );
    }

    #[test]
    fn arrowing_back_over_an_atom_enters_it() {
        let mut d = doc();
        d.body_mut()[0] = Block::Paragraph(vec![
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
        d.body_mut()[0] = Block::Paragraph(vec![Inline::Math(vec![math::MathNode::Frac {
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

        d.enter_math_at(0, 0, 0, cursor.clone());

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
            d.body()[0].inlines()[0],
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
        *d.body_mut() = vec![
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

        assert!(d.set_math_node_variant_at(0, 1, 6, &at(0), "bold"));
        assert!(matches!(
            &d.body()[0].inlines()[1],
            Inline::Math(list)
                if matches!(&list[0], math::MathNode::Resolved {
                    role: math::SymbolRole::Variable,
                    variant,
                    ..
                } if variant == "bold")
        ));
        assert!(d.set_math_node_role_at(0, 1, 6, &at(0), math::SymbolRole::Constant));
        assert!(d.set_math_group_delimiter_at(0, 1, 6, &at(1), '['));
        assert!(d.set_math_accent_kind_at(0, 1, 6, &at(2), math::AccentKind::Dot));
        assert!(d.set_math_big_op_kind_at(0, 1, 6, &at(3), math::BigOp::ContourIntegral));
        assert!(d.is_dirty());
        assert_eq!(d.caret, caret);
        assert!(d.math.is_none());
        assert!(matches!(
            &d.body()[0].inlines()[1],
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
        assert!(!d.set_math_node_role_at(0, 1, 6, &at(0), math::SymbolRole::Constant));
        assert!(!d.set_math_node_variant_at(0, 1, 6, &at(0), "bold"));
        assert!(!d.set_math_group_delimiter_at(0, 1, 6, &at(1), '['));
        assert!(!d.set_math_accent_kind_at(0, 1, 6, &at(2), math::AccentKind::Dot));
        assert!(!d.set_math_big_op_kind_at(0, 1, 6, &at(3), math::BigOp::ContourIntegral));
        assert!(!d.set_math_node_variant_at(0, 1, 6, &at(4), "missing"));
        assert!(!d.set_math_node_role_at(9, 9, 0, &at(0), math::SymbolRole::Variable));
        assert!(!d.is_dirty());
        assert_eq!(d.caret, caret);
        assert!(matches!(
            &d.body()[0].inlines()[1],
            Inline::Math(list) if matches!(list.get(4), Some(math::MathNode::Sym('q')))
        ));
    }

    #[test]
    fn a_raw_greek_letter_accepts_and_persists_a_variant() {
        let mut d = doc();
        d.body_mut()[0] = Block::Paragraph(vec![Inline::Math(vec![math::MathNode::Sym('α')])]);
        let address = math::NodeAddress {
            path: Vec::new(),
            index: 0,
        };

        assert!(d.set_math_node_variant_at(0, 0, 0, &address, "bold"));
        let Inline::Math(list) = &d.body()[0].inlines()[0] else {
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
        *d.body_mut() = vec![
            Block::Paragraph(content.clone()),
            Block::Paragraph(vec![plain_run("caret")]),
        ];
        d.set_caret(1, 0, 3);
        let caret = d.caret;

        assert!(d.set_block_heading_at(0, Some(2)));
        assert!(matches!(
            &d.body()[0],
            Block::Heading { level: 2, content: actual, .. } if actual == &content
        ));
        assert_eq!(d.caret, caret);

        d.dirty = false;
        assert!(!d.set_block_heading_at(0, Some(2)));
        assert!(!d.is_dirty());
        assert!(d.set_block_code_at(0, true));
        assert!(matches!(
            &d.body()[0],
            Block::CodeLine { content, first: true, lang: None }
                if content.len() == 3 && matches!(content[1], Inline::Math(_))
        ));
        assert_eq!(d.caret, caret);

        assert!(d.set_block_code_at(0, false));
        assert!(matches!(&d.body()[0], Block::Paragraph(content) if content.len() == 3));
        assert_eq!(d.caret, caret);
    }

    #[test]
    fn enter_math_before_ignores_prose() {
        let mut d = doc();
        d.body_mut()[0] = Block::Paragraph(vec![plain_run("text")]);
        d.set_caret(0, 0, 1);

        assert!(!d.enter_math_before());
        assert!(d.math.is_none());
    }

    #[test]
    fn a_stale_focus_is_dropped_by_clamp() {
        let mut d = doc();
        d.insert_inline_math();
        d.body_mut().remove(0);
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
                folded: false,
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
        assert!(d.body()[0].is_code());
        assert_eq!(
            runs(&d.body()[0]),
            vec![("hello world".into(), code_style())]
        );
        assert_invariants(&d);
        d.set_code(false);
        assert!(!d.body()[0].is_code());
        assert_eq!(
            runs(&d.body()[0]),
            vec![("hello world".into(), Style::PLAIN)]
        );
        assert_invariants(&d);
    }

    #[test]
    fn set_code_collapses_multi_run_paragraph_into_one_run() {
        let mut d = doc();
        d.body_mut()[0] = Block::Paragraph(vec![plain_run("ab"), bold_run("cd")]);
        d.set_code(true);
        assert_eq!(runs(&d.body()[0]), vec![("abcd".into(), code_style())]);
        assert_invariants(&d);
    }

    #[test]
    fn set_code_clamps_caret_mid_block() {
        let mut d = doc();
        d.insert_text("hello world");
        d.set_caret(0, 0, 5);
        d.set_code(true);
        assert!(d.body()[0].is_code());
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
        assert!(d.body()[0].is_code());
        d.set_heading(Some(1));
        let block = &d.body()[0];
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
        assert_eq!(d.body().len(), 2);
        assert!(d.body()[0].is_code());
        assert!(d.body()[1].is_code());
        assert_eq!(d.caret.block, 1);
        assert_eq!((d.caret.inline, d.caret.offset), (0, 0));
        // Split creates a continuation: first == false, lang == None
        assert!(
            matches!(
                d.body()[1],
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
        assert!(d.body()[0].is_code());
        assert_eq!(d.body()[0].inlines().len(), 1);
        assert!(d.body()[0].inlines()[0].text().is_empty());
        assert_eq!(d.body()[0].inlines()[0].style(), code_style());
        assert_invariants(&d);
    }

    #[test]
    fn prune_runs_emptied_code_line_stays_code_line() {
        let mut d = doc();
        d.insert_text("x");
        d.set_code(true);
        d.set_caret(0, 0, 0);
        d.delete_char(); // removes the only char
        assert!(d.body()[0].is_code(), "emptied code line stays a code line");
        assert_eq!(d.body()[0].inlines().len(), 1);
        assert_invariants(&d);
    }

    #[test]
    fn flat_range_delete_preserves_runs_and_merges_block_edges() {
        let mut d = doc();
        *d.body_mut() = vec![
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
        assert_eq!(d.body().len(), 1);
        assert_invariants(&d);
    }

    #[test]
    fn text_objects_find_words_quotes_parens_and_nested_heading_section() {
        let mut d = doc();
        *d.body_mut() = vec![
            Block::Heading {
                level: 1,
                folded: false,
                content: vec![plain_run("Top")],
            },
            Block::Paragraph(vec![plain_run("body (inside) and \"quoted\"")]),
            Block::Heading {
                level: 2,
                folded: false,
                content: vec![plain_run("Nested")],
            },
            Block::Paragraph(vec![plain_run("child")]),
            Block::Heading {
                level: 1,
                folded: false,
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
        *d.body_mut() = vec![
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
        d.body_mut()[0] = Block::Paragraph(vec![plain_run("hello world")]);
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
            runs(&d.body()[0]),
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
            runs(&d.body()[0])
                .iter()
                .all(|(_, style)| *style == Style::PLAIN)
        );
        assert_invariants(&d);
    }

    #[test]
    fn a_word_that_is_already_italic_opens_a_menu_with_the_italic_row_checked() {
        let italic = Style {
            italic: true,
            ..Style::PLAIN
        };
        let mut d = doc();
        d.body_mut()[0] = Block::Paragraph(vec![Inline::Text(Text {
            text: "hello".into(),
            style: italic,
        })]);
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

        assert!(d.style_range_is_active(range, italic));
    }

    #[test]
    fn a_word_that_is_only_partly_italic_opens_a_menu_with_the_italic_row_unchecked() {
        let italic = Style {
            italic: true,
            ..Style::PLAIN
        };
        let mut d = doc();
        d.body_mut()[0] = Block::Paragraph(vec![
            Inline::Text(Text {
                text: "hel".into(),
                style: italic,
            }),
            plain_run("lo"),
        ]);
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

        assert!(!d.style_range_is_active(range, italic));
        d.toggle_style_range(range, italic);
        assert!(d.style_range_is_active(range, italic));
    }

    #[test]
    fn a_plain_word_opens_a_menu_with_nothing_checked() {
        let mut d = doc();
        d.body_mut()[0] = Block::Paragraph(vec![plain_run("hello")]);
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
        for mask in [
            Style {
                bold: true,
                ..Style::PLAIN
            },
            Style {
                italic: true,
                ..Style::PLAIN
            },
            Style {
                highlight: true,
                ..Style::PLAIN
            },
            Style {
                code: true,
                ..Style::PLAIN
            },
            Style {
                badge: true,
                ..Style::PLAIN
            },
        ] {
            assert!(!d.style_range_is_active(range, mask));
        }
    }

    #[test]
    fn a_word_italicised_and_un_italicised_leaves_the_block_with_the_run_structure_it_started_with_whatever_block_the_caret_is_in_and_wherever_in_it_the_caret_sits()
     {
        let italic = Style {
            italic: true,
            ..Style::PLAIN
        };

        for block in 0..2 {
            for offset in [0, 1, 5, 6, 11] {
                let mut d = doc();
                *d.body_mut() = vec![
                    Block::Paragraph(vec![plain_run("hello world")]),
                    Block::Paragraph(vec![plain_run("hello world")]),
                ];
                d.set_caret(block, 0, offset);
                let range =
                    FlatRange::new(FlatPos { block, offset: 0 }, FlatPos { block, offset: 5 });

                d.toggle_style_range(range, italic);
                d.toggle_style_range(range, italic);

                assert_eq!(
                    runs(&d.body()[block]),
                    vec![("hello world".into(), Style::PLAIN)]
                );
                assert_eq!(d.caret_position(), FlatPos { block, offset });
                assert_invariants(&d);
            }
        }
    }

    #[test]
    fn toggle_style_range_flips_inline_code_on_selected_text() {
        let mut d = doc();
        d.body_mut()[0] = Block::Paragraph(vec![plain_run("hello world")]);
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
            runs(&d.body()[0]),
            vec![("hello".into(), code), (" world".into(), Style::PLAIN)]
        );
        d.toggle_style_range(range, code);
        assert_eq!(text_of_block(&d, 0), "hello world");
        assert!(
            runs(&d.body()[0])
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
        d.body_mut()[0] = Block::Paragraph(vec![
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
        let styled = runs(&d.body()[0]);
        assert_eq!(styled.len(), 2);
        assert_eq!(styled[0].1.badge_color, BadgeColor::Blue);
        assert_eq!(styled[1].1.badge_color, BadgeColor::Orange);

        d.toggle_style_range(first, badge);
        assert_eq!(runs(&d.body()[0])[0].1, Style::PLAIN);
        assert_invariants(&d);
    }

    #[test]
    fn copying_a_range_with_an_expression_yields_its_notation() {
        let mut d = doc();
        *d.body_mut() = vec![Block::Paragraph(vec![
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
        *source.body_mut() = vec![Block::Paragraph(vec![
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
            target.body()[0].inlines(),
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
        *d.body_mut() = vec![Block::Paragraph(vec![
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
        *d.body_mut() = vec![Block::Paragraph(vec![plain_run("costs $40 and $12")])];
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
        let block = &d.body()[0];
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
        let block = &d.body()[0];
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
        d.body_mut()[0] = Block::Paragraph(vec![
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
            d.body()[0].inlines()[d.caret.inline],
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
        let blocks_before = d.body().to_vec();
        d.focus = Focus::Note(0);
        d.insert_text("a note");
        assert_eq!(
            d.notes[0].body,
            vec![Block::Paragraph(vec![plain_run("a note")])]
        );
        assert_eq!(d.body(), blocks_before, "the body is untouched");
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
        d.body_mut()[0]
            .inlines_mut()
            .retain(|run| !matches!(run, Inline::Note(_)));
        d.enforce();
        assert!(d.notes.is_empty(), "dropping the anchor drops the note");
        assert_eq!(d.focus, Focus::Body, "focus falls back to the body");
        assert!(d.caret.block < d.body().len(), "caret stays in bounds");
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
        assert_eq!(text_of_block(&d, 0), format!("{ATOM}body"));
    }

    #[test]
    fn table_cells_keep_their_positions_through_typing_and_navigation() {
        let mut d = doc();
        d.insert_table();
        assert_eq!(d.body().len(), 2);
        assert!(d.body().iter().all(Block::is_table));
        assert_eq!(d.body()[0].cells().len(), 2);
        assert!(d.body()[0].table_first());
        assert!(!d.body()[1].table_first());

        d.insert_text("left");
        d.table_tab(false);
        assert_eq!((d.caret.block, d.caret.inline, d.caret.offset), (0, 1, 0));
        d.insert_text("right");
        assert_eq!(d.block_text(0), "leftright");
        assert_eq!(cell_text(&d.body()[0].cells()[0]), "left");
        assert_eq!(cell_text(&d.body()[0].cells()[1]), "right");

        d.table_move(TableDirection::Down);
        assert_eq!((d.caret.block, d.caret.inline), (1, 1));
        d.set_caret(1, 1, 0);
        d.table_move(TableDirection::Left);
        assert_eq!(d.caret.inline, 0);
        d.table_tab(true);
        assert_eq!((d.caret.block, d.caret.inline), (0, 1));
        assert_invariants(&d);
    }

    #[test]
    fn table_line_and_track_edits_update_the_whole_group() {
        let mut d = doc();
        d.insert_table();
        assert!(d.toggle_table_line(0, table::GridLine::Top));
        assert!(d.resize_table_column(0, 0, 0.1));
        assert!(d.resize_table_row(0, 0, 4.0));
        let first = d.body()[0].table_settings().unwrap();
        let second = d.body()[1].table_settings().unwrap();
        assert_eq!(first, second);
        assert!(!first.lines.top);
        assert!(first.column_shares[0] > first.column_shares[1]);
        assert!(first.row_heights[0] > first.row_heights[1]);

        d.set_caret(0, 0, 0);
        d.newline();
        assert_eq!(
            d.body().len(),
            2,
            "Enter does not create a hidden table row"
        );
        assert!(d.insert_table_row(0, 0));
        assert_eq!(d.body().len(), 3);
        assert!(d.body().iter().all(Block::is_table));
        assert_eq!(d.body()[0].table_settings().unwrap().row_heights.len(), 3);
        assert_invariants(&d);
    }

    #[test]
    fn table_card_can_add_and_remove_tracks_without_losing_its_shape() {
        let mut d = doc();
        d.insert_table();
        assert!(d.insert_table_row(0, 0));
        assert!(d.insert_table_column(0, 0));
        assert_eq!(d.body().len(), 3);
        assert!(d.body().iter().all(|row| row.cells().len() == 3));
        let settings = d.body()[0].table_settings().unwrap();
        assert_eq!(settings.row_heights.len(), 3);
        assert_eq!(settings.column_shares.len(), 3);
        assert!((settings.column_shares.iter().sum::<f32>() - 1.0).abs() < 0.0001);

        assert!(d.remove_table_row(0, 0));
        assert!(d.remove_table_column(0, 1));
        assert_eq!(d.body().len(), 2);
        assert!(d.body()[0].table_first());
        assert!(d.body().iter().all(|row| row.cells().len() == 2));
        assert_invariants(&d);
    }

    #[test]
    fn formatting_a_table_word_preserves_its_cell_boundary() {
        let mut d = doc();
        d.insert_table();
        d.insert_text("alpha");
        let bold = Style {
            bold: true,
            ..Style::PLAIN
        };
        let range = FlatRange::new(d.position(0, 1), d.position(0, 4));
        d.toggle_style_range(range, bold);
        assert_eq!(d.body()[0].cells().len(), 2);
        assert!(
            d.body()[0].cells()[0]
                .runs()
                .iter()
                .any(|run| run.style().bold)
        );
        assert!(d.style_range_is_active(range, bold));
        d.toggle_style_range(range, bold);
        assert!(
            d.body()[0].cells()[0]
                .runs()
                .iter()
                .all(|run| !run.style().bold)
        );
        assert_invariants(&d);
    }

    #[test]
    fn leaving_bold_in_a_table_preserves_the_bold_text_already_typed() {
        let mut d = doc();
        d.insert_table();
        d.set_emphasis(true, false);
        d.insert_text("bold");
        d.set_emphasis(false, false);
        d.insert_text(" plain");

        let contents = d.body()[0].cells()[0].runs();
        assert!(
            matches!(&contents[0], Inline::Text(text) if text.text == "bold" && text.style.bold)
        );
        assert!(
            matches!(&contents[1], Inline::Text(text) if text.text == " plain" && text.style == Style::PLAIN)
        );
        assert_eq!(cell_text(&d.body()[0].cells()[0]), "bold plain");
        assert_invariants(&d);
    }

    #[test]
    fn an_empty_table_cell_can_hold_editable_math() {
        let mut d = doc();
        d.insert_table();
        d.insert_inline_math();
        d.math_insert_char('x');
        assert!(d.math.is_some());
        assert!(matches!(d.body()[0].cells()[0].runs(), [Inline::Math(_)]));
        assert_eq!(d.body()[0].cells().len(), 2);
        assert_eq!(d.body()[1].cells().len(), 2);

        d.math_exit_after();
        d.insert_inline_math();
        assert!(d.math.is_some(), "the math cell can be re-entered");
        assert_invariants(&d);
    }

    #[test]
    fn block_commands_cannot_break_a_table_group() {
        let mut d = doc();
        d.insert_table();
        let original = d.body().to_vec();
        d.set_heading(Some(1));
        d.set_code(true);
        d.set_list(Some(ListMarker::Bullet));
        d.insert_divider();
        d.insert_math_block();
        d.insert_table();
        assert_eq!(d.body(), original);
        // Enter is a cell edit, not a block command: it splits the cell's
        // own line and leaves every row and track exactly as it was.
        d.newline();
        assert!(d.body().iter().all(Block::is_table));
        assert_eq!(d.body().len(), 2);
        assert!(d.body().iter().all(|row| row.cells().len() == 2));
        assert_eq!(d.body()[0].cells()[0].lines().len(), 2);
        assert_eq!(d.body()[1].cells()[0].lines().len(), 1);

        d.open_above();
        assert!(matches!(d.body()[0], Block::Paragraph(_)));
        assert!(d.body()[1].table_first());

        let mut below = doc();
        below.insert_table();
        below.open_below();
        assert!(matches!(below.body()[2], Block::Paragraph(_)));
        assert_eq!(below.caret.block, 2);
        assert_invariants(&d);
        assert_invariants(&below);
    }

    #[test]
    fn backspace_and_line_delete_never_remove_table_tracks() {
        let mut d = doc();
        d.insert_table();
        d.insert_text("cell");
        d.set_caret(0, 0, 0);
        d.backspace();
        assert_eq!(cell_text(&d.body()[0].cells()[0]), "cell");
        assert_eq!(d.body().len(), 2);
        assert!(d.body().iter().all(|row| row.cells().len() == 2));

        d.delete_line();
        assert_eq!(cell_text(&d.body()[0].cells()[0]), "");
        assert_eq!(d.body().len(), 2);
        assert!(d.body().iter().all(|row| row.cells().len() == 2));
        assert_invariants(&d);
    }
}
