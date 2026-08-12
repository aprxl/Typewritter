//! App identity and the vault search affordance.

use crate::layout::Rect;
use crate::renderer::{Layer, Rounding};
use crate::theme::{self, TextStyle, icons};
use crate::ui::{Component, Context, Dirty, Hover};

pub const HEIGHT: f32 = 48.0;

pub struct TitleBar {
    title: String,
    kind: String,
    search: String,
    finder_open: bool,
    caret_on: bool,
    shortcut: String,
    /// Filled in by `draw`, hit-tested by `sync` on the next frame.
    search_rect: Rect,
    hover: Hover,
    dirty: Dirty,
}

impl TitleBar {
    pub fn new(title: &str, kind: &str, finder_query: Option<String>) -> Self {
        Self {
            title: title.into(),
            kind: kind.into(),
            finder_open: finder_query.is_some(),
            search: finder_query.unwrap_or_default(),
            caret_on: true,
            shortcut: "⌘ /".into(),
            search_rect: Rect::default(),
            hover: Hover::new(),
            dirty: Dirty::new(),
        }
    }
}

fn compact_text<F>(text: &str, max_width: f32, mut width: F) -> String
where
    F: FnMut(&str) -> f32,
{
    if max_width <= 0.0 {
        return String::new();
    }
    if width(text) <= max_width {
        return text.into();
    }

    let marker = "...";
    if width(marker) <= max_width {
        let mut suffix = String::new();
        for character in text.chars().rev() {
            let candidate = format!("{marker}{character}{suffix}");
            if width(&candidate) > max_width {
                break;
            }
            suffix.insert(0, character);
        }
        return format!("{marker}{suffix}");
    }

    let mut suffix = String::new();
    for character in text.chars().rev() {
        let candidate = format!("{character}{suffix}");
        if width(&candidate) > max_width {
            break;
        }
        suffix.insert(0, character);
    }
    suffix
}

/// Search-box geometry shared by drawing and shell-level click handling.
pub fn search_box_rect(layer: &Layer, rect: Rect) -> Rect {
    let search_style = TextStyle::serif(12.5, theme::COMMENT);
    let shortcut_style = TextStyle::mono(10.0, theme::FAINT).tracked(0.1);
    let box_width = theme::width(layer, "search the vault", &search_style)
        + theme::width(layer, "⌘ /", &shortcut_style)
        + 55.0;
    Rect::new(
        rect.right() - 18.0 - box_width,
        rect.y + (rect.height - 2.0) / 2.0 - 13.0,
        box_width,
        26.0,
    )
}

impl Component for TitleBar {
    fn measure(&mut self, layer: &Layer) -> (f32, f32) {
        let title = theme::width(layer, &self.title, &TextStyle::serif(17.0, theme::INK));
        let search = theme::width(
            layer,
            "search the vault",
            &TextStyle::serif(12.5, theme::COMMENT),
        );
        (title + search + 190.0, HEIGHT)
    }

    fn sync(&mut self, context: &Context) {
        let over = context.hovering(self.search_rect);
        let mut dirty = self.hover.update(over, context.animation_dt);
        if self.finder_open {
            let before = self.caret_on;
            self.caret_on = context.caret_on;
            dirty |= before != self.caret_on;
        }
        if dirty {
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
        layer.draw_rectangle(rect.position(), rect.size(), theme::PANEL, Rounding::NONE);
        theme::rule(
            layer,
            (rect.x, rect.bottom() - 2.0),
            rect.width,
            2.0,
            theme::BORDER,
        );
        let middle = rect.y + (rect.height - 2.0) / 2.0;

        // The mark: a filled square with the wordmark's initial knocked out.
        layer.draw_rectangle(
            (rect.x + 18.0, middle - 12.0),
            (24.0, 24.0),
            theme::ACCENT,
            Rounding::NONE,
        );
        theme::draw(
            layer,
            "T",
            (rect.x + 30.0, middle),
            &TextStyle::mono(14.0, theme::BACKGROUND).bold(),
            theme::CENTER,
        );

        let title_style = TextStyle::serif(17.0, theme::INK);
        theme::draw(
            layer,
            &self.title,
            (rect.x + 54.0, middle),
            &title_style,
            theme::LEFT,
        );
        let mut x = rect.x + 54.0 + theme::width(layer, &self.title, &title_style) + 9.0;
        theme::vertical_rule(layer, (x, middle - 6.0), 12.0, 1.0, theme::BORDER);
        x += 10.0;
        theme::draw(
            layer,
            &self.kind,
            (x, middle),
            &TextStyle::mono(9.5, theme::COMMENT).tracked(0.2),
            theme::LEFT,
        );

        // Search sits against the right edge, so it is laid out backwards
        // from there and simply runs off if the window is too narrow.
        let shortcut_style = TextStyle::mono(10.0, theme::FAINT).tracked(0.1);
        let search_style = TextStyle::serif(
            12.5,
            if self.finder_open {
                theme::INK
            } else {
                theme::COMMENT
            },
        );
        self.search_rect = search_box_rect(layer, rect);
        let box_left = self.search_rect.x;
        let box_right = self.search_rect.right();
        theme::hover_fill(layer, self.search_rect, self.hover.value());
        theme::outline(
            layer,
            self.search_rect,
            if self.finder_open {
                theme::ACCENT
            } else if self.hover.value() > 0.0 {
                theme::fade(theme::ACCENT, 0.25 + self.hover.value() * 0.75)
            } else {
                theme::BORDER
            },
        );
        theme::icon(
            layer,
            icons::SEARCH,
            (box_left + 9.0, middle - 6.5),
            13.0,
            theme::COMMENT,
            1.8,
        );
        let input_right = box_right - 9.0;
        let displayed = if self.finder_open {
            compact_text(&self.search, input_right - (box_left + 30.0), |text| {
                theme::width(layer, text, &search_style)
            })
        } else {
            "search the vault".into()
        };
        theme::draw(
            layer,
            &displayed,
            (box_left + 30.0, middle),
            &search_style,
            theme::LEFT,
        );
        if self.finder_open {
            if self.caret_on {
                let x = (box_left + 30.0 + theme::width(layer, &displayed, &search_style))
                    .min(input_right - 2.0);
                layer.draw_rectangle(
                    (x, middle - 8.0),
                    (2.0, 16.0),
                    theme::ACCENT,
                    Rounding::NONE,
                );
            }
        } else {
            theme::draw(
                layer,
                &self.shortcut,
                (box_right - 9.0, middle),
                &shortcut_style,
                theme::RIGHT,
            );
        }
    }
}
