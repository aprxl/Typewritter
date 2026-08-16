//! Inside the canvas, not a panel: the margin scrolls with the text column
//! (spec §3.2), unlike the topics list further right. Each note sits beside
//! the anchor it belongs to, resolved by [`stack`] off the typing path.

use crate::document::math_notation;
use crate::document::{Inline, Style};
use crate::layout::Rect;
use crate::prose::Run;
use crate::renderer::{Layer, Rounding};
use crate::theme::{self, TextStyle};
use crate::ui::Component;

pub const WIDTH: f32 = 215.0;

/// The gap a pushed note leaves below the one above it, in document
/// coordinates. Large enough that two crowded notes read as separate, small
/// enough that a pushed note still sits near the sentence it belongs to.
pub const GAP: f32 = 12.0;

/// Notes are set tighter than the page's body text.
const LINE_HEIGHT: f32 = 21.0;

/// One note: its marker (the raised number matching its anchor), its body,
/// and the y it finally sits at after collision resolution.
pub struct Note {
    marker: String,
    body: Vec<Run>,
    y: f32,
}

impl Note {
    pub fn new(marker: &str, body: Vec<Run>, y: f32) -> Self {
        Self {
            marker: marker.into(),
            body,
            y,
        }
    }
}

/// Places notes at their anchors, pushing each one down until it clears the
/// one above. Never pushes up: a note that has moved has still not moved
/// above the sentence it belongs to, which is what keeps the column
/// readable when several anchors land close together.
///
/// `anchors` is `(wanted_y, height)` per note, in document order; the result
/// is the final top of each note in the same order. One forward pass.
pub fn stack(anchors: &[(f32, f32)], gap: f32) -> Vec<f32> {
    let mut out = Vec::with_capacity(anchors.len());
    let mut bottom = f32::NEG_INFINITY;
    for &(y, height) in anchors {
        let placed = y.max(bottom + gap);
        out.push(placed);
        bottom = placed + height;
    }
    out
}

/// The margin's own style for a note's body text: the document's `Style`
/// flags mapped onto the smaller serif the margin uses, so emphasis and
/// code survive the move into the margin.
fn note_style(style: Style) -> TextStyle {
    if style.code {
        return TextStyle::mono(13.5, theme::DIM);
    }
    let mut text = TextStyle::serif(13.5, theme::DIM);
    if style.bold {
        text = text.bold();
    }
    if style.italic {
        text = text.italic();
    }
    text
}

/// Converts a note's body runs to the margin's own run type. A math atom is
/// shown as its linear notation — the margin is a caption, not an editor,
/// and nothing here is meant to be edited in place.
pub fn runs_of(body: &[Inline]) -> Vec<Run> {
    body.iter()
        .filter_map(|run| match run {
            Inline::Text(text) => Some(Run::text(&text.text, note_style(text.style))),
            Inline::Math(list) => Some(Run::text(
                &math_notation::print(list),
                TextStyle::math(12.5, theme::DIM),
            )),
            // An anchor inside a note's body is degenerate; drop it.
            Inline::Note(_) => None,
        })
        .collect()
}

/// One placed word of a note body.
struct Placed {
    x: f32,
    line: usize,
    text: String,
    style: TextStyle,
}

/// Greedy word wrap of a note body. Deliberately a body-only cousin of
/// `prose::Paragraph`: the margin needs a height *before* it draws, so it
/// can stack notes, and `Paragraph::draw` only reports the height after
/// drawing. Both the measure and the draw below run through this one pass,
/// so they can never disagree about where a line breaks.
fn place(runs: &[Run], width: f32, measure: &dyn Fn(&str, &TextStyle) -> f32) -> Vec<Placed> {
    let mut out = Vec::new();
    let (mut cursor, mut line, mut first) = (0.0f32, 0usize, true);
    for run in runs {
        let Run::Text(text, style) = run else {
            continue;
        };
        let space = measure(" ", style);
        for word in text.split_whitespace() {
            let w = measure(word, style);
            if !first && cursor + space + w > width {
                line += 1;
                cursor = 0.0;
            } else if !first {
                cursor += space;
            }
            out.push(Placed {
                x: cursor,
                line,
                text: word.to_string(),
                style: style.clone(),
            });
            cursor += w;
            first = false;
        }
    }
    out
}

/// The height `runs` occupy when wrapped to `width`. Exposed for the shell,
/// which stacks notes in `rebuild_views` before they are drawn.
pub fn body_height(runs: &[Run], width: f32, measure: &dyn Fn(&str, &TextStyle) -> f32) -> f32 {
    let lines = place(runs, width, measure).last().map_or(0, |p| p.line + 1);
    lines as f32 * LINE_HEIGHT
}

/// Draws `runs` wrapped to `width`; returns the height used, so the caller
/// can stack notes without guessing how many lines each took.
fn draw_body(layer: &Layer, runs: &[Run], top_left: (f32, f32), width: f32) -> f32 {
    let placed = place(runs, width, &|text, style| theme::width(layer, text, style));
    for piece in &placed {
        let baseline = top_left.1 + LINE_HEIGHT * (piece.line as f32 + 0.5);
        theme::draw(
            layer,
            &piece.text,
            (top_left.0 + piece.x, baseline),
            &piece.style,
            theme::LEFT,
        );
    }
    let lines = placed.last().map_or(0, |p| p.line + 1);
    lines as f32 * LINE_HEIGHT
}

pub struct SidenoteMargin {
    notes: Vec<Note>,
    /// The editor's content-top offset, so a note aligns with its anchor's
    /// line rather than with the region's own top.
    top: f32,
    /// The editor's scroll, so the margin scrolls with the text (spec §3.2).
    scroll: f32,
}

impl SidenoteMargin {
    pub fn new(notes: Vec<Note>, top: f32, scroll: f32) -> Self {
        Self { notes, top, scroll }
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

        // Document coordinates become screen coordinates the same way the
        // editor does: content top, minus the shared scroll.
        let top = rect.y + self.top - self.scroll;
        for note in &self.notes {
            let y = top + note.y;
            theme::rule(layer, (rect.x, y + 8.0), 14.0, 1.0, theme::BORDER);
            theme::draw(
                layer,
                &note.marker,
                (rect.x + 20.0, y + 4.0),
                &TextStyle::serif(10.0, theme::ACCENT),
                theme::LEFT,
            );
            draw_body(layer, &note.body, (rect.x + 30.0, y), rect.width - 46.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_sits_at_its_anchor() {
        // A lone note keeps the y its anchor asked for.
        assert_eq!(stack(&[(100.0, 20.0)], GAP), vec![100.0]);
    }

    #[test]
    fn a_crowded_note_is_pushed_down_not_up() {
        // Two anchors close together: the second is pushed below the first's
        // bottom, never above its own anchor.
        assert_eq!(
            stack(&[(100.0, 20.0), (105.0, 20.0)], GAP),
            vec![100.0, 132.0]
        );
    }

    #[test]
    fn notes_far_apart_are_left_where_they_are() {
        // Room between the two: neither moves.
        assert_eq!(
            stack(&[(100.0, 20.0), (200.0, 20.0)], GAP),
            vec![100.0, 200.0]
        );
    }

    #[test]
    fn stacking_is_stable_for_an_empty_column() {
        assert_eq!(stack(&[], GAP), Vec::<f32>::new());
    }
}
