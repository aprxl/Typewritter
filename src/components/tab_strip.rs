//! Open notes across the top: activate, close, and the new-note button.
//!
//! The strip mutates the tabs it shows, so it holds the shared [`Tabs`]
//! rather than a copy of it. What it *draws* is a snapshot the shell hands
//! it, refreshed whenever the revision moves — a component that re-read the
//! model every frame would have to re-measure every frame too.

use std::cell::RefCell;
use std::rc::Rc;

use crate::layout::Rect;
use crate::renderer::{Layer, Rounding};
use crate::tabs::Tabs;
use crate::theme::{self, TextStyle, icons};
use crate::ui::{Component, Context, Dirty, Hover};

pub const HEIGHT: f32 = 46.0;
/// Room for the icon, the padding either side, and the close button.
const TAB_CHROME: f32 = 69.0;
const PLUS_WIDTH: f32 = 42.0;

/// One tab as the strip draws it.
pub struct TabView {
    pub name: String,
    pub preview: bool,
    pub unsaved: bool,
}

/// Where a tab's close button is. Shared by the drawing and the hit-test so
/// what is clickable is exactly what is drawn.
fn close_rect(tab: Rect) -> Rect {
    Rect::new(tab.right() - 24.0, tab.y + 4.0, 16.0, tab.height - 8.0)
}

pub struct TabStrip {
    docs: Rc<RefCell<Tabs>>,
    tabs: Vec<TabView>,
    active: Option<usize>,
    /// Filled in by `draw`, hit-tested by `sync` on the next frame.
    tab_rects: Vec<Rect>,
    plus_rect: Rect,
    hovered: Option<usize>,
    hover: Hover,
    plus_hover: Hover,
    dirty: Dirty,
}

impl TabStrip {
    pub fn new(docs: Rc<RefCell<Tabs>>, tabs: Vec<TabView>, active: Option<usize>) -> Self {
        Self {
            docs,
            tabs,
            active,
            tab_rects: Vec::new(),
            plus_rect: Rect::default(),
            hovered: None,
            hover: Hover::new(),
            plus_hover: Hover::new(),
            dirty: Dirty::new(),
        }
    }

    fn style(tab: &TabView) -> TextStyle {
        let style = TextStyle::sans(
            12.5,
            if tab.preview {
                theme::comment()
            } else {
                theme::ink()
            },
        );
        if tab.preview { style.italic() } else { style }
    }
}

impl Component for TabStrip {
    fn measure(&mut self, layer: &Layer) -> (f32, f32) {
        let width: f32 = self
            .tabs
            .iter()
            .map(|tab| theme::width(layer, &tab.name, &Self::style(tab)) + TAB_CHROME)
            .sum();
        ((width + PLUS_WIDTH + 24.0).min(600.0), HEIGHT)
    }

    fn sync(&mut self, context: &Context) {
        let hovered = context.hovered_index(&self.tab_rects);
        if self
            .hover
            .track(&mut self.hovered, hovered, context.animation_dt)
        {
            self.dirty.set();
        }
        if self
            .plus_hover
            .update(context.hovering(self.plus_rect), context.animation_dt)
        {
            self.dirty.set();
        }

        if let Some(position) = context.click_position()
            && let Some(index) = self
                .tab_rects
                .iter()
                .position(|rect| rect.contains(position))
        {
            if close_rect(self.tab_rects[index]).contains(position) {
                let _ = self.docs.borrow_mut().try_close(index);
            } else {
                self.docs.borrow_mut().activate(index);
            }
        }
    }

    fn is_dirty(&self) -> bool {
        self.dirty.get()
    }

    fn clear_dirty(&mut self) {
        self.dirty.clear();
    }

    fn is_animating(&self) -> bool {
        self.hover.is_animating() || self.plus_hover.is_animating()
    }

    fn draw(&mut self, layer: &Layer, rect: Rect) {
        layer.draw_rectangle(
            rect.position(),
            rect.size(),
            theme::chrome(),
            Rounding::NONE,
        );
        theme::rule(
            layer,
            (rect.x, rect.bottom() - 1.0),
            rect.width,
            1.0,
            theme::border(),
        );
        let middle = rect.y + (rect.height - 1.0) / 2.0;

        let mut x = rect.x + 12.0;
        self.tab_rects.clear();
        for (index, tab) in self.tabs.iter().enumerate() {
            let style = Self::style(tab);
            let width = theme::width(layer, &tab.name, &style) + TAB_CHROME;
            let tab_rect = Rect::new(x, rect.y + 6.0, width, rect.height - 12.0);
            self.tab_rects.push(tab_rect);

            let active = self.active == Some(index);
            if active {
                layer.draw_rectangle(
                    tab_rect.position(),
                    tab_rect.size(),
                    theme::background(),
                    Rounding::uniform(8.0),
                );
                theme::rounded_outline(layer, tab_rect.inset(0.5), 7.5, 1.0, theme::border());
            }
            if self.hovered == Some(index) {
                theme::hover_fill(layer, tab_rect, self.hover.value());
            }
            theme::icon(
                layer,
                icons::FILE,
                (x + 18.0, middle - 6.5),
                13.0,
                if active {
                    theme::accent()
                } else {
                    theme::comment()
                },
                1.7,
            );
            theme::draw(layer, &tab.name, (x + 39.0, middle), &style, theme::LEFT);

            // An unsaved tab carries a dot, next to its close button.
            if tab.unsaved {
                layer.draw_rectangle(
                    (x + width - 30.0, middle - 2.0),
                    (4.0, 4.0),
                    theme::warning(),
                    Rounding::NONE,
                );
            }
            theme::icon(
                layer,
                icons::X,
                (close_rect(tab_rect).x + 4.0, middle - 6.5),
                13.0,
                if self.hovered == Some(index) {
                    theme::ink()
                } else {
                    theme::faint()
                },
                1.8,
            );
            x += width + 5.0;
        }

        self.plus_rect = Rect::new(x, rect.y + 7.0, PLUS_WIDTH, rect.height - 14.0);
        theme::hover_fill(layer, self.plus_rect, self.plus_hover.value());
        theme::icon(
            layer,
            icons::PLUS,
            (x + 14.0, middle - 7.0),
            14.0,
            theme::faint(),
            1.7,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_close_button_lives_in_the_tab_and_hits_alone() {
        let tab = Rect::new(0.0, 0.0, 120.0, HEIGHT);
        let close = close_rect(tab);
        assert!(close.contains((tab.right() - 12.0, tab.y + tab.height / 2.0)));
        assert!(!close.contains((40.0, tab.y + tab.height / 2.0)));
    }
}
