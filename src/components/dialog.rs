//! One card over everything: name a new note, or confirm a deletion.
//!
//! The dialog owns no state of its own. The shell holds the live [`Prompt`]
//! (it is the one taking keystrokes) and hands this component a snapshot on
//! every change, so what is drawn is always what Enter would act on.

use crate::layout::Rect;
use crate::renderer::{Layer, Rounding};
use crate::theme::{self, TextStyle};
use crate::ui::{Component, Context, Dirty};

use super::popup::{CARD_RADIUS, Slide, paint_shadow_slab};

const CARD_W: f32 = 460.0;
const CARD_H: f32 = 216.0;
const BUTTON_W: f32 = 96.0;
const BUTTON_H: f32 = 30.0;

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
}

impl Prompt {
    fn title(&self) -> &str {
        match self {
            Self::NewNote { .. } => "New note",
            Self::NewFolder { .. } => "New folder",
            Self::DeleteNote { .. } => "Delete note",
        }
    }

    fn confirm_label(&self) -> &str {
        match self {
            Self::NewNote { .. } => "Create",
            Self::NewFolder { .. } => "Create",
            Self::DeleteNote { .. } => "Delete",
        }
    }

    fn hint(&self) -> &str {
        match self {
            Self::NewNote { .. } => "Enter creates · Esc cancels",
            Self::NewFolder { .. } => "Enter creates · Esc cancels",
            Self::DeleteNote { .. } => "Enter deletes · Esc cancels",
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
    slide: Slide,
    started: bool,
    dirty: Dirty,
}

/// Corner radius of the field and buttons — nested smaller things round
/// less than the card itself.
const FIELD_RADIUS: f32 = 6.0;

impl Dialog {
    pub fn new(prompt: Option<Prompt>) -> Self {
        Self {
            prompt,
            caret_on: true,
            shadow: None,
            reveal: 0.0,
            slide: Slide::new(),
            started: false,
            dirty: Dirty::new(),
        }
    }

    /// Attaches the shell's blurred shadow layer (see `Palette`).
    pub fn with_shadow(mut self, shadow: Layer) -> Self {
        self.shadow = Some(shadow);
        self
    }

    /// Paints the shadow slab from sync; closed clears unconditionally.
    fn paint_shadow(&mut self, owns: bool) {
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
        paint_shadow_slab(&shadow, card(Rect::default()), self.reveal);
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
        self.paint_shadow(context.owns_shadow);
        if self.prompt.is_some() {
            self.dirty.write(&mut self.caret_on, context.caret_on);

            // The pill lands nowhere: a dialog has no row highlight. The
            // Slide is only along for the shared construction; park it off
            // screen so it never draws.
            if !self.started {
                self.slide.park(Rect::default());
                self.started = true;
            }
        }
        self.slide.advance(context.animation_dt);
        if self.slide.advancing() {
            self.dirty.set();
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
            theme::fade(theme::background(), 0.72 * ea),
            Rounding::NONE,
        );

        let resting = card(rect);
        let travel = (1.0 - ea) * -4.0;
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
            Prompt::NewNote { .. } | Prompt::NewFolder { .. } => theme::accent(),
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
