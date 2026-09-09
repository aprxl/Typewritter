//! Scratch harness: what does the first of each thing cost?
//!
//! The lazy statics behind syntax highlighting and font handling are built on
//! first use, which lands on whichever frame happens to touch them — usually
//! the first keystroke. This times them in isolation.
//!
//! `cargo run --release --example startup`

use std::time::Instant;

use typewritter::document::code::{self, Language};

fn main() {
    // Every bundled grammar's highlight configuration, built on first use.
    let t = Instant::now();
    let first = code::highlight(Language::Rust, "fn main() {}\n");
    let build = t.elapsed();
    std::hint::black_box(&first);
    println!("first highlight call (builds all grammars): {build:?}");

    let t = Instant::now();
    let again = code::highlight(Language::Rust, "fn other() -> u32 { 1 }\n");
    println!(
        "second highlight call:                      {:?}",
        t.elapsed()
    );
    std::hint::black_box(&again);

    // And once per language, in case the cost is per-grammar rather than
    // all-at-once.
    for language in Language::ALL {
        let t = Instant::now();
        let out = code::highlight(language, "int x = 1;\n");
        std::hint::black_box(&out);
        println!("  {:?}: {:?}", language, t.elapsed());
    }
}
