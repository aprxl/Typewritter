//! What painting graph widgets costs, per repaint.
//!
//!     cargo run --release --example graph_bench
//!
//! Release only: a debug build is 10–30x slower for reasons unrelated to
//! graphs, and the harness prints its profile so a wrong run is visible.
//!
//! The editor repaints its page for the caret blink (twice a second), for
//! hovers, typing and scrolling, so what a graph costs on a repaint that did
//! not change it is the number that matters. Each case paints the same
//! widget rows several times through [`widget_paint::row`] — the very call
//! the editor and the PDF export make — into a canvas that only counts what
//! it is asked to draw, so the timings are the painter's own work (parsing,
//! sampling, plotters) and not the GPU's. `built` counts graphs worked out
//! from scratch over all the repaints: once per distinct graph is right.

use std::hint::black_box;
use std::time::Instant;

use typewritter::canvas::Canvas;
use typewritter::components::editor::band_visible;
use typewritter::document::math_notation;
use typewritter::document::widget::{GraphWidget, Widget, WidgetPlacement, WidgetRow};
use typewritter::document::widget_layout::widget_layout;
use typewritter::document::widget_paint::{self, Interaction, PaintCache};
use typewritter::renderer::{Alignment, Color, PathPaint, Rounding};
use typewritter::theme::TextStyle;

/// Counts draw calls and the path data it was handed; measures text with a
/// fixed advance.
#[derive(Default)]
struct Counter {
    calls: usize,
    path_bytes: usize,
}

impl Canvas for Counter {
    fn draw_rectangle(&mut self, _: (f32, f32), _: (f32, f32), _: Color, _: Rounding) {
        self.calls += 1;
    }
    fn draw_circle(&mut self, _: (f32, f32), _: f32, _: Color) {
        self.calls += 1;
    }
    fn draw_path(&mut self, d: &str, _: (f32, f32), _: f32, _: &PathPaint) {
        self.calls += 1;
        self.path_bytes += d.len();
    }
    fn draw_text(&mut self, _: &str, _: (f32, f32), _: &TextStyle, _: Alignment) {
        self.calls += 1;
    }
    fn measure(&self, text: &str, style: &TextStyle) -> f32 {
        text.chars().count() as f32 * style.size * 0.55
    }
}

fn row(expression: &str, span: usize) -> WidgetRow {
    WidgetRow {
        placements: vec![WidgetPlacement {
            slot: 0,
            span,
            widget: Widget::Graph(GraphWidget {
                expression: math_notation::parse(expression),
                ..GraphWidget::default()
            }),
        }],
    }
}

/// Paints `rows` `repaints` times with one cache, as the editor does, and
/// reports the first paint and the mean of the rest.
fn case(name: &str, rows: &[WidgetRow], repaints: usize) {
    let layouts: Vec<_> = rows
        .iter()
        .enumerate()
        .map(|(index, row)| widget_layout(row, 704.0, index as f32 * 360.0, 1.0))
        .collect();
    let mut cache = PaintCache::default();
    let mut times = Vec::with_capacity(repaints);
    let mut counter = Counter::default();
    for _ in 0..repaints {
        counter = Counter::default();
        let started = Instant::now();
        for (row, layout) in rows.iter().zip(&layouts) {
            widget_paint::row(
                &mut counter,
                row,
                layout,
                Interaction::default(),
                &mut cache,
            );
        }
        times.push(started.elapsed().as_secs_f64() * 1e6);
        black_box(&counter);
    }
    let first = times[0];
    let rest = &times[1..];
    let warm = rest.iter().sum::<f64>() / rest.len().max(1) as f64;
    println!(
        "{name:<30} first {first:>9.0} µs   repaint {warm:>6.0} µs   {:>2} built  {:>4} calls  {:>7} path bytes",
        cache.graph_builds(),
        counter.calls,
        counter.path_bytes
    );
}

fn main() {
    println!(
        "graph_bench ({} build)\n",
        if cfg!(debug_assertions) {
            "DEBUG — timings are not meaningful"
        } else {
            "release"
        }
    );
    let wave = "3sym{function|sin|plain}{sin}(2x)sym{constant|euler_number|plain}{e}^{-x^2\\/8}";
    let integral = "int{0}{x}sym{function|cos|plain}{cos}(t^2)dt";
    case("one small graph", &[row(wave, 2)], 20);
    case("one large graph", &[row(wave, 3)], 20);
    case("one integral graph", &[row(integral, 2)], 5);
    case(
        "derivative of an integral",
        &[row("d/{dx}int{0}{x}t^4dt", 2)],
        5,
    );
    case("nested integral", &[row("int{0}{x}int{0}{t}sdsdt", 2)], 5);
    let many: Vec<_> = (0..20).map(|_| row(wave, 2)).collect();
    case("twenty graphs, all painted", &many, 10);
    // What the page paints: only the rows `editor::band_visible` admits.
    let view = typewritter::layout::Rect::new(0.0, 0.0, 704.0, 900.0);
    let on_screen: Vec<_> = many
        .iter()
        .enumerate()
        .filter(|(index, _)| band_visible(*index as f32 * 360.0, 240.0, view))
        .map(|(_, row)| row.clone())
        .collect();
    case(
        &format!("twenty graphs, {} on screen", on_screen.len()),
        &on_screen,
        10,
    );
    let distinct: Vec<_> = (0..20)
        .map(|power| row(&format!("x^{power}\\/{}", power + 1), 2))
        .collect();
    case("twenty different graphs", &distinct, 10);
}
