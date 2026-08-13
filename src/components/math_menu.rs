//! The in-math completion card: what the word being typed inside an
//! expression can become.
//!
//! Unlike the slash menu it owns no query — the query is the word in the
//! document, read from the tree — and it has no open state of its own: the
//! shell shows it whenever the word under the math cursor has completions,
//! so a backspace, an arrow move and a click elsewhere need no handling.
//! Same snapshot relationship to the shell as [`SlashMenu`](super::SlashMenu):
//! the shell owns the selected row and the keystrokes, this component holds
//! a snapshot and draws it.

use crate::layout::Rect;
use crate::renderer::{Layer, Rounding};
use crate::theme::{self, TextStyle};
use crate::ui::{Component, Context, Dirty};

const CARD_W: f32 = 320.0;
const ROW_HEIGHT: f32 = 32.0;
/// The card's own padding above the first row and below the last.
const PAD_Y: f32 = 8.0;

/// One offered completion as the card shows it.
pub struct Row {
    /// What you type to reach it: "alpha", "sqrt".
    pub name: String,
    /// "Greek", "Structure" — drawn dimmed after the name.
    pub group: String,
    /// What it produces: "α", "√" — drawn right-aligned in the math font.
    pub preview: String,
}

/// The card for `rows` rows, with its top-left corner at `anchor`. It
/// flips up when it would overflow the viewport's bottom and shifts left
/// when it would overflow the right edge — same rule as the context menu's
/// card.
pub fn card_anchored(viewport: Rect, anchor: (f32, f32), rows: usize) -> Rect {
    let height = PAD_Y * 2.0 + rows as f32 * ROW_HEIGHT;
    let x = if anchor.0 + CARD_W > viewport.right() {
        viewport.right() - CARD_W
    } else {
        anchor.0
    }
    .clamp(viewport.x, viewport.right() - CARD_W);
    let y = if anchor.1 + height > viewport.bottom() {
        anchor.1 - height
    } else {
        anchor.1
    }
    .clamp(viewport.y, viewport.bottom() - height);
    Rect::new(x, y, CARD_W, height)
}

pub struct MathMenu {
    rows: Vec<Row>,
    selected: usize,
    anchor: (f32, f32),
    open: bool,
    dirty: Dirty,
}

impl MathMenu {
    pub fn new(rows: Vec<Row>, selected: usize, anchor: (f32, f32)) -> Self {
        let selected = selected.min(rows.len().saturating_sub(1));
        Self {
            rows,
            selected,
            anchor,
            open: true,
            dirty: Dirty::new(),
        }
    }

    pub fn closed() -> Self {
        Self {
            rows: Vec::new(),
            selected: 0,
            anchor: (0.0, 0.0),
            open: false,
            dirty: Dirty::new(),
        }
    }
}

impl Component for MathMenu {
    fn measure(&mut self, _: &Layer) -> (f32, f32) {
        (0.0, 0.0)
    }

    fn sync(&mut self, _: &Context) {}

    fn is_dirty(&self) -> bool {
        self.dirty.get()
    }

    fn clear_dirty(&mut self) {
        self.dirty.clear();
    }

    fn draw(&mut self, layer: &Layer, rect: Rect) {
        if !self.open || self.rows.is_empty() {
            return;
        }

        let card = card_anchored(rect, self.anchor, self.rows.len());
        layer.draw_rectangle(card.position(), card.size(), theme::PANEL, Rounding::NONE);
        theme::outline(layer, card, theme::BORDER);

        let name_style = TextStyle::serif(15.0, theme::INK);
        let group_style = TextStyle::serif(11.5, theme::COMMENT);
        let preview_style = TextStyle::math(17.0, theme::INK);
        for (index, row) in self.rows.iter().enumerate() {
            let top = card.y + PAD_Y + index as f32 * ROW_HEIGHT;
            let middle = top + ROW_HEIGHT / 2.0;

            if index == self.selected {
                layer.draw_rectangle(
                    (card.x, top),
                    (card.width, ROW_HEIGHT),
                    theme::SELECTION,
                    Rounding::NONE,
                );
            }

            theme::draw(
                layer,
                &row.name,
                (card.x + 20.0, middle),
                &name_style,
                theme::LEFT,
            );
            let group_x = card.x + 20.0 + theme::width(layer, &row.name, &name_style) + 8.0;
            theme::draw(
                layer,
                &row.group,
                (group_x, middle),
                &group_style,
                theme::LEFT,
            );

            if !row.preview.is_empty() {
                theme::draw(
                    layer,
                    &row.preview,
                    (card.right() - 20.0, middle),
                    &preview_style,
                    theme::RIGHT,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(count: usize) -> Vec<Row> {
        (0..count)
            .map(|index| Row {
                name: format!("name{index}"),
                group: "Group".into(),
                preview: String::new(),
            })
            .collect()
    }

    #[test]
    fn anchor_opens_downward_by_default() {
        let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
        let r = card_anchored(viewport, (100.0, 200.0), 3);
        assert_eq!(r.x, 100.0);
        assert_eq!(r.y, 200.0);
        assert_eq!(r.height, PAD_Y * 2.0 + 3.0 * ROW_HEIGHT);
    }

    #[test]
    fn anchor_flips_upward_near_bottom() {
        let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
        let r = card_anchored(viewport, (100.0, 500.0), 3);
        assert_eq!(r.bottom(), 500.0);
    }

    #[test]
    fn anchor_clamps_near_right_edge() {
        let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
        let r = card_anchored(viewport, (700.0, 200.0), 3);
        assert_eq!(r.x, 800.0 - CARD_W);
    }

    #[test]
    fn closed_has_open_false() {
        let menu = MathMenu::closed();
        assert!(!menu.open);
        assert!(menu.rows.is_empty());
    }

    #[test]
    fn new_clamps_selected_into_bounds() {
        let menu = MathMenu::new(rows(3), 99, (0.0, 0.0));
        assert_eq!(menu.selected, 2);

        let menu = MathMenu::new(Vec::new(), 0, (0.0, 0.0));
        assert_eq!(menu.selected, 0);
    }
}
