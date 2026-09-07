//! PDF export: the editor's page on a sheet of paper.
//!
//! One call, [`export_pdf`]. The design, the decisions behind it, and the
//! recipe for adding an element are in [`PDF.md`](../../PDF.md) — read §1
//! and §6 of it before changing anything here.
//!
//! The pipeline is five stages and each is one file:
//!
//! ```text
//! Document ─ layout ─→ DocLayout ─ paginate ─→ Pages ─ paint ─→ Canvas ─→ PDF
//! ```
//!
//! Only `paint` and `pdf` are new work. `layout` is `document::layout`,
//! unchanged and shared with the editor, which is what makes the page and
//! the screen break their lines in the same places rather than merely in
//! similar ones.

mod geometry;
mod notes;
mod paginate;
mod paint;
mod pdf;
mod text;

use std::path::Path;

use krilla::page::PageSettings;

use crate::components::editor::MEASURE;
use crate::document::layout::{self, DocLayout};
use crate::document::{Block, Document};
use crate::renderer::Layer;
use crate::theme::{self, TextStyle, Theme};

pub use geometry::{PageGeometry, Paper};

/// What an export is free to decide. Everything else — the column width,
/// the line breaks, the type — is the document's and is not negotiable.
#[derive(Clone, Debug)]
pub struct Options {
    pub paper: Paper,
    /// The palette the page is printed with.
    ///
    /// Strict 1:1 would print a dark note on black. This defaults to
    /// [`Theme::LIGHT`] regardless of what the editor is showing, because
    /// printing a dark page is a choice almost nobody makes on purpose —
    /// and the one who does can say so here.
    pub theme: Theme,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            paper: Paper::A4,
            theme: Theme::LIGHT,
        }
    }
}

#[derive(Debug)]
pub enum Error {
    Render(krilla::error::KrillaError),
    Write(std::io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Render(e) => write!(f, "could not build the PDF: {e:?}"),
            Error::Write(e) => write!(f, "could not write the PDF: {e}"),
        }
    }
}

/// Render `document` to a PDF at `path`.
///
/// `layer` is the editor's own — the export measures and shapes through it,
/// so the page's line breaks are the ones on screen rather than a second
/// shaper's opinion of them. That makes this a main-thread, synchronous
/// call; it is not on the typing path and has no business being fast.
pub fn export_pdf(
    document: &Document,
    layer: &Layer,
    path: &Path,
    options: Options,
) -> Result<(), Error> {
    let bytes = render(document, layer, options)?;
    std::fs::write(path, bytes).map_err(Error::Write)
}

/// [`export_pdf`], stopping at the bytes. Separate so a test can render
/// without touching the filesystem.
pub fn render(document: &Document, layer: &Layer, options: Options) -> Result<Vec<u8>, Error> {
    // `layout::text_style` reads ink colours *while laying out*, so the
    // palette has to be in place before the layout pass, not between it and
    // the paint pass. Restored below: this is a global, and the editor is
    // still showing whatever the reader chose.
    let restore = theme::current();
    theme::set(options.theme);

    let measure = |text: &str, style: &TextStyle| theme::width(layer, text, style);
    let layout = lay_out(document, &measure);
    // Both geometries lay the text column out at `MEASURE`, so the layout
    // above is the same either way and the choice can wait until after it.
    // Only the sheet under it changes: a document with notes needs room
    // beside its column, and pays for it in scale (`PDF.md` §2, decision 2).
    let geometry = if notes::present(document, &layout) {
        PageGeometry::noted(options.paper)
    } else {
        PageGeometry::plain(options.paper)
    };
    let pages = paginate::paginate(&layout, &geometry);
    // After pagination, never before: a note follows its anchor onto
    // whichever page the prose put it on, and never moves the prose.
    let placed = notes::place(document, &layout, &pages, geometry.content_height, &measure);
    let numbers = heading_numbers(&layout);

    let shaper = text::Shaper::new(layer);
    let mut fonts = pdf::Fonts::default();
    let mut pdf = krilla::Document::new();
    for (page, notes) in pages.iter().zip(&placed) {
        let settings = PageSettings::new(
            krilla::geom::Size::from_wh(geometry.paper.width, geometry.paper.height)
                .expect("a sheet has a positive size"),
        );
        let mut sheet = pdf.start_page_with(settings);
        let mut canvas = pdf::PdfCanvas::new(sheet.surface(), &shaper, layer, &mut fonts, geometry);
        // First, and edge to edge: everything below is drawn on it.
        paint::ground(&mut canvas, geometry.sheet());
        paint::page(&mut canvas, &layout, page, &numbers, geometry.content_width);
        paint::notes(&mut canvas, notes);
    }

    let bytes = pdf.finish();
    theme::set(restore);
    bytes.map_err(Error::Render)
}

/// The document's layout at the page's column width.
///
/// [`MEASURE`] and not a page-derived width: the editor's column is a fixed
/// number of pixels, so laying out at exactly that number is what makes the
/// page's line breaks the editor's, character for character. Every
/// [`PageGeometry`] agrees — they differ in how large that column prints,
/// never in how wide it is.
///
/// Not the editor's cached layout either: folds are a reading aid, and a
/// collapsed section prints in full (`PDF.md` §2, decision 3), so this is
/// taken from a copy with every fold opened.
fn lay_out(document: &Document, measure: &dyn Fn(&str, &TextStyle) -> f32) -> DocLayout {
    let mut unfolded = document.clone();
    for block in unfolded.body_mut() {
        if let Block::Heading { folded, .. } = block {
            *folded = false;
        }
    }
    layout::layout(&unfolded, MEASURE, measure)
}

/// The heading auto-numbers, derived the way the editor derives them —
/// from position, never stored.
fn heading_numbers(layout: &DocLayout) -> Vec<Option<String>> {
    let mut numbers = vec![None; layout.source.len()];
    for node in crate::document::outline::outline(&layout.source) {
        numbers[node.block] = Some(node.number);
    }
    numbers
}
