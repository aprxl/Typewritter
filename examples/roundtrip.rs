//! Scratch harness: does a hand-written note parse to exactly what it says?
//!
//! `parse` then `serialize` is byte-identity on canonical text, so any
//! difference is a construct the file spelled in a way the reader would not
//! have got back. Prints the first differing line pair, plus a structural
//! summary to eyeball.
//!
//! `cargo run --example roundtrip -- <path> [--fix]` — `--fix` writes the
//! canonical form back, so a hand-written note can be normalized by the
//! same code that would normalize it on the reader's first save.

use std::path::Path;

use typewritter::document::markdown;
use typewritter::document::{Block, Inline};

fn main() {
    let path = std::env::args().nth(1).expect("usage: roundtrip <path>");
    let path = Path::new(&path);
    let text = std::fs::read_to_string(path).expect("readable note");
    let document = markdown::parse(path, &text);
    let back = markdown::serialize(&document);

    let mut blocks = 0;
    let mut math = 0;
    let mut inline_math = 0;
    let mut notes = 0;
    let mut refs = 0;
    for block in document.body() {
        blocks += 1;
        if matches!(block, Block::Math { .. }) {
            math += 1;
        }
        for run in block.inlines() {
            match run {
                Inline::Math(_) => inline_math += 1,
                Inline::Note(_) => notes += 1,
                Inline::EqRef(_) => refs += 1,
                Inline::Text(_) => {}
                Inline::TableCell(contents) => {
                    for run in contents {
                        match run {
                            Inline::Math(_) => inline_math += 1,
                            Inline::Note(_) => notes += 1,
                            Inline::EqRef(_) => refs += 1,
                            Inline::Text(_) => {}
                            Inline::TableCell(_) => {
                                unreachable!("nested table cells are invalid")
                            }
                        }
                    }
                }
            }
        }
    }
    println!(
        "{blocks} blocks, {math} display equations, {inline_math} inline expressions, \
         {notes} note anchors, {refs} equation references, {} note bodies",
        document.notes.len()
    );

    let original: Vec<&str> = text.lines().collect();
    let printed: Vec<&str> = back.lines().collect();
    let mut clean = true;
    for (index, (a, b)) in original.iter().zip(&printed).enumerate() {
        if a != b {
            println!("\nline {} differs:\n  wrote: {a}\n  reads: {b}", index + 1);
            clean = false;
            break;
        }
    }
    if clean && original.len() != printed.len() {
        println!(
            "\nline count differs: wrote {}, reads {}",
            original.len(),
            printed.len()
        );
        let shared = original.len().min(printed.len());
        for line in original.iter().skip(shared).take(3) {
            println!("  only in source: {line}");
        }
        for line in printed.iter().skip(shared).take(3) {
            println!("  only in output: {line}");
        }
        clean = false;
    }
    if !clean && std::env::args().any(|arg| arg == "--fix") {
        std::fs::write(path, &back).expect("writable note");
        println!("\nrewrote {} in canonical form", path.display());
        return;
    }
    println!(
        "{}",
        if clean {
            "\ncanonical"
        } else {
            "\nNOT canonical"
        }
    );
}
