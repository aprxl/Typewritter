//! Structural table edits.
//!
//! A table is a run of `Block::TableRow` blocks; every edit here is a
//! document-level operation on that run and its shared settings. Cell-level
//! arithmetic (lines, flat offsets) lives on [`table::Cell`] itself.

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::document::layout;
    use crate::document::markdown::{parse, serialize};
    use crate::document::{Block, Document, Inline, cell_text};
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
}
