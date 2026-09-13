//! A small self-assessment card, with three stages of understanding.

use super::fitted_text;
use crate::canvas::Canvas;
use crate::document::widget::ClarityLevel;
use crate::document::widget_layout::{WIDGET_PAD, WidgetCardLayout};
use crate::layout::Rect;
use crate::renderer::Rounding;
use crate::theme::{self, TextStyle};

pub(super) fn draw(
    canvas: &mut dyn Canvas,
    value: Option<ClarityLevel>,
    card: &WidgetCardLayout,
    scale: f32,
    hot: f32,
) {
    let inner = card.rect.inset(WIDGET_PAD * scale);
    let (label, filled, color) = match value {
        None => ("Not rated", 0, theme::comment()),
        Some(ClarityLevel::Review) => ("Needs review", 1, theme::warning()),
        Some(ClarityLevel::Working) => ("Getting there", 2, theme::cool()),
        Some(ClarityLevel::Clear) => ("Understood", 3, theme::live()),
    };
    fitted_text(
        canvas,
        "CLARITY",
        Rect::new(inner.x, inner.y, inner.width, 12.0 * scale),
        &TextStyle::sans(9.0 * scale, theme::comment()).tracked(0.08),
        false,
    );
    fitted_text(
        canvas,
        label,
        Rect::new(inner.x, inner.y + 20.0 * scale, inner.width, 20.0 * scale),
        &TextStyle::sans(14.0 * scale, theme::ink()).bold(),
        false,
    );
    let gap = 4.0 * scale;
    let width = ((inner.width - 2.0 * gap) / 3.0).max(0.0);
    for index in 0..3 {
        canvas.draw_rectangle(
            (
                inner.x + index as f32 * (width + gap),
                inner.y + 52.0 * scale,
            ),
            (width, 4.0 * scale),
            if index < filled {
                color.clone()
            } else {
                theme::mix(theme::border(), color.clone(), hot * 0.12)
            },
            Rounding::uniform(2.0 * scale),
        );
    }
}
