//! Focus mode owns panel restoration and the writing camera's scroll range.

use super::Shell;
use crate::components::editor;
use crate::layout::Layout;

impl Shell {
    pub(super) fn focus_mode(&self) -> bool {
        self.focus_panels.is_some()
    }

    pub(super) fn toggle_focus(&mut self) {
        if self.focus_mode() {
            self.leave_focus();
        } else {
            // A hidden sidenote must never keep receiving keystrokes.
            let in_note =
                self.docs.borrow().active().is_some_and(|tab| {
                    matches!(tab.document.focus, crate::document::Focus::Note(_))
                });
            if in_note {
                self.exit_visual();
                self.goal_x = None;
                super::input::return_to_anchor(&mut self.docs.borrow_mut());
            }
            self.focus_panels = Some(self.panels().map(|panel| panel.open));
            for panel in self.panels_mut() {
                panel.set_open(false);
            }
        }
    }

    pub(super) fn leave_focus(&mut self) {
        if let Some(open) = self.focus_panels.take() {
            for (panel, open) in self.panels_mut().into_iter().zip(open) {
                panel.set_open(open);
            }
        }
    }

    /// Also covers switching back to a tab whose caret is in a sidenote.
    pub(super) fn reveal_focused_note(&mut self) {
        let in_note = self
            .docs
            .borrow()
            .active()
            .is_some_and(|tab| matches!(tab.document.focus, crate::document::Focus::Note(_)));
        if self.focus_mode() && in_note {
            self.leave_focus();
            self.sidenotes.set_open(true);
        }
    }

    pub(super) fn toggle_sidebar(&mut self) {
        if self.focus_mode() {
            self.leave_focus();
            self.tree.set_open(true);
        } else {
            self.tree.toggle();
        }
    }

    pub(super) fn editor_scroll_bounds(&self) -> (f32, f32) {
        let rect = self.layout.rect(self.text_column);
        let Some((_, _, _, _, _, layout)) = &self.doc_layout else {
            return (0.0, 0.0);
        };
        if self.focus_mode() {
            let origin = editor::focus_scroll((0.0, 0.0), rect, self.layout.rect(Layout::ROOT));
            (origin, origin + layout.height)
        } else {
            (
                0.0,
                editor::max_scroll(layout.height, rect.height - editor::TOP),
            )
        }
    }
}
