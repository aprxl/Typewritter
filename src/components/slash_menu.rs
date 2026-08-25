//! The slash command menu: a compact inline popup next to the text caret.
//!
//! Same relationship to the shell as [`Palette`](super::Palette) — the shell
//! owns the keystrokes and the query, this component holds a snapshot and
//! draws it.

use crate::layout::Rect;
use crate::renderer::{Layer, Rounding};
use crate::theme::{self, TextStyle};
use crate::ui::{Component, Context, Dirty};

use super::palette::{self, Entry};

const CARD_W: f32 = 260.0;
const QUERY_H: f32 = 56.0;
const ROW_HEIGHT: f32 = 34.0;
/// Derived from `QUERY_H` + 6 rows, so a constant and its derivation live
/// next to each other rather than drifting apart.
const CARD_H: f32 = QUERY_H + 6.0 * ROW_HEIGHT;
const MAX_ROWS: usize = ((CARD_H - QUERY_H) / ROW_HEIGHT) as usize;

/// The card's top-left corner sits at `anchor` by default. If it would
/// overflow the viewport bottom, the card flips upward (bottom-left corner
/// at anchor); if it would overflow the right edge, it shifts left.
pub fn card_anchored(viewport: Rect, anchor: (f32, f32)) -> Rect {
    let mut x = anchor.0;
    let mut y = anchor.1;

    if y + CARD_H > viewport.bottom() {
        y = anchor.1 - CARD_H;
    }

    if x + CARD_W > viewport.right() {
        x = viewport.right() - CARD_W;
    }

    Rect::new(x, y, CARD_W, CARD_H)
}

pub struct SlashMenu {
    entries: Vec<Entry>,
    visible: Vec<usize>,
    query: String,
    selected: usize,
    first_visible: usize,
    anchor: (f32, f32),
    caret_on: bool,
    open: bool,
    dirty: Dirty,
}

impl SlashMenu {
    pub fn new(entries: Vec<Entry>, query: String, selected: usize, anchor: (f32, f32)) -> Self {
        let visible = palette::filter(&entries, &query);
        let selected = if visible.is_empty() {
            0
        } else {
            selected.min(visible.len() - 1)
        };
        let first_visible = selected.saturating_sub(MAX_ROWS.saturating_sub(1));
        Self {
            entries,
            visible,
            query,
            selected,
            first_visible,
            anchor,
            caret_on: true,
            open: true,
            dirty: Dirty::new(),
        }
    }

    pub fn closed() -> Self {
        Self {
            entries: Vec::new(),
            visible: Vec::new(),
            query: String::new(),
            selected: 0,
            first_visible: 0,
            anchor: (0.0, 0.0),
            caret_on: true,
            open: false,
            dirty: Dirty::new(),
        }
    }
}

impl Component for SlashMenu {
    fn measure(&mut self, _: &Layer) -> (f32, f32) {
        (0.0, 0.0)
    }

    fn sync(&mut self, context: &Context) {
        if self.open {
            self.dirty.write(&mut self.caret_on, context.caret_on);
        }
    }

    fn is_dirty(&self) -> bool {
        self.dirty.get()
    }

    fn clear_dirty(&mut self) {
        self.dirty.clear();
    }

    fn draw(&mut self, layer: &Layer, rect: Rect) {
        if !self.open {
            return;
        }

        let card = card_anchored(rect, self.anchor);
        layer.draw_rectangle(card.position(), card.size(), theme::popup(), Rounding::NONE);
        theme::outline(layer, card, theme::border());

        self.draw_query(layer, card);
        theme::rule(
            layer,
            (card.x + 20.0, card.y + QUERY_H),
            card.width - 40.0,
            1.0,
            theme::border(),
        );

        if self.visible.is_empty() {
            theme::draw(
                layer,
                "No matching command",
                (card.x + card.width / 2.0, card.y + QUERY_H + 32.0),
                &TextStyle::serif(13.5, theme::faint()),
                theme::CENTER,
            );
        } else {
            self.draw_rows(layer, card);
        }
    }
}

impl SlashMenu {
    fn draw_query(&self, layer: &Layer, card: Rect) {
        let style = TextStyle::serif(16.0, theme::ink());
        let middle = card.y + QUERY_H / 2.0;
        if self.query.is_empty() {
            theme::draw(
                layer,
                "Type a command",
                (card.x + 20.0, middle),
                &style.clone().color(theme::faint()),
                theme::LEFT,
            );
        } else {
            theme::draw(
                layer,
                &self.query,
                (card.x + 20.0, middle),
                &style,
                theme::LEFT,
            );
        }

        if self.caret_on {
            let x = card.x + 20.0 + theme::width(layer, &self.query, &style);
            layer.draw_rectangle(
                (x, middle - 10.0),
                (2.0, 20.0),
                theme::accent(),
                Rounding::NONE,
            );
        }
    }

    fn draw_rows(&self, layer: &Layer, card: Rect) {
        let title_style = TextStyle::serif(15.0, theme::ink());
        let group_style = TextStyle::serif(11.5, theme::comment());
        let hint_style = TextStyle::mono(10.5, theme::faint());

        let window = self.visible[self.first_visible..]
            .iter()
            .take(MAX_ROWS)
            .enumerate();
        for (offset, &entry_index) in window {
            let entry = &self.entries[entry_index];
            let row = Rect::new(
                card.x,
                card.y + QUERY_H + offset as f32 * ROW_HEIGHT,
                card.width,
                ROW_HEIGHT,
            );
            let middle = row.y + row.height / 2.0;

            if self.first_visible + offset == self.selected {
                layer.draw_rectangle(row.position(), row.size(), theme::selection(), Rounding::NONE);
            }

            theme::draw(
                layer,
                &entry.title,
                (row.x + 20.0, middle),
                &title_style,
                theme::LEFT,
            );
            let group_x = row.x + 20.0 + theme::width(layer, &entry.title, &title_style) + 8.0;
            theme::draw(
                layer,
                &entry.group,
                (group_x, middle),
                &group_style,
                theme::LEFT,
            );

            if !entry.hint.is_empty() {
                theme::draw(
                    layer,
                    &entry.hint,
                    (row.right() - 20.0, middle),
                    &hint_style,
                    theme::RIGHT,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(title: &str, group: &str) -> Entry {
        Entry {
            title: title.into(),
            group: group.into(),
            hint: String::new(),
        }
    }

    #[test]
    fn anchor_opens_downward_by_default() {
        let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
        let r = card_anchored(viewport, (100.0, 200.0));
        assert_eq!(r.x, 100.0);
        assert_eq!(r.y, 200.0);
    }

    #[test]
    fn anchor_flips_upward_near_bottom() {
        let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
        let r = card_anchored(viewport, (100.0, 400.0));
        assert_eq!(r.x, 100.0);
        assert_eq!(r.y, 400.0 - CARD_H);
        // Bottom edge should be at the anchor point.
        assert_eq!(r.bottom(), 400.0);
    }

    #[test]
    fn anchor_clamps_near_right_edge() {
        let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
        let r = card_anchored(viewport, (700.0, 200.0));
        assert_eq!(r.x, 800.0 - CARD_W);
        assert_eq!(r.y, 200.0);
    }

    #[test]
    fn anchor_flips_and_clamps_near_bottom_right_corner() {
        let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
        let r = card_anchored(viewport, (750.0, 500.0));
        assert_eq!(r.x, 800.0 - CARD_W);
        assert_eq!(r.y, 500.0 - CARD_H);
    }

    #[test]
    fn closed_has_open_false() {
        let m = SlashMenu::closed();
        assert!(!m.open);
    }

    #[test]
    fn new_clamps_selected_into_bounds() {
        let entries = vec![entry("Bold", "Format"), entry("Italic", "Format")];
        let m = SlashMenu::new(entries, "".into(), 99, (0.0, 0.0));
        assert_eq!(m.selected, 1);
    }

    #[test]
    fn new_clamps_selected_to_zero_when_empty() {
        let m = SlashMenu::new(vec![], "x".into(), 0, (0.0, 0.0));
        assert_eq!(m.selected, 0);
        assert!(m.visible.is_empty());
    }
}
