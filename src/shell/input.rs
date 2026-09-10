//! What the keyboard and the mouse do to the shell.
//!
//! Input modes, in priority order: the palette owns the keyboard while it's
//! open, then a dialog, then onboarding's vault path, then the command
//! table (every chord in [`commands`]), then the vim keymap. The keymap
//! stays live with no file open too — the leader still has to reach the
//! palette from there — only the parts of it that need an actual buffer
//! (scrolling, arrow keys, click-to-place) are guarded on a tab being open.
//! Only one of them ever runs in a frame.

use winit::event::MouseButton;
use winit::keyboard::KeyCode;

use super::commands;
use crate::components::dialog::{self, Prompt};
use crate::components::palette;
use crate::components::table_lines;
use crate::components::{
    ContextMenu, Dialog, Editor, FileTree, Finder, FormatBar, MathMenu, Onboarding, Palette,
    SlashMenu, SymbolMenu, TabStrip, Topics, context_menu, editor, file_tree, format_bar,
    math_menu, onboarding, scrollbar, sidenotes, symbol_menu, theme_switch, title_bar,
};
use crate::config::Config;
use crate::document::layout::{ContextHit, DocLayout, RangeKind, TableResize};
use crate::document::math::{self, AccentKind, BigOp, MathNode, NodeAddress, Slot, SymbolRole};
use crate::document::math_style::{self, HighlightShape, MathHue};
use crate::document::math_symbols;
use crate::document::{
    BadgeColor, Block, Caret, Document, FlatPos, FlatRange, Focus, Inline, ListMarker, Style,
    TableDirection, math_conversion, math_layout,
};
use crate::input::Input;
use crate::layout::Rect;
use crate::tabs::Tabs;
use crate::theme::{self, TextStyle, Theme};
use crate::vault::Vault;
use crate::vim::{
    Edit, ExtendedAction, Key, Mode, Motion, Operator, OperatorTarget, Vim, VimMode, VisualAction,
    VisualMode, motion,
};

use std::cell::RefCell;
use std::rc::Rc;

use super::{
    ContextGhost, ContextMenuState, InsertEvent, InsertMarker, InsertShortcuts, MathMenuState,
    MenuDismiss, PaletteState, Shell, SlashMenuState, TableDrag, TableLinesState, WordFormatState,
};

/// Apply one recorded Insert-mode event. Text and editing keys are recorded
/// alongside structural gestures so `.` repeats the document operation that
/// was actually performed, rather than spelling its hidden marker into
/// prose.
fn apply_insert_event(docs: &mut Tabs, event: &InsertEvent) {
    match event {
        InsertEvent::Text(c) => docs.type_text(&c.to_string()),
        InsertEvent::Backspace => docs.backspace(),
        InsertEvent::Delete => docs.delete_forward(),
        InsertEvent::Enter => docs.newline(),
        InsertEvent::SetHeading(level) => docs.set_heading(Some(*level)),
        InsertEvent::SetEmphasis { bold, italic } => {
            docs.touch(|doc| doc.set_emphasis(*bold, *italic));
        }
        InsertEvent::ToggleInlineCode => docs.toggle_code(),
        InsertEvent::ToggleBadge => docs.toggle_badge(),
        InsertEvent::ToggleHighlight => docs.toggle_highlight(),
        InsertEvent::BulletList => docs.set_list(Some(ListMarker::Bullet)),
        InsertEvent::NumberedList => docs.set_list(Some(ListMarker::Number(1))),
        InsertEvent::TaskList => docs.set_list(Some(ListMarker::Task { done: false })),
        InsertEvent::InsertDivider => docs.insert_divider(),
        InsertEvent::InsertSidenote => docs.insert_sidenote(),
        InsertEvent::CodeBlock => docs.set_code(true),
    }
}

/// Apply and record a structural Insert gesture.
fn insert_event(docs: &mut Tabs, repeat: &mut Vec<InsertEvent>, event: InsertEvent) {
    apply_insert_event(docs, &event);
    repeat.push(event);
}

/// Forget the typed marker text the current structural gesture replaces.
///
/// The candidate only exists after this Insert session inserted the text, so
/// its matching [`InsertEvent::Text`] is always the tail of `repeat`.
fn erase_marker_text(docs: &mut Tabs, repeat: &mut Vec<InsertEvent>, count: usize) {
    for _ in 0..count {
        docs.backspace();
        if matches!(repeat.last(), Some(InsertEvent::Text(_))) {
            repeat.pop();
        }
    }
}

fn empty_body_block(docs: &Tabs) -> bool {
    docs.active().is_some_and(|tab| {
        matches!(tab.document.focus, Focus::Body)
            && !tab.document.scope()[tab.document.caret.block].is_code()
            && !tab.document.in_table()
            && tab.document.block_len(tab.document.caret.block) == 0
    })
}

fn emphasis_allowed(docs: &Tabs) -> bool {
    docs.active()
        .is_some_and(|tab| !tab.document.caret.style.is_boxed())
}

fn bracket_allowed(docs: &Tabs) -> bool {
    docs.active().is_some_and(|tab| {
        !tab.document.caret.style.is_boxed()
            && !tab.document.scope()[tab.document.caret.block].is_code()
    })
}

/// How many characters form the hidden task marker at the cursor, if it is
/// the only content of a bullet item apart from optional Markdown spacing.
fn task_marker_len(docs: &Tabs) -> Option<usize> {
    let tab = docs.active()?;
    if !matches!(tab.document.focus, Focus::Body) {
        return None;
    }
    let block = tab.document.caret.block;
    if !matches!(
        tab.document.scope().get(block),
        Some(Block::ListItem {
            marker: ListMarker::Bullet,
            ..
        })
    ) {
        return None;
    }
    let text = tab.document.block_text(block);
    let (before, after) = text.split_once('[')?;
    (before.chars().all(char::is_whitespace) && after.chars().all(char::is_whitespace))
        .then(|| text.chars().count())
}

impl InsertShortcuts {
    /// Handles the custom Markdown gestures that create or configure
    /// structural nodes. Returns `true` when `c` was consumed; otherwise the
    /// normal Insert path writes it as prose.
    ///
    /// The gestures deliberately differ from the on-disk spelling where
    /// that makes typing faster: `*` is bold, `**` italic, and `***` both,
    /// exactly as the reader requested. Serialization remains canonical.
    fn apply(&mut self, c: char, docs: &mut Tabs, repeat: &mut Vec<InsertEvent>) -> bool {
        let marker = self.marker.take();
        match marker {
            Some(InsertMarker::OpenBracket) if c == '[' => {
                erase_marker_text(docs, repeat, 1);
                insert_event(docs, repeat, InsertEvent::ToggleBadge);
                self.reset();
                return true;
            }
            Some(InsertMarker::OpenBracket)
                if c == '^' && docs.active().is_some_and(|tab| !tab.document.in_table()) =>
            {
                erase_marker_text(docs, repeat, 1);
                insert_event(docs, repeat, InsertEvent::InsertSidenote);
                self.reset();
                return true;
            }
            Some(InsertMarker::BadgeClose) if c == ']' => {
                erase_marker_text(docs, repeat, 1);
                insert_event(docs, repeat, InsertEvent::ToggleBadge);
                self.reset();
                return true;
            }
            Some(InsertMarker::Equals) if c == '=' => {
                erase_marker_text(docs, repeat, 1);
                insert_event(docs, repeat, InsertEvent::ToggleHighlight);
                self.reset();
                return true;
            }
            _ => {}
        }

        // `- [ ]` is the on-disk spelling; the leading dash has already
        // made the empty block a bullet, so finishing the box simply
        // promotes that bullet to a task and removes the marker text.
        if c == ']'
            && let Some(count) = task_marker_len(docs)
        {
            erase_marker_text(docs, repeat, count);
            insert_event(docs, repeat, InsertEvent::TaskList);
            self.reset();
            return true;
        }

        if c != '*' {
            self.stars = 0;
        }
        if c != '`' {
            self.backticks = 0;
        }
        if c != '-' {
            self.dashes = 0;
        }
        if !c.is_ascii_digit() && c != '.' {
            self.ordered_digits = 0;
        }

        if c == '#' && empty_body_block(docs) && emphasis_allowed(docs) {
            let level = docs.active().and_then(|tab| {
                match tab.document.scope()[tab.document.caret.block] {
                    Block::Heading { level, .. } => level.checked_add(1),
                    _ => Some(1),
                }
            });
            if let Some(level @ 1..=4) = level {
                insert_event(docs, repeat, InsertEvent::SetHeading(level));
                return true;
            }
        }

        if c == '*' && emphasis_allowed(docs) {
            self.stars += 1;
            let (bold, italic) = match self.stars {
                1 => (true, false),
                2 => (false, true),
                3 => (true, true),
                _ => {
                    self.stars = 0;
                    return false;
                }
            };
            insert_event(docs, repeat, InsertEvent::SetEmphasis { bold, italic });
            return true;
        }

        if c == '`'
            && docs
                .active()
                .is_some_and(|tab| !tab.document.caret.style.badge)
        {
            if self.backticks == 2 && empty_body_block(docs) {
                insert_event(docs, repeat, InsertEvent::CodeBlock);
                self.reset();
            } else {
                self.backticks += 1;
                if self.backticks == 1 {
                    insert_event(docs, repeat, InsertEvent::ToggleInlineCode);
                }
            }
            return true;
        }

        if c == '-' && empty_body_block(docs) && emphasis_allowed(docs) {
            self.dashes += 1;
            match self.dashes {
                1 => insert_event(docs, repeat, InsertEvent::BulletList),
                2 => {}
                3 => {
                    insert_event(docs, repeat, InsertEvent::InsertDivider);
                    self.reset();
                }
                _ => unreachable!("three dashes reset the marker"),
            }
            return true;
        }

        if c.is_ascii_digit() {
            if self.ordered_digits > 0 || (empty_body_block(docs) && emphasis_allowed(docs)) {
                self.ordered_digits += 1;
            }
        } else if c == '.' && self.ordered_digits > 0 {
            let marker_is_intact = docs.active().is_some_and(|tab| {
                matches!(tab.document.focus, Focus::Body)
                    && tab
                        .document
                        .block_text(tab.document.caret.block)
                        .chars()
                        .count()
                        == self.ordered_digits
                    && tab
                        .document
                        .block_text(tab.document.caret.block)
                        .chars()
                        .all(|digit| digit.is_ascii_digit())
            });
            if marker_is_intact {
                erase_marker_text(docs, repeat, self.ordered_digits);
                insert_event(docs, repeat, InsertEvent::NumberedList);
                self.reset();
                return true;
            }
            self.ordered_digits = 0;
        } else {
            self.ordered_digits = 0;
        }

        match c {
            '[' if bracket_allowed(docs) => self.marker = Some(InsertMarker::OpenBracket),
            ']' if docs
                .active()
                .is_some_and(|tab| tab.document.caret.style.badge) =>
            {
                self.marker = Some(InsertMarker::BadgeClose)
            }
            '=' if emphasis_allowed(docs) => self.marker = Some(InsertMarker::Equals),
            _ => {}
        }
        false
    }
}

impl Shell {
    /// Choosing a vault is a shell job: the picker persists the config and
    /// hands the open vault back to the tree. While onboarding, only that
    /// one command can run — matched through the table like any other chord
    /// — plus the Enter key and the splash button as equivalent ways to ask
    /// for it. Once a vault is open, every chord in [`commands`] dispatches
    /// normally, `Ctrl+O` included.
    pub(super) fn handle_input(&mut self, input: &Input, viewport: Rect) {
        if input.is_key_pressed(KeyCode::Escape)
            && reset_brush_selection(
                &mut self.brush_selected,
                &mut self.brush_inside,
                &mut self.brush_point,
            )
        {
            self.brush_revision = self.brush_revision.wrapping_add(1);
        }
        // The finder owns all input while open, including title-bar clicks.
        if self.finder.is_some() {
            self.handle_finder_input(input);
            return;
        }
        // The palette swallows everything else while it's open.
        if self.palette.is_some() {
            self.handle_palette_input(input);
            return;
        }
        // The slash menu swallows everything else while it's open.
        if self.slash_menu.is_some() {
            self.handle_slash_menu_input(input);
            return;
        }
        // A dialog owns mouse input too, including Ctrl+clicks.
        if self.dialog.is_some() {
            self.handle_dialog_input(input, viewport);
            return;
        }
        // The line picker is a direct manipulation surface: while it is up,
        // clicks belong to its strokes and Escape/outside-click close it.
        if self.table_lines.is_some() {
            self.handle_table_lines_input(input, viewport);
            return;
        }
        // Ctrl+drag is an editor gesture, not a click-to-place or context
        // click. It also gets first refusal over an already-open context
        // menu so the existing multi-selection survives another stroke.
        if self.handle_brush_input(input) {
            return;
        }
        // The context menu swallows input while it is open.
        if self.context_menu.is_some() && self.handle_context_menu_input(input, viewport) {
            return;
        }
        // The word-format bar swallows input while open, like the context
        // menu; an outside click closes it and keeps routing so Normal mode
        // can immediately target what was clicked.
        if self.format_bar.is_some() && self.handle_format_bar_input(input, viewport) {
            return;
        }
        // Onboarding has exactly one live command — opening a vault — and the
        // splash's button and Enter key are the same request. Matching it out
        // of the table rather than against a literal chord keeps `COMMANDS`
        // the only place a binding is written down.
        if self.onboarding {
            let open = commands::matching(input).is_some_and(|command| command.id == "vault.open")
                || input.is_key_pressed(KeyCode::Enter)
                || (input.is_mouse_pressed(MouseButton::Left)
                    && onboarding::button(viewport).contains(input.mouse_position()));
            if open {
                self.open_picker();
            }
            return;
        }

        // The palette switch. Its rect comes from the title bar's node
        // rather than from anything the component recorded while drawing,
        // so a click on the first frame lands as well as one on the
        // thousandth. A swap already running swallows the click — see
        // `ThemeSwap`.
        if input.is_mouse_pressed(MouseButton::Left) && input.is_cursor_in_window() {
            let bar = self.layout.rect(self.regions[self.title_region].node());
            let command_id = if title_bar::sidebar_rect(bar).contains(input.mouse_position()) {
                Some("view.sidebar")
            } else if title_bar::focus_rect(bar).contains(input.mouse_position()) {
                Some("view.focus")
            } else {
                None
            };
            if let Some(command) =
                command_id.and_then(|id| commands::COMMANDS.iter().find(|command| command.id == id))
            {
                (command.run)(self);
                return;
            }
            let switch =
                theme_switch::switch_rect(self.layout.rect(self.regions[self.title_region].node()));
            if switch.contains(input.mouse_position()) {
                self.request_theme_swap((
                    switch.x + switch.width / 2.0,
                    switch.y + switch.height / 2.0,
                ));
                return;
            }
        }

        if input.is_mouse_pressed(MouseButton::Left)
            && input.is_cursor_in_window()
            && title_bar::search_box_rect(self.layout.rect(self.regions[self.title_region].node()))
                .contains(input.mouse_position())
        {
            self.open_finder();
            return;
        }

        if input.is_mouse_pressed(MouseButton::Left) && input.is_cursor_in_window() {
            let point = input.mouse_position();
            let new_tab = self.regions[self.tab_region]
                .component_as::<TabStrip>()
                .is_some_and(|tabs| tabs.new_note_at(point));
            let empty_action = if self.docs.borrow().active().is_none() {
                crate::components::empty_state::buttons(self.layout.rect(self.text_column))
                    .iter()
                    .position(|rect| rect.contains(point))
            } else {
                None
            };
            if new_tab || empty_action == Some(0) {
                if let Some(command) = commands::COMMANDS
                    .iter()
                    .find(|command| command.id == "file.new")
                {
                    (command.run)(self);
                }
                return;
            }
            if empty_action == Some(1) {
                self.open_finder();
                return;
            }
        }

        // The topics panel jumps the caret to its heading — through the
        // fold-aware jump, so a row that lists a folded heading (folds never
        // leave the outline) unfolds over it instead of landing nowhere.
        if input.is_mouse_pressed(MouseButton::Left)
            && input.is_cursor_in_window()
            && let Some(block) = self.topics_row_at(input.mouse_position())
        {
            self.docs.borrow_mut().jump_to_block(block);
            return;
        }

        // The tree owns its row hit-test. Consume its request first so a
        // right-click cannot also be interpreted as an editor menu request.
        if let Some((at, target)) = self.tree_menu_request.take() {
            let ids = match target {
                file_tree::MenuTarget::Row => commands::TREE_MENU,
                file_tree::MenuTarget::Empty => commands::TREE_ROOT_MENU,
            };
            self.open_context_menu(ids, at);
            return;
        }
        if input.is_mouse_pressed(MouseButton::Right)
            && input.is_cursor_in_window()
            && self
                .layout
                .rect(self.text_column)
                .contains(input.mouse_position())
        {
            // Right-click deliberately leaves the caret where it is: Cut and
            // Copy use the selection, or the caret's line when there is none.
            let has_file = self.docs.borrow().active().is_some();
            if has_file {
                self.open_context_menu(commands::EDITOR_MENU, input.mouse_position());
                return;
            }
        }

        // The in-math completion card claims its pick-and-accept chords
        // ahead of the command table — Ctrl+N and Ctrl+1..4 are global
        // commands outside math. Only claimed while the card is actually
        // showing; every other key keeps its math meaning.
        if self.math_menu.is_some() && self.handle_math_menu_input(input, viewport) {
            return;
        }

        if let Some(command) = commands::matching(input) {
            (command.run)(self);
            return;
        }

        // Past here the vim keymap owns the keyboard whether or not a file
        // is open — the leader has to reach the palette either way, and
        // `Tabs::touch`/`Tabs::edit` already no-op against an empty `Tabs`.
        self.edit_frame(input);
    }

    /// `note.edit`: focuses the note anchored at the caret, or does nothing
    /// when the caret is not on an anchor. The command-table way into a note;
    /// clicking the anchor and clicking the note are the other two.
    pub(super) fn edit_note_at_caret(&mut self) {
        let index = {
            let docs = self.docs.borrow();
            let Some(tab) = docs.active() else { return };
            note_at_caret(&tab.document, tab.document.caret)
        };
        if let Some(index) = index {
            self.sidenotes.set_open(true);
            focus_note(&mut self.docs.borrow_mut(), index);
        }
    }

    fn open_table_lines(&mut self, first: usize, row: usize, column: usize, anchor: (f32, f32)) {
        self.context_menu = None;
        self.format_bar = None;
        self.table_lines = Some(TableLinesState {
            first,
            row,
            column,
            anchor,
        });
        self.refresh_table_lines();
    }

    fn refresh_table_lines(&mut self) {
        let menu = self.table_lines.as_ref().and_then(|state| {
            self.docs.borrow().active().and_then(|tab| {
                let body = tab.document.body().get(state.first..)?;
                let settings = body.first()?.table_settings()?;
                // The first row anchors this table. A following table also
                // has a `first` row, but it is the next block rather than a
                // continuation of this one.
                let rows = body
                    .iter()
                    .enumerate()
                    .take_while(|(index, block)| {
                        *index == 0 || matches!(block, Block::TableRow { first: false, .. })
                    })
                    .count();
                let columns = body.first().map_or(0, |block| block.inlines().len());
                Some(crate::components::TableLinesMenu::new(
                    settings.lines,
                    state.anchor,
                    state.row.min(rows.saturating_sub(1)),
                    state.column.min(columns.saturating_sub(1)),
                    rows,
                    columns,
                ))
            })
        });
        self.regions[self.table_lines_region].set_component(Box::new(
            menu.unwrap_or_else(crate::components::TableLinesMenu::closed),
        ));
    }

    fn close_table_lines(&mut self) {
        self.table_lines = None;
        self.refresh_table_lines();
    }

    fn handle_table_lines_input(&mut self, input: &Input, viewport: Rect) {
        if input.is_key_pressed(KeyCode::Escape) {
            self.close_table_lines();
            return;
        }
        if !input.is_mouse_pressed(MouseButton::Left) || !input.is_cursor_in_window() {
            return;
        }
        let point = input.mouse_position();
        let Some(state) = self.table_lines.as_ref() else {
            return;
        };
        let card = table_lines::card_anchored(viewport, state.anchor);
        match table_lines::hit_at(card, point) {
            Some(table_lines::TableHit::Line(line)) => {
                self.docs.borrow_mut().toggle_table_line(state.first, line);
                self.refresh_table_lines();
            }
            Some(table_lines::TableHit::Action(action)) => {
                let mut docs = self.docs.borrow_mut();
                match action {
                    table_lines::TableAction::InsertRow => {
                        docs.insert_table_row(state.first, state.row);
                    }
                    table_lines::TableAction::RemoveRow => {
                        docs.remove_table_row(state.first, state.row);
                    }
                    table_lines::TableAction::InsertColumn => {
                        docs.insert_table_column(state.first, state.column);
                    }
                    table_lines::TableAction::RemoveColumn => {
                        docs.remove_table_column(state.first, state.column);
                    }
                }
                drop(docs);
                self.refresh_table_lines();
            }
            None => self.close_table_lines(),
        }
    }

    /// Update a live table divider drag. The delta uses the last point, not
    /// the drag origin, so every frame applies one stable adjustment and the
    /// table does not jump when its layout is rebuilt under the pointer.
    fn drive_table_drag(&mut self, input: &Input) -> bool {
        let Some(drag) = &mut self.table_drag else {
            return false;
        };
        if !input.is_mouse_down(MouseButton::Left) {
            self.docs.borrow_mut().end_transaction();
            self.table_drag = None;
            return true;
        }
        let point = input.mouse_position();
        let delta = (point.0 - drag.last.0, point.1 - drag.last.1);
        if delta == (0.0, 0.0) {
            return true;
        }
        let resize = drag.resize;
        drag.last = point;
        let rect = self.layout.rect(self.text_column);
        let layout = self.current_layout(Editor::content_width(rect));
        match resize {
            TableResize::Column { first, divider } => {
                let width = layout
                    .tables
                    .get(first)
                    .and_then(Option::as_ref)
                    .map(|table| table.columns.iter().sum::<f32>())
                    .unwrap_or(0.0);
                if width > 0.0 {
                    self.docs
                        .borrow_mut()
                        .resize_table_column(first, divider, delta.0 / width);
                }
            }
            TableResize::Row { first, divider } => {
                self.docs
                    .borrow_mut()
                    .resize_table_row(first, divider, delta.1 / layout.scale);
            }
        }
        true
    }

    /// Whether a click landed on a fold affordance — a gutter chevron or a
    /// collapsed-body indicator — and toggled/unfolded it. Runs before caret
    /// placement in either mode: these are controls, not text.
    fn fold_click(&mut self, rect: Rect, mouse: (f32, f32)) -> bool {
        {
            let docs = self.docs.borrow();
            if docs.active().is_none() {
                return false;
            }
        }
        let (local_x, local_y) = {
            // Unclamped x on purpose: the chevron lives in the gutter left
            // of the text column, where a clamped point would read as the
            // text's own left edge. y keeps the page's scroll like clicks.
            let scroll = self.docs.borrow().editor_scroll;
            (
                mouse.0 - (Editor::content_x(rect)),
                mouse.1 - rect.y - editor::TOP + scroll,
            )
        };
        let layout = self.current_layout(editor::Editor::content_width(rect));
        let layer = self.regions[self.text_region].layer();
        let measure = |text: &str, style: &TextStyle| theme::width(layer, text, style);
        if let Some(block) = layout.fold_chevron_at(local_x, local_y, &measure) {
            self.docs.borrow_mut().toggle_fold_at(block);
            return true;
        }
        if local_x >= 0.0
            && let Some(block) = layout.fold_indicator_at(local_y)
        {
            self.docs.borrow_mut().reveal_block(block);
            return true;
        }
        false
    }

    /// The outline heading a click on the topics panel landed on, if any —
    /// the same rects the hover reads, so click and highlight cannot drift.
    fn topics_row_at(&self, point: (f32, f32)) -> Option<usize> {
        if !self.topics.open || !self.layout.rect(self.topics.node).contains(point) {
            return None;
        }
        let index = self.regions[self.topics_region]
            .component_as::<Topics>()
            .and_then(|panel| panel.entry_at(point))?;
        self.outline().get(index).map(|node| node.block)
    }

    /// A Normal-mode click on an anchor opens that note. Resolves the click
    /// to its caret, then asks whether that caret sits on an anchor — the
    /// same question `note.edit` asks, so the two ways in cannot disagree
    /// about what counts as "on an anchor". Returns whether the click was
    /// consumed.
    fn click_anchor(&mut self, rect: Rect, mouse: (f32, f32)) -> bool {
        let Some(caret) = self.caret_at(rect, mouse) else {
            return false;
        };
        let index = {
            let docs = self.docs.borrow();
            let Some(tab) = docs.active() else {
                return false;
            };
            note_at_caret(&tab.document, caret)
        };
        let Some(index) = index else { return false };
        self.goal_x = None;
        self.sidenotes.set_open(true);
        focus_note(&mut self.docs.borrow_mut(), index);
        true
    }

    /// The note under `point` in the margin, as `(note index, caret)`, or
    /// `None` when the point is not on a note. The caret is resolved against
    /// the note's own layout — never handed to a `body()`-resolving function,
    /// whose coordinates come from the page's layout and are wrong here.
    fn margin_note_at(&mut self, point: (f32, f32)) -> Option<(usize, Caret)> {
        if !self.layout.style(self.sidenotes.node).visible {
            return None;
        }
        let rect = self.layout.rect(self.sidenotes.node);
        let (scroll, index_of) = {
            let docs = self.docs.borrow();
            let index_of: Vec<(String, usize)> = match docs.active() {
                Some(tab) => tab
                    .document
                    .notes
                    .iter()
                    .enumerate()
                    .map(|(index, note)| (note.label.clone(), index))
                    .collect(),
                None => Vec::new(),
            };
            (docs.editor_scroll, index_of)
        };
        let content_top = rect.y + editor::TOP - scroll;
        let notes: Vec<(String, f32, Rc<DocLayout>)> = self
            .stacked_notes()
            .into_iter()
            .map(|(label, _, y, layout)| (label, y, layout))
            .collect();
        let layer = self.regions[self.text_region].layer();
        let measure = |text: &str, style: &TextStyle| theme::width(layer, text, style);
        let (label, caret) = note_at_point(
            &notes,
            point.0,
            point.1,
            rect.x,
            content_top,
            sidenotes::NOTE_INSET,
            &measure,
        )?;
        let index = index_of
            .iter()
            .find(|(note_label, _)| *note_label == label)
            .map(|(_, index)| *index)?;
        Some((index, caret))
    }

    fn handle_brush_input(&mut self, input: &Input) -> bool {
        let revision = self.docs.borrow().revision();
        if self.brush_document_revision != revision {
            if reset_brush_selection(
                &mut self.brush_selected,
                &mut self.brush_inside,
                &mut self.brush_point,
            ) {
                self.brush_revision = self.brush_revision.wrapping_add(1);
            }
            self.brush_document_revision = revision;
        }
        if !(input.ctrl() && input.is_mouse_down(MouseButton::Left)) {
            let released = self.brush_point.take().is_some();
            if released || !self.brush_inside.is_empty() {
                self.brush_inside.clear();
                self.brush_revision = self.brush_revision.wrapping_add(1);
            }
            if released {
                // A stroke that touched code words is a colouring gesture:
                // its palette is the more specific tool, and it claims the
                // release ahead of the prose bar.
                if let Some(target) = self
                    .brush_selected
                    .iter()
                    .find(|hit| {
                        matches!(
                            hit,
                            ContextHit::Range {
                                kind: RangeKind::CodeWord,
                                ..
                            }
                        )
                    })
                    .cloned()
                {
                    self.open_context_target(
                        commands::CODE_COLOR_MENU,
                        input.mouse_position(),
                        Some(target),
                    );
                    return true;
                }
                // Anything else with a word in it opens the format bar over
                // the whole selection — the point of sweeping several words
                // is to format them together.
                if self.brush_selected.iter().any(|hit| {
                    matches!(
                        hit,
                        ContextHit::Range {
                            kind: RangeKind::Word,
                            ..
                        }
                    )
                }) {
                    self.open_format_bar(self.brush_selected.clone(), input.mouse_position());
                    return true;
                }
            }
            return false;
        }

        let rect = self.layout.rect(self.text_column);
        let point = input.mouse_position();
        let over_editor = self.docs.borrow().active().is_some()
            && input.is_cursor_in_window()
            && rect.contains(point);
        if !over_editor {
            let was_brushing = self.brush_point.is_some() || !self.brush_inside.is_empty();
            if self.brush_point.take().is_some() || !self.brush_inside.is_empty() {
                self.brush_inside.clear();
                self.brush_revision = self.brush_revision.wrapping_add(1);
            }
            return was_brushing;
        }

        if self.context_menu.is_some() {
            self.close_context_menu();
        }
        if self.format_bar.is_some() {
            self.close_format_bar();
        }
        let Some((x, y)) = self.editor_point(rect, point) else {
            return true;
        };
        let content_width = editor::Editor::content_width(rect);
        let layout = self.current_layout(content_width);
        let layer = self.regions[self.text_region].layer();
        let measure = |text: &str, style: &TextStyle| theme::width(layer, text, style);
        let current_hits =
            layout.hit_contexts_in_circle(x, y, editor::BRUSH_RADIUS, content_width, &measure);
        let mut swept_hits = current_hits.clone();
        if let Some(previous) = self
            .brush_point
            .and_then(|previous| self.editor_point(rect, previous))
        {
            for (sample_x, sample_y) in brush_sweep_samples(previous, (x, y), editor::BRUSH_RADIUS)
            {
                for hit in layout.hit_contexts_in_circle(
                    sample_x,
                    sample_y,
                    editor::BRUSH_RADIUS,
                    content_width,
                    &measure,
                ) {
                    if !swept_hits.contains(&hit) {
                        swept_hits.push(hit);
                    }
                }
            }
        }

        add_brush_hits(
            &mut self.brush_selected,
            &mut self.brush_inside,
            swept_hits,
            current_hits,
        );
        self.brush_point = Some(point);
        self.brush_revision = self.brush_revision.wrapping_add(1);
        true
    }

    /// Scroll, click-to-place, and the arrow/Home/End keys need an actual
    /// buffer, so they're guarded on a tab being active. The vim keymap and
    /// the divider drag aren't: what a plain key means past scroll/arrows is
    /// mode-dependent, so it's split into
    /// [`Shell::edit_frame_insert`]/[`Shell::edit_frame_normal`].
    fn edit_frame(&mut self, input: &Input) {
        if self.drive_table_drag(input) {
            return;
        }
        let rect = self.layout.rect(self.text_column);
        let mouse = input.mouse_position();
        let over_editor = input.is_cursor_in_window() && rect.contains(mouse);
        let has_tab = self.docs.borrow().active().is_some();
        let in_math = self.docs.borrow().in_math();

        // A pending two-character marker belongs to one uninterrupted typing
        // gesture. Moving the caret or clicking elsewhere makes its first
        // character ordinary prose, rather than allowing a later keystroke
        // to reach back across the move and reinterpret it.
        if matches!(self.vim.current_mode(), VimMode::Insert)
            && (input.is_key_typed(KeyCode::ArrowUp)
                || input.is_key_typed(KeyCode::ArrowDown)
                || input.is_key_typed(KeyCode::ArrowLeft)
                || input.is_key_typed(KeyCode::ArrowRight)
                || input.is_key_typed(KeyCode::Home)
                || input.is_key_typed(KeyCode::End)
                || input.is_mouse_pressed(MouseButton::Left))
        {
            self.insert_shortcuts.reset();
        }

        // Anywhere over the sheet, not just over the text: a pointer that
        // has drifted onto a margin note is still pointing at the document.
        let over_canvas =
            input.is_cursor_in_window() && self.layout.rect(self.canvas).contains(mouse);
        if has_tab && over_canvas && input.scroll_delta().1 != 0.0 {
            let (min, max) = self.editor_scroll_bounds();
            // Aimed from where the page is heading, not from where it is:
            // a second notch mid-glide must add its own distance rather
            // than measure from a journey that has not finished.
            let step =
                input.scroll_delta().1 * super::SCROLL_LINES * crate::document::layout::LINE_BODY;
            self.scroll
                .aim((self.scroll.target() - step).clamp(min, max));
        }

        if has_tab && !in_math && !self.vim.command_active() {
            self.arrow_keys(input);
        }

        match self.vim.current_mode() {
            VimMode::Insert => self.edit_frame_insert(input),
            _ => self.edit_frame_normal(input),
        }

        if has_tab && input.is_mouse_pressed(MouseButton::Left) && over_editor {
            // Fold affordances claim their clicks first: the gutter chevron
            // and the collapsed-body indicator are controls, not text, in
            // either mode.
            if self.fold_click(rect, mouse) {
                return;
            }
            if let Some((local_x, local_y)) = self.editor_point(rect, mouse) {
                let layout = self.current_layout(editor::Editor::content_width(rect));
                if let Some(resize) = layout.table_resize_at(local_x, local_y, 5.0) {
                    self.goal_x = None;
                    self.docs.borrow_mut().begin_transaction();
                    self.table_drag = Some(TableDrag {
                        resize,
                        last: mouse,
                    });
                    return;
                }
            }
            // A click on a task's checkbox toggles it, whatever the mode:
            // it is a control, not a caret placement.
            if let Some((local_x, local_y)) = self.editor_point(rect, mouse)
                && let Some(block) = self
                    .current_layout(editor::Editor::content_width(rect))
                    .task_at(local_x, local_y)
            {
                self.goal_x = None;
                self.docs.borrow_mut().toggle_task_at(block);
            } else if self.vim.current_mode() == VimMode::Normal {
                if self.click_anchor(rect, mouse) {
                    // The click focused a note; nothing else to do with it.
                } else if let Some(target) = self.context_at(rect, mouse) {
                    if matches!(
                        target,
                        ContextHit::Range {
                            kind: RangeKind::CodeWord,
                            ..
                        }
                    ) && !self.brush_selected.contains(&target)
                    {
                        self.brush_selected.clear();
                        self.brush_revision = self.brush_revision.wrapping_add(1);
                    }
                    if matches!(
                        target,
                        ContextHit::Range {
                            kind: RangeKind::Word,
                            ..
                        }
                    ) {
                        // A word opens the format bar; any other target
                        // keeps the classic list menu.
                        self.open_format_bar(vec![target], mouse);
                    } else {
                        let ids = self.context_ids(&target);
                        self.open_context_target(&ids, mouse, Some(target));
                    }
                } else if let Some((local_x, local_y)) = self.editor_point(rect, mouse)
                    && let Some(cell) = self
                        .current_layout(editor::Editor::content_width(rect))
                        .table_cell_at(local_x, local_y)
                {
                    // Text itself reaches the familiar word-format bar above;
                    // the cell's quiet gutter opens the table's own direct
                    // manipulation card.
                    self.open_table_lines(cell.first, cell.row, cell.column, mouse);
                }
            } else if let Some((block, inline, offset, cursor)) = self.math_at(rect, mouse) {
                self.goal_x = None;
                set_body_coordinate_caret(
                    &mut self.docs.borrow_mut(),
                    BodyCoordinate::Caret(Caret {
                        block,
                        inline,
                        offset: 0,
                        style: Style::PLAIN,
                    }),
                );
                self.docs
                    .borrow_mut()
                    .enter_math_at(block, inline, offset, cursor);
                self.apply(ExtendedAction::Enter(Mode::Insert));
            } else if let Some(caret) = self.caret_at(rect, mouse) {
                self.goal_x = None;
                set_body_coordinate_caret(
                    &mut self.docs.borrow_mut(),
                    BodyCoordinate::Caret(caret),
                );
            }
        }

        // A click on a note in the margin opens it at the clicked position.
        // The margin shares the page's scroll, so the same rect math the
        // editor uses places the note's text on screen.
        if has_tab && input.is_mouse_pressed(MouseButton::Left) && input.is_cursor_in_window() {
            let margin = self.layout.rect(self.sidenotes.node);
            if self.sidenotes.open
                && margin.contains(mouse)
                && let Some((index, caret)) = self.margin_note_at(mouse)
            {
                self.goal_x = None;
                focus_note_at(&mut self.docs.borrow_mut(), index, caret);
            }
        }

        self.divider_drag(input);
        self.scrollbar_drag(input);
    }

    /// Arrow keys, Home, End. Vertical motion goes through the layout so it
    /// aims at a remembered goal x; horizontal through the model's style
    /// machine; Home/End are the visual line's text extremes. Vertical
    /// motion records the goal x (`goal_col` in pixels, spec §5); any
    /// non-vertical caret move clears it.
    fn arrow_keys(&mut self, input: &Input) {
        let table_direction = if input.is_key_typed(KeyCode::ArrowUp) {
            Some(TableDirection::Up)
        } else if input.is_key_typed(KeyCode::ArrowDown) {
            Some(TableDirection::Down)
        } else if input.is_key_typed(KeyCode::ArrowLeft) {
            Some(TableDirection::Left)
        } else if input.is_key_typed(KeyCode::ArrowRight) {
            Some(TableDirection::Right)
        } else {
            None
        };
        if let Some(direction) = table_direction
            && self.docs.borrow_mut().table_move(direction)
        {
            self.goal_x = None;
            return;
        }
        if input.is_key_typed(KeyCode::ArrowUp) || input.is_key_typed(KeyCode::ArrowDown) {
            let rect = self.layout.rect(self.text_column);
            let width = crate::components::editor::Editor::content_width(rect);
            let layout = self.current_layout(width);
            let layer = self.regions[self.text_region].layer();
            let measure = |text: &str, style: &TextStyle| theme::width(layer, text, style);
            let (goal, next, in_table) = {
                let docs = self.docs.borrow();
                let Some(tab) = docs.active() else {
                    return;
                };
                let caret = tab.document.caret;
                let goal = self
                    .goal_x
                    .unwrap_or_else(|| layout.caret_pos(caret, &measure).0);
                let next = if input.is_key_typed(KeyCode::ArrowUp) {
                    layout.line_up(caret, goal, &measure)
                } else {
                    layout.line_down(caret, goal, &measure)
                };
                (goal, next, tab.document.in_table())
            };
            self.goal_x = Some(goal);
            if let Some(next) = next {
                self.docs
                    .borrow_mut()
                    .move_caret_to(next.block, next.inline, next.offset);
            } else if in_table {
                if input.is_key_typed(KeyCode::ArrowUp) {
                    self.docs.borrow_mut().open_above();
                } else {
                    self.docs.borrow_mut().open_below();
                }
            }
            return;
        }
        // Any other arrow clears the vertical goals.
        self.goal_x = None;
        if input.is_key_typed(KeyCode::ArrowLeft) {
            if matches!(self.vim.current_mode(), VimMode::Insert) {
                let entered = self.docs.borrow_mut().enter_math_before();
                if entered {
                    return;
                }
            }
            self.docs.borrow_mut().move_left();
        }
        if input.is_key_typed(KeyCode::ArrowRight) {
            if matches!(self.vim.current_mode(), VimMode::Insert) {
                let entered = self.docs.borrow_mut().enter_math_after();
                if entered {
                    return;
                }
            }
            self.docs.borrow_mut().move_right();
        }
        if input.is_key_typed(KeyCode::Home) {
            self.visual_line_home_end(true);
        }
        if input.is_key_typed(KeyCode::End) {
            self.visual_line_home_end(false);
        }
    }

    /// Home/End on the caret's visual line, via a hit test at x=0 / x=∞ of
    /// that line's band — the layout owns where visual lines are.
    fn visual_line_home_end(&mut self, home: bool) {
        let rect = self.layout.rect(self.text_column);
        let width = crate::components::editor::Editor::content_width(rect);
        let layout = self.current_layout(width);
        let layer = self.regions[self.text_region].layer();
        let measure = |text: &str, style: &TextStyle| theme::width(layer, text, style);
        let hit = {
            let docs = self.docs.borrow();
            let Some(tab) = docs.active() else {
                return;
            };
            let caret = tab.document.caret;
            let (top, bottom) = layout.caret_band(caret);
            let y = (top + bottom) / 2.0;
            if home {
                layout.hit(0.0, y, &measure)
            } else {
                layout.hit(f32::MAX, y, &measure)
            }
        };
        self.goal_x = None;
        self.docs
            .borrow_mut()
            .move_caret_to(hit.block, hit.inline, hit.offset);
    }

    /// Insert: today's typing, plus Escape popping one level back (style
    /// context first, then mode). Formatting lives in the slash menu.
    fn edit_frame_insert(&mut self, input: &Input) {
        // Inside an expression, math owns every key: `/` builds a fraction
        // rather than opening the slash menu, Tab walks slots, and Esc pops
        // one level of structure before it pops the mode. Vim must not see
        // these keys; math has no Vim state to advance, and feeding it
        // characters would desynchronise that state.
        let in_math = self.docs.borrow().in_math();
        if in_math {
            self.edit_frame_math(input);
            return;
        }

        if input.is_key_typed(KeyCode::Tab) && self.docs.borrow_mut().table_tab(input.shift()) {
            self.insert_shortcuts.reset();
            self.goal_x = None;
            return;
        }

        // Bare / opens the slash menu — before the docs borrow so we can
        // call refresh_slash_menu (which needs &mut self) without a conflict.
        let text = input.text();
        if text == "/" {
            self.insert_shortcuts.reset();
            let anchor = self.compute_slash_anchor();
            // An in-flight fade-out of THIS menu is superseded: it is back.
            if matches!(self.menu_dismiss, Some(MenuDismiss::Slash { .. })) {
                self.menu_dismiss = None;
                self.menu_dismiss_clock = 0.0;
            }
            self.slash_menu = Some(SlashMenuState {
                query: String::new(),
                selected: 0,
                anchor,
            });
            self.popup_reveal.restart();
            self.refresh_slash_menu();
            return;
        }
        {
            for c in input.text().chars() {
                // `$` is math entry (SPEC §12.1: the one-key alias for the
                // highest-frequency gesture in the app). A second `$` before
                // anything is typed takes it back and leaves a literal
                // dollar, so the character is still reachable — the same
                // escape hatch `//` gives the slash menu.
                //
                // Once open, the rest of this frame's text belongs to the
                // expression: at speed, two characters land in one frame, and
                // the `in_math` read at the top of this function is already
                // stale by then.
                if self.docs.borrow().in_math() {
                    self.math_type_char(c);
                    continue;
                }
                if c == '$' {
                    self.insert_shortcuts.reset();
                    self.docs.borrow_mut().insert_inline_math();
                    continue;
                }
                let consumed = {
                    let mut docs = self.docs.borrow_mut();
                    self.insert_shortcuts
                        .apply(c, &mut docs, &mut self.insert_repeat)
                };
                if consumed {
                    continue;
                }
                if matches!(
                    self.vim.key_extended(Key::Char(c)),
                    ExtendedAction::Passthrough
                ) {
                    self.docs.borrow_mut().type_text(&c.to_string());
                    self.insert_repeat.push(super::InsertEvent::Text(c));
                }
            }
            // A `$` in this frame moved the caret into an expression, and
            // everything below here is prose-addressed: a prose backspace
            // with the caret on an atom eats the character *before* it. The
            // rest of the frame belongs to math, which picks it up next
            // frame; dropping one key beats editing the wrong scope.
            if self.docs.borrow().in_math() {
                self.insert_shortcuts.reset();
                self.goal_x = None;
                return;
            }
            if input.is_key_typed(KeyCode::Backspace) {
                self.insert_shortcuts.reset();
                let _ = self.vim.key_extended(Key::Backspace);
                self.docs.borrow_mut().backspace();
                self.insert_repeat.push(super::InsertEvent::Backspace);
            }
            if input.is_key_typed(KeyCode::Delete) {
                self.insert_shortcuts.reset();
                self.docs.borrow_mut().delete_forward();
                self.insert_repeat.push(super::InsertEvent::Delete);
            }
            if input.is_key_typed(KeyCode::Enter) {
                self.insert_shortcuts.reset();
                let _ = self.vim.key_extended(Key::Enter);
                self.docs.borrow_mut().newline();
                self.insert_repeat.push(super::InsertEvent::Enter);
            }
        }
        if input.is_key_pressed(KeyCode::Escape) {
            self.insert_shortcuts.reset();
            // Esc pops one level (§4.3): a pending style context → plain,
            // still Insert; plain → Normal. Popping the style must bump the
            // revision (like any caret change) so the B/I marker disappears
            // this frame, not on the next typed character.
            let was_styled = self
                .docs
                .borrow()
                .active()
                .is_some_and(|t| !t.document.caret.style.is_plain());
            if was_styled {
                self.docs.borrow_mut().touch(|doc| {
                    doc.caret.style = crate::document::Style::PLAIN;
                });
            } else {
                self.docs.borrow_mut().end_transaction();
                if !self.insert_repeat.is_empty() {
                    let insert = super::RepeatOp::Insert(std::mem::take(&mut self.insert_repeat));
                    self.repeat = Some(match self.insert_prefix.take() {
                        Some(prefix) => super::RepeatOp::Sequence(vec![prefix, insert]),
                        None => insert,
                    });
                }
                let action = self.vim.key_extended(Key::Escape);
                self.apply(action);
            }
            self.goal_x = None;
        }
    }

    /// One typed character while the caret is inside an expression. Split out
    /// of [`Shell::edit_frame_math`] because Insert mode reaches it too: a `$`
    /// opens math mid-frame, and everything typed after it in that same frame
    /// belongs to the expression, not to the prose.
    fn math_type_char(&mut self, c: char) {
        match c {
            // `$` closes an expression the way it opened one. On an
            // expression still empty it is taken back entirely and a literal
            // dollar is typed instead — the escape hatch, mirroring `//`.
            '$' => {
                let discarded = self.docs.borrow_mut().math_discard_if_empty();
                if discarded {
                    self.docs.borrow_mut().type_text("$");
                } else {
                    self.docs.borrow_mut().math_exit_after();
                }
            }
            c if math::PAIRS
                .iter()
                .any(|&(open, close)| open == c || close == c) =>
            {
                let closed = {
                    let mut docs = self.docs.borrow_mut();
                    docs.math_close_group(c)
                };
                if !closed {
                    let opened = {
                        let mut docs = self.docs.borrow_mut();
                        docs.math_open_group(c)
                    };
                    if !opened {
                        self.docs.borrow_mut().math_type(c);
                    }
                }
            }
            ' ' => {
                let inserted = self.docs.borrow_mut().math_insert_word();
                if !inserted {
                    self.docs.borrow_mut().math_type(c);
                }
            }
            '/' => self.docs.borrow_mut().math_fraction(),
            '^' => self.docs.borrow_mut().math_script(Slot::Sup),
            '_' => self.docs.borrow_mut().math_script(Slot::Sub),
            _ => self.docs.borrow_mut().math_type(c),
        }
    }

    fn edit_frame_math(&mut self, input: &Input) {
        for c in input.text().chars() {
            self.math_type_char(c);
        }
        if input.is_key_typed(KeyCode::Backspace) {
            let _ = delete_inside_math(&mut self.docs.borrow_mut(), false);
        }
        if input.is_key_typed(KeyCode::Delete) {
            let _ = delete_inside_math(&mut self.docs.borrow_mut(), true);
        }
        if input.is_key_typed(KeyCode::Tab) {
            if input.shift() {
                self.docs.borrow_mut().math_slot_prev();
            } else {
                self.docs.borrow_mut().math_slot_next();
            }
        }
        if input.is_key_typed(KeyCode::ArrowLeft) {
            let moved = self.docs.borrow_mut().math_left();
            if !moved {
                self.docs.borrow_mut().math_exit_before();
            }
        }
        if input.is_key_typed(KeyCode::ArrowRight) {
            let moved = self.docs.borrow_mut().math_right();
            if !moved {
                self.docs.borrow_mut().math_exit_after();
            }
        }
        if input.is_key_pressed(KeyCode::Escape) {
            let popped = self.docs.borrow_mut().math_pop();
            if !popped {
                self.docs.borrow_mut().math_exit_after();
            }
        }
        if input.is_key_typed(KeyCode::Enter) {
            self.docs.borrow_mut().math_exit_after();
            self.docs.borrow_mut().newline();
        }
    }

    /// Normal and command modes consume resolved characters one at a time.
    fn edit_frame_normal(&mut self, input: &Input) {
        if input.is_key_typed(KeyCode::Tab) && self.docs.borrow_mut().table_tab(input.shift()) {
            self.goal_x = None;
            return;
        }
        for c in input.text().chars() {
            if !self.vim.command_active() && c == '/' && self.vim.visual_mode().is_some() {
                let anchor = self.compute_slash_anchor();
                if matches!(self.menu_dismiss, Some(MenuDismiss::Slash { .. })) {
                    self.menu_dismiss = None;
                    self.menu_dismiss_clock = 0.0;
                }
                self.slash_menu = Some(SlashMenuState {
                    query: String::new(),
                    selected: 0,
                    anchor,
                });
                self.popup_reveal.restart();
                self.refresh_slash_menu();
                return;
            }
            if !self.vim.command_active() && matches!(c, '/' | '?') {
                self.search = Some(super::SearchState {
                    query: String::new(),
                    forward: c == '/',
                });
                let _ = self.vim.key_extended(Key::Char(c));
                continue;
            }
            let action = self.vim.key_extended(Key::Char(c));
            self.apply(action);
            // The leader opened the palette, which now owns the keyboard —
            // any further characters buffered this same frame belong to it,
            // not to a buffer edit underneath.
            if self.palette.is_some() {
                return;
            }
        }
        if input.ctrl() && input.is_key_typed(KeyCode::KeyR) {
            let action = self.vim.key_extended(Key::Control('r'));
            self.apply(action);
        }
        if input.is_key_typed(KeyCode::Backspace) {
            let action = self.vim.key_extended(Key::Backspace);
            self.apply(action);
        }
        if input.is_key_typed(KeyCode::Enter) {
            let action = self.vim.key_extended(Key::Enter);
            self.apply(action);
        }
        if input.is_key_pressed(KeyCode::Escape) {
            // Escape pops one level in Normal mode too: leaving a focused
            // note returns focus to the body. Esc while searching (or in any
            // other state that already owns Escape) clears that first, so
            // leaving the note is one more Escape away — the same shape as
            // leaving Insert and then leaving anything else.
            let normal = self.vim.current_mode() == VimMode::Normal;
            if self.vim.command_active() {
                self.search = None;
            }
            let action = self.vim.key_extended(Key::Escape);
            self.apply(action);
            if normal {
                return_to_anchor(&mut self.docs.borrow_mut());
            }
        }
    }

    /// Applies one extended vim action — the only place a motion or edit actually
    /// reaches a document. Vertical motions are applied through the layout,
    /// which is the only thing that knows where visual lines are.
    fn apply(&mut self, action: ExtendedAction) {
        match action {
            ExtendedAction::None
            | ExtendedAction::OperatorPending(..)
            | ExtendedAction::Passthrough => {}
            ExtendedAction::Move(Motion::Up, count) => {
                self.visual_override = None;
                self.vertical_motion(true, count)
            }
            ExtendedAction::Move(Motion::Down, count) => {
                self.visual_override = None;
                self.vertical_motion(false, count)
            }
            ExtendedAction::Move(m, count) => {
                self.visual_override = None;
                let mut docs = self.docs.borrow_mut();
                if !move_inside_math(&mut docs, m, count) {
                    docs.touch(|doc| motion::apply(doc, m, count));
                }
                self.goal_x = None;
            }
            ExtendedAction::InsertAt(m) => {
                self.insert_prefix = None;
                self.start_insert();
                self.docs.borrow_mut().touch(|doc| motion::apply(doc, m, 1));
                self.goal_x = None;
            }
            ExtendedAction::Edit(edit, count) => {
                self.repeat = Some(super::RepeatOp::Edit(edit, count));
                self.insert_prefix = if matches!(edit, Edit::OpenBelow | Edit::OpenAbove) {
                    Some(super::RepeatOp::Edit(edit, count))
                } else {
                    None
                };
                self.apply_edit(edit, count)
            }
            ExtendedAction::Enter(mode) => {
                if self.search.is_some() {
                    self.finish_search();
                }
                match mode {
                    Mode::Insert => self.start_insert(),
                    Mode::Normal => self.vim.set_mode(Mode::Normal),
                }
                self.goal_x = None;
            }
            ExtendedAction::Leader => self.open_palette(),
            ExtendedAction::Operate(operator, target, count) => {
                if operator != Operator::Yank {
                    self.repeat = Some(super::RepeatOp::Operate(operator, target, count));
                }
                self.apply_operator(operator, target, count);
            }
            ExtendedAction::Visual(action) => self.apply_visual(action),
            ExtendedAction::CommandInput(key) => self.command_input(key),
            ExtendedAction::Undo => self.docs.borrow_mut().undo(),
            ExtendedAction::Redo => self.docs.borrow_mut().redo(),
            ExtendedAction::Repeat => self.repeat(),
            ExtendedAction::Search { forward, count } => self.search_repeat(forward, count),
            ExtendedAction::Substitute(count) => {
                self.docs.borrow_mut().begin_transaction();
                self.repeat = Some(super::RepeatOp::Substitute(count));
                self.insert_prefix = Some(super::RepeatOp::Substitute(count));
                self.apply_edit(Edit::DeleteChar, count);
                self.start_insert();
            }
            ExtendedAction::Paste { before, count } => {
                self.repeat = Some(super::RepeatOp::Paste { before, count });
                self.paste(before, count);
                self.goal_x = None;
            }
        }
    }

    fn command_input(&mut self, key: Key) {
        let Some(search) = &mut self.search else {
            return;
        };
        match key {
            Key::Char(c) => search.query.push(c),
            Key::Backspace => {
                search.query.pop();
            }
            _ => {}
        }
    }

    fn finish_search(&mut self) {
        let Some(search) = self.search.take() else {
            return;
        };
        if search.query.is_empty() {
            return;
        }
        self.last_search = Some((search.query.clone(), search.forward));
        self.find_text(&search.query, search.forward, 1);
    }

    fn search_repeat(&mut self, forward: bool, count: usize) {
        let Some((query, last_forward)) = self.last_search.clone() else {
            return;
        };
        self.find_text(
            &query,
            if forward { last_forward } else { !last_forward },
            count,
        );
    }

    fn find_text(&mut self, query: &str, forward: bool, count: usize) {
        if query.is_empty() {
            return;
        }
        let (current, blocks) = {
            let docs = self.docs.borrow();
            let Some(tab) = docs.active() else { return };
            let blocks = tab
                .document
                .body()
                .iter()
                .map(|block| {
                    crate::document::block_runs(block)
                        .into_iter()
                        .map(|run| match run {
                            crate::document::Inline::Text(text) => text.text.clone(),
                            crate::document::Inline::Math(_)
                            | crate::document::Inline::Note(_)
                            | crate::document::Inline::EqRef(_) => "\u{FFFC}".into(),
                        })
                        .collect::<String>()
                        .chars()
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            (tab.document.caret_position(), blocks)
        };
        let needle: Vec<char> = query.chars().collect();
        let matches: Vec<FlatPos> = blocks
            .iter()
            .enumerate()
            .flat_map(|(block, text)| {
                text.windows(needle.len())
                    .enumerate()
                    .filter(|(_, part)| *part == needle.as_slice())
                    .map(move |(offset, _)| FlatPos { block, offset })
            })
            .collect();
        if matches.is_empty() {
            return;
        }
        let pivot = if forward {
            matches.iter().position(|position| {
                (position.block, position.offset) > (current.block, current.offset)
            })
        } else {
            matches.iter().rposition(|position| {
                (position.block, position.offset) < (current.block, current.offset)
            })
        };
        let index = match pivot {
            Some(pivot) if forward => (pivot + count.max(1) - 1) % matches.len(),
            Some(pivot) => {
                (pivot + matches.len() - (count.max(1) - 1) % matches.len()) % matches.len()
            }
            None if forward => (count.max(1) - 1) % matches.len(),
            None => (matches.len() - 1).wrapping_sub(count.max(1) - 1) % matches.len(),
        };
        let position = matches[index];
        self.docs
            .borrow_mut()
            .jump_to_flat(position.block, position.offset);
    }

    fn start_insert(&mut self) {
        self.insert_repeat.clear();
        self.insert_shortcuts.reset();
        enter_insert(&mut self.docs.borrow_mut(), &mut self.vim);
    }

    fn apply_visual(&mut self, action: VisualAction) {
        match action {
            VisualAction::Enter(mode) => {
                let anchor = self
                    .docs
                    .borrow()
                    .active()
                    .map(|tab| tab.document.caret_position());
                self.visual_anchor = anchor;
                self.visual_override = None;
                self.visual_line_mode = mode == VisualMode::Line;
                if let Some(anchor) = anchor {
                    self.vim.start_visual(mode, anchor);
                }
            }
            VisualAction::Move(motion, count) => {
                self.visual_override = None;
                if matches!(motion, Motion::Up) {
                    self.vertical_motion(true, count);
                } else if matches!(motion, Motion::Down) {
                    self.vertical_motion(false, count);
                } else {
                    self.docs
                        .borrow_mut()
                        .touch(|doc| motion::apply(doc, motion, count));
                }
                self.goal_x = None;
            }
            VisualAction::TextObject(object) => {
                self.visual_override = self
                    .docs
                    .borrow()
                    .active()
                    .and_then(|tab| tab.document.text_object_range(object));
            }
            VisualAction::Operate(operator, count) => {
                let range = self.current_selection().map(|range| {
                    if self.visual_line_mode && count > 1 {
                        let docs = self.docs.borrow();
                        docs.active().map_or(range, |tab| {
                            tab.document
                                .line_range(range.start.block, range.end.block + count - 1)
                        })
                    } else {
                        range
                    }
                });
                if let Some(range) = range {
                    if operator != Operator::Yank {
                        let repeat = super::RepeatOp::Visual {
                            operator,
                            line: self.visual_line_mode,
                            range,
                        };
                        self.repeat = Some(repeat.clone());
                        if operator == Operator::Change {
                            self.insert_prefix = Some(repeat);
                        }
                    }
                    match operator {
                        Operator::Delete => {
                            if self.visual_line_mode {
                                self.docs
                                    .borrow_mut()
                                    .delete_lines(range.start.block, range.end.block);
                            } else {
                                self.docs.borrow_mut().delete_range(range);
                            }
                        }
                        Operator::Yank => self.docs.borrow_mut().yank_range(range),
                        Operator::Change => {
                            self.docs.borrow_mut().begin_transaction();
                            if self.visual_line_mode {
                                self.docs
                                    .borrow_mut()
                                    .delete_lines(range.start.block, range.end.block);
                            } else {
                                self.docs.borrow_mut().delete_range(range);
                            }
                            self.start_insert();
                        }
                    };
                }
                self.visual_anchor = None;
                self.visual_override = None;
                self.visual_line_mode = false;
                self.goal_x = None;
            }
            VisualAction::Exit => {
                self.visual_anchor = None;
                self.visual_override = None;
                self.visual_line_mode = false;
            }
        }
    }

    fn apply_operator(&mut self, operator: Operator, target: OperatorTarget, count: usize) {
        if target == OperatorTarget::Line {
            let block = self
                .docs
                .borrow()
                .active()
                .map(|tab| tab.document.caret.block);
            if let Some(block) = block {
                match operator {
                    Operator::Delete => self
                        .docs
                        .borrow_mut()
                        .delete_lines(block, block + count - 1),
                    Operator::Yank => {
                        let range = self
                            .docs
                            .borrow()
                            .active()
                            .map(|tab| tab.document.line_range(block, block + count - 1));
                        if let Some(range) = range {
                            self.docs.borrow_mut().yank_range(range);
                        }
                    }
                    Operator::Change => {
                        self.docs.borrow_mut().begin_transaction();
                        self.docs
                            .borrow_mut()
                            .delete_lines(block, block + count - 1);
                        self.insert_prefix = Some(super::RepeatOp::Operate(
                            operator,
                            OperatorTarget::Line,
                            count,
                        ));
                        self.start_insert();
                    }
                }
            }
            self.goal_x = None;
            return;
        }
        let original = self
            .docs
            .borrow()
            .active()
            .map(|tab| tab.document.caret_position());
        let Some(original) = original else { return };
        let range = match target {
            OperatorTarget::TextObject(object) => {
                let docs = self.docs.borrow();
                docs.active()
                    .and_then(|tab| tab.document.text_object_range(object))
            }
            OperatorTarget::Motion(motion) => {
                if matches!(motion, Motion::Up) {
                    self.vertical_motion(true, count);
                } else if matches!(motion, Motion::Down) {
                    self.vertical_motion(false, count);
                } else {
                    self.docs
                        .borrow_mut()
                        .touch(|doc| motion::apply(doc, motion, count));
                }
                let Some(current) = self
                    .docs
                    .borrow()
                    .active()
                    .map(|tab| tab.document.caret_position())
                else {
                    return;
                };
                let docs = self.docs.borrow();
                docs.active().map(|tab| {
                    crate::vim::Vim::operator_range(&tab.document, original, motion, current)
                })
            }
            OperatorTarget::Line => unreachable!(),
        };
        let Some(range) = range else { return };
        match operator {
            Operator::Delete => {
                self.docs.borrow_mut().delete_range(range);
            }
            Operator::Yank => {
                self.docs.borrow_mut().yank_range(range);
            }
            Operator::Change => {
                self.docs.borrow_mut().begin_transaction();
                self.docs.borrow_mut().delete_range(range);
                self.insert_prefix = Some(super::RepeatOp::Operate(operator, target, count));
                self.start_insert();
            }
        }
        self.goal_x = None;
    }

    pub(super) fn current_selection(&self) -> Option<FlatRange> {
        let docs = self.docs.borrow();
        let tab = docs.active()?;
        if let Some(range) = self.visual_override {
            return Some(range);
        }
        let anchor = self.visual_anchor?;
        let current = tab.document.caret_position();
        if self.visual_line_mode {
            return Some(tab.document.line_range(
                anchor.block.min(current.block),
                anchor.block.max(current.block),
            ));
        }
        let after = |position: FlatPos| {
            if position.offset < tab.document.block_len(position.block) {
                tab.document.position(position.block, position.offset + 1)
            } else {
                position
            }
        };
        let (start, end) = if (anchor.block, anchor.offset) <= (current.block, current.offset) {
            (anchor, after(current))
        } else {
            (current, after(anchor))
        };
        Some(FlatRange::new(start, end))
    }

    /// Publishes the yank register to the OS clipboard when it has changed.
    /// One call site covers every path that yanks — present and future —
    /// because it compares state instead of hooking each of them.
    pub(super) fn export_yank(&mut self) {
        let yank = self.docs.borrow().yank().unwrap_or_default().to_string();
        if !yank.is_empty() && yank != self.exported_yank {
            crate::clipboard::set(&yank);
            self.exported_yank = yank;
        }
    }

    /// Pulls the OS clipboard into the yank register, so `p` and Ctrl+V
    /// paste what was copied in another application. Text this app itself
    /// published is skipped — re-importing it would be a no-op that only
    /// risks losing the register to a clipboard read that failed.
    pub(super) fn import_yank(&mut self) {
        if let Some(text) = crate::clipboard::get()
            && !text.is_empty()
            && text != self.exported_yank
        {
            self.docs.borrow_mut().set_yank(text);
        }
    }

    /// Copies the selection, or the caret's whole line when there is none —
    /// the same "no selection means this line" rule the vim operators use.
    /// `cut` deletes what it copied.
    pub(super) fn copy_selection(&mut self, cut: bool) {
        let visual_active = self.vim.visual_mode().is_some();
        // Both reads are resolved into locals first: a `Ref` held in an
        // `if let` scrutinee is still alive inside the branch, and the
        // `borrow_mut()` below would panic against it.
        let line = self.docs.borrow().active().map(|tab| {
            let block = tab.document.caret.block;
            (block, tab.document.line_range(block, block))
        });
        if let Some(range) = self.current_selection() {
            if cut {
                self.docs.borrow_mut().delete_range(range);
            } else {
                self.docs.borrow_mut().yank_range(range);
            }
        } else if let Some((block, range)) = line {
            // `delete_line` does not write the yank register; `delete_lines`
            // does, and cutting has to copy what it removed.
            if cut {
                self.docs.borrow_mut().delete_lines(block, block);
            } else {
                self.docs.borrow_mut().yank_range(range);
            }
        }
        if visual_active {
            self.exit_visual();
        }
    }

    /// Pastes the OS clipboard at the caret.
    pub(super) fn paste_clipboard(&mut self) {
        self.paste(false, 1);
    }

    /// Pastes the yank register at the caret. The register holds either text
    /// this app itself published — identical to `exported_yank`, so it is
    /// re-read as notation — or text from another application, which is
    /// inserted exactly as it arrived. The provenance decision lives here:
    /// the document methods never guess where a string came from.
    fn paste(&mut self, before: bool, count: usize) {
        self.import_yank();
        paste(
            &mut self.docs.borrow_mut(),
            before,
            count,
            &self.exported_yank,
        );
    }

    pub(super) fn exit_visual(&mut self) {
        self.vim.set_mode(Mode::Normal);
        self.visual_anchor = None;
        self.visual_override = None;
        self.visual_line_mode = false;
    }

    fn repeat(&mut self) {
        let Some(operation) = self.repeat.clone() else {
            return;
        };
        self.apply_repeat(operation);
    }

    fn apply_repeat(&mut self, operation: super::RepeatOp) {
        match operation {
            super::RepeatOp::Sequence(operations) => {
                for operation in operations {
                    self.apply_repeat(operation);
                }
                if self.vim.mode() == Mode::Insert {
                    self.vim.set_mode(Mode::Normal);
                }
            }
            super::RepeatOp::Visual {
                operator,
                line,
                range: shape,
            } => {
                let current = self
                    .docs
                    .borrow()
                    .active()
                    .map(|tab| tab.document.caret_position());
                let Some(current) = current else { return };
                let range = if line {
                    let docs = self.docs.borrow();
                    docs.active().map(|tab| {
                        tab.document.line_range(
                            current.block,
                            current.block + shape.end.block - shape.start.block,
                        )
                    })
                } else {
                    let docs = self.docs.borrow();
                    docs.active().map(|tab| {
                        let block_delta = shape.end.block - shape.start.block;
                        let end = if block_delta == 0 {
                            tab.document.position(
                                current.block,
                                current.offset + shape.end.offset - shape.start.offset,
                            )
                        } else {
                            tab.document
                                .position(current.block + block_delta, shape.end.offset)
                        };
                        FlatRange::new(current, end)
                    })
                };
                let Some(range) = range else { return };
                match operator {
                    Operator::Delete => {
                        if line {
                            self.docs
                                .borrow_mut()
                                .delete_lines(range.start.block, range.end.block);
                        } else {
                            self.docs.borrow_mut().delete_range(range);
                        }
                    }
                    Operator::Change => {
                        if line {
                            self.docs
                                .borrow_mut()
                                .delete_lines(range.start.block, range.end.block);
                        } else {
                            self.docs.borrow_mut().delete_range(range);
                        }
                        self.start_insert();
                    }
                    Operator::Yank => self.docs.borrow_mut().yank_range(range),
                }
            }
            super::RepeatOp::Edit(edit, count) => self.apply_edit(edit, count),
            super::RepeatOp::Operate(operator, target, count) => {
                self.apply_operator(operator, target, count)
            }
            super::RepeatOp::Substitute(count) => {
                self.apply_edit(Edit::DeleteChar, count);
                self.start_insert();
            }
            super::RepeatOp::Paste { before, count } => {
                self.paste(before, count);
            }
            super::RepeatOp::Insert(events) => {
                self.docs.borrow_mut().transaction(|docs| {
                    for event in events {
                        apply_insert_event(docs, &event);
                    }
                });
            }
        }
    }

    /// `count` repetitions of a vertical motion against the layout, keeping
    /// the goal x that the first press measured.
    fn vertical_motion(&mut self, up: bool, count: usize) {
        let rect = self.layout.rect(self.text_column);
        let width = crate::components::editor::Editor::content_width(rect);
        let layout = self.current_layout(width);
        let layer = self.regions[self.text_region].layer();
        let measure = |text: &str, style: &TextStyle| theme::width(layer, text, style);

        let mut last: Option<crate::document::Caret> = None;
        for _ in 0..count.max(1) {
            let (next, in_table) = {
                let docs = self.docs.borrow();
                let Some(tab) = docs.active() else {
                    break;
                };
                let caret = last.unwrap_or(tab.document.caret);
                let goal = self
                    .goal_x
                    .unwrap_or_else(|| layout.caret_pos(caret, &measure).0);
                self.goal_x = Some(goal);
                let next = if up {
                    layout.line_up(caret, goal, &measure)
                } else {
                    layout.line_down(caret, goal, &measure)
                };
                (next, tab.document.in_table())
            };
            let Some(next) = next else {
                if in_table {
                    if up {
                        self.docs.borrow_mut().open_above();
                    } else {
                        self.docs.borrow_mut().open_below();
                    }
                }
                break;
            };
            last = Some(next);
            self.docs
                .borrow_mut()
                .move_caret_to(next.block, next.inline, next.offset);
        }
    }

    /// `count` repeats of one buffer edit. `o`/`O` also enter Insert — the
    /// buffer edit itself has no notion of a mode to switch.
    fn apply_edit(&mut self, edit: Edit, count: usize) {
        let count = count.max(1);
        match edit {
            Edit::DeleteChar => {
                delete_chars(&mut self.docs.borrow_mut(), count);
            }
            Edit::DeleteLine => {
                let mut docs = self.docs.borrow_mut();
                docs.transaction(|docs| {
                    for _ in 0..count {
                        docs.delete_line();
                    }
                });
            }
            Edit::OpenBelow => {
                self.docs.borrow_mut().begin_transaction();
                self.docs.borrow_mut().open_below();
                self.start_insert();
            }
            Edit::OpenAbove => {
                self.docs.borrow_mut().begin_transaction();
                self.docs.borrow_mut().open_above();
                self.start_insert();
            }
        }
        self.goal_x = None;
    }

    // ---- palette ------------------------------------------------------

    fn open_palette(&mut self) {
        self.palette = Some(PaletteState {
            query: String::new(),
            selected: 0,
        });
        self.popup_reveal.restart();
        self.refresh_palette();
    }

    fn close_palette(&mut self) {
        self.palette = None;
        self.refresh_palette();
    }

    /// Rebuilds the palette region from the live query/selection, the way
    /// `refresh_dialog` does for the dialog.
    /// Shadow layer rides every arm; a closed snapshot clears the layer so
    /// the halo leaves with the modal (see `refresh_slash_menu`).
    fn refresh_palette(&mut self) {
        let palette = match &self.palette {
            Some(state) => Palette::new(commands::entries(), state.query.clone(), state.selected),
            None => Palette::closed(),
        }
        .with_shadow(self.popup_shadow.clone());
        self.regions[self.palette_region].set_component(Box::new(palette));
    }

    fn handle_palette_input(&mut self, input: &Input) {
        if input.is_mouse_pressed(MouseButton::Left) && input.is_cursor_in_window() {
            let viewport = self.layout.rect(crate::layout::Layout::ROOT);
            let point = input.mouse_position();
            let row = self.regions[self.palette_region]
                .component_as::<Palette>()
                .and_then(|palette| palette.row_at(viewport, point));
            if let Some(row) = row {
                if let Some(state) = &mut self.palette {
                    state.selected = row;
                }
                self.run_selected_palette_command();
                return;
            }
            if !palette::card(viewport).contains(point) {
                self.close_palette();
                return;
            }
        }

        if input.is_key_pressed(KeyCode::Escape) {
            self.close_palette();
            return;
        }
        if input.is_key_pressed(KeyCode::Enter) {
            self.run_selected_palette_command();
            return;
        }
        let changed = match &mut self.palette {
            Some(state) => palette_input(state, input),
            None => false,
        };
        if changed {
            self.refresh_palette();
        }
    }

    fn run_selected_palette_command(&mut self) {
        let Some(state) = &self.palette else { return };
        let entries = commands::entries();
        let commands = commands::palette_commands();
        let visible = palette::filter(&entries, &state.query);
        let command = visible.get(state.selected).and_then(|&i| commands.get(i));
        self.close_palette();
        if let Some(command) = command {
            (command.run)(self);
        }
    }

    // ---- file finder --------------------------------------------------

    /// Closes the active tab only after a dirty document has been persisted.
    /// A failed save leaves the tab and its undo history in place.
    pub(super) fn close_active(&mut self) {
        match self.docs.borrow_mut().try_close_active() {
            Ok(_) => self.save_error = None,
            Err(error) => {
                self.save_error = Some(format!(
                    "close failed: {error} (Ctrl+S to retry; Discard changes to close)"
                ));
                eprintln!("close failed: {error}");
            }
        }
        self.rebuild_views();
    }

    pub(super) fn save_active(&mut self) {
        match self.docs.borrow_mut().save_active() {
            Ok(()) => self.save_error = None,
            Err(error) => {
                self.save_error = Some(format!("save failed: {error}"));
                eprintln!("save failed: {error}");
            }
        }
        self.rebuild_views();
    }

    /// Explicitly drops the active tab without attempting persistence. This
    /// is the recovery route when a close/save error cannot be repaired.
    pub(super) fn discard_active(&mut self) {
        let index = self.docs.borrow().active_index();
        if let Some(index) = index {
            self.docs.borrow_mut().close(index);
            self.save_error = None;
            self.rebuild_views();
        }
    }

    pub(super) fn open_finder(&mut self) {
        let Some(vault) = &self.vault else {
            return;
        };
        self.popup_reveal.restart();
        let files = vault.borrow().files();
        let index = crate::search::SearchIndex::build(&files);
        self.finder = Some(super::FinderState {
            rows: index.search(""),
            index,
            query: String::new(),
            selected: 0,
        });
        self.refresh_finder();
    }

    fn close_finder(&mut self) {
        self.finder = None;
        self.refresh_finder();
    }

    /// Re-ranks the rows against the current query. The shell owns the
    /// ranked list (the component only draws a snapshot); the index was built
    /// when the finder opened, so this path is memory-only.
    fn rerank_finder(&mut self) {
        let Some(state) = self.finder.as_mut() else {
            return;
        };
        state.rows = state.index.search(&state.query);
        state.selected = state.selected.min(state.rows.len().saturating_sub(1));
    }

    fn refresh_finder(&mut self) {
        let query = self.finder.as_ref().map(|state| state.query.clone());
        self.regions[self.title_region].set_component(Box::new(title_bar::TitleBar::new(query)));
        let finder = match &self.finder {
            Some(state) => Finder::new(state.rows.clone(), state.query.clone(), state.selected),
            None => Finder::closed(),
        }
        .with_shadow(self.popup_shadow.clone());
        self.regions[self.finder_region].set_component(Box::new(finder));
    }

    /// Picks one of the first five rows outright (Ctrl+1..5). The finder
    /// is the only consumer of these chords: the view toggles that used to
    /// hold them are gone.
    fn finder_row_picked(&mut self, input: &Input) -> bool {
        if !input.ctrl() {
            return false;
        }
        const PICK_KEYS: [KeyCode; 5] = [
            KeyCode::Digit1,
            KeyCode::Digit2,
            KeyCode::Digit3,
            KeyCode::Digit4,
            KeyCode::Digit5,
        ];
        let Some(row) = PICK_KEYS.iter().position(|key| input.is_key_typed(*key)) else {
            return false;
        };
        if let Some(state) = &mut self.finder {
            state.selected = row.min(state.rows.len().saturating_sub(1));
            self.open_selected_finder_file();
        }
        true
    }

    fn handle_finder_input(&mut self, input: &Input) {
        if input.is_mouse_pressed(MouseButton::Left) && input.is_cursor_in_window() {
            let viewport = self.layout.rect(crate::layout::Layout::ROOT);
            let point = input.mouse_position();
            let row = self.regions[self.finder_region]
                .component_as::<Finder>()
                .and_then(|finder| finder.row_at(viewport, point));
            if let Some(row) = row {
                if let Some(state) = &mut self.finder {
                    state.selected = row;
                }
                self.open_selected_finder_file();
                return;
            }
            if !crate::components::search::card(viewport).contains(point) {
                self.close_finder();
                return;
            }
        }
        if input.is_key_pressed(KeyCode::Escape) {
            self.close_finder();
            return;
        }
        if self.finder_row_picked(input) {
            return;
        }
        if input.is_key_typed(KeyCode::Enter) {
            self.open_selected_finder_file();
            return;
        }
        let changed = match &mut self.finder {
            Some(state) => finder_input(state, input),
            None => false,
        };
        if changed {
            self.rerank_finder();
            self.refresh_finder();
        }
    }

    fn open_selected_finder_file(&mut self) {
        let picked = self
            .finder
            .as_ref()
            .and_then(|state| state.rows.get(state.selected).cloned());
        let Some(row) = picked else {
            return;
        };
        {
            let mut docs = self.docs.borrow_mut();
            docs.open_preview(row.path());
            if !docs.active().is_some_and(|tab| tab.path() == row.path()) {
                return;
            }
            docs.tree_selected = Some(row.path().to_path_buf());
            if let Some((block, offset)) = row.position() {
                docs.jump_to_flat(block, offset);
            }
        }
        if let Some(vault) = &self.vault {
            vault.borrow_mut().reveal(row.path());
            self.regions[self.tree_region].poke();
        }
        self.close_finder();
    }

    // ---- slash menu -----------------------------------------------------

    /// The caret's screen position, used as the slash-menu anchor. This
    /// is the same coordinate math the editor's `draw()` uses, so the menu
    /// opens right next to the text caret.
    fn compute_slash_anchor(&mut self) -> (f32, f32) {
        let rect = self.layout.rect(self.text_column);
        let width = crate::components::editor::Editor::content_width(rect);
        let layout = self.current_layout(width);
        let layer = self.regions[self.text_region].layer();
        let measure = |text: &str, style: &TextStyle| theme::width(layer, text, style);
        let (caret_x, caret_baseline, caret_height, scroll) = {
            let docs = self.docs.borrow();
            let Some(tab) = docs.active() else {
                return (rect.x, rect.y);
            };
            let (cx, cb, ch) = layout.caret_pos(tab.document.caret, &measure);
            (cx, cb, ch, docs.editor_scroll)
        };
        let screen_x = Editor::content_x(rect) + caret_x;
        let screen_y =
            rect.y + crate::components::editor::TOP + caret_baseline - scroll + caret_height / 2.0;
        (screen_x, screen_y)
    }

    /// Every arm carries the blurred shadow layer — closed ghosts included:
    /// `paint_shadow` clears it before anything else, so the snapshot that
    /// closes the menu is also the one that takes the halo off the screen.
    pub(super) fn refresh_slash_menu(&mut self) {
        let menu = match &self.slash_menu {
            Some(state) => SlashMenu::new(
                commands::editor_entries(),
                state.query.clone(),
                state.selected,
                state.anchor,
            ),
            None => match &self.menu_dismiss {
                Some(MenuDismiss::Slash {
                    state,
                    anchor,
                    pointer_row,
                }) => SlashMenu::dismissing(state, *anchor, *pointer_row),
                _ => SlashMenu::closed(),
            },
        }
        .with_shadow(self.popup_shadow.clone());
        self.regions[self.slash_region].set_component(Box::new(menu));
    }

    // ---- context menu ----------------------------------------------------

    fn context_ids(&self, target: &ContextHit) -> Vec<&'static str> {
        match target {
            ContextHit::Range { kind, .. } => match kind {
                RangeKind::Word => commands::WORD_MENU.to_vec(),
                RangeKind::Badge => commands::BADGE_MENU.to_vec(),
                RangeKind::InlineCode => commands::INLINE_CODE_MENU.to_vec(),
                RangeKind::CodeBlock => commands::CODE_BLOCK_MENU.to_vec(),
                RangeKind::CodeWord => commands::CODE_COLOR_MENU.to_vec(),
            },
            ContextHit::Math {
                block,
                inline,
                offset,
                node: Some(address),
            } => {
                let docs = self.docs.borrow();
                let node = docs
                    .active()
                    .and_then(|tab| tab.document.math_node_at(*block, *inline, *offset, address));
                match node {
                    Some(MathNode::Sym(ch)) if ch.is_alphabetic() => symbol_context_ids(*ch),
                    Some(MathNode::Resolved { variant, body, .. }) => {
                        // A multi-letter identity like `sin` has no
                        // mathematical-alphanumeric variants, but role and
                        // styling still apply to it.
                        symbol_base_glyph(body, variant)
                            .map(symbol_context_ids)
                            .unwrap_or_else(symbol_style_ids)
                    }
                    Some(MathNode::Group { .. }) => commands::GROUP_MENU.to_vec(),
                    Some(MathNode::Accent { .. }) => commands::ACCENT_MENU.to_vec(),
                    Some(MathNode::BigOp { .. }) => commands::BIG_OP_MENU.to_vec(),
                    _ => Vec::new(),
                }
            }
            ContextHit::Math { node: None, .. } => Vec::new(),
        }
    }

    /// Which document targets the open formatting surface — the classic
    /// context menu or the word-format bar — is operating on. Both keep
    /// them in the shell, so the two surfaces cannot disagree about what a
    /// toggle applies to. The menu always names exactly one; the bar names
    /// as many as the brush swept up.
    fn context_targets(&self) -> Vec<ContextHit> {
        if let Some(state) = &self.context_menu {
            return state.target.clone().into_iter().collect();
        }
        match &self.format_bar {
            Some(state) => state.targets.clone(),
            None => Vec::new(),
        }
    }

    fn context_target(&self) -> Option<ContextHit> {
        self.context_targets().into_iter().next()
    }

    fn context_range(&self) -> Option<FlatRange> {
        match self.context_target() {
            Some(ContextHit::Range { range, .. }) => Some(range),
            _ => None,
        }
    }

    /// The prose ranges a text style lands on, and how many targets it has
    /// to leave alone.
    ///
    /// One target is styled whatever its kind: the surface over it offers
    /// only what that kind can take, and un-badging a badge or un-coding an
    /// inline-code run are its own rows. A brush selection is several
    /// things at once, and a text style reaches only the words in it —
    /// math, code and boxed runs are counted instead, for the warning that
    /// goes up before anything changes.
    fn format_ranges(&self) -> (Vec<FlatRange>, usize) {
        style_targets(&self.context_targets())
    }

    fn context_math_target(&self) -> Option<(usize, usize, usize, NodeAddress)> {
        match self.context_target() {
            Some(ContextHit::Math {
                block,
                inline,
                offset,
                node: Some(address),
            }) => Some((block, inline, offset, address.clone())),
            _ => None,
        }
    }

    fn context_command_checked(&self, id: &str) -> bool {
        let docs = self.docs.borrow();
        let Some(document) = docs.active().map(|tab| &tab.document) else {
            return false;
        };
        match self.context_targets().as_slice() {
            // One target answers for itself, whatever kind it is: a math
            // node's card asks about roles and variants, not about bold.
            [target] => Self::context_command_checked_for(document, target.clone(), id),
            // A brush selection's ring describes what the button would
            // change, so it reads the words the toggle reaches and nothing
            // else — lit only when every one of them already carries the
            // format, which is what makes one press mean one thing.
            _ => {
                let (ranges, _) = self.format_ranges();
                !ranges.is_empty()
                    && ranges.iter().all(|range| {
                        Self::context_command_checked_for(
                            document,
                            ContextHit::Range {
                                range: *range,
                                kind: RangeKind::Word,
                            },
                            id,
                        )
                    })
            }
        }
    }

    fn context_command_checked_for(document: &Document, target: ContextHit, id: &str) -> bool {
        match target {
            ContextHit::Range { range, .. } => {
                if let Some(choice) = id.strip_prefix("context.code.") {
                    let block = &document.body()[range.start.block];
                    let syntax = document.code_style_at(range.start).unwrap_or_default();
                    if let Some(color) = choice.strip_prefix("color.") {
                        return syntax.manual && syntax.color == MathHue::from_keyword(color);
                    }
                    let language = syntax.language.or_else(|| match block {
                        Block::CodeLine { lang, .. } => lang
                            .as_deref()
                            .and_then(crate::document::code::Language::from_name),
                        _ => None,
                    });
                    return match choice {
                        "manual" => syntax.manual,
                        "plain" => !syntax.manual && language.is_none(),
                        _ => !syntax.manual && language.is_some_and(|lang| lang.name() == choice),
                    };
                }
                let mask = match id {
                    "context.bold" => Style {
                        bold: true,
                        ..Style::PLAIN
                    },
                    "context.italic" => Style {
                        italic: true,
                        ..Style::PLAIN
                    },
                    "context.highlight" => Style {
                        highlight: true,
                        ..Style::PLAIN
                    },
                    "context.inline_code" => Style {
                        code: true,
                        ..Style::PLAIN
                    },
                    "context.badge" => Style {
                        badge: true,
                        ..Style::PLAIN
                    },
                    _ => {
                        return match id {
                            "context.badge.orange" => {
                                document.badge_color_is_active(range, BadgeColor::Orange)
                            }
                            "context.badge.blue" => {
                                document.badge_color_is_active(range, BadgeColor::Blue)
                            }
                            "context.badge.green" => {
                                document.badge_color_is_active(range, BadgeColor::Green)
                            }
                            "context.badge.purple" => {
                                document.badge_color_is_active(range, BadgeColor::Purple)
                            }
                            _ => false,
                        };
                    }
                };
                document.style_range_is_active(range, mask)
            }
            ContextHit::Math {
                block,
                inline,
                offset,
                node: Some(address),
            } => {
                let node = document.math_node_at(block, inline, offset, &address);
                // Hue and shape are not stored on the node — they are the
                // reader's own vocabulary, kept in the vault config — so
                // they are answered from the style server rather than from
                // the tree, before the arms that read the tree.
                if let Some(checked) = node.and_then(|node| symbol_style_checked(id, node)) {
                    return checked;
                }
                match (id, node) {
                    ("context.symbol.variable", Some(MathNode::Resolved { role, .. })) => {
                        *role == SymbolRole::Variable
                    }
                    ("context.symbol.constant", Some(MathNode::Resolved { role, .. })) => {
                        *role == SymbolRole::Constant
                    }
                    ("context.symbol.function", Some(MathNode::Resolved { role, .. })) => {
                        *role == SymbolRole::Function
                    }
                    // A bare letter is a variable nobody has said anything
                    // else about — which is a role, and the card should
                    // show it as the one the symbol already stands on.
                    ("context.symbol.variable", Some(MathNode::Sym(ch))) => ch.is_alphabetic(),
                    ("context.variant.plain", Some(MathNode::Sym(ch))) => ch.is_alphabetic(),
                    ("context.variant.plain", Some(MathNode::Resolved { variant, .. })) => {
                        variant == "plain"
                    }
                    ("context.variant.bold", Some(MathNode::Resolved { variant, .. })) => {
                        variant == "bold"
                    }
                    ("context.variant.italic", Some(MathNode::Resolved { variant, .. })) => {
                        variant == "italic"
                    }
                    ("context.variant.bold_italic", Some(MathNode::Resolved { variant, .. })) => {
                        variant == "bold_italic"
                    }
                    ("context.variant.sans", Some(MathNode::Resolved { variant, .. })) => {
                        variant == "sans"
                    }
                    ("context.variant.sans_bold", Some(MathNode::Resolved { variant, .. })) => {
                        variant == "sans_bold"
                    }
                    ("context.variant.sans_italic", Some(MathNode::Resolved { variant, .. })) => {
                        variant == "sans_italic"
                    }
                    (
                        "context.variant.sans_bold_italic",
                        Some(MathNode::Resolved { variant, .. }),
                    ) => variant == "sans_bold_italic",
                    ("context.variant.monospace", Some(MathNode::Resolved { variant, .. })) => {
                        variant == "monospace"
                    }
                    ("context.group.parentheses", Some(MathNode::Group { open, .. })) => {
                        *open == '('
                    }
                    ("context.group.brackets", Some(MathNode::Group { open, .. })) => *open == '[',
                    ("context.group.bars", Some(MathNode::Group { open, .. })) => *open == '|',
                    ("context.group.double_bars", Some(MathNode::Group { open, .. })) => {
                        *open == '‖'
                    }
                    ("context.group.angles", Some(MathNode::Group { open, .. })) => *open == '⟨',
                    ("context.accent.vector", Some(MathNode::Accent { kind, .. })) => {
                        *kind == AccentKind::Vector
                    }
                    ("context.accent.dot", Some(MathNode::Accent { kind, .. })) => {
                        *kind == AccentKind::Dot
                    }
                    ("context.accent.ddot", Some(MathNode::Accent { kind, .. })) => {
                        *kind == AccentKind::DoubleDot
                    }
                    ("context.accent.dddot", Some(MathNode::Accent { kind, .. })) => {
                        *kind == AccentKind::TripleDot
                    }
                    ("context.accent.hat", Some(MathNode::Accent { kind, .. })) => {
                        *kind == AccentKind::Hat
                    }
                    ("context.accent.bar", Some(MathNode::Accent { kind, .. })) => {
                        *kind == AccentKind::Bar
                    }
                    ("context.op.sum", Some(MathNode::BigOp { kind, .. })) => *kind == BigOp::Sum,
                    ("context.op.product", Some(MathNode::BigOp { kind, .. })) => {
                        *kind == BigOp::Prod
                    }
                    ("context.op.integral", Some(MathNode::BigOp { kind, .. })) => {
                        *kind == BigOp::Integral
                    }
                    ("context.op.ring_integral", Some(MathNode::BigOp { kind, .. })) => {
                        *kind == BigOp::ContourIntegral
                    }
                    ("context.op.limit", Some(MathNode::BigOp { kind, .. })) => {
                        *kind == BigOp::Limit
                    }
                    _ => false,
                }
            }
            ContextHit::Math { node: None, .. } => false,
        }
    }

    /// Applies one text style to every range the open surface holds, as a
    /// single undo step.
    ///
    /// A selection where some words already carry the style and some do not
    /// is made uniform rather than inverted word by word: one press turns
    /// the style on unless every range already has it. Toggling each range
    /// against its own state would leave the reader looking at a selection
    /// half of which went bold and half of which went plain, from one press
    /// of one button.
    fn context_toggle_style(&mut self, mask: Style) {
        let (ranges, _) = self.format_ranges();
        apply_style_to_ranges(&mut self.docs.borrow_mut(), &ranges, mask);
    }

    pub(super) fn context_toggle_bold(&mut self) {
        self.context_toggle_style(Style {
            bold: true,
            ..Style::PLAIN
        });
    }

    pub(super) fn context_toggle_italic(&mut self) {
        self.context_toggle_style(Style {
            italic: true,
            ..Style::PLAIN
        });
    }

    pub(super) fn context_toggle_highlight(&mut self) {
        self.context_toggle_style(Style {
            highlight: true,
            ..Style::PLAIN
        });
    }

    pub(super) fn context_toggle_inline_code(&mut self) {
        self.context_toggle_style(Style {
            code: true,
            ..Style::PLAIN
        });
    }

    pub(super) fn context_toggle_badge(&mut self) {
        self.context_toggle_style(Style {
            badge: true,
            ..Style::PLAIN
        });
    }

    pub(super) fn context_code_settings(&mut self) {
        if let Some(state) = &self.context_menu {
            let target = state.target.clone();
            let anchor = state.anchor;
            let is_block = target
                .as_ref()
                .and_then(|hit| match hit {
                    ContextHit::Range { range, .. } => Some(range.start.block),
                    _ => None,
                })
                .is_some_and(|index| {
                    self.docs
                        .borrow()
                        .active()
                        .is_some_and(|tab| tab.document.body()[index].is_code())
                });
            self.open_context_target(
                if is_block {
                    commands::CODE_BLOCK_MENU
                } else {
                    commands::INLINE_CODE_MENU
                },
                anchor,
                target,
            );
        }
    }

    pub(super) fn context_code_options(
        &mut self,
        language: Option<crate::document::code::Language>,
        manual: bool,
    ) {
        if let Some(range) = self.context_range() {
            self.docs
                .borrow_mut()
                .set_code_options(range, language, manual);
        }
        self.brush_selected.clear();
        self.brush_revision = self.brush_revision.wrapping_add(1);
    }

    pub(super) fn context_code_color(&mut self, color: Option<MathHue>) {
        let mut ranges: Vec<_> = self
            .brush_selected
            .iter()
            .filter_map(|hit| match hit {
                ContextHit::Range {
                    range,
                    kind: RangeKind::CodeWord,
                } => Some(*range),
                _ => None,
            })
            .collect();
        if ranges.is_empty()
            && let Some(range) = self.context_range()
        {
            ranges.push(range);
        }
        self.docs.borrow_mut().transaction(|docs| {
            for range in ranges {
                docs.set_code_color(range, color);
            }
        });
        self.brush_selected.clear();
        self.brush_revision = self.brush_revision.wrapping_add(1);
    }

    pub(super) fn context_set_badge_color(&mut self, color: BadgeColor) {
        if let Some(range) = self.context_range() {
            self.docs.borrow_mut().set_badge_color(range, color);
        }
    }

    pub(super) fn context_set_heading(&mut self, level: Option<u8>) {
        if let Some(range) = self.context_range() {
            let range = range.normalized();
            self.docs.borrow_mut().transaction(|docs| {
                for block in range.start.block..=range.end.block {
                    docs.set_block_heading_at(block, level);
                }
            });
        }
    }

    pub(super) fn context_set_math_role(&mut self, role: SymbolRole) {
        if let Some((block, inline, offset, address)) = self.context_math_target() {
            self.docs
                .borrow_mut()
                .set_math_node_role_at(block, inline, offset, &address, role);
        }
    }

    pub(super) fn context_set_math_variant(&mut self, variant: &str) {
        if let Some((block, inline, offset, address)) = self.context_math_target() {
            self.docs
                .borrow_mut()
                .set_math_node_variant_at(block, inline, offset, &address, variant);
        }
    }

    pub(super) fn context_set_math_delimiter(&mut self, open: char) {
        if let Some((block, inline, offset, address)) = self.context_math_target() {
            self.docs
                .borrow_mut()
                .set_math_group_delimiter_at(block, inline, offset, &address, open);
        }
    }

    pub(super) fn context_set_math_accent(&mut self, kind: AccentKind) {
        if let Some((block, inline, offset, address)) = self.context_math_target() {
            self.docs
                .borrow_mut()
                .set_math_accent_kind_at(block, inline, offset, &address, kind);
        }
    }

    pub(super) fn context_set_math_big_op(&mut self, kind: BigOp) {
        if let Some((block, inline, offset, address)) = self.context_math_target() {
            self.docs
                .borrow_mut()
                .set_math_big_op_kind_at(block, inline, offset, &address, kind);
        }
    }

    /// The node the open formatting surface points at, if it is a math one.
    fn context_math_node(&self) -> Option<MathNode> {
        let (block, inline, offset, address) = self.context_math_target()?;
        let docs = self.docs.borrow();
        docs.active()
            .and_then(|tab| tab.document.math_node_at(block, inline, offset, &address))
            .cloned()
    }

    /// The role and identity of the symbol under the open menu — the key its
    /// styling is stored against.
    ///
    /// A bare letter has no `Resolved` wrapper and so no stored role; it is
    /// a variable nobody has said anything else about, and it is keyed by
    /// the letter itself. That is the same key math layout styles it under,
    /// so recolouring an unresolved `x` reaches every `x` in the vault.
    pub(super) fn context_symbol(&self) -> Option<(SymbolRole, String)> {
        match self.context_math_node()? {
            MathNode::Sym(ch) if ch.is_alphabetic() => Some((SymbolRole::Variable, ch.to_string())),
            MathNode::Resolved { id, role, .. } => Some((role, id)),
            _ => None,
        }
    }

    pub(super) fn context_set_math_hue(&mut self, hue: MathHue) {
        self.edit_symbol_style(|edit| edit.hue = Some(hue));
    }

    pub(super) fn context_set_math_shape(&mut self, shape: HighlightShape) {
        self.edit_symbol_style(|edit| edit.shape = Some(shape));
    }

    /// Puts a symbol back on its automatic look — the hue its letter falls
    /// on and the shape its role asks for.
    pub(super) fn context_reset_math_style(&mut self) {
        self.edit_symbol_style(|edit| *edit = math_style::Override::default());
    }

    /// Applies `edit` to the targeted symbol's override, installs the result,
    /// and writes it to the vault config.
    ///
    /// Persisting here rather than on exit is deliberate: this is a
    /// preference the reader set by hand, and a crash between setting it and
    /// closing the window should not quietly take it back.
    fn edit_symbol_style(&mut self, edit: impl FnOnce(&mut math_style::Override)) {
        let Some((_, id)) = self.context_symbol() else {
            return;
        };
        let mut overrides = math_style::overrides();
        let mut entry = overrides.get(&id);
        edit(&mut entry);
        overrides.set(&id, entry);
        if !math_style::install(overrides.clone()) {
            return;
        }
        if let Some(config) = &mut self.config {
            config.math = overrides;
            if let Err(e) = config.save() {
                eprintln!("could not save symbol styling: {e}");
            }
        }
        self.refresh_context_menu();
    }

    fn open_context_menu(&mut self, ids: &[&str], anchor: (f32, f32)) {
        self.open_context_target(ids, anchor, None);
    }

    fn open_context_target(
        &mut self,
        ids: &[&str],
        anchor: (f32, f32),
        target: Option<crate::document::layout::ContextHit>,
    ) {
        self.format_bar = None;
        // An in-flight fade-out of THIS menu is superseded: it is back.
        if matches!(self.menu_dismiss, Some(MenuDismiss::Context { .. })) {
            self.menu_dismiss = None;
            self.menu_dismiss_clock = 0.0;
        }
        let items = commands::menu(ids);
        if items.is_empty() {
            return;
        }
        self.context_menu = Some(ContextMenuState {
            items,
            selected: 0,
            anchor,
            target,
        });
        self.popup_reveal.restart();
        self.refresh_context_menu();
        self.rebuild_views();
    }

    /// Shadow layer rides every arm — ghosts included — so a closing menu
    /// always takes its halo away (see `refresh_slash_menu`).
    ///
    /// One region, two faces. Over a symbol the card is the inspector; over
    /// anything else it is the command list. Which one is not stored state:
    /// it follows from the target, and the sections are rebuilt on every
    /// refresh so a colour picked from the card is on the card by the time
    /// the click finishes.
    pub(super) fn refresh_context_menu(&mut self) {
        if let Some(menu) = self.symbol_menu_component() {
            self.regions[self.menu_region]
                .set_component(Box::new(menu.with_shadow(self.popup_shadow.clone())));
            return;
        }
        let menu = match &self.context_menu {
            Some(state) => {
                let ids: Vec<&str> = state.items.iter().map(|command| command.id).collect();
                let checked: Vec<bool> = state
                    .items
                    .iter()
                    .map(|command| self.context_command_checked(command.id))
                    .collect();
                ContextMenu::new(
                    commands::menu_entries(&ids),
                    checked,
                    state.selected,
                    state.anchor,
                )
            }
            None => match &self.menu_dismiss {
                Some(MenuDismiss::Context {
                    state: ContextGhost::Commands(state),
                    anchor,
                    pill_row,
                }) => ContextMenu::dismissing(state, *anchor, *pill_row),
                _ => ContextMenu::closed(),
            },
        }
        .with_shadow(self.popup_shadow.clone());
        self.regions[self.menu_region].set_component(Box::new(menu));
    }

    /// The head and sections the inspector draws, or `None` when the open
    /// menu is not over a symbol.
    ///
    /// Every cell shows its own outcome rather than naming it, so each one
    /// is built from the style `resolve` *would* return after that choice —
    /// not from the choice's own defaults. That matters once the reader has
    /// pinned a shape by hand: picking a different role then leaves the
    /// shape alone, and the role rows have to say so.
    fn symbol_menu_content(&self) -> Option<(symbol_menu::Head, Vec<symbol_menu::Section>)> {
        let state = self.context_menu.as_ref()?;
        let (role, id) = self.context_symbol()?;
        let glyph = self.context_symbol_glyph()?;
        let current = math_style::resolve(role, &id);
        let pinned = math_style::overrides().get(&id);
        let automatic = math_style::automatic(role, &id);

        let mut sections: Vec<symbol_menu::Section> = Vec::new();
        for command in &state.items {
            let art = symbol_art(command.id, current, pinned, automatic);
            let cell = symbol_menu::Cell {
                label: command.title.to_string(),
                glyph: match art {
                    symbol_menu::Art::Bare => variant_glyph(command.id, &id, &glyph),
                    symbol_menu::Art::Preview(_) => glyph.clone(),
                },
                art,
                checked: self.context_command_checked(command.id),
            };
            match sections.last_mut() {
                Some(section) if section.title == command.group => section.cells.push(cell),
                _ => sections.push(symbol_menu::Section {
                    title: command.group.to_string(),
                    shelf: shelf_for(command.group),
                    cells: vec![cell],
                }),
            }
        }

        let head = symbol_menu::Head {
            glyph,
            name: id,
            role: format!(
                "{} \u{00b7} {} {}",
                role_label(role),
                current.hue.label(),
                current.shape.label().to_lowercase()
            ),
            style: current,
        };
        Some((head, sections))
    }

    /// The snapshot a closing inspector leaves behind, and the cell its pill
    /// had parked on.
    fn symbol_menu_snapshot(&self) -> Option<(symbol_menu::Snapshot, Option<usize>)> {
        let component = self.regions[self.menu_region].component_as::<SymbolMenu>();
        let (head, sections) = self.symbol_menu_content()?;
        let selected = self.context_menu.as_ref().map_or(0, |state| state.selected);
        Some((
            symbol_menu::Snapshot {
                head,
                sections,
                selected,
            },
            component.and_then(|menu| menu.pill_cell()),
        ))
    }

    /// The characters the symbol is actually set with — the current variant's
    /// letterform, not the identity it stands for, so a bold `x` previews as
    /// a bold `x`.
    fn context_symbol_glyph(&self) -> Option<String> {
        match self.context_math_node()? {
            MathNode::Sym(ch) if ch.is_alphabetic() => Some(ch.to_string()),
            MathNode::Resolved { body, .. } => Some(
                body.iter()
                    .filter_map(|node| match node {
                        MathNode::Sym(ch) => Some(*ch),
                        _ => None,
                    })
                    .collect(),
            ),
            _ => None,
        }
    }

    /// The inspector for this menu, or `None` when the menu is not over a
    /// symbol — in which case the command list takes the region instead.
    fn symbol_menu_component(&self) -> Option<SymbolMenu> {
        let standing = self.regions[self.menu_region]
            .component_as::<SymbolMenu>()
            .and_then(SymbolMenu::resting);
        if let Some(state) = &self.context_menu {
            let (head, sections) = self.symbol_menu_content()?;
            return Some(
                SymbolMenu::new(head, sections, state.selected, state.anchor).resuming(standing),
            );
        }
        match &self.menu_dismiss {
            Some(MenuDismiss::Context {
                state: ContextGhost::Symbol(state),
                anchor,
                pill_row,
            }) => Some(SymbolMenu::dismissing(state, *anchor, *pill_row)),
            _ => None,
        }
    }

    fn close_context_menu(&mut self) {
        // Ghost fodder comes from the LIVE menu (rows with current checks +
        // the pill's row) — read before `take` empties it.
        let symbol = self.symbol_menu_snapshot();
        let ghost = self.context_menu.take().map(|state| {
            let (snap, pill_row) = match symbol {
                Some((snap, pill)) => {
                    let fallback = state.selected;
                    (ContextGhost::Symbol(snap), pill.unwrap_or(fallback))
                }
                None => {
                    let ids: Vec<&str> = state.items.iter().map(|command| command.id).collect();
                    let checked: Vec<bool> = state
                        .items
                        .iter()
                        .map(|command| self.context_command_checked(command.id))
                        .collect();
                    let entries = commands::menu_entries(&ids);
                    let snap = crate::components::context_menu::Snapshot { entries, checked };
                    let pill_row = self.regions[self.menu_region]
                        .component_as::<ContextMenu>()
                        .and_then(|m| m.pill_row())
                        .unwrap_or(state.selected.min(snap.entries.len().saturating_sub(1)));
                    (ContextGhost::Commands(snap), pill_row)
                }
            };
            MenuDismiss::Context {
                state: snap,
                anchor: state.anchor,
                pill_row,
            }
        });
        // Zero-row menus have no ghost to show.
        let spawn = matches!(
            &ghost,
            Some(MenuDismiss::Context { state, .. }) if !state.is_empty()
        );
        if spawn {
            self.menu_dismiss = ghost;
            self.menu_dismiss_clock = 1.0;
        } else {
            self.menu_dismiss = None;
            self.menu_dismiss_clock = 0.0;
        }
        self.refresh_context_menu();
        self.rebuild_views();
    }

    // ---- word-format bar ---------------------------------------------------

    /// Opens the format bar over `targets` at `anchor`: one word for a
    /// click, or a whole Ctrl-brush selection for a stroke. The bar keeps
    /// its targets so toggles keep applying to the same words while it
    /// stays open — the whole point of a toolbar over a one-shot menu.
    ///
    /// The bar is the *word* format bar, so it takes its rows from the
    /// first word in the selection; a stroke that swept up math or code
    /// as well still shows what those words can take, and the warning at
    /// [`Shell::warn_mixed_format`] handles the rest.
    fn open_format_bar(&mut self, targets: Vec<ContextHit>, anchor: (f32, f32)) {
        let Some(word) = targets.iter().find(|target| {
            matches!(
                target,
                ContextHit::Range {
                    kind: RangeKind::Word,
                    ..
                }
            )
        }) else {
            return;
        };
        let ids = self.context_ids(word);
        let items = commands::menu(&ids);
        if items.is_empty() {
            return;
        }
        // The two surfaces are mutually exclusive; the click that opened
        // this bar could not have left the menu claimed, so clear the field
        // directly rather than through the menu's rebuild path.
        self.context_menu = None;
        // An in-flight fade-out is superseded: the bar is back.
        if matches!(self.menu_dismiss, Some(MenuDismiss::Format { .. })) {
            self.menu_dismiss = None;
            self.menu_dismiss_clock = 0.0;
        }
        self.format_bar = Some(WordFormatState {
            items,
            selected: 0,
            anchor,
            targets,
            hover_cell: None,
            mixed_ack: false,
        });
        // A fresh pop every time the bar opens, driven from the shell so
        // the per-toggle refresh below never replays it.
        self.popup_reveal.restart();
        self.refresh_format_bar();
        self.rebuild_views();
    }

    /// Rebuilds the drawn snapshot from the live shell state — the checked
    /// flags come from the document, so a toggle lands on the next frame.
    pub(super) fn refresh_format_bar(&mut self) {
        // Every arm carries the blurred shadow layer, the closed ones
        // included: `paint_shadow` clears it before anything else, so the
        // snapshot that closes the bar is also the one that takes the
        // halo off the screen. A closed bar without the layer would leave
        // the last slab frozen in the Manual-mode blur forever.
        //
        // While a dismissal is in flight the region shows the ghost: dead
        // placeholder cells at the bar's last geometry, whose reveal weight
        // the shell is dropping back toward 0 — the fade-out.
        let view = match &self.format_bar {
            Some(state) => {
                let items = self.format_bar_geometry();
                if items.is_empty() {
                    FormatBar::closed()
                } else {
                    let mut bar = FormatBar::open(items, state.selected, state.anchor);
                    bar.set_pointer_cell(state.hover_cell);
                    bar
                }
            }
            None => match &self.menu_dismiss {
                Some(MenuDismiss::Format {
                    items,
                    anchor,
                    pointer_cell,
                }) => FormatBar::dismissing(items.clone(), *anchor, *pointer_cell),
                _ => FormatBar::closed(),
            },
        }
        .with_shadow(self.popup_shadow.clone());
        self.regions[self.format_region].set_component(Box::new(view));
        // The shell owns the pill memory across refreshes: ask the fresh
        // snapshot where its pill is (it may have been seeded, and its
        // first sync may already have moved it) and keep that.
        if let Some(state) = &mut self.format_bar {
            state.hover_cell = self.regions[self.format_region]
                .component_as::<FormatBar>()
                .and_then(|bar| bar.pointer_cell());
        }
    }

    fn close_format_bar(&mut self) {
        // The ghost's shape comes from the live state, so read the geometry
        // before `take` empties it — afterwards `format_bar_geometry` sees
        // `None` and would report an empty bar, killing every fade-out.
        let ghost_items = self.format_bar_geometry();
        let Some(state) = self.format_bar.take() else {
            return;
        };
        // Keep a ghost alive for the fade-out: the same item count and
        // anchor, drawn from a falling clock. Nothing accepts input while
        // it fades — `format_bar` is already `None`, so input routing sees
        // a closed bar from this frame on.
        if !ghost_items.is_empty() {
            // The ghost's pill parks on the keyboard selection — the same
            // place a fresh open starts — and freezes there for the fade.
            let last = ghost_items.len() - 1;
            self.menu_dismiss = Some(MenuDismiss::Format {
                items: ghost_items,
                anchor: state.anchor,
                pointer_cell: Some(state.selected.min(last)),
            });
            self.menu_dismiss_clock = 1.0;
        } else {
            self.menu_dismiss = None;
            self.menu_dismiss_clock = 0.0;
        }
        self.refresh_format_bar();
        self.rebuild_views();
    }

    /// The bar's affordances as the drawing and hit-testing see them, with
    /// each one's live checked state read off the document. A `Dismiss`
    /// cell is appended last — it is the bar's own close affordance, not a
    /// format command, so it maps to no `format_bar::from_id`.
    fn format_bar_geometry(&self) -> Vec<format_bar::Item> {
        let mut items = Vec::new();
        if let Some(state) = &self.format_bar {
            for command in &state.items {
                if let Some(kind) = format_bar::from_id(command.id) {
                    items.push(format_bar::Item {
                        kind,
                        checked: self.context_command_checked(command.id),
                    });
                }
            }
            items.push(format_bar::Item {
                kind: format_bar::Kind::Dismiss,
                checked: false,
            });
        }
        items
    }

    /// Runs the bar cell's command, keeping the bar open so a word can take
    /// several formats in one pass; the refreshed snapshot shows the new
    /// checked state.
    fn toggle_format(&mut self, cell: usize) {
        // The last cell is the close affordance; pressing or clicking it
        // dismisses the bar rather than toggling a format.
        if let Some(state) = &self.format_bar
            && cell == state.items.len()
        {
            self.close_format_bar();
            return;
        }
        let command = self
            .format_bar
            .as_ref()
            .and_then(|state| state.items.get(cell))
            .copied();
        let Some(command) = command else {
            return;
        };
        if let Some(state) = &mut self.format_bar {
            state.selected = cell;
        }
        // A selection holding more than prose gets asked once, before
        // anything changes; the card's own confirm runs the command.
        if self.warn_mixed_format(command) {
            return;
        }
        (command.run)(self);
        self.refresh_format_bar();
    }

    /// A brush selection can hold math and code as well as words, and a
    /// text style only ever lands on the words. Rather than half-apply a
    /// format in silence, the first one pressed on such a selection says
    /// what it can reach and offers exactly that.
    ///
    /// Answering settles it for as long as the bar stays open: a reader who
    /// has said "yes, the words" once is not asked again for italic.
    /// `true` means the card is up and the command has not run.
    fn warn_mixed_format(&mut self, command: &'static commands::Command) -> bool {
        if self.format_bar.as_ref().is_none_or(|state| state.mixed_ack) {
            return false;
        }
        let (ranges, skipped) = self.format_ranges();
        if skipped == 0 || ranges.is_empty() {
            return false;
        }
        self.open_dialog(Prompt::MixedFormat {
            command: command.id,
            label: command.title,
            words: ranges.len(),
            skipped,
        });
        true
    }

    /// The mixed-selection card was confirmed: remember the answer so the
    /// bar stops asking, then run the format it was holding back.
    fn apply_mixed_format(&mut self, command: &str) {
        self.close_dialog();
        if let Some(state) = &mut self.format_bar {
            state.mixed_ack = true;
        }
        if let Some(command) = commands::COMMANDS.iter().find(|entry| entry.id == command) {
            (command.run)(self);
        }
        self.refresh_format_bar();
    }

    /// `false` means an outside click closed the bar and should keep
    /// routing so Normal mode can immediately target what was clicked.
    fn handle_format_bar_input(&mut self, input: &Input, viewport: Rect) -> bool {
        if input.is_key_pressed(KeyCode::Escape) {
            self.close_format_bar();
            return true;
        }
        if input.is_key_typed(KeyCode::ArrowRight) {
            let changed = if let Some(state) = &mut self.format_bar {
                let next = (state.selected + 1).min(state.items.len() - 1);
                let changed = next != state.selected;
                state.selected = next;
                changed
            } else {
                false
            };
            if changed {
                self.refresh_format_bar();
                self.rebuild_views();
            }
            return true;
        }
        if input.is_key_typed(KeyCode::ArrowLeft) {
            let changed = if let Some(state) = &mut self.format_bar {
                let next = state.selected.saturating_sub(1);
                let changed = next != state.selected;
                state.selected = next;
                changed
            } else {
                false
            };
            if changed {
                self.refresh_format_bar();
                self.rebuild_views();
            }
            return true;
        }
        if input.is_key_pressed(KeyCode::Enter) {
            if let Some(state) = &self.format_bar {
                self.toggle_format(state.selected);
            }
            return true;
        }

        let Some(state) = &self.format_bar else {
            return true;
        };
        let geometry = self.format_bar_geometry();
        if geometry.is_empty() {
            return true;
        }
        let card = format_bar::card_anchored(viewport, state.anchor, &geometry);
        let point = input.mouse_position();
        // Persistent hover lives in the shell: a touch of a cell moves the
        // memory, and the move is applied to the state (and mirrored into a
        // fresh snapshot) before anything else uses it.
        let touched = input
            .is_cursor_in_window()
            .then(|| format_bar::cell_at(card, &geometry, point))
            .flatten();
        let click = input.is_mouse_pressed(MouseButton::Left);
        let out_click = click && input.is_cursor_in_window() && !card.contains(point);
        // `state`'s borrow ends here; everything below re-borrows `self`.

        if let Some(cell) = touched {
            let moved = self
                .format_bar
                .as_ref()
                .is_some_and(|s| s.hover_cell != Some(cell));
            if let Some(state) = &mut self.format_bar {
                state.hover_cell = Some(cell);
            }
            if moved {
                self.refresh_format_bar();
            }
            if click {
                self.toggle_format(cell);
            }
            return true;
        }
        if out_click {
            self.close_format_bar();
            return false;
        }
        true
    }

    /// `false` means an outside left click closed the popup and should keep
    /// routing so Normal mode can immediately target what was clicked.
    fn handle_context_menu_input(&mut self, input: &Input, viewport: Rect) -> bool {
        if input.is_key_pressed(KeyCode::Escape) {
            self.close_context_menu();
            return true;
        }
        if input.is_key_typed(KeyCode::ArrowDown) {
            let changed = if let Some(state) = &mut self.context_menu {
                let next = (state.selected + 1).min(state.items.len() - 1);
                let changed = next != state.selected;
                state.selected = next;
                changed
            } else {
                false
            };
            if changed {
                self.refresh_context_menu();
            }
        }
        if input.is_key_typed(KeyCode::ArrowUp) {
            let changed = if let Some(state) = &mut self.context_menu {
                let next = state.selected.saturating_sub(1);
                let changed = next != state.selected;
                state.selected = next;
                changed
            } else {
                false
            };
            if changed {
                self.refresh_context_menu();
            }
        }
        if input.is_key_pressed(KeyCode::Enter) {
            self.run_selected_menu_command();
            return true;
        }

        if self.context_menu.is_none() {
            return true;
        }
        // Whichever face the card is wearing, its own geometry answers —
        // never the other one's, or a click would land on the row a
        // different card would have drawn there.
        let sections = self.symbol_menu_content().map(|(_, sections)| sections);
        let Some(state) = &self.context_menu else {
            return true;
        };
        let card = match &sections {
            Some(sections) => symbol_menu::card_anchored(viewport, state.anchor, sections),
            None => crate::components::popup::revealed_card(
                context_menu::card_anchored(viewport, state.anchor, state.items.len()),
                state.anchor,
                crate::components::popup::MENU_SLIDE_EASING.apply(self.popup_reveal.weight()),
            ),
        };
        let point = input.mouse_position();
        if input.is_cursor_in_window() {
            let hit = match &sections {
                Some(sections) => symbol_menu::cell_at(card, sections, point),
                None => context_menu::row_at(card, state.items.len(), point),
            };
            if let Some(row) = hit {
                let changed = self
                    .context_menu
                    .as_ref()
                    .is_some_and(|state| state.selected != row);
                if changed {
                    if let Some(state) = &mut self.context_menu {
                        state.selected = row;
                    }
                    self.refresh_context_menu();
                }
                if input.is_mouse_pressed(MouseButton::Left) {
                    self.run_selected_menu_command();
                }
            } else if input.is_mouse_pressed(MouseButton::Left) && !card.contains(point) {
                self.close_context_menu();
                return false;
            }
        }
        true
    }

    fn run_selected_menu_command(&mut self) {
        let command = self
            .context_menu
            .as_ref()
            .and_then(|state| state.items.get(state.selected))
            .copied();
        if let Some(command) = command {
            (command.run)(self);
            if command.id == "context.code.settings" {
                return;
            }
        }
        self.close_context_menu();
    }

    fn handle_slash_menu_input(&mut self, input: &Input) {
        if input.is_key_pressed(KeyCode::Escape) {
            self.close_slash_menu();
            return;
        }
        if input.is_key_pressed(KeyCode::Enter) {
            self.run_selected_slash_command();
            return;
        }
        // Typing / again while the menu is open closes it and inserts a
        // literal / into the document — the escape hatch for writing a
        // real slash character.
        let text = input.text();
        if text == "/" {
            self.close_slash_menu();
            if self.vim.visual_mode().is_none() {
                self.docs.borrow_mut().type_text("/");
            }
            return;
        }
        let changed = match &mut self.slash_menu {
            Some(state) => slash_menu_input(state, input),
            None => false,
        };
        if changed {
            self.refresh_slash_menu();
        }
    }

    fn close_slash_menu(&mut self) {
        // The ghost's shape comes from the live state, so read the snapshot
        // before `take` empties it.
        let snap = self
            .slash_menu
            .as_ref()
            .map(|state| crate::components::slash_menu::Snapshot {
                entries: commands::editor_entries(),
                visible: palette::filter(&commands::editor_entries(), &state.query),
                query: state.query.clone(),
                selected: state.selected,
                // Scroll window rule from `SlashMenu::new`: keep the
                // selection visible at the bottom of the window.
                first_visible: state
                    .selected
                    .saturating_sub(crate::components::slash_menu::MAX_ROWS.saturating_sub(1)),
            });
        let anchor = self.slash_menu.as_ref().map(|state| state.anchor);
        // The pill parks where the keyboard selection is — the same place a
        // fresh open starts (see `Format`'s equivalent).
        let pointer_row = self
            .slash_menu
            .as_ref()
            .and_then(|state| self.pill_row_of_slash(state));
        self.slash_menu = None;
        if let (Some(snap), Some(anchor)) = (snap, anchor) {
            if !snap.visible.is_empty() {
                self.menu_dismiss = Some(MenuDismiss::Slash {
                    state: snap,
                    anchor,
                    pointer_row,
                });
                self.menu_dismiss_clock = 1.0;
            } else {
                self.menu_dismiss = None;
                self.menu_dismiss_clock = 0.0;
            }
        }
        self.refresh_slash_menu();
    }

    /// Which row the slash menu's pill sits on. The component holds the live
    /// one after hover; fall back to the keyboard selection.
    fn pill_row_of_slash(&self, _state: &SlashMenuState) -> Option<usize> {
        self.regions[self.slash_region]
            .component_as::<crate::components::slash_menu::SlashMenu>()
            .and_then(crate::components::slash_menu::SlashMenu::pill_row)
            .or(Some(_state.selected))
    }

    fn run_selected_slash_command(&mut self) {
        let Some(state) = &self.slash_menu else {
            return;
        };
        let entries = commands::editor_entries();
        let visible = palette::filter(&entries, &state.query);
        let command = visible
            .get(state.selected)
            .and_then(|&i| commands::editor_commands().get(i).copied());
        let visual_active = self.vim.visual_mode().is_some();
        let selection = visual_active.then(|| self.current_selection()).flatten();
        let saved_caret = selection.and_then(|_| {
            self.docs
                .borrow()
                .active()
                .map(|tab| tab.document.caret_position())
        });
        self.close_slash_menu();
        if let Some(command) = command {
            let range_style = match command.id {
                "format.bold" => Some(Style {
                    bold: true,
                    ..Style::PLAIN
                }),
                "format.italic" => Some(Style {
                    italic: true,
                    ..Style::PLAIN
                }),
                "format.highlight" => Some(Style {
                    highlight: true,
                    ..Style::PLAIN
                }),
                "format.inline_code" => Some(Style {
                    code: true,
                    ..Style::PLAIN
                }),
                "format.badge" => Some(Style {
                    badge: true,
                    ..Style::PLAIN
                }),
                _ => None,
            };

            if let (Some(range), Some(mask)) = (selection, range_style) {
                self.docs.borrow_mut().toggle_style_range(range, mask);
            } else {
                (command.run)(self);
            }
            if visual_active {
                if let Some(caret) = saved_caret {
                    self.docs
                        .borrow_mut()
                        .touch(|doc| doc.set_flat_position(caret));
                }
                self.exit_visual();
            }
        }
    }

    // ---- math completion card --------------------------------------------

    /// The card's claims while it is showing: `Esc` dismisses it for the
    /// word it is on, `Enter` accepts the selected row, `↑`/`↓` and
    /// `Ctrl+N`/`Ctrl+P` move the selection, and `Ctrl+1..9` pick a row
    /// outright. Returns whether a claim was consumed. Deliberately does not
    /// claim `Tab`, which must keep walking slots, `←`/`→`, which move the
    /// math cursor, or any typed character, which keeps typing — the word
    /// changing is what closes and reopens the card.
    fn handle_math_menu_input(&mut self, input: &Input, viewport: Rect) -> bool {
        if input.is_key_pressed(KeyCode::Escape) {
            self.dismiss_math_menu();
            return true;
        }
        if input.is_key_typed(KeyCode::Enter) {
            self.accept_math_menu();
            return true;
        }
        if input.is_key_typed(KeyCode::ArrowDown) {
            self.move_math_menu(1);
            return true;
        }
        if input.is_key_typed(KeyCode::ArrowUp) {
            self.move_math_menu(-1);
            return true;
        }
        if input.ctrl() {
            let digits = [
                KeyCode::Digit1,
                KeyCode::Digit2,
                KeyCode::Digit3,
                KeyCode::Digit4,
                KeyCode::Digit5,
                KeyCode::Digit6,
                KeyCode::Digit7,
                KeyCode::Digit8,
                KeyCode::Digit9,
            ];
            if let Some(row) = digits.iter().position(|key| input.is_key_typed(*key)) {
                self.accept_math_menu_row(row);
                return true;
            }
            if input.is_key_typed(KeyCode::KeyN) {
                self.move_math_menu(1);
                return true;
            }
            if input.is_key_typed(KeyCode::KeyP) {
                self.move_math_menu(-1);
                return true;
            }
        }

        let Some(state) = &self.math_menu else {
            return false;
        };
        let offers = math_conversion::offers(&state.query).offers;
        let variant_start = math_menu_variant_start(&offers);
        let card = math_menu::card_anchored(viewport, state.anchor, offers.len(), variant_start);
        if input.is_cursor_in_window() {
            let point = input.mouse_position();
            if card.contains(point) && input.scroll_delta().1 != 0.0 {
                self.move_math_menu(if input.scroll_delta().1 > 0.0 { -1 } else { 1 });
                return true;
            }
            if let Some(index) = math_menu::item_at(
                card,
                offers.len(),
                variant_start,
                state.first_visible,
                point,
            ) {
                let changed = self
                    .math_menu
                    .as_ref()
                    .is_some_and(|state| state.selected != index);
                if changed
                    && (input.mouse_delta() != (0.0, 0.0)
                        || input.is_mouse_pressed(MouseButton::Left))
                {
                    if let Some(state) = &mut self.math_menu {
                        state.selected = index;
                    }
                    self.refresh_math_menu();
                }
                if input.is_mouse_pressed(MouseButton::Left) {
                    self.accept_math_menu_row(index);
                    return true;
                }
            } else if input.is_mouse_pressed(MouseButton::Left) {
                if card.contains(point) {
                    return true;
                }
                self.dismiss_math_menu();
            }
        }
        false
    }

    /// Esc on the card: stop offering completions for the current word, but
    /// leave the word itself alone — typing more letters brings the card
    /// back.
    fn dismiss_math_menu(&mut self) {
        self.math_dismissed = self.math_menu.as_ref().map(|state| state.query.clone());
        self.spawn_math_ghost_from_live();
        self.math_menu = None;
        self.refresh_math_menu();
    }

    fn accept_math_menu(&mut self) {
        let selected = self.math_menu.as_ref().map_or(0, |state| state.selected);
        self.accept_math_menu_row(selected);
    }

    fn accept_math_menu_row(&mut self, index: usize) {
        let Some(query) = self.math_menu.as_ref().map(|state| state.query.clone()) else {
            return;
        };
        let Some(offer) = math_conversion::offers(&query).offers.get(index).cloned() else {
            return;
        };
        self.spawn_math_ghost_from_live();
        self.docs
            .borrow_mut()
            .math_accept_conversion(&query, &offer);
        self.math_dismissed = None;
        self.math_menu = None;
        self.refresh_math_menu();
    }

    fn move_math_menu(&mut self, delta: isize) {
        let Some(state) = &mut self.math_menu else {
            return;
        };
        let offers = math_conversion::offers(&state.query).offers;
        let count = offers.len();
        if count == 0 {
            return;
        }
        let next = moved_math_menu_selection(state.selected, count, delta);
        if next != state.selected {
            state.selected = next;
            state.first_visible = math_menu::follow_selection(
                math_menu_variant_start(&offers),
                next,
                state.first_visible,
            );
            self.refresh_math_menu();
        }
    }

    /// Recomputed every frame, the card's whole life: show it when the word
    /// under the math cursor has offers it was not dismissed for, hide it
    /// otherwise. Movement and edits replace the precise tree query.
    pub(super) fn sync_math_menu(&mut self) {
        let query = self.docs.borrow().math_conversion_query();
        let Some(query) = query else {
            if self.math_menu.is_some() {
                self.spawn_math_ghost_from_live();
                self.math_menu = None;
                self.refresh_math_menu();
            }
            return;
        };
        if self.math_dismissed.as_ref() == Some(&query)
            || math_conversion::offers(&query).offers.is_empty()
        {
            if self.math_menu.is_some() {
                // The card just closed with content on screen: leave a
                // ghost falling away instead of popping to nothing. (An
                // empty-offers close has no rows; the spawn check below
                // covers it.)
                self.spawn_math_ghost_from_live();
                self.math_menu = None;
                self.refresh_math_menu();
            }
            return;
        }
        if self.math_menu.is_none() {
            // A new word supersedes a fading card immediately, including
            // when typing resumes before its dismissal has finished.
            if matches!(self.menu_dismiss, Some(MenuDismiss::Math { .. })) {
                self.menu_dismiss = None;
                self.menu_dismiss_clock = 0.0;
            }
            self.popup_reveal.restart();
        }
        let anchor = self.compute_math_menu_anchor();
        let changed = match &self.math_menu {
            Some(state) => state.query != query,
            None => true,
        };
        let state = self.math_menu.get_or_insert_with(|| MathMenuState {
            query: query.clone(),
            selected: 0,
            first_visible: 0,
            anchor,
        });
        let moved = state.anchor != anchor;
        state.anchor = anchor;
        // A new query is a new list: reset the selection and rebuild rows.
        if changed {
            state.query = query;
            state.selected = 0;
            state.first_visible = 0;
        }
        if changed || moved {
            self.refresh_math_menu();
        }
    }

    /// The math caret's screen position, used as the card's anchor. The
    /// same coordinate math the editor's `draw()` uses: the atom's pen
    /// position from `caret_pos`, plus the math cursor's offset within the
    /// expression from `math_layout::cursor_pos`.
    fn compute_math_menu_anchor(&mut self) -> (f32, f32) {
        let rect = self.layout.rect(self.text_column);
        let width = crate::components::editor::Editor::content_width(rect);
        let layout = self.current_layout(width);
        let layer = self.regions[self.text_region].layer();
        let measure = |text: &str, style: &TextStyle| theme::width(layer, text, style);
        let (base_x, base_y, scroll, offset_x, offset_y) = {
            let docs = self.docs.borrow();
            let Some(tab) = docs.active() else {
                return (rect.x, rect.y);
            };
            let doc = &tab.document;
            let (cx, cy, _) = layout.caret_pos(doc.caret, &measure);
            let Some((list, cursor)) = doc.focused_math_view() else {
                return (rect.x, rect.y);
            };
            let (ox, oy, _) = math_layout::cursor_pos(list, cursor, 0, layout.scale, &measure);
            (cx, cy, docs.editor_scroll, ox, oy)
        };
        let screen_x = Editor::content_x(rect) + base_x + offset_x;
        let screen_y = rect.y + crate::components::editor::TOP + base_y - offset_y - scroll;
        (screen_x, screen_y)
    }

    /// Captures the live math card's REAL state into `MenuDismiss::Math`
    /// and arms the clock — call while `self.math_menu` still holds the
    /// pre-close state and BEFORE refreshing the region.
    fn spawn_math_ghost_from_live(&mut self) {
        let snap = self.math_menu.as_ref().map(|state| {
            let offers = math_conversion::offers(&state.query).offers;
            crate::components::math_menu::Snapshot {
                rows: math_menu_rows(&offers),
                variant_start: math_menu_variant_start(&offers),
                selected: state.selected,
                first_visible: state.first_visible,
            }
        });
        let anchor = self.math_menu.as_ref().map(|state| state.anchor);
        let pill_row = self.regions[self.math_menu_region]
            .component_as::<MathMenu>()
            .and_then(MathMenu::pill_row);
        if let (Some(snap), Some(anchor)) = (snap, anchor)
            && !snap.rows.is_empty()
        {
            // The live component clamps its selection; mirror it.
            let pill_row = pill_row.unwrap_or(snap.selected.min(snap.rows.len() - 1));
            self.menu_dismiss = Some(MenuDismiss::Math {
                state: snap,
                anchor,
                pill_row,
            });
            self.menu_dismiss_clock = 1.0;
            return;
        }
        self.menu_dismiss = None;
        self.menu_dismiss_clock = 0.0;
    }

    /// Shadow layer rides every arm — ghosts included — so a closing card
    /// always takes its halo away (see `refresh_slash_menu`).
    pub(super) fn refresh_math_menu(&mut self) {
        let menu = match &self.math_menu {
            Some(state) => {
                let offers = math_conversion::offers(&state.query).offers;
                MathMenu::new(
                    math_menu_rows(&offers),
                    math_menu_variant_start(&offers),
                    state.selected,
                    state.first_visible,
                    state.anchor,
                )
                .resuming(self.regions[self.math_menu_region].component_as::<MathMenu>())
            }
            None => match &self.menu_dismiss {
                Some(MenuDismiss::Math {
                    state,
                    anchor,
                    pill_row,
                }) => MathMenu::dismissing(state, *anchor, *pill_row),
                _ => MathMenu::closed(),
            },
        }
        .with_shadow(self.popup_shadow.clone());
        self.regions[self.math_menu_region].set_component(Box::new(menu));
    }

    /// The scroll thumb's drag, shaped like [`Shell::divider_drag`]: the
    /// press takes hold, the release lets go, and every frame in between
    /// puts the page where the pointer says.
    fn scrollbar_drag(&mut self, input: &Input) {
        if !input.is_mouse_down(MouseButton::Left) || !self.layout.style(self.scroll_column).visible
        {
            self.scroll_grab = None;
            return;
        }
        let strip = self.layout.rect(self.scroll_column);
        let track = scrollbar::track(strip);
        // Last frame's span — the one the thumb on screen was drawn from,
        // which is the thumb the reader just aimed at.
        let span = self.scroll_span.get();
        let mouse = input.mouse_position();

        if input.is_mouse_pressed(MouseButton::Left) {
            if !input.is_cursor_in_window() || !strip.contains(mouse) {
                return;
            }
            let Some(thumb) = scrollbar::thumb(track, span) else {
                return;
            };
            // On the thumb, hold it exactly where it was taken. On the track
            // above or below, throw it to the pointer and hold it by the
            // middle, so the jump and the drag that may follow are one
            // movement rather than a jump and then a second one.
            self.scroll_grab = Some(if thumb.contains(mouse) {
                mouse.1 - thumb.y
            } else {
                thumb.height / 2.0
            });
        }
        let Some(grab) = self.scroll_grab else {
            return;
        };
        let (min, max) = self.editor_scroll_bounds();
        let to = scrollbar::scroll_at(track, mouse.1 - grab, span).clamp(min, max);
        // Under the hand, not behind it: a thumb that eased toward the
        // pointer holding it would drift away from the pointer holding it.
        self.scroll.settle(to);
    }

    fn divider_drag(&mut self, input: &Input) {
        let mouse = input.mouse_position();
        let over = input
            .is_cursor_in_window()
            .then(|| self.divider_at(mouse))
            .flatten();
        // Only the tree's divider has a hover affordance drawn for it.
        self.divider_hot = over == Some(super::Divider::Tree);
        if input.is_mouse_pressed(MouseButton::Left) {
            self.dragging = over;
        }
        if !input.is_mouse_down(MouseButton::Left) {
            self.dragging = None;
        }
        let Some(which) = self.dragging else {
            return;
        };
        // A panel can only grow into the slack the text column has above its
        // own minimum, so the drag stops exactly where the canvas would start
        // being squeezed.
        let slack =
            self.layout.rect(self.text_column).width - self.layout.min_size(self.text_column).0;
        let node = self.panel_mut(which).node;
        let rect = self.layout.rect(node);
        let wanted = super::dragged_width(rect, mouse.0, which != super::Divider::Tree);
        let smallest = self.layout.min_size(node).0;
        let largest = (rect.width + slack.max(0.0)).max(smallest);
        self.panel_mut(which)
            .resize(wanted.clamp(smallest, largest));
    }

    // ---- dialogs ----------------------------------------------------------

    /// `pub(super)`: also the run function for `commands::COMMANDS`'s
    /// `file.new`, which lives in the sibling `commands` module.
    pub(super) fn open_dialog(&mut self, prompt: Prompt) {
        self.dialog = Some(prompt);
        self.refresh_dialog();
    }

    fn close_dialog(&mut self) {
        self.dialog = None;
        self.refresh_dialog();
    }

    /// Rebuilds the dialog region from the current prompt.
    /// Shadow layer rides every arm; a closed snapshot clears the layer so
    /// the halo leaves with the dialog (see `refresh_slash_menu`).
    fn refresh_dialog(&mut self) {
        let prompt = self.dialog.clone();
        let dialog = Dialog::new(prompt).with_shadow(self.popup_shadow.clone());
        self.regions[self.dialog_region].set_component(Box::new(dialog));
    }

    fn handle_dialog_input(&mut self, input: &Input, viewport: Rect) {
        let mut changed = false;
        let click = input
            .is_mouse_pressed(MouseButton::Left)
            .then(|| input.mouse_position());

        if let Some(Prompt::ExportPdf { dark }) = &mut self.dialog {
            // The pill is a click target and the space bar is its keyboard
            // equal. Neither reaches the buttons below — the switch's rect
            // is nowhere near them — so a flick of the switch cannot
            // confirm the card by accident.
            if input.is_key_typed(KeyCode::Space)
                || click.is_some_and(|at| dialog::toggle(viewport).contains(at))
            {
                *dark = !*dark;
                changed = true;
            }
        }

        if let Some(
            Prompt::NewNote { input: name, caret } | Prompt::NewFolder { input: name, caret },
        ) = &mut self.dialog
        {
            let text = input.text();
            if !text.is_empty() {
                name.splice(*caret..*caret, text.chars());
                *caret += text.chars().count();
                changed = true;
            }
            if input.is_key_typed(KeyCode::Backspace) && *caret > 0 {
                *caret -= 1;
                name.remove(*caret);
                changed = true;
            }
            if input.is_key_typed(KeyCode::Delete) && *caret < name.len() {
                name.remove(*caret);
                changed = true;
            }
            if input.is_key_typed(KeyCode::ArrowLeft) && *caret > 0 {
                *caret -= 1;
                changed = true;
            }
            if input.is_key_typed(KeyCode::ArrowRight) && *caret < name.len() {
                *caret += 1;
                changed = true;
            }
        }

        let (confirm_button, cancel_button) = dialog::buttons(viewport);
        // A fresh press, never the OS repeat: the format bar's own Enter is
        // what raises the mixed-selection card, and a held key would then
        // answer the question it had only just asked. Nothing a card does
        // wants repeating anyway — Esc has always been edge-triggered here.
        let confirmed = input.is_key_pressed(KeyCode::Enter)
            || click.is_some_and(|at| confirm_button.contains(at));
        let cancelled = input.is_key_pressed(KeyCode::Escape)
            || click.is_some_and(|at| cancel_button.contains(at));

        if confirmed {
            self.confirm_dialog();
        } else if cancelled {
            self.close_dialog();
        } else if changed {
            self.refresh_dialog();
        }
    }

    /// Enter (or the confirm button): act on the open prompt.
    fn confirm_dialog(&mut self) {
        match self.dialog.clone() {
            Some(Prompt::NewNote { input, .. }) => {
                self.create_note(&input.iter().collect::<String>())
            }
            Some(Prompt::NewFolder { input, .. }) => {
                self.create_folder(&input.iter().collect::<String>())
            }
            Some(Prompt::DeleteNote { .. }) => self.delete_selected(),
            // Closed before the file dialog opens, not after it returns: it
            // is a native modal, and a card left standing behind one is a
            // card asking a question that has already been answered.
            Some(Prompt::ExportPdf { dark }) => {
                self.close_dialog();
                self.run_export(if dark { Theme::DARK } else { Theme::LIGHT });
            }
            Some(Prompt::MixedFormat { command, .. }) => self.apply_mixed_format(command),
            None => {}
        }
    }

    /// Where a new note or folder goes: the folder selected in the tree, the
    /// parent folder of the selected file, or the vault root when nothing is
    /// selected.
    fn new_file_base(&self) -> Option<std::path::PathBuf> {
        let vault = self.vault.as_ref()?;
        let root = vault.borrow().root().to_path_buf();
        let selected = self.docs.borrow().tree_selected.clone();
        let base = match selected {
            Some(path) if path.is_dir() => path,
            Some(path) => path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or(root.clone()),
            None => root.clone(),
        };
        if base.starts_with(&root) {
            Some(base)
        } else {
            Some(root)
        }
    }

    /// A typed name is a relative path inside the vault: `lecture 3` or
    /// `math/lecture 3`. Absolute paths and `..` are refused outright.
    fn resolve_new_path(&self, name: &str) -> Option<std::path::PathBuf> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        let relative = std::path::Path::new(name);
        if relative.is_absolute()
            || relative.components().any(|c| {
                matches!(
                    c,
                    std::path::Component::ParentDir
                        | std::path::Component::Prefix(_)
                        | std::path::Component::RootDir
                )
            })
        {
            return None;
        }
        Some(self.new_file_base()?.join(relative))
    }

    /// Creates the file (and any folders named in the path), opens it in a
    /// full tab, and closes the dialog. An empty or unusable name keeps the
    /// dialog open; naming a file that already exists just opens it.
    fn create_note(&mut self, name: &str) {
        let Some(mut path) = self.resolve_new_path(name) else {
            return;
        };
        if path.extension().is_none() {
            path.set_extension("md");
        }
        if path.exists() {
            self.docs.borrow_mut().open_full(&path);
            self.reveal_in_tree(&path);
            self.close_dialog();
            return;
        }
        if let Some(parent) = path.parent()
            && std::fs::create_dir_all(parent).is_err()
        {
            return;
        }
        if std::fs::write(&path, "").is_ok() {
            self.docs.borrow_mut().open_full(&path);
            self.reveal_in_tree(&path);
            self.close_dialog();
        }
    }

    /// Creates the folder (and any parents named in the path), selects it in
    /// the tree so the next new note lands inside it, and closes the dialog.
    fn create_folder(&mut self, name: &str) {
        let Some(path) = self.resolve_new_path(name) else {
            return;
        };
        if !path.is_dir() && std::fs::create_dir_all(&path).is_err() {
            return;
        }
        self.docs.borrow_mut().tree_selected = Some(path.clone());
        self.reveal_in_tree(&path);
        self.close_dialog();
    }

    /// Re-reads the vault, expands everything above `path`, and repaints the
    /// tree region.
    fn reveal_in_tree(&mut self, path: &std::path::Path) {
        if let Some(vault) = &self.vault {
            vault.borrow_mut().refresh();
            vault.borrow_mut().reveal(path);
        }
        self.regions[self.tree_region].poke();
    }

    /// Ctrl+D with a file selected in the tree. Deleting is not undoable
    /// and there is no trash, so this only ever raises the confirmation —
    /// the delete itself is in [`Shell::delete_selected`]. `pub(super)`:
    /// also the run function for `commands::COMMANDS`'s `file.delete`.
    pub(super) fn ask_delete_selected(&mut self) {
        let path = self.docs.borrow().tree_selected.clone();
        let Some(path) = path.filter(|path| path.is_file()) else {
            return;
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        self.open_dialog(Prompt::DeleteNote { name });
    }

    /// Deletes the selected file from disk, closes every tab holding it,
    /// and drops it from the tree. Only reachable through the confirmation.
    fn delete_selected(&mut self) {
        let path = self.docs.borrow().tree_selected.clone();
        let Some(path) = path.filter(|path| path.is_file()) else {
            self.close_dialog();
            return;
        };
        if std::fs::remove_file(&path).is_err() {
            self.close_dialog();
            return;
        }
        self.docs.borrow_mut().close_path(&path);
        if let Some(vault) = &self.vault {
            vault.borrow_mut().remove(&path);
        }
        self.regions[self.tree_region].poke();
        self.close_dialog();
    }

    /// Runs the folder picker; on a successful choice, saves the config,
    /// opens the vault, and swaps the splash for the real tree. `pub(super)`:
    /// also the run function for `commands::COMMANDS`'s `vault.open`.
    pub(super) fn open_picker(&mut self) {
        let Some(config) = Config::onboard() else {
            return; // dismissed; the splash stays.
        };
        let vault = Vault::open(&config.vault).map(|v| Rc::new(RefCell::new(v)));
        self.config = Some(config);
        self.onboarding = false;
        self.vault = vault.clone();
        self.regions[self.tree_region].set_component(Box::new(FileTree::new(
            vault,
            self.docs.clone(),
            self.tree_menu_request.clone(),
        )));
        self.regions[self.onboard_region].set_component(Box::new(Onboarding::new(false)));
        self.rebuild_views();
    }
}

/// Deletes structurally while math is focused and exits at the matching root
/// edge. Returns the tree result when math owned the keypress.
fn delete_inside_math(docs: &mut Tabs, forward: bool) -> Option<math::Removed> {
    if !docs.in_math() {
        return None;
    }
    let removed = if forward {
        docs.math_delete_forward()
    } else {
        docs.math_backspace()
    }?;
    match (forward, removed) {
        (false, math::Removed::AtStart) => docs.math_exit_before(),
        (true, math::Removed::AtEnd) => docs.math_exit_after(),
        _ => {}
    }
    Some(removed)
}

/// Normal-mode h/l traverses a focused tree before returning to prose.
fn move_inside_math(docs: &mut Tabs, motion: Motion, count: usize) -> bool {
    if !docs.in_math() || !matches!(motion, Motion::Left | Motion::Right) {
        return false;
    }
    for _ in 0..count.max(1) {
        let moved = match motion {
            Motion::Left => docs.math_left(),
            Motion::Right => docs.math_right(),
            _ => unreachable!("math motion was checked above"),
        };
        if !moved {
            match motion {
                Motion::Left => docs.math_exit_before(),
                Motion::Right => docs.math_exit_after(),
                _ => unreachable!("math motion was checked above"),
            }
            break;
        }
    }
    true
}

/// Normal `x`: focused math deletes structurally; an opaque math position
/// enters its tree through `Document::delete_char` on the first iteration.
fn delete_chars(docs: &mut Tabs, count: usize) {
    docs.transaction(|docs| {
        for _ in 0..count.max(1) {
            if matches!(
                delete_inside_math(docs, true),
                None | Some(math::Removed::AtEnd)
            ) {
                docs.delete_char();
            }
        }
    });
}

/// Pastes the yank register at the caret, `count` times. `exported_yank` is
/// the string this app last published to the OS clipboard, so a register that
/// equals it is our own notation: it is re-read through `insert_notation`,
/// where `$…$` is an expression and `\$` a literal dollar. Anything else
/// arrived from elsewhere and is inserted byte-for-byte through `insert_text`.
/// The provenance decision is made here, once, rather than guessed at by the
/// document.
fn paste(docs: &mut Tabs, before: bool, count: usize, exported_yank: &str) {
    let text = docs.yank().unwrap_or_default().to_string();
    if text.is_empty() {
        return;
    }
    let notation = text == exported_yank;
    docs.transaction(|docs| {
        let was_preview = docs.active().map(|tab| tab.preview).unwrap_or(false);
        docs.touch(|doc| {
            if !before {
                let position = doc.caret_position();
                let offset = (position.offset + 1).min(doc.block_len(position.block));
                doc.set_flat_position(doc.position(position.block, offset));
            }
            for _ in 0..count.max(1) {
                if notation {
                    doc.insert_notation(&text);
                } else {
                    doc.insert_text(&text);
                }
            }
        });
        // Editing a preview makes it permanent (spec §7.1); `touch` never
        // promotes, so promote here exactly as `Tabs::edit` would.
        if was_preview && let Some(tab) = docs.active_mut() {
            tab.preview = false;
        }
    });
}

/// Enter Insert mode, opening the undo transaction that groups the whole
/// insert session into one step. This is the one place every path into
/// Insert mode funnels through — `i`/`a`/`I`/`A`, the click that lands in a
/// math atom, `o`/`O`, `s`, and the change operators — so they cannot drift
/// apart on undo granularity again. `Tabs::begin_transaction` already guards
/// against nesting, so a second entry while one is open is a no-op.
fn enter_insert(docs: &mut Tabs, vim: &mut Vim) {
    docs.begin_transaction();
    vim.set_mode(Mode::Insert);
}

/// A caret resolved against the page's body layout. Exact hit-test carets
/// must reset focus before they touch the document, so a body coordinate can
/// never be applied to a note by accident. Flat positions — search and finder
/// landings — go through [`Tabs::jump_to_flat`], which owns the same rule
/// plus the unfold-over-the-landing one.
#[derive(Clone, Copy)]
enum BodyCoordinate {
    Caret(Caret),
}

/// Applies a body coordinate as one operation: focus body, then place caret.
/// Keeping those writes together makes every page-derived caret move obey the
/// same scope boundary.
fn set_body_coordinate_caret(docs: &mut Tabs, coordinate: BodyCoordinate) {
    docs.touch(|doc| {
        doc.focus = Focus::Body;
        match coordinate {
            BodyCoordinate::Caret(caret) => doc.set_caret(caret.block, caret.inline, caret.offset),
        }
    });
}

/// The note anchored at the caret — the anchor run the caret sits on, or the
/// one immediately before it. A sidenote anchor occupies one flat position,
/// so "on" and "immediately after" are the two flat offsets around it.
fn note_at_caret(doc: &Document, caret: Caret) -> Option<usize> {
    if doc.focus != Focus::Body {
        return None;
    }
    let label = anchor_label_at(doc.body(), caret)?;
    doc.notes.iter().position(|note| note.label == label)
}

/// The anchor run's label at `caret`, or `None` when the caret is neither on
/// nor immediately after an anchor.
fn anchor_label_at(blocks: &[Block], caret: Caret) -> Option<&str> {
    let runs = blocks.get(caret.block)?.inlines();
    match runs.get(caret.inline) {
        Some(Inline::Note(label)) => Some(label.as_str()),
        _ if caret.offset == 0 && caret.inline > 0 => match runs.get(caret.inline - 1) {
            Some(Inline::Note(label)) => Some(label.as_str()),
            _ => None,
        },
        _ => None,
    }
}

/// The anchor's position in the body — `(block, inline)` — for the note whose
/// label is `label`.
fn anchor_position(doc: &Document, label: &str) -> Option<(usize, usize)> {
    doc.body()
        .iter()
        .enumerate()
        .find_map(|(block, block_runs)| {
            block_runs
                .inlines()
                .iter()
                .position(|run| matches!(run, Inline::Note(l) if l == label))
                .map(|inline| (block, inline))
        })
}

/// Focuses `index`'s note and puts the caret at the end of its body. A note
/// is a single paragraph, so "end of body" is the end of block 0. Caret-only,
/// so it goes through `touch` and never promotes a preview tab.
fn focus_note(docs: &mut Tabs, index: usize) {
    docs.touch(|doc| {
        doc.focus = Focus::Note(index);
        doc.move_end();
    });
}

/// Focuses `index`'s note with the caret at `caret` — a margin click's
/// position, resolved against the note's own layout, never the body's.
fn focus_note_at(docs: &mut Tabs, index: usize, caret: Caret) {
    docs.touch(|doc| {
        doc.focus = Focus::Note(index);
        doc.set_caret(caret.block, caret.inline, caret.offset);
    });
}

/// Leaves a focused note: focus returns to the body with the caret on the
/// note's anchor. Called only from Normal mode's Escape — Insert mode's
/// Escape still leaves Insert and stays in the note, so leaving a note while
/// typing is two presses, the same shape as leaving Insert and then leaving
/// anything else.
pub(super) fn return_to_anchor(docs: &mut Tabs) {
    let anchor = {
        let Some(tab) = docs.active() else { return };
        let Focus::Note(i) = tab.document.focus else {
            return;
        };
        let Some(label) = tab.document.notes.get(i).map(|note| note.label.clone()) else {
            return;
        };
        anchor_position(&tab.document, &label)
    };
    if let Some((block, inline)) = anchor {
        docs.touch(|doc| {
            doc.focus = Focus::Body;
            doc.set_caret(block, inline, 0);
        });
    }
}

/// The note whose placed y-band contains `y`, and the caret inside that note
/// nearest `x` — resolved against the note's own layout, never the body's.
/// `notes` is `(label, placed y, layout)`; `margin_x` and `inset` place the
/// note's content column on screen, `content_top` the margin's content top.
/// Pure, so the margin's click math is a test rather than a runtime surprise.
fn note_at_point(
    notes: &[(String, f32, Rc<DocLayout>)],
    x: f32,
    y: f32,
    margin_x: f32,
    content_top: f32,
    inset: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> Option<(String, Caret)> {
    for (label, note_y, layout) in notes {
        let top = content_top + note_y;
        if y >= top && y <= top + layout.height {
            let caret = layout.hit(x - (margin_x + inset), y - top, measure);
            return Some((label.clone(), caret));
        }
    }
    None
}

/// Edits `state` from one frame's input. Returns whether anything changed
/// enough to need the palette region rebuilt. A free function rather than
/// a method: it only ever needs the one field's worth of state, and taking
/// `&mut PaletteState` directly (instead of routing through `&mut self`)
/// keeps `handle_palette_input` from having to fight the borrow checker
/// over `self.palette` while still calling other `Shell` methods.
fn palette_input(state: &mut PaletteState, input: &Input) -> bool {
    let mut changed = false;
    let text = input.text();
    if !text.is_empty() {
        state.query.push_str(text);
        state.selected = 0;
        changed = true;
    }
    if input.is_key_typed(KeyCode::Backspace) && state.query.pop().is_some() {
        state.selected = 0;
        changed = true;
    }
    let visible = palette::filter(&commands::entries(), &state.query).len();
    if input.is_key_typed(KeyCode::ArrowDown) && visible > 0 {
        state.selected = (state.selected + 1).min(visible - 1);
        changed = true;
    }
    if input.is_key_typed(KeyCode::ArrowUp) && state.selected > 0 {
        state.selected -= 1;
        changed = true;
    }
    changed
}

/// Edits `state` from one frame's input. Returns whether anything changed
/// enough to need the slash menu region rebuilt. Same pattern as
/// `palette_input` — a free function so the borrow checker doesn't fight
/// `handle_slash_menu_input` over `self.slash_menu`.
fn slash_menu_input(state: &mut SlashMenuState, input: &Input) -> bool {
    let mut changed = false;
    let text = input.text();
    if !text.is_empty() {
        state.query.push_str(text);
        state.selected = 0;
        changed = true;
    }
    if input.is_key_typed(KeyCode::Backspace) && state.query.pop().is_some() {
        state.selected = 0;
        changed = true;
    }
    let visible = palette::filter(&commands::editor_entries(), &state.query).len();
    if input.is_key_typed(KeyCode::ArrowDown) && visible > 0 {
        state.selected = (state.selected + 1).min(visible - 1);
        changed = true;
    }
    if input.is_key_typed(KeyCode::ArrowUp) && state.selected > 0 {
        state.selected -= 1;
        changed = true;
    }
    changed
}

/// The completion card rows, preserving the conversion engine's ranking.
fn math_menu_rows(offers: &[math_conversion::Offer]) -> Vec<math_menu::Row> {
    offers
        .iter()
        .map(|offer| match offer {
            math_conversion::Offer::Named(math::Completion::Symbol { name, group, glyph }) => {
                math_menu::Row {
                    name: (*name).to_owned(),
                    group: (*group).to_owned(),
                    preview: glyph.to_string(),
                }
            }
            math_conversion::Offer::Named(math::Completion::Structure { name, preview }) => {
                math_menu::Row {
                    name: (*name).to_owned(),
                    group: "Structure".to_owned(),
                    preview: (*preview).to_owned(),
                }
            }
            math_conversion::Offer::Rewrite {
                title,
                group,
                preview,
                ..
            } => math_menu::Row {
                name: title.to_string(),
                group: group.to_string(),
                preview: preview.to_string(),
            },
            math_conversion::Offer::Variant {
                name,
                group,
                preview,
                ..
            } => math_menu::Row {
                name: (*name).to_owned(),
                group: (*group).to_owned(),
                preview: preview.clone(),
            },
        })
        .collect()
}

/// First flat item drawn as a variant cell. Variant offers are appended by
/// the conversion engine, so ordinary rows remain a contiguous prefix.
fn math_menu_variant_start(offers: &[math_conversion::Offer]) -> usize {
    offers
        .iter()
        .position(|offer| matches!(offer, math_conversion::Offer::Variant { .. }))
        .unwrap_or(offers.len())
}

/// What the inspector offers about a symbol's meaning and look, before any
/// question of letterform. Every symbol has these, whatever it is spelled
/// with.
///
/// Grouped by each command's own `group`, which is what turns this flat
/// list into the card's sections — so a command's place in the table and
/// its place on the card cannot disagree.
fn symbol_style_ids() -> Vec<&'static str> {
    let mut ids = commands::SYMBOL_ROLE_MENU.to_vec();
    ids.extend(commands::SYMBOL_HUE_MENU);
    ids.extend(commands::SYMBOL_SHAPE_MENU);
    ids.extend(commands::SYMBOL_AUTOMATIC_MENU);
    ids
}

/// Whether `command` names the hue or shape `node` already carries, or
/// whether `node` is already on its automatic look. `None` for every
/// command that is about something else.
fn symbol_style_checked(command: &str, node: &MathNode) -> Option<bool> {
    let (role, id) = match node {
        MathNode::Sym(ch) if ch.is_alphabetic() => (SymbolRole::Variable, ch.to_string()),
        MathNode::Resolved { id, role, .. } => (*role, id.clone()),
        _ => return None,
    };
    if command == "context.symbol.automatic" {
        return Some(math_style::overrides().get(&id).is_empty());
    }
    let current = math_style::resolve(role, &id);
    if let Some(hue) = command
        .strip_prefix("context.hue.")
        .and_then(MathHue::from_keyword)
    {
        return Some(hue == current.hue);
    }
    command
        .strip_prefix("context.shape.")
        .and_then(HighlightShape::from_keyword)
        .map(|shape| shape == current.shape)
}

/// How each section of the inspector arranges its cells. A choice whose
/// whole content is how it looks belongs in a grid — ten hues read as a
/// palette, where ten rows spelling colours out read as a list.
fn shelf_for(group: &str) -> symbol_menu::Shelf {
    match group {
        "Colour" => symbol_menu::Shelf::Grid {
            columns: 5,
            height: 28.0,
        },
        "Highlight" => symbol_menu::Shelf::Grid {
            columns: 3,
            height: 34.0,
        },
        "Variant" => symbol_menu::Shelf::Grid {
            columns: 5,
            height: 32.0,
        },
        _ => symbol_menu::Shelf::Rows,
    }
}

/// What one cell of the inspector draws: the style the symbol would carry if
/// that cell were chosen. `pinned` is what the reader has fixed by hand, and
/// it survives every choice that does not name the same axis — so picking a
/// role does not quietly undo a shape they set.
fn symbol_art(
    command: &str,
    current: math_style::SymbolStyle,
    pinned: math_style::Override,
    automatic: math_style::SymbolStyle,
) -> symbol_menu::Art {
    use math_style::SymbolStyle;
    let preview = symbol_menu::Art::Preview;
    if command == "context.symbol.automatic" {
        return preview(automatic);
    }
    if let Some(role) = command
        .strip_prefix("context.symbol.")
        .and_then(SymbolRole::from_keyword)
    {
        return preview(SymbolStyle {
            hue: current.hue,
            shape: pinned.shape.unwrap_or_else(|| math_style::role_shape(role)),
        });
    }
    if let Some(hue) = command
        .strip_prefix("context.hue.")
        .and_then(MathHue::from_keyword)
    {
        // A swatch is a colour sample, so it shows the hue whole — fill and
        // edge — whatever shape the symbol is currently wearing. Previewing
        // an outlined symbol here would leave ten hues to be judged from
        // ten hairlines, which is the one thing a palette must not ask.
        return preview(SymbolStyle {
            hue,
            shape: HighlightShape::Both,
        });
    }
    if let Some(shape) = command
        .strip_prefix("context.shape.")
        .and_then(HighlightShape::from_keyword)
    {
        return preview(SymbolStyle {
            hue: current.hue,
            shape,
        });
    }
    // A variant is a letterform, and a wash over it would hide the one
    // thing being chosen.
    symbol_menu::Art::Bare
}

fn role_label(role: SymbolRole) -> &'static str {
    match role {
        SymbolRole::Variable => "Variable",
        SymbolRole::Constant => "Constant",
        SymbolRole::Function => "Function",
    }
}

/// The letterform a variant command would spell `identity` with. `plain` is
/// the symbol's own base; everything else comes out of the mathematical
/// alphanumeric block.
fn variant_glyph(command: &str, identity: &str, current: &str) -> String {
    let Some(key) = command.strip_prefix("context.variant.") else {
        return current.to_string();
    };
    let base = math_symbols::exact(identity)
        .map(|symbol| symbol.glyph)
        .or_else(|| {
            let mut chars = identity.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Some(c),
                _ => None,
            }
        });
    let Some(base) = base else {
        return current.to_string();
    };
    if key == "plain" {
        return base.to_string();
    }
    math_symbols::variants(base)
        .into_iter()
        .find(|variant| variant.key == key)
        .map(|variant| variant.glyph.to_string())
        .unwrap_or_else(|| base.to_string())
}

/// The same, plus the letterforms a single letter can be spelled with.
fn symbol_context_ids(glyph: char) -> Vec<&'static str> {
    let mut ids = symbol_style_ids();
    ids.push("context.variant.plain");
    ids.extend(
        crate::document::math_symbols::variants(glyph)
            .into_iter()
            .filter_map(|variant| match variant.key {
                "bold" => Some("context.variant.bold"),
                "italic" => Some("context.variant.italic"),
                "bold_italic" => Some("context.variant.bold_italic"),
                "sans" => Some("context.variant.sans"),
                "sans_bold" => Some("context.variant.sans_bold"),
                "sans_italic" => Some("context.variant.sans_italic"),
                "sans_bold_italic" => Some("context.variant.sans_bold_italic"),
                "monospace" => Some("context.variant.monospace"),
                _ => None,
            }),
    );
    ids
}

fn symbol_base_glyph(list: &[MathNode], current_variant: &str) -> Option<char> {
    for node in list {
        if let MathNode::Sym(glyph) = node {
            if current_variant == "plain"
                && !crate::document::math_symbols::variants(*glyph).is_empty()
            {
                return Some(*glyph);
            }
            if let Some(base) = crate::document::math_symbols::SYMBOLS
                .iter()
                .find_map(|symbol| {
                    crate::document::math_symbols::variants(symbol.glyph)
                        .into_iter()
                        .any(|variant| variant.key == current_variant && variant.glyph == *glyph)
                        .then_some(symbol.glyph)
                })
            {
                return Some(base);
            }
        }
        for slot in node.slots() {
            if let Some(base) = symbol_base_glyph(
                node.slot(slot)
                    .expect("a node's reported slots must resolve"),
                current_variant,
            ) {
                return Some(base);
            }
        }
    }
    None
}

fn moved_math_menu_selection(selected: usize, count: usize, delta: isize) -> usize {
    if count == 0 {
        return 0;
    }
    ((selected as isize + delta).clamp(0, count as isize - 1)) as usize
}

/// Fold a stroke's swept hits into the selection. A hit already in `inside`
/// is skipped, and one already in `selected` is not pushed again, so a stroke
/// only ever grows the selection: re-entering a target never deselects it,
/// and no target appears twice. `inside` is refreshed to the current circle.
fn add_brush_hits(
    selected: &mut Vec<ContextHit>,
    inside: &mut Vec<ContextHit>,
    swept_hits: Vec<ContextHit>,
    current_hits: Vec<ContextHit>,
) {
    for hit in swept_hits.iter().filter(|hit| !inside.contains(hit)) {
        if !selected.contains(hit) {
            selected.push(hit.clone());
        }
    }
    *inside = current_hits;
}

/// Split what a formatting surface is aimed at into the prose ranges a text
/// style lands on and a count of the targets it has to leave alone.
///
/// One target is styled whatever its kind: the surface over it offers only
/// what that kind can take, and un-badging a badge or un-coding an
/// inline-code run are its own rows. A brush selection is several things at
/// once, and a text style reaches only the words in it — math, code and
/// boxed runs are counted instead, for the warning that goes up before
/// anything changes.
fn style_targets(targets: &[ContextHit]) -> (Vec<FlatRange>, usize) {
    if let [ContextHit::Range { range, .. }] = targets {
        return (vec![*range], 0);
    }
    let mut ranges = Vec::new();
    let mut skipped = 0;
    for target in targets {
        match target {
            ContextHit::Range {
                range,
                kind: RangeKind::Word,
            } => ranges.push(*range),
            _ => skipped += 1,
        }
    }
    (ranges, skipped)
}

/// Apply one text style to every range, as a single undo step.
///
/// A selection where some words already carry the style and some do not is
/// made uniform rather than inverted word by word: one press turns the style
/// on unless every range already has it. Toggling each range against its own
/// state would leave the reader looking at a selection half of which went
/// bold and half of which went plain, from one press of one button.
fn apply_style_to_ranges(docs: &mut Tabs, ranges: &[FlatRange], mask: Style) {
    if ranges.is_empty() {
        return;
    }
    let Some(document) = docs.active().map(|tab| &tab.document) else {
        return;
    };
    let enable = !ranges
        .iter()
        .all(|range| document.style_range_is_active(*range, mask));
    docs.transaction(|docs| {
        for range in ranges {
            let active = docs
                .active()
                .is_some_and(|tab| tab.document.style_range_is_active(*range, mask));
            if active != enable {
                docs.toggle_style_range(*range, mask);
            }
        }
    });
}

fn brush_sweep_samples(from: (f32, f32), to: (f32, f32), radius: f32) -> Vec<(f32, f32)> {
    let dx = to.0 - from.0;
    let dy = to.1 - from.1;
    let steps = (dx.hypot(dy) / radius).ceil() as usize;
    (1..steps)
        .map(|step| {
            let t = step as f32 / steps as f32;
            (from.0 + dx * t, from.1 + dy * t)
        })
        .collect()
}

fn reset_brush_selection(
    selected: &mut Vec<ContextHit>,
    inside: &mut Vec<ContextHit>,
    point: &mut Option<(f32, f32)>,
) -> bool {
    let changed = !selected.is_empty() || !inside.is_empty() || point.is_some();
    selected.clear();
    inside.clear();
    *point = None;
    changed
}

fn finder_input(state: &mut super::FinderState, input: &Input) -> bool {
    let mut changed = false;
    if !input.text().is_empty() {
        state.query.push_str(input.text());
        state.selected = 0;
        changed = true;
    }
    if input.is_key_typed(KeyCode::Backspace) && state.query.pop().is_some() {
        state.selected = 0;
        changed = true;
    }
    let visible = state.rows.len();
    if input.is_key_typed(KeyCode::ArrowDown) && visible > 0 {
        state.selected = (state.selected + 1).min(visible - 1);
        changed = true;
    }
    if input.is_key_typed(KeyCode::ArrowUp) && state.selected > 0 {
        state.selected -= 1;
        changed = true;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::{
        BodyCoordinate, Shell, add_brush_hits, apply_style_to_ranges, brush_sweep_samples,
        delete_chars, delete_inside_math, enter_insert, focus_note, focus_note_at, math_menu_rows,
        math_menu_variant_start, move_inside_math, moved_math_menu_selection, note_at_caret,
        note_at_point, paste, reset_brush_selection, return_to_anchor, set_body_coordinate_caret,
        style_targets, symbol_base_glyph, symbol_context_ids,
    };
    use crate::document::layout::{ContextHit, RangeKind};
    use crate::document::math::{AccentKind, MathNode, NodeAddress, SymbolRole};
    use crate::document::{
        Block, Caret, Document, FlatPos, FlatRange, Focus, Inline, Style, Text, math_conversion,
        math_symbols,
    };
    use crate::tabs::Tabs;
    use crate::vim::{Key, Mode, Motion, Vim, motion};

    #[test]
    fn symbol_context_keeps_roles_first_and_offers_only_valid_variants() {
        let latin = symbol_context_ids('x');
        assert_eq!(&latin[..3], super::commands::SYMBOL_ROLE_MENU);
        assert!(latin.contains(&"context.variant.plain"));
        assert!(latin.contains(&"context.variant.bold"));

        let italic_alpha = math_symbols::variants('α')
            .into_iter()
            .find(|variant| variant.key == "italic")
            .unwrap()
            .glyph;
        assert_eq!(
            symbol_base_glyph(&[MathNode::Sym(italic_alpha)], "italic"),
            Some('α')
        );
        let greek = symbol_context_ids('α');
        assert!(greek.contains(&"context.variant.plain"));
        assert!(greek.contains(&"context.variant.italic"));
        assert!(!greek.contains(&"context.variant.sans"));
    }

    #[test]
    fn a_symbol_already_set_to_the_constant_role_opens_the_role_menu_with_that_row_checked() {
        let mut document = Document::new(Path::new("test-context-role.md"));
        document.body_mut()[0] = Block::Paragraph(vec![Inline::Math(vec![MathNode::Resolved {
            id: "pi".into(),
            role: SymbolRole::Constant,
            variant: "plain".into(),
            body: vec![MathNode::Sym('π')],
        }])]);
        let target = ContextHit::Math {
            block: 0,
            inline: 0,
            offset: 0,
            node: Some(NodeAddress {
                path: Vec::new(),
                index: 0,
            }),
        };
        let checked: Vec<bool> = super::commands::SYMBOL_ROLE_MENU
            .iter()
            .map(|id| Shell::context_command_checked_for(&document, target.clone(), id))
            .collect();

        for (index, id) in super::commands::SYMBOL_ROLE_MENU.iter().enumerate() {
            assert_eq!(checked[index], *id == "context.symbol.constant");
        }
    }

    #[test]
    fn an_accent_menu_opened_on_a_hat_shows_the_hat_row_checked_and_the_others_unchecked() {
        let mut document = Document::new(Path::new("test-context-hat.md"));
        document.body_mut()[0] = Block::Paragraph(vec![Inline::Math(vec![MathNode::Accent {
            kind: AccentKind::Hat,
            body: vec![MathNode::Sym('x')],
        }])]);
        let target = ContextHit::Math {
            block: 0,
            inline: 0,
            offset: 0,
            node: Some(NodeAddress {
                path: Vec::new(),
                index: 0,
            }),
        };
        let checked: Vec<bool> = super::commands::ACCENT_MENU
            .iter()
            .map(|id| Shell::context_command_checked_for(&document, target.clone(), id))
            .collect();

        for (index, id) in super::commands::ACCENT_MENU.iter().enumerate() {
            assert_eq!(checked[index], *id == "context.accent.hat");
        }
    }

    #[test]
    fn a_group_menu_opened_on_a_bar_group_shows_the_bar_row_checked() {
        let mut document = Document::new(Path::new("test-context-bars.md"));
        document.body_mut()[0] = Block::Paragraph(vec![Inline::Math(vec![MathNode::Group {
            open: '|',
            close: '|',
            body: vec![MathNode::Sym('x')],
        }])]);
        let target = ContextHit::Math {
            block: 0,
            inline: 0,
            offset: 0,
            node: Some(NodeAddress {
                path: Vec::new(),
                index: 0,
            }),
        };
        let checked: Vec<bool> = super::commands::GROUP_MENU
            .iter()
            .map(|id| Shell::context_command_checked_for(&document, target.clone(), id))
            .collect();

        for (index, id) in super::commands::GROUP_MENU.iter().enumerate() {
            assert_eq!(checked[index], *id == "context.group.bars");
        }
    }

    #[test]
    fn brush_reentering_a_target_does_not_select_it_twice() {
        let target = ContextHit::Range {
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
        let mut selected = Vec::new();
        let mut inside = Vec::new();

        add_brush_hits(
            &mut selected,
            &mut inside,
            vec![target.clone()],
            vec![target.clone()],
        );
        assert_eq!(selected, vec![target.clone()]);
        add_brush_hits(
            &mut selected,
            &mut inside,
            vec![target.clone()],
            vec![target.clone()],
        );
        assert_eq!(selected, vec![target.clone()]);

        add_brush_hits(&mut selected, &mut inside, Vec::new(), Vec::new());
        add_brush_hits(
            &mut selected,
            &mut inside,
            vec![target.clone()],
            vec![target.clone()],
        );
        assert_eq!(selected, vec![target]);
    }

    #[test]
    fn brush_click_preserves_unrelated_selected_targets() {
        let word = |offset| ContextHit::Range {
            range: FlatRange::new(
                FlatPos { block: 0, offset },
                FlatPos {
                    block: 0,
                    offset: offset + 1,
                },
            ),
            kind: RangeKind::Word,
        };
        let (first, second, third) = (word(0), word(2), word(4));
        let mut selected = vec![first.clone(), second.clone()];
        let mut inside = Vec::new();

        add_brush_hits(
            &mut selected,
            &mut inside,
            vec![third.clone()],
            vec![third.clone()],
        );
        assert_eq!(selected, vec![first.clone(), second.clone(), third.clone()]);
        add_brush_hits(&mut selected, &mut inside, Vec::new(), Vec::new());
        add_brush_hits(
            &mut selected,
            &mut inside,
            vec![first.clone()],
            vec![first.clone()],
        );
        assert_eq!(selected, vec![first, second, third]);
    }

    #[test]
    fn fast_brush_motion_is_sampled_no_farther_apart_than_its_radius() {
        let radius = 18.0;
        let from = (0.0, 5.0);
        let to = (100.0, 5.0);
        let mut points = vec![from];
        points.extend(brush_sweep_samples(from, to, radius));
        points.push(to);

        assert!(points.len() > 2);
        assert!(points.windows(2).all(|pair| {
            let dx: f32 = pair[1].0 - pair[0].0;
            let dy: f32 = pair[1].1 - pair[0].1;
            dx.hypot(dy) <= radius
        }));
    }

    #[test]
    fn swept_hits_add_once_but_inside_tracks_only_the_current_circle() {
        let word = |offset| ContextHit::Range {
            range: FlatRange::new(
                FlatPos { block: 0, offset },
                FlatPos {
                    block: 0,
                    offset: offset + 1,
                },
            ),
            kind: RangeKind::Word,
        };
        let (crossed, current) = (word(0), word(2));
        let mut selected = Vec::new();
        let mut inside = Vec::new();

        add_brush_hits(
            &mut selected,
            &mut inside,
            vec![crossed.clone(), current.clone()],
            vec![current.clone()],
        );

        assert_eq!(selected, vec![crossed.clone(), current.clone()]);
        assert_eq!(inside, vec![current]);

        add_brush_hits(
            &mut selected,
            &mut inside,
            vec![crossed.clone()],
            vec![crossed.clone()],
        );
        assert_eq!(selected, vec![crossed.clone(), word(2)]);
        assert_eq!(inside, vec![crossed]);
    }

    fn word(block: usize, start: usize, end: usize) -> ContextHit {
        ContextHit::Range {
            range: FlatRange::new(
                FlatPos {
                    block,
                    offset: start,
                },
                FlatPos { block, offset: end },
            ),
            kind: RangeKind::Word,
        }
    }

    fn span(block: usize, start: usize, end: usize) -> FlatRange {
        FlatRange::new(
            FlatPos {
                block,
                offset: start,
            },
            FlatPos { block, offset: end },
        )
    }

    /// The brush picks up whatever the circle touched, and a stroke across a
    /// line of prose with an equation in it takes both. Bold has nothing to
    /// say to a math node, so the words go through and the rest is counted —
    /// that count is the whole basis of the warning the bar puts up.
    #[test]
    fn a_style_reaches_the_words_in_a_mixed_selection_and_counts_the_rest() {
        let targets = vec![
            word(0, 0, 9),
            ContextHit::Math {
                block: 0,
                inline: 1,
                offset: 9,
                node: Some(NodeAddress {
                    path: Vec::new(),
                    index: 0,
                }),
            },
            word(0, 12, 15),
            ContextHit::Range {
                range: span(2, 0, 4),
                kind: RangeKind::CodeBlock,
            },
        ];
        let (ranges, skipped) = style_targets(&targets);
        assert_eq!(ranges.len(), 2, "both words are styled: {ranges:?}");
        assert_eq!(ranges[0].start.offset, 0);
        assert_eq!(ranges[1].start.offset, 12);
        assert_eq!(skipped, 2, "the math node and the code block are counted");
    }

    /// A single click is not a mixed selection, whatever it landed on: the
    /// surface over one target offers only what that target can take, so a
    /// badge's own menu still un-badges it and nothing is ever counted out.
    #[test]
    fn one_target_is_styled_whatever_kind_it_is() {
        for kind in [
            RangeKind::Word,
            RangeKind::Badge,
            RangeKind::InlineCode,
            RangeKind::CodeBlock,
        ] {
            let (ranges, skipped) = style_targets(&[ContextHit::Range {
                range: span(0, 0, 3),
                kind,
            }]);
            assert_eq!(ranges.len(), 1, "{kind:?} is styled on its own");
            assert_eq!(skipped, 0, "{kind:?} is not counted out on its own");
        }
    }

    /// One press of one button has one meaning. A selection where only some
    /// words are already bold goes all-bold rather than inverting each word
    /// against itself, which would leave the reader looking at a selection
    /// they had just asked to make bold with half of it plain.
    #[test]
    fn a_style_over_a_half_formatted_selection_makes_it_uniform() {
        let mut tabs = insert_tabs("uniform-style", "alpha beta gamma\n");
        let bold = Style {
            bold: true,
            ..Style::PLAIN
        };
        let (alpha, gamma) = (span(0, 0, 5), span(0, 11, 16));
        tabs.toggle_style_range(alpha, bold);
        {
            let document = &tabs.active().unwrap().document;
            assert!(
                document.style_range_is_active(alpha, bold)
                    && !document.style_range_is_active(gamma, bold),
                "the selection starts half bold"
            );
        }

        apply_style_to_ranges(&mut tabs, &[alpha, gamma], bold);
        let document = &tabs.active().unwrap().document;
        assert!(
            document.style_range_is_active(alpha, bold)
                && document.style_range_is_active(gamma, bold),
            "a half-bold selection turns fully bold, it does not swap halves"
        );
    }

    /// The second press is the one that takes it off, and it takes it off
    /// everywhere: fully formatted is the only state a toggle reads as
    /// "already on".
    #[test]
    fn a_style_over_a_fully_formatted_selection_clears_it() {
        let mut tabs = insert_tabs("clear-style", "alpha beta gamma\n");
        let italic = Style {
            italic: true,
            ..Style::PLAIN
        };
        let (alpha, gamma) = (span(0, 0, 5), span(0, 11, 16));
        apply_style_to_ranges(&mut tabs, &[alpha, gamma], italic);
        apply_style_to_ranges(&mut tabs, &[alpha, gamma], italic);
        let document = &tabs.active().unwrap().document;
        assert!(
            !document.style_range_is_active(alpha, italic)
                && !document.style_range_is_active(gamma, italic),
            "a second press clears every word it set"
        );
    }

    /// Several words formatted together came from one gesture, so they undo
    /// together too — one press, one step back.
    #[test]
    fn formatting_a_selection_is_one_undo_step() {
        let mut tabs = insert_tabs("undo-style", "alpha beta gamma\n");
        let bold = Style {
            bold: true,
            ..Style::PLAIN
        };
        let (alpha, gamma) = (span(0, 0, 5), span(0, 11, 16));
        apply_style_to_ranges(&mut tabs, &[alpha, gamma], bold);
        tabs.undo();
        let document = &tabs.active().unwrap().document;
        assert!(
            !document.style_range_is_active(alpha, bold)
                && !document.style_range_is_active(gamma, bold),
            "one undo takes the whole selection back"
        );
    }

    #[test]
    fn escape_reset_clears_the_entire_brush_selection() {
        let target = ContextHit::Math {
            block: 2,
            inline: 0,
            offset: 0,
            node: None,
        };
        let mut selected = vec![target.clone()];
        let mut inside = vec![target];
        let mut point = Some((20.0, 30.0));

        assert!(reset_brush_selection(
            &mut selected,
            &mut inside,
            &mut point
        ));
        assert!(selected.is_empty());
        assert!(inside.is_empty());
        assert_eq!(point, None);
        assert!(!reset_brush_selection(
            &mut selected,
            &mut inside,
            &mut point
        ));
    }
    use std::fs;
    use std::path::{Path, PathBuf};

    fn math_tabs(tag: &str) -> Tabs {
        let path: PathBuf =
            std::env::temp_dir().join(format!("tw-shell-math-{tag}-{}.md", std::process::id()));
        fs::write(&path, "").unwrap();
        let mut tabs = Tabs::new();
        tabs.open_full(&path);
        tabs.insert_inline_math();
        tabs
    }

    fn insert_tabs(tag: &str, content: &str) -> Tabs {
        let path: PathBuf =
            std::env::temp_dir().join(format!("tw-shell-insert-{tag}-{}.md", std::process::id()));
        fs::write(&path, content).unwrap();
        let mut tabs = Tabs::new();
        tabs.open_full(&path);
        tabs
    }

    /// Feed text through the same structural-marker layer Insert mode uses.
    /// The shell's Vim pass-through check is intentionally absent here: all
    /// of these fixture characters are ordinary Insert text.
    fn type_shortcuts(tabs: &mut Tabs, text: &str) -> Vec<super::InsertEvent> {
        let mut shortcuts = super::InsertShortcuts::default();
        let mut repeat = Vec::new();
        for c in text.chars() {
            if !shortcuts.apply(c, tabs, &mut repeat) {
                tabs.type_text(&c.to_string());
                repeat.push(super::InsertEvent::Text(c));
            }
        }
        repeat
    }

    fn text_style(run: &Inline) -> (&str, Style) {
        match run {
            Inline::Text(text) => (&text.text, text.style),
            _ => panic!("expected text run"),
        }
    }

    #[test]
    fn insert_markers_make_heading_levels_without_leaving_hashes_in_text() {
        let mut first = insert_tabs("marker-h1", "");
        type_shortcuts(&mut first, "#Title");
        assert!(matches!(
            first.active().unwrap().document.body()[0],
            Block::Heading { level: 1, .. }
        ));
        assert_eq!(first.active().unwrap().document.block_text(0), "Title");

        let mut second = insert_tabs("marker-h2", "");
        type_shortcuts(&mut second, "##Subtitle");
        assert!(matches!(
            second.active().unwrap().document.body()[0],
            Block::Heading { level: 2, .. }
        ));
        assert_eq!(second.active().unwrap().document.block_text(0), "Subtitle");
    }

    #[test]
    fn insert_star_markers_follow_typewritters_emphasis_convention() {
        let mut bold = insert_tabs("marker-bold", "");
        type_shortcuts(&mut bold, "*strong");
        let bold_run = &bold.active().unwrap().document.body()[0].inlines()[0];
        let (text, style) = text_style(bold_run);
        assert!(style.bold);
        assert!(!style.italic);
        assert_eq!(text, "strong");

        let mut italic = insert_tabs("marker-italic", "");
        type_shortcuts(&mut italic, "**slanted");
        let italic_run = &italic.active().unwrap().document.body()[0].inlines()[0];
        let (text, style) = text_style(italic_run);
        assert!(!style.bold);
        assert!(style.italic);
        assert_eq!(text, "slanted");

        let mut both = insert_tabs("marker-both", "");
        type_shortcuts(&mut both, "***both");
        let both_run = &both.active().unwrap().document.body()[0].inlines()[0];
        let (text, style) = text_style(both_run);
        assert!(style.bold && style.italic);
        assert_eq!(text, "both");
    }

    #[test]
    fn paired_inline_markers_leave_only_the_structural_content() {
        let mut tabs = insert_tabs("marker-inline", "");
        type_shortcuts(&mut tabs, "[[TODO]] ==important== `code`");
        let runs = tabs.active().unwrap().document.body()[0].inlines();
        let (text, style) = text_style(&runs[0]);
        assert_eq!(text, "TODO");
        assert!(style.badge);
        assert_eq!(text_style(&runs[1]).0, " ");
        let (text, style) = text_style(&runs[2]);
        assert_eq!(text, "important");
        assert!(style.highlight);
        assert_eq!(text_style(&runs[3]).0, " ");
        let (text, style) = text_style(&runs[4]);
        assert_eq!(text, "code");
        assert!(style.code);
    }

    #[test]
    fn structural_insert_markers_repeat_as_structural_edits() {
        let mut source = insert_tabs("marker-repeat-source", "");
        let repeat = type_shortcuts(&mut source, "[[TODO]] ==important==");

        let mut target = insert_tabs("marker-repeat-target", "");
        target.transaction(|tabs| {
            for event in &repeat {
                super::apply_insert_event(tabs, event);
            }
        });

        let runs = target.active().unwrap().document.body()[0].inlines();
        let (text, style) = text_style(&runs[0]);
        assert_eq!(text, "TODO");
        assert!(style.badge);
        assert_eq!(text_style(&runs[1]).0, " ");
        let (text, style) = text_style(&runs[2]);
        assert_eq!(text, "important");
        assert!(style.highlight);
    }

    #[test]
    fn block_markers_reach_the_existing_list_rule_and_code_operations() {
        let mut bullet = insert_tabs("marker-bullet", "");
        type_shortcuts(&mut bullet, "-item");
        assert!(matches!(
            bullet.active().unwrap().document.body()[0],
            Block::ListItem {
                marker: crate::document::ListMarker::Bullet,
                ..
            }
        ));
        assert_eq!(bullet.active().unwrap().document.block_text(0), "item");

        let mut numbered = insert_tabs("marker-numbered", "");
        type_shortcuts(&mut numbered, "12.item");
        assert!(matches!(
            numbered.active().unwrap().document.body()[0],
            Block::ListItem {
                marker: crate::document::ListMarker::Number(1),
                ..
            }
        ));
        assert_eq!(numbered.active().unwrap().document.block_text(0), "item");

        let mut task = insert_tabs("marker-task", "");
        type_shortcuts(&mut task, "- [ ]item");
        assert!(matches!(
            task.active().unwrap().document.body()[0],
            Block::ListItem {
                marker: crate::document::ListMarker::Task { done: false },
                ..
            }
        ));
        assert_eq!(task.active().unwrap().document.block_text(0), "item");

        let mut divider = insert_tabs("marker-divider", "");
        type_shortcuts(&mut divider, "---");
        assert!(matches!(
            divider.active().unwrap().document.body()[0],
            Block::Divider(_)
        ));
        assert!(matches!(
            divider.active().unwrap().document.body()[1],
            Block::Paragraph(_)
        ));

        let mut code = insert_tabs("marker-code", "");
        type_shortcuts(&mut code, "```let x");
        assert!(matches!(
            code.active().unwrap().document.body()[0],
            Block::CodeLine { first: true, .. }
        ));
        assert_eq!(code.active().unwrap().document.block_text(0), "let x");
    }

    #[test]
    fn sidenote_marker_uses_the_documents_existing_anchor_operation() {
        let mut tabs = insert_tabs("marker-sidenote", "");
        type_shortcuts(&mut tabs, "[^after");
        let document = &tabs.active().unwrap().document;
        assert_eq!(document.notes.len(), 1);
        assert!(matches!(document.body()[0].inlines()[0], Inline::Note(_)));
        assert_eq!(document.block_text(0), "\u{FFFC}after");
    }

    fn note_tabs(tag: &str) -> Tabs {
        insert_tabs(tag, "body[^1] tail\n\n[^1]: original\n")
    }

    fn note_text(tabs: &Tabs) -> String {
        tabs.active().unwrap().document.notes[0].body[0]
            .inlines()
            .iter()
            .map(|run| match run {
                Inline::Text(text) => text.text.as_str(),
                Inline::Math(_) | Inline::Note(_) | Inline::EqRef(_) => "\u{FFFC}",
            })
            .collect()
    }

    fn note_block(text: &str) -> Block {
        Block::Paragraph(vec![Inline::Text(Text {
            text: text.into(),
            style: Style::PLAIN,
        })])
    }

    fn fake_note_measure(text: &str, _: &crate::theme::TextStyle) -> f32 {
        text.chars().count() as f32 * 10.0
    }

    #[test]
    fn edit_sidenote_command_at_anchor_focuses_that_note() {
        let mut tabs = note_tabs("edit-anchor");
        tabs.move_caret_to(0, 1, 0);
        let index = note_at_caret(
            &tabs.active().unwrap().document,
            tabs.active().unwrap().document.caret,
        );
        assert_eq!(index, Some(0));

        focus_note(&mut tabs, index.unwrap());

        let doc = &tabs.active().unwrap().document;
        assert_eq!(doc.focus, Focus::Note(0));
        assert_eq!(note_text(&tabs), "original");
        assert_eq!(doc.caret.offset, "original".chars().count());
    }

    #[test]
    fn edit_sidenote_command_in_ordinary_prose_does_nothing() {
        let mut tabs = note_tabs("edit-prose");
        tabs.move_caret_to(0, 0, 0);
        let doc = &tabs.active().unwrap().document;

        assert_eq!(note_at_caret(doc, doc.caret), None);
        assert_eq!(doc.focus, Focus::Body);
    }

    #[test]
    fn typing_after_focusing_a_note_lands_in_note_body_not_prose() {
        let mut tabs = note_tabs("type-note");
        tabs.move_caret_to(0, 1, 0);
        focus_note(&mut tabs, 0);
        let body_before = tabs.active().unwrap().document.body()[0].clone();

        tabs.type_text(" edited");

        let doc = &tabs.active().unwrap().document;
        assert_eq!(doc.body()[0], body_before);
        assert_eq!(note_text(&tabs), "original edited");
        assert_eq!(doc.focus, Focus::Note(0));
    }

    #[test]
    fn escape_in_normal_mode_inside_note_returns_focus_to_body_at_anchor() {
        let mut tabs = note_tabs("escape-normal");
        tabs.move_caret_to(0, 1, 0);
        focus_note(&mut tabs, 0);
        let mut vim = Vim::new();

        let _ = vim.key_extended(Key::Escape);
        return_to_anchor(&mut tabs);

        let doc = &tabs.active().unwrap().document;
        assert_eq!(vim.mode(), Mode::Normal);
        assert_eq!(doc.focus, Focus::Body);
        assert_eq!(doc.caret.block, 0);
        assert_eq!(doc.caret.inline, 1);
        assert_eq!(doc.caret.offset, 0);
    }

    #[test]
    fn escape_in_insert_mode_inside_note_leaves_insert_mode_and_stays_in_note() {
        let mut tabs = note_tabs("escape-insert");
        tabs.move_caret_to(0, 1, 0);
        focus_note(&mut tabs, 0);
        let mut vim = Vim::new();
        vim.set_mode(Mode::Insert);

        let _ = vim.key_extended(Key::Escape);

        assert_eq!(vim.mode(), Mode::Normal);
        assert_eq!(tabs.active().unwrap().document.focus, Focus::Note(0));
    }

    #[test]
    fn clicking_a_note_in_the_margin_focuses_it_at_the_clicked_caret() {
        let layout = std::rc::Rc::new(crate::document::layout::layout_blocks(
            &[note_block("one two")],
            crate::components::sidenotes::NOTE_WIDTH,
            crate::components::sidenotes::SCALE,
            &fake_note_measure,
        ));
        let notes = vec![("1".into(), 40.0, layout.clone())];
        let point = (
            crate::components::sidenotes::NOTE_INSET + 45.0,
            40.0 + layout.blocks[0].lines[0].y + layout.blocks[0].lines[0].height / 2.0,
        );
        let (label, caret) = note_at_point(
            &notes,
            point.0,
            point.1,
            0.0,
            0.0,
            crate::components::sidenotes::NOTE_INSET,
            &fake_note_measure,
        )
        .unwrap();
        assert_eq!(label, "1");

        let mut tabs = note_tabs("click-note");
        focus_note_at(&mut tabs, 0, caret);
        let doc = &tabs.active().unwrap().document;
        assert_eq!(doc.focus, Focus::Note(0));
        assert!(doc.caret.offset > 0);
    }

    #[test]
    fn clicking_in_prose_while_a_note_is_focused_returns_focus_to_the_body_and_puts_the_caret_where_the_click_landed()
     {
        let mut tabs = note_tabs("click-prose");
        focus_note(&mut tabs, 0);

        set_body_coordinate_caret(
            &mut tabs,
            BodyCoordinate::Caret(Caret {
                block: 0,
                inline: 0,
                offset: 2,
                style: Style::PLAIN,
            }),
        );

        let doc = &tabs.active().unwrap().document;
        assert_eq!(doc.focus, Focus::Body);
        assert_eq!(
            (doc.caret.block, doc.caret.inline, doc.caret.offset),
            (0, 0, 2)
        );
    }

    #[test]
    fn typing_after_that_click_lands_in_the_prose_not_in_the_note() {
        let mut tabs = note_tabs("click-type");
        focus_note(&mut tabs, 0);
        set_body_coordinate_caret(
            &mut tabs,
            BodyCoordinate::Caret(Caret {
                block: 0,
                inline: 0,
                offset: 2,
                style: Style::PLAIN,
            }),
        );

        tabs.type_text("X");

        let doc = &tabs.active().unwrap().document;
        assert_eq!(doc.focus, Focus::Body);
        assert_eq!(doc.block_text(0), "boXdy\u{FFFC} tail");
        assert_eq!(note_text(&tabs), "original");
    }

    #[test]
    fn a_search_jump_while_a_note_is_focused_returns_focus_to_the_body_at_the_match() {
        let mut tabs = note_tabs("search-note");
        focus_note(&mut tabs, 0);
        tabs.jump_to_flat(0, 5);

        let doc = &tabs.active().unwrap().document;
        assert_eq!(doc.focus, Focus::Body);
        assert_eq!(
            doc.caret_position(),
            FlatPos {
                block: 0,
                offset: 5
            }
        );
    }

    #[test]
    fn a_motion_inside_a_note_stays_inside_the_notes_own_text() {
        let mut tabs = note_tabs("motion-note");
        focus_note(&mut tabs, 0);
        tabs.touch(|doc| doc.set_caret(0, 0, 0));

        motion::apply(
            &mut tabs.active_mut().unwrap().document,
            Motion::LastLine,
            1,
        );

        let doc = &tabs.active().unwrap().document;
        assert_eq!(doc.focus, Focus::Note(0));
        assert_eq!(doc.caret.block, 0);
        assert_eq!(doc.caret.offset, 0);
    }

    #[test]
    fn note_edit_while_a_note_is_already_focused_does_nothing() {
        let mut tabs = note_tabs("edit-note");
        focus_note(&mut tabs, 0);
        let before = tabs.active().unwrap().document.caret;

        let index = {
            let doc = &tabs.active().unwrap().document;
            note_at_caret(doc, doc.caret)
        };

        assert_eq!(index, None);
        assert_eq!(tabs.active().unwrap().document.focus, Focus::Note(0));
        assert_eq!(tabs.active().unwrap().document.caret, before);
    }

    #[test]
    fn every_way_into_insert_mode_opens_one_transaction() {
        // The `i` path (`InsertAt`) and the click-into-math path
        // (`Enter(Mode::Insert)`) both reach Insert mode through
        // `start_insert`, whose transaction-open is `enter_insert`. Drive it
        // directly here — `apply` itself needs a live Renderer — and check
        // one entry groups the whole session while a repeated entry does not
        // stack a second snapshot.
        let mut tabs = insert_tabs("i", "abc");
        let mut vim = Vim::new();
        tabs.move_caret_to(0, 0, 3);
        enter_insert(&mut tabs, &mut vim);
        assert_eq!(vim.mode(), Mode::Insert);
        tabs.type_text("d");
        tabs.type_text("e");
        tabs.end_transaction();
        assert_eq!(tabs.active().unwrap().document.block_text(0), "abcde");
        tabs.undo();
        assert_eq!(tabs.active().unwrap().document.block_text(0), "abc");
        tabs.undo();
        assert_eq!(
            tabs.active().unwrap().document.block_text(0),
            "abc",
            "the whole insert session undoes in one step"
        );

        // A second entry before Esc — the click handler firing Enter again
        // while already in Insert — must not open a nested transaction.
        let mut tabs = insert_tabs("click", "");
        let mut vim = Vim::new();
        enter_insert(&mut tabs, &mut vim);
        enter_insert(&mut tabs, &mut vim);
        tabs.type_text("x");
        tabs.type_text("y");
        tabs.end_transaction();
        assert_eq!(tabs.active().unwrap().document.block_text(0), "xy");
        tabs.undo();
        assert_eq!(tabs.active().unwrap().document.block_text(0), "");
    }

    #[test]
    fn inline_code_toggles_the_caret_context_without_dirtying() {
        let mut doc = Document::new(Path::new("test.md"));
        assert!(!doc.caret.style.code);
        doc.toggle_code();
        assert!(doc.caret.style.code);
        doc.toggle_code();
        assert!(!doc.caret.style.code);
        assert!(!doc.is_dirty());
    }

    #[test]
    fn directional_delete_exits_math_at_the_matching_outer_edge() {
        let mut tabs = math_tabs("delete-edges");
        tabs.math_type('x');
        tabs.math_left();

        assert!(delete_inside_math(&mut tabs, true).is_some());
        assert!(tabs.in_math(), "deleting content keeps the atom focused");
        assert!(delete_inside_math(&mut tabs, true).is_some());
        assert!(!tabs.in_math());
        assert_eq!(tabs.active().unwrap().document.caret.offset, 1);

        assert!(tabs.enter_math_before());
        assert!(delete_inside_math(&mut tabs, false).is_some());
        assert!(!tabs.in_math());
        assert_eq!(tabs.active().unwrap().document.caret.offset, 0);
    }

    #[test]
    fn normal_horizontal_motions_traverse_math_then_exit() {
        let mut tabs = math_tabs("normal-motion");
        tabs.math_type('x');
        tabs.math_type('y');

        assert!(move_inside_math(&mut tabs, Motion::Left, 2));
        assert_eq!(
            tabs.active().unwrap().document.math.as_ref().unwrap().index,
            0
        );
        assert!(move_inside_math(&mut tabs, Motion::Left, 1));
        assert!(!tabs.in_math());
        assert_eq!(tabs.active().unwrap().document.caret.offset, 0);

        assert!(tabs.enter_math_after());
        assert!(move_inside_math(&mut tabs, Motion::Right, 3));
        assert!(!tabs.in_math());
        assert_eq!(tabs.active().unwrap().document.caret.offset, 1);
    }

    #[test]
    fn normal_x_enters_an_atom_and_starts_structural_deletion() {
        let mut tabs = math_tabs("normal-delete");
        tabs.math_type('x');
        tabs.math_exit_before();

        delete_chars(&mut tabs, 1);

        assert!(tabs.in_math());
        assert!(matches!(
            tabs.active().unwrap().document.body()[0].inlines()[0],
            Inline::Math(ref list) if list.is_empty()
        ));
    }

    #[test]
    fn counted_normal_x_retries_after_exiting_math_at_end() {
        let mut tabs = math_tabs("counted-normal-delete");
        tabs.math_type('x');
        tabs.math_exit_after();
        tabs.type_text("abc");
        tabs.move_caret_to(0, 0, 0);

        delete_chars(&mut tabs, 2);

        let doc = &tabs.active().unwrap().document;
        assert!(!tabs.in_math());
        assert!(matches!(doc.body()[0].inlines()[0], Inline::Math(ref list) if list.is_empty()));
        assert!(matches!(
            doc.body()[0].inlines()[1],
            Inline::Text(ref text) if text.text == "bc"
        ));
    }

    #[test]
    fn menu_rows_include_explicit_compact_rewrites_and_named_offers() {
        let e0 = math_conversion::Query {
            path: Vec::new(),
            start: 0,
            end: 2,
            source: "e0".into(),
        };
        let rows = math_menu_rows(&math_conversion::offers(&e0).offers);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].name, "Vacuum permittivity");
        assert_eq!(rows[0].preview, "ε₀");

        let alpha = math_conversion::Query {
            source: "alpha".into(),
            end: 5,
            ..e0
        };
        let rows = math_menu_rows(&math_conversion::offers(&alpha).offers);
        assert!(rows.iter().any(|row| row.preview == "α"));
        let sqrt = math_conversion::Query {
            source: "sqrt".into(),
            end: 4,
            ..alpha
        };
        let rows = math_menu_rows(&math_conversion::offers(&sqrt).offers);
        assert!(rows.iter().any(|row| row.group == "Structure"));
    }

    #[test]
    fn math_menu_selection_walks_one_flat_list() {
        assert_eq!(moved_math_menu_selection(1, 8, 1), 2);
        assert_eq!(moved_math_menu_selection(2, 8, 5), 7);
        assert_eq!(moved_math_menu_selection(7, 8, 1), 7);
        assert_eq!(moved_math_menu_selection(0, 8, -1), 0);
    }

    #[test]
    fn exact_symbols_append_variant_cells_after_normal_rows() {
        let query = math_conversion::Query {
            path: Vec::new(),
            start: 0,
            end: 1,
            source: "x".into(),
        };
        let offers = math_conversion::offers(&query).offers;
        let variant_start = math_menu_variant_start(&offers);
        let rows = math_menu_rows(&offers);

        assert!(variant_start > 0);
        assert!(variant_start < rows.len());
        assert!(
            rows[variant_start..]
                .iter()
                .all(|row| !row.preview.is_empty())
        );
    }

    #[test]
    fn our_own_clipboard_text_pastes_as_notation() {
        let mut tabs = insert_tabs("own", "");
        tabs.set_yank("costs $a/b$ today".to_string());
        let before = tabs.layout_revision();
        paste(&mut tabs, false, 1, "costs $a/b$ today");
        assert!(tabs.layout_revision() > before);
        let runs = tabs.active().unwrap().document.body()[0].inlines();
        assert!(matches!(runs[0], Inline::Text(ref t) if t.text == "costs "));
        assert!(matches!(runs[1], Inline::Math(_)));
        assert!(matches!(runs[2], Inline::Text(ref t) if t.text == " today"));
    }

    #[test]
    fn foreign_clipboard_text_pastes_literally() {
        let mut tabs = insert_tabs("foreign", "");
        tabs.set_yank("the board costs $40 and the meter $12".to_string());
        paste(&mut tabs, false, 1, "");
        let runs = tabs.active().unwrap().document.body()[0].inlines();
        assert_eq!(runs.len(), 1);
        assert!(matches!(
            runs[0],
            Inline::Text(ref t) if t.text == "the board costs $40 and the meter $12"
        ));
    }
}
