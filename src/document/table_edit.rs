//! Structural table edits.
//!
//! A table is a run of `Block::TableRow` blocks, and every edit here is a
//! document-level operation on that run and its shared settings. Cell-level
//! arithmetic — lines, flat offsets, run positions — lives on
//! [`super::table::Cell`] itself.

use super::math_notation;
use super::table;
use super::{BadgeColor, Block, Document, FlatPos, Focus, Inline, Style, TableDirection};
use super::{flat_len, placeholder_if_empty, slice_inline_runs, table_row};

/// The extent of one table in the block stream. `first` and `end` are the
/// half-open row range; `rows` and `columns` are its shape.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TableRange {
    pub first: usize,
    pub end: usize,
    pub rows: usize,
    pub columns: usize,
}

impl Document {
    /// Move through table cells in row-major order. The last cell wraps to
    /// the first (and vice versa for Shift-Tab), which is the useful cycling
    /// behaviour while filling in a small layout table.
    pub fn table_tab(&mut self, backwards: bool) -> bool {
        self.clamp_caret();
        let block = self.caret.block;
        let Some(range) = self.table_bounds(block) else {
            return false;
        };
        let (first, end, columns) = (range.first, range.end, range.columns);
        let row = block - first;
        let cell = self.caret.inline.min(columns.saturating_sub(1));
        let total = (end - first) * columns;
        let current = row * columns + cell;
        let next = if backwards {
            (current + total - 1) % total
        } else {
            (current + 1) % total
        };
        let next_block = first + next / columns;
        let next_cell = next % columns;
        self.set_caret(next_block, next_cell, 0);
        true
    }

    /// Navigate the table grid in the arrow's physical direction. Horizontal
    /// movement keeps character-level editing inside a cell, then crosses a
    /// cell boundary; vertical movement preserves the character offset.
    pub fn table_move(&mut self, direction: TableDirection) -> bool {
        self.clamp_caret();
        let block = self.caret.block;
        let Some(range) = self.table_bounds(block) else {
            return false;
        };
        let (first, end, columns) = (range.first, range.end, range.columns);
        let cell = self.caret.inline.min(columns.saturating_sub(1));
        let offset = self.caret.offset;
        let len = self.scope()[block].cells()[cell].flat_len();
        match direction {
            TableDirection::Left if offset > 0 => {
                self.set_caret(block, cell, offset - 1);
                true
            }
            TableDirection::Left if cell > 0 => {
                let previous = cell - 1;
                let end = self.scope()[block].cells()[previous].flat_len();
                self.set_caret(block, previous, end);
                true
            }
            TableDirection::Right if offset < len => {
                self.set_caret(block, cell, offset + 1);
                true
            }
            TableDirection::Right if cell + 1 < columns => {
                self.set_caret(block, cell + 1, 0);
                true
            }
            TableDirection::Up if block > first => {
                let target = block - 1;
                let target_len = self.scope()[target].cells()[cell].flat_len();
                self.set_caret(target, cell, offset.min(target_len));
                true
            }
            TableDirection::Down if block + 1 < end => {
                let target = block + 1;
                let target_len = self.scope()[target].cells()[cell].flat_len();
                self.set_caret(target, cell, offset.min(target_len));
                true
            }
            _ => false,
        }
    }

    /// Enter inside a table cell: split the cell's current line at the
    /// caret's own line offset. The break costs one flat position, so the
    /// caret lands on the new line by moving one past its insert point.
    /// Returns whether a line was split.
    pub fn split_cell_line(&mut self) -> bool {
        self.clamp_caret();
        let block = self.caret.block;
        if !self.scope()[block].is_table() {
            return false;
        }
        let cell = self.caret.inline;
        let flat = self.caret.offset;
        let (line, offset) = self.scope()[block].cells()[cell].position(flat);
        let (head, tail) = {
            let runs = self.scope()[block].cells()[cell].lines()[line].as_slice();
            let len: usize = runs.iter().map(flat_len).sum();
            (
                slice_inline_runs(runs, 0, offset),
                slice_inline_runs(runs, offset, len),
            )
        };
        let contents = &mut self.scope_mut()[block].cells_mut().expect("table row")[cell];
        let lines = contents.lines_mut();
        lines[line] = placeholder_if_empty(head);
        lines.insert(line + 1, placeholder_if_empty(tail));
        self.caret.offset = flat + 1;
        self.dirty = true;
        self.enforce_block(block);
        true
    }

    /// Backspace at the start of a cell line: fold it onto the line above,
    /// leaving the caret at the junction. On the first line of a cell (and
    /// at the start of a cell) nothing is joined. Returns whether a line was
    /// joined.
    pub fn join_cell_line(&mut self) -> bool {
        self.clamp_caret();
        let block = self.caret.block;
        if !self.scope()[block].is_table() {
            return false;
        }
        let cell = self.caret.inline;
        let flat = self.caret.offset;
        let (line, offset) = self.scope()[block].cells()[cell].position(flat);
        if offset != 0 || line == 0 {
            return false;
        }
        self.merge_cell_line(cell, line - 1);
        self.caret.offset = flat - 1;
        self.dirty = true;
        self.enforce_block(block);
        true
    }

    /// Delete at the end of a cell line: fold the next line onto this one,
    /// leaving the caret at the junction. On the last line of a cell nothing
    /// is joined. Returns whether a line was joined.
    pub fn join_cell_line_forward(&mut self) -> bool {
        self.clamp_caret();
        let block = self.caret.block;
        if !self.scope()[block].is_table() {
            return false;
        }
        let cell = self.caret.inline;
        let flat = self.caret.offset;
        let (line, offset) = self.scope()[block].cells()[cell].position(flat);
        let line_len: usize = self.scope()[block].cells()[cell].lines()[line]
            .iter()
            .map(flat_len)
            .sum();
        if offset != line_len || line + 1 >= self.scope()[block].cells()[cell].lines().len() {
            return false;
        }
        self.merge_cell_line(cell, line);
        self.caret.offset = flat;
        self.dirty = true;
        self.enforce_block(block);
        true
    }

    /// Fold the cell line after `line` onto `line` — the one geometric core
    /// of both joins. The caret is left where the caller put it; `enforce`
    /// clamps it into the joined line.
    fn merge_cell_line(&mut self, cell: usize, line: usize) {
        let block = self.caret.block;
        let mut joined = self.scope()[block].cells()[cell].lines()[line].clone();
        joined.extend_from_slice(&self.scope()[block].cells()[cell].lines()[line + 1]);
        let contents = &mut self.scope_mut()[block].cells_mut().expect("table row")[cell];
        contents.lines_mut()[line] = joined;
        contents.lines_mut().remove(line + 1);
    }

    /// Inserts the standard two-by-two table below the caret (or replaces an
    /// empty paragraph) and starts typing in its first cell.
    pub fn insert_table(&mut self) {
        self.clamp_caret();
        if matches!(self.focus, Focus::Note(_)) {
            return;
        }
        let block = self.caret.block;
        if self.scope()[block].is_table() {
            return;
        }
        let settings = std::sync::Arc::new(table::TableSettings::new(2, 2));
        let rows = [
            table_row(2, true, std::sync::Arc::clone(&settings)),
            table_row(2, false, settings),
        ];
        let first = if self.block_flat_len(block) == 0 && !self.scope()[block].is_table() {
            self.scope_mut().splice(block..=block, rows);
            block
        } else {
            self.scope_mut().splice(block + 1..block + 1, rows);
            block + 1
        };
        self.dirty = true;
        self.enforce();
        self.set_caret(first, 0, 0);
    }

    /// Toggle one grid stroke for the table containing `block`.
    pub fn toggle_table_line(&mut self, block: usize, line: table::GridLine) -> bool {
        if !matches!(self.focus, Focus::Body) {
            return false;
        }
        let Some(range) = self.table_bounds(block) else {
            return false;
        };
        let (first, end, columns) = (range.first, range.end, range.columns);
        let rows = end - first;
        let mut settings = self.scope()[first]
            .table_settings()
            .expect("table bounds names a table")
            .clone()
            .normalized(columns, rows);
        settings.lines.toggle(line);
        self.set_table_settings(first, end, std::sync::Arc::new(settings));
        self.dirty = true;
        true
    }

    /// Drag the divider after column `divider`, transferring width only to
    /// its right-hand neighbour so the editable table width remains fixed.
    pub fn resize_table_column(&mut self, block: usize, divider: usize, delta_share: f32) -> bool {
        if !matches!(self.focus, Focus::Body) {
            return false;
        }
        let Some(range) = self.table_bounds(block) else {
            return false;
        };
        let (first, end, columns) = (range.first, range.end, range.columns);
        if divider + 1 >= columns || !delta_share.is_finite() {
            return false;
        }
        let rows = end - first;
        let mut settings = self.scope()[first]
            .table_settings()
            .expect("table bounds names a table")
            .clone()
            .normalized(columns, rows);
        let left = settings.column_shares[divider];
        let right = settings.column_shares[divider + 1];
        let delta = delta_share.clamp(
            table::MIN_COLUMN_SHARE - left,
            right - table::MIN_COLUMN_SHARE,
        );
        if delta == 0.0 {
            return false;
        }
        settings.column_shares[divider] += delta;
        settings.column_shares[divider + 1] -= delta;
        self.set_table_settings(first, end, std::sync::Arc::new(settings));
        self.dirty = true;
        true
    }

    /// Drag the divider below row `divider`, transferring height to the row
    /// under it while retaining the table's total height.
    pub fn resize_table_row(&mut self, block: usize, divider: usize, delta: f32) -> bool {
        if !matches!(self.focus, Focus::Body) {
            return false;
        }
        let Some(range) = self.table_bounds(block) else {
            return false;
        };
        let (first, end, columns) = (range.first, range.end, range.columns);
        let rows = end - first;
        if divider + 1 >= rows || !delta.is_finite() {
            return false;
        }
        let mut settings = self.scope()[first]
            .table_settings()
            .expect("table bounds names a table")
            .clone()
            .normalized(columns, rows);
        let top = settings.row_heights[divider];
        let bottom = settings.row_heights[divider + 1];
        let delta = delta.clamp(table::MIN_ROW_HEIGHT - top, bottom - table::MIN_ROW_HEIGHT);
        if delta == 0.0 {
            return false;
        }
        settings.row_heights[divider] += delta;
        settings.row_heights[divider + 1] -= delta;
        self.set_table_settings(first, end, std::sync::Arc::new(settings));
        self.dirty = true;
        true
    }

    /// Insert a blank row immediately after `row` in the table containing
    /// `block`. The table card always acts on the row the reader clicked, so
    /// no secondary table selection state can go stale.
    pub fn insert_table_row(&mut self, block: usize, row: usize) -> bool {
        if !matches!(self.focus, Focus::Body) {
            return false;
        }
        let Some(range) = self.table_bounds(block) else {
            return false;
        };
        let (first, end, columns) = (range.first, range.end, range.columns);
        let rows = end - first;
        let index = row.min(rows.saturating_sub(1)) + 1;
        let mut settings = self.scope()[first]
            .table_settings()
            .expect("table bounds names a table")
            .clone()
            .normalized(columns, rows);
        settings
            .row_heights
            .insert(index, table::DEFAULT_ROW_HEIGHT);
        let settings = std::sync::Arc::new(settings);
        self.scope_mut().insert(
            first + index,
            table_row(columns, false, std::sync::Arc::clone(&settings)),
        );
        self.set_table_settings(first, end + 1, settings);
        if self.caret.block >= first + index {
            self.caret.block += 1;
        }
        self.dirty = true;
        true
    }

    /// Remove the selected row. A one-row table keeps its final row: a table
    /// is still a table with empty cells, while deleting its last row would
    /// silently turn a local layout block into unrelated prose.
    pub fn remove_table_row(&mut self, block: usize, row: usize) -> bool {
        if !matches!(self.focus, Focus::Body) {
            return false;
        }
        let Some(range) = self.table_bounds(block) else {
            return false;
        };
        let (first, end, columns) = (range.first, range.end, range.columns);
        let rows = end - first;
        if rows <= 1 {
            return false;
        }
        let index = row.min(rows - 1);
        let mut settings = self.scope()[first]
            .table_settings()
            .expect("table bounds names a table")
            .clone()
            .normalized(columns, rows);
        settings.row_heights.remove(index);
        self.scope_mut().remove(first + index);
        if index == 0 {
            // The next physical row becomes the Markdown table's anchor.
            // Without promoting it, serialisation would see a continuation
            // row without a table start and turn the surviving grid into
            // prose on disk.
            if let Block::TableRow { first: marker, .. } = &mut self.scope_mut()[first] {
                *marker = true;
            }
        }
        self.set_table_settings(first, end - 1, std::sync::Arc::new(settings));
        if self.caret.block > first + index {
            self.caret.block -= 1;
        } else if self.caret.block == first + index {
            let target = (first + index).min(end - 2);
            self.set_caret(target, self.caret.inline, self.caret.offset);
        }
        self.dirty = true;
        true
    }

    /// Insert a blank column immediately after `column`, splitting that
    /// column's share in two so the table remains full-width and does not
    /// visibly jump when its shape changes.
    pub fn insert_table_column(&mut self, block: usize, column: usize) -> bool {
        if !matches!(self.focus, Focus::Body) {
            return false;
        }
        let Some(range) = self.table_bounds(block) else {
            return false;
        };
        let (first, end, columns) = (range.first, range.end, range.columns);
        let rows = end - first;
        let previous = column.min(columns.saturating_sub(1));
        let index = previous + 1;
        let mut settings = self.scope()[first]
            .table_settings()
            .expect("table bounds names a table")
            .clone()
            .normalized(columns, rows);
        let share = settings.column_shares[previous] * 0.5;
        settings.column_shares[previous] -= share;
        settings.column_shares.insert(index, share);
        for row in &mut self.scope_mut()[first..end] {
            if let Some(cells) = row.cells_mut() {
                cells.insert(index, table::Cell::new());
            }
        }
        self.set_table_settings(first, end, std::sync::Arc::new(settings));
        if (first..end).contains(&self.caret.block) && self.caret.inline >= index {
            self.caret.inline += 1;
        }
        self.dirty = true;
        true
    }

    /// Remove the selected column, giving its width to a surviving neighbour
    /// and preserving the final column as the table's irreducible cell.
    pub fn remove_table_column(&mut self, block: usize, column: usize) -> bool {
        if !matches!(self.focus, Focus::Body) {
            return false;
        }
        let Some(range) = self.table_bounds(block) else {
            return false;
        };
        let (first, end, columns) = (range.first, range.end, range.columns);
        if columns <= 1 {
            return false;
        }
        let rows = end - first;
        let index = column.min(columns - 1);
        let mut settings = self.scope()[first]
            .table_settings()
            .expect("table bounds names a table")
            .clone()
            .normalized(columns, rows);
        let share = settings.column_shares.remove(index);
        let recipient = if index == 0 { 0 } else { index - 1 };
        settings.column_shares[recipient] += share;
        for row in &mut self.scope_mut()[first..end] {
            if let Some(cells) = row.cells_mut() {
                cells.remove(index);
            }
        }
        self.set_table_settings(first, end, std::sync::Arc::new(settings));
        if (first..end).contains(&self.caret.block) {
            if self.caret.inline > index {
                self.caret.inline -= 1;
            } else if self.caret.inline == index {
                self.caret.inline = index.min(columns - 2);
                self.caret.offset = self
                    .caret
                    .offset
                    .min(self.scope()[self.caret.block].cells()[self.caret.inline].flat_len());
            }
        }
        self.dirty = true;
        true
    }

    /// The extent of the table containing `block`: its first and last row
    /// indices, its row count, and its column count.
    pub fn table_bounds(&self, block: usize) -> Option<TableRange> {
        if !self.scope().get(block).is_some_and(Block::is_table) {
            return None;
        }
        let mut first = block;
        while first > 0
            && matches!(
                self.scope().get(first),
                Some(Block::TableRow { first: false, .. })
            )
        {
            first -= 1;
        }
        if !self.scope().get(first).is_some_and(Block::table_first) {
            return None;
        }
        let mut end = first + 1;
        while matches!(
            self.scope().get(end),
            Some(Block::TableRow { first: false, .. })
        ) {
            end += 1;
        }
        Some(TableRange {
            first,
            end,
            rows: end - first,
            columns: self.scope()[first].cells().len(),
        })
    }

    pub(crate) fn set_table_settings(
        &mut self,
        first: usize,
        end: usize,
        settings: std::sync::Arc<table::TableSettings>,
    ) {
        for row in &mut self.scope_mut()[first..end] {
            if let Block::TableRow {
                settings: row_settings,
                ..
            } = row
            {
                *row_settings = std::sync::Arc::clone(&settings);
            }
        }
    }
}

impl Document {
    /// One table row as the GFM line `markdown::serialize` would write:
    /// `| a | b |`. Each cell is its lines joined with `<br>`, a literal `|`
    /// escaped, math written `$…$` and styles carrying their Markdown
    /// markers, so a copied row parses back into the same cells.
    pub(crate) fn table_row_markdown(&self, block: usize) -> String {
        let Some(row) = self.scope().get(block).filter(|row| row.is_table()) else {
            return String::new();
        };
        let mut out = String::from("|");
        for cell in row.cells() {
            out.push(' ');
            out.push_str(&cell_markdown(cell));
            out.push_str(" |");
        }
        out
    }

    /// The Markdown divider row of the table whose header is `block` —
    /// `| --- | --- |` — carrying its per-column alignment. A copy of a
    /// header row together with the rows under it needs it to read back as a
    /// table rather than a stack of unrelated pipe rows.
    pub(crate) fn table_row_divider(&self, block: usize) -> String {
        let Some(settings) = self.scope().get(block).and_then(Block::table_settings) else {
            return String::new();
        };
        let mut out = String::from("|");
        for align in &settings.align {
            out.push(' ');
            out.push_str(match align {
                table::ColumnAlign::None => "---",
                table::ColumnAlign::Left => ":---",
                table::ColumnAlign::Centre => ":---:",
                table::ColumnAlign::Right => "---:",
            });
            out.push_str(" |");
        }
        out
    }

    /// `(row, cell, from, to)` for every cell a range covers, named in that
    /// cell's own flat space, when both ends of the range lie inside one and
    /// the same table. `None` otherwise, so a range that mixes a table with
    /// prose keeps the block-wise behaviour and never splices prose into a
    /// row.
    pub(crate) fn table_range_cells(
        &self,
        start: FlatPos,
        end: FlatPos,
    ) -> Option<Vec<(usize, usize, usize, usize)>> {
        if (start.block, start.offset) >= (end.block, end.offset) {
            return None;
        }
        let first = self.table_bounds(start.block)?;
        let last = self.table_bounds(end.block)?;
        if first.first != last.first || first.end != last.end {
            return None;
        }
        let mut cells = Vec::new();
        for row in start.block..=end.block {
            let from = if row == start.block { start.offset } else { 0 };
            let to = if row == end.block {
                end.offset
            } else {
                self.block_len(row)
            };
            cells.extend(
                self.table_cells_in_range(row, from, to)
                    .into_iter()
                    .map(|(cell, from, to)| (row, cell, from, to)),
            );
        }
        Some(cells)
    }

    /// The runs a cell-flat `[from, to)` window covers, line by line — a cell
    /// is a container, so a range reaching into it maps through its lines
    /// rather than assuming one.
    pub(crate) fn cell_slice_runs(
        &self,
        row: usize,
        cell: usize,
        from: usize,
        to: usize,
    ) -> Vec<Inline> {
        let mut out = Vec::new();
        let mut line_start = 0;
        for line in self.scope()[row].cells()[cell].lines() {
            let len: usize = line.iter().map(flat_len).sum();
            let start = from.saturating_sub(line_start).min(len);
            let end = to.saturating_sub(line_start).min(len);
            if start < end {
                out.extend(slice_inline_runs(line, start, end));
            }
            line_start += len + 1;
        }
        out
    }

    /// Applies `f` to every run a cell-flat `[from, to)` window covers, line
    /// by line, rebuilding only the lines it reaches. The mutating mirror of
    /// [`Self::cell_slice_runs`].
    pub(crate) fn style_cell_slice(
        cell: &mut table::Cell,
        from: usize,
        to: usize,
        mut f: impl FnMut(&mut Inline),
    ) {
        if from >= to {
            return;
        }
        let mut line_start = 0;
        for line in cell.lines_mut() {
            let len: usize = line.iter().map(flat_len).sum();
            let start = from.saturating_sub(line_start).min(len);
            let end = to.saturating_sub(line_start).min(len);
            if start < end {
                let mut rebuilt = slice_inline_runs(line, 0, start);
                let mut selected = slice_inline_runs(line, start, end);
                for run in &mut selected {
                    f(run);
                }
                rebuilt.extend(selected);
                rebuilt.extend(slice_inline_runs(line, end, len));
                *line = rebuilt;
            }
            line_start += len + 1;
        }
    }

    /// Removes the runs a cell-flat `[from, to)` window covers, line by
    /// line, folding the covered lines into one when the window spans a line
    /// break. The removal mirror of [`Self::style_cell_slice`].
    pub(crate) fn clear_cell_slice(cell: &mut table::Cell, from: usize, to: usize) {
        if from >= to {
            return;
        }
        let (start_line, start_offset) = cell.position(from);
        let (end_line, end_offset) = cell.position(to);
        if start_line == end_line {
            let runs = cell.lines()[start_line].clone();
            let len: usize = runs.iter().map(flat_len).sum();
            let mut rebuilt = slice_inline_runs(&runs, 0, start_offset);
            rebuilt.extend(slice_inline_runs(&runs, end_offset, len));
            cell.lines_mut()[start_line] = placeholder_if_empty(rebuilt);
            return;
        }
        let first = cell.lines()[start_line].clone();
        let last = cell.lines()[end_line].clone();
        let last_len: usize = last.iter().map(flat_len).sum();
        let mut joined = slice_inline_runs(&first, 0, start_offset);
        joined.extend(slice_inline_runs(&last, end_offset, last_len));
        let joined = placeholder_if_empty(joined);
        cell.lines_mut()[start_line] = joined;
        cell.lines_mut().drain(start_line + 1..=end_line);
    }

    /// The text of the cell the caret sits in, if it is in a table. Read
    /// before a cell-clearing edit so the yank register can carry what the
    /// edit removed.
    pub fn caret_cell_text(&self) -> Option<String> {
        if !self.in_table() {
            return None;
        }
        let block = self.caret.block.min(self.scope().len().saturating_sub(1));
        let cells = self.scope().get(block)?.cells();
        if cells.is_empty() {
            return None;
        }
        let cell = self.caret.inline.min(cells.len() - 1);
        Some(super::cell_text(&cells[cell]))
    }
}

/// One cell as its on-disk Markdown: lines joined with `<br>`, a literal
/// `<br` escaped, a literal `|` escaped exactly as
/// `markdown::serialize_table_cell` writes it.
fn cell_markdown(cell: &table::Cell) -> String {
    cell.lines()
        .iter()
        .map(|line| runs_markdown(line).replace("<br", "\\<br"))
        .collect::<Vec<_>>()
        .join("<br>")
        .replace('|', "\\|")
}

/// A line's runs in Markdown: text escaped then wrapped in its style markers,
/// math as `$…$`, and one shared `==…==` over a stretch of marked runs.
fn runs_markdown(runs: &[Inline]) -> String {
    let one = |run: &Inline, style: Style| match run {
        Inline::Text(t) => {
            if style.is_boxed() {
                wrap_markdown(style, &t.text)
            } else {
                wrap_markdown(style, &escape_markdown(&t.text))
            }
        }
        Inline::Math(list) => format!("${}$", math_notation::print(list)),
        Inline::Note(label) => format!("[^{label}]"),
        Inline::EqRef(label) => format!("@{label}"),
    };
    let mut out = String::new();
    let mut i = 0;
    while i < runs.len() {
        if !runs[i].style().highlight {
            out.push_str(&one(&runs[i], runs[i].style()));
            i += 1;
            continue;
        }
        let mut end = i;
        while runs.get(end + 1).is_some_and(|run| run.style().highlight) {
            end += 1;
        }
        out.push_str("==");
        for run in &runs[i..=end] {
            out.push_str(&one(
                run,
                Style {
                    highlight: false,
                    ..run.style()
                },
            ));
        }
        out.push_str("==");
        i = end + 1;
    }
    out
}

/// Wrap one run's text in the marker for its style, the same spellings
/// `markdown::parse` reads back.
fn wrap_markdown(style: Style, escaped: &str) -> String {
    if style.code {
        format!("`{escaped}`")
    } else if style.badge {
        match style.badge_color {
            BadgeColor::Orange => format!("[[{escaped}]]"),
            BadgeColor::Blue => format!("[[blue|{escaped}]]"),
            BadgeColor::Green => format!("[[green|{escaped}]]"),
            BadgeColor::Purple => format!("[[purple|{escaped}]]"),
        }
    } else if style.bold && style.italic {
        format!("***{escaped}***")
    } else if style.bold {
        format!("**{escaped}**")
    } else if style.italic {
        format!("*{escaped}*")
    } else {
        escaped.to_string()
    }
}

/// Escape the characters `markdown::parse` would read as notation, matching
/// `markdown::escape_run_text`.
fn escape_markdown(text: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        match c {
            '\\' => out.push_str("\\\\"),
            '*' => out.push_str("\\*"),
            '`' => out.push_str("\\`"),
            '$' => out.push_str("\\$"),
            '=' | '[' if chars.get(i + 1) == Some(&c) => {
                out.push('\\');
                out.push(c);
            }
            '[' if chars.get(i + 1) == Some(&'^') => out.push_str("\\["),
            c => out.push(c),
        }
    }
    out
}

impl Document {
    /// Remove the whole table containing `block`, and hand its Markdown back so
    /// the register keeps what the grid held — a delete that destroys a table
    /// has to be recoverable, and the rows are what a paste needs to rebuild
    /// it. Refuses (`None`) when `block` is not a table row. A document emptied
    /// by the removal keeps one empty paragraph, like any other block delete.
    pub fn delete_table(&mut self, block: usize) -> Option<String> {
        let range = self.table_bounds(block)?;
        let mut markdown = String::new();
        for row in range.first..range.end {
            if row > range.first {
                markdown.push('\n');
            }
            markdown.push_str(&self.table_row_markdown(row));
            if row == range.first {
                markdown.push('\n');
                markdown.push_str(&self.table_row_divider(row));
            }
        }
        self.scope_mut().drain(range.first..range.end);
        if self.scope().is_empty() {
            self.scope_mut().push(super::empty_block());
        }
        self.caret.block = range.first.min(self.scope().len() - 1);
        self.caret.inline = 0;
        self.caret.offset = 0;
        self.caret.style = Style::PLAIN;
        self.math = None;
        self.dirty = true;
        self.enforce();
        Some(markdown)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::document::layout;
    use crate::document::markdown::{parse, serialize};
    use crate::document::{Block, Document, Inline, ListMarker, cell_text};
    use crate::theme::TextStyle;

    /// Deterministic text width: half the font size per character.
    fn measure(text: &str, style: &TextStyle) -> f32 {
        text.chars().count() as f32 * style.size * 0.5
    }

    fn table() -> Document {
        let mut document = Document::new(Path::new("table.md"));
        document.insert_table();
        document
    }

    #[test]
    fn enter_splits_a_cell_line_and_leaves_the_caret_on_it() {
        let mut document = table();
        document.insert_text("alpha");
        document.set_caret(0, 0, 2);
        assert!(document.split_cell_line());
        assert!(document.body().iter().all(Block::is_table));
        assert_eq!(document.body().len(), 2, "Enter never adds a row");
        let cell = &document.body()[0].cells()[0];
        assert_eq!(cell.lines().len(), 2);
        assert_eq!(cell_text(cell), "al\npha");
        assert_eq!(
            (
                document.caret.block,
                document.caret.inline,
                document.caret.offset
            ),
            (0, 0, 3),
            "the caret lands on the new line"
        );
    }

    #[test]
    fn backspace_at_a_cell_line_start_joins_it_up() {
        let mut document = table();
        document.insert_text("alpha");
        document.set_caret(0, 0, 2);
        assert!(document.split_cell_line());
        assert!(document.join_cell_line());
        let cell = &document.body()[0].cells()[0];
        assert_eq!(cell.lines().len(), 1);
        assert_eq!(cell_text(cell), "alpha");
        assert_eq!(document.caret.offset, 2);
    }

    #[test]
    fn backspace_on_a_cell_first_line_joins_nothing() {
        let mut document = table();
        document.insert_text("alpha");
        document.set_caret(0, 0, 0);
        assert!(!document.join_cell_line());
        assert_eq!(cell_text(&document.body()[0].cells()[0]), "alpha");
    }

    #[test]
    fn delete_at_a_cell_line_end_joins_the_next_line_down() {
        let mut document = table();
        document.insert_text("alpha");
        document.set_caret(0, 0, 2);
        assert!(document.split_cell_line());
        document.set_caret(0, 0, 2);
        assert!(document.join_cell_line_forward());
        let cell = &document.body()[0].cells()[0];
        assert_eq!(cell.lines().len(), 1);
        assert_eq!(cell_text(cell), "alpha");
        assert_eq!(document.caret.offset, 2);
    }

    #[test]
    fn backspace_and_delete_in_a_two_line_cell_keep_the_grid() {
        let mut document = table();
        document.insert_text("one");
        document.set_caret(0, 0, 3);
        document.newline();
        document.insert_text("two");
        assert_eq!(cell_text(&document.body()[0].cells()[0]), "one\ntwo");
        // Backspace at the very start of the second line folds it away.
        document.set_caret(0, 0, 4);
        document.backspace();
        assert_eq!(cell_text(&document.body()[0].cells()[0]), "onetwo");
        // Enter again, then Delete at the end of the first line.
        document.set_caret(0, 0, 3);
        document.newline();
        assert_eq!(cell_text(&document.body()[0].cells()[0]), "one\ntwo");
        document.set_caret(0, 0, 3);
        document.delete_forward();
        assert_eq!(cell_text(&document.body()[0].cells()[0]), "onetwo");
        assert!(document.body().iter().all(Block::is_table));
        assert_eq!(document.body()[0].cells().len(), 2);
    }

    #[test]
    fn column_alignment_survives_a_save() {
        let path = Path::new("table.md");
        let text = "| A | B | C | D |\n| :--- | ---: | :---: | --- |\n| a | b | c | d |\n\
                    <!-- typewritter-table v1 lines=111111 cols=0.250000,0.250000,0.250000,0.250000 rows=38.00,38.00 -->\n";
        let document = parse(path, text);
        assert_eq!(
            document.body()[0].table_settings().unwrap().align,
            vec![
                crate::document::table::ColumnAlign::Left,
                crate::document::table::ColumnAlign::Right,
                crate::document::table::ColumnAlign::Centre,
                crate::document::table::ColumnAlign::None,
            ]
        );
        assert_eq!(
            serialize(&document),
            text,
            "alignment is not silently dropped"
        );
        assert_eq!(parse(path, &serialize(&document)).body(), document.body());
    }

    #[test]
    fn a_two_line_cell_round_trips_through_disk() {
        let path = Path::new("table.md");
        let text = "| A | B |\n| --- | --- |\n| one<br>two | x |\n\
                    <!-- typewritter-table v1 lines=111111 cols=0.500000,0.500000 rows=38.00,38.00 -->\n";
        let document = parse(path, text);
        assert_eq!(document.body().len(), 2);
        let cell = &document.body()[1].cells()[0];
        assert_eq!(cell.lines().len(), 2);
        assert_eq!(cell_text(cell), "one\ntwo");
        assert_eq!(serialize(&document), text);
        assert_eq!(parse(path, &serialize(&document)).body(), document.body());
    }

    #[test]
    fn a_two_line_cell_round_trips_with_a_pipe_and_a_literal_break() {
        let path = Path::new("table.md");
        let mut document = table();
        document.insert_text("a|b");
        document.set_caret(0, 0, 3);
        document.split_cell_line();
        document.insert_text("lit <br> here");
        let text = serialize(&document);
        assert!(
            text.contains("| a\\|b<br>lit \\<br> here |"),
            "the row stays one GFM line: {text}"
        );
        let back = parse(path, &text);
        let cell = &back.body()[0].cells()[0];
        assert_eq!(cell.lines().len(), 2);
        assert_eq!(cell_text(cell), "a|b\nlit <br> here");
        assert_eq!(serialize(&back), text, "the escaping is a fixpoint");
    }

    #[test]
    fn a_two_line_cell_lays_out_both_lines() {
        let mut document = table();
        document.insert_text("one");
        document.set_caret(0, 0, 3);
        document.newline();
        document.insert_text("two");
        let laid = layout::layout(&document, 400.0, &measure);
        let table = laid.tables[0].as_ref().expect("a table layout");
        let lines = &table.cells[0];
        assert_eq!(lines.len(), 2, "each cell line lays out");
        assert_eq!(lines[0].cell_line, 0);
        assert_eq!(lines[1].cell_line, 1);
        assert_eq!(
            lines[1].y, lines[0].height,
            "the second line sits below the first"
        );
        assert!(table.row_height >= lines[0].height + lines[1].height);
        assert_eq!(
            lines[0]
                .segments
                .iter()
                .map(|segment| segment.len)
                .sum::<usize>(),
            3,
            "the first line carries its own run, not the whole cell"
        );
    }

    #[test]
    fn a_display_math_cell_line_leads_and_centres() {
        let mut document = table();
        document.insert_inline_math();
        document.math_insert_char('x');
        let laid = layout::layout(&document, 400.0, &measure);
        let table = laid.tables[0].as_ref().expect("a table layout");
        let math_line = &table.cells[0][0];
        let text_height = table.cells[1][0].height;
        assert!(
            math_line.height > text_height,
            "a display cell line leads: {} vs {text_height}",
            math_line.height
        );
        assert!(math_line.x > 0.0, "the atom centres in its column");
        assert!(matches!(
            document.body()[0].cells()[0].lines()[0].as_slice(),
            [Inline::Math(_)]
        ));
    }

    #[test]
    fn a_display_math_cell_line_round_trips() {
        let path = Path::new("table.md");
        let text = "| A | B |\n| --- | --- |\n| $x$<br>second | y |\n\
                    <!-- typewritter-table v1 lines=111111 cols=0.500000,0.500000 rows=38.00,38.00 -->\n";
        let document = parse(path, text);
        let cell = &document.body()[1].cells()[0];
        assert_eq!(cell.lines().len(), 2);
        assert!(matches!(cell.lines()[0].as_slice(), [Inline::Math(_)]));
        assert!(
            matches!(&cell.lines()[1][0], Inline::Text(t) if t.text == "second"),
            "the second line reads back as text"
        );
        let once = serialize(&document);
        assert!(once.contains("$x$<br>second"), "{once}");
        assert_eq!(serialize(&parse(path, &once)), once);
    }

    #[test]
    fn a_cell_line_break_survives_a_typed_escape_and_a_round_trip() {
        let path = Path::new("table.md");
        let mut document = table();
        document.insert_text("x");
        document.split_cell_line();
        document.insert_notation("$a$");
        let text = serialize(&document);
        let back = parse(path, &text);
        let cell = &back.body()[0].cells()[0];
        assert_eq!(cell.lines().len(), 2);
        assert!(cell.lines()[0].iter().all(|run| !run.text().is_empty()));
        assert_eq!(serialize(&back), text);
    }

    /// Every run vector of the document, named so a change can be located:
    /// a block's own runs, or each cell line's.
    fn run_pointers(document: &Document) -> Vec<(String, *const Inline)> {
        let mut pointers = Vec::new();
        for (index, block) in document.body().iter().enumerate() {
            match block {
                Block::TableRow { cells, .. } => {
                    for (column, cell) in cells.iter().enumerate() {
                        for (line, runs) in cell.lines().iter().enumerate() {
                            pointers.push((format!("{index}.{column}.{line}"), runs.as_ptr()));
                        }
                    }
                }
                block => pointers.push((index.to_string(), block.inlines().as_ptr())),
            }
        }
        pointers
    }

    #[test]
    fn editing_a_cell_leaves_every_other_block_untouched() {
        let mut document = table();
        document.insert_text("seed");
        document.set_caret(0, 1, 0);
        document.insert_text("second");
        // A paragraph and an ordered list after the table: a document-wide
        // prune rebuilds both of their run vectors.
        document.open_below();
        document.insert_text("prose");
        document.open_below();
        document.set_list(Some(ListMarker::Number(1)));
        document.insert_text("one");
        document.set_caret(0, 1, 0);

        let before = run_pointers(&document);
        document.insert_text("!");
        let after = run_pointers(&document);

        assert_eq!(cell_text(&document.body()[0].cells()[1]), "!second");
        assert_eq!(before.len(), after.len(), "the shape did not change");
        for ((key, before), (_, after)) in before.iter().zip(&after) {
            if key == "0.1.0" {
                continue;
            }
            assert_eq!(
                before, after,
                "block {key} was rebuilt by the edit, not just the touched cell"
            );
        }

        // That guard only means something if a document-wide prune does move
        // those pointers: prove the comparison can see one.
        let mut probe = document.clone();
        probe.enforce();
        let after_all = run_pointers(&probe);
        assert!(
            before.iter().zip(&after_all).any(|((_, a), (_, b))| a != b),
            "a document-wide prune must be detectable, or this test is silent"
        );
    }
}

#[cfg(test)]
mod clip_tests {
    use std::path::Path;

    use crate::document::markdown::parse;
    use crate::document::{Block, Document, FlatRange, Style, cell_text};

    fn path() -> &'static Path {
        Path::new("table.md")
    }

    /// Copy a whole-block range with the operator path the shell uses.
    fn copy(document: &Document, first: usize, last: usize) -> String {
        document.range_text(document.line_range(first, last))
    }

    fn table_document() -> Document {
        let mut document = Document::new(path());
        document.insert_table();
        document
    }

    #[test]
    fn a_newline_pasted_into_a_cell_becomes_two_cell_lines() {
        let mut document = table_document();
        document.insert_text("a\nb");
        assert_eq!(document.body().len(), 2, "the row is not duplicated");
        assert!(document.body().iter().all(Block::is_table));
        let cell = &document.body()[0].cells()[0];
        assert_eq!(cell.lines().len(), 2);
        assert_eq!(cell_text(cell), "a\nb");
    }

    #[test]
    fn a_newline_outside_a_table_keeps_its_meaning() {
        let mut document = Document::new(path());
        document.insert_text("a\nb");
        assert_eq!(document.body().len(), 1, "a soft wrap stays one block");
        assert_eq!(document.block_text(0), "a\nb");
    }

    #[test]
    fn a_copied_header_row_is_a_gfm_line_that_reparses_to_the_same_cells() {
        let document = parse(path(), "| **A** | $x$ |\n| --- | --- |\n| a | b |\n");
        let copied = copy(&document, 0, 0);
        assert_eq!(copied, "| **A** | $x$ |");
        let back = parse(path(), &format!("{copied}\n| --- | --- |"));
        assert_eq!(back.body()[0].cells(), document.body()[0].cells());
    }

    #[test]
    fn a_copied_whole_table_reparses_to_the_same_table() {
        let text = "| **A** | $x$ |\n| :--- | ---: |\n| a\\|b<br>c | d |\n";
        let document = parse(path(), text);
        let copied = copy(&document, 0, 1);
        assert_eq!(
            copied,
            "| **A** | $x$ |\n| :--- | ---: |\n| a\\|b<br>c | d |"
        );
        let back = parse(path(), &copied);
        assert_eq!(back.body(), document.body());
    }

    /// Whether every run of a cell carries bold.
    fn bold_of(document: &Document, row: usize, cell: usize) -> bool {
        document.body()[row].cells()[cell]
            .lines()
            .iter()
            .flat_map(|line| line.iter())
            .all(|run| run.style().bold)
    }

    fn fill(document: &mut Document, row: usize, cell: usize, text: &str) {
        document.set_caret(row, cell, 0);
        document.insert_text(text);
    }

    fn bold() -> Style {
        Style {
            bold: true,
            ..Style::PLAIN
        }
    }

    #[test]
    fn a_copied_row_carries_boxed_styles_back_to_the_same_cells() {
        let document = parse(path(), "| `c` | [[B]] |\n| --- | --- |\n| x | y |\n");
        let copied = copy(&document, 0, 0);
        assert_eq!(copied, "| `c` | [[B]] |");
        let back = parse(path(), &format!("{copied}\n| --- | --- |"));
        assert_eq!(back.body()[0].cells(), document.body()[0].cells());
    }

    #[test]
    fn a_style_range_across_two_cells_of_a_row_styles_both() {
        let mut document = table_document();
        fill(&mut document, 0, 0, "a");
        fill(&mut document, 0, 1, "b");
        let range = FlatRange::new(
            document.position(0, 0),
            document.position(0, document.block_len(0)),
        );
        document.toggle_style_range(range, bold());
        assert!(bold_of(&document, 0, 0), "the first cell is styled");
        assert!(bold_of(&document, 0, 1), "so is the second");
    }

    #[test]
    fn a_cross_row_style_range_styles_every_covered_cell() {
        let mut document = table_document();
        fill(&mut document, 0, 0, "a");
        fill(&mut document, 0, 1, "b");
        fill(&mut document, 1, 0, "c");
        fill(&mut document, 1, 1, "d");
        let range = FlatRange::new(
            document.position(0, 0),
            document.position(1, document.block_len(1)),
        );
        document.toggle_style_range(range, bold());
        for (row, cell) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
            assert!(bold_of(&document, row, cell), "cell {row}.{cell} is styled");
        }
    }

    #[test]
    fn a_style_range_spanning_a_table_and_prose_leaves_the_grid_intact() {
        let mut document = parse(path(), "| A | B |\n| --- | --- |\n| a | b |\n\nafter\n");
        document.set_caret(0, 0, 0);
        let range = FlatRange::new(
            document.position(0, 0),
            document.position(2, document.block_len(2)),
        );
        document.toggle_style_range(range, bold());
        let body = document.body();
        assert!(
            body[0].is_table() && body[1].is_table(),
            "the grid survives"
        );
        assert_eq!(body[0].cells().len(), 2);
        assert_eq!(cell_text(&body[1].cells()[0]), "a");
        assert!(!bold_of(&document, 1, 0), "the table part is left alone");
        let paragraph = body[2].inlines();
        assert!(
            paragraph.iter().all(|run| run.style().bold),
            "the prose part is styled"
        );
    }
}

/// Removing a table — whole, by rows, or through a selection that reaches into
/// one. A table row is a line like any other, so nothing about a table may make
/// a delete silently do less than it says.
#[cfg(test)]
mod delete_tests {
    use std::path::Path;

    use crate::document::markdown::{parse, serialize};
    use crate::document::{Block, Document, FlatRange, cell_text};

    fn path() -> &'static Path {
        Path::new("table.md")
    }

    /// The table's first row block. Derived, never counted by hand: the header
    /// is a block and the divider row is not.
    fn first_row(d: &Document) -> usize {
        d.body()
            .iter()
            .position(Block::is_table)
            .expect("the document holds a table")
    }

    fn last_row(d: &Document) -> usize {
        first_row(d) + d.body().iter().filter(|block| block.is_table()).count() - 1
    }

    fn rows(d: &Document) -> Vec<String> {
        d.body()
            .iter()
            .filter(|block| block.is_table())
            .map(|row| {
                row.cells()
                    .iter()
                    .map(cell_text)
                    .collect::<Vec<_>>()
                    .join("|")
            })
            .collect()
    }

    /// A math cell and an empty one, beside plain text: the shapes a reader
    /// actually writes.
    fn document() -> Document {
        parse(path(), "| A | B |\n| --- | --- |\n| $x$ | two |\n| a | |\n")
    }

    #[test]
    fn deleting_a_table_removes_every_row_and_keeps_its_markdown() {
        let mut d = document();
        let first = first_row(&d);
        let markdown = d.delete_table(first).expect("the caret's table goes");
        assert!(
            d.body().iter().all(|block| !block.is_table()),
            "no rows survive"
        );
        assert_eq!(d.body().len(), 1, "one empty paragraph holds the place");
        assert_eq!(d.block_len(0), 0);
        // What the register keeps rebuilds the table it destroyed.
        assert!(
            markdown.contains("| $x$ | two |"),
            "math as notation: {markdown}"
        );
        assert!(markdown.contains("| --- | --- |"), "divider: {markdown}");
        let again = parse(path(), &format!("{markdown}\n"));
        assert!(again.body().iter().all(Block::is_table));
        assert_eq!(rows(&again).len(), 3, "header and two data rows");
    }

    #[test]
    fn deleting_a_table_from_a_document_with_neighbours_leaves_them() {
        let mut d = parse(
            path(),
            "before\n\n| A | B |\n| --- | --- |\n| a | b |\n\nafter\n",
        );
        let first = first_row(&d);
        let markdown = d.delete_table(first).expect("the table goes");
        assert!(markdown.contains("| A | B |"), "header in the register");
        assert_eq!(d.body().len(), 2);
        assert_eq!(d.block_text(0), "before");
        assert_eq!(d.block_text(1), "after");
        assert_eq!(d.caret.block, first, "caret lands on the table's place");
    }

    #[test]
    fn a_line_wise_delete_over_a_whole_table_removes_it() {
        let mut d = parse(
            path(),
            "before\n\n| A | B |\n| --- | --- |\n| a | b |\n| c | d |\n\nafter\n",
        );
        let (first, last) = (first_row(&d), last_row(&d));
        // The `V`-over-a-table case that used to delete nothing at all.
        let deleted = d
            .delete_lines(first, last)
            .expect("the selection is not empty");
        assert!(
            deleted.contains("| A | B |"),
            "rows yank as Markdown: {deleted}"
        );
        assert!(d.body().iter().all(|block| !block.is_table()));
        assert_eq!(d.body().len(), 2, "the prose closes up over the table");
        assert_eq!(d.block_text(0), "before");
        assert_eq!(d.block_text(1), "after");
    }

    #[test]
    fn a_line_wise_delete_of_a_header_row_promotes_the_row_below_it() {
        let mut d = document();
        let first = first_row(&d);
        d.delete_lines(first, first).expect("the header goes");
        assert_eq!(rows(&d).len(), 2, "two data rows survive");
        assert!(d.body()[first].table_first(), "the next row anchors");
        let saved = serialize(&d);
        assert!(
            saved.starts_with("| $x$ | two |\n| --- | --- |"),
            "still a table on disk: {saved}"
        );
        assert_eq!(parse(path(), &saved).body(), d.body(), "and it reads back");
    }

    #[test]
    fn a_line_wise_delete_over_a_row_and_prose_removes_both() {
        let mut d = parse(
            path(),
            "before\n\n| A | B |\n| --- | --- |\n| a | b |\n\nafter\n",
        );
        let last = last_row(&d);
        // The last data row plus the paragraph under it.
        let deleted = d.delete_lines(last, last + 1).expect("not empty");
        assert!(deleted.contains("| a | b |"), "the row is in the register");
        assert!(deleted.ends_with("after"), "and the prose: {deleted}");
        assert_eq!(rows(&d).len(), 1, "the header row is left");
        assert_eq!(d.body().len(), 2, "before and the surviving header");
    }

    #[test]
    fn a_characterwise_delete_spanning_a_table_and_prose_clears_both() {
        let mut d = parse(path(), "| A | B |\n| --- | --- |\n| a | b |\n\nafter\n");
        let last = last_row(&d);
        // From the start of the last row's first cell into the prose below: the
        // range that used to be refused, deleting nothing anywhere.
        let range = FlatRange::new(d.position(last, 0), d.position(last + 1, 3));
        let deleted = d.delete_range(range).expect("the range is not empty");
        assert!(deleted.contains('a'), "the row's text is in the register");
        assert!(deleted.contains("aft"), "and the prose: {deleted}");
        assert!(
            d.body()[0].is_table() && d.body()[last].is_table(),
            "the grid survives"
        );
        // The range runs on into the prose, so the rest of the row lies inside
        // it: both cells go, and only the grid survives.
        assert_eq!(cell_text(&d.body()[last].cells()[0]), "");
        assert_eq!(cell_text(&d.body()[last].cells()[1]), "");
        assert_eq!(d.block_text(last + 1), "er", "the prose keeps its tail");
    }

    #[test]
    fn a_characterwise_delete_inside_one_row_still_keeps_the_grid() {
        let mut d = document();
        let last = last_row(&d);
        let range = FlatRange::new(d.position(last, 0), d.position(last, 2));
        d.delete_range(range).expect("the range is not empty");
        assert_eq!(d.body()[last].cells().len(), 2, "the cells stay");
        assert_eq!(cell_text(&d.body()[last].cells()[0]), "");
    }
}
