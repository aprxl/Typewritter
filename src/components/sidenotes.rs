//! Inside the canvas, not a panel: the margin scrolls with the text column
//! (spec §3.2), unlike the topics list further right. Each note sits beside
//! the anchor it belongs to, resolved by [`stack`] off the typing path.
//!
//! A note's body is drawn by the same [`Editor`] the page uses, set smaller
//! by one scale factor. The margin owns no text layout of its own, so a line
//! can never break in two places because two wrappers disagreed.

use std::rc::Rc;

use crate::components::editor::{Editor, Metrics};
use crate::document::layout::DocLayout;
use crate::document::{Caret, Style};
use crate::layout::Rect;
use crate::renderer::{Layer, Rounding};
use crate::theme::{self, TextStyle};
use crate::ui::{Component, Context, Dirty};

pub const WIDTH: f32 = 215.0;

/// The gap a pushed note leaves below the one above it, in document
/// coordinates. Large enough that two crowded notes read as separate, small
/// enough that a pushed note still sits near the sentence it belongs to.
pub const GAP: f32 = 12.0;

/// Notes are set smaller than the page's body text, by one scale factor so
/// the note's layout and its drawing read one number and cannot disagree
/// about where a line breaks.
pub const SCALE: f32 = 13.5 / 17.5;

/// The note text column's inset within the margin, past the marker and rule.
pub const NOTE_INSET: f32 = 30.0;
/// Space kept at the note column's right edge.
const NOTE_RIGHT_MARGIN: f32 = 16.0;

/// The note text column's width — what each note's layout is laid out at.
pub const NOTE_WIDTH: f32 = WIDTH - NOTE_INSET - NOTE_RIGHT_MARGIN;

// The tick beside a note, and the marker above it. Short and small enough to
// read as punctuation rather than as a rule — the rule is the vertical one at
// the margin's own edge. The page draws both from these too, so a note in an
// export sits where a note on screen sits.

/// How far below the note's top the tick starts.
pub const TICK_TOP: f32 = 8.0;
pub const TICK_LENGTH: f32 = 14.0;
pub const TICK_THICKNESS: f32 = 1.0;

/// The marker's left edge, from the margin column's.
pub const MARKER_X: f32 = 20.0;
/// The marker's top, from the note's.
pub const MARKER_TOP: f32 = 4.0;
const MARKER_SIZE: f32 = 10.0;

/// The ink a note's marker draws with. The accent, matching the anchor in
/// the prose it answers, so the eye pairs the two without counting.
///
/// A function rather than a constant because the colour is the theme's, and
/// the theme is read when it is drawn — the same shape as
/// [`layout::anchor_style`](crate::document::layout::anchor_style), which is
/// the other half of this pair.
pub fn marker_style() -> TextStyle {
    TextStyle::sans(MARKER_SIZE, theme::accent())
}

/// The editor metrics a note draws with: the margin's own insets and measure,
/// no page furniture. An editor embedded in the margin fills no background;
/// its container already painted. The focused note's editor still draws the
/// caret and current-line band — the `caret` it is given is what enables that,
/// not the page flag.
const NOTE_METRICS: Metrics = Metrics {
    inset: NOTE_INSET,
    top: 0.0,
    measure: NOTE_WIDTH,
    right_margin: NOTE_RIGHT_MARGIN,
    page: false,
};

/// One note: its marker (the raised number matching its anchor), the editor
/// that draws its body, and the y it finally sits at after collision
/// resolution.
pub struct Note {
    marker: String,
    editor: Editor,
    y: f32,
}

impl Note {
    /// Builds a note over `layout`, already laid out at the margin's width
    /// and scale. `caret` is `Some` only for the focused note — its editor
    /// then draws the caret and the current-line band, and blinks in step
    /// with the page's, because it is the page's, just drawn here.
    pub fn new(
        marker: &str,
        layout: Rc<DocLayout>,
        y: f32,
        caret: Option<Caret>,
        block_caret: bool,
    ) -> Self {
        let caret_style = caret.map_or(Style::PLAIN, |caret| caret.style);
        let editor = Editor::new(layout, caret, 0.0, block_caret, caret_style, NOTE_METRICS);
        Self {
            marker: marker.into(),
            editor,
            y,
        }
    }

    /// The note's laid-out height, in document coordinates.
    pub fn height(&self) -> f32 {
        self.editor.content_height()
    }

    /// Whether this is the focused note — its editor draws the caret.
    pub fn is_focused(&self) -> bool {
        self.editor.has_caret()
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

pub struct SidenoteMargin {
    notes: Vec<Note>,
    /// The editor's content-top offset, so a note aligns with its anchor's
    /// line rather than with the region's own top.
    top: f32,
    /// The editor's scroll, so the margin scrolls with the text (spec §3.2).
    scroll: f32,
    dirty: Dirty,
}

impl SidenoteMargin {
    pub fn new(notes: Vec<Note>, top: f32, scroll: f32) -> Self {
        Self {
            notes,
            top,
            scroll,
            dirty: Dirty::new(),
        }
    }
}

impl Component for SidenoteMargin {
    fn measure(&mut self, _: &Layer) -> (f32, f32) {
        (150.0, 120.0)
    }

    fn sync(&mut self, context: &Context) {
        // The notes' editors are read-only today, but forwarding sync now is
        // what lets a caret blink in one the day it is not.
        for note in &mut self.notes {
            note.editor.sync(context);
        }
    }

    fn is_dirty(&self) -> bool {
        self.dirty.get() || self.notes.iter().any(|note| note.editor.is_dirty())
    }

    fn clear_dirty(&mut self) {
        self.dirty.clear();
        for note in &mut self.notes {
            note.editor.clear_dirty();
        }
    }

    fn draw(&mut self, layer: &Layer, rect: Rect) {
        layer.draw_rectangle(
            rect.position(),
            rect.size(),
            theme::background(),
            Rounding::NONE,
        );
        theme::vertical_rule(
            layer,
            (rect.x, rect.y),
            rect.height,
            1.0,
            theme::selection(),
        );

        // Document coordinates become screen coordinates the same way the
        // editor does: content top, minus the shared scroll.
        let top = rect.y + self.top - self.scroll;
        for note in &mut self.notes {
            let y = top + note.y;
            // The focused note's tick is accent, the others' is the quiet
            // border — one small cue, on top of the caret that already blinks
            // there, so the focused note reads at a glance.
            let tick = if note.is_focused() {
                theme::accent()
            } else {
                theme::border()
            };
            theme::rule(
                layer,
                (rect.x, y + TICK_TOP),
                TICK_LENGTH,
                TICK_THICKNESS,
                tick,
            );
            theme::draw(
                layer,
                &note.marker,
                (rect.x + MARKER_X, y + MARKER_TOP),
                &marker_style(),
                theme::LEFT,
            );
            // The note editor draws into the note's own placed rectangle,
            // which is exactly the note's laid-out height.
            let note_rect = Rect {
                x: rect.x,
                y,
                width: rect.width,
                height: note.editor.content_height(),
            };
            note.editor.draw(layer, note_rect);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::layout::layout_blocks;
    use crate::document::{Block, Inline, Text};

    fn para(text: &str) -> Block {
        Block::Paragraph(vec![Inline::Text(Text {
            text: text.into(),
            style: Style::PLAIN,
        })])
    }

    /// Every glyph 10 wide, so line breaks are countable by hand.
    fn fake_measure(text: &str, _: &TextStyle) -> f32 {
        text.chars().count() as f32 * 10.0
    }

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

    #[test]
    fn a_notes_height_is_its_own_layouts_not_a_guess() {
        // A note body that wraps is taller than one that does not, and the
        // height used for stacking is the note's own layout height — a second
        // note clears the first's real bottom, not a fixed line-count guess.
        let tall = Rc::new(layout_blocks(
            &[para("word word word word word")],
            NOTE_WIDTH,
            SCALE,
            &fake_measure,
        ));
        let small = Rc::new(layout_blocks(
            &[para("word")],
            NOTE_WIDTH,
            SCALE,
            &fake_measure,
        ));
        assert!(tall.height > small.height);

        let note = Note::new("1", tall.clone(), 0.0, None, false);
        assert_eq!(note.editor.content_height(), tall.height);

        let wanted = vec![(100.0, tall.height), (100.0, small.height)];
        assert_eq!(stack(&wanted, GAP), vec![100.0, 100.0 + tall.height + GAP]);
    }
}
