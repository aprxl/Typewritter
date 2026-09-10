//! One table cell, and the persistent geometry and chrome choices of the
//! table it belongs to.
//!
//! A table is written as ordinary pipe Markdown. Its grid is a Typewritter
//! presentation choice, however, so the line switches and reader-resized
//! tracks live beside the table in a small file-local comment.

use super::{Inline, Style, Text};

/// One table cell: one or more lines of inline runs.
///
/// A cell is a container, not a run: its content is addressed by
/// `(line, offset)` and its flat space costs one position per run plus one per
/// line break, exactly the way an atom costs one. Nothing in the run list has
/// to know a cell exists, which is what keeps every prose path total.
#[derive(Clone, PartialEq, Debug)]
pub struct Cell {
    pub lines: Vec<Vec<Inline>>,
}

impl Default for Cell {
    fn default() -> Self {
        Self::new()
    }
}

impl Cell {
    /// An empty cell: one line holding one empty text run — the invariant
    /// every block keeps, so a cell is never a special case for the renderer.
    pub fn new() -> Self {
        Self::from_runs(vec![Inline::Text(Text {
            text: String::new(),
            style: Style::PLAIN,
        })])
    }

    /// A single-line cell holding `runs`.
    pub fn from_runs(runs: Vec<Inline>) -> Self {
        Self { lines: vec![runs] }
    }

    /// The cell's first line — the whole cell while it holds one line, which
    /// is what every single-line consumer wants.
    pub fn runs(&self) -> &[Inline] {
        self.lines.first().map_or(&[], Vec::as_slice)
    }

    pub fn runs_mut(&mut self) -> &mut Vec<Inline> {
        if self.lines.is_empty() {
            self.lines.push(Vec::new());
        }
        &mut self.lines[0]
    }

    pub fn lines(&self) -> &[Vec<Inline>] {
        &self.lines
    }

    pub fn lines_mut(&mut self) -> &mut Vec<Vec<Inline>> {
        &mut self.lines
    }

    /// Every run of every line, in cell order.
    pub fn all_runs(&self) -> impl Iterator<Item = &Inline> {
        self.lines.iter().flat_map(|line| line.iter())
    }

    /// Flat length: runs cost their own length, each line break costs one.
    pub fn flat_len(&self) -> usize {
        let runs: usize = self
            .lines
            .iter()
            .map(|line| line.iter().map(super::flat_len).sum::<usize>())
            .sum();
        runs + self.lines.len().saturating_sub(1)
    }

    /// Cell-flat offset to `(line, offset on that line)`.
    pub fn position(&self, flat: usize) -> (usize, usize) {
        let mut remaining = flat;
        for (index, line) in self.lines.iter().enumerate() {
            let len: usize = line.iter().map(super::flat_len).sum();
            if remaining <= len || index + 1 == self.lines.len() {
                return (index, remaining.min(len));
            }
            remaining -= len + 1;
        }
        (0, 0)
    }

    /// `(line, offset)` to a cell-flat offset.
    pub fn flat_of(&self, line: usize, offset: usize) -> usize {
        let line = line.min(self.lines.len().saturating_sub(1));
        let mut flat = 0;
        for index in 0..line {
            flat += self.lines[index].iter().map(super::flat_len).sum::<usize>() + 1;
        }
        let len: usize = self.lines[line].iter().map(super::flat_len).sum();
        flat + offset.min(len)
    }

    /// A position inside the cell: which line, which run on that line, and
    /// the char offset within that run.
    pub fn run_at(&self, flat: usize) -> CellPos {
        let (line, offset) = self.position(flat);
        let mut position = 0;
        for (run, value) in self.lines[line].iter().enumerate() {
            let len = super::flat_len(value);
            if offset < position + len {
                return CellPos {
                    line,
                    run,
                    offset: offset - position,
                };
            }
            position += len;
        }
        let run = self.lines[line].len().saturating_sub(1);
        CellPos {
            line,
            run,
            offset: super::flat_len(&self.lines[line][run]),
        }
    }

    /// Cell-flat offset of the first char of `(line, run)`.
    pub fn run_start(&self, line: usize, run: usize) -> usize {
        let line = line.min(self.lines.len().saturating_sub(1));
        let mut flat = 0;
        for index in 0..line {
            flat += self.lines[index].iter().map(super::flat_len).sum::<usize>() + 1;
        }
        flat + self.lines[line][..run.min(self.lines[line].len())]
            .iter()
            .map(super::flat_len)
            .sum::<usize>()
    }

    /// Cell-flat offset of `(line, run, offset)`.
    pub fn at(&self, line: usize, run: usize, offset: usize) -> usize {
        self.run_start(line, run) + offset
    }

    /// Style of the char at cell-flat `flat`, or `None` at the cell's end.
    pub fn style_at(&self, flat: usize) -> Option<Style> {
        let (line, offset) = self.position(flat);
        let mut position = 0;
        for run in &self.lines[line] {
            let len = super::flat_len(run);
            if offset < position + len {
                return Some(run.style());
            }
            position += len;
        }
        None
    }

    /// Style of the char before cell-flat `flat`.
    pub fn style_before(&self, flat: usize) -> Option<Style> {
        flat.checked_sub(1).and_then(|position| self.style_at(position))
    }

    /// Whether the cell holds nothing but whitespace.
    pub fn is_blank(&self) -> bool {
        self.all_runs()
            .filter_map(|run| match run {
                Inline::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .all(|text| text.chars().all(char::is_whitespace))
            && self.all_runs().all(|run| matches!(run, Inline::Text(_)))
    }
}

/// A position inside a cell.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CellPos {
    pub line: usize,
    pub run: usize,
    pub offset: usize,
}

/// The default height of a freshly inserted table row, in logical pixels.
pub const DEFAULT_ROW_HEIGHT: f32 = 38.0;
/// A cell never shrinks below this height while a reader drags a row divider.
pub const MIN_ROW_HEIGHT: f32 = 24.0;
/// The least share of the table a column can occupy while it is dragged.
pub const MIN_COLUMN_SHARE: f32 = 0.08;

/// One selectable stroke in the table-line picker.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GridLine {
    Top,
    Bottom,
    Left,
    Right,
    Horizontal,
    Vertical,
}

/// Which strokes of a table grid are visible. The two inner switches apply
/// to every interior row or column divider, keeping the picker compact while
/// still allowing a completely line-free layout table.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TableLines {
    pub top: bool,
    pub bottom: bool,
    pub left: bool,
    pub right: bool,
    pub horizontal: bool,
    pub vertical: bool,
}

impl Default for TableLines {
    fn default() -> Self {
        Self {
            top: true,
            bottom: true,
            left: true,
            right: true,
            horizontal: true,
            vertical: true,
        }
    }
}

impl TableLines {
    pub fn enabled(self, line: GridLine) -> bool {
        match line {
            GridLine::Top => self.top,
            GridLine::Bottom => self.bottom,
            GridLine::Left => self.left,
            GridLine::Right => self.right,
            GridLine::Horizontal => self.horizontal,
            GridLine::Vertical => self.vertical,
        }
    }

    pub fn toggle(&mut self, line: GridLine) {
        match line {
            GridLine::Top => self.top = !self.top,
            GridLine::Bottom => self.bottom = !self.bottom,
            GridLine::Left => self.left = !self.left,
            GridLine::Right => self.right = !self.right,
            GridLine::Horizontal => self.horizontal = !self.horizontal,
            GridLine::Vertical => self.vertical = !self.vertical,
        }
    }

    /// Compact, deterministic on-disk spelling: top, bottom, left, right,
    /// horizontal, then vertical.
    pub fn bits(self) -> String {
        [
            self.top,
            self.bottom,
            self.left,
            self.right,
            self.horizontal,
            self.vertical,
        ]
        .iter()
        .map(|on| if *on { '1' } else { '0' })
        .collect()
    }

    pub fn from_bits(bits: &str) -> Option<Self> {
        let bytes = bits.as_bytes();
        if bytes.len() != 6 || !bytes.iter().all(|byte| matches!(byte, b'0' | b'1')) {
            return None;
        }
        Some(Self {
            top: bytes[0] == b'1',
            bottom: bytes[1] == b'1',
            left: bytes[2] == b'1',
            right: bytes[3] == b'1',
            horizontal: bytes[4] == b'1',
            vertical: bytes[5] == b'1',
        })
    }
}

/// The saved, reader-controlled dimensions for a table. Column widths are
/// shares, so a table always fills the editable measure after a window resize.
/// Row heights are logical pixels and therefore retain the rhythm the reader
/// set while still respecting the editor's display scale.
#[derive(Clone, PartialEq, Debug)]
pub struct TableSettings {
    pub lines: TableLines,
    pub column_shares: Vec<f32>,
    pub row_heights: Vec<f32>,
}

impl TableSettings {
    pub fn new(columns: usize, rows: usize) -> Self {
        let columns = columns.max(1);
        Self {
            lines: TableLines::default(),
            column_shares: vec![1.0 / columns as f32; columns],
            row_heights: vec![DEFAULT_ROW_HEIGHT; rows.max(1)],
        }
    }

    /// Repairs foreign or stale metadata into a usable shape. This is called
    /// only at table boundaries, never on the typing path.
    pub fn normalized(mut self, columns: usize, rows: usize) -> Self {
        let columns = columns.max(1);
        let rows = rows.max(1);
        if self.column_shares.len() != columns
            || self
                .column_shares
                .iter()
                .any(|share| !share.is_finite() || *share <= 0.0)
        {
            self.column_shares = vec![1.0 / columns as f32; columns];
        } else {
            let total: f32 = self.column_shares.iter().sum();
            for share in &mut self.column_shares {
                *share /= total;
            }
        }
        if self.row_heights.len() != rows {
            self.row_heights.resize(rows, DEFAULT_ROW_HEIGHT);
        }
        for height in &mut self.row_heights {
            if !height.is_finite() {
                *height = DEFAULT_ROW_HEIGHT;
            }
            *height = height.max(MIN_ROW_HEIGHT);
        }
        self
    }
}
