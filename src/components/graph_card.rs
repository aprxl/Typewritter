//! The graph card: a graph widget's properties, edited in place. The
//! expression is typed with the same structural math input as a note, the
//! ranges as plain numbers; the widget redraws as they change.
//!
//! Like the table card, every control's geometry is a function of the card
//! rectangle, shared by drawing, pointer hits and the keyboard's Tab order.

use crate::document::math::{MathCursor, MathList};
use crate::document::{math_layout, math_paint};
use crate::layout::Rect;
use crate::renderer::{Layer, Rounding};
use crate::theme::{self, TextStyle};
use crate::ui::{Component, Context, Dirty, Hover};

const CARD_WIDTH: f32 = 300.0;
const CARD_HEIGHT: f32 = 286.0;
const PAD: f32 = 16.0;
const EXPRESSION_TOP: f32 = 38.0;
const EXPRESSION_HEIGHT: f32 = 48.0;
const ROW_HEIGHT: f32 = 29.0;
const ROW_GAP: f32 = 6.0;
/// Where the first property row starts, under the expression's status line.
const ROWS_TOP: f32 = EXPRESSION_TOP + EXPRESSION_HEIGHT + 34.0;
const NUMBER_WIDTH: f32 = 70.0;
const SWITCH: (f32, f32) = (32.0, 18.0);
const SEGMENT_WIDTH: f32 = 56.0;
const LABEL: f32 = 10.5;

/// Every control in the card, in Tab order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphTarget {
    Expression,
    XMin,
    XMax,
    YMin,
    YMax,
    Grid,
    Small,
    Large,
}

pub const TARGETS: [GraphTarget; 8] = [
    GraphTarget::Expression,
    GraphTarget::XMin,
    GraphTarget::XMax,
    GraphTarget::YMin,
    GraphTarget::YMax,
    GraphTarget::Grid,
    GraphTarget::Small,
    GraphTarget::Large,
];

impl GraphTarget {
    /// Which of the four range fields this is, in `XMin, XMax, YMin, YMax`
    /// order.
    pub fn number(self) -> Option<usize> {
        match self {
            Self::XMin => Some(0),
            Self::XMax => Some(1),
            Self::YMin => Some(2),
            Self::YMax => Some(3),
            _ => None,
        }
    }

    /// Whether typing goes into this control.
    pub fn takes_text(self) -> bool {
        self == Self::Expression || self.number().is_some()
    }
}

/// The target after `current` in [`TARGETS`], wrapping at both ends.
pub fn step_focus(current: GraphTarget, forward: bool) -> GraphTarget {
    let index = TARGETS
        .iter()
        .position(|target| *target == current)
        .unwrap_or(0);
    let next = if forward {
        (index + 1) % TARGETS.len()
    } else {
        (index + TARGETS.len() - 1) % TARGETS.len()
    };
    TARGETS[next]
}

/// The resting card rectangle, below `anchor` when it fits and above it
/// otherwise.
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

fn inner_width(card: Rect) -> f32 {
    (card.width - PAD * 2.0).max(0.0)
}

pub fn expression_rect(card: Rect) -> Rect {
    Rect::new(
        card.x + PAD,
        card.y + EXPRESSION_TOP,
        inner_width(card),
        EXPRESSION_HEIGHT,
    )
}

fn status_rect(card: Rect) -> Rect {
    let field = expression_rect(card);
    Rect::new(field.x, field.bottom() + 6.0, field.width, 16.0)
}

/// Property row `index`: 0 x range, 1 y range, 2 grid, 3 size.
fn row(card: Rect, index: usize) -> Rect {
    Rect::new(
        card.x + PAD,
        card.y + ROWS_TOP + index as f32 * (ROW_HEIGHT + ROW_GAP),
        inner_width(card),
        ROW_HEIGHT,
    )
}

fn target_rect(card: Rect, target: GraphTarget) -> Rect {
    match target {
        GraphTarget::Expression => expression_rect(card),
        GraphTarget::XMin | GraphTarget::XMax | GraphTarget::YMin | GraphTarget::YMax => {
            let index = target.number().expect("a range field");
            let row = row(card, index / 2);
            let max_x = row.right() - NUMBER_WIDTH;
            let x = if index % 2 == 1 {
                max_x
            } else {
                max_x - 28.0 - NUMBER_WIDTH
            };
            Rect::new(x, row.y, NUMBER_WIDTH, row.height)
        }
        GraphTarget::Grid => {
            let row = row(card, 2);
            Rect::new(
                row.right() - SWITCH.0,
                row.y + (row.height - SWITCH.1) * 0.5,
                SWITCH.0,
                SWITCH.1,
            )
        }
        GraphTarget::Small | GraphTarget::Large => {
            let row = row(card, 3);
            let x = row.right()
                - SEGMENT_WIDTH
                    * if target == GraphTarget::Small {
                        2.0
                    } else {
                        1.0
                    };
            Rect::new(x, row.y + 2.0, SEGMENT_WIDTH, row.height - 4.0)
        }
    }
}

/// The control under `point`.
pub fn hit_at(card: Rect, point: (f32, f32)) -> Option<GraphTarget> {
    if !card.contains(point) {
        return None;
    }
    TARGETS
        .into_iter()
        .find(|target| target_rect(card, *target).contains(point))
}

/// What the card shows, rebuilt by the shell from the graph and its own
/// editing state.
#[derive(Clone, Debug)]
pub struct GraphCardView {
    pub expression: MathList,
    /// The math caret, while the expression has focus.
    pub cursor: Option<MathCursor>,
    /// The four range fields as typed, `XMin, XMax, YMin, YMax`.
    pub numbers: [String; 4],
    /// Which fields hold something that is not a usable range.
    pub invalid: [bool; 4],
    /// What the expression reads as, or why it reads as nothing.
    pub status: Result<String, String>,
    pub grid: bool,
    pub large: bool,
    /// Whether the other size fits in the row.
    pub can_resize: bool,
}

pub struct GraphCard {
    view: Option<GraphCardView>,
    anchor: (f32, f32),
    focus: Option<GraphTarget>,
    hover: Option<GraphTarget>,
    hover_fade: Hover,
    dirty: Dirty,
}

impl GraphCard {
    pub fn new(view: GraphCardView, anchor: (f32, f32), focus: GraphTarget) -> Self {
        Self {
            view: Some(view),
            anchor,
            focus: Some(focus),
            hover: None,
            hover_fade: Hover::new(),
            dirty: Dirty::new(),
        }
    }

    pub fn closed() -> Self {
        Self {
            view: None,
            anchor: (0.0, 0.0),
            focus: None,
            hover: None,
            hover_fade: Hover::new(),
            dirty: Dirty::new(),
        }
    }

    fn lit(&self, target: GraphTarget) -> bool {
        self.focus == Some(target)
    }

    /// The shared look of an input box: a quiet well, outlined in the
    /// accent while it has focus and in the warning ink while it is wrong.
    fn well(&self, layer: &Layer, rect: Rect, target: GraphTarget, invalid: bool) {
        if self.hover == Some(target) && !self.lit(target) {
            theme::hover_fill(layer, rect, self.hover_fade.value());
        }
        layer.draw_rectangle(
            rect.position(),
            rect.size(),
            theme::fade(theme::alt(), 0.72),
            Rounding::uniform(6.0),
        );
        let (width, color) = if self.lit(target) {
            (1.5, theme::accent())
        } else if invalid {
            (1.0, theme::warning())
        } else {
            (1.0, theme::border())
        };
        theme::rounded_outline(layer, rect.inset(0.5), 5.5, width, color);
    }

    fn draw_expression(&self, layer: &Layer, card: Rect, view: &GraphCardView) {
        let field = expression_rect(card);
        self.well(
            layer,
            field,
            GraphTarget::Expression,
            view.status.is_err() && !view.expression.is_empty(),
        );
        let measure = |text: &str, style: &TextStyle| theme::width(layer, text, style);
        let prefix_style = TextStyle::math(16.0, theme::dim());
        let prefix = "y =";
        let prefix_width = theme::width(layer, prefix, &prefix_style);
        let content_x = field.x + 12.0 + prefix_width + 8.0;
        let room = (field.right() - 12.0 - content_x).max(1.0);
        theme::draw(
            layer,
            prefix,
            (field.x + 12.0, field.y + field.height * 0.5),
            &prefix_style,
            theme::LEFT,
        );

        let editing = view.cursor.is_some();
        if view.expression.is_empty() && !editing {
            theme::draw(
                layer,
                "Type a curve in x",
                (content_x, field.y + field.height * 0.5),
                &TextStyle::sans(12.0, theme::faint()),
                theme::LEFT,
            );
            return;
        }
        // Set as large as the field allows, down to whatever fits: the card
        // cannot clip one control, so the expression shrinks instead.
        let natural = math_layout::layout(&view.expression, 0, 1.0, &measure);
        let fit = (room / natural.width.max(1.0))
            .min((field.height - 10.0) / (natural.ascent + natural.descent).max(1.0))
            .min(1.0);
        let set = math_layout::layout(&view.expression, 0, fit, &measure);
        let middle = field.y + field.height * 0.5;
        let baseline = middle + (set.ascent - set.descent) * 0.5;
        let mut canvas = layer;
        math_paint::draw(&mut canvas, &set, (content_x, baseline), editing);
        if let Some(cursor) = &view.cursor {
            let (x, y, height) =
                math_layout::cursor_pos(&view.expression, cursor, 0, fit, &measure);
            layer.draw_rectangle(
                (content_x + x, baseline - y - height * 0.5 + 2.0),
                (2.0, (height - 4.0).max(2.0)),
                theme::accent(),
                Rounding::NONE,
            );
        }
    }

    fn draw_status(&self, layer: &Layer, card: Rect, view: &GraphCardView) {
        let rect = status_rect(card);
        let (text, color) = match &view.status {
            Ok(reading) => (reading.as_str(), theme::dim()),
            Err(_) if view.expression.is_empty() => ("Nothing to plot yet", theme::faint()),
            Err(error) => (error.as_str(), theme::warning()),
        };
        theme::draw(
            layer,
            text,
            (rect.x + 2.0, rect.y + rect.height * 0.5),
            &TextStyle::sans(11.0, color),
            theme::LEFT,
        );
    }

    fn draw_label(&self, layer: &Layer, row: Rect, label: &str) {
        theme::draw(
            layer,
            label,
            (row.x, row.y + row.height * 0.5),
            &TextStyle::sans(LABEL, theme::dim()).tracked(0.08),
            theme::LEFT,
        );
    }

    fn draw_ranges(&self, layer: &Layer, card: Rect, view: &GraphCardView) {
        for (index, label) in ["X RANGE", "Y RANGE"].into_iter().enumerate() {
            let row = row(card, index);
            self.draw_label(layer, row, label);
            let pair = if index == 0 {
                [GraphTarget::XMin, GraphTarget::XMax]
            } else {
                [GraphTarget::YMin, GraphTarget::YMax]
            };
            let first = target_rect(card, pair[0]);
            theme::draw(
                layer,
                "to",
                (first.right() + 14.0, row.y + row.height * 0.5),
                &TextStyle::sans(11.0, theme::faint()),
                theme::CENTER,
            );
            for target in pair {
                let rect = target_rect(card, target);
                let number = target.number().expect("a range field");
                let invalid = view.invalid[number];
                self.well(layer, rect, target, invalid);
                let style = TextStyle::mono(
                    12.0,
                    if invalid {
                        theme::warning()
                    } else {
                        theme::ink()
                    },
                );
                let text = &view.numbers[number];
                let at = (rect.x + 9.0, rect.y + rect.height * 0.5);
                theme::draw(layer, text, at, &style, theme::LEFT);
                if self.lit(target) {
                    let width = theme::width(layer, text, &style);
                    layer.draw_rectangle(
                        (at.0 + width + 1.0, rect.y + 7.0),
                        (1.5, rect.height - 14.0),
                        theme::accent(),
                        Rounding::NONE,
                    );
                }
            }
        }
    }

    fn draw_grid(&self, layer: &Layer, card: Rect, view: &GraphCardView) {
        let row = row(card, 2);
        self.draw_label(layer, row, "GRID");
        let rect = target_rect(card, GraphTarget::Grid);
        if self.hover == Some(GraphTarget::Grid) {
            theme::hover_fill(layer, rect.inset(-3.0), self.hover_fade.value());
        }
        layer.draw_rectangle(
            rect.position(),
            rect.size(),
            if view.grid {
                theme::accent()
            } else {
                theme::fade(theme::alt(), 0.9)
            },
            Rounding::uniform(rect.height * 0.5),
        );
        theme::rounded_outline(
            layer,
            rect.inset(0.5),
            rect.height * 0.5 - 0.5,
            if self.lit(GraphTarget::Grid) {
                1.5
            } else {
                1.0
            },
            if self.lit(GraphTarget::Grid) {
                theme::ink()
            } else if view.grid {
                theme::accent()
            } else {
                theme::border()
            },
        );
        let knob = rect.height * 0.5 - 3.0;
        let x = if view.grid {
            rect.right() - 3.0 - knob
        } else {
            rect.x + 3.0 + knob
        };
        layer.draw_circle(
            (x, rect.y + rect.height * 0.5),
            knob,
            if view.grid {
                theme::background()
            } else {
                theme::dim()
            },
        );
    }

    fn draw_size(&self, layer: &Layer, card: Rect, view: &GraphCardView) {
        let row = row(card, 3);
        self.draw_label(layer, row, "SIZE");
        for (target, label, selected) in [
            (GraphTarget::Small, "Small", !view.large),
            (GraphTarget::Large, "Large", view.large),
        ] {
            let rect = target_rect(card, target);
            let available = selected || view.can_resize;
            if available && !selected && self.hover == Some(target) {
                theme::hover_fill(layer, rect, self.hover_fade.value());
            }
            if selected {
                layer.draw_rectangle(
                    rect.position(),
                    rect.size(),
                    theme::fade(theme::selection(), 0.9),
                    Rounding::uniform(6.0),
                );
            }
            if self.lit(target) {
                theme::rounded_outline(layer, rect.inset(0.5), 5.5, 1.5, theme::accent());
            }
            theme::draw(
                layer,
                label,
                (rect.x + rect.width * 0.5, rect.y + rect.height * 0.5),
                &TextStyle::sans(
                    11.5,
                    if !available {
                        theme::faint()
                    } else if selected {
                        theme::accent()
                    } else {
                        theme::dim()
                    },
                ),
                theme::CENTER,
            );
        }
    }
}

impl Component for GraphCard {
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
        let Some(view) = self.view.clone() else {
            return;
        };
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
            "GRAPH",
            (card.x + PAD, card.y + 21.0),
            &TextStyle::sans(10.5, theme::faint()).tracked(0.12),
            theme::LEFT,
        );
        theme::draw(
            layer,
            "curve",
            (card.right() - PAD, card.y + 21.0),
            &TextStyle::sans(11.0, theme::dim()).italic(),
            theme::RIGHT,
        );
        self.draw_expression(layer, card, &view);
        self.draw_status(layer, card, &view);
        theme::rule(
            layer,
            (card.x + PAD, row(card, 0).y - 12.0),
            card.width - PAD * 2.0,
            1.0,
            theme::border(),
        );
        self.draw_ranges(layer, card, &view);
        self.draw_grid(layer, card, &view);
        self.draw_size(layer, card, &view);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card() -> Rect {
        card_anchored(Rect::new(0.0, 0.0, 800.0, 600.0), (100.0, 100.0))
    }

    fn center(rect: Rect) -> (f32, f32) {
        (rect.x + rect.width * 0.5, rect.y + rect.height * 0.5)
    }

    #[test]
    fn every_target_is_hit_at_its_own_geometry_and_inside_the_card() {
        let card = card();
        for target in TARGETS {
            let rect = target_rect(card, target);
            assert_eq!(hit_at(card, center(rect)), Some(target), "{target:?}");
            assert!(
                rect.x >= card.x && rect.right() <= card.right(),
                "{target:?} is inside the card horizontally"
            );
            assert!(
                rect.bottom() + PAD * 0.5 <= card.bottom(),
                "{target:?} is inside the card vertically"
            );
        }
    }

    #[test]
    fn no_two_targets_overlap() {
        let card = card();
        for (index, a) in TARGETS.iter().enumerate() {
            for b in &TARGETS[index + 1..] {
                let (a_rect, b_rect) = (target_rect(card, *a), target_rect(card, *b));
                let overlap = a_rect.x < b_rect.right()
                    && b_rect.x < a_rect.right()
                    && a_rect.y < b_rect.bottom()
                    && b_rect.y < a_rect.bottom();
                assert!(!overlap, "{a:?} and {b:?} overlap");
            }
        }
    }

    #[test]
    fn tab_walks_every_target_and_wraps() {
        let mut focus = GraphTarget::Expression;
        let mut seen = vec![focus];
        for _ in 1..TARGETS.len() {
            focus = step_focus(focus, true);
            seen.push(focus);
        }
        assert_eq!(seen, TARGETS.to_vec());
        assert_eq!(step_focus(focus, true), GraphTarget::Expression);
        assert_eq!(
            step_focus(GraphTarget::Expression, false),
            GraphTarget::Large
        );
    }

    #[test]
    fn a_click_outside_the_card_hits_nothing() {
        let card = card();
        assert_eq!(hit_at(card, (card.x - 5.0, card.y + 10.0)), None);
        // Inside the card but between controls.
        assert_eq!(hit_at(card, (card.x + 4.0, card.y + 4.0)), None);
    }
}
