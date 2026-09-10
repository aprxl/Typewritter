//! Direct manipulation for a table's presentation and structure.
//!
//! The little grid is a real six-stroke selector rather than a translation
//! layer for `Top` and `Bottom` labels. The row and column controls beneath
//! it make changing the grid's shape just as local as changing its lines.

use crate::document::table::{GridLine, TableLines};
use crate::layout::Rect;
use crate::renderer::{Layer, Rounding};
use crate::theme::{self, TextStyle};
use crate::ui::{Component, Context, Dirty, Hover};

const CARD_WIDTH: f32 = 218.0;
/// Tall enough for the grid picker, the row and column controls, and the
/// destructive control beneath them.
const CARD_HEIGHT: f32 = 265.0;
const GRID: f32 = 96.0;
const HIT: f32 = 11.0;
const PAD: f32 = 16.0;
const BUTTON: f32 = 25.0;
const ROW_HEIGHT: f32 = 29.0;

/// A structural action selected from the lower half of the table card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableAction {
    InsertRow,
    RemoveRow,
    InsertColumn,
    RemoveColumn,
    /// The whole table, not a row or a column of it.
    DeleteTable,
}

/// Every direct target in the card. The shell consumes this instead of
/// recovering intent from a point after the component has already drawn it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableHit {
    Line(GridLine),
    Action(TableAction),
}

/// The resting popup rectangle. Shared by drawing and input so a control's
/// hit target always stays over its painted shape.
pub fn card_anchored(viewport: Rect, anchor: (f32, f32)) -> Rect {
    let width = CARD_WIDTH.min(viewport.width);
    let height = CARD_HEIGHT.min(viewport.height);
    let x = anchor
        .0
        .clamp(viewport.x, (viewport.right() - width).max(viewport.x));
    let y = if anchor.1 + height > viewport.bottom() {
        anchor.1 - height
    } else {
        anchor.1
    }
    .clamp(viewport.y, (viewport.bottom() - height).max(viewport.y));
    Rect::new(x, y, width, height)
}

fn grid(card: Rect) -> Rect {
    let size = GRID.min(card.width - PAD * 2.0).max(0.0);
    Rect::new(
        card.x + (card.width - size) * 0.5,
        card.y + 39.0,
        size,
        size,
    )
}

fn control_row(card: Rect, column: bool) -> Rect {
    Rect::new(
        card.x + PAD,
        grid(card).bottom() + 13.0 + if column { ROW_HEIGHT + 4.0 } else { 0.0 },
        (card.width - PAD * 2.0).max(0.0),
        ROW_HEIGHT,
    )
}

/// The destructive control's row: full width, under the row and column
/// controls, so it is the last thing the eye reaches in the card.
fn delete_row(card: Rect) -> Rect {
    Rect::new(
        card.x + PAD,
        control_row(card, true).bottom() + 10.0,
        (card.width - PAD * 2.0).max(0.0),
        ROW_HEIGHT,
    )
}

fn action_rect(card: Rect, action: TableAction) -> Rect {
    let row = control_row(
        card,
        matches!(
            action,
            TableAction::InsertColumn | TableAction::RemoveColumn
        ),
    );
    let x = match action {
        TableAction::InsertRow | TableAction::InsertColumn => row.right() - BUTTON,
        TableAction::RemoveRow | TableAction::RemoveColumn => row.right() - BUTTON * 2.0 - 4.0,
        // The destructive control is a full-width row of its own rather than a
        // button in the row and column pairs.
        TableAction::DeleteTable => return delete_row(card),
    };
    Rect::new(x, row.y + (row.height - BUTTON) * 0.5, BUTTON, BUTTON)
}

/// The semantic grid line hit by the pointer.
pub fn line_at(card: Rect, point: (f32, f32)) -> Option<GridLine> {
    if !card.contains(point) {
        return None;
    }
    let grid = grid(card);
    let mid_x = grid.x + grid.width * 0.5;
    let mid_y = grid.y + grid.height * 0.5;
    let near = |a: f32, b: f32| (a - b).abs() <= HIT;
    let across = point.0 >= grid.x - HIT && point.0 <= grid.right() + HIT;
    let down = point.1 >= grid.y - HIT && point.1 <= grid.bottom() + HIT;
    if across && near(point.1, grid.y) {
        Some(GridLine::Top)
    } else if across && near(point.1, grid.bottom()) {
        Some(GridLine::Bottom)
    } else if down && near(point.0, grid.x) {
        Some(GridLine::Left)
    } else if down && near(point.0, grid.right()) {
        Some(GridLine::Right)
    } else if across && near(point.1, mid_y) {
        Some(GridLine::Horizontal)
    } else if down && near(point.0, mid_x) {
        Some(GridLine::Vertical)
    } else {
        None
    }
}

/// The direct line or structural control under `point`.
pub fn hit_at(card: Rect, point: (f32, f32)) -> Option<TableHit> {
    line_at(card, point).map(TableHit::Line).or_else(|| {
        [
            TableAction::InsertRow,
            TableAction::RemoveRow,
            TableAction::InsertColumn,
            TableAction::RemoveColumn,
            TableAction::DeleteTable,
        ]
        .into_iter()
        .find(|action| action_rect(card, *action).contains(point))
        .map(TableHit::Action)
    })
}

/// Every keyboard-reachable target in the card, in the order Tab walks
/// them: the six grid strokes, the four structural buttons, then the
/// destructive one. Each target's geometry comes from the same
/// `line_at`/`action_rect` the mouse hit-test reads, so the two paths cannot
/// describe different controls.
pub const TARGETS: [TableHit; 11] = [
    TableHit::Line(GridLine::Top),
    TableHit::Line(GridLine::Bottom),
    TableHit::Line(GridLine::Left),
    TableHit::Line(GridLine::Right),
    TableHit::Line(GridLine::Horizontal),
    TableHit::Line(GridLine::Vertical),
    TableHit::Action(TableAction::InsertRow),
    TableHit::Action(TableAction::RemoveRow),
    TableHit::Action(TableAction::InsertColumn),
    TableHit::Action(TableAction::RemoveColumn),
    TableHit::Action(TableAction::DeleteTable),
];

/// The target after `current` in [`TARGETS`], wrapping at both ends. `None`
/// starts at the first target (forward) or the last (backward).
pub fn step_focus(current: Option<TableHit>, forward: bool) -> Option<TableHit> {
    let index = current.and_then(|hit| TARGETS.iter().position(|target| *target == hit));
    let next = match (index, forward) {
        (Some(index), true) => (index + 1) % TARGETS.len(),
        (Some(index), false) => (index + TARGETS.len() - 1) % TARGETS.len(),
        (None, true) => 0,
        (None, false) => TARGETS.len() - 1,
    };
    Some(TARGETS[next])
}

pub struct TableLinesMenu {
    lines: TableLines,
    anchor: (f32, f32),
    row: usize,
    column: usize,
    rows: usize,
    columns: usize,
    /// The keyboard's own cursor. It is shell-owned and seeded to `None`
    /// for every open, so a keyboard-raised card and a clicked one start in
    /// the same state; the pointer's `hover` stays independent of it.
    focus: Option<TableHit>,
    hover: Option<TableHit>,
    hover_fade: Hover,
    dirty: Dirty,
}

impl TableLinesMenu {
    pub fn new(
        lines: TableLines,
        anchor: (f32, f32),
        row: usize,
        column: usize,
        rows: usize,
        columns: usize,
    ) -> Self {
        Self {
            lines,
            anchor,
            row,
            column,
            rows,
            columns,
            focus: None,
            hover: None,
            hover_fade: Hover::new(),
            dirty: Dirty::new(),
        }
    }

    /// The keyboard cursor the shell feeds in on each refresh. Not a
    /// separate construction path: the card is built exactly as before and
    /// this only states which target is lit.
    pub fn with_focus(mut self, focus: Option<TableHit>) -> Self {
        self.focus = focus;
        self
    }

    pub fn closed() -> Self {
        Self::new(TableLines::default(), (0.0, 0.0), 0, 0, 0, 0)
    }

    /// The card's one destructive control. It takes the whole table, so it is
    /// drawn apart from the row and column pairs and wears the palette's
    /// warning role rather than the neutral button ink.
    fn draw_delete(&self, layer: &Layer, card: Rect) {
        let rect = delete_row(card);
        let hit = TableHit::Action(TableAction::DeleteTable);
        let hovered = self.hover == Some(hit);
        let focused = self.focus == Some(hit);
        if hovered {
            theme::hover_fill(layer, rect, self.hover_fade.value());
        }
        layer.draw_rectangle(
            rect.position(),
            rect.size(),
            theme::fade(theme::alt(), 0.72),
            Rounding::uniform(6.0),
        );
        theme::rounded_outline(
            layer,
            rect.inset(0.5),
            5.5,
            if focused { 1.5 } else { 1.0 },
            if focused {
                theme::accent()
            } else {
                theme::border()
            },
        );
        theme::draw(
            layer,
            "Delete table",
            (rect.x + rect.width * 0.5, rect.y + rect.height * 0.5),
            &TextStyle::sans(
                11.5,
                if hovered || focused {
                    theme::warning()
                } else {
                    theme::dim()
                },
            ),
            theme::CENTER,
        );
    }

    fn draw_control(&self, layer: &Layer, card: Rect, column: bool) {
        let row = control_row(card, column);
        let count = if column { self.columns } else { self.rows };
        let selected = if column { self.column } else { self.row };
        let name = if column { "COLUMNS" } else { "ROWS" };
        let label = format!("{name}  {} / {count}", selected.saturating_add(1));
        let label_style = TextStyle::sans(10.5, theme::dim()).tracked(0.08);
        theme::draw(
            layer,
            &label,
            (row.x, row.y + row.height * 0.5),
            &label_style,
            theme::LEFT,
        );
        let actions = if column {
            [TableAction::RemoveColumn, TableAction::InsertColumn]
        } else {
            [TableAction::RemoveRow, TableAction::InsertRow]
        };
        for action in actions {
            let rect = action_rect(card, action);
            let available =
                !matches!(action, TableAction::RemoveRow | TableAction::RemoveColumn) || count > 1;
            let hovered = available && self.hover == Some(TableHit::Action(action));
            if hovered {
                theme::hover_fill(layer, rect, self.hover_fade.value());
            }
            let focused = self.focus == Some(TableHit::Action(action));
            layer.draw_rectangle(
                rect.position(),
                rect.size(),
                theme::fade(theme::alt(), 0.72),
                Rounding::uniform(6.0),
            );
            theme::rounded_outline(
                layer,
                rect.inset(0.5),
                5.5,
                if focused { 1.5 } else { 1.0 },
                if focused {
                    theme::accent()
                } else {
                    theme::border()
                },
            );
            let glyph = match action {
                TableAction::InsertRow | TableAction::InsertColumn => "+",
                TableAction::RemoveRow | TableAction::RemoveColumn => "−",
                // Never one of this pair: [`Self::draw_delete`] paints it.
                TableAction::DeleteTable => continue,
            };
            theme::draw(
                layer,
                glyph,
                (rect.x + rect.width * 0.5, rect.y + rect.height * 0.5),
                &TextStyle::sans(
                    16.0,
                    if !available {
                        theme::faint()
                    } else if hovered || focused {
                        theme::accent()
                    } else {
                        theme::ink()
                    },
                ),
                theme::CENTER,
            );
        }
    }
}

impl Component for TableLinesMenu {
    fn measure(&mut self, _: &Layer) -> (f32, f32) {
        (0.0, 0.0)
    }

    fn sync(&mut self, context: &Context) {
        let hover = context
            .mouse
            .in_window
            .then(|| {
                hit_at(
                    card_anchored(context.self_rect, self.anchor),
                    context.mouse.position,
                )
            })
            .flatten();
        if self
            .hover_fade
            .track(&mut self.hover, hover, context.animation_dt)
        {
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
        self.hover_fade.is_animating()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn draw(&mut self, layer: &Layer, rect: Rect) {
        let card = card_anchored(rect, self.anchor);
        layer.draw_rectangle(
            card.position(),
            card.size(),
            theme::elevated_popup(1.0),
            Rounding::uniform(super::popup::CARD_RADIUS),
        );
        theme::rounded_outline(
            layer,
            card.inset(0.5),
            super::popup::CARD_RADIUS - 0.5,
            1.0,
            theme::non_text(),
        );
        theme::draw(
            layer,
            "TABLE",
            (card.x + PAD, card.y + 21.0),
            &TextStyle::sans(10.5, theme::faint()).tracked(0.12),
            theme::LEFT,
        );
        theme::draw(
            layer,
            "grid",
            (card.right() - PAD, card.y + 21.0),
            &TextStyle::sans(11.0, theme::dim()).italic(),
            theme::RIGHT,
        );

        let grid = grid(card);
        let mid_x = grid.x + grid.width * 0.5;
        let mid_y = grid.y + grid.height * 0.5;
        let lit = |hit: TableHit| self.hover == Some(hit) || self.focus == Some(hit);
        let stroke = |line: GridLine| {
            if self.lines.enabled(line) {
                theme::accent()
            } else if lit(TableHit::Line(line)) {
                theme::fade(theme::accent(), 0.7)
            } else {
                theme::faint()
            }
        };
        let thickness = |line: GridLine| {
            if self.lines.enabled(line) || lit(TableHit::Line(line)) {
                2.5
            } else {
                1.0
            }
        };
        theme::rule(
            layer,
            (grid.x, grid.y),
            grid.width,
            thickness(GridLine::Top),
            stroke(GridLine::Top),
        );
        theme::rule(
            layer,
            (grid.x, grid.bottom()),
            grid.width,
            thickness(GridLine::Bottom),
            stroke(GridLine::Bottom),
        );
        theme::vertical_rule(
            layer,
            (grid.x, grid.y),
            grid.height,
            thickness(GridLine::Left),
            stroke(GridLine::Left),
        );
        theme::vertical_rule(
            layer,
            (grid.right(), grid.y),
            grid.height,
            thickness(GridLine::Right),
            stroke(GridLine::Right),
        );
        theme::rule(
            layer,
            (grid.x, mid_y),
            grid.width,
            thickness(GridLine::Horizontal),
            stroke(GridLine::Horizontal),
        );
        theme::vertical_rule(
            layer,
            (mid_x, grid.y),
            grid.height,
            thickness(GridLine::Vertical),
            stroke(GridLine::Vertical),
        );
        theme::rule(
            layer,
            (card.x + PAD, grid.bottom() + 6.0),
            card.width - PAD * 2.0,
            1.0,
            theme::border(),
        );
        self.draw_control(layer, card, false);
        self.draw_control(layer, card, true);
        self.draw_delete(layer, card);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_painted_stroke_has_a_direct_hit() {
        let card = card_anchored(Rect::new(0.0, 0.0, 400.0, 400.0), (80.0, 80.0));
        let grid = grid(card);
        assert_eq!(line_at(card, (grid.x + 30.0, grid.y)), Some(GridLine::Top));
        assert_eq!(
            line_at(card, (grid.x + 30.0, grid.bottom())),
            Some(GridLine::Bottom)
        );
        assert_eq!(line_at(card, (grid.x, grid.y + 30.0)), Some(GridLine::Left));
        assert_eq!(
            line_at(card, (grid.right(), grid.y + 30.0)),
            Some(GridLine::Right)
        );
        assert_eq!(
            line_at(card, (grid.x + 20.0, grid.y + grid.height * 0.5)),
            Some(GridLine::Horizontal)
        );
        assert_eq!(
            line_at(card, (grid.x + grid.width * 0.5, grid.y + 20.0)),
            Some(GridLine::Vertical)
        );
    }

    #[test]
    fn tab_walks_every_target_once_and_wraps_at_both_ends() {
        let mut focus = None;
        let mut seen = Vec::new();
        for _ in 0..TARGETS.len() {
            focus = step_focus(focus, true);
            seen.push(focus.unwrap());
        }
        assert_eq!(seen, TARGETS.to_vec(), "forward walks the whole card");
        assert_eq!(
            step_focus(focus, true),
            Some(TARGETS[0]),
            "wraps at the end"
        );
        assert_eq!(
            step_focus(None, false),
            Some(TARGETS[TARGETS.len() - 1]),
            "backward starts at the end"
        );
        assert_eq!(
            step_focus(Some(TARGETS[0]), false),
            Some(TARGETS[TARGETS.len() - 1]),
            "backward wraps at the start"
        );
    }

    /// A point that hit-tests as `line` on the painted grid.
    fn point_on_line(card: Rect, line: GridLine) -> (f32, f32) {
        let grid = grid(card);
        match line {
            GridLine::Top => (grid.x + 30.0, grid.y),
            GridLine::Bottom => (grid.x + 30.0, grid.bottom()),
            GridLine::Left => (grid.x, grid.y + 30.0),
            GridLine::Right => (grid.right(), grid.y + 30.0),
            GridLine::Horizontal => (grid.x + 20.0, grid.y + grid.height * 0.5),
            GridLine::Vertical => (grid.x + grid.width * 0.5, grid.y + 20.0),
        }
    }

    #[test]
    fn every_keyboard_target_is_the_mouse_target_at_its_own_geometry() {
        let card = card_anchored(Rect::new(0.0, 0.0, 400.0, 400.0), (80.0, 80.0));
        for target in TARGETS {
            let hit = match target {
                TableHit::Line(line) => {
                    line_at(card, point_on_line(card, line)).map(TableHit::Line)
                }
                TableHit::Action(action) => {
                    let rect = action_rect(card, action);
                    hit_at(card, (rect.x + 2.0, rect.y + 2.0))
                }
            };
            assert_eq!(hit, Some(target), "{target:?} has its own hit test");
        }
    }

    /// The destructive control is the last thing in the card, reachable by
    /// mouse and by Tab, and the card is tall enough to hold it — the height
    /// constant is part of the geometry, so it is asserted rather than eyeballed.
    #[test]
    fn the_delete_row_lies_under_the_controls_inside_the_card() {
        let card = card_anchored(Rect::new(0.0, 0.0, 400.0, 400.0), (80.0, 80.0));
        let rect = delete_row(card);
        assert_eq!(
            hit_at(
                card,
                (rect.x + rect.width * 0.5, rect.y + rect.height * 0.5)
            ),
            Some(TableHit::Action(TableAction::DeleteTable))
        );
        for action in [
            TableAction::InsertRow,
            TableAction::RemoveRow,
            TableAction::InsertColumn,
            TableAction::RemoveColumn,
        ] {
            let button = action_rect(card, action);
            assert!(
                button.bottom() <= rect.y,
                "{action:?} sits above the destructive row"
            );
        }
        assert!(
            rect.bottom() + PAD <= card.bottom(),
            "the card is tall enough for every control it draws"
        );
    }

    #[test]
    fn row_and_column_buttons_have_direct_hits() {
        let card = card_anchored(Rect::new(0.0, 0.0, 400.0, 400.0), (80.0, 80.0));
        for action in [
            TableAction::InsertRow,
            TableAction::RemoveRow,
            TableAction::InsertColumn,
            TableAction::RemoveColumn,
        ] {
            let rect = action_rect(card, action);
            assert_eq!(
                hit_at(card, (rect.x + 2.0, rect.y + 2.0)),
                Some(TableHit::Action(action))
            );
        }
    }
}
