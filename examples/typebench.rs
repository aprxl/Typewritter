//! Scratch harness: what does one keystroke actually cost?
//!
//! Replays single-character inserts through the real `Tabs` edit path and,
//! after each, relays out the document the way `Shell::current_layout` does
//! on every content change. Text measurement is stubbed (no GPU here), so
//! this isolates the layout algorithm rather than glyph shaping.
//!
//! `cargo run --release --example typebench -- <path> [count]`

use std::path::Path;
use std::time::{Duration, Instant};

use typewritter::document::layout;
use typewritter::tabs::Tabs;
use typewritter::theme::TextStyle;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: typebench <path> [count]");
    let count: usize = std::env::args()
        .nth(2)
        .and_then(|n| n.parse().ok())
        .unwrap_or(300);
    let path = Path::new(&path);

    let mut tabs = Tabs::new();
    tabs.open_full(path);

    // Stand-in for `theme::width`, which needs a live layer.
    let measure = |text: &str, style: &TextStyle| text.chars().count() as f32 * style.size * 0.5;

    let mut edit = Vec::with_capacity(count);
    let mut relayout = Vec::with_capacity(count);
    for _ in 0..count {
        let t = Instant::now();
        tabs.type_text("a");
        edit.push(t.elapsed());

        let doc = &tabs.active().expect("open tab").document;
        let t = Instant::now();
        let l = layout::layout(doc, 700.0, &measure);
        relayout.push(t.elapsed());
        std::hint::black_box(&l);
    }

    let bucket = (count / 6).max(1);
    println!("{count} keystrokes, buckets of {bucket}:");
    println!(
        "{:>14} {:>14} {:>14}",
        "keystroke#", "edit avg", "relayout avg"
    );
    for i in 0..(count / bucket) {
        let r = i * bucket..(i * bucket + bucket).min(count);
        let e: Duration = edit[r.clone()].iter().sum::<Duration>() / bucket as u32;
        let l: Duration = relayout[r.clone()].iter().sum::<Duration>() / bucket as u32;
        println!("{:>14} {:>14.3?} {:>14.3?}", r.start, e, l);
    }
    let worst = relayout.iter().max().unwrap();
    println!("\nworst single relayout: {worst:?}");
}
