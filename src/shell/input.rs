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
use crate::components::{
    ContextMenu, Dialog, FileFinder, FileTree, Onboarding, Palette, SlashMenu, context_menu,
    file_finder, file_tree, onboarding, title_bar,
};
use crate::config::Config;
use crate::document::{FlatPos, FlatRange, Style};
use crate::input::Input;
use crate::layout::Rect;
use crate::theme::{self, TextStyle};
use crate::vault::Vault;
use crate::vim::{
    Edit, ExtendedAction, Key, Mode, Motion, Operator, OperatorTarget, VimMode, VisualAction,
    VisualMode, motion,
};

use std::cell::RefCell;
use std::rc::Rc;

use super::{ContextMenuState, PaletteState, Shell, SlashMenuState};

impl Shell {
    /// Choosing a vault is a shell job: the picker persists the config and
    /// hands the open vault back to the tree. While onboarding, only that
    /// one command can run — matched through the table like any other chord
    /// — plus the Enter key and the splash button as equivalent ways to ask
    /// for it. Once a vault is open, every chord in [`commands`] dispatches
    /// normally, `Ctrl+O` included.
    pub(super) fn handle_input(&mut self, input: &Input, viewport: Rect) {
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
        // The context menu swallows input while it is open.
        if self.context_menu.is_some() {
            self.handle_context_menu_input(input, viewport);
            return;
        }
        // A dialog swallows everything else until it resolves.
        if self.dialog.is_some() {
            self.handle_dialog_input(input, viewport);
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

        if input.is_mouse_pressed(MouseButton::Left)
            && input.is_cursor_in_window()
            && title_bar::search_box_rect(
                self.regions[self.title_region].layer(),
                self.layout.rect(self.regions[self.title_region].node()),
            )
            .contains(input.mouse_position())
        {
            self.open_finder();
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

        if let Some(command) = commands::matching(input) {
            (command.run)(self);
            return;
        }

        // Past here the vim keymap owns the keyboard whether or not a file
        // is open — the leader has to reach the palette either way, and
        // `Tabs::touch`/`Tabs::edit` already no-op against an empty `Tabs`.
        self.edit_frame(input);
    }

    /// Scroll, click-to-place, and the arrow/Home/End keys need an actual
    /// buffer, so they're guarded on a tab being active. The vim keymap and
    /// the divider drag aren't: what a plain key means past scroll/arrows is
    /// mode-dependent, so it's split into
    /// [`Shell::edit_frame_insert`]/[`Shell::edit_frame_normal`].
    fn edit_frame(&mut self, input: &Input) {
        let rect = self.layout.rect(self.text_column);
        let mouse = input.mouse_position();
        let over_editor = input.is_cursor_in_window() && rect.contains(mouse);
        let has_tab = self.docs.borrow().active().is_some();

        if has_tab && over_editor && input.scroll_delta().1 != 0.0 {
            let max = self.editor_max_scroll();
            let scroll = self.docs.borrow().editor_scroll;
            let next = (scroll - input.scroll_delta().1 * crate::document::layout::LINE_BODY)
                .clamp(0.0, max);
            self.docs.borrow_mut().set_editor_scroll(next);
        }

        if has_tab && !self.vim.command_active() {
            self.arrow_keys(input);
        }

        match self.vim.current_mode() {
            VimMode::Insert => self.edit_frame_insert(input),
            _ => self.edit_frame_normal(input),
        }

        if has_tab
            && input.is_mouse_pressed(MouseButton::Left)
            && over_editor
            && let Some(caret) = self.caret_at(rect, mouse)
        {
            self.goal_x = None;
            self.docs
                .borrow_mut()
                .move_caret_to(caret.block, caret.inline, caret.offset);
        }

        self.divider_drag(input);
    }

    /// Arrow keys, Home, End. Vertical motion goes through the layout so it
    /// aims at a remembered goal x; horizontal through the model's style
    /// machine; Home/End are the visual line's text extremes. Vertical
    /// motion records the goal x (`goal_col` in pixels, spec §5); any
    /// non-vertical caret move clears it.
    fn arrow_keys(&mut self, input: &Input) {
        if input.is_key_typed(KeyCode::ArrowUp) || input.is_key_typed(KeyCode::ArrowDown) {
            let rect = self.layout.rect(self.text_column);
            let width = crate::components::editor::Editor::content_width(rect);
            let layout = self.current_layout(width);
            let layer = self.regions[self.text_region].layer();
            let measure = |text: &str, style: &TextStyle| theme::width(layer, text, style);
            let (goal, next) = {
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
                (goal, next)
            };
            self.goal_x = Some(goal);
            if let Some(next) = next {
                self.docs
                    .borrow_mut()
                    .move_caret_to(next.block, next.inline, next.offset);
            }
            return;
        }
        // Any other arrow clears the vertical goals.
        self.goal_x = None;
        if input.is_key_typed(KeyCode::ArrowLeft) {
            self.docs.borrow_mut().move_left();
        }
        if input.is_key_typed(KeyCode::ArrowRight) {
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
        // Bare / opens the slash menu — before the docs borrow so we can
        // call refresh_slash_menu (which needs &mut self) without a conflict.
        let text = input.text();
        if text == "/" {
            let anchor = self.compute_slash_anchor();
            self.slash_menu = Some(SlashMenuState {
                query: String::new(),
                selected: 0,
                anchor,
            });
            self.refresh_slash_menu();
            return;
        }
        {
            for c in input.text().chars() {
                if matches!(
                    self.vim.key_extended(Key::Char(c)),
                    ExtendedAction::Passthrough
                ) {
                    self.docs.borrow_mut().type_text(&c.to_string());
                    self.insert_repeat.push(super::InsertEvent::Text(c));
                }
            }
            if input.is_key_typed(KeyCode::Backspace) {
                let _ = self.vim.key_extended(Key::Backspace);
                self.docs.borrow_mut().backspace();
                self.insert_repeat.push(super::InsertEvent::Backspace);
            }
            if input.is_key_typed(KeyCode::Delete) {
                self.docs.borrow_mut().delete_forward();
                self.insert_repeat.push(super::InsertEvent::Delete);
            }
            if input.is_key_typed(KeyCode::Enter) {
                let _ = self.vim.key_extended(Key::Enter);
                self.docs.borrow_mut().newline();
                self.insert_repeat.push(super::InsertEvent::Enter);
            }
        }
        if input.is_key_pressed(KeyCode::Escape) {
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

    /// Normal and command modes consume resolved characters one at a time.
    fn edit_frame_normal(&mut self, input: &Input) {
        for c in input.text().chars() {
            if !self.vim.command_active() && c == '/' && self.vim.visual_mode().is_some() {
                let anchor = self.compute_slash_anchor();
                self.slash_menu = Some(SlashMenuState {
                    query: String::new(),
                    selected: 0,
                    anchor,
                });
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
            if self.vim.command_active() {
                self.search = None;
            }
            let action = self.vim.key_extended(Key::Escape);
            self.apply(action);
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
                self.docs
                    .borrow_mut()
                    .touch(|doc| motion::apply(doc, m, count));
                self.goal_x = None;
            }
            ExtendedAction::InsertAt(m) => {
                self.docs.borrow_mut().begin_transaction();
                self.docs.borrow_mut().touch(|doc| motion::apply(doc, m, 1));
                self.vim.set_mode(Mode::Insert);
                self.insert_repeat.clear();
                self.insert_prefix = None;
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
                self.vim.set_mode(mode);
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
                self.import_yank();
                self.docs.borrow_mut().paste(before, count);
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
                .blocks
                .iter()
                .map(|block| {
                    block
                        .inlines()
                        .iter()
                        .map(|run| match run {
                            crate::document::Inline::Text(text) => text.text.as_str(),
                            crate::document::Inline::Math(_) => "\u{FFFC}",
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
            .touch(|doc| doc.set_flat_position(position));
    }

    fn start_insert(&mut self) {
        self.insert_repeat.clear();
        self.vim.set_mode(Mode::Insert);
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
        self.import_yank();
        self.docs.borrow_mut().paste(false, 1);
    }

    fn exit_visual(&mut self) {
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
                self.import_yank();
                self.docs.borrow_mut().paste(before, count);
            }
            super::RepeatOp::Insert(events) => {
                self.docs.borrow_mut().transaction(|docs| {
                    for event in events {
                        match event {
                            super::InsertEvent::Text(c) => docs.type_text(&c.to_string()),
                            super::InsertEvent::Backspace => docs.backspace(),
                            super::InsertEvent::Delete => docs.delete_forward(),
                            super::InsertEvent::Enter => docs.newline(),
                        }
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
            let next = {
                let docs = self.docs.borrow();
                let Some(tab) = docs.active() else {
                    break;
                };
                let caret = last.unwrap_or(tab.document.caret);
                let goal = self
                    .goal_x
                    .unwrap_or_else(|| layout.caret_pos(caret, &measure).0);
                self.goal_x = Some(goal);
                if up {
                    layout.line_up(caret, goal, &measure)
                } else {
                    layout.line_down(caret, goal, &measure)
                }
            };
            let Some(next) = next else {
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
                let mut docs = self.docs.borrow_mut();
                docs.transaction(|docs| {
                    for _ in 0..count {
                        docs.delete_char();
                    }
                });
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
        self.refresh_palette();
    }

    fn close_palette(&mut self) {
        self.palette = None;
        self.refresh_palette();
    }

    /// Rebuilds the palette region from the live query/selection, the way
    /// `refresh_dialog` does for the dialog.
    fn refresh_palette(&mut self) {
        let palette = match &self.palette {
            Some(state) => Palette::new(commands::entries(), state.query.clone(), state.selected),
            None => Palette::closed(),
        };
        self.regions[self.palette_region].set_component(Box::new(palette));
    }

    fn handle_palette_input(&mut self, input: &Input) {
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
        let visible = palette::filter(&entries, &state.query);
        let command = visible
            .get(state.selected)
            .and_then(|&i| commands::COMMANDS.get(i));
        self.close_palette();
        if let Some(command) = command {
            (command.run)(self);
        }
    }

    // ---- file finder --------------------------------------------------

    pub(super) fn open_finder(&mut self) {
        let Some(vault) = &self.vault else {
            return;
        };
        self.finder = Some(super::FileFinderState {
            files: vault.borrow().files(),
            query: String::new(),
            selected: 0,
        });
        self.refresh_finder();
    }

    fn close_finder(&mut self) {
        self.finder = None;
        self.refresh_finder();
    }

    fn refresh_finder(&mut self) {
        let query = self.finder.as_ref().map(|state| state.query.clone());
        self.regions[self.title_region].set_component(Box::new(title_bar::TitleBar::new(
            "Typewritter",
            "LECTURE CAPTURE",
            query,
        )));
        let finder = match &self.finder {
            Some(state) => {
                FileFinder::new(state.files.clone(), state.query.clone(), state.selected)
            }
            None => FileFinder::closed(),
        };
        self.regions[self.finder_region].set_component(Box::new(finder));
    }

    fn handle_finder_input(&mut self, input: &Input) {
        if input.is_key_pressed(KeyCode::Escape) {
            self.close_finder();
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
            self.refresh_finder();
        }
    }

    fn open_selected_finder_file(&mut self) {
        let path = self
            .finder
            .as_ref()
            .and_then(|state| file_finder::path_at(&state.files, &state.query, state.selected));
        let Some(path) = path else {
            return;
        };
        self.docs.borrow_mut().open_preview(&path);
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
        let screen_x = rect.x + crate::components::editor::INSET + caret_x;
        let screen_y =
            rect.y + crate::components::editor::TOP + caret_baseline - scroll + caret_height / 2.0;
        (screen_x, screen_y)
    }

    fn refresh_slash_menu(&mut self) {
        let menu = match &self.slash_menu {
            Some(state) => SlashMenu::new(
                commands::editor_entries(),
                state.query.clone(),
                state.selected,
                state.anchor,
            ),
            None => SlashMenu::closed(),
        };
        self.regions[self.slash_region].set_component(Box::new(menu));
    }

    // ---- context menu ----------------------------------------------------

    fn open_context_menu(&mut self, ids: &[&str], anchor: (f32, f32)) {
        let items = commands::menu(ids);
        if items.is_empty() {
            return;
        }
        self.context_menu = Some(ContextMenuState {
            items,
            selected: 0,
            anchor,
        });
        self.refresh_context_menu();
    }

    fn refresh_context_menu(&mut self) {
        let menu = match &self.context_menu {
            Some(state) => {
                let ids: Vec<&str> = state.items.iter().map(|command| command.id).collect();
                ContextMenu::new(commands::menu_entries(&ids), state.selected, state.anchor)
            }
            None => ContextMenu::closed(),
        };
        self.regions[self.menu_region].set_component(Box::new(menu));
    }

    fn close_context_menu(&mut self) {
        self.context_menu = None;
        self.refresh_context_menu();
    }

    fn handle_context_menu_input(&mut self, input: &Input, viewport: Rect) {
        if input.is_key_pressed(KeyCode::Escape) {
            self.close_context_menu();
            return;
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
            return;
        }

        let Some(state) = &self.context_menu else {
            return;
        };
        let card = context_menu::card_anchored(viewport, state.anchor, state.items.len());
        let point = input.mouse_position();
        if input.is_cursor_in_window() {
            if let Some(row) = context_menu::row_at(card, state.items.len(), point) {
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
            }
        }
    }

    fn run_selected_menu_command(&mut self) {
        let command = self
            .context_menu
            .as_ref()
            .and_then(|state| state.items.get(state.selected))
            .copied();
        self.close_context_menu();
        if let Some(command) = command {
            (command.run)(self);
        }
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
        self.slash_menu = None;
        self.refresh_slash_menu();
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
    fn refresh_dialog(&mut self) {
        let prompt = self.dialog.clone();
        self.regions[self.dialog_region].set_component(Box::new(Dialog::new(prompt)));
    }

    fn handle_dialog_input(&mut self, input: &Input, viewport: Rect) {
        let mut changed = false;
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
        let click = input
            .is_mouse_pressed(MouseButton::Left)
            .then(|| input.mouse_position());
        let confirmed = input.is_key_typed(KeyCode::Enter)
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

fn finder_input(state: &mut super::FileFinderState, input: &Input) -> bool {
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
    let visible = file_finder::filter(&state.files, &state.query).len();
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
    use crate::document::Document;
    use std::path::Path;

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
}
