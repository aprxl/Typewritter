//! Widget geometry shared by rendering, pointer input, and document flow.

use crate::layout::Rect;

/// The gap between the four horizontal widget tracks.
pub const WIDGET_GAP: f32 = 20.0;
pub const WIDGET_EMPTY_HEIGHT: f32 = 112.0;
pub const WIDGET_CALENDAR_HEIGHT: f32 = 240.0;
pub const WIDGET_RADIUS: f32 = 14.0;
pub const WIDGET_PAD: f32 = 12.0;
pub const WIDGET_CALENDAR_CONTROL: f32 = 24.0;
pub const WIDGET_CALENDAR_CONTROL_GAP: f32 = 4.0;

/// One selectable calendar day, in the same document coordinates as its
/// containing widget card.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WidgetDayLayout {
    pub day: u8,
    pub rect: Rect,
}

/// Geometry of one placed widget. The source placement remains in
/// `DocLayout::source`; this snapshot only contains hit-testable rectangles.
#[derive(Clone, Debug, PartialEq)]
pub struct WidgetCardLayout {
    pub placement: usize,
    pub slot: usize,
    pub span: usize,
    pub rect: Rect,
    pub days: Vec<WidgetDayLayout>,
    pub heading: Rect,
    pub weekdays: Rect,
    pub footer: Rect,
    pub previous: Option<Rect>,
    pub next: Option<Rect>,
}

/// Geometry of a widget row, including unused tracks for edit-only affordances
/// and the largest free Markdown lane beside the cards.
#[derive(Clone, Debug, PartialEq)]
pub struct WidgetRowLayout {
    pub tracks: Vec<Rect>,
    pub cards: Vec<WidgetCardLayout>,
    pub lane: Option<Rect>,
    /// The complete widget wall this row belongs to. A wall can contain one
    /// row; export uses its bounds to keep the visual unit whole.
    pub wall: Option<Rect>,
    pub lane_has_content: bool,
    pub height: f32,
    pub scale: f32,
}

/// Pointer targets use placement indices, except an unoccupied track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Card(usize),
    Day(usize, u8),
    Previous(usize),
    Next(usize),
    Add(usize),
}

impl Hit {
    pub fn placement(self) -> Option<usize> {
        match self {
            Self::Card(index) | Self::Day(index, _) | Self::Previous(index) | Self::Next(index) => {
                Some(index)
            }
            Self::Add(_) => None,
        }
    }
}

impl WidgetRowLayout {
    pub fn text_track(&self, slot: usize) -> bool {
        self.lane_has_content
            && self.lane.is_some_and(|lane| {
                let track = self.tracks[slot];
                lane.contains((track.x + track.width * 0.5, track.y + 1.0))
            })
    }

    pub fn hit(&self, point: (f32, f32)) -> Option<Hit> {
        for card in &self.cards {
            if !card.rect.contains(point) {
                continue;
            }
            if card.previous.is_some_and(|rect| rect.contains(point)) {
                return Some(Hit::Previous(card.placement));
            }
            if card.next.is_some_and(|rect| rect.contains(point)) {
                return Some(Hit::Next(card.placement));
            }
            if let Some(day) = card.days.iter().find(|day| day.rect.contains(point)) {
                return Some(Hit::Day(card.placement, day.day));
            }
            return Some(Hit::Card(card.placement));
        }
        self.tracks.iter().enumerate().find_map(|(slot, track)| {
            (track.contains(point)
                && !self.text_track(slot)
                && !self
                    .cards
                    .iter()
                    .any(|card| card.slot <= slot && slot < card.slot + card.span))
            .then_some(Hit::Add(slot))
        })
    }

    /// Preview and commit use the same span-aware acceptance rule. A card may
    /// enter the Markdown lane: committing the move recomputes that lane and
    /// reflows its prose around the card's new position.
    pub fn drop_slot(&self, from: usize, point: (f32, f32)) -> Option<usize> {
        let card = self.cards.iter().find(|card| card.slot == from)?;
        let to = self.tracks.iter().position(|track| track.contains(point))?;
        if to + card.span > self.tracks.len()
            || self.cards.iter().any(|other| {
                other.slot != from && other.slot < to + card.span && to < other.slot + other.span
            })
        {
            return None;
        }
        Some(to)
    }

    pub fn drop_rect(&self, from: usize, to: usize) -> Option<Rect> {
        let card = self.cards.iter().find(|card| card.slot == from)?;
        let start = self.tracks.get(to)?;
        let end = self.tracks.get(to + card.span - 1)?;
        Some(Rect::new(
            start.x,
            start.y,
            end.right() - start.x,
            card.rect.height,
        ))
    }
}

fn widget_height(widget: &super::widget::Widget, scale: f32) -> f32 {
    match widget {
        super::widget::Widget::Calendar(_) => WIDGET_CALENDAR_HEIGHT * scale,
        super::widget::Widget::Empty | super::widget::Widget::Clarity(_) => {
            WIDGET_EMPTY_HEIGHT * scale
        }
    }
}

/// Measure a widget row and choose the ordinary Markdown lane beside it. A
/// lane is a contiguous run of genuinely free tracks; explicit Empty widgets
/// are occupied and therefore reserve their track.
pub fn widget_layout(
    row: &super::widget::WidgetRow,
    width: f32,
    y: f32,
    scale: f32,
) -> WidgetRowLayout {
    let gap = WIDGET_GAP * scale;
    let track_width = ((width - gap * (super::widget::TRACKS - 1) as f32)
        / super::widget::TRACKS as f32)
        .max(0.0);
    let row_height = row
        .placements
        .iter()
        .map(|placement| widget_height(&placement.widget, scale))
        .fold(WIDGET_EMPTY_HEIGHT * scale, f32::max);
    let tracks = (0..super::widget::TRACKS)
        .map(|slot| {
            Rect::new(
                slot as f32 * (track_width + gap),
                y,
                track_width,
                row_height,
            )
        })
        .collect::<Vec<_>>();
    let cards = row
        .placements
        .iter()
        .enumerate()
        .map(|(placement_index, placement)| {
            let left = tracks[placement.slot].x;
            let card_width =
                track_width * placement.span as f32 + gap * (placement.span - 1) as f32;
            let rect = Rect::new(left, y, card_width, widget_height(&placement.widget, scale));
            let inner = rect.inset(WIDGET_PAD * scale);
            // Narrow cards give navigation a separate row. Optional headings
            // never collapse this band into the selectable day cells.
            let stacked = inner.width < 128.0 * scale;
            let control = (WIDGET_CALENDAR_CONTROL * scale).min(inner.width * 0.4);
            let control_gap = WIDGET_CALENDAR_CONTROL_GAP * scale;
            let controls_y = inner.y + if stacked { 42.0 * scale } else { 2.0 * scale };
            let heading = Rect::new(
                inner.x,
                inner.y,
                if stacked {
                    inner.width
                } else {
                    inner.width - 2.0 * control - control_gap - 6.0 * scale
                },
                40.0 * scale,
            );
            let weekdays = Rect::new(
                inner.x,
                inner.y + if stacked { 72.0 } else { 44.0 } * scale,
                inner.width,
                20.0 * scale,
            );
            let footer = Rect::new(
                inner.x,
                inner.bottom() - 16.0 * scale,
                inner.width,
                16.0 * scale,
            );
            let days = match &placement.widget {
                super::widget::Widget::Calendar(calendar) => {
                    let grid = Rect::new(
                        inner.x,
                        weekdays.bottom(),
                        inner.width,
                        (footer.y - weekdays.bottom() - 6.0 * scale).max(0.0),
                    );
                    let cell_width = grid.width / 7.0;
                    let cell_height = grid.height / 6.0;
                    (1..=calendar.days())
                        .map(|day| {
                            let index = calendar.first_weekday() + day - 1;
                            WidgetDayLayout {
                                day: day as u8,
                                rect: Rect::new(
                                    grid.x + (index % 7) as f32 * cell_width,
                                    grid.y + (index / 7) as f32 * cell_height,
                                    cell_width,
                                    cell_height,
                                ),
                            }
                        })
                        .collect()
                }
                super::widget::Widget::Empty | super::widget::Widget::Clarity(_) => Vec::new(),
            };
            let (previous, next) = match &placement.widget {
                super::widget::Widget::Calendar(_) => {
                    let next_x = (inner.right() - control).max(inner.x);
                    let previous_x = (next_x - control - control_gap).max(inner.x);
                    (
                        Some(Rect::new(previous_x, controls_y, control, control)),
                        Some(Rect::new(next_x, controls_y, control, control)),
                    )
                }
                super::widget::Widget::Empty | super::widget::Widget::Clarity(_) => (None, None),
            };
            WidgetCardLayout {
                placement: placement_index,
                slot: placement.slot,
                span: placement.span,
                rect,
                days,
                heading,
                weekdays,
                footer,
                previous,
                next,
            }
        })
        .collect::<Vec<_>>();

    let occupied = (0..super::widget::TRACKS)
        .map(|slot| row.placement_at(slot).is_some())
        .collect::<Vec<_>>();
    let mut best: Option<(usize, usize)> = None;
    let mut start = 0;
    while start < occupied.len() {
        if occupied[start] {
            start += 1;
            continue;
        }
        let end = (start..occupied.len())
            .find(|&slot| occupied[slot])
            .unwrap_or(occupied.len());
        let length = end - start;
        let center_distance = ((start + end) as f32 * 0.5 - 2.0).abs();
        let better = best.is_none_or(|(best_start, best_end)| {
            let best_length = best_end - best_start;
            let best_distance = ((best_start + best_end) as f32 * 0.5 - 2.0).abs();
            length > best_length
                || (length == best_length
                    && (center_distance < best_distance
                        || (center_distance == best_distance && start < best_start)))
        });
        if better {
            best = Some((start, end));
        }
        start = end;
    }
    let lane = best.map(|(start, end)| {
        Rect::new(
            tracks[start].x,
            y,
            track_width * (end - start) as f32 + gap * (end - start - 1) as f32,
            row_height,
        )
    });
    WidgetRowLayout {
        tracks,
        cards,
        lane,
        wall: None,
        lane_has_content: false,
        height: row_height,
        scale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::widget::{
        CalendarHeading, CalendarWidget, Widget, WidgetPlacement, WidgetRow,
    };

    fn center(rect: Rect) -> (f32, f32) {
        (rect.x + rect.width * 0.5, rect.y + rect.height * 0.5)
    }
    fn overlaps(a: Rect, b: Rect) -> bool {
        a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom()
    }
    fn contains(outer: Rect, inner: Rect) -> bool {
        inner.x >= outer.x
            && inner.y >= outer.y
            && inner.right() <= outer.right() + 0.001
            && inner.bottom() <= outer.bottom() + 0.001
    }

    #[test]
    fn calendar_controls_and_days_fit_and_hit_across_months_sizes_and_headings() {
        for width in [340.0, 600.0, 704.0] {
            for scale in [0.75, 1.0, 1.5] {
                for span in [1, 2] {
                    for month in 1..=12 {
                        for heading in [
                            CalendarHeading::Both,
                            CalendarHeading::Month,
                            CalendarHeading::Year,
                            CalendarHeading::None,
                        ] {
                            let calendar = CalendarWidget {
                                year: 2024,
                                month,
                                heading,
                                selected: vec![],
                            };
                            let source = WidgetRow {
                                placements: vec![WidgetPlacement {
                                    slot: 1,
                                    span,
                                    widget: Widget::Calendar(calendar.clone()),
                                }],
                            };
                            let layout = widget_layout(&source, width * scale, 47.0, scale);
                            let card = &layout.cards[0];
                            let previous = card.previous.unwrap();
                            let next = card.next.unwrap();
                            assert!(contains(card.rect, previous) && contains(card.rect, next));
                            assert!(!overlaps(previous, next));
                            assert!(
                                !overlaps(card.heading, previous) && !overlaps(card.heading, next)
                            );
                            assert_eq!(layout.hit(center(previous)), Some(Hit::Previous(0)));
                            assert_eq!(layout.hit(center(next)), Some(Hit::Next(0)));
                            assert_eq!(card.days.len(), calendar.days() as usize);
                            for day in &card.days {
                                assert!(contains(card.rect, day.rect));
                                assert!(!overlaps(day.rect, previous) && !overlaps(day.rect, next));
                                assert!(
                                    !overlaps(day.rect, card.weekdays)
                                        && !overlaps(day.rect, card.footer)
                                );
                                assert_eq!(
                                    layout.hit(center(day.rect)),
                                    Some(Hit::Day(0, day.day))
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn wide_drop_can_reflow_text_when_pointer_is_in_a_free_track() {
        let source = WidgetRow {
            placements: vec![WidgetPlacement {
                slot: 0,
                span: 2,
                widget: Widget::Empty,
            }],
        };
        let mut layout = widget_layout(&source, 704.0, 20.0, 1.0);
        layout.lane_has_content = true;
        let on_source_edge = center(layout.tracks[1]);
        assert!(!layout.text_track(1));
        assert_eq!(layout.drop_slot(0, on_source_edge), Some(1));
        assert_eq!(layout.hit(center(layout.tracks[3])), None);
        assert_eq!(layout.drop_slot(0, center(layout.tracks[3])), None);
        let target = center(layout.tracks[2]);
        assert_eq!(layout.drop_slot(0, target), Some(2));
        let preview = layout.drop_rect(0, 2).unwrap();
        let mut moved = source.clone();
        assert!(moved.move_at(0, 2));
        assert_eq!(
            widget_layout(&moved, 704.0, 20.0, 1.0).cards[0].rect,
            preview
        );
    }

    #[test]
    fn drops_refuse_other_cards_and_other_rows() {
        let source = WidgetRow {
            placements: vec![
                WidgetPlacement {
                    slot: 0,
                    span: 1,
                    widget: Widget::Empty,
                },
                WidgetPlacement {
                    slot: 2,
                    span: 1,
                    widget: Widget::Empty,
                },
            ],
        };
        let layout = widget_layout(&source, 704.0, 20.0, 1.0);
        assert_eq!(layout.drop_slot(0, center(layout.tracks[2])), None);
        assert_eq!(
            layout.drop_slot(0, (20.0, layout.tracks[0].bottom() + 5.0)),
            None
        );
        assert_eq!(layout.drop_slot(0, center(layout.tracks[3])), Some(3));
    }
}
