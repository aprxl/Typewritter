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
    ContextMenu, Dialog, FileFinder, FileTree, MathMenu, Onboarding, Palette, SlashMenu,
    context_menu, editor, file_finder, file_tree, math_menu, onboarding, title_bar,
};
use crate::config::Config;
use crate::document::layout::{ContextHit, RangeKind};
use crate::document::math::{self, AccentKind, BigOp, MathNode, NodeAddress, Slot, SymbolRole};
use crate::document::{
    BadgeColor, FlatPos, FlatRange, Inline, Style, math_conversion, math_layout,
};
use crate::input::Input;
use crate::layout::Rect;
use crate::tabs::Tabs;
use crate::theme::{self, TextStyle};
use crate::vault::Vault;
use crate::vim::{
    Edit, ExtendedAction, Key, Mode, Motion, Operator, OperatorTarget, VimMode, VisualAction,
    VisualMode, motion,
};

use std::cell::RefCell;
use std::rc::Rc;

use super::{ContextMenuState, MathMenuState, PaletteState, Shell, SlashMenuState};

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

    fn handle_brush_input(&mut self, input: &Input) -> bool {
        if !(input.ctrl() && input.is_mouse_down(MouseButton::Left)) {
            if self.brush_point.take().is_some() || !self.brush_inside.is_empty() {
                self.brush_inside.clear();
                self.brush_revision = self.brush_revision.wrapping_add(1);
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

        toggle_brush_hits(
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
        let rect = self.layout.rect(self.text_column);
        let mouse = input.mouse_position();
        let over_editor = input.is_cursor_in_window() && rect.contains(mouse);
        let has_tab = self.docs.borrow().active().is_some();
        let in_math = self.docs.borrow().in_math();

        if has_tab && over_editor && input.scroll_delta().1 != 0.0 {
            let max = self.editor_max_scroll();
            let scroll = self.docs.borrow().editor_scroll;
            let next = (scroll - input.scroll_delta().1 * crate::document::layout::LINE_BODY)
                .clamp(0.0, max);
            self.docs.borrow_mut().set_editor_scroll(next);
        }

        if has_tab && !in_math && !self.vim.command_active() {
            self.arrow_keys(input);
        }

        match self.vim.current_mode() {
            VimMode::Insert => self.edit_frame_insert(input),
            _ => self.edit_frame_normal(input),
        }

        if has_tab && input.is_mouse_pressed(MouseButton::Left) && over_editor {
            if self.vim.current_mode() == VimMode::Normal {
                if let Some(target) = self.context_at(rect, mouse) {
                    let ids = self.context_ids(&target);
                    self.open_context_target(&ids, mouse, Some(target));
                }
            } else if let Some((block, inline, cursor)) = self.math_at(rect, mouse) {
                self.goal_x = None;
                self.docs.borrow_mut().enter_math_at(block, inline, cursor);
                self.apply(ExtendedAction::Enter(Mode::Insert));
            } else if let Some(caret) = self.caret_at(rect, mouse) {
                self.goal_x = None;
                self.docs
                    .borrow_mut()
                    .move_caret_to(caret.block, caret.inline, caret.offset);
            }
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

    fn edit_frame_math(&mut self, input: &Input) {
        for c in input.text().chars() {
            match c {
                c if math::PAIRS.iter().any(|&(open, _)| open == c) => {
                    self.docs.borrow_mut().math_open_group(c);
                }
                c if math::PAIRS.iter().any(|&(_, close)| close == c) => {
                    let closed = self.docs.borrow_mut().math_close_group(c);
                    if !closed {
                        self.docs.borrow_mut().math_type(c);
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
                let mut docs = self.docs.borrow_mut();
                if !move_inside_math(&mut docs, m, count) {
                    docs.touch(|doc| motion::apply(doc, m, count));
                }
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
        let commands = commands::palette_commands();
        let visible = palette::filter(&entries, &state.query);
        let command = visible.get(state.selected).and_then(|&i| commands.get(i));
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

    fn context_ids(&self, target: &ContextHit) -> Vec<&'static str> {
        match target {
            ContextHit::Range { kind, .. } => match kind {
                RangeKind::Word => commands::WORD_MENU.to_vec(),
                RangeKind::Badge => commands::BADGE_MENU.to_vec(),
                RangeKind::InlineCode => commands::INLINE_CODE_MENU.to_vec(),
                RangeKind::CodeBlock => commands::CODE_BLOCK_MENU.to_vec(),
            },
            ContextHit::Math {
                block,
                inline,
                node: Some(address),
            } => {
                let docs = self.docs.borrow();
                let node = docs
                    .active()
                    .and_then(|tab| tab.document.blocks.get(*block))
                    .and_then(|block| block.inlines().get(*inline))
                    .and_then(|run| match run {
                        Inline::Math(list) => math::node_at(list, address),
                        Inline::Text(_) => None,
                    });
                match node {
                    Some(MathNode::Sym(ch)) if ch.is_alphabetic() => {
                        symbol_context_ids(*ch, "plain")
                    }
                    Some(MathNode::Resolved { variant, body, .. }) => {
                        symbol_base_glyph(body, variant)
                            .map(|glyph| symbol_context_ids(glyph, variant))
                            .unwrap_or_else(|| commands::SYMBOL_ROLE_MENU.to_vec())
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

    fn context_range(&self) -> Option<FlatRange> {
        match self.context_menu.as_ref()?.target.as_ref()? {
            ContextHit::Range { range, .. } => Some(*range),
            ContextHit::Math { .. } => None,
        }
    }

    fn context_math_target(&self) -> Option<(usize, usize, NodeAddress)> {
        match self.context_menu.as_ref()?.target.as_ref()? {
            ContextHit::Math {
                block,
                inline,
                node: Some(address),
            } => Some((*block, *inline, address.clone())),
            _ => None,
        }
    }

    pub(super) fn context_toggle_bold(&mut self) {
        if let Some(range) = self.context_range() {
            self.docs.borrow_mut().toggle_style_range(
                range,
                Style {
                    bold: true,
                    ..Style::PLAIN
                },
            );
        }
    }

    pub(super) fn context_toggle_italic(&mut self) {
        if let Some(range) = self.context_range() {
            self.docs.borrow_mut().toggle_style_range(
                range,
                Style {
                    italic: true,
                    ..Style::PLAIN
                },
            );
        }
    }

    pub(super) fn context_toggle_highlight(&mut self) {
        if let Some(range) = self.context_range() {
            self.docs.borrow_mut().toggle_style_range(
                range,
                Style {
                    highlight: true,
                    ..Style::PLAIN
                },
            );
        }
    }

    pub(super) fn context_toggle_inline_code(&mut self) {
        if let Some(range) = self.context_range() {
            self.docs.borrow_mut().toggle_style_range(
                range,
                Style {
                    code: true,
                    ..Style::PLAIN
                },
            );
        }
    }

    pub(super) fn context_toggle_badge(&mut self) {
        if let Some(range) = self.context_range() {
            self.docs.borrow_mut().toggle_style_range(
                range,
                Style {
                    badge: true,
                    ..Style::PLAIN
                },
            );
        }
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
        if let Some((block, inline, address)) = self.context_math_target() {
            self.docs
                .borrow_mut()
                .set_math_node_role_at(block, inline, &address, role);
        }
    }

    pub(super) fn context_set_math_variant(&mut self, variant: &str) {
        if let Some((block, inline, address)) = self.context_math_target() {
            self.docs
                .borrow_mut()
                .set_math_node_variant_at(block, inline, &address, variant);
        }
    }

    pub(super) fn context_set_math_delimiter(&mut self, open: char) {
        if let Some((block, inline, address)) = self.context_math_target() {
            self.docs
                .borrow_mut()
                .set_math_group_delimiter_at(block, inline, &address, open);
        }
    }

    pub(super) fn context_set_math_accent(&mut self, kind: AccentKind) {
        if let Some((block, inline, address)) = self.context_math_target() {
            self.docs
                .borrow_mut()
                .set_math_accent_kind_at(block, inline, &address, kind);
        }
    }

    pub(super) fn context_set_math_big_op(&mut self, kind: BigOp) {
        if let Some((block, inline, address)) = self.context_math_target() {
            self.docs
                .borrow_mut()
                .set_math_big_op_kind_at(block, inline, &address, kind);
        }
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
        self.refresh_context_menu();
        self.rebuild_views();
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
        self.rebuild_views();
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

        let Some(state) = &self.context_menu else {
            return true;
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
            if let Some(index) = math_menu::item_at(card, offers.len(), variant_start, point) {
                let changed = self
                    .math_menu
                    .as_ref()
                    .is_some_and(|state| state.selected != index);
                if changed {
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
        let count = math_conversion::offers(&state.query).offers.len();
        if count == 0 {
            return;
        }
        let next = moved_math_menu_selection(state.selected, count, delta);
        if next != state.selected {
            state.selected = next;
            self.refresh_math_menu();
        }
    }

    /// Recomputed every frame, the card's whole life: show it when the word
    /// under the math cursor has offers it was not dismissed for, hide it
    /// otherwise. Movement and edits replace the precise tree query.
    pub(super) fn sync_math_menu(&mut self) {
        let query = self.docs.borrow().math_conversion_query();
        let Some(query) = query else {
            if self.math_menu.take().is_some() {
                self.refresh_math_menu();
            }
            return;
        };
        if self.math_dismissed.as_ref() == Some(&query)
            || math_conversion::offers(&query).offers.is_empty()
        {
            if self.math_menu.take().is_some() {
                self.refresh_math_menu();
            }
            return;
        }
        let anchor = self.compute_math_menu_anchor();
        let changed = match &self.math_menu {
            Some(state) => state.query != query,
            None => true,
        };
        let state = self.math_menu.get_or_insert_with(|| MathMenuState {
            query: query.clone(),
            selected: 0,
            anchor,
        });
        let moved = state.anchor != anchor;
        state.anchor = anchor;
        // A new query is a new list: reset the selection and rebuild rows.
        if changed {
            state.query = query;
            state.selected = 0;
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
            let (ox, oy, _) = math_layout::cursor_pos(list, cursor, 0, &measure);
            (cx, cy, docs.editor_scroll, ox, oy)
        };
        let screen_x = rect.x + crate::components::editor::INSET + base_x + offset_x;
        let screen_y = rect.y + crate::components::editor::TOP + base_y - offset_y - scroll;
        (screen_x, screen_y)
    }

    fn refresh_math_menu(&mut self) {
        let menu = match &self.math_menu {
            Some(state) => {
                let offers = math_conversion::offers(&state.query).offers;
                MathMenu::new(
                    math_menu_rows(&offers),
                    math_menu_variant_start(&offers),
                    state.selected,
                    state.anchor,
                )
            }
            None => MathMenu::closed(),
        };
        self.regions[self.math_menu_region].set_component(Box::new(menu));
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

fn symbol_context_ids(glyph: char, current_variant: &str) -> Vec<&'static str> {
    let mut ids = commands::SYMBOL_ROLE_MENU.to_vec();
    if current_variant != "plain" {
        ids.push("context.variant.plain");
    }
    ids.extend(
        crate::document::math_symbols::variants(glyph)
            .into_iter()
            .filter(|variant| variant.key != current_variant)
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

fn toggle_brush_hits(
    selected: &mut Vec<ContextHit>,
    inside: &mut Vec<ContextHit>,
    swept_hits: Vec<ContextHit>,
    current_hits: Vec<ContextHit>,
) {
    for hit in swept_hits.iter().filter(|hit| !inside.contains(hit)) {
        if let Some(index) = selected.iter().position(|selected| selected == hit) {
            selected.remove(index);
        } else {
            selected.push(hit.clone());
        }
    }
    *inside = current_hits;
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
    use super::{
        brush_sweep_samples, delete_chars, delete_inside_math, math_menu_rows,
        math_menu_variant_start, move_inside_math, moved_math_menu_selection,
        reset_brush_selection, symbol_base_glyph, symbol_context_ids, toggle_brush_hits,
    };
    use crate::document::layout::{ContextHit, RangeKind};
    use crate::document::math::MathNode;
    use crate::document::{Document, FlatPos, FlatRange, Inline, math_conversion, math_symbols};
    use crate::tabs::Tabs;

    #[test]
    fn symbol_context_keeps_roles_first_and_offers_only_valid_variants() {
        let latin = symbol_context_ids('x', "plain");
        assert_eq!(&latin[..3], super::commands::SYMBOL_ROLE_MENU);
        assert!(latin.contains(&"context.variant.bold"));
        assert!(!latin.contains(&"context.variant.plain"));

        let italic_alpha = math_symbols::variants('α')
            .into_iter()
            .find(|variant| variant.key == "italic")
            .unwrap()
            .glyph;
        assert_eq!(
            symbol_base_glyph(&[MathNode::Sym(italic_alpha)], "italic"),
            Some('α')
        );
        let greek = symbol_context_ids('α', "italic");
        assert!(greek.contains(&"context.variant.plain"));
        assert!(!greek.contains(&"context.variant.sans"));
    }

    #[test]
    fn brush_toggles_only_when_a_target_is_entered() {
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

        toggle_brush_hits(
            &mut selected,
            &mut inside,
            vec![target.clone()],
            vec![target.clone()],
        );
        assert_eq!(selected, vec![target.clone()]);
        toggle_brush_hits(
            &mut selected,
            &mut inside,
            vec![target.clone()],
            vec![target.clone()],
        );
        assert_eq!(selected, vec![target.clone()]);

        toggle_brush_hits(&mut selected, &mut inside, Vec::new(), Vec::new());
        toggle_brush_hits(
            &mut selected,
            &mut inside,
            vec![target.clone()],
            vec![target],
        );
        assert!(selected.is_empty());
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

        toggle_brush_hits(
            &mut selected,
            &mut inside,
            vec![third.clone()],
            vec![third.clone()],
        );
        assert_eq!(selected, vec![first.clone(), second.clone(), third]);
        toggle_brush_hits(&mut selected, &mut inside, Vec::new(), Vec::new());
        toggle_brush_hits(&mut selected, &mut inside, vec![first.clone()], vec![first]);
        assert_eq!(selected, vec![second, word(4)]);
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
    fn swept_hits_toggle_once_but_inside_tracks_only_the_current_circle() {
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

        toggle_brush_hits(
            &mut selected,
            &mut inside,
            vec![crossed.clone(), current.clone()],
            vec![current.clone()],
        );

        assert_eq!(selected, vec![crossed.clone(), current.clone()]);
        assert_eq!(inside, vec![current]);

        toggle_brush_hits(
            &mut selected,
            &mut inside,
            vec![crossed.clone()],
            vec![crossed.clone()],
        );
        assert_eq!(selected, vec![word(2)]);
        assert_eq!(inside, vec![crossed]);
    }

    #[test]
    fn escape_reset_clears_the_entire_brush_selection() {
        let target = ContextHit::Math {
            block: 2,
            inline: 0,
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
    use crate::vim::Motion;
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
            tabs.active().unwrap().document.blocks[0].inlines()[0],
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
        assert!(matches!(doc.blocks[0].inlines()[0], Inline::Math(ref list) if list.is_empty()));
        assert!(matches!(
            doc.blocks[0].inlines()[1],
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
}
