//! Where a motion sends the caret, and the character-class rules `w`/`b`/`e`
//! use to find word boundaries.
//!
//! Motions work on the document's blocks as flat text: a position is a
//! `(block, flat)` pair into that block's concatenated run text. A block
//! boundary is whitespace (words never straddle one), which is what lets `w`
//! and `b` cross blocks without special-casing the ends. Up/Down stay out of
//! here — moving between *visual* lines needs pixels, so the shell applies
//! them through the layout's `line_up`/`line_down`.

use crate::document::{Block, Document, Inline};

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

/// The block's flat text — its runs' strings concatenated.
fn block_text(block: &Block) -> String {
    block
        .inlines()
        .iter()
        .map(|run| match run {
            Inline::Text(t) => t.text.as_str(),
            Inline::Math(_) => "\u{FFFC}",
            Inline::Note(_) => "\u{FFFC}",
        })
        .collect()
}

fn block_len(block: &Block) -> usize {
    block_text(block).chars().count()
}

/// The class at flat `pos` of a block, or `None` both for whitespace and
/// for a position at or past the end — treating "off the end" as whitespace
/// is what lets the word motions cross a block boundary without
/// special-casing it at every call site.
fn class_at(doc: &Document, block: usize, flat: usize) -> Option<CharClass> {
    block_text(&doc.blocks[block])
        .chars()
        .nth(flat)
        .and_then(class)
}

/// The caret as a `(block, flat)` pair, clamped into bounds.
fn caret_pos(doc: &Document) -> (usize, usize) {
    let caret = doc.caret;
    let block = &doc.blocks[caret.block];
    let runs = block.inlines();
    let inline = caret.inline.min(runs.len().saturating_sub(1));
    let prefix: usize = runs[..inline]
        .iter()
        .map(|run| match run {
            Inline::Text(t) => t.text.chars().count(),
            Inline::Math(_) => 1,
            Inline::Note(_) => 1,
        })
        .sum();
    let offset = caret.offset.min(block_len(block) - prefix);
    let flat = (prefix + offset).min(block_len(block));
    (caret.block, flat)
}

/// Places the caret at `(block, flat)` — the model's `set_caret` does the
/// clamping and the style-before context rule. `flat` past the block's end
/// clamps to it (the block always has ≥1 run).
fn set_pos(doc: &mut Document, block: usize, flat: usize) {
    let len = block_len(&doc.blocks[block]);
    if len == 0 {
        doc.set_caret(block, 0, 0);
        return;
    }
    let flat = flat.min(len);
    let runs = doc.blocks[block].inlines();
    let mut acc = 0;
    for (i, run) in runs.iter().enumerate() {
        let run_len = match run {
            Inline::Text(t) => t.text.chars().count(),
            Inline::Math(_) => 1,
            Inline::Note(_) => 1,
        };
        if flat < acc + run_len {
            doc.set_caret(block, i, flat - acc);
            return;
        }
        acc += run_len;
    }
    // Flat landed exactly at the end of the last run.
    let last = runs.len() - 1;
    let last_len = match &runs[last] {
        Inline::Text(t) => t.text.chars().count(),
        Inline::Math(_) => 1,
        Inline::Note(_) => 1,
    };
    doc.set_caret(block, last, last_len);
}

/// One character forward, stepping onto the next block when the current one
/// runs out. `None` at the end of the document.
fn step_forward(doc: &Document, block: usize, flat: usize) -> Option<(usize, usize)> {
    if flat < block_len(&doc.blocks[block]) {
        Some((block, flat + 1))
    } else if block + 1 < doc.blocks.len() {
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
        Some((block - 1, block_len(&doc.blocks[block - 1])))
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
        if block_text(&doc.blocks[pos.0]).is_empty() && pos.1 == 0 && pos != start {
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
        if block_text(&doc.blocks[pos.0]).is_empty() {
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
                doc.move_left();
            }
        }
        Motion::Right => {
            for _ in 0..count {
                doc.move_right();
            }
        }
        Motion::Append => {
            // One logical character right, clamped to the block's end —
            // `move_right` would cross into the next block, which `a` must
            // not do at the end of a line.
            let (block, flat) = caret_pos(doc);
            set_pos(
                doc,
                block,
                flat.saturating_add(1).min(block_len(&doc.blocks[block])),
            );
        }
        Motion::Up | Motion::Down => {}
        Motion::WordForward => {
            let mut pos = caret_pos(doc);
            for _ in 0..count {
                pos = word_forward(doc, pos);
            }
            set_pos(doc, pos.0, pos.1);
        }
        Motion::WordBack => {
            let mut pos = caret_pos(doc);
            for _ in 0..count {
                pos = word_back(doc, pos);
            }
            set_pos(doc, pos.0, pos.1);
        }
        Motion::WordEnd => {
            let mut pos = caret_pos(doc);
            for _ in 0..count {
                pos = word_end(doc, pos);
            }
            set_pos(doc, pos.0, pos.1);
        }
        Motion::LineStart => doc.move_home(),
        Motion::FirstNonBlank => {
            let (block, _) = caret_pos(doc);
            let text = block_text(&doc.blocks[block]);
            let col = text.chars().take_while(|c| c.is_whitespace()).count();
            set_pos(doc, block, col);
        }
        Motion::LineEnd => doc.move_end(),
        // A count picks the block (1-based, vim style); plain `gg`/`G` go to
        // the first/last block.
        Motion::FirstLine => {
            let block = count.saturating_sub(1).min(doc.blocks.len() - 1);
            set_pos(doc, block, 0);
        }
        Motion::LastLine => {
            let block = if count > 1 {
                count.saturating_sub(1).min(doc.blocks.len() - 1)
            } else {
                doc.blocks.len() - 1
            };
            set_pos(doc, block, 0);
        }
        Motion::ParagraphBack => {
            let block = doc.caret.block.saturating_sub(count);
            set_pos(doc, block, 0);
        }
        Motion::ParagraphForward => {
            let block = (doc.caret.block + count).min(doc.blocks.len() - 1);
            set_pos(doc, block, 0);
        }
        Motion::FindForward(target) => {
            let mut pos = caret_pos(doc);
            for _ in 0..count {
                let text = block_text(&doc.blocks[pos.0]);
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
                let text = block_text(&doc.blocks[pos.0]);
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
                let text: Vec<char> = block_text(&doc.blocks[pos.0]).chars().collect();
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
                let text: Vec<char> = block_text(&doc.blocks[pos.0]).chars().collect();
                let found = text[..pos.1.min(text.len())]
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, c)| **c == target);
                if let Some((offset, _)) = found {
                    pos = (pos.0, (offset + 1).min(block_len(&doc.blocks[pos.0])));
                }
            }
            set_pos(doc, pos.0, pos.1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn doc(blocks: Vec<Block>) -> Document {
        let mut d = Document::new(Path::new("notes/t.md"));
        // Build a source-like flat text so `parse` produces the blocks we
        // hand it. Simpler: set blocks directly (they are `pub`).
        d.blocks = blocks;
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
}
