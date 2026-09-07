//! One card over everything: name a new note, or confirm a deletion.
//!
//! The dialog owns no state of its own. The shell holds the live [`Prompt`]
//! (it is the one taking keystrokes) and hands this component a snapshot on
//! every change, so what is drawn is always what Enter would act on.

use crate::layout::Rect;
use crate::renderer::{Layer, Rounding};
use crate::theme::{self, TextStyle};
use crate::ui::{Component, Context, Dirty};

use super::popup::{CARD_RADIUS, paint_shadow_slab};

const CARD_W: f32 = 460.0;
const CARD_H: f32 = 216.0;
const BUTTON_W: f32 = 96.0;
const BUTTON_H: f32 = 36.0;
/// A switch's pill. Wide enough that the knob's travel reads as travel
/// rather than a twitch, and no taller than the row it shares with its
/// label.
const SWITCH_W: f32 = 44.0;
const SWITCH_H: f32 = 24.0;
/// Gap between the knob and the pill it slides inside.
const KNOB_INSET: f32 = 3.0;

/// What the card is asking.
#[derive(Clone)]
pub enum Prompt {
    /// Name a note to create. Enter creates it.
    NewNote { input: Vec<char>, caret: usize },
    /// Name a folder to create. Enter creates it.
    NewFolder { input: Vec<char>, caret: usize },
    /// Deleting is not undoable and there is no trash, so it is never done
    /// on a keystroke alone — this asks first, and names the file it means.
    DeleteNote { name: String },
    /// Export the open note as a PDF. The palette the page prints in is the
    /// only thing an export gets to decide (`PDF.md` §2), so it is the only
    /// thing this asks. Enter goes on to the file dialog.
    ExportPdf { dark: bool },
}

impl Prompt {
    fn title(&self) -> &str {
        match self {
            Self::NewNote { .. } => "New note",
            Self::NewFolder { .. } => "New folder",
            Self::DeleteNote { .. } => "Delete note",
            Self::ExportPdf { .. } => "Export as PDF",
        }
    }

    fn confirm_label(&self) -> &str {
        match self {
            Self::NewNote { .. } => "Create",
            Self::NewFolder { .. } => "Create",
            Self::DeleteNote { .. } => "Delete",
            Self::ExportPdf { .. } => "Export",
        }
    }

    fn hint(&self) -> &str {
        match self {
            Self::NewNote { .. } => "Enter creates · Esc cancels",
            Self::NewFolder { .. } => "Enter creates · Esc cancels",
            Self::DeleteNote { .. } => "Enter deletes · Esc cancels",
            Self::ExportPdf { .. } => "Space switches · Enter exports · Esc cancels",
        }
    }
}

pub fn card(viewport: Rect) -> Rect {
    Rect::new(
        viewport.x + (viewport.width - CARD_W) / 2.0,
        viewport.y + (viewport.height - CARD_H) / 2.0,
        CARD_W,
        CARD_H,
    )
}

fn field(card: Rect) -> Rect {
    Rect::new(card.x + 18.0, card.y + 64.0, card.width - 36.0, 34.0)
}

/// The palette switch's pill — the same rect the shell hit-tests, so a
/// click lands on the switch that was drawn there.
///
/// It sits in the row a named prompt puts its field in, so both cards have
/// one vertical rhythm rather than two.
pub fn toggle(viewport: Rect) -> Rect {
    let row = field(card(viewport));
    Rect::new(
        row.right() - SWITCH_W,
        row.y + (row.height - SWITCH_H) * 0.5,
        SWITCH_W,
        SWITCH_H,
    )
}

/// `(confirm, cancel)` — the same rects the shell hit-tests, so a click
/// lands on the button that was drawn there.
pub fn buttons(viewport: Rect) -> (Rect, Rect) {
    let card = card(viewport);
    let confirm = Rect::new(
        card.right() - 18.0 - BUTTON_W,
        card.bottom() - 18.0 - BUTTON_H,
        BUTTON_W,
        BUTTON_H,
    );
    let cancel = Rect::new(confirm.x - 12.0 - BUTTON_W, confirm.y, BUTTON_W, BUTTON_H);
    (confirm, cancel)
}

pub struct Dialog {
    prompt: Option<Prompt>,
    caret_on: bool,
    /// The blurred layer this card paints its shadow slab into.
    shadow: Option<Layer>,
    reveal: f32,
    dirty: Dirty,
}

/// Corner radius of the field and buttons — nested smaller things round
/// less than the card itself.
const FIELD_RADIUS: f32 = 8.0;

impl Dialog {
    pub fn new(prompt: Option<Prompt>) -> Self {
        Self {
            prompt,
            caret_on: true,
            shadow: None,
            reveal: 0.0,
            dirty: Dirty::new(),
        }
    }

    /// Attaches the shell's blurred shadow layer (see `Palette`).
    pub fn with_shadow(mut self, shadow: Layer) -> Self {
        self.shadow = Some(shadow);
        self
    }

    /// Paints the shadow slab from sync; closed clears unconditionally.
    fn paint_shadow(&mut self, owns: bool, viewport: Rect) {
        if !owns {
            return;
        }
        let Some(shadow) = self.shadow.clone() else {
            return;
        };
        if self.prompt.is_none() || !(0.0..=1.0).contains(&self.reveal) {
            paint_shadow_slab(&shadow, Rect::default(), -1.0);
            return;
        }
        paint_shadow_slab(&shadow, card(viewport), self.reveal);
    }

    /// Draws the name field, showing as much of the name as fits, with the
    /// caret at its character offset.
    fn draw_field(&self, layer: &Layer, card: Rect, input: &[char], caret: usize) {
        let field = field(card);
        layer.draw_rectangle(
            field.position(),
            field.size(),
            theme::alt(),
            Rounding::uniform(FIELD_RADIUS),
        );
        theme::rounded_outline(
            layer,
            field.inset(0.5),
            FIELD_RADIUS - 0.5,
            1.0,
            theme::border(),
        );

        let style = TextStyle::sans(16.0, theme::ink());
        let middle = field.y + field.height / 2.0;
        let mut shown = String::new();
        let mut shown_width = 0.0;
        for ch in input {
            let advance = theme::width(layer, &ch.to_string(), &style);
            if shown_width + advance > field.width - 30.0 {
                break;
            }
            shown_width += advance;
            shown.push(*ch);
        }
        if shown.is_empty() {
            theme::draw(
                layer,
                "note-name",
                (field.x + 12.0, middle),
                &style.clone().color(theme::faint()),
                theme::LEFT,
            );
        } else {
            theme::draw(layer, &shown, (field.x + 12.0, middle), &style, theme::LEFT);
        }

        if self.caret_on {
            let prefix: String = input[..caret.min(input.len())].iter().collect();
            let x =
                (field.x + 12.0 + theme::width(layer, &prefix, &style)).min(field.right() - 4.0);
            layer.draw_rectangle(
                (x, middle - 10.0),
                (2.0, 20.0),
                theme::accent(),
                Rounding::NONE,
            );
        }
    }

    /// The palette row: what the switch is called, what it changes, and the
    /// pill itself.
    ///
    /// The knob snaps rather than slides. This card is redrawn from a
    /// snapshot the shell hands it on every change and owns no state of its
    /// own, so an animated knob would need somewhere to keep its position
    /// across those rebuilds — a whole mechanism for a 20-pixel slide.
    fn draw_switch(&self, layer: &Layer, card: Rect, track: Rect, on: bool, ea: f32) {
        let middle = track.y + track.height / 2.0;
        theme::draw(
            layer,
            "Dark page",
            (card.x + 18.0, middle),
            &TextStyle::sans(16.0, theme::fade(theme::ink(), ea)),
            theme::LEFT,
        );
        theme::draw(
            layer,
            "Ink and paper both come from the dark palette.",
            (card.x + 18.0, track.bottom() + 22.0),
            &TextStyle::sans(13.5, theme::fade(theme::dim(), ea)),
            theme::LEFT,
        );

        // A stadium: the radius is half the height, so the pill's ends are
        // the same circle the knob is.
        let radius = track.height / 2.0;
        layer.draw_rectangle(
            track.position(),
            track.size(),
            theme::fade(if on { theme::accent() } else { theme::alt() }, ea),
            Rounding::uniform(radius),
        );
        // Off, the pill is the same quiet surface the name field is, and it
        // needs the field's border to read as a control at all. On, the
        // accent is the edge.
        if !on {
            theme::rounded_outline(
                layer,
                track.inset(0.5),
                radius - 0.5,
                1.0,
                theme::fade(theme::border(), ea),
            );
        }
        layer.draw_circle(
            (
                track.x + radius + if on { track.width - track.height } else { 0.0 },
                middle,
            ),
            radius - KNOB_INSET,
            theme::fade(
                if on {
                    theme::background()
                } else {
                    theme::non_text()
                },
                ea,
            ),
        );
    }
}

impl Component for Dialog {
    fn measure(&mut self, _: &Layer) -> (f32, f32) {
        // An overlay bound to the viewport; it asks the layout for nothing.
        (0.0, 0.0)
    }

    fn sync(&mut self, context: &Context) {
        let reveal_changed = self.reveal != context.reveal;
        if reveal_changed {
            self.reveal = context.reveal;
            self.dirty.set();
        }
        self.paint_shadow(context.owns_shadow, context.self_rect);
        if self.prompt.is_some() {
            self.dirty.write(&mut self.caret_on, context.caret_on);
        }
    }

    fn is_dirty(&self) -> bool {
        self.dirty.get()
    }

    fn clear_dirty(&mut self) {
        self.dirty.clear()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn is_animating(&self) -> bool {
        self.prompt.is_some() && (self.reveal > 0.0 && self.reveal < 1.0)
    }

    fn draw(&mut self, layer: &Layer, rect: Rect) {
        let Some(prompt) = &self.prompt else {
            return;
        };
        let ea = self.reveal.clamp(0.0, 1.0);
        // Dimmed rather than opaque — the note underneath is the context
        // for what is being asked — fading up with the card.
        layer.draw_rectangle(
            rect.position(),
            rect.size(),
            theme::fade(theme::background(), 0.48 * ea),
            Rounding::NONE,
        );

        let resting = card(rect);
        let travel = (1.0 - ea) * 8.0;
        let card = Rect::new(resting.x, resting.y + travel, resting.width, resting.height);

        layer.draw_rectangle(
            card.position(),
            card.size(),
            theme::elevated_popup(ea),
            Rounding::uniform(CARD_RADIUS),
        );
        theme::rounded_outline(
            layer,
            card.inset(0.5),
            CARD_RADIUS - 0.5,
            1.0,
            theme::fade(theme::non_text(), ea),
        );

        theme::draw(
            layer,
            prompt.title(),
            (card.x + card.width / 2.0, card.y + 34.0),
            &TextStyle::sans(19.0, theme::fade(theme::ink(), ea)).bold(),
            theme::CENTER,
        );

        match prompt {
            Prompt::NewNote { input, caret } | Prompt::NewFolder { input, caret } => {
                self.draw_field(layer, card, input, *caret)
            }
            Prompt::DeleteNote { name } => {
                theme::draw(
                    layer,
                    name,
                    (card.x + card.width / 2.0, card.y + 82.0),
                    &TextStyle::sans(16.0, theme::fade(theme::ink(), ea)),
                    theme::CENTER,
                );
                theme::draw(
                    layer,
                    "is deleted from disk. This cannot be undone.",
                    (card.x + card.width / 2.0, card.y + 108.0),
                    &TextStyle::sans(13.5, theme::fade(theme::dim(), ea)),
                    theme::CENTER,
                );
            }
            // `rect` and not `card`: the switch is hit-tested from the
            // viewport, so it is drawn from the viewport too, and the two
            // cannot drift.
            Prompt::ExportPdf { dark } => self.draw_switch(layer, card, toggle(rect), *dark, ea),
        }

        theme::draw(
            layer,
            prompt.hint(),
            (card.x + 18.0, card.bottom() - 26.0),
            &TextStyle::mono(10.0, theme::fade(theme::faint(), ea)),
            theme::LEFT,
        );

        let (confirm, cancel) = buttons(rect);
        let accent = match prompt {
            Prompt::NewNote { .. } | Prompt::NewFolder { .. } | Prompt::ExportPdf { .. } => {
                theme::accent()
            }
            // A destructive default deserves a different colour from the
            // one the whole interface uses for "this is where you are".
            Prompt::DeleteNote { .. } => theme::structure(),
        };
        layer.draw_rectangle(
            confirm.position(),
            confirm.size(),
            theme::fade(accent, ea),
            Rounding::uniform(FIELD_RADIUS),
        );
        theme::draw(
            layer,
            prompt.confirm_label(),
            (
                confirm.x + confirm.width / 2.0,
                confirm.y + confirm.height / 2.0,
            ),
            &TextStyle::sans(13.0, theme::fade(theme::background(), ea)),
            theme::CENTER,
        );

        layer.draw_rectangle(
            cancel.position(),
            cancel.size(),
            theme::elevated_popup(ea),
            Rounding::uniform(FIELD_RADIUS),
        );
        theme::rounded_outline(
            layer,
            cancel.inset(0.5),
            FIELD_RADIUS - 0.5,
            1.0,
            theme::fade(theme::border(), ea),
        );
        theme::draw(
            layer,
            "Cancel",
            (
                cancel.x + cancel.width / 2.0,
                cancel.y + cancel.height / 2.0,
            ),
            &TextStyle::sans(13.0, theme::fade(theme::dim(), ea)),
            theme::CENTER,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEWPORT: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 1440.0,
        height: 900.0,
    };

    /// The switch is hit-tested from one function and drawn from another
    /// call to the same one, so what has to hold is that the rect it names
    /// is somewhere a click can reach and nowhere a click already means
    /// something else.
    #[test]
    fn the_switch_sits_inside_the_card_and_clear_of_its_buttons() {
        let card = card(VIEWPORT);
        let track = toggle(VIEWPORT);
        assert!(
            track.x >= card.x && track.right() <= card.right(),
            "the pill is inside the card: {track:?} in {card:?}"
        );
        assert!(track.y >= card.y && track.bottom() <= card.bottom());

        let (confirm, cancel) = buttons(VIEWPORT);
        for button in [confirm, cancel] {
            assert!(
                track.bottom() <= button.y,
                "the pill clears the buttons, or a flick of it confirms the card: \
                 {track:?} against {button:?}"
            );
        }
    }

    #[test]
    fn the_switch_is_a_stadium_the_knob_can_travel_in() {
        let track = toggle(VIEWPORT);
        assert!(
            track.width > track.height,
            "a pill as wide as it is tall has nowhere to slide"
        );
        assert!(
            track.height / 2.0 > KNOB_INSET,
            "the knob must have a positive radius"
        );
    }
}
