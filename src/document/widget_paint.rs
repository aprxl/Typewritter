//! Shared widget-row painter for the screen and PDF.

use crate::canvas::{self, Canvas};
use crate::document::layout::{WIDGET_PAD, WIDGET_RADIUS, WidgetRowLayout};
use crate::document::widget::{CalendarHeading, ClarityLevel, Widget, WidgetRow};
use crate::layout::Rect;
use crate::renderer::{Color, LineCap, LineJoin, PathPaint, Rounding, Stroke};
use crate::theme::{self, TextStyle};

const WEEKDAYS: [&str; 7] = ["M", "T", "W", "T", "F", "S", "S"];
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

/// Paint one widget row. `active_slot` is editor state; `show_unused` keeps
/// free tracks visible only while the caret is on this row. PDF callers pass
/// `None, false`, so those affordances never leak into exports.
pub fn row(
    canvas: &mut dyn Canvas,
    source: &WidgetRow,
    layout: &WidgetRowLayout,
    active_slot: Option<usize>,
    show_unused: bool,
) {
    if show_unused {
        for (slot, track) in layout.tracks.iter().enumerate() {
            if source.placement_at(slot).is_none() {
                canvas::rounded_outline(
                    canvas,
                    *track,
                    WIDGET_RADIUS,
                    1.0,
                    theme::fade(theme::non_text(), 0.7),
                );
                let centre = (track.x + track.width * 0.5, track.y + track.height * 0.5);
                canvas.draw_path(
                    theme::icons::PLUS,
                    (centre.0 - 9.0, centre.1 - 9.0),
                    0.0,
                    &PathPaint::Stroke(Stroke::new(theme::fade(theme::non_text(), 0.8), 1.2)),
                );
            }
        }
    }

    for card in &layout.cards {
        let placement = &source.placements[card.placement];
        let active = active_slot
            .is_some_and(|slot| placement.slot <= slot && slot < placement.slot + placement.span);
        let fill = match &placement.widget {
            Widget::Empty => theme::fade(theme::alt(), 0.55),
            Widget::Calendar(_) => theme::fade(theme::panel(), 0.72),
            Widget::Clarity(_) => theme::fade(theme::alt(), 0.65),
        };
        canvas.draw_rectangle(
            card.rect.position(),
            card.rect.size(),
            fill,
            Rounding::uniform(WIDGET_RADIUS),
        );
        canvas::rounded_outline(
            canvas,
            card.rect,
            WIDGET_RADIUS,
            if active { 1.5 } else { 1.0 },
            if active {
                theme::accent()
            } else {
                theme::fade(theme::border(), 0.9)
            },
        );
        match &placement.widget {
            Widget::Empty => {}
            Widget::Calendar(calendar) => calendar_widget(canvas, calendar, card),
            Widget::Clarity(clarity) => clarity_widget(canvas, clarity.value, card.rect),
        }
    }
}

fn icon(canvas: &mut dyn Canvas, path: &str, at: (f32, f32), color: Color) {
    let mut pen = Stroke::new(color, 1.2);
    pen.cap = LineCap::Round;
    pen.join = LineJoin::Round;
    canvas.draw_path(path, at, 0.0, &PathPaint::Stroke(pen));
}

fn calendar_widget(
    canvas: &mut dyn Canvas,
    calendar: &crate::document::widget::CalendarWidget,
    card: &crate::document::layout::WidgetCardLayout,
) {
    let inner = card.rect.inset(WIDGET_PAD);
    let heading = match calendar.heading {
        CalendarHeading::Both => {
            format!("{} {}", MONTHS[calendar.month as usize - 1], calendar.year)
        }
        CalendarHeading::Month => MONTHS[calendar.month as usize - 1].to_string(),
        CalendarHeading::Year => calendar.year.to_string(),
        CalendarHeading::None => String::new(),
    };
    if !heading.is_empty() {
        canvas.draw_text(
            &heading,
            (inner.x, inner.y + 14.0),
            &TextStyle::sans(12.0, theme::ink()).bold(),
            theme::LEFT,
        );
    }
    let first_day = card.days.first().map_or(inner.y, |day| day.rect.y);
    let cell_width = card.days.first().map_or(0.0, |day| day.rect.width);
    for (index, weekday) in WEEKDAYS.iter().enumerate() {
        canvas.draw_text(
            weekday,
            (inner.x + (index as f32 + 0.5) * cell_width, first_day - 6.0),
            &TextStyle::mono(8.0, theme::comment()),
            theme::CENTER,
        );
    }
    for day in &card.days {
        let selected = calendar.is_selected(u32::from(day.day));
        let rect = day.rect.inset(1.5);
        if selected {
            canvas.draw_rectangle(
                rect.position(),
                rect.size(),
                theme::fade(theme::accent(), 0.33),
                Rounding::uniform(5.0),
            );
        }
        canvas.draw_text(
            &day.day.to_string(),
            (
                day.rect.x + day.rect.width * 0.5,
                day.rect.y + day.rect.height * 0.5,
            ),
            &TextStyle::sans(
                10.0,
                if selected {
                    theme::accent()
                } else {
                    theme::ink()
                },
            ),
            theme::CENTER,
        );
    }
    if let Some(previous) = card.previous {
        icon(
            canvas,
            theme::icons::CHEVRON_LEFT,
            (previous.x + previous.width * 0.5, previous.y + 3.0),
            theme::dim(),
        );
    }
    if let Some(next) = card.next {
        icon(
            canvas,
            theme::icons::CHEVRON_RIGHT,
            (next.x + next.width * 0.5, next.y + 3.0),
            theme::dim(),
        );
    }
}

fn clarity_widget(canvas: &mut dyn Canvas, value: Option<ClarityLevel>, rect: Rect) {
    let centre = (rect.x + rect.width * 0.5, rect.y + rect.height * 0.5);
    let (symbol, label, color) = match value {
        Some(ClarityLevel::Review) => ("?", "Review", theme::warning()),
        Some(ClarityLevel::Working) => ("~", "Working", theme::cool()),
        Some(ClarityLevel::Clear) => ("✓", "Clear", theme::live()),
        None => ("?", "Clarity", theme::comment()),
    };
    canvas.draw_text(
        symbol,
        (centre.0, centre.1 - 8.0),
        &TextStyle::sans(28.0, color.clone()).bold(),
        theme::CENTER,
    );
    canvas.draw_text(
        label,
        (centre.0, centre.1 + 22.0),
        &TextStyle::sans(11.0, color),
        theme::CENTER,
    );
}
