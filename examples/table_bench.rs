//! Reproducible performance harness for tables.
//!
//!     cargo run --release --example table_bench
//!
//! **Release, because that is the only profile whose timings are worth
//! anything.** A debug build measures the same thing 10–30x slower for
//! reasons that have to do with `debug` and nothing to do with tables. The
//! harness prints the profile it was built with, so a wrong-profile run is
//! visible in its own output. (Allocation counts are identical in both
//! profiles, so a debug run still answers the "how many" question.)
//!
//! **Determinism.** Every case is measured with a counting global allocator,
//! so the `allocs` and `bytes` columns are reproducible: run it twice on the
//! same tree and they do not move. The `micros` column is one wall-clock
//! sample of one pass and moves with the machine, the scheduler and whatever
//! else is running — compare allocation counts before and after a refactor
//! and treat micros as a sanity check, not a measurement.
//!
//! What it measures, one section per performance rule in `CONTRACT.md` §5:
//!
//!   * **typing** — 20 `Document::insert_text` keystrokes into one cell of a
//!     2x2, 8x4 and 24x4 table, then the same 20 keystrokes into a 20- and a
//!     120-paragraph document as the prose baseline. The measured point:
//!     an edit costs a document-wide walk today, not a per-edit one.
//!   * **layout** — one `document::layout::layout(&doc, 700.0, &measure)`
//!     pass over tables of 2, 8, 24, 48 and 64 rows at 1 and 4 columns,
//!     reported per pass and per cell, plus a 24x4 table of EMPTY cells to
//!     separate the per-cell structural cost from the text.
//!   * **drag** — 20 `Document::resize_table_column` steps on a 24x4 table.
//!   * **paint** — the per-row walk inside `Editor::draw_table`
//!     (`src/components/editor.rs:202`). `draw_table` needs a live Atomos
//!     `Layer` (a wgpu render target), so it cannot be called from here; the
//!     walk is mirrored in `paint_table` instead. That function's comment
//!     lists exactly which lines are faithful and what is omitted — read it
//!     before trusting the numbers.
//!
//! The baseline these numbers record is in `WORK.md`.

use std::alloc::{GlobalAlloc, Layout as AllocLayout, System};
use std::fmt::Write as _;
use std::hint::black_box;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use typewritter::document::ATOM;
use typewritter::document::layout::{DocLayout, layout};
use typewritter::document::markdown::parse;
use typewritter::document::{Block, Inline};
use typewritter::theme::TextStyle;

// ---- the counting allocator ------------------------------------------------

struct Counting;

static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: AllocLayout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: AllocLayout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: AllocLayout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(new_size.saturating_sub(layout.size()), Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

fn reset() {
    ALLOCS.store(0, Ordering::Relaxed);
    BYTES.store(0, Ordering::Relaxed);
}

fn counts() -> (usize, usize) {
    (
        ALLOCS.load(Ordering::Relaxed),
        BYTES.load(Ordering::Relaxed),
    )
}

/// Run one case with the counters reset, returning `(allocs, bytes, micros)`.
/// Nothing in here allocates, so every allocation counted belongs to `run`.
fn measure(run: impl FnOnce()) -> (usize, usize, f64) {
    reset();
    let start = Instant::now();
    run();
    let micros = start.elapsed().as_secs_f64() * 1e6;
    let (allocs, bytes) = counts();
    (allocs, bytes, micros)
}

// ---- reporting -------------------------------------------------------------

fn header() {
    println!(
        "{:<46} {:>8} {:>9} {:>9}  per",
        "case", "allocs", "bytes", "micros"
    );
}

fn report(label: &str, allocs: usize, bytes: usize, micros: f64, per: &str) {
    println!("{label:<46} {allocs:>8} {bytes:>9} {micros:>9.1}  {per}");
}

// ---- fixtures --------------------------------------------------------------

const KEYSTROKES: usize = 20;
const LAYOUT_WIDTH: f32 = 700.0;
const CHAR_WIDTH: f32 = 8.0;

/// The one rendering input `layout` takes: a fixed advance per character, so
/// wrapping is a function of the text and nothing else.
fn width_of(text: &str, _style: &TextStyle) -> f32 {
    text.chars().count() as f32 * CHAR_WIDTH
}

fn push_row(out: &mut String, columns: usize, lead: &str) {
    out.push('|');
    for column in 0..columns {
        write!(out, " {lead}{column} |").expect("writing to a String cannot fail");
    }
}

/// A pipe table of `rows` rows (header included) and `columns` columns with a
/// short word in every cell. Block 0 is the header row, so block 1 is the
/// first data row — which is where the typing cases put the caret.
fn table_markdown(rows: usize, columns: usize) -> String {
    let mut out = String::new();
    push_row(&mut out, columns, "h");
    out.push('\n');
    out.push('|');
    for _ in 0..columns {
        out.push_str(" --- |");
    }
    for row in 1..rows {
        out.push('\n');
        push_row(&mut out, columns, &format!("r{row}"));
    }
    out.push('\n');
    out
}

/// The same shape with every cell empty. Text out of the picture, what is
/// left per cell is the model's own structure.
fn table_markdown_empty(rows: usize, columns: usize) -> String {
    let empty_row = " |".repeat(columns);
    let divider = " --- |".repeat(columns);
    let mut out = format!("|{empty_row}\n|{divider}");
    for _ in 1..rows {
        write!(out, "\n|{empty_row}").expect("writing to a String cannot fail");
    }
    out.push('\n');
    out
}

fn prose_markdown(paragraphs: usize) -> String {
    let mut out = String::new();
    for paragraph in 0..paragraphs {
        writeln!(out, "paragraph number {paragraph} with some words in it")
            .expect("writing to a String cannot fail");
        out.push('\n');
    }
    out
}

/// The one place this harness looks inside a cell, for the sanity line under
/// the typing cases. The table model is being refactored (a cell becomes a
/// A row's first cell, read through the container shape: a cell owns its lines,
/// so this is the one place here that names it.
fn first_cell_text(row: &Block) -> String {
    row.cells()
        .first()
        .map(|cell| cell.runs().iter().map(run_text).collect())
        .unwrap_or_default()
}

/// A run's readable text: an opaque atom is one position, never its guts.
fn run_text(run: &Inline) -> String {
    match run {
        Inline::Text(text) => text.text.clone(),
        _ => ATOM.to_string(),
    }
}

// ---- the paint path --------------------------------------------------------

/// The per-row body of `Editor::draw_table` (`src/components/editor.rs:202`)
/// with the `Layer` calls removed, because a `Layer` is a wgpu render target
/// and there is no headless one to hand.
///
/// Reproduced exactly, and these are the lines the case is about:
///
///   * the whole-table viewport test, `bottom < rect.y || top > rect.bottom()`
///     (`:217`), so a table scrolled entirely off screen really does return
///     before any row work;
///   * `for row in first..end` (`:272`) — EVERY row of the table, because
///     there is still no per-row viewport guard (`CONTRACT.md` §5.3 asks for
///     one), so one visible row costs allocations proportional to the whole
///     table;
///   * per cell, `Block::Paragraph(contents.to_vec())` (`:284`);
///   * per visual line, `Vec::with_capacity(line.segments.len())` (`:308`);
///   * per segment, the sliced `String` (`:321`).
///
/// `black_box` keeps the optimizer from deleting the clone and the pieces
/// vector, which the real function reads and this one does not. Omitted:
/// `theme::width` (a glyph measure through the layer, which may allocate
/// inside glyphon) and every `layer.draw_*` call — so this mirror is a lower
/// bound on the real paint path, never an overstatement.
///
/// Returns the number of rows walked: `0` means the early return fired, and
/// equals `end - first` otherwise.
fn paint_table(doc_layout: &DocLayout, first: usize, scroll: f32, viewport_height: f32) -> usize {
    let Some(table) = doc_layout.tables.get(first).and_then(Option::as_ref) else {
        return 0;
    };
    let mut end = first + 1;
    while matches!(
        doc_layout.source.get(end),
        Some(Block::TableRow { first: false, .. })
    ) {
        end += 1;
    }
    let top = doc_layout.blocks[first].y - scroll;
    let bottom = doc_layout.blocks[end - 1].y + doc_layout.blocks[end - 1].height - scroll;
    if bottom < 0.0 || top > viewport_height {
        return 0;
    }
    for row in first..end {
        let row_table = doc_layout.tables[row]
            .as_ref()
            .expect("table row must have table layout");
        for (column, cell) in doc_layout.source[row].cells().iter().enumerate() {
            let contents = cell.runs();
            let cell_block = Block::Paragraph(contents.to_vec());
            black_box(&cell_block);
            for line in &row_table.cells[column] {
                let mut pieces: Vec<String> = Vec::with_capacity(line.segments.len());
                for segment in &line.segments {
                    let run = &contents[segment.inline];
                    let text: String = match run {
                        Inline::Text(text) => text
                            .text
                            .chars()
                            .skip(segment.start)
                            .take(segment.len)
                            .collect(),
                        _ => segment.number.clone().unwrap_or_else(|| ATOM.to_string()),
                    };
                    pieces.push(text);
                }
                black_box(&pieces);
            }
        }
    }
    black_box(&table.columns);
    end - first
}

// ---- the cases -------------------------------------------------------------

fn typing_cases() {
    println!("\n-- typing: 20 `Document::insert_text` keystrokes --");
    for (rows, columns) in [(2usize, 2usize), (8, 4), (24, 4)] {
        let markdown = table_markdown(rows, columns);
        let mut doc = parse(Path::new("bench.md"), &markdown);
        // Block 1 is the first data row; cell 0, offset 0 inside it.
        doc.set_caret(1, 0, 0);
        let (allocs, bytes, micros) = measure(|| {
            for _ in 0..KEYSTROKES {
                doc.insert_text("x");
            }
        });
        report(
            &format!("typing: {rows}x{columns} table, one cell"),
            allocs,
            bytes,
            micros,
            &format!("{:.1} allocs/keystroke", allocs as f64 / KEYSTROKES as f64),
        );
        if (rows, columns) == (24, 4) {
            let cell = first_cell_text(&doc.body()[1]);
            println!(
                "      sanity: first data cell {:?} — {} of the {KEYSTROKES} keystrokes in it",
                cell,
                cell.chars().filter(|c| *c == 'x').count(),
            );
        }
    }
    for paragraphs in [20usize, 120] {
        let markdown = prose_markdown(paragraphs);
        let mut doc = parse(Path::new("bench.md"), &markdown);
        doc.set_caret(0, 0, 0);
        let (allocs, bytes, micros) = measure(|| {
            for _ in 0..KEYSTROKES {
                doc.insert_text("x");
            }
        });
        report(
            &format!("typing: {paragraphs}-paragraph document (prose)"),
            allocs,
            bytes,
            micros,
            &format!("{:.1} allocs/keystroke", allocs as f64 / KEYSTROKES as f64),
        );
    }
}

fn layout_cases() {
    for columns in [1usize, 4] {
        println!("\n-- layout: one `layout(&doc, 700.0, &measure)` pass, {columns} column(s) --");
        for rows in [2usize, 8, 24, 48, 64] {
            let markdown = table_markdown(rows, columns);
            let doc = parse(Path::new("bench.md"), &markdown);
            let cells = rows * columns;
            let (allocs, bytes, micros) = measure(|| {
                black_box(layout(&doc, LAYOUT_WIDTH, &width_of));
            });
            report(
                &format!("layout: {rows}x{columns} ({cells} cells), one pass"),
                allocs,
                bytes,
                micros,
                &format!(
                    "{:.1} allocs/cell, {:.1} KB/cell",
                    allocs as f64 / cells as f64,
                    bytes as f64 / cells as f64 / 1024.0
                ),
            );
        }
    }
    println!("\n-- layout: empty cells, to separate structure from text --");
    let doc = parse(Path::new("bench.md"), &table_markdown_empty(24, 4));
    let (allocs, bytes, micros) = measure(|| {
        black_box(layout(&doc, LAYOUT_WIDTH, &width_of));
    });
    report(
        "layout: 24x4, EMPTY cells, one pass",
        allocs,
        bytes,
        micros,
        &format!("{:.1} allocs/cell", allocs as f64 / 96.0),
    );
}

fn drag_cases() {
    println!("\n-- drag: 20 `Document::resize_table_column` steps on a 24x4 table --");
    let mut doc = parse(Path::new("bench.md"), &table_markdown(24, 4));
    // A drag alternates direction, so the shares never pin against the floor.
    let (allocs, bytes, micros) = measure(|| {
        for step in 0..KEYSTROKES {
            doc.resize_table_column(0, 0, 0.002 * ((step % 3) + 1) as f32);
        }
    });
    report(
        "drag: 24x4, 20 column steps",
        allocs,
        bytes,
        micros,
        &format!("{:.1} allocs/step", allocs as f64 / KEYSTROKES as f64),
    );
}

fn paint_cases() {
    println!("\n-- paint: the per-row walk of `Editor::draw_table` (mirrored, no GPU) --");
    // The viewport is about one row tall, so the first case has one row on
    // screen and the rest of the table below the fold.
    const VIEWPORT: f32 = 40.0;
    for (rows, scroll, label) in [
        (2usize, 0.0f32, "2x4, first row on screen"),
        (64, 0.0, "64x4, first row on screen"),
        (64, 100_000.0, "64x4, whole table scrolled far away"),
    ] {
        let doc = parse(Path::new("bench.md"), &table_markdown(rows, 4));
        let laid = layout(&doc, LAYOUT_WIDTH, &width_of);
        let mut walked = 0;
        let (allocs, bytes, micros) = measure(|| {
            walked = paint_table(&laid, 0, scroll, VIEWPORT);
        });
        report(
            &format!("paint: {label}"),
            allocs,
            bytes,
            micros,
            &format!("{walked} of {rows} rows walked (0 = early return)"),
        );
    }
}

fn main() {
    let profile = if cfg!(debug_assertions) {
        "debug (counts are right, timings are not — run --release)"
    } else {
        "release"
    };
    println!("table_bench — profile {profile}");
    println!(
        "measure = {CHAR_WIDTH} px/char, layout width {LAYOUT_WIDTH}, \
         allocation counts repeat run to run, micros do not"
    );
    header();
    typing_cases();
    layout_cases();
    drag_cases();
    paint_cases();
}
