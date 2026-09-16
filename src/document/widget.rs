//! Persistent widget rows.
//!
//! A widget row is a four-track horizontal band in the document flow. The
//! row owns only the occupied tracks; unoccupied tracks are deliberately not
//! model objects, which lets ordinary Markdown use the free space beside a
//! widget. The serialized form is a single compact JSON comment so Markdown
//! readers that do not know Typewritter still retain the marker as visible
//! text.

use std::ops::RangeInclusive;

use chrono::{Datelike, Local, NaiveDate};
use serde_json::{Map, Value};

use super::math::MathList;
use super::math_notation;

pub const TRACKS: usize = 4;

#[derive(Clone, PartialEq, Debug)]
pub struct WidgetRow {
    pub placements: Vec<WidgetPlacement>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct WidgetPlacement {
    pub slot: usize,
    pub span: usize,
    pub widget: Widget,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Widget {
    Empty,
    Calendar(CalendarWidget),
    Clarity(ClarityWidget),
    Graph(GraphWidget),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CalendarWidget {
    pub year: i32,
    pub month: u32,
    pub heading: CalendarHeading,
    pub selected: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CalendarHeading {
    #[default]
    Both,
    Month,
    Year,
    None,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ClarityWidget {
    pub value: Option<ClarityLevel>,
}

/// A plot of one curve, `y = expression`. Its size is its placement's span:
/// two tracks for the small graph, three for the large one.
#[derive(Clone, PartialEq, Debug)]
pub struct GraphWidget {
    /// The right-hand side of `y = …`, as a math tree.
    pub expression: MathList,
    pub x: GraphRange,
    pub y: GraphRange,
    pub grid: bool,
}

/// A plotted interval. Only [`GraphRange::new`] builds one, so both ends are
/// finite and `min < max`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct GraphRange {
    min: f64,
    max: f64,
}

impl GraphRange {
    pub fn new(min: f64, max: f64) -> Option<Self> {
        (min.is_finite() && max.is_finite() && min < max).then_some(Self { min, max })
    }

    pub fn min(self) -> f64 {
        self.min
    }

    pub fn max(self) -> f64 {
        self.max
    }
}

impl GraphWidget {
    /// The curve a new graph shows: y = 3 sin(2x) e^(−x²/8), its exponent's
    /// slash kept inline (`\/`) so the heading stays one line tall.
    const SHOWCASE: &str =
        "3sym{function|sin|plain}{sin}(2x)sym{constant|euler_number|plain}{e}^{-x^2\\/8}";
    pub const DEFAULT_X: GraphRange = GraphRange {
        min: -5.0,
        max: 5.0,
    };
    pub const DEFAULT_Y: GraphRange = GraphRange {
        min: -3.0,
        max: 3.0,
    };
}

impl Default for GraphWidget {
    fn default() -> Self {
        Self {
            expression: math_notation::parse(Self::SHOWCASE),
            x: Self::DEFAULT_X,
            y: Self::DEFAULT_Y,
            grid: true,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClarityLevel {
    Review,
    Working,
    Clear,
}

impl WidgetRow {
    pub fn new(widget: Widget) -> Self {
        Self {
            placements: vec![WidgetPlacement {
                slot: 0,
                span: widget.default_span(),
                widget,
            }],
        }
    }

    pub fn is_empty(&self) -> bool {
        self.placements.is_empty()
    }

    pub fn placement_at(&self, slot: usize) -> Option<&WidgetPlacement> {
        self.placements
            .iter()
            .find(|placement| placement.slot <= slot && slot < placement.slot + placement.span)
    }

    pub fn set(&mut self, slot: usize, span: usize, widget: Widget) -> bool {
        if slot >= TRACKS || !widget.spans().contains(&span) || slot + span > TRACKS {
            return false;
        }
        self.placements.retain(|placement| {
            placement.slot + placement.span <= slot || slot + span <= placement.slot
        });
        self.placements.push(WidgetPlacement { slot, span, widget });
        self.normalize();
        true
    }

    pub fn remove_at(&mut self, slot: usize) -> Option<WidgetPlacement> {
        let index = self.placements.iter().position(|placement| {
            placement.slot <= slot && slot < placement.slot + placement.span
        })?;
        Some(self.placements.remove(index))
    }

    /// Changes the span of the placement covering `slot`, keeping its start.
    /// Refused when the widget does not allow the span, it would leave the
    /// row, or it would cover another placement.
    pub fn resize_at(&mut self, slot: usize, span: usize) -> bool {
        let Some(index) = self
            .placements
            .iter()
            .position(|placement| placement.slot <= slot && slot < placement.slot + placement.span)
        else {
            return false;
        };
        let placement = &self.placements[index];
        let start = placement.slot;
        if span == placement.span
            || !placement.widget.spans().contains(&span)
            || start + span > TRACKS
            || self
                .placements
                .iter()
                .enumerate()
                .any(|(other, placement)| {
                    other != index
                        && placement.slot < start + span
                        && start < placement.slot + placement.span
                })
        {
            return false;
        }
        self.placements[index].span = span;
        true
    }

    /// Moves the placement covering `from` to a free horizontal track. A
    /// placement keeps its span, and another placement is never overwritten.
    pub fn move_at(&mut self, from: usize, to: usize) -> bool {
        let Some(index) = self
            .placements
            .iter()
            .position(|placement| placement.slot <= from && from < placement.slot + placement.span)
        else {
            return false;
        };
        let span = self.placements[index].span;
        if to >= TRACKS || to + span > TRACKS || to == self.placements[index].slot {
            return false;
        }
        if self
            .placements
            .iter()
            .enumerate()
            .any(|(other, placement)| {
                other != index && placement.slot < to + span && to < placement.slot + placement.span
            })
        {
            return false;
        }
        self.placements[index].slot = to;
        self.normalize();
        true
    }

    pub fn normalize(&mut self) {
        self.placements.sort_by_key(|placement| placement.slot);
    }

    pub fn local_calendar() -> CalendarWidget {
        let today = Local::now().date_naive();
        CalendarWidget {
            year: today.year(),
            month: today.month(),
            heading: CalendarHeading::Both,
            selected: Vec::new(),
        }
    }
}

impl CalendarWidget {
    pub fn days_in_month(year: i32, month: u32) -> u32 {
        let month = month.clamp(1, 12);
        let next = if month == 12 {
            NaiveDate::from_ymd_opt(year + 1, 1, 1)
        } else {
            NaiveDate::from_ymd_opt(year, month + 1, 1)
        };
        next.and_then(|date| date.pred_opt())
            .map_or(28, |date| date.day())
    }

    /// Monday-first index of the month's first day.
    pub fn first_weekday(&self) -> u32 {
        NaiveDate::from_ymd_opt(self.year, self.month, 1)
            .map_or(0, |date| date.weekday().num_days_from_monday())
    }

    pub fn days(&self) -> u32 {
        Self::days_in_month(self.year, self.month)
    }

    pub fn is_selected(&self, day: u32) -> bool {
        self.selected
            .iter()
            .any(|selected| u32::from(*selected) == day)
    }

    pub fn toggle_day(&mut self, day: u32) -> bool {
        if !(1..=self.days()).contains(&day) {
            return false;
        }
        if let Some(index) = self
            .selected
            .iter()
            .position(|selected| u32::from(*selected) == day)
        {
            self.selected.remove(index);
        } else {
            self.selected.push(day as u8);
            self.selected.sort_unstable();
        }
        true
    }

    pub fn shift_month(&mut self, delta: i32) {
        let absolute = self.year.saturating_mul(12) + self.month as i32 - 1 + delta;
        self.year = absolute.div_euclid(12);
        self.month = absolute.rem_euclid(12) as u32 + 1;
        self.selected.clear();
    }

    pub fn shift_year(&mut self, delta: i32) {
        self.year = self.year.saturating_add(delta);
        self.selected.clear();
    }
}

impl Widget {
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Calendar(_) => "calendar",
            Self::Clarity(_) => "clarity",
            Self::Graph(_) => "graph",
        }
    }

    /// How many tracks this kind of widget may cover.
    pub fn spans(&self) -> RangeInclusive<usize> {
        match self {
            Self::Graph(_) => 2..=3,
            Self::Empty | Self::Calendar(_) | Self::Clarity(_) => 1..=2,
        }
    }

    /// The span a newly inserted widget of this kind takes.
    pub fn default_span(&self) -> usize {
        *self.spans().start()
    }
}

impl CalendarHeading {
    fn as_str(self) -> Option<&'static str> {
        match self {
            Self::Both => None,
            Self::Month => Some("month"),
            Self::Year => Some("year"),
            Self::None => Some("none"),
        }
    }

    fn parse(value: Option<&Value>) -> Option<Self> {
        match value.and_then(Value::as_str) {
            None => Some(Self::Both),
            Some("month") => Some(Self::Month),
            Some("year") => Some(Self::Year),
            Some("none") => Some(Self::None),
            _ => None,
        }
    }
}

impl ClarityLevel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Review => "review",
            Self::Working => "working",
            Self::Clear => "clear",
        }
    }

    fn parse(value: Option<&Value>) -> Option<Option<Self>> {
        match value {
            None => Some(None),
            Some(Value::String(value)) => Some(match value.as_str() {
                "review" => Some(Self::Review),
                "working" => Some(Self::Working),
                "clear" => Some(Self::Clear),
                _ => return None,
            }),
            _ => None,
        }
    }
}

fn only_keys(object: &Map<String, Value>, allowed: &[&str]) -> bool {
    object.keys().all(|key| allowed.contains(&key.as_str()))
}

fn string_field<'a>(object: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    object.get(key)?.as_str()
}

fn parse_date(value: &str) -> Option<(i32, u32)> {
    let (year, month) = value.split_once('-')?;
    if year.len() != 4 || month.len() != 2 {
        return None;
    }
    let year = year.parse().ok()?;
    let month = month.parse().ok()?;
    NaiveDate::from_ymd_opt(year, month, 1).map(|_| (year, month))
}

fn parse_selected(value: Option<&Value>, days: u32) -> Option<Vec<u8>> {
    let Some(value) = value else {
        return Some(Vec::new());
    };
    let values = value.as_array()?;
    let mut selected = Vec::with_capacity(values.len());
    for value in values {
        let day: u8 = value.as_u64()?.try_into().ok()?;
        if !(1..=days).contains(&u32::from(day)) {
            return None;
        }
        selected.push(day);
    }
    selected.sort_unstable();
    selected.dedup();
    Some(selected)
}

/// `[min, max]`, or `default` when the key is absent.
fn parse_range(value: Option<&Value>, default: GraphRange) -> Option<GraphRange> {
    let Some(value) = value else {
        return Some(default);
    };
    match value.as_array()?.as_slice() {
        [min, max] => GraphRange::new(min.as_f64()?, max.as_f64()?),
        _ => None,
    }
}

fn parse_placement(value: &Value) -> Option<WidgetPlacement> {
    let object = value.as_object()?;
    let slot: usize = object.get("slot")?.as_u64()?.try_into().ok()?;
    let span: usize = match object.get("span") {
        Some(value) => value.as_u64()?.try_into().ok()?,
        None => 1,
    };
    if !(1..=TRACKS).contains(&slot) || span == 0 || slot - 1 + span > TRACKS {
        return None;
    }
    let kind = string_field(object, "type")?;
    let widget = match kind {
        "empty" if only_keys(object, &["slot", "span", "type"]) => Widget::Empty,
        "calendar"
            if only_keys(
                object,
                &["slot", "span", "type", "date", "show", "selected"],
            ) =>
        {
            let (year, month) = parse_date(string_field(object, "date")?)?;
            let selected = parse_selected(
                object.get("selected"),
                CalendarWidget::days_in_month(year, month),
            )?;
            Widget::Calendar(CalendarWidget {
                year,
                month,
                heading: CalendarHeading::parse(object.get("show"))?,
                selected,
            })
        }
        "clarity" if only_keys(object, &["slot", "span", "type", "value"]) => {
            Widget::Clarity(ClarityWidget {
                value: ClarityLevel::parse(object.get("value"))?,
            })
        }
        "graph"
            if only_keys(
                object,
                &["slot", "span", "type", "expression", "x", "y", "grid"],
            ) =>
        {
            let defaults = GraphWidget::default();
            Widget::Graph(GraphWidget {
                expression: match object.get("expression") {
                    None => defaults.expression,
                    Some(value) => math_notation::parse(value.as_str()?),
                },
                x: parse_range(object.get("x"), defaults.x)?,
                y: parse_range(object.get("y"), defaults.y)?,
                grid: match object.get("grid") {
                    None => defaults.grid,
                    Some(value) => value.as_bool()?,
                },
            })
        }
        _ => return None,
    };
    // A graph written without a span is still its smallest size.
    let span = if object.contains_key("span") {
        span
    } else {
        widget.default_span()
    };
    if !widget.spans().contains(&span) || slot - 1 + span > TRACKS {
        return None;
    }
    Some(WidgetPlacement {
        slot: slot - 1,
        span,
        widget,
    })
}

/// Parses the JSON payload of a widget marker, rejecting the whole row when
/// one placement is malformed, overlaps, or names an unsupported version.
pub fn parse_payload(payload: &str) -> Option<WidgetRow> {
    let values = serde_json::from_str::<Value>(payload)
        .ok()?
        .as_array()?
        .clone();
    if values.is_empty() {
        return None;
    }
    let mut placements = Vec::with_capacity(values.len());
    for value in values {
        placements.push(parse_placement(&value)?);
    }
    placements.sort_by_key(|placement| placement.slot);
    if placements
        .windows(2)
        .any(|pair| pair[0].slot + pair[0].span > pair[1].slot)
    {
        return None;
    }
    Some(WidgetRow { placements })
}

fn placement_json(placement: &WidgetPlacement) -> String {
    let mut fields = vec![format!("\"slot\":{}", placement.slot + 1)];
    if placement.span != placement.widget.default_span() {
        fields.push(format!("\"span\":{}", placement.span));
    }
    match &placement.widget {
        Widget::Empty => {
            fields.push("\"type\":\"empty\"".into());
        }
        Widget::Calendar(calendar) => {
            fields.push("\"type\":\"calendar\"".into());
            fields.push(format!(
                "\"date\":\"{:04}-{:02}\"",
                calendar.year, calendar.month
            ));
            if let Some(show) = calendar.heading.as_str() {
                fields.push(format!("\"show\":\"{show}\""));
            }
            if !calendar.selected.is_empty() {
                let selected = calendar
                    .selected
                    .iter()
                    .map(u8::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                fields.push(format!("\"selected\":[{selected}]"));
            }
        }
        Widget::Clarity(clarity) => {
            fields.push("\"type\":\"clarity\"".into());
            if let Some(value) = clarity.value {
                fields.push(format!("\"value\":\"{}\"", value.as_str()));
            }
        }
        Widget::Graph(graph) => {
            fields.push("\"type\":\"graph\"".into());
            let defaults = GraphWidget::default();
            if graph.expression != defaults.expression {
                let notation = math_notation::print(&graph.expression);
                fields.push(format!(
                    "\"expression\":{}",
                    serde_json::to_string(&notation).expect("a string serializes")
                ));
            }
            for (key, range, default) in [("x", graph.x, defaults.x), ("y", graph.y, defaults.y)] {
                if range != default {
                    fields.push(format!("\"{key}\":[{},{}]", range.min, range.max));
                }
            }
            if graph.grid != defaults.grid {
                fields.push(format!("\"grid\":{}", graph.grid));
            }
        }
    }
    format!("{{{}}}", fields.join(","))
}

pub fn serialize_payload(row: &WidgetRow) -> String {
    let mut placements = row.placements.clone();
    placements.sort_by_key(|placement| placement.slot);
    format!(
        "[{}]",
        placements
            .iter()
            .map(placement_json)
            .collect::<Vec<_>>()
            .join(",")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_payload_round_trips_optional_defaults() {
        let row = WidgetRow {
            placements: vec![
                WidgetPlacement {
                    slot: 1,
                    span: 1,
                    widget: Widget::Calendar(CalendarWidget {
                        year: 2026,
                        month: 9,
                        heading: CalendarHeading::Year,
                        selected: vec![3, 12],
                    }),
                },
                WidgetPlacement {
                    slot: 3,
                    span: 1,
                    widget: Widget::Clarity(ClarityWidget {
                        value: Some(ClarityLevel::Working),
                    }),
                },
            ],
        };
        let payload = serialize_payload(&row);
        let parsed = parse_payload(&payload).expect("canonical widgets parse");
        assert_eq!(parsed.placements[0].slot, 1);
        assert_eq!(parsed.placements[0].widget, row.placements[0].widget);
        assert_eq!(parsed.placements[1].widget, row.placements[1].widget);
    }

    #[test]
    fn malformed_rows_are_rejected_atomically() {
        assert!(
            parse_payload(r#"[{"slot":1,"type":"calendar","date":"2026-02","selected":[30]}]"#)
                .is_none()
        );
        assert!(
            parse_payload(r#"[{"slot":1,"type":"empty"},{"slot":1,"type":"empty"}]"#).is_none()
        );
        assert!(parse_payload(r#"[{"slot":1,"type":"unknown"}]"#).is_none());
    }

    #[test]
    fn graphs_are_two_or_three_tracks_wide() {
        let small = parse_payload(r#"[{"slot":1,"type":"graph"}]"#).expect("small graph");
        assert_eq!(small.placements[0].span, 2);
        assert_eq!(
            small.placements[0].widget,
            Widget::Graph(GraphWidget::default())
        );
        assert_eq!(serialize_payload(&small), r#"[{"slot":1,"type":"graph"}]"#);

        let large = parse_payload(r#"[{"slot":2,"span":3,"type":"graph"}]"#).expect("large graph");
        assert_eq!(large.placements[0].span, 3);
        assert_eq!(
            serialize_payload(&large),
            r#"[{"slot":2,"span":3,"type":"graph"}]"#
        );

        for rejected in [
            r#"[{"slot":1,"span":1,"type":"graph"}]"#,
            r#"[{"slot":1,"span":4,"type":"graph"}]"#,
            r#"[{"slot":3,"span":3,"type":"graph"}]"#,
            r#"[{"slot":4,"type":"graph"}]"#,
            r#"[{"slot":1,"span":3,"type":"calendar","date":"2026-09"}]"#,
            r#"[{"slot":1,"type":"graph","colour":"red"}]"#,
            r#"[{"slot":1,"type":"graph","x":[5,-5]}]"#,
            r#"[{"slot":1,"type":"graph","x":[0]}]"#,
            r#"[{"slot":1,"type":"graph","grid":"no"}]"#,
            r#"[{"slot":1,"type":"graph","expression":3}]"#,
        ] {
            assert!(parse_payload(rejected).is_none(), "{rejected}");
        }

        let edited = WidgetRow {
            placements: vec![WidgetPlacement {
                slot: 0,
                span: 3,
                widget: Widget::Graph(GraphWidget {
                    expression: math_notation::parse("x^2/2"),
                    x: GraphRange::new(-1.5, 2.0).unwrap(),
                    y: GraphWidget::DEFAULT_Y,
                    grid: false,
                }),
            }],
        };
        let payload = serialize_payload(&edited);
        assert_eq!(
            payload,
            r#"[{"slot":1,"span":3,"type":"graph","expression":"{x^2}/2","x":[-1.5,2],"grid":false}]"#
        );
        assert_eq!(parse_payload(&payload), Some(edited));

        let mut row = WidgetRow::new(Widget::Graph(GraphWidget::default()));
        assert_eq!(row.placements[0].span, 2);
        assert!(!row.set(0, 1, Widget::Graph(GraphWidget::default())));
        assert!(!row.set(2, 3, Widget::Graph(GraphWidget::default())));
        assert!(row.set(1, 3, Widget::Graph(GraphWidget::default())));
        assert!(!row.set(0, 3, Widget::Empty));
    }

    #[test]
    fn resizing_keeps_the_start_and_refuses_overlaps() {
        let mut row = WidgetRow {
            placements: vec![
                WidgetPlacement {
                    slot: 0,
                    span: 2,
                    widget: Widget::Graph(GraphWidget::default()),
                },
                WidgetPlacement {
                    slot: 2,
                    span: 1,
                    widget: Widget::Empty,
                },
            ],
        };
        assert!(!row.resize_at(1, 3), "track 2 is taken");
        row.remove_at(2);
        assert!(row.resize_at(1, 3));
        assert_eq!((row.placements[0].slot, row.placements[0].span), (0, 3));
        assert!(!row.resize_at(0, 3), "already that size");
        assert!(!row.resize_at(0, 1), "a graph is never one track");
        assert!(row.resize_at(2, 2));
        assert!(!row.resize_at(3, 2), "no placement covers track 3");
    }

    #[test]
    fn local_calendar_and_month_math_are_valid() {
        let calendar = WidgetRow::local_calendar();
        assert!((1..=12).contains(&calendar.month));
        assert_eq!(CalendarWidget::days_in_month(2024, 2), 29);
        assert_eq!(CalendarWidget::days_in_month(2025, 2), 28);
    }
}
