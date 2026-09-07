//! Where the note is: the vault path to the file followed by the heading
//! trail the caret sits under. The status line answers what the editor is
//! doing — spec §3.2 keeps those two jobs apart.

use crate::layout::Rect;
use crate::renderer::{Layer, Rounding};
use crate::theme::{self, TextStyle, icons};
use crate::ui::Component;

pub const HEIGHT: f32 = 34.0;

pub struct Breadcrumb {
    path: Vec<String>,
}

impl Breadcrumb {
    pub fn new(path: Vec<String>) -> Self {
        Self { path }
    }

    fn style() -> TextStyle {
        TextStyle::sans(11.0, theme::faint())
    }
}

impl Component for Breadcrumb {
    fn measure(&mut self, _: &Layer) -> (f32, f32) {
        (300.0, HEIGHT)
    }

    fn draw(&mut self, layer: &Layer, rect: Rect) {
        layer.draw_rectangle(rect.position(), rect.size(), theme::panel(), Rounding::NONE);
        theme::rule(
            layer,
            (rect.x, rect.bottom() - 1.0),
            rect.width,
            1.0,
            theme::border(),
        );
        let middle = rect.y + (rect.height - 2.0) / 2.0;

        theme::icon(
            layer,
            icons::FOLDER,
            (rect.x + 18.0, middle - 6.0),
            12.0,
            theme::faint(),
            1.8,
        );
        let crumb = Self::style();
        let separator = TextStyle::mono(11.5, theme::non_text());
        let mut x = rect.x + 37.0;
        let available = (rect.right() - x - 20.0).max(0.0);
        // Preserve the end of a deep path: that is the current note and heading.
        let mut start = 0;
        let mut total: f32 = self
            .path
            .iter()
            .map(|name| theme::width(layer, name, &crumb) + 22.0)
            .sum();
        while total > available && start + 1 < self.path.len() {
            total -= theme::width(layer, &self.path[start], &crumb) + 22.0;
            start += 1;
        }
        if start > 0 {
            theme::draw(layer, "…  ›", (x, middle), &crumb, theme::LEFT);
            x += 32.0;
        }
        for (index, name) in self.path.iter().enumerate().skip(start) {
            let last = index + 1 == self.path.len();
            let style = if last {
                crumb.clone().color(theme::ink())
            } else {
                crumb.clone()
            };
            let label = theme::elide(layer, name, rect.right() - 20.0 - x, &style);
            theme::draw(layer, &label, (x, middle), &style, theme::LEFT);
            x += theme::width(layer, &label, &style) + 7.0;
            if !last {
                theme::draw(layer, "›", (x, middle), &separator, theme::LEFT);
                x += theme::width(layer, "›", &separator) + 7.0;
            }
        }
    }
}
