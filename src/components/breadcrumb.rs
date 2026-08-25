//! Where the note is: the vault path to the file followed by the heading
//! trail the caret sits under. The status line answers what the editor is
//! doing — spec §3.2 keeps those two jobs apart.

use crate::layout::Rect;
use crate::renderer::{Layer, Rounding};
use crate::theme::{self, TextStyle, icons};
use crate::ui::{Component, Context, Dirty, Hover};

pub const HEIGHT: f32 = 32.0;

pub struct Breadcrumb {
    path: Vec<String>,
    hover: Hover,
    dirty: Dirty,
}

impl Breadcrumb {
    pub fn new(path: Vec<String>) -> Self {
        Self {
            path,
            hover: Hover::new(),
            dirty: Dirty::new(),
        }
    }

    fn style() -> TextStyle {
        TextStyle::mono(11.5, theme::dim())
    }
}

impl Component for Breadcrumb {
    fn measure(&mut self, layer: &Layer) -> (f32, f32) {
        // Only the last two crumbs are a floor: a deep path elides, and a
        // minimum that grew with the depth would push the window wider for
        // a file that happens to be nested.
        let style = Self::style();
        let tail: f32 = self
            .path
            .iter()
            .rev()
            .take(2)
            .map(|c| theme::width(layer, c, &style) + 22.0)
            .sum();
        (tail + 46.0, HEIGHT)
    }

    fn sync(&mut self, context: &Context) {
        let over = context.hovering(context.self_rect);
        if self.hover.update(over, context.animation_dt) {
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
        self.hover.is_animating()
    }

    fn draw(&mut self, layer: &Layer, rect: Rect) {
        layer.draw_rectangle(rect.position(), rect.size(), theme::chrome(), Rounding::NONE);
        theme::hover_fill(layer, rect, self.hover.value());
        theme::rule(
            layer,
            (rect.x, rect.bottom() - 2.0),
            rect.width,
            2.0,
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
        for (index, name) in self.path.iter().enumerate() {
            let last = index + 1 == self.path.len();
            let style = if last {
                crumb.clone().color(theme::ink())
            } else {
                crumb.clone()
            };
            theme::draw(layer, name, (x, middle), &style, theme::LEFT);
            x += theme::width(layer, name, &style) + 7.0;
            if !last {
                theme::draw(layer, "›", (x, middle), &separator, theme::LEFT);
                x += theme::width(layer, "›", &separator) + 7.0;
            }
        }
    }
}
