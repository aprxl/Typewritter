//! The in-math completion card: ordinary interpretations followed by a
//! compact grid of notation variants for an exact known symbol.
//!
//! The shell owns the query, selection, and input. This component owns all
//! menu geometry so drawing and mouse hit-testing use the same rectangles.

use crate::layout::Rect;
use crate::renderer::{Layer, Rounding};
use crate::theme::{self, TextStyle};
use crate::ui::{Component, Context, Dirty};

const CARD_W: f32 = 320.0;
const ROW_HEIGHT: f32 = 32.0;
const PAD_Y: f32 = 8.0;
const GRID_PAD_X: f32 = 8.0;
const GRID_HEADER_HEIGHT: f32 = 22.0;
const GRID_CELL_HEIGHT: f32 = 34.0;
const GRID_GAP: f32 = 5.0;
const GRID_COLUMNS: usize = 5;

/// One offered completion as the card shows it.
pub struct Row {
    pub name: String,
    pub group: String,
    pub preview: String,
}

fn split(items: usize, variant_start: usize) -> (usize, usize) {
    let regular = variant_start.min(items);
    (regular, items - regular)
}

fn grid_rows(variants: usize) -> usize {
    variants.div_ceil(GRID_COLUMNS)
}

fn card_height(items: usize, variant_start: usize) -> f32 {
    let (regular, variants) = split(items, variant_start);
    let grid = grid_rows(variants);
    PAD_Y * 2.0
        + regular as f32 * ROW_HEIGHT
        + if grid == 0 {
            0.0
        } else {
            GRID_HEADER_HEIGHT
                + grid as f32 * GRID_CELL_HEIGHT
                + grid.saturating_sub(1) as f32 * GRID_GAP
        }
}

/// The card for `items`, where `variant_start..items` is drawn as a grid.
pub fn card_anchored(
    viewport: Rect,
    anchor: (f32, f32),
    items: usize,
    variant_start: usize,
) -> Rect {
    let height = card_height(items, variant_start);
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

/// The exact rectangle used to draw a flat item index.
pub fn item_rect(card: Rect, items: usize, variant_start: usize, index: usize) -> Option<Rect> {
    if index >= items {
        return None;
    }
    let (regular, _) = split(items, variant_start);
    if index < regular {
        return Some(Rect::new(
            card.x,
            card.y + PAD_Y + index as f32 * ROW_HEIGHT,
            card.width,
            ROW_HEIGHT,
        ));
    }

    let variant = index - regular;
    let column = variant % GRID_COLUMNS;
    let row = variant / GRID_COLUMNS;
    let width =
        (card.width - GRID_PAD_X * 2.0 - GRID_GAP * (GRID_COLUMNS.saturating_sub(1)) as f32)
            / GRID_COLUMNS as f32;
    Some(Rect::new(
        card.x + GRID_PAD_X + column as f32 * (width + GRID_GAP),
        card.y
            + PAD_Y
            + regular as f32 * ROW_HEIGHT
            + GRID_HEADER_HEIGHT
            + row as f32 * (GRID_CELL_HEIGHT + GRID_GAP),
        width,
        GRID_CELL_HEIGHT,
    ))
}

/// Flat item index under `point`, sharing [`item_rect`] with drawing.
pub fn item_at(card: Rect, items: usize, variant_start: usize, point: (f32, f32)) -> Option<usize> {
    (0..items).find(|&index| {
        item_rect(card, items, variant_start, index).is_some_and(|rect| rect.contains(point))
    })
}

pub struct MathMenu {
    rows: Vec<Row>,
    variant_start: usize,
    selected: usize,
    anchor: (f32, f32),
    open: bool,
    dirty: Dirty,
}

impl MathMenu {
    pub fn new(rows: Vec<Row>, variant_start: usize, selected: usize, anchor: (f32, f32)) -> Self {
        let selected = selected.min(rows.len().saturating_sub(1));
        Self {
            variant_start: variant_start.min(rows.len()),
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
            variant_start: 0,
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

    fn draw(&mut self, layer: &Layer, viewport: Rect) {
        if !self.open || self.rows.is_empty() {
            return;
        }

        let card = card_anchored(viewport, self.anchor, self.rows.len(), self.variant_start);
        layer.draw_rectangle(card.position(), card.size(), theme::PANEL, Rounding::NONE);
        theme::outline(layer, card, theme::BORDER);

        let name_style = TextStyle::serif(15.0, theme::INK);
        let group_style = TextStyle::serif(11.5, theme::COMMENT);
        let preview_style = TextStyle::math(17.0, theme::INK);
        for (index, row) in self.rows[..self.variant_start].iter().enumerate() {
            let rect = item_rect(card, self.rows.len(), self.variant_start, index)
                .expect("a menu row must have geometry");
            if index == self.selected {
                layer.draw_rectangle(
                    rect.position(),
                    rect.size(),
                    theme::SELECTION,
                    Rounding::NONE,
                );
            }
            let middle = rect.y + rect.height / 2.0;
            theme::draw(
                layer,
                &row.name,
                (rect.x + 20.0, middle),
                &name_style,
                theme::LEFT,
            );
            let group_x = rect.x + 20.0 + theme::width(layer, &row.name, &name_style) + 8.0;
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
                    (rect.right() - 20.0, middle),
                    &preview_style,
                    theme::RIGHT,
                );
            }
        }

        if self.variant_start < self.rows.len() {
            let header_y =
                card.y + PAD_Y + self.variant_start as f32 * ROW_HEIGHT + GRID_HEADER_HEIGHT / 2.0;
            theme::draw(
                layer,
                "VARIANTS",
                (card.x + GRID_PAD_X, header_y),
                &group_style,
                theme::LEFT,
            );
        }
        for (index, row) in self.rows[self.variant_start..].iter().enumerate() {
            let flat_index = self.variant_start + index;
            let rect = item_rect(card, self.rows.len(), self.variant_start, flat_index)
                .expect("a variant must have geometry");
            let color = if flat_index == self.selected {
                theme::SELECTION
            } else {
                theme::ALT
            };
            layer.draw_rectangle(rect.position(), rect.size(), color, Rounding::uniform(5.0));
            theme::draw(
                layer,
                &row.preview,
                (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0),
                &preview_style,
                theme::CENTER,
            );
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
                preview: format!("p{index}"),
            })
            .collect()
    }

    #[test]
    fn anchor_sizes_for_rows_and_variant_grid() {
        let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
        let rows_only = card_anchored(viewport, (100.0, 200.0), 3, 3);
        assert_eq!(rows_only.height, PAD_Y * 2.0 + 3.0 * ROW_HEIGHT);

        let mixed = card_anchored(viewport, (100.0, 200.0), 8, 2);
        assert_eq!(
            mixed.height,
            PAD_Y * 2.0 + 2.0 * ROW_HEIGHT + GRID_HEADER_HEIGHT + 2.0 * GRID_CELL_HEIGHT + GRID_GAP
        );
    }

    #[test]
    fn anchor_flips_and_clamps_at_viewport_edges() {
        let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
        let r = card_anchored(viewport, (700.0, 500.0), 8, 2);
        assert_eq!(r.x, 800.0 - CARD_W);
        assert_eq!(r.bottom(), 500.0);
    }

    #[test]
    fn hit_test_maps_rows_and_cells_to_one_flat_selection() {
        let card = card_anchored(Rect::new(0.0, 0.0, 800.0, 600.0), (100.0, 100.0), 8, 2);
        for index in 0..8 {
            let rect = item_rect(card, 8, 2, index).unwrap();
            assert_eq!(
                item_at(
                    card,
                    8,
                    2,
                    (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0)
                ),
                Some(index)
            );
        }
        let gap = item_rect(card, 8, 2, 2).unwrap().right() + GRID_GAP / 2.0;
        assert_eq!(
            item_at(card, 8, 2, (gap, item_rect(card, 8, 2, 2).unwrap().y)),
            None
        );
    }

    #[test]
    fn new_clamps_selection_and_variant_boundary() {
        let menu = MathMenu::new(rows(3), 99, 99, (0.0, 0.0));
        assert_eq!(menu.selected, 2);
        assert_eq!(menu.variant_start, 3);

        let menu = MathMenu::new(Vec::new(), 9, 9, (0.0, 0.0));
        assert_eq!(menu.selected, 0);
        assert_eq!(menu.variant_start, 0);
    }
}
