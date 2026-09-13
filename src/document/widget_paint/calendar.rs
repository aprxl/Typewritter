//! A quiet month view: a typographic heading and generous, grid-free days.

use super::{Interaction, fitted_text};
use crate::canvas::{self, Canvas};
use crate::document::widget::{CalendarHeading, CalendarWidget};
use crate::document::widget_layout::{Hit, WidgetCardLayout};
use crate::layout::Rect;
use crate::renderer::Rounding;
use crate::theme::{self, TextStyle};

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const WEEKDAYS: [&str; 7] = ["M", "T", "W", "T", "F", "S", "S"];

pub(super) fn draw(
    canvas: &mut dyn Canvas,
    calendar: &CalendarWidget,
    card: &WidgetCardLayout,
    scale: f32,
    state: Interaction,
) {
    let heading = card.heading;
    if matches!(
        calendar.heading,
        CalendarHeading::Both | CalendarHeading::Month
    ) {
        let month = MONTHS[calendar.month as usize - 1];
        let style = TextStyle::sans(14.0 * scale, theme::ink()).bold();
        let label = if canvas.measure(month, &style) <= heading.width {
            month
        } else {
            &month[..3]
        };
        fitted_text(
            canvas,
            label,
            Rect::new(heading.x, heading.y, heading.width, 20.0 * scale),
            &style,
            false,
        );
    }
    if matches!(
        calendar.heading,
        CalendarHeading::Both | CalendarHeading::Year
    ) {
        let y = heading.y
            + if calendar.heading == CalendarHeading::Both {
                20.0 * scale
            } else {
                0.0
            };
        fitted_text(
            canvas,
            &calendar.year.to_string(),
            Rect::new(heading.x, y, heading.width, 16.0 * scale),
            &TextStyle::sans(10.0 * scale, theme::dim()),
            false,
        );
    }
    for (rect, direction, hit) in [
        (card.previous, -1.0, Hit::Previous(card.placement)),
        (card.next, 1.0, Hit::Next(card.placement)),
    ] {
        if let Some(rect) = rect {
            button(canvas, rect, direction, scale, state.amount(hit));
        }
    }
    let cell_width = card.weekdays.width / 7.0;
    for (index, weekday) in WEEKDAYS.iter().enumerate() {
        canvas.draw_text(
            weekday,
            (
                card.weekdays.x + (index as f32 + 0.5) * cell_width,
                card.weekdays.y + card.weekdays.height * 0.5,
            ),
            &TextStyle::sans((9.0 * scale).min(cell_width), theme::comment()),
            theme::CENTER,
        );
    }
    for day in &card.days {
        let selected = calendar.is_selected(u32::from(day.day));
        let hot = state.amount(Hit::Day(card.placement, day.day));
        let center = (
            day.rect.x + day.rect.width * 0.5,
            day.rect.y + day.rect.height * 0.5,
        );
        let diameter = day.rect.width.min(day.rect.height) - 2.0 * scale;
        if diameter > 0.0 && (selected || hot > 0.0) {
            canvas.draw_circle(
                center,
                diameter * 0.5,
                if selected {
                    theme::accent()
                } else {
                    theme::fade(theme::selection(), hot)
                },
            );
        }
        let style = TextStyle::sans(
            (12.0 * scale).min(day.rect.width * 0.72),
            if selected {
                theme::background()
            } else {
                theme::ink()
            },
        );
        canvas.draw_text(
            &day.day.to_string(),
            center,
            &if selected { style.bold() } else { style },
            theme::CENTER,
        );
    }
}

fn button(canvas: &mut dyn Canvas, rect: Rect, direction: f32, scale: f32, hot: f32) {
    canvas.draw_rectangle(
        rect.position(),
        rect.size(),
        theme::mix(theme::alt(), theme::selection(), 0.75 * hot),
        Rounding::uniform(8.0 * scale),
    );
    let center = (rect.x + rect.width * 0.5, rect.y + rect.height * 0.5);
    let step = (3.0 * scale).min(rect.width * 0.2);
    canvas::polyline(
        canvas,
        &[
            (center.0 - direction * step * 0.5, center.1 - step),
            (center.0 + direction * step * 0.5, center.1),
            (center.0 - direction * step * 0.5, center.1 + step),
        ],
        theme::dim(),
        1.4 * scale,
    );
}
