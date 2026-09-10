//! Structural table edits.
//!
//! A table is a run of `Block::TableRow` blocks; every edit here is a
//! document-level operation on that run and its shared settings. Cell-level
//! arithmetic (lines, flat offsets) lives on [`table::Cell`] itself.

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::document::markdown::{parse, serialize};
    use crate::document::{Block, Document, cell_text};

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
}
