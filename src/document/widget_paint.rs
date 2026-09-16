//! Shared widget surfaces for the screen and PDF. Editing chrome is opt-in.

mod calendar;
mod clarity;
mod graph;
#[cfg(test)]
mod tests;

use crate::canvas::{self, Canvas};
use crate::document::widget::{Widget, WidgetRow};
use crate::document::widget_layout::{Hit, WIDGET_RADIUS, WidgetRowLayout};
use crate::layout::Rect;
use crate::renderer::Rounding;
use crate::theme::{self, TextStyle};

/// Work widget painters keep between repaints. Whoever paints the same rows
/// again and again owns one — the shell, for the page — so a repaint that
/// changed nothing about a widget redoes none of that widget's work.
#[derive(Default)]
pub struct PaintCache {
    graphs: graph::GraphCache,
}

impl PaintCache {
    /// How many graphs have been drawn from scratch through this cache.
    pub fn graph_builds(&self) -> u64 {
        self.graphs.builds()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Interaction {
    pub active_slot: Option<usize>,
    pub hover: Option<Hit>,
    pub hover_amount: f32,
    pub drag: Option<DragPreview>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DragPreview {
    pub slot: usize,
    pub target: Option<usize>,
}

impl Interaction {
    fn amount(self, hit: Hit) -> f32 {
        if self.hover == Some(hit) {
            self.hover_amount
        } else {
            0.0
        }
    }
}

pub fn row(
    canvas: &mut dyn Canvas,
    source: &WidgetRow,
    layout: &WidgetRowLayout,
    state: Interaction,
    cache: &mut PaintCache,
) {
    let scale = layout.scale;
    let radius = WIDGET_RADIUS * scale;
    let editing = state.active_slot.is_some() || state.hover.is_some() || state.drag.is_some();
    if editing {
        for (slot, track) in layout.tracks.iter().enumerate() {
            if source.placement_at(slot).is_some() || layout.text_track(slot) {
                continue;
            }
            let center = (track.x + track.width * 0.5, track.y + 32.0 * scale);
            let hot = state.amount(Hit::Add(slot));
            let active = state.active_slot == Some(slot);
            canvas.draw_circle(
                center,
                13.0 * scale,
                theme::mix(
                    theme::alt(),
                    theme::selection(),
                    if active { 0.75 } else { hot * 0.7 },
                ),
            );
            let color = if active {
                theme::accent()
            } else {
                theme::dim()
            };
            canvas::polyline(
                canvas,
                &[
                    (center.0 - 4.0 * scale, center.1),
                    (center.0 + 4.0 * scale, center.1),
                ],
                color.clone(),
                1.2 * scale,
            );
            canvas::polyline(
                canvas,
                &[
                    (center.0, center.1 - 4.0 * scale),
                    (center.0, center.1 + 4.0 * scale),
                ],
                color,
                1.2 * scale,
            );
            fitted_text(
                canvas,
                "Add widget",
                Rect::new(track.x, center.1 + 19.0 * scale, track.width, 14.0 * scale),
                &TextStyle::sans(10.0 * scale, theme::comment()),
                true,
            );
        }
    }
    for card in &layout.cards {
        let placement = &source.placements[card.placement];
        let active = state
            .active_slot
            .is_some_and(|slot| card.slot <= slot && slot < card.slot + card.span);
        let hot = state.hover.and_then(Hit::placement) == Some(card.placement);
        let dragging = state.drag.filter(|drag| drag.slot == card.slot);
        // A two-pixel contact shadow gives a little depth without a blur pass.
        canvas.draw_rectangle(
            (card.rect.x, card.rect.y + 2.0 * scale),
            card.rect.size(),
            theme::scale_alpha(theme::shadow_ink(), 0.16),
            Rounding::uniform(radius),
        );
        let fill = theme::mix(
            theme::background(),
            theme::panel(),
            if matches!(placement.widget, Widget::Empty) {
                0.42
            } else {
                0.28
            },
        );
        canvas.draw_rectangle(
            card.rect.position(),
            card.rect.size(),
            theme::surface_color(fill),
            Rounding::uniform(radius),
        );
        canvas::rounded_outline(
            canvas,
            card.rect,
            radius,
            scale,
            theme::mix(
                theme::border(),
                theme::accent(),
                if active || dragging.is_some() {
                    0.42
                } else if hot {
                    0.16 * state.hover_amount
                } else {
                    0.0
                },
            ),
        );
        match &placement.widget {
            Widget::Empty => {}
            Widget::Calendar(calendar) => calendar::draw(canvas, calendar, card, scale, state),
            Widget::Clarity(clarity) => clarity::draw(
                canvas,
                clarity.value,
                card,
                scale,
                if hot { state.hover_amount } else { 0.0 },
            ),
            Widget::Graph(graph) => graph::draw(
                canvas,
                graph,
                card,
                scale,
                if hot { state.hover_amount } else { 0.0 },
                &mut cache.graphs,
            ),
        }
        if active || hot || dragging.is_some() {
            let label = if let Some(drag) = dragging {
                if drag.target.is_some() {
                    "Release to place"
                } else {
                    "Space occupied"
                }
            } else if layout.lane_has_content {
                "Drag to reflow text"
            } else {
                "Drag to move"
            };
            let footer = card.footer;
            for column in 0..2 {
                for dot in 0..3 {
                    canvas.draw_circle(
                        (
                            footer.x + (2.0 + column as f32 * 3.0) * scale,
                            footer.y + (4.0 + dot as f32 * 3.0) * scale,
                        ),
                        0.8 * scale,
                        theme::comment(),
                    );
                }
            }
            fitted_text(
                canvas,
                label,
                Rect::new(
                    footer.x + 13.0 * scale,
                    footer.y,
                    (footer.width - 13.0 * scale).max(0.0),
                    footer.height,
                ),
                &TextStyle::sans(9.0 * scale, theme::comment()),
                false,
            );
        }
    }
    if let Some(drag) = state.drag
        && let Some(target) = drag.target
        && target != drag.slot
        && let Some(rect) = layout.drop_rect(drag.slot, target)
    {
        canvas.draw_rectangle(
            rect.position(),
            rect.size(),
            theme::fade(theme::selection(), 0.5),
            Rounding::uniform(radius),
        );
        canvas::rounded_outline(canvas, rect, radius, 1.5 * scale, theme::accent());
    }
}

/// Fit labels to their own rectangle; the document layer cannot scissor
/// individual cards. This also covers long years and very narrow viewports.
fn fitted_text(canvas: &mut dyn Canvas, text: &str, rect: Rect, style: &TextStyle, centered: bool) {
    if rect.width <= 0.0 {
        return;
    }
    let mut label = text.to_string();
    if canvas.measure(&label, style) > rect.width {
        while !label.is_empty() && canvas.measure(&format!("{label}…"), style) > rect.width {
            label.pop();
        }
        if label.is_empty() {
            return;
        }
        label.push('…');
    }
    canvas.draw_text(
        &label,
        (
            rect.x + if centered { rect.width * 0.5 } else { 0.0 },
            rect.y + rect.height * 0.5,
        ),
        style,
        if centered { theme::CENTER } else { theme::LEFT },
    );
}
