use super::*;
use crate::document::widget::{CalendarHeading, CalendarWidget};
use crate::document::widget_layout::widget_layout;
use crate::renderer::{Alignment, Color, HorizontalAlign, PathPaint};

#[derive(Default)]
struct RecordingCanvas {
    marks: Vec<Rect>,
    labels: Vec<String>,
}

impl Canvas for RecordingCanvas {
    fn draw_rectangle(&mut self, at: (f32, f32), size: (f32, f32), _: Color, _: Rounding) {
        self.marks.push(Rect::new(at.0, at.1, size.0, size.1));
    }
    fn draw_circle(&mut self, center: (f32, f32), radius: f32, _: Color) {
        self.marks.push(Rect::new(
            center.0 - radius,
            center.1 - radius,
            2.0 * radius,
            2.0 * radius,
        ));
    }
    fn draw_path(&mut self, _: &str, _: (f32, f32), _: f32, _: &PathPaint) {}
    fn draw_text(&mut self, text: &str, at: (f32, f32), style: &TextStyle, align: Alignment) {
        let width = self.measure(text, style);
        let offset = match align.horizontal {
            HorizontalAlign::Left => 0.0,
            HorizontalAlign::Center => width * 0.5,
            HorizontalAlign::Right => width,
        };
        self.marks.push(Rect::new(
            at.0 - offset,
            at.1 - style.size * 0.5,
            width,
            style.size,
        ));
        self.labels.push(text.to_string());
    }
    fn measure(&self, text: &str, style: &TextStyle) -> f32 {
        text.chars().count() as f32 * style.size * 0.55
    }
}

#[test]
fn active_and_hovered_cards_never_paint_in_the_markdown_lane() {
    let source = WidgetRow::new(Widget::Calendar(CalendarWidget {
        year: 2026,
        month: 3,
        heading: CalendarHeading::Both,
        selected: vec![1, 31],
    }));
    for width in [340.0, 600.0, 704.0] {
        for scale in [0.75, 1.0, 1.5] {
            let mut layout = widget_layout(&source, width * scale, 19.0, scale);
            layout.lane_has_content = true;
            let card = layout.cards[0].rect;
            for state in [
                Interaction::default(),
                Interaction {
                    active_slot: Some(0),
                    hover: Some(Hit::Day(0, 31)),
                    hover_amount: 1.0,
                    drag: None,
                },
                Interaction {
                    active_slot: Some(2),
                    drag: Some(DragPreview {
                        slot: 0,
                        target: None,
                    }),
                    ..Default::default()
                },
            ] {
                let mut canvas = RecordingCanvas::default();
                row(
                    &mut canvas,
                    &source,
                    &layout,
                    state,
                    &mut PaintCache::default(),
                );
                for mark in canvas.marks {
                    // Only the two-pixel contact shadow extends below the card.
                    assert!(
                        mark.x >= card.x - 0.001 && mark.right() <= card.right() + 0.001,
                        "{mark:?} escapes {card:?}"
                    );
                    assert!(
                        mark.y >= card.y && mark.bottom() <= card.bottom() + 2.0 * scale + 0.001
                    );
                }
                assert!(!canvas.labels.iter().any(|label| label == "Add widget"));
                if state.active_slot.is_none() {
                    assert!(!canvas.labels.iter().any(|label| label.contains("lane")));
                }
            }
        }
    }
}
