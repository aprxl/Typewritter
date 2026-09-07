//! Live outline, two levels (spec §3.1). In a lecture it is a progress
//! indicator as much as a navigation aid.

use crate::layout::Rect;
use crate::renderer::{Layer, Rounding};
use crate::theme::{self, TextStyle, icons};
use crate::ui::{Component, Context, Dirty, Hover};

pub const WIDTH: f32 = 230.0;
const HEADER: f32 = 70.0;
const ROW: f32 = 32.0;

/// One outline row. `depth` is the outline's nesting depth, not the
/// heading's level — a document that opens at H3 still has depth-0 roots.
pub struct Entry {
    pub number: String,
    pub name: String,
    pub depth: usize,
}

impl Entry {
    pub fn new(number: &str, name: &str, depth: usize) -> Self {
        Self {
            number: number.into(),
            name: name.into(),
            depth,
        }
    }
}

pub struct Topics {
    entries: Vec<Entry>,
    active: usize,
    /// Filled in by `draw`, hit-tested by `sync` on the next frame.
    entry_rects: Vec<Rect>,
    first_visible: Option<usize>,
    scroll_remainder: f32,
    hovered: Option<usize>,
    hover: Hover,

    dirty: Dirty,
}

impl Topics {
    /// The outline row `point` is over, hit-tested against the rects `draw`
    /// recorded — the same geometry the hover reads, so a click can never
    /// land on a row the eye was not over.
    pub fn entry_at(&self, point: (f32, f32)) -> Option<usize> {
        self.entry_rects
            .iter()
            .position(|rect| rect.contains(point))
    }

    pub fn new(entries: Vec<Entry>, active: usize) -> Self {
        Self {
            entries,
            active,
            entry_rects: Vec::new(),
            first_visible: None,
            scroll_remainder: 0.0,
            hovered: None,
            hover: Hover::new(),
            dirty: Dirty::new(),
        }
    }
}

fn visible_rows(rect: Rect) -> usize {
    ((rect.height - HEADER - 8.0).max(ROW) / ROW) as usize
}

impl Component for Topics {
    fn measure(&mut self, _: &Layer) -> (f32, f32) {
        (190.0, 200.0)
    }

    fn sync(&mut self, context: &Context) {
        if context.hovering(context.self_rect) && context.scroll_y != 0.0 {
            let max = self
                .entries
                .len()
                .saturating_sub(visible_rows(context.self_rect));
            let first = self.first_visible.unwrap_or(0) as f32;
            let movement = self.scroll_remainder - context.scroll_y;
            self.scroll_remainder = movement.fract();
            let next = (first + movement.trunc()).clamp(0.0, max as f32) as usize;
            self.dirty.write(&mut self.first_visible, Some(next));
        }
        let hovered = context.hovered_index(&self.entry_rects);
        if self
            .hover
            .track(&mut self.hovered, hovered, context.animation_dt)
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
        self.hover.is_animating()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn draw(&mut self, layer: &Layer, rect: Rect) {
        layer.draw_rectangle(rect.position(), rect.size(), theme::panel(), Rounding::NONE);
        theme::vertical_rule(layer, rect.position(), rect.height, 1.0, theme::border());
        theme::draw(
            layer,
            "ON THIS PAGE",
            (rect.x + 22.0, rect.y + 28.0),
            &TextStyle::sans(9.5, theme::faint()).bold().tracked(0.1),
            theme::LEFT,
        );
        theme::draw(
            layer,
            &format!("{:02}", self.entries.len()),
            (rect.right() - 22.0, rect.y + 28.0),
            &TextStyle::mono(10.0, theme::faint()),
            theme::RIGHT,
        );

        let count = visible_rows(rect);
        let max = self.entries.len().saturating_sub(count);
        let first = self
            .first_visible
            .unwrap_or_else(|| self.active.saturating_sub(count / 2))
            .min(max);
        self.first_visible = Some(first);
        self.entry_rects = vec![Rect::default(); self.entries.len()];
        for (index, entry) in self.entries.iter().enumerate().skip(first).take(count) {
            let y = rect.y + HEADER + (index - first) as f32 * ROW;
            let row = Rect::new(rect.x + 10.0, y - ROW / 2.0, rect.width - 20.0, ROW - 3.0);
            self.entry_rects[index] = row;
            let active = index == self.active;
            if active {
                layer.draw_rectangle(
                    row.position(),
                    row.size(),
                    theme::fade(theme::selection(), 0.6),
                    Rounding::uniform(7.0),
                );
                layer.draw_rectangle(
                    (row.x + 1.0, y - 7.0),
                    (2.0, 14.0),
                    theme::accent(),
                    Rounding::uniform(1.0),
                );
            } else if self.hovered == Some(index) {
                theme::hover_fill(layer, row, self.hover.value());
            }
            let x = rect.x + 22.0 + entry.depth.min(3) as f32 * 12.0;
            let number = TextStyle::mono(9.0, theme::faint());
            theme::draw(layer, &entry.number, (x, y - 1.5), &number, theme::LEFT);
            let name_x = x + theme::width(layer, &entry.number, &number) + 9.0;
            let style = TextStyle::sans(
                12.0,
                if active {
                    theme::accent()
                } else {
                    theme::dim()
                },
            );
            let label = theme::elide(layer, &entry.name, rect.right() - 22.0 - name_x, &style);
            theme::draw(layer, &label, (name_x, y - 1.5), &style, theme::LEFT);
        }
        if max > 0 {
            let track = rect.height - HEADER - 12.0;
            let height = (track * count as f32 / self.entries.len() as f32).max(18.0);
            let y = rect.y + HEADER - ROW / 2.0 + (track - height) * first as f32 / max as f32;
            layer.draw_rectangle(
                (rect.right() - 5.0, y),
                (2.0, height),
                theme::non_text(),
                Rounding::uniform(1.0),
            );
        }
        if self.entries.is_empty() {
            theme::icon(
                layer,
                icons::TOPICS,
                (rect.x + 23.0, rect.y + HEADER - 10.0),
                16.0,
                theme::non_text(),
                1.5,
            );
            theme::draw(
                layer,
                "A little structure helps.",
                (rect.x + 22.0, rect.y + HEADER + 26.0),
                &TextStyle::sans(12.0, theme::dim()),
                theme::LEFT,
            );
            theme::draw(
                layer,
                "Your headings appear here.",
                (rect.x + 22.0, rect.y + HEADER + 49.0),
                &TextStyle::sans(11.0, theme::faint()),
                theme::LEFT,
            );
        }
    }
}
