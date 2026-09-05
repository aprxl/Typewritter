//! The empty editor's invitation. Buttons share their bounds with the shell.

use crate::layout::Rect;
use crate::renderer::Layer;
use crate::theme::{self, TextStyle, icons};
use crate::ui::{Context, Hover};

#[derive(Default)]
pub struct EmptyState {
    new_hover: Hover,
    open_hover: Hover,
}

fn origin(rect: Rect) -> (f32, f32) {
    (
        rect.x + rect.width / 2.0,
        rect.y + (rect.height * 0.36).clamp(124.0, 260.0),
    )
}

pub fn buttons(rect: Rect) -> [Rect; 2] {
    let (cx, cy) = origin(rect);
    let width = (rect.width - 64.0).clamp(0.0, 340.0);
    [
        Rect::new(cx - width / 2.0, cy + 90.0, width, 44.0),
        Rect::new(cx - width / 2.0, cy + 142.0, width, 44.0),
    ]
}

impl EmptyState {
    pub fn sync(&mut self, context: &Context) -> bool {
        let [new, open] = buttons(context.self_rect);
        let new = self
            .new_hover
            .update(context.hovering(new), context.animation_dt);
        let open = self
            .open_hover
            .update(context.hovering(open), context.animation_dt);
        new || open
    }

    pub fn is_animating(&self) -> bool {
        self.new_hover.is_animating() || self.open_hover.is_animating()
    }

    pub fn draw(&self, layer: &Layer, rect: Rect) {
        let (cx, cy) = origin(rect);
        // Concentric mathematical construction: quiet native geometry,
        // behind a single luminous page mark.
        for (size, alpha) in [(112.0, 0.28), (144.0, 0.12)] {
            theme::rounded_outline(
                layer,
                Rect::new(cx - size / 2.0, cy - 46.0 - size / 2.0, size, size),
                size * 0.3,
                1.0,
                theme::fade(theme::non_text(), alpha),
            );
        }
        theme::app_mark(layer, Rect::new(cx - 28.0, cy - 74.0, 56.0, 56.0));
        theme::draw(
            layer,
            "Room to think.",
            (cx, cy + 30.0),
            &TextStyle::sans(28.0, theme::ink()).bold().tracked(-0.035),
            theme::CENTER,
        );
        let subtitle = theme::elide(
            layer,
            "Your next idea starts with a note.",
            rect.width - 40.0,
            &TextStyle::sans(13.0, theme::dim()),
        );
        theme::draw(
            layer,
            &subtitle,
            (cx, cy + 60.0),
            &TextStyle::sans(13.0, theme::dim()),
            theme::CENTER,
        );

        for (index, (button, label, icon, hover)) in buttons(rect)
            .into_iter()
            .zip([
                ("New note", icons::PLUS, self.new_hover.value()),
                ("Open a note", icons::SEARCH, self.open_hover.value()),
            ])
            .map(|(rect, (label, icon, hover))| (rect, label, icon, hover))
            .enumerate()
        {
            theme::surface(
                layer,
                button,
                if index == 0 {
                    theme::selection()
                } else {
                    theme::chrome()
                },
                9.0,
            );
            theme::hover_fill(layer, button, hover);
            theme::rounded_outline(layer, button.inset(0.5), 8.5, 1.0, theme::border());
            let middle = button.y + button.height / 2.0;
            theme::icon(
                layer,
                icon,
                (button.x + 16.0, middle - 8.0),
                16.0,
                if index == 0 {
                    theme::accent()
                } else {
                    theme::dim()
                },
                1.7,
            );
            theme::draw(
                layer,
                label,
                (button.x + 44.0, middle),
                &TextStyle::sans(13.0, theme::ink()),
                theme::LEFT,
            );
            if index == 0 {
                theme::keycap(layer, "Ctrl+N", (button.right() - 62.0, middle), 1.0);
            }
        }
        if cy + 224.0 < rect.bottom() - 20.0 {
            theme::draw(
                layer,
                "Made for the speed of thought",
                (cx, cy + 220.0),
                &TextStyle::sans(11.0, theme::faint()),
                theme::CENTER,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_actions_fit_the_minimum_editor() {
        let rect = Rect::new(100.0, 50.0, 420.0, 320.0);
        let [new, open] = buttons(rect);
        for button in [new, open] {
            assert!(button.x >= rect.x && button.right() <= rect.right());
            assert!(button.y >= rect.y && button.bottom() <= rect.bottom());
        }
        assert!(new.bottom() < open.y);
    }
}
