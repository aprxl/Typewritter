//! Open notes: the tab strip's model.
//!
//! A preview (or peek) tab is transient — the next click on a different file
//! replaces it — while a full tab stays until closed. A file is never open
//! twice, and the active tab is where editing lands.

use std::path::{Path, PathBuf};

use crate::document::{Document, FlatRange, Style};

pub struct Tab {
    pub document: Document,
    /// Transient preview tabs are replaced by the next open.
    pub preview: bool,
    undo: Vec<Document>,
    redo: Vec<Document>,
    yank: String,
}

impl Tab {
    /// Loads a file into a tab, or `None` when it cannot be read.
    fn load(path: &Path, preview: bool) -> Option<Tab> {
        Document::load(path).map(|document| Tab {
            document,
            preview,
            undo: Vec::new(),
            redo: Vec::new(),
            yank: String::new(),
        })
    }
}

pub struct Tabs {
    pub tabs: Vec<Tab>,
    pub active: Option<usize>,
    /// The file most recently clicked in the tree (drives Ctrl+D).
    pub tree_selected: Option<PathBuf>,
    /// The editor pane's scroll for the active tab, in logical pixels.
    pub editor_scroll: f32,
    /// Bumped on every structural or content change; the shell rebuilds its
    /// views when it moves.
    revision: u64,
    transaction: Option<Document>,
}

impl Default for Tabs {
    fn default() -> Self {
        Self::new()
    }
}

impl Tabs {
    pub fn new() -> Self {
        Self {
            tabs: Vec::new(),
            active: None,
            tree_selected: None,
            editor_scroll: 0.0,
            revision: 0,
            transaction: None,
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    fn bump(&mut self) {
        self.revision += 1;
    }

    pub fn active(&self) -> Option<&Tab> {
        self.active.and_then(|i| self.tabs.get(i))
    }

    pub fn active_mut(&mut self) -> Option<&mut Tab> {
        let i = self.active?;
        self.tabs.get_mut(i)
    }

    pub fn active_index(&self) -> Option<usize> {
        self.active
    }

    pub fn activate(&mut self, index: usize) {
        if index < self.tabs.len() && self.active != Some(index) {
            self.active = Some(index);
            self.bump();
        }
    }

    /// Scroll changes are view-only, but they must still rebuild the editor
    /// pane, so a real change bumps like any other.
    pub fn set_editor_scroll(&mut self, scroll: f32) {
        if (self.editor_scroll - scroll).abs() > 0.5 {
            self.editor_scroll = scroll;
            self.bump();
        }
    }

    /// Ctrl+W / the tab's close button.
    pub fn close_active(&mut self) {
        if let Some(i) = self.active {
            self.close(i);
        }
    }

    /// Saves the active tab, promoting the preview it is. `Ok` even when
    /// there is nothing to save.
    pub fn save_active(&mut self) -> std::io::Result<()> {
        let Some(tab) = self.active.and_then(|i| self.tabs.get_mut(i)) else {
            return Ok(());
        };
        if tab.preview {
            tab.preview = false;
        }
        tab.document.save()?;
        self.bump();
        Ok(())
    }

    /// Single click: show `path` in a preview tab, replaced by the next
    /// preview open. A file already in a full tab just gets activated.
    /// Exactly one preview tab exists at any time.
    pub fn open_preview(&mut self, path: &Path) {
        if let Some(i) = self
            .tabs
            .iter()
            .position(|t| !t.preview && t.path() == path)
        {
            self.activate(i);
            return;
        }
        let Some(tab) = Tab::load(path, true) else {
            return;
        };
        // Whatever preview is already open becomes the new one — even one
        // that is not the active tab (a full tab can sit between two
        // single clicks).
        if let Some(i) = self.tabs.iter().position(|t| t.preview) {
            self.tabs[i] = tab;
            self.active = Some(i);
            self.bump();
            return;
        }
        match self.active {
            // A preview sits right after the active tab.
            Some(i) => {
                self.tabs.insert(i + 1, tab);
                self.active = Some(i + 1);
            }
            None => {
                self.tabs.push(tab);
                self.active = Some(0);
            }
        }
        self.bump();
    }

    /// Double click: `path` becomes a full tab. A preview of it is promoted.
    pub fn open_full(&mut self, path: &Path) {
        if let Some(i) = self.tabs.iter().position(|t| t.path() == path) {
            if self.tabs[i].preview {
                self.tabs[i].preview = false;
                self.bump();
            }
            self.activate(i);
            return;
        }
        let Some(tab) = Tab::load(path, false) else {
            return;
        };
        let index = self.tabs.len();
        self.tabs.push(tab);
        self.active = Some(index);
        self.bump();
    }

    /// Editing a preview makes it permanent (spec §7.1).
    fn promote_active(&mut self) {
        if let Some(tab) = self.active_mut()
            && tab.preview
        {
            tab.preview = false;
            self.bump();
        }
    }

    pub fn close(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        self.active = match self.active {
            Some(a) if a == index => {
                if self.tabs.is_empty() {
                    None
                } else {
                    Some(index.min(self.tabs.len() - 1))
                }
            }
            Some(a) if a > index => Some(a - 1),
            other => other,
        };
        self.bump();
    }

    /// Removes every tab pointing at `path` (used when a file is deleted).
    pub fn close_path(&mut self, path: &Path) {
        let mut i = 0;
        while i < self.tabs.len() {
            if self.tabs[i].path() == path {
                self.close(i);
            } else {
                i += 1;
            }
        }
        if self.tree_selected.as_deref() == Some(path) {
            self.tree_selected = None;
            self.bump();
        }
    }

    // ---- editing the active tab -------------------------------------------

    fn edit(&mut self, f: impl FnOnce(&mut Document)) {
        if self.active.is_none() {
            return;
        }
        let index = self.active.unwrap();
        let before = self.tabs[index].document.clone();
        let was_preview = self.tabs[index].preview;
        if let Some(tab) = self.active_mut() {
            f(&mut tab.document);
        }
        let changed = self.tabs[index].document.blocks != before.blocks;
        if changed && self.transaction.is_none() {
            self.tabs[index].undo.push(before);
            self.tabs[index].redo.clear();
        }
        if changed && was_preview {
            self.promote_active();
        }
        self.bump();
    }

    /// Group edits spanning multiple input frames into one snapshot.
    pub fn begin_transaction(&mut self) {
        if self.transaction.is_none() {
            self.transaction = self.active().map(|tab| tab.document.clone());
        }
    }

    pub fn end_transaction(&mut self) {
        let Some(before) = self.transaction.take() else {
            return;
        };
        let Some(index) = self.active else {
            return;
        };
        if self.tabs[index].document.blocks != before.blocks {
            self.tabs[index].undo.push(before);
            self.tabs[index].redo.clear();
        }
    }

    pub fn transaction(&mut self, f: impl FnOnce(&mut Self)) {
        let nested = self.transaction.is_some();
        if !nested {
            self.begin_transaction();
        }
        f(self);
        if !nested {
            self.end_transaction();
        }
    }

    /// Caret-only movement: bumps the revision (the views still need to
    /// redraw) but never promotes a preview tab — only an actual content
    /// change does that (spec §7.1). Arrow keys and click-to-place go
    /// through this, and so do vim motions applied from the shell.
    pub fn touch(&mut self, f: impl FnOnce(&mut Document)) {
        if let Some(tab) = self.active_mut() {
            f(&mut tab.document);
            self.bump();
        }
    }

    pub fn type_text(&mut self, text: &str) {
        self.edit(|doc| doc.insert_text(text));
    }

    pub fn backspace(&mut self) {
        self.edit(Document::backspace);
    }

    pub fn delete_forward(&mut self) {
        self.edit(Document::delete_forward);
    }

    pub fn newline(&mut self) {
        self.edit(Document::newline);
    }

    pub fn delete_char(&mut self) {
        self.edit(Document::delete_char);
    }

    pub fn delete_line(&mut self) {
        self.edit(Document::delete_line);
    }

    pub fn delete_range(&mut self, range: FlatRange) {
        let mut yank = None;
        self.edit(|doc| yank = Some(doc.delete_range(range)));
        if let Some(text) = yank
            && let Some(tab) = self.active_mut()
        {
            tab.yank = text;
        }
    }

    pub fn delete_lines(&mut self, first: usize, last: usize) {
        let mut yank = None;
        self.edit(|doc| yank = Some(doc.delete_lines(first, last)));
        if let Some(text) = yank
            && let Some(tab) = self.active_mut()
        {
            tab.yank = text;
        }
    }

    pub fn replace_range(&mut self, range: FlatRange, text: &str) {
        self.edit(|doc| {
            doc.replace_range(range, text);
        });
    }

    pub fn toggle_style_range(&mut self, range: FlatRange, mask: Style) {
        self.edit(|doc| doc.toggle_style_range(range, mask));
    }

    pub fn open_change(&mut self, range: FlatRange) {
        self.delete_range(range);
    }

    pub fn yank_range(&mut self, range: FlatRange) {
        if let Some(tab) = self.active_mut() {
            tab.yank = tab.document.yank_range(range);
        }
    }

    pub fn yank(&self) -> Option<&str> {
        self.active().map(|tab| tab.yank.as_str())
    }

    /// Replaces the yank register. The shell uses this to feed in text that
    /// was copied outside the app; `Tabs` itself never looks at the OS.
    pub fn set_yank(&mut self, text: String) {
        if let Some(tab) = self.active_mut() {
            tab.yank = text;
        }
    }

    pub fn paste(&mut self, before: bool, count: usize) {
        let text = self
            .active()
            .map(|tab| tab.yank.clone())
            .unwrap_or_default();
        if text.is_empty() {
            return;
        }
        self.edit(|doc| {
            if !before {
                let position = doc.caret_position();
                let offset = (position.offset + 1).min(doc.block_len(position.block));
                doc.set_flat_position(doc.position(position.block, offset));
            }
            for _ in 0..count.max(1) {
                doc.insert_text(&text);
            }
        });
    }

    pub fn undo(&mut self) {
        let Some(index) = self.active else { return };
        let Some(previous) = self.tabs[index].undo.pop() else {
            return;
        };
        let current = std::mem::replace(&mut self.tabs[index].document, previous);
        self.tabs[index].redo.push(current);
        self.bump();
    }

    pub fn redo(&mut self) {
        let Some(index) = self.active else { return };
        let Some(next) = self.tabs[index].redo.pop() else {
            return;
        };
        let current = std::mem::replace(&mut self.tabs[index].document, next);
        self.tabs[index].undo.push(current);
        self.bump();
    }

    pub fn open_below(&mut self) {
        self.edit(Document::open_below);
    }

    pub fn open_above(&mut self) {
        self.edit(Document::open_above);
    }

    /// Convert the caret's block: `None` → paragraph, `Some(1..=3)` →
    /// heading. A content change — promotes a preview tab.
    pub fn set_heading(&mut self, level: Option<u8>) {
        self.edit(|doc| doc.set_heading(level));
    }

    pub fn insert_divider(&mut self) {
        self.edit(|doc| doc.insert_divider());
    }

    pub fn insert_inline_math(&mut self) {
        self.edit(Document::insert_inline_math);
    }

    pub fn insert_math_block(&mut self) {
        self.edit(Document::insert_math_block);
    }

    pub fn math_type(&mut self, c: char) {
        self.edit(|doc| doc.math_insert_char(c));
    }

    pub fn math_fraction(&mut self) {
        self.edit(Document::math_insert_fraction);
    }

    pub fn math_backspace(&mut self) {
        self.edit(|doc| {
            doc.math_backspace();
        });
    }

    // Math movement changes caret state, not content, so it touches rather than edits.
    pub fn math_left(&mut self) -> bool {
        let mut moved = false;
        self.touch(|doc| moved = doc.math_move_left());
        moved
    }

    pub fn math_right(&mut self) -> bool {
        let mut moved = false;
        self.touch(|doc| moved = doc.math_move_right());
        moved
    }

    pub fn math_slot_next(&mut self) -> bool {
        let mut moved = false;
        self.touch(|doc| moved = doc.math_slot_next());
        moved
    }

    pub fn math_slot_prev(&mut self) -> bool {
        let mut moved = false;
        self.touch(|doc| moved = doc.math_slot_prev());
        moved
    }

    pub fn math_pop(&mut self) -> bool {
        let mut popped = false;
        self.touch(|doc| popped = doc.math_pop());
        popped
    }

    pub fn math_exit(&mut self) {
        self.touch(Document::math_exit);
    }

    /// Whether the caret is inside a math expression — the shell routes
    /// keys on this, so it must come from the document rather than from
    /// any state the shell keeps of its own.
    pub fn in_math(&self) -> bool {
        self.active().is_some_and(|tab| tab.document.math.is_some())
    }

    /// Flip bold/italic on the pending context. Caret-only — never promotes
    /// a preview tab (a bold/italic toggle doesn't touch content).
    pub fn toggle_bold(&mut self) {
        self.touch(Document::toggle_bold);
    }

    pub fn toggle_italic(&mut self) {
        self.touch(Document::toggle_italic);
    }

    /// Flip the code flag on the pending style. Caret-only — never promotes
    /// a preview tab.
    pub fn toggle_code(&mut self) {
        self.touch(Document::toggle_code);
    }

    /// Flip the badge flag on the pending style: what is typed next becomes
    /// the chip's label. Caret-only, like the other style toggles.
    pub fn toggle_badge(&mut self) {
        self.touch(Document::toggle_badge);
    }

    pub fn toggle_highlight(&mut self) {
        self.touch(Document::toggle_highlight);
    }

    /// Convert the caret's block to/from a fenced code line. A content
    /// change — promotes a preview tab, same as `set_heading`.
    pub fn set_code(&mut self, on: bool) {
        self.edit(|doc| doc.set_code(on));
    }

    pub fn move_left(&mut self) {
        self.touch(Document::move_left);
    }

    pub fn move_right(&mut self) {
        self.touch(Document::move_right);
    }

    pub fn move_home(&mut self) {
        self.touch(Document::move_home);
    }

    pub fn move_end(&mut self) {
        self.touch(Document::move_end);
    }

    /// Jump the caret to a model position (click-to-place / vim jumps).
    pub fn move_caret_to(&mut self, block: usize, inline: usize, offset: usize) {
        self.touch(|doc| doc.set_caret(block, inline, offset));
    }
}

impl Tab {
    pub fn path(&self) -> &Path {
        &self.document.path
    }

    pub fn name(&self) -> &str {
        &self.document.name
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_file(tag: &str, text: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("tw-tabs-{tag}-{}.md", std::process::id()));
        fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn a_single_click_replaces_the_current_preview() {
        let a = temp_file("ppa", "a");
        let b = temp_file("ppb", "b");
        let mut tabs = Tabs::new();
        tabs.open_preview(&a);
        assert_eq!(tabs.tabs.len(), 1);
        assert!(tabs.active().unwrap().preview);

        tabs.open_preview(&b);
        assert_eq!(
            tabs.tabs.len(),
            1,
            "the old preview is replaced, not stacked"
        );
        assert_eq!(tabs.active().unwrap().path(), &b);
    }

    #[test]
    fn a_full_tab_is_never_duplicated() {
        let a = temp_file("ndup", "a");
        let mut tabs = Tabs::new();
        tabs.open_full(&a);
        tabs.open_full(&a);
        tabs.open_preview(&a);
        assert_eq!(tabs.tabs.len(), 1);
        assert!(
            !tabs.active().unwrap().preview,
            "preview click found the full tab"
        );
    }

    #[test]
    fn opening_an_existing_file_activates_instead_of_stacking() {
        let a = temp_file("act", "a");
        let b = temp_file("actb", "b");
        let mut tabs = Tabs::new();
        tabs.open_full(&a);
        tabs.open_full(&b);
        assert_eq!(tabs.active_index(), Some(1));

        tabs.open_preview(&a);
        assert_eq!(tabs.tabs.len(), 2);
        assert_eq!(
            tabs.active_index(),
            Some(0),
            "preview click on a full tab activates it"
        );
    }

    #[test]
    fn editing_a_preview_promotes_it() {
        let a = temp_file("promote", "a");
        let mut tabs = Tabs::new();
        tabs.open_preview(&a);
        assert!(tabs.active().unwrap().preview);

        tabs.type_text("x");
        assert!(
            !tabs.active().unwrap().preview,
            "first edit makes it permanent"
        );
    }

    #[test]
    fn moving_the_caret_in_a_preview_tab_does_not_promote_it_but_typing_does() {
        let a = temp_file("caretpreview", "ab cd");
        let mut tabs = Tabs::new();
        tabs.open_preview(&a);
        assert!(tabs.active().unwrap().preview);

        tabs.move_right();
        tabs.move_caret_to(0, 0, 2);
        assert!(
            tabs.active().unwrap().preview,
            "caret-only movement must not pin a preview tab"
        );

        tabs.type_text("x");
        assert!(
            !tabs.active().unwrap().preview,
            "an actual edit still promotes it"
        );
    }

    #[test]
    fn math_edits_promote_a_preview_tab_but_moves_do_not() {
        let edit_path = temp_file("math-edit-preview", "a");
        let mut tabs = Tabs::new();
        tabs.open_preview(&edit_path);
        tabs.insert_inline_math();
        tabs.math_type('x');
        assert!(!tabs.active().unwrap().preview);

        let move_path = temp_file("math-move-preview", "b");
        tabs.open_preview(&move_path);
        tabs.active_mut().unwrap().document.insert_inline_math();
        tabs.math_left();
        assert!(tabs.active().unwrap().preview);
    }

    #[test]
    fn closing_the_active_tab_activates_its_left_neighbour() {
        let a = temp_file("cla", "a");
        let b = temp_file("clb", "b");
        let c = temp_file("clc", "c");
        let mut tabs = Tabs::new();
        tabs.open_full(&a);
        tabs.open_full(&b);
        tabs.open_full(&c);
        assert_eq!(tabs.active_index(), Some(2));

        tabs.close(1);
        assert_eq!(tabs.tabs.len(), 2);
        tabs.close(1);
        assert_eq!(tabs.tabs.len(), 1);
        assert_eq!(
            tabs.active_index(),
            Some(0),
            "closing the last tab lands on the survivor"
        );

        tabs.close(0);
        assert_eq!(tabs.active, None);
    }

    #[test]
    fn closing_a_preview_preserves_more_tabs() {
        let a = temp_file("precl", "a");
        let b = temp_file("preclb", "b");
        let mut tabs = Tabs::new();
        tabs.open_full(&a);
        tabs.open_preview(&b);
        assert_eq!(tabs.tabs.len(), 2);

        // Clicking another file closes the preview without touching A.
        let c = temp_file("preclc", "c");
        tabs.open_preview(&c);
        assert_eq!(tabs.tabs.len(), 2);
        assert_eq!(tabs.tabs[0].path(), &a);
        assert_eq!(tabs.active().unwrap().path(), &c);
    }

    #[test]
    fn at_most_one_preview_ever_exists() {
        let a = temp_file("onea", "a");
        let b = temp_file("oneb", "b");
        let c = temp_file("onec", "c");
        let d = temp_file("oned", "d");
        let mut tabs = Tabs::new();
        tabs.open_full(&a);
        tabs.open_preview(&b);
        tabs.open_full(&c);
        assert_eq!(tabs.tabs.len(), 3);

        // A full tab is active; a second single click still replaces the
        // leftover preview instead of adding a third.
        tabs.open_preview(&d);
        assert_eq!(
            tabs.tabs.iter().filter(|t| t.preview).count(),
            1,
            "the preview invariant holds across a full-tab open"
        );
        assert_eq!(tabs.active().unwrap().path(), &d);
    }

    #[test]
    fn deletion_closes_every_tab_for_that_file() {
        let a = temp_file("del", "a");
        let mut tabs = Tabs::new();
        tabs.open_full(&a);
        tabs.tree_selected = Some(a.clone());
        tabs.close_path(&a);
        assert!(tabs.tabs.is_empty());
        assert_eq!(tabs.tree_selected, None);
    }

    #[test]
    fn save_writes_canonical_markdown_and_clears_dirty() {
        let a = temp_file("save", "**bold** and *italic*");
        let mut tabs = Tabs::new();
        tabs.open_full(&a);
        tabs.type_text(" hi");
        assert!(tabs.active().unwrap().document.is_dirty());
        tabs.save_active().unwrap();
        assert!(!tabs.active().unwrap().document.is_dirty());
        let on_disk = fs::read_to_string(&a).unwrap();
        let reopened = Document::load(&a).unwrap();
        assert_eq!(
            reopened.word_count(),
            tabs.active().unwrap().document.word_count()
        );
        assert!(on_disk.contains("**bold**"));
    }

    #[test]
    fn active_tab_undo_redo_restores_document_snapshots() {
        let path = temp_file("history", "abc");
        let mut tabs = Tabs::new();
        tabs.open_full(&path);
        tabs.move_caret_to(0, 0, 3);
        tabs.type_text("d");
        assert_eq!(tabs.active().unwrap().document.word_count(), 1);
        assert_eq!(tabs.active().unwrap().document.block_text(0), "abcd");
        tabs.undo();
        assert_eq!(tabs.active().unwrap().document.block_text(0), "abc");
        tabs.redo();
        assert_eq!(tabs.active().unwrap().document.block_text(0), "abcd");
    }

    #[test]
    fn transaction_groups_insert_session_into_one_undo_step() {
        let path = temp_file("grouped", "abc");
        let mut tabs = Tabs::new();
        tabs.open_full(&path);
        tabs.move_caret_to(0, 0, 3);
        tabs.begin_transaction();
        tabs.type_text("d");
        tabs.type_text("e");
        tabs.end_transaction();
        assert_eq!(tabs.active().unwrap().document.block_text(0), "abcde");
        tabs.undo();
        assert_eq!(tabs.active().unwrap().document.block_text(0), "abc");
    }

    #[test]
    fn counted_deletes_inside_transaction_undo_together() {
        let path = temp_file("grouped-delete", "abcd");
        let mut tabs = Tabs::new();
        tabs.open_full(&path);
        tabs.begin_transaction();
        tabs.delete_char();
        tabs.delete_char();
        tabs.delete_char();
        tabs.end_transaction();
        assert_eq!(tabs.active().unwrap().document.block_text(0), "d");
        tabs.undo();
        assert_eq!(tabs.active().unwrap().document.block_text(0), "abcd");
    }

    #[test]
    fn paste_before_at_block_start_does_not_cross_block() {
        let path = temp_file("paste-block", "a\n\nb");
        let mut tabs = Tabs::new();
        tabs.open_full(&path);
        tabs.yank_range(FlatRange::new(
            crate::document::FlatPos {
                block: 0,
                offset: 0,
            },
            crate::document::FlatPos {
                block: 0,
                offset: 1,
            },
        ));
        tabs.move_caret_to(1, 0, 0);
        tabs.paste(true, 1);
        assert_eq!(tabs.active().unwrap().document.block_text(0), "a");
        assert_eq!(tabs.active().unwrap().document.block_text(1), "ab");
    }

    #[test]
    fn paste_after_stays_on_current_block_at_its_end() {
        let path = temp_file("paste-after", "a\n\nb");
        let mut tabs = Tabs::new();
        tabs.open_full(&path);
        tabs.yank_range(FlatRange::new(
            crate::document::FlatPos {
                block: 1,
                offset: 0,
            },
            crate::document::FlatPos {
                block: 1,
                offset: 1,
            },
        ));
        tabs.move_caret_to(0, 0, 0);
        tabs.paste(false, 1);
        assert_eq!(tabs.active().unwrap().document.block_text(0), "ab");
        assert_eq!(tabs.active().unwrap().document.block_text(1), "b");
    }
}
