//! The persistent geometry and chrome choices of one Markdown table.
//!
//! A table is written as ordinary pipe Markdown. Its grid is a Typewritter
//! presentation choice, however, so the line switches and reader-resized
//! tracks live beside the table in a small file-local comment.

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
