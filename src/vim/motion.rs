//! Where a motion sends the caret, and the character-class rules `w`/`b`/`e`
//! use to find word boundaries.
//!
//! Motions work on the document's blocks as flat text: a position is a
//! `(block, flat)` pair into that block's concatenated run text. A block
//! boundary is whitespace (words never straddle one), which is what lets `w`
//! and `b` cross blocks without special-casing the ends. Up/Down stay out of
//! here — moving between *visual* lines needs pixels, so the shell applies
//! them through the layout's `line_up`/`line_down`.

use crate::document::{Block, Document, FlatPos, Inline};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Here,
    Left,
    Right,
    /// `a`: insert *after* the character under the caret, but never crossing
    /// a block boundary — at the end of a line it appends on that same line.
    Append,
    Up,
    Down,
    WordForward,
    WordBack,
    WordEnd,
    LineStart,
    FirstNonBlank,
    LineEnd,
    FirstLine,
    LastLine,
    ParagraphBack,
    ParagraphForward,
    FindForward(char),
    TillForward(char),
    FindBack(char),
    TillBack(char),
}

/// vim's word classes: a run of alphanumerics/`_`, or a run of punctuation.
/// Whitespace (and the `None` this returns for it) separates runs and never
/// belongs to one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CharClass {
    Word,
    Punct,
}

fn class(c: char) -> Option<CharClass> {
    if c.is_whitespace() {
        None
    } else if c.is_alphanumeric() || c == '_' {
        Some(CharClass::Word)
    } else {
        Some(CharClass::Punct)
    }
}

fn inline_text(run: &Inline) -> String {
    match run {
        Inline::Text(text) => text.text.clone(),
        Inline::Math(_) | Inline::Note(_) | Inline::EqRef(_) => "\u{FFFC}".into(),
    }
}

/// The block's flat text — one char per flat position, a table row's cells
/// included. A cell's flat space is its runs plus one position per line
/// break, and `cell_text` joins those lines with exactly that one `\n`, so
/// concatenating the cells gives a row the same flat text prose has. One
/// char-per-position is what lets `0`/`a`/`f`/`t` be a single code path for
/// prose and for a row.
fn block_text(block: &Block) -> String {
    match block {
        Block::TableRow { cells, .. } => cells.iter().map(crate::document::cell_text).collect(),
        block => block.inlines().iter().map(inline_text).collect(),
    }
}

/// Flat length of a block, its cells included — the model's one flat-length
/// rule (`Block::flat_len`), so this reader can never drift from the caret
/// arithmetic that addresses the same positions.
fn block_len(block: &Block) -> usize {
    block.flat_len()
}

fn table_cell_text(doc: &Document) -> String {
    // A cell's flat text. Word motion is cell-local (a cell is a scope it
    // must not leave), so this is the one reader that reaches into a cell
    // directly; every positional read goes through `block_text` instead.
    crate::document::cell_text(&doc.scope()[doc.caret.block].cells()[doc.caret.inline])
}

fn table_word_forward(doc: &mut Document) {
    let text: Vec<char> = table_cell_text(doc).chars().collect();
    let mut offset = doc.caret.offset.min(text.len());
    let start_class = text.get(offset).copied().and_then(class);
    if start_class.is_some() {
        while offset < text.len() && text.get(offset).copied().and_then(class) == start_class {
            offset += 1;
        }
    }
    while offset < text.len() && text[offset].is_whitespace() {
        offset += 1;
    }
    doc.set_caret(doc.caret.block, doc.caret.inline, offset);
}

fn table_word_back(doc: &mut Document) {
    let text: Vec<char> = table_cell_text(doc).chars().collect();
    let mut offset = doc.caret.offset.min(text.len()).saturating_sub(1);
    while offset > 0 && text.get(offset).is_some_and(|c| c.is_whitespace()) {
        offset -= 1;
    }
    let target_class = text.get(offset).copied().and_then(class);
    while offset > 0 && text.get(offset - 1).copied().and_then(class) == target_class {
        offset -= 1;
    }
    doc.set_caret(doc.caret.block, doc.caret.inline, offset);
}

fn table_word_end(doc: &mut Document) {
    let text: Vec<char> = table_cell_text(doc).chars().collect();
    if text.is_empty() {
        return;
    }
    let mut offset = (doc.caret.offset + 1).min(text.len() - 1);
    while offset + 1 < text.len() && text[offset].is_whitespace() {
        offset += 1;
    }
    let target_class = text.get(offset).copied().and_then(class);
    while offset + 1 < text.len() && text.get(offset + 1).copied().and_then(class) == target_class {
        offset += 1;
    }
    doc.set_caret(doc.caret.block, doc.caret.inline, offset);
}

/// The class at flat `pos` of a block, or `None` both for whitespace and
/// for a position at or past the end — treating "off the end" as whitespace
/// is what lets the word motions cross a block boundary without
/// special-casing it at every call site.
fn class_at(doc: &Document, block: usize, flat: usize) -> Option<CharClass> {
    block_text(&doc.scope()[block])
        .chars()
        .nth(flat)
        .and_then(class)
}

/// The caret as a `(block, flat)` pair, clamped into bounds. The model owns
/// this conversion (`Document::caret_position`), so a table row — whose flat
/// space is its cells — is the same read as prose.
fn caret_pos(doc: &Document) -> (usize, usize) {
    let position = doc.caret_position();
    (position.block, position.offset)
}

/// Places the caret at `(block, flat)` — the model's `set_flat_position`
/// does the clamping, the style-before context rule and the flat-to-unit
/// split, so a row's flat position becomes its `(cell, cell-offset)` for
/// free. `flat` past the block's end clamps to it (the block always has
/// ≥1 run).
fn set_pos(doc: &mut Document, block: usize, flat: usize) {
    doc.set_flat_position(FlatPos {
        block,
        offset: flat,
    });
}

/// One character forward, stepping onto the next block when the current one
/// runs out. `None` at the end of the document.
fn step_forward(doc: &Document, block: usize, flat: usize) -> Option<(usize, usize)> {
    if flat < block_len(&doc.scope()[block]) {
        Some((block, flat + 1))
    } else if block + 1 < doc.scope().len() {
        Some((block + 1, 0))
    } else {
        None
    }
}

/// One character back, stepping onto the end of the previous block. `None`
/// at the start of the document.
fn step_back(doc: &Document, block: usize, flat: usize) -> Option<(usize, usize)> {
    if flat > 0 {
        Some((block, flat - 1))
    } else if block > 0 {
        Some((block - 1, block_len(&doc.scope()[block - 1])))
    } else {
        None
    }
}

/// `w`: past the current run (if the caret sits inside one), then past any
/// whitespace. A blank block is a word stop in its own right — without that
/// rule the loop below would step straight through it looking for the next
/// non-whitespace character.
fn word_forward(doc: &Document, start: (usize, usize)) -> (usize, usize) {
    let mut pos = start;
    let start_class = class_at(doc, pos.0, pos.1);
    if start_class.is_some() {
        while class_at(doc, pos.0, pos.1) == start_class {
            match step_forward(doc, pos.0, pos.1) {
                Some(next) => pos = next,
                None => return pos,
            }
        }
    }
    loop {
        if block_text(&doc.scope()[pos.0]).is_empty() && pos.1 == 0 && pos != start {
            return pos;
        }
        if class_at(doc, pos.0, pos.1).is_some() {
            return pos;
        }
        match step_forward(doc, pos.0, pos.1) {
            Some(next) => pos = next,
            None => return pos,
        }
    }
}

/// `b`: the mirror of `word_forward` — step back at least one character to
/// make progress, skip whitespace (a blank block stops it, same as `w`),
/// then walk back to the start of whatever run that lands on.
fn word_back(doc: &Document, start: (usize, usize)) -> (usize, usize) {
    let mut pos = match step_back(doc, start.0, start.1) {
        Some(p) => p,
        None => return start,
    };
    loop {
        if block_text(&doc.scope()[pos.0]).is_empty() {
            return pos;
        }
        if class_at(doc, pos.0, pos.1).is_some() {
            break;
        }
        match step_back(doc, pos.0, pos.1) {
            Some(p) => pos = p,
            None => return pos,
        }
    }
    let run_class = class_at(doc, pos.0, pos.1);
    while let Some(prev) = step_back(doc, pos.0, pos.1) {
        if class_at(doc, prev.0, prev.1) != run_class {
            break;
        }
        pos = prev;
    }
    pos
}

/// `e`: step forward at least once, skip whitespace (blank blocks do *not*
/// stop `e` — only `w`/`b` treat them as their own word), then ride the run
/// under the caret to its last character.
fn word_end(doc: &Document, start: (usize, usize)) -> (usize, usize) {
    let mut pos = match step_forward(doc, start.0, start.1) {
        Some(p) => p,
        None => return start,
    };
    while class_at(doc, pos.0, pos.1).is_none() {
        match step_forward(doc, pos.0, pos.1) {
            Some(p) => pos = p,
            None => return pos,
        }
    }
    let run_class = class_at(doc, pos.0, pos.1);
    while let Some(next) = step_forward(doc, pos.0, pos.1) {
        if class_at(doc, next.0, next.1) != run_class {
            break;
        }
        pos = next;
    }
    pos
}

/// The side an atom-opening motion approaches an atom from: `Some(true)`
/// when it steps *forward* (the caret comes from the atom's left, so the
/// tree opens at its start), `Some(false)` when it steps back (it opens at
/// the end). `None` for motions that do not land on a position an atom can
/// occupy — vertical moves, `Here`, and the block jumps.
fn approaches_from_left(motion: Motion) -> Option<bool> {
    match motion {
        Motion::Right
        | Motion::Append
        | Motion::WordForward
        | Motion::WordEnd
        | Motion::LineEnd
        | Motion::FindForward(_)
        | Motion::TillForward(_) => Some(true),
        Motion::Left
        | Motion::WordBack
        | Motion::LineStart
        | Motion::FirstNonBlank
        | Motion::FindBack(_)
        | Motion::TillBack(_) => Some(false),
        Motion::Here
        | Motion::Up
        | Motion::Down
        | Motion::FirstLine
        | Motion::LastLine
        | Motion::ParagraphBack
        | Motion::ParagraphForward => None,
    }
}

/// Applies `motion` to the document's caret, `count` times. Up/Down are out
/// of scope here — the shell routes them through the layout.
pub fn apply(doc: &mut Document, motion: Motion, count: usize) {
    let count = count.max(1);
    match motion {
        Motion::Here => {}
        // `h`/`l` delegate to the model's style machine, which crosses block
        // boundaries and pops the caret out of a styled run the same way
        // the arrows do — a block end is not a wall for characters.
        Motion::Left => {
            for _ in 0..count {
                if doc.in_table() {
                    doc.table_move(crate::document::TableDirection::Left);
                } else {
                    doc.move_left();
                }
            }
        }
        Motion::Right => {
            for _ in 0..count {
                if doc.in_table() {
                    doc.table_move(crate::document::TableDirection::Right);
                } else {
                    doc.move_right();
                }
            }
        }
        // One logical character right, clamped to the block's end — the
        // model's `move_right` would cross into the next block, which `a`
        // must not do at the end of a line. A row's flat space includes its
        // cells, so this is the same step in prose and in a cell.
        Motion::Append => {
            let (block, flat) = caret_pos(doc);
            set_pos(doc, block, flat.saturating_add(1));
        }
        Motion::Up | Motion::Down => {}
        Motion::WordForward => {
            if doc.in_table() {
                for _ in 0..count {
                    table_word_forward(doc);
                }
            } else {
                let mut pos = caret_pos(doc);
                for _ in 0..count {
                    pos = word_forward(doc, pos);
                }
                set_pos(doc, pos.0, pos.1);
            }
        }
        Motion::WordBack => {
            if doc.in_table() {
                for _ in 0..count {
                    table_word_back(doc);
                }
            } else {
                let mut pos = caret_pos(doc);
                for _ in 0..count {
                    pos = word_back(doc, pos);
                }
                set_pos(doc, pos.0, pos.1);
            }
        }
        Motion::WordEnd => {
            if doc.in_table() {
                for _ in 0..count {
                    table_word_end(doc);
                }
            } else {
                let mut pos = caret_pos(doc);
                for _ in 0..count {
                    pos = word_end(doc, pos);
                }
                set_pos(doc, pos.0, pos.1);
            }
        }
        Motion::LineStart => {
            if doc.in_table() {
                doc.set_caret(doc.caret.block, doc.caret.inline, 0);
            } else {
                doc.move_home();
            }
        }
        Motion::FirstNonBlank => {
            if doc.in_table() {
                let text = table_cell_text(doc);
                let offset = text.chars().take_while(|c| c.is_whitespace()).count();
                doc.set_caret(doc.caret.block, doc.caret.inline, offset);
            } else {
                let (block, _) = caret_pos(doc);
                let text = block_text(&doc.scope()[block]);
                let col = text.chars().take_while(|c| c.is_whitespace()).count();
                set_pos(doc, block, col);
            }
        }
        Motion::LineEnd => {
            if doc.in_table() {
                let len = table_cell_text(doc).chars().count();
                doc.set_caret(doc.caret.block, doc.caret.inline, len);
            } else {
                doc.move_end();
            }
        }
        // A count picks the block (1-based, vim style); plain `gg`/`G` go to
        // the first/last block.
        Motion::FirstLine => {
            let block = count.saturating_sub(1).min(doc.scope().len() - 1);
            set_pos(doc, block, 0);
        }
        Motion::LastLine => {
            let block = if count > 1 {
                count.saturating_sub(1).min(doc.scope().len() - 1)
            } else {
                doc.scope().len() - 1
            };
            set_pos(doc, block, 0);
        }
        Motion::ParagraphBack => {
            let block = doc.caret.block.saturating_sub(count);
            set_pos(doc, block, 0);
        }
        Motion::ParagraphForward => {
            let block = (doc.caret.block + count).min(doc.scope().len() - 1);
            set_pos(doc, block, 0);
        }
        Motion::FindForward(target) => {
            let mut pos = caret_pos(doc);
            for _ in 0..count {
                let text = block_text(&doc.scope()[pos.0]);
                let found = text
                    .chars()
                    .enumerate()
                    .skip(pos.1 + 1)
                    .find(|(_, c)| *c == target);
                if let Some((offset, _)) = found {
                    pos = (pos.0, offset);
                }
            }
            set_pos(doc, pos.0, pos.1);
        }
        Motion::TillForward(target) => {
            let mut pos = caret_pos(doc);
            for _ in 0..count {
                let text = block_text(&doc.scope()[pos.0]);
                let found = text
                    .chars()
                    .enumerate()
                    .skip(pos.1 + 1)
                    .find(|(_, c)| *c == target);
                if let Some((offset, _)) = found {
                    pos = (pos.0, offset.saturating_sub(1));
                }
            }
            set_pos(doc, pos.0, pos.1);
        }
        Motion::FindBack(target) => {
            let mut pos = caret_pos(doc);
            for _ in 0..count {
                let text: Vec<char> = block_text(&doc.scope()[pos.0]).chars().collect();
                let found = text[..pos.1.min(text.len())]
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, c)| **c == target);
                if let Some((offset, _)) = found {
                    pos = (pos.0, offset);
                }
            }
            set_pos(doc, pos.0, pos.1);
        }
        Motion::TillBack(target) => {
            let mut pos = caret_pos(doc);
            for _ in 0..count {
                let text: Vec<char> = block_text(&doc.scope()[pos.0]).chars().collect();
                let found = text[..pos.1.min(text.len())]
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, c)| **c == target);
                if let Some((offset, _)) = found {
                    pos = (pos.0, (offset + 1).min(block_len(&doc.scope()[pos.0])));
                }
            }
            set_pos(doc, pos.0, pos.1);
        }
    }
    // The keyboard's one enter/leave rule for an inline atom, shared by
    // prose and a table cell: a motion that lands the caret on an atom's own
    // position opens its tree, cursor on the side the caret came from; one
    // that lands off it closes the tree. Vertical moves and block jumps do
    // not enter (they land on a block start, not on an atom).
    if let Some(from_left) = approaches_from_left(motion) {
        doc.settle_math_at_caret(from_left);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Focus, Sidenote, math};
    use std::path::Path;

    fn doc(blocks: Vec<Block>) -> Document {
        let mut d = Document::new(Path::new("notes/t.md"));
        *d.body_mut() = blocks;
        d
    }

    fn flat<B>(b: B) -> Block
    where
        B: Into<String>,
    {
        Block::Paragraph(vec![Inline::Text(crate::document::Text {
            text: b.into(),
            style: crate::document::Style::PLAIN,
        })])
    }

    fn pos(doc: &Document) -> (usize, usize) {
        caret_pos(doc)
    }

    #[test]
    fn a_count_multiplies_the_motion_and_then_clears() {
        let mut d = doc(vec![flat("abcdef")]);
        apply(&mut d, Motion::Right, 3);
        assert_eq!(pos(&d), (0, 3), "3l should land on the 4th character");
    }

    #[test]
    fn left_and_right_cross_block_boundaries() {
        let mut d = doc(vec![flat("ab"), flat("cd")]);
        apply(&mut d, Motion::Right, 4);
        assert_eq!(
            pos(&d),
            (1, 1),
            "l crosses into the next block, ending on its last char"
        );

        apply(&mut d, Motion::Left, 5);
        assert_eq!(pos(&d), (0, 0), "h crosses back to the first block start");
    }

    #[test]
    fn gg_and_g_go_to_first_and_last_block() {
        let mut d = doc(vec![flat("one"), flat("two"), flat("three"), flat("four")]);
        d.set_caret(2, 0, 1);

        apply(&mut d, Motion::FirstLine, 1);
        assert_eq!(pos(&d), (0, 0), "gg goes to block 1");

        apply(&mut d, Motion::LastLine, 1);
        assert_eq!(pos(&d), (3, 0), "G goes to the last block");

        apply(&mut d, Motion::LastLine, 2);
        assert_eq!(pos(&d), (1, 0), "2G goes to block 2");
    }

    #[test]
    fn word_forward_crosses_punctuation_whitespace_and_a_blank_block() {
        let mut d = doc(vec![flat("foo, bar"), flat(""), flat("baz")]);
        apply(&mut d, Motion::WordForward, 1);
        assert_eq!(pos(&d), (0, 3), "w stops on the comma");

        apply(&mut d, Motion::WordForward, 1);
        assert_eq!(pos(&d), (0, 5), "w then lands on bar");

        apply(&mut d, Motion::WordForward, 1);
        assert_eq!(pos(&d), (1, 0), "w stops on the blank block");

        apply(&mut d, Motion::WordForward, 1);
        assert_eq!(pos(&d), (2, 0), "w then lands on baz");
    }

    #[test]
    fn word_back_mirrors_word_forward() {
        let mut d = doc(vec![flat("foo, bar"), flat(""), flat("baz")]);
        d.set_caret(2, 0, 0);

        apply(&mut d, Motion::WordBack, 1);
        assert_eq!(pos(&d), (1, 0), "b stops on the blank block");

        apply(&mut d, Motion::WordBack, 1);
        assert_eq!(pos(&d), (0, 5), "b then lands on bar");

        apply(&mut d, Motion::WordBack, 1);
        assert_eq!(pos(&d), (0, 3), "b then lands on the comma");

        apply(&mut d, Motion::WordBack, 1);
        assert_eq!(pos(&d), (0, 0), "b then lands on foo");
    }

    #[test]
    fn word_end_rides_a_run_to_its_last_character() {
        let mut d = doc(vec![flat("foo, bar")]);
        apply(&mut d, Motion::WordEnd, 1);
        assert_eq!(pos(&d), (0, 2), "e ends foo on its last letter");

        apply(&mut d, Motion::WordEnd, 1);
        assert_eq!(pos(&d), (0, 3), "e then ends on the comma itself");

        apply(&mut d, Motion::WordEnd, 1);
        assert_eq!(
            pos(&d),
            (0, 7),
            "e at the last word ends on its last letter"
        );
    }

    #[test]
    fn word_end_crosses_a_block_boundary_when_already_at_a_word_end() {
        let mut d = doc(vec![flat("foo bar"), flat("baz")]);
        d.set_caret(0, 0, 6);

        apply(&mut d, Motion::WordEnd, 1);
        assert_eq!(
            pos(&d),
            (1, 2),
            "already at the end of bar, e crosses onto baz"
        );
    }

    #[test]
    fn append_stays_on_the_current_line_at_its_end() {
        let mut d = doc(vec![flat("one"), flat("two")]);
        d.set_caret(0, 0, 2); // caret after "on", before the second block
        apply(&mut d, Motion::Append, 1);
        assert_eq!(pos(&d), (0, 3), "a at the end of a line stays there");

        // A vim `a` appends after the char under the caret; from mid-line it
        // moves one logical char right, still on the same block.
        d.set_caret(0, 0, 0);
        apply(&mut d, Motion::Append, 1);
        assert_eq!(pos(&d), (0, 1), "a mid-line moves right by one char");
    }

    #[test]
    fn table_motions_respect_cell_boundaries() {
        let mut d = Document::new(Path::new("notes/table.md"));
        d.insert_table();
        d.insert_text("one two");
        d.table_tab(false);
        d.insert_text("other");
        d.set_caret(0, 0, 0);

        apply(&mut d, Motion::WordEnd, 1);
        assert_eq!((d.caret.inline, d.caret.offset), (0, 2));
        apply(&mut d, Motion::WordForward, 1);
        assert_eq!((d.caret.inline, d.caret.offset), (0, 4));
        apply(&mut d, Motion::LineEnd, 1);
        assert_eq!((d.caret.inline, d.caret.offset), (0, 7));
        apply(&mut d, Motion::Right, 1);
        assert_eq!(
            (d.caret.inline, d.caret.offset),
            (1, 0),
            "l crosses one visible cell boundary without concatenating words"
        );
        apply(&mut d, Motion::LineStart, 1);
        assert_eq!((d.caret.inline, d.caret.offset), (1, 0));
    }

    #[test]
    fn word_forward_stops_at_the_note_own_word_boundaries_when_a_note_is_focused() {
        let mut d = doc(vec![flat("aa bb")]);
        d.notes.push(Sidenote {
            label: "1".into(),
            body: vec![flat("ccc ddd")],
            anchored: true,
        });
        d.focus = Focus::Note(0);
        d.set_caret(0, 0, 0);

        apply(&mut d, Motion::WordForward, 1);

        // The note's own gap after "ccc" is offset 4; the body's after "aa"
        // is offset 3 — landing on 4 proves the motion read the note, not the
        // body, even though both live at block 0.
        assert_eq!(pos(&d), (0, 4));
    }

    #[test]
    fn cell_positional_motions_read_the_cells_flat_text() {
        // `a`, `f`, `t`, `F` and `T` are positional reads of the block's
        // flat space. A row's flat space is its cells, so they now run
        // through the generic arm with no per-table branch: the search
        // crosses cell boundaries exactly as it crosses a run boundary.
        let mut d = Document::new(Path::new("notes/table.md"));
        d.insert_table();
        d.insert_text("one two");
        d.set_caret(0, 0, 0);

        apply(&mut d, Motion::Append, 1);
        assert_eq!(
            (d.caret.inline, d.caret.offset),
            (0, 1),
            "a steps one char right"
        );

        apply(&mut d, Motion::FindForward('t'), 1);
        assert_eq!((d.caret.inline, d.caret.offset), (0, 4), "f lands on the t");

        apply(&mut d, Motion::TillForward('o'), 1);
        assert_eq!(
            (d.caret.inline, d.caret.offset),
            (0, 5),
            "t stops before the o"
        );

        apply(&mut d, Motion::FindBack('n'), 1);
        assert_eq!(
            (d.caret.inline, d.caret.offset),
            (0, 1),
            "F walks back to the n"
        );
    }

    /// A paragraph "ab", an atom, "cd" — flat 0..4, the atom at flat 2.
    fn prose_with_atom() -> Document {
        let mut d = Document::new(Path::new("notes/m.md"));
        *d.body_mut() = vec![Block::Paragraph(vec![
            Inline::Text(crate::document::Text {
                text: "ab".into(),
                style: crate::document::Style::PLAIN,
            }),
            Inline::Math(vec![math::MathNode::Sym('x')]),
            Inline::Text(crate::document::Text {
                text: "cd".into(),
                style: crate::document::Style::PLAIN,
            }),
        ])];
        d
    }

    /// A 2x2 table whose first cell is "ab" then a one-symbol atom (flat
    /// 0..2, the atom at 2); the second cell is "cd".
    fn cell_with_atom() -> Document {
        let mut d = Document::new(Path::new("notes/table.md"));
        d.insert_table();
        d.insert_text("ab");
        d.set_caret(0, 0, 2);
        d.insert_inline_math();
        d.math_insert_char('x');
        d.math_exit_after();
        d.set_caret(0, 0, 2);
        assert!(d.math.is_none(), "the fixture starts with the tree closed");
        d
    }

    #[test]
    fn a_prose_motion_onto_an_atom_opens_its_tree() {
        // `l` from "ab" steps onto the atom (flat 2): the caret came from
        // the atom's left, so the tree opens at its start.
        let mut d = prose_with_atom();
        d.set_caret(0, 0, 1);
        apply(&mut d, Motion::Right, 1);
        assert_eq!((d.caret.inline, d.caret.offset), (1, 0));
        assert_eq!(
            d.math,
            Some(math::MathCursor {
                path: Vec::new(),
                index: 0
            }),
            "l onto the atom opens it at its start"
        );

        // `h` from "cd" steps back onto it: the caret came from the right,
        // so the tree opens at its end.
        let mut d = prose_with_atom();
        d.set_caret(0, 2, 0);
        apply(&mut d, Motion::Left, 1);
        assert_eq!((d.caret.inline, d.caret.offset), (1, 0));
        assert_eq!(
            d.math,
            Some(math::MathCursor {
                path: Vec::new(),
                index: 1
            }),
            "h onto the atom opens it at its end"
        );

        // `$` moves to the block's end (one past the atom) and leaves it
        // closed: landing past an atom is not landing on it.
        let mut d = prose_with_atom();
        apply(&mut d, Motion::LineEnd, 1);
        assert_eq!((d.caret.inline, d.caret.offset), (2, 2));
        assert!(d.math.is_none(), "$ lands past the atom, so none opens");

        // A step off an open atom closes the tree and lands on the neighbour.
        let mut d = prose_with_atom();
        d.set_caret(0, 1, 0);
        assert!(d.settle_math_at_caret(true), "the caret is on the atom");
        apply(&mut d, Motion::Left, 1);
        assert!(d.math.is_none(), "h off the atom closes the tree");
        assert_eq!((d.caret.inline, d.caret.offset), (0, 1));
    }

    #[test]
    fn a_cell_motion_onto_an_atom_opens_its_tree() {
        // `h` off the atom lands on the cell's text and leaves the tree
        // closed; `l` back onto it opens it at its start.
        let mut d = cell_with_atom();
        apply(&mut d, Motion::Left, 1);
        assert_eq!((d.caret.inline, d.caret.offset), (0, 1));
        assert!(d.math.is_none());

        apply(&mut d, Motion::Right, 1);
        assert_eq!((d.caret.inline, d.caret.offset), (0, 2));
        assert_eq!(
            d.math,
            Some(math::MathCursor {
                path: Vec::new(),
                index: 0
            }),
            "l onto a cell atom opens it at its start"
        );

        // `h` at the next cell's start crosses to the previous cell's end
        // (a cell boundary is a scope edge), then a second `h` steps onto
        // the atom and enters it from the right.
        let mut d = cell_with_atom();
        d.set_caret(0, 1, 0);
        apply(&mut d, Motion::Left, 1);
        assert_eq!(
            (d.caret.inline, d.caret.offset),
            (0, 3),
            "h at a cell start crosses to the previous cell's end"
        );
        assert!(d.math.is_none());
        apply(&mut d, Motion::Left, 1);
        assert_eq!((d.caret.inline, d.caret.offset), (0, 2));
        assert_eq!(
            d.math,
            Some(math::MathCursor {
                path: Vec::new(),
                index: 1
            }),
            "entering from the right puts the cursor at the atom's end"
        );
    }

    #[test]
    fn a_display_math_cell_line_is_reachable_and_enter_splits_it() {
        // A cell whose only run is an atom — the layout renders it
        // display-style. The keyboard reaches it like any other atom, and
        // Enter inside it exits and splits the cell line, the rule
        // `split_cell_line` already implements.
        let mut d = Document::new(Path::new("notes/equations.md"));
        d.insert_table();
        d.insert_inline_math();
        assert!(
            matches!(
                d.body()[0].cells()[0].lines()[0].as_slice(),
                [Inline::Math(_)]
            ),
            "prune leaves the display cell line as a lone atom: {:?}",
            d.body()[0].cells()[0].lines()[0]
        );
        d.math_insert_char('x');
        d.math_exit_after();

        // Reach it from the keyboard with no mouse: from the right-hand
        // cell, `h` crosses to this cell's end, and a second `h` steps onto
        // the atom and opens the tree from the right.
        d.set_caret(0, 1, 0);
        apply(&mut d, Motion::Left, 1);
        assert_eq!(
            (d.caret.inline, d.caret.offset),
            (0, 1),
            "h from the neighbour crosses to this cell's end"
        );
        assert!(d.math.is_none());
        apply(&mut d, Motion::Left, 1);
        assert_eq!((d.caret.inline, d.caret.offset), (0, 0));
        assert_eq!(
            d.math,
            Some(math::MathCursor {
                path: Vec::new(),
                index: 1
            }),
            "a second h steps onto the display cell atom and opens it"
        );

        // Enter inside the tree: the shell's insert path exits then calls
        // `newline`, which in a table row is `split_cell_line`.
        d.math_exit_after();
        d.newline();
        let lines = d.body()[0].cells()[0].lines();
        assert_eq!(lines.len(), 2, "Enter made a second cell line");
        assert!(matches!(lines[0].as_slice(), [Inline::Math(_)]));
        assert!(
            matches!(lines[1].as_slice(), [Inline::Text(t)] if t.text.is_empty()),
            "the split left a placeholder line below the equation: {:?}",
            lines[1]
        );
    }
}
