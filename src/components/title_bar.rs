//! Workspace toolbar. Every control owns the geometry used by input routing.

use super::theme_switch;
use crate::layout::Rect;
use crate::renderer::{Layer, Rounding};
use crate::theme::{self, TextStyle, icons};
use crate::ui::{Component, Context, Dirty, Hover};

pub const HEIGHT: f32 = 60.0;

pub struct TitleBar {
    query: Option<String>,
    caret_on: bool,
    search_hover: Hover,
    sidebar_hover: Hover,
    focus_hover: Hover,
    focus_mode: bool,
    switch_hover: Hover,
    locked: bool,
    dirty: Dirty,
}

pub fn sidebar_rect(bar: Rect) -> Rect {
    Rect::new(bar.x + 188.0, bar.y + 13.0, 34.0, 34.0)
}

pub fn focus_rect(bar: Rect) -> Rect {
    Rect::new(
        theme_switch::switch_rect(bar).x - 94.0,
        bar.y + 13.0,
        80.0,
        34.0,
    )
}

pub fn search_box_rect(bar: Rect) -> Rect {
    let left = sidebar_rect(bar).right() + 24.0;
    let right = focus_rect(bar).x - 24.0;
    let width = (right - left).clamp(0.0, 340.0);
    Rect::new(
        (bar.x + (bar.width - width) / 2.0).clamp(left, right.max(left) - width),
        bar.y + 12.0,
        width,
        36.0,
    )
}

impl TitleBar {
    pub fn new(query: Option<String>) -> Self {
        Self {
            query,
            caret_on: true,
            search_hover: Hover::new(),
            sidebar_hover: Hover::new(),
            focus_hover: Hover::new(),
            focus_mode: false,
            switch_hover: Hover::new(),
            locked: false,
            dirty: Dirty::new(),
        }
    }
}

impl Component for TitleBar {
    fn measure(&mut self, _: &Layer) -> (f32, f32) {
        (620.0, HEIGHT)
    }

    fn sync(&mut self, context: &Context) {
        let bar = context.self_rect;
        let search = search_box_rect(bar);
        let mut changed = self
            .search_hover
            .update(context.hovering(search), context.animation_dt);
        changed |= self
            .sidebar_hover
            .update(context.hovering(sidebar_rect(bar)), context.animation_dt);
        changed |= self
            .focus_hover
            .update(context.hovering(focus_rect(bar)), context.animation_dt);
        changed |= self.switch_hover.update(
            !context.theme_locked && context.hovering(theme_switch::switch_rect(bar)),
            context.animation_dt,
        );
        self.dirty.write(&mut self.locked, context.theme_locked);
        self.dirty.write(&mut self.focus_mode, context.focus_mode);
        if self.query.is_some() {
            self.dirty.write(&mut self.caret_on, context.caret_on);
        }
        if changed {
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
        self.search_hover.is_animating()
            || self.sidebar_hover.is_animating()
            || self.focus_hover.is_animating()
            || self.switch_hover.is_animating()
    }

    fn draw(&mut self, layer: &Layer, bar: Rect) {
        theme::surface(layer, bar, theme::chrome(), 0.0);
        theme::rule(
            layer,
            (bar.x, bar.bottom() - 1.0),
            bar.width,
            1.0,
            theme::border(),
        );
        let middle = bar.y + bar.height / 2.0;
        theme::app_mark(layer, Rect::new(bar.x + 20.0, middle - 15.0, 30.0, 30.0));
        theme::draw(
            layer,
            "Typewritter",
            (bar.x + 61.0, middle),
            &TextStyle::sans(16.0, theme::ink()).bold().tracked(-0.02),
            theme::LEFT,
        );

        let sidebar = sidebar_rect(bar);
        theme::hover_fill(layer, sidebar, self.sidebar_hover.value());
        theme::icon(
            layer,
            icons::SIDEBAR,
            (sidebar.x + 8.0, middle - 9.0),
            18.0,
            theme::dim(),
            1.6,
        );

        let search = search_box_rect(bar);
        theme::surface(layer, search, theme::background(), 9.0);
        theme::hover_fill(layer, search, self.search_hover.value());
        theme::rounded_outline(
            layer,
            search.inset(0.5),
            8.5,
            1.0,
            if self.query.is_some() {
                theme::accent()
            } else {
                theme::border()
            },
        );
        theme::icon(
            layer,
            icons::SEARCH,
            (search.x + 12.0, middle - 7.0),
            14.0,
            theme::faint(),
            1.7,
        );
        let style = TextStyle::sans(
            12.5,
            if self.query.is_some() {
                theme::ink()
            } else {
                theme::faint()
            },
        );
        let label = theme::elide(
            layer,
            self.query.as_deref().unwrap_or("Search notes"),
            search.width - 76.0,
            &style,
        );
        theme::draw(
            layer,
            &label,
            (search.x + 36.0, middle),
            &style,
            theme::LEFT,
        );
        if self.query.is_some() && self.caret_on {
            layer.draw_rectangle(
                (
                    search.x + 36.0 + theme::width(layer, &label, &style),
                    middle - 8.0,
                ),
                (1.5, 16.0),
                theme::accent(),
                Rounding::uniform(0.75),
            );
        }

        let focus = focus_rect(bar);
        if self.focus_mode {
            theme::surface(layer, focus, theme::selection(), 8.0);
        }
        theme::hover_fill(layer, focus, self.focus_hover.value());
        theme::icon(
            layer,
            icons::FOCUS,
            (focus.x + 8.0, middle - 7.0),
            14.0,
            if self.focus_mode {
                theme::accent()
            } else {
                theme::dim()
            },
            1.6,
        );
        theme::draw(
            layer,
            "Focus",
            (focus.x + 30.0, middle),
            &TextStyle::sans(
                12.0,
                if self.focus_mode {
                    theme::accent()
                } else {
                    theme::dim()
                },
            ),
            theme::LEFT,
        );
        theme_switch::draw(
            layer,
            theme_switch::switch_rect(bar),
            self.switch_hover.value(),
            self.locked,
        );
    }
}
