//! The right-click context menu: a small command card beside the pointer.
//!
//! Same snapshot relationship to the shell as [`Palette`](super::Palette) and
//! [`SlashMenu`](super::SlashMenu): the shell owns live state and keystrokes,
//! while this component holds a snapshot and draws it.

use crate::layout::Rect;
use crate::renderer::{Layer, Rounding};
use crate::theme::{self, TextStyle};
use crate::ui::{Component, Context, Dirty};

/// One row: reuse the palette's `Entry` — title, group, and the keybinding
/// hint, which is exactly what an OS menu shows on the right of a row.
pub use super::palette::Entry;

const CARD_W: f32 = 224.0;
const ROW_HEIGHT: f32 = 30.0;
/// The card's own padding above the first row and below the last.
const PAD_Y: f32 = 6.0;
const TITLE_X: f32 = 34.0;

/// The card for `rows` items, with its top-left corner at `anchor`. It
/// flips up when it would overflow the viewport's bottom and shifts left
/// when it would overflow the right edge, so a menu opened near a corner
/// stays fully on screen — same rule as the slash menu's card.
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

/// The row `point` is over, if any. The component owns this so the
/// hit-test and the drawing cannot drift apart.
pub fn row_at(card: Rect, rows: usize, point: (f32, f32)) -> Option<usize> {
    if !card.contains(point) || point.1 < card.y + PAD_Y {
        return None;
    }
    let index = ((point.1 - card.y - PAD_Y) / ROW_HEIGHT) as usize;
    (index < rows).then_some(index)
}

pub struct ContextMenu {
    entries: Vec<Entry>,
    checked: Vec<bool>,
    /// Indexes `entries`. Also what a hovered row sets, so keyboard and
    /// mouse drive one highlight rather than two.
    selected: usize,
    anchor: (f32, f32),
    open: bool,
    dirty: Dirty,
}

impl ContextMenu {
    pub fn new(
        entries: Vec<Entry>,
        checked: Vec<bool>,
        selected: usize,
        anchor: (f32, f32),
    ) -> Self {
        let selected = selected.min(entries.len().saturating_sub(1));
        Self {
            entries,
            checked,
            selected,
            anchor,
            open: true,
            dirty: Dirty::new(),
        }
    }

    pub fn closed() -> Self {
        Self {
            entries: Vec::new(),
            checked: Vec::new(),
            selected: 0,
            anchor: (0.0, 0.0),
            open: false,
            dirty: Dirty::new(),
        }
    }
}

impl Component for ContextMenu {
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
        if !self.open {
            return;
        }

        // Unlike the modal palette, a context menu is local to the pointer;
        // dimming the document behind it would make a small action menu noisy.
        let card = card_anchored(rect, self.anchor, self.entries.len());
        layer.draw_rectangle(card.position(), card.size(), theme::PANEL, Rounding::NONE);
        theme::outline(layer, card, theme::BORDER);

        let title_style = TextStyle::serif(14.5, theme::INK);
        let hint_style = TextStyle::mono(10.0, theme::FAINT);
        for (index, entry) in self.entries.iter().enumerate() {
            let row = Rect::new(
                card.x,
                card.y + PAD_Y + index as f32 * ROW_HEIGHT,
                card.width,
                ROW_HEIGHT,
            );
            if index == self.selected {
                layer.draw_rectangle(row.position(), row.size(), theme::SELECTION, Rounding::NONE);
            }
            let middle = row.y + row.height / 2.0;
            if self.checked.get(index).copied().unwrap_or(false) {
                theme::icon(
                    layer,
                    theme::icons::CHECK,
                    (row.x + 10.0, middle - 7.0),
                    14.0,
                    theme::ACCENT,
                    1.8,
                );
            }
            theme::draw(
                layer,
                &entry.title,
                (row.x + TITLE_X, middle),
                &title_style,
                theme::LEFT,
            );
            if !entry.hint.is_empty() {
                theme::draw(
                    layer,
                    &entry.hint,
                    (row.right() - 14.0, middle),
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

    #[test]
    fn a_menu_near_the_bottom_flips_up() {
        let viewport = Rect::new(0.0, 0.0, 400.0, 300.0);
        let anchor = (100.0, 290.0);
        let card = card_anchored(viewport, anchor, 3);
        assert!(card.bottom() <= anchor.1);
        assert!(card.y >= viewport.y);
        assert!(card.bottom() <= viewport.bottom());
    }

    #[test]
    fn a_menu_near_the_right_edge_shifts_left() {
        let viewport = Rect::new(0.0, 0.0, 400.0, 300.0);
        let anchor = (390.0, 100.0);
        let card = card_anchored(viewport, anchor, 3);
        assert!(card.x < anchor.0);
        assert!(card.x >= viewport.x);
        assert!(card.right() <= viewport.right());
    }

    #[test]
    fn row_at_maps_a_point_to_the_row_drawn_there() {
        let card = card_anchored(Rect::new(0.0, 0.0, 500.0, 400.0), (40.0, 40.0), 3);
        assert_eq!(
            row_at(card, 3, (card.x + 20.0, card.y + PAD_Y + ROW_HEIGHT / 2.0)),
            Some(0)
        );
        assert_eq!(
            row_at(card, 3, (card.x + 20.0, card.y + PAD_Y + ROW_HEIGHT * 2.5)),
            Some(2)
        );
        assert_eq!(row_at(card, 3, (card.x + 20.0, card.y + PAD_Y / 2.0)), None);
        assert_eq!(row_at(card, 3, (card.right() + 1.0, card.y + 15.0)), None);
    }

    #[test]
    fn a_check_is_drawn_on_the_row_with_the_same_entry_index() {
        let entries = vec![
            Entry {
                title: "First".into(),
                group: "".into(),
                hint: "".into(),
            },
            Entry {
                title: "Second".into(),
                group: "".into(),
                hint: "".into(),
            },
        ];
        let menu = ContextMenu::new(entries, vec![false, true], 0, (0.0, 0.0));
        assert!(!menu.checked[0]);
        assert!(menu.checked[1]);
    }
}
