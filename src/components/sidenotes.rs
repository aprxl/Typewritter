//! Inside the canvas, not a panel: the margin scrolls with the text column
//! (spec §3.2), unlike the topics list further right.

use crate::layout::Rect;
use crate::prose::{Paragraph, Run};
use crate::renderer::{Layer, Rounding};
use crate::theme::{self, TextStyle};
use crate::ui::Component;

pub const WIDTH: f32 = 215.0;

/// One note: its marker, and its body as a wrappable paragraph.
pub struct Note {
    marker: String,
    body: Paragraph,
}

impl Note {
    pub fn new(marker: &str, runs: Vec<Run>) -> Self {
        Self {
            marker: marker.into(),
            body: Paragraph::new(LINE_HEIGHT, runs),
        }
    }
}

/// Notes are set tighter than the page's body text.
const LINE_HEIGHT: f32 = 21.0;

pub struct SidenoteMargin {
    notes: Vec<Note>,
}

impl SidenoteMargin {
    pub fn new(notes: Vec<Note>) -> Self {
        Self { notes }
    }
}

impl Component for SidenoteMargin {
    fn measure(&mut self, _: &Layer) -> (f32, f32) {
        (150.0, 120.0)
    }

    fn draw(&mut self, layer: &Layer, rect: Rect) {
        layer.draw_rectangle(
            rect.position(),
            rect.size(),
            theme::BACKGROUND,
            Rounding::NONE,
        );
        theme::vertical_rule(layer, (rect.x, rect.y), rect.height, 1.0, theme::SELECTION);

        // Fixed offsets for now: aligning a note to its anchor needs the
        // anchor, and the anchor is a document position we do not have.
        let mut y = rect.y + 600.0;
        for note in &self.notes {
            theme::rule(layer, (rect.x, y + 8.0), 14.0, 1.0, theme::BORDER);
            theme::draw(
                layer,
                &note.marker,
                (rect.x + 20.0, y + 4.0),
                &TextStyle::serif(10.0, theme::ACCENT),
                theme::LEFT,
            );
            y += note.body.draw(layer, (rect.x + 30.0, y), rect.width - 46.0) + 20.0;
        }
    }
}
