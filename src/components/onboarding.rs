//! First-run splash: an opaque full-screen welcome, so the native folder
//! dialog appears over the app instead of out of thin air.
//!
//! The choice itself belongs to the shell (it has to persist the config and
//! swap the tree); this component only draws the invitation and reports
//! where the button is.

use crate::layout::Rect;
use crate::renderer::{Layer, Rounding};
use crate::theme::{self, TextStyle};
use crate::ui::{Component, Context, Dirty, Hover};

use super::popup::CARD_RADIUS;

/// Corner radius of the splash button — nested smaller things round less.
const BUTTON_RADIUS: f32 = 6.0;

const CARD_W: f32 = 480.0;
const CARD_H: f32 = 340.0;
const BUTTON_W: f32 = 210.0;
const BUTTON_H: f32 = 42.0;

fn card(viewport: Rect) -> Rect {
    Rect::new(
        viewport.x + (viewport.width - CARD_W) / 2.0,
        viewport.y + (viewport.height - CARD_H) / 2.0,
        CARD_W,
        CARD_H,
    )
}

/// The button inside the card. Both this component's `draw` and the shell's
/// click handling use it, so what is drawn is exactly what is clickable.
pub fn button(viewport: Rect) -> Rect {
    let card = card(viewport);
    Rect::new(
        card.x + (card.width - BUTTON_W) / 2.0,
        card.bottom() - BUTTON_H - 44.0,
        BUTTON_W,
        BUTTON_H,
    )
}

pub struct Onboarding {
    active: bool,
    button_hover: Hover,
    dirty: Dirty,
}

impl Onboarding {
    pub fn new(active: bool) -> Self {
        Self {
            active,
            button_hover: Hover::new(),
            dirty: Dirty::new(),
        }
    }
}

impl Component for Onboarding {
    fn measure(&mut self, _layer: &Layer) -> (f32, f32) {
        // An overlay bound to the viewport; it must not ask the layout for
        // any space of its own.
        (0.0, 0.0)
    }

    fn sync(&mut self, context: &Context) {
        // The splash *is* the overlay, so it reads the mouse directly
        // rather than through `Context::hovering`.
        let over = self.active
            && context.mouse.in_window
            && button(context.self_rect).contains(context.mouse.position);
        if self.button_hover.update(over, context.animation_dt) {
            self.dirty.set();
        }
    }

    fn is_dirty(&self) -> bool {
        self.dirty.get()
    }

    fn clear_dirty(&mut self) {
        self.dirty.clear();
    }

    fn is_animating(&self) -> bool {
        self.button_hover.is_animating()
    }

    fn draw(&mut self, layer: &Layer, rect: Rect) {
        if !self.active {
            return;
        }
        // Opaque, not a dim: a clean splash hides the shell behind it.
        layer.draw_rectangle(
            rect.position(),
            rect.size(),
            theme::background(),
            Rounding::NONE,
        );

        let card = card(rect);
        layer.draw_rectangle(
            card.position(),
            card.size(),
            theme::elevated_popup(1.0),
            Rounding::uniform(CARD_RADIUS),
        );
        theme::rounded_outline(
            layer,
            card.inset(0.5),
            CARD_RADIUS - 0.5,
            1.0,
            theme::non_text(),
        );

        let center_x = card.x + card.width / 2.0;
        theme::app_mark(layer, Rect::new(center_x - 28.0, card.y + 36.0, 56.0, 56.0));
        theme::draw(
            layer,
            "A home for your thinking.",
            (center_x, card.y + 132.0),
            &TextStyle::sans(26.0, theme::ink()).bold().tracked(-0.035),
            theme::CENTER,
        );
        theme::draw(
            layer,
            "Lecture notes. Beautiful mathematics.",
            (center_x, card.y + 172.0),
            &TextStyle::sans(13.0, theme::dim()),
            theme::CENTER,
        );
        theme::draw(
            layer,
            "Choose a folder and make it yours.",
            (center_x, card.y + 197.0),
            &TextStyle::sans(13.0, theme::dim()),
            theme::CENTER,
        );

        let button = button(rect);
        let weight = self.button_hover.value();
        // Opaque fill (never a translucent band, which reads as text): the
        // button fills with the accent colour while the label blends to the
        // background tone so it stays readable on the solid fill.
        layer.draw_rectangle(
            button.position(),
            button.size(),
            theme::mix(theme::popup(), theme::accent(), weight),
            Rounding::uniform(BUTTON_RADIUS),
        );
        theme::rounded_outline(
            layer,
            button.inset(0.5),
            BUTTON_RADIUS - 0.5,
            1.0,
            theme::mix(theme::accent(), theme::background(), weight),
        );
        theme::draw(
            layer,
            "Choose a vault folder",
            (
                button.x + button.width / 2.0,
                button.y + button.height / 2.0,
            ),
            &TextStyle::sans(
                14.0,
                theme::mix(theme::accent(), theme::background(), weight),
            ),
            theme::CENTER,
        );
        theme::draw(
            layer,
            "or press Enter / Ctrl+O",
            (center_x, button.bottom() + 12.0),
            &TextStyle::mono(10.0, theme::faint()),
            theme::CENTER,
        );
    }
}
