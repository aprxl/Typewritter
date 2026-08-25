//! Mode, note, what the editor is doing, and how it is doing.

use std::time::Duration;

use crate::layout::Rect;
use crate::renderer::{Color, Layer, Rounding};
use crate::theme::{self, TextStyle, icons};
use crate::ui::{Component, Context, Dirty};

pub const HEIGHT: f32 = 38.0;

/// What the bar says about the active note. Rebuilt by the shell whenever
/// the tabs change; the instrumentation on the right comes from [`Context`]
/// instead, because it changes every frame.
pub struct StatusLine {
    mode: String,
    /// The badge's fill — `theme::cool()` in vim's Normal mode,
    /// `theme::accent()` in Insert, chosen by the shell so this component
    /// doesn't need to know vim exists.
    mode_color: Color,
    note: String,
    saved: String,
    words: String,
    math_path: String,
    command: String,
    show_stats: bool,
    frametime: Duration,
    layouts: u32,
    redraws: (usize, usize),
    dirty: Dirty,
}

impl StatusLine {
    pub fn new(
        mode: &str,
        mode_color: Color,
        note: String,
        saved: String,
        words: String,
        show_stats: bool,
        command: String,
    ) -> Self {
        Self {
            mode: mode.into(),
            mode_color,
            note,
            saved,
            words,
            math_path: String::new(),
            command,
            show_stats,
            frametime: Duration::ZERO,
            layouts: 0,
            redraws: (0, 0),
            dirty: Dirty::new(),
        }
    }

    pub fn with_math_path(mut self, math_path: String) -> Self {
        self.math_path = math_path;
        self
    }

    fn stats(&self) -> String {
        format!(
            "solves {} · redrew {}/{} · {:.2}ms",
            self.layouts,
            self.redraws.0,
            self.redraws.1,
            self.frametime.as_secs_f64() * 1000.0
        )
    }
}

impl Component for StatusLine {
    fn measure(&mut self, layer: &Layer) -> (f32, f32) {
        // Measured against a fixed-width sample, not the live numbers: a
        // minimum that changed with every frametime digit would dirty the
        // layout on every frame, which is the opposite of the point.
        let body = TextStyle::serif(13.5, theme::dim());
        let note = if self.command.is_empty() {
            &self.note
        } else {
            &self.command
        };
        let math_width = if self.math_path.is_empty() {
            0.0
        } else {
            theme::width(layer, &self.math_path, &body) + 32.0
        };
        let left = theme::width(layer, &self.mode, &TextStyle::mono(10.0, theme::background()))
            + theme::width(layer, note, &body)
            + math_width
            + 90.0;
        let right = theme::width(layer, &self.saved, &body)
            + theme::width(layer, &self.words, &body)
            + theme::width(
                layer,
                "solves 000 · redrew 0/0 · 00.00ms",
                &TextStyle::mono(11.0, theme::faint()),
            )
            + 90.0;
        (left + right, HEIGHT)
    }

    fn sync(&mut self, context: &Context) {
        self.dirty.write(&mut self.frametime, context.frametime);
        self.dirty.write(&mut self.layouts, context.layouts);
        self.dirty.write(&mut self.redraws, context.redraws);
        self.dirty.write(&mut self.show_stats, context.show_stats);
    }

    fn is_dirty(&self) -> bool {
        self.dirty.get()
    }

    fn clear_dirty(&mut self) {
        self.dirty.clear();
    }

    fn draw(&mut self, layer: &Layer, rect: Rect) {
        layer.draw_rectangle(rect.position(), rect.size(), theme::panel(), Rounding::NONE);
        theme::rule(layer, (rect.x, rect.y), rect.width, 2.0, theme::border());
        let middle = rect.y + 2.0 + (rect.height - 2.0) / 2.0;

        let mode_style = TextStyle::mono(10.0, theme::background()).tracked(0.18);
        let mode_width = theme::width(layer, &self.mode, &mode_style) + 20.0;
        layer.draw_rectangle(
            (rect.x + 18.0, middle - 9.0),
            (mode_width, 18.0),
            self.mode_color.clone(),
            Rounding::NONE,
        );
        theme::draw(
            layer,
            &self.mode,
            (rect.x + 28.0, middle),
            &mode_style,
            theme::LEFT,
        );

        let body = TextStyle::serif(13.5, theme::dim());
        let x = rect.x + 18.0 + mode_width + 12.0;

        // The right cluster is laid out backwards from the right edge and
        // drawn first, so the note on the left knows how much room is left.
        // Both live in this one region, so a scissor cannot keep them
        // apart; only not drawing can.
        let mut right = rect.right() - 18.0;
        if self.show_stats {
            let stats = self.stats();
            let stats_style = TextStyle::mono(11.0, theme::faint());
            theme::draw(layer, &stats, (right, middle), &stats_style, theme::RIGHT);
            right -= theme::width(layer, &stats, &stats_style) + 12.0;
            theme::draw(
                layer,
                "·",
                (right, middle),
                &body.clone().color(theme::non_text()),
                theme::RIGHT,
            );
            right -= 14.0;
        }
        if !self.words.is_empty() {
            theme::draw(layer, &self.words, (right, middle), &body, theme::RIGHT);
            right -= theme::width(layer, &self.words, &body) + 12.0;
            theme::draw(
                layer,
                "·",
                (right, middle),
                &body.clone().color(theme::non_text()),
                theme::RIGHT,
            );
            right -= 14.0;
        }
        if !self.saved.is_empty() {
            theme::draw(layer, &self.saved, (right, middle), &body, theme::RIGHT);
            right -= theme::width(layer, &self.saved, &body) + 7.0;
            theme::icon(
                layer,
                icons::BRANCH,
                (right - 13.0, middle - 6.5),
                13.0,
                theme::live(),
                1.9,
            );
            right -= 26.0;
        }

        // The note's name is the first thing to go when the two clusters
        // would meet — the right cluster is the state that matters.
        let note = if self.command.is_empty() {
            &self.note
        } else {
            &self.command
        };
        let note_width = theme::width(layer, note, &body);
        if x + note_width < right {
            theme::draw(
                layer,
                note,
                (x, middle),
                &body.clone().color(theme::ink()),
                theme::LEFT,
            );
            if !self.math_path.is_empty() {
                let path_x = x + note_width + 12.0;
                let path_width = theme::width(layer, &self.math_path, &body);
                if path_x + 20.0 + path_width < right {
                    theme::icon(
                        layer,
                        icons::NEXT_SLOT,
                        (path_x, middle - 6.5),
                        13.0,
                        theme::non_text(),
                        1.9,
                    );
                    theme::draw(
                        layer,
                        &self.math_path,
                        (path_x + 20.0, middle),
                        &body,
                        theme::LEFT,
                    );
                }
            }
        }
    }
}
