//! Live outline, two levels (spec §3.1). In a lecture it is a progress
//! indicator as much as a navigation aid.

use crate::layout::Rect;
use crate::renderer::{Layer, Rounding};
use crate::theme::{self, TextStyle, icons};
use crate::ui::{Component, Context, Dirty, Hover};

pub const WIDTH: f32 = 214.0;

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
    hovered: Option<usize>,
    hover: Hover,
    /// 0..1 from the writing-indicator animation.
    pulse: f32,
    dirty: Dirty,
}

impl Topics {
    pub fn new(entries: Vec<Entry>, active: usize) -> Self {
        Self {
            entries,
            active,
            entry_rects: Vec::new(),
            hovered: None,
            hover: Hover::new(),
            pulse: 0.0,
            dirty: Dirty::new(),
        }
    }
}

impl Component for Topics {
    fn measure(&mut self, layer: &Layer) -> (f32, f32) {
        let style = TextStyle::serif(14.5, theme::dim());
        let widest = self
            .entries
            .iter()
            .map(|entry| theme::width(layer, &entry.name, &style))
            .fold(0.0, f32::max);
        ((widest * 0.7).max(90.0) + 60.0, 200.0)
    }

    fn sync(&mut self, context: &Context) {
        // Quantised: the eye cannot see a 6% opacity step, and without this
        // the panel would rebuild its layer on every single frame.
        let pulse = (context.pulse * 16.0).round() / 16.0;
        self.dirty.write(&mut self.pulse, pulse);

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

    fn draw(&mut self, layer: &Layer, rect: Rect) {
        layer.draw_rectangle(rect.position(), rect.size(), theme::panel(), Rounding::NONE);
        theme::vertical_rule(layer, (rect.x, rect.y), rect.height, 1.0, theme::border());

        let mut y = rect.y + 24.0;
        theme::icon(
            layer,
            icons::TOPICS,
            (rect.x + 18.0, y - 5.5),
            11.0,
            theme::comment(),
            2.0,
        );
        theme::draw(
            layer,
            "TOPICS",
            (rect.x + 36.0, y),
            &TextStyle::mono(10.0, theme::comment()).tracked(0.18),
            theme::LEFT,
        );
        y += 28.0;

        self.entry_rects.clear();
        for (index, entry) in self.entries.iter().enumerate() {
            let active = index == self.active;
            let row = Rect::new(rect.x + 1.0, y - 11.0, rect.width - 1.0, 22.0);
            self.entry_rects.push(row);
            if active {
                layer.draw_rectangle(row.position(), row.size(), theme::selection(), Rounding::NONE);
            }
            // Same rule as the file tree: the active row's treatment wins
            // over the hover surface.
            if !active && self.hovered == Some(index) {
                theme::hover_fill(layer, row, self.hover.value());
            }
            let depth = entry.depth.min(3);
            let x = rect.x + 18.0 + depth as f32 * 13.0;
            let size = 14.5 - depth as f32 * 0.6;
            let color = if entry.depth == 0 {
                theme::dim()
            } else {
                theme::comment()
            };
            let number_style =
                TextStyle::mono(size - 3.0, if active { theme::dim() } else { theme::faint() });
            theme::draw(layer, &entry.number, (x, y), &number_style, theme::LEFT);
            // Measure number so long values do not collide with heading text.
            let name_x = x + theme::width(layer, &entry.number, &number_style) + 10.0;
            theme::draw(
                layer,
                &entry.name,
                (name_x, y),
                &TextStyle::serif(size, if active { theme::ink() } else { color }),
                theme::LEFT,
            );
            y += if entry.depth == 0 { 24.0 } else { 21.0 };
        }

        y += 12.0;
        theme::rule(
            layer,
            (rect.x + 18.0, y),
            rect.width - 36.0,
            1.0,
            theme::border(),
        );
        y += 20.0;
        // The indicator fades rather than blinking: a hard on/off in the
        // corner of the eye reads as an error, a fade reads as a pulse.
        let dot = theme::fade(theme::live(), 0.35 + 0.65 * self.pulse);
        layer.draw_rectangle((rect.x + 18.0, y - 3.0), (6.0, 6.0), dot, Rounding::NONE);
        theme::draw(
            layer,
            "WRITING NOW",
            (rect.x + 32.0, y),
            &TextStyle::mono(9.5, theme::faint()).tracked(0.16),
            theme::LEFT,
        );
    }
}
