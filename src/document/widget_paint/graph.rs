//! A plot of one curve: the expression typeset as its heading, a rounded
//! plot area, and tick marks standing off the area's edges with their
//! values beyond them.
//!
//! plotters draws the inside of the plot area (grid, axes, curve) through
//! [`CanvasBackend`]; the area's border, marks and labels are drawn here,
//! in the theme's type and colour. A canvas cannot clip, so the curve is
//! clipped to the plot's value range before plotters sees it, and the data
//! sits a hair inside the area so no line reaches a rounded corner.
//!
//! Evaluating and sampling a curve is the expensive part — an integral can
//! take a tenth of a second — and a page repaints for every caret blink,
//! hover and scroll. So that work is done once per distinct graph and kept
//! in a [`GraphCache`] as [`Ink`]: the drawing recorded relative to the plot
//! area, replayed wherever the card currently sits. Only a change to what
//! the ink depends on (the graph itself, its size, the zoom, the theme)
//! draws it again.

use plotters::coord::ranged1d::{BoldPoints, Ranged};
use plotters::coord::{CoordTranslate, Shift};
use plotters::prelude::{
    ChartBuilder, Color as _, DrawingArea, IntoDrawingArea, LineSeries, PathElement, TRANSPARENT,
};

use std::collections::HashMap;
use std::rc::Rc;

use super::fitted_text;
use crate::canvas::{self, Canvas};
use crate::document::math::{MathList, MathNode};
use crate::document::math_eval::Curve;
use crate::document::math_layout::MathBox;
use crate::document::widget::{GraphRange, GraphWidget};
use crate::document::widget_layout::{WIDGET_PAD, WidgetCardLayout};
use crate::document::{math_layout, math_notation, math_paint};
use crate::layout::Rect;
use crate::plot::{CanvasBackend, SUBPIXELS, plotters_color};
use crate::renderer::{Alignment, Color, PathPaint, Rounding};
use crate::theme::{self, TextStyle};

const X_TICKS: usize = 6;
const Y_TICKS: usize = 4;

const PLOT_RADIUS: f32 = 8.0;
/// How far the data sits inside the plot area's border.
const DATA_INSET: f32 = 3.0;
const CAPTION: f32 = 12.0;
/// The equation's band. It never grows: a taller or wider equation is set
/// smaller, so the plot does not move while the expression is typed.
const HEADING: f32 = 26.0;
const HEADING_GAP: f32 = 4.0;
/// The smallest the equation is set before it is left out.
const HEADING_MIN_SCALE: f32 = 0.55;
const LABEL: f32 = 9.5;
const MARK: f32 = 3.0;
const MARK_GAP: f32 = 3.0;
/// Room for the y labels beside the plot area.
const Y_BAND: f32 = 22.0;
/// Room for the x labels below it.
const X_BAND: f32 = 16.0;
const LABEL_GAP: f32 = 6.0;

/// How many distinct graphs a cache keeps. A page's worth, and then some:
/// scrolling back to a graph finds its ink still there.
const CAPACITY: usize = 64;

/// A labelled position along one edge, in logical pixels from the data
/// rectangle's start, with the label's measured width.
#[derive(Default)]
struct Tick {
    at: f32,
    label: String,
    width: f32,
}

/// Everything about a graph's drawing that is expensive to work out.
struct Ink {
    /// The typeset equation and its baseline, from the heading band's top.
    heading: Option<(MathBox, f32)>,
    /// The plot's inside, relative to the data rectangle's top-left.
    plot: Vec<Op>,
    x_ticks: Vec<Tick>,
    y_ticks: Vec<Tick>,
    /// Why there is no curve, and whether that is a mistake or just empty.
    message: Option<(String, bool)>,
}

/// One recorded canvas call.
enum Op {
    Rectangle((f32, f32), (f32, f32), Color, Rounding),
    Circle((f32, f32), f32, Color),
    Path(String, (f32, f32), f32, PathPaint),
    Text(String, (f32, f32), TextStyle, Alignment),
}

/// A canvas that keeps what it is asked to draw, measuring with the canvas
/// the ink will be replayed on.
struct Recorder<'a> {
    measurer: &'a dyn Canvas,
    ops: Vec<Op>,
}

impl Canvas for Recorder<'_> {
    fn draw_rectangle(
        &mut self,
        at: (f32, f32),
        size: (f32, f32),
        color: Color,
        rounding: Rounding,
    ) {
        self.ops.push(Op::Rectangle(at, size, color, rounding));
    }

    fn draw_circle(&mut self, center: (f32, f32), radius: f32, color: Color) {
        self.ops.push(Op::Circle(center, radius, color));
    }

    fn draw_path(&mut self, d: &str, at: (f32, f32), rotation: f32, paint: &PathPaint) {
        self.ops
            .push(Op::Path(d.to_owned(), at, rotation, paint.clone()));
    }

    fn draw_text(&mut self, text: &str, at: (f32, f32), style: &TextStyle, align: Alignment) {
        self.ops
            .push(Op::Text(text.to_owned(), at, style.clone(), align));
    }

    fn measure(&self, text: &str, style: &TextStyle) -> f32 {
        self.measurer.measure(text, style)
    }
}

fn replay(canvas: &mut dyn Canvas, ops: &[Op], (x, y): (f32, f32)) {
    let moved = |(a, b): (f32, f32)| (a + x, b + y);
    for op in ops {
        match op {
            Op::Rectangle(at, size, color, rounding) => {
                canvas.draw_rectangle(moved(*at), *size, color.clone(), *rounding);
            }
            Op::Circle(center, radius, color) => {
                canvas.draw_circle(moved(*center), *radius, color.clone());
            }
            Op::Path(d, at, rotation, paint) => canvas.draw_path(d, moved(*at), *rotation, paint),
            Op::Text(text, at, style, align) => canvas.draw_text(text, moved(*at), style, *align),
        }
    }
}

/// What a graph's ink depends on. Positions are not in it: a scrolled card
/// replays the same ink somewhere else.
#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    notation: String,
    ranges: [u64; 4],
    grid: bool,
    heading: [u32; 2],
    data: [u32; 2],
    scale: u32,
    theme: u64,
}

impl Key {
    fn new(graph: &GraphWidget, heading: (f32, f32), data: (f32, f32), scale: f32) -> Self {
        Self {
            notation: math_notation::print(&graph.expression),
            ranges: [graph.x.min(), graph.x.max(), graph.y.min(), graph.y.max()].map(f64::to_bits),
            grid: graph.grid,
            heading: [heading.0.to_bits(), heading.1.to_bits()],
            data: [data.0.to_bits(), data.1.to_bits()],
            scale: scale.to_bits(),
            theme: theme::revision(),
        }
    }
}

/// Graph ink kept between repaints, least recently used first out.
#[derive(Default)]
pub struct GraphCache {
    entries: HashMap<Key, (Rc<Ink>, u64)>,
    clock: u64,
    built: u64,
}

impl GraphCache {
    /// How many times ink has been worked out rather than reused.
    pub fn builds(&self) -> u64 {
        self.built
    }

    fn ink(
        &mut self,
        measurer: &dyn Canvas,
        graph: &GraphWidget,
        heading: (f32, f32),
        data: (f32, f32),
        scale: f32,
    ) -> Rc<Ink> {
        self.clock += 1;
        let key = Key::new(graph, heading, data, scale);
        if let Some((ink, used)) = self.entries.get_mut(&key) {
            *used = self.clock;
            return ink.clone();
        }
        if self.entries.len() >= CAPACITY
            && let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(key, _)| key.clone())
        {
            self.entries.remove(&oldest);
        }
        self.built += 1;
        let ink = Rc::new(build(measurer, graph, heading, data, scale));
        self.entries.insert(key, (ink.clone(), self.clock));
        ink
    }
}

/// Works out a graph's ink from scratch.
fn build(
    measurer: &dyn Canvas,
    graph: &GraphWidget,
    heading: (f32, f32),
    data: (f32, f32),
    scale: f32,
) -> Ink {
    let mut recorder = Recorder {
        measurer,
        ops: Vec::new(),
    };
    let area = Rect::new(0.0, 0.0, data.0, data.1);
    let curve = Curve::parse(&graph.expression);
    let (mut x_ticks, mut y_ticks) =
        inside(&mut recorder, area, graph, curve.as_ref().ok(), scale).unwrap_or_default();
    let label = label_style(scale);
    for tick in x_ticks.iter_mut().chain(&mut y_ticks) {
        tick.width = measurer.measure(&tick.label, &label);
    }
    let message = match &curve {
        Ok(_) => None,
        Err(_) if graph.expression.is_empty() => {
            Some(("Click to write the curve".to_owned(), false))
        }
        Err(error) => Some((error.to_string(), true)),
    };
    Ink {
        heading: equation(measurer, &graph.expression, heading, scale),
        plot: recorder.ops,
        x_ticks,
        y_ticks,
        message,
    }
}

fn label_style(scale: f32) -> TextStyle {
    TextStyle::sans(LABEL * scale, theme::comment())
}

pub(super) fn draw(
    canvas: &mut dyn Canvas,
    graph: &GraphWidget,
    card: &WidgetCardLayout,
    scale: f32,
    hot: f32,
    cache: &mut GraphCache,
) {
    let inner = card.rect.inset(WIDGET_PAD * scale);
    fitted_text(
        canvas,
        "GRAPH",
        Rect::new(inner.x, inner.y, inner.width, CAPTION * scale),
        &TextStyle::sans(9.0 * scale, theme::comment()).tracked(0.08),
        false,
    );
    let heading = Rect::new(
        inner.x,
        inner.y + CAPTION * scale,
        inner.width,
        HEADING * scale,
    );
    // The footer band stays clear for the drag hint every card shares.
    let plot_top = heading.bottom() + HEADING_GAP * scale;
    let plot = Rect::new(
        inner.x + Y_BAND * scale,
        plot_top,
        inner.width - Y_BAND * scale - 4.0 * scale,
        card.footer.y - 4.0 * scale - X_BAND * scale - plot_top,
    );
    if plot.width <= 0.0 || plot.height <= 0.0 {
        return;
    }
    let data = plot.inset(DATA_INSET * scale);
    let ink = cache.ink(canvas, graph, heading.size(), data.size(), scale);

    if let Some((equation, baseline)) = &ink.heading {
        math_paint::draw(canvas, equation, (heading.x, heading.y + baseline), false);
    }
    let radius = PLOT_RADIUS * scale;
    canvas.draw_rectangle(
        plot.position(),
        plot.size(),
        theme::mix(theme::background(), theme::panel(), 0.08),
        Rounding::uniform(radius),
    );
    replay(canvas, &ink.plot, data.position());
    if let Some((message, mistake)) = &ink.message {
        fitted_text(
            canvas,
            message,
            Rect::new(
                data.x + 6.0 * scale,
                data.y + 4.0 * scale,
                data.width - 12.0 * scale,
                14.0 * scale,
            ),
            &TextStyle::sans(
                10.0 * scale,
                if *mistake {
                    theme::warning()
                } else {
                    theme::comment()
                },
            ),
            false,
        );
    }
    canvas::rounded_outline(
        canvas,
        plot,
        radius,
        scale,
        theme::mix(theme::border(), theme::accent(), hot * 0.12),
    );
    frame(canvas, plot, data, &ink.x_ticks, &ink.y_ticks, scale);
}

/// `y = expression`, typeset plainly and set as large as a band of `size`
/// allows; with the baseline measured from the band's top.
fn equation(
    measurer: &dyn Canvas,
    expression: &MathList,
    size: (f32, f32),
    scale: f32,
) -> Option<(MathBox, f32)> {
    let mut tree = vec![MathNode::Sym('y'), MathNode::Sym('=')];
    tree.extend(expression.iter().cloned());
    let measure = |text: &str, style: &TextStyle| measurer.measure(text, style);
    let natural = math_layout::layout_plain(&tree, 0, scale, &measure);
    let fit = (size.0 / natural.width)
        .min(size.1 / (natural.ascent + natural.descent))
        .min(1.0);
    if fit < HEADING_MIN_SCALE {
        return None;
    }
    let set = if fit < 1.0 {
        math_layout::layout_plain(&tree, 0, scale * fit, &measure)
    } else {
        natural
    };
    // Centred in the band on its own ink, so a superscript does not push
    // the baseline down.
    let baseline = size.1 * 0.5 + (set.ascent - set.descent) * 0.5;
    Some((set, baseline))
}

/// Grid, axes and curve through plotters. Returns where the ticks fall.
fn inside(
    canvas: &mut dyn Canvas,
    data: Rect,
    graph: &GraphWidget,
    curve: Option<&Curve>,
    scale: f32,
) -> Option<(Vec<Tick>, Vec<Tick>)> {
    let (x, y) = (graph.x, graph.y);
    let area: DrawingArea<_, Shift> = CanvasBackend::new(canvas, data).into_drawing_area();
    let mut chart = ChartBuilder::on(&area)
        .build_cartesian_2d(x.min()..x.max(), y.min()..y.max())
        .ok()?;
    let grid = plotters_color(&theme::mix(theme::border(), theme::background(), 0.45));
    let axis = plotters_color(&theme::border());
    let stroke = |width: f32| (width * scale * SUBPIXELS).round().max(1.0) as u32;
    if graph.grid {
        chart
            .configure_mesh()
            .disable_axes()
            .x_labels(X_TICKS)
            .y_labels(Y_TICKS)
            .light_line_style(TRANSPARENT)
            .bold_line_style(grid.stroke_width(stroke(1.0)))
            .draw()
            .ok()?;
    }
    // The axes, where the view includes them.
    let mut axes = Vec::new();
    if (y.min()..=y.max()).contains(&0.0) {
        axes.push(PathElement::new(
            [(x.min(), 0.0), (x.max(), 0.0)],
            axis.stroke_width(stroke(1.0)),
        ));
    }
    if (x.min()..=x.max()).contains(&0.0) {
        axes.push(PathElement::new(
            [(0.0, y.min()), (0.0, y.max())],
            axis.stroke_width(stroke(1.0)),
        ));
    }
    chart.draw_series(axes).ok()?;

    if let Some(curve) = curve {
        // About two samples per logical pixel.
        let samples = (data.width * 2.0).max(2.0) as usize;
        let ink = plotters_color(&theme::accent()).stroke_width(stroke(2.0));
        for run in clipped_runs(curve, x, y, samples) {
            chart.draw_series(LineSeries::new(run, ink)).ok()?;
        }
    }

    let spec = chart.as_coord_spec();
    let x_ticks = spec
        .x_spec()
        .key_points(BoldPoints(X_TICKS))
        .into_iter()
        .map(|value| Tick {
            at: spec.translate(&(value, y.min())).0 as f32 / SUBPIXELS,
            label: number(value),
            width: 0.0,
        })
        .collect();
    let y_ticks = spec
        .y_spec()
        .key_points(BoldPoints(Y_TICKS))
        .into_iter()
        .map(|value| Tick {
            at: spec.translate(&(x.min(), value)).1 as f32 / SUBPIXELS,
            label: number(value),
            width: 0.0,
        })
        .collect();
    Some((x_ticks, y_ticks))
}

/// The curve sampled across the x range, cut exactly where it leaves the
/// y range so nothing is drawn outside, and broken where it is undefined or
/// jumps by far more than the range between two samples — a pole, not a
/// steep stretch to join.
fn clipped_runs(
    curve: &Curve,
    x_range: GraphRange,
    y_range: GraphRange,
    samples: usize,
) -> Vec<Vec<(f64, f64)>> {
    let (bottom, top) = (y_range.min(), y_range.max());
    let within = |y: f64| (bottom..=top).contains(&y);
    let jump = 4.0 * (top - bottom);
    let mut runs = Vec::new();
    let mut run = Vec::new();
    let mut previous: Option<(f64, f64)> = None;
    for index in 0..=samples {
        let x = x_range.min() + (x_range.max() - x_range.min()) * index as f64 / samples as f64;
        let y = curve.eval(x);
        let current = y.is_finite().then_some((x, y));
        match (previous, current) {
            (Some((px, py)), Some((x, y))) if (y - py).abs() <= jump => {
                // Where the segment meets a bound.
                let crossing = |bound: f64| (px + (x - px) * (bound - py) / (y - py), bound);
                match (within(py), within(y)) {
                    (true, true) => {}
                    (true, false) => {
                        run.push(crossing(y.clamp(bottom, top)));
                        finish(&mut run, &mut runs);
                    }
                    (false, true) => run.push(crossing(py.clamp(bottom, top))),
                    // From below the range to above it, or back, in one step.
                    (false, false) if (py < bottom) != (y < bottom) => runs.push(vec![
                        crossing(py.clamp(bottom, top)),
                        crossing(y.clamp(bottom, top)),
                    ]),
                    (false, false) => {}
                }
            }
            _ => finish(&mut run, &mut runs),
        }
        if let Some((x, y)) = current
            && within(y)
        {
            run.push((x, y));
        }
        previous = current;
    }
    finish(&mut run, &mut runs);
    runs
}

/// Ends `run`, keeping it when it has a line to draw.
fn finish(run: &mut Vec<(f64, f64)>, runs: &mut Vec<Vec<(f64, f64)>>) {
    if run.len() > 1 {
        runs.push(std::mem::take(run));
    } else {
        run.clear();
    }
}

/// Marks off the plot area's left and bottom edges, and their labels.
fn frame(
    canvas: &mut dyn Canvas,
    plot: Rect,
    data: Rect,
    x_ticks: &[Tick],
    y_ticks: &[Tick],
    scale: f32,
) {
    let style = label_style(scale);
    let mark = theme::border();
    let radius = PLOT_RADIUS * scale;
    // Where the edge curves away, a mark would float beside it; its label
    // still reads there.
    let straight = |at: f32, start: f32, end: f32| (start + radius..=end - radius).contains(&at);

    let mut previous_right = f32::NEG_INFINITY;
    for tick in x_ticks {
        let x = data.x + tick.at;
        if !(plot.x..=plot.right()).contains(&x) {
            continue;
        }
        if straight(x, plot.x, plot.right()) {
            canvas.draw_rectangle(
                (x - 0.5 * scale, plot.bottom()),
                (scale, MARK * scale),
                mark.clone(),
                Rounding::NONE,
            );
        }
        let width = tick.width;
        if x - width / 2.0 < previous_right + LABEL_GAP * scale {
            continue;
        }
        previous_right = x + width / 2.0;
        canvas.draw_text(
            &tick.label,
            (
                x,
                plot.bottom() + (MARK + MARK_GAP) * scale + style.size * 0.5,
            ),
            &style,
            theme::CENTER,
        );
    }

    let mut previous_y: Option<f32> = None;
    for tick in y_ticks {
        let y = data.y + tick.at;
        if !(plot.y..=plot.bottom()).contains(&y) {
            continue;
        }
        if straight(y, plot.y, plot.bottom()) {
            canvas.draw_rectangle(
                (plot.x - MARK * scale, y - 0.5 * scale),
                (MARK * scale, scale),
                mark.clone(),
                Rounding::NONE,
            );
        }
        if previous_y.is_some_and(|last| (y - last).abs() < style.size + LABEL_GAP * scale * 0.5) {
            continue;
        }
        previous_y = Some(y);
        canvas.draw_text(
            &tick.label,
            (plot.x - (MARK + MARK_GAP) * scale, y),
            &style,
            theme::RIGHT,
        );
    }
}

/// A tick value without float noise, and with a real minus sign.
fn number(value: f64) -> String {
    let rounded = (value * 1000.0).round() / 1000.0 + 0.0;
    let text = format!("{rounded}");
    match text.strip_prefix('-') {
        Some(magnitude) => format!("−{magnitude}"),
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::math_notation;

    fn curve(notation: &str) -> Curve {
        Curve::parse(&math_notation::parse(notation)).expect(notation)
    }

    #[test]
    fn the_default_curve_evaluates() {
        let graph = GraphWidget::default();
        let curve = Curve::parse(&graph.expression).expect("the default parses");
        assert_eq!(curve.input.as_deref(), Some("x"));
        let expected = |x: f64| 3.0 * (2.0 * x).sin() * (-x * x / 8.0).exp();
        for x in [-4.0, -0.7, 0.0, 1.3, 4.5] {
            assert!((curve.eval(x) - expected(x)).abs() < 1e-9, "at {x}");
        }
    }

    #[test]
    fn runs_stay_inside_the_value_range_and_break_at_poles() {
        let (x, y) = (GraphWidget::DEFAULT_X, GraphWidget::DEFAULT_Y);
        let runs = clipped_runs(&curve("x^3"), x, y, 400);
        assert_eq!(runs.len(), 1);
        let run = &runs[0];
        assert!(run.iter().all(|(_, v)| (y.min()..=y.max()).contains(v)));
        // Cut exactly at the bounds, not at the last sample inside.
        assert!((run[0].1 - y.min()).abs() < 1e-9);
        assert!((run[run.len() - 1].1 - y.max()).abs() < 1e-9);

        // 400 samples land on x = 0; 401 step over it.
        for samples in [400, 401] {
            let runs = clipped_runs(&curve("1/x"), x, y, samples);
            assert_eq!(runs.len(), 2, "{samples} samples");
            for run in &runs {
                assert!(run.iter().all(|(_, v)| (y.min()..=y.max()).contains(v)));
                // No run joins the two branches across the pole.
                assert!(run.iter().all(|(t, _)| *t < 0.0) || run.iter().all(|(t, _)| *t > 0.0));
            }
        }
    }

    #[test]
    fn runs_follow_the_graph_ranges() {
        let x = GraphRange::new(2.0, 3.0).unwrap();
        let y = GraphRange::new(0.0, 100.0).unwrap();
        let runs = clipped_runs(&curve("x^2"), x, y, 10);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].first(), Some(&(2.0, 4.0)));
        assert_eq!(runs[0].last(), Some(&(3.0, 9.0)));
    }

    /// Measures with a fixed advance and draws nothing.
    struct Blank;

    impl Canvas for Blank {
        fn draw_rectangle(&mut self, _: (f32, f32), _: (f32, f32), _: Color, _: Rounding) {}
        fn draw_circle(&mut self, _: (f32, f32), _: f32, _: Color) {}
        fn draw_path(&mut self, _: &str, _: (f32, f32), _: f32, _: &PathPaint) {}
        fn draw_text(&mut self, _: &str, _: (f32, f32), _: &TextStyle, _: Alignment) {}
        fn measure(&self, text: &str, style: &TextStyle) -> f32 {
            text.chars().count() as f32 * style.size * 0.5
        }
    }

    fn card(x: f32, y: f32, span: usize) -> WidgetCardLayout {
        let row = crate::document::widget::WidgetRow {
            placements: vec![crate::document::widget::WidgetPlacement {
                slot: 0,
                span,
                widget: crate::document::widget::Widget::Graph(GraphWidget::default()),
            }],
        };
        let mut card =
            crate::document::widget_layout::widget_layout(&row, 704.0, y, 1.0).cards[0].clone();
        card.rect.x += x;
        card.footer.x += x;
        card
    }

    #[test]
    fn a_graph_is_worked_out_once_until_what_it_draws_changes() {
        let mut cache = GraphCache::default();
        let mut graph = GraphWidget::default();
        let paint = |cache: &mut GraphCache, graph: &GraphWidget, card: &WidgetCardLayout| {
            draw(&mut Blank, graph, card, 1.0, 0.0, cache);
        };
        paint(&mut cache, &graph, &card(0.0, 0.0, 2));
        assert_eq!(cache.builds(), 1);

        // Repaints, hovers and scrolling reuse the ink.
        for y in [0.0, 120.0, -3000.0] {
            paint(&mut cache, &graph, &card(0.0, y, 2));
            draw(&mut Blank, &graph, &card(40.0, y, 2), 1.0, 1.0, &mut cache);
        }
        assert_eq!(cache.builds(), 1);

        // What the ink shows does not.
        graph.expression = math_notation::parse("x^2");
        paint(&mut cache, &graph, &card(0.0, 0.0, 2));
        assert_eq!(cache.builds(), 2);
        graph.x = GraphRange::new(0.0, 1.0).unwrap();
        paint(&mut cache, &graph, &card(0.0, 0.0, 2));
        assert_eq!(cache.builds(), 3);
        graph.grid = false;
        paint(&mut cache, &graph, &card(0.0, 0.0, 2));
        assert_eq!(cache.builds(), 4);
        paint(&mut cache, &graph, &card(0.0, 0.0, 3));
        assert_eq!(cache.builds(), 5, "a larger card is a larger plot");
        draw(&mut Blank, &graph, &card(0.0, 0.0, 3), 1.5, 0.0, &mut cache);
        assert_eq!(cache.builds(), 6, "zoom sets the ink at another size");

        // Going back finds the earlier ink.
        paint(&mut cache, &GraphWidget::default(), &card(0.0, 0.0, 2));
        assert_eq!(cache.builds(), 6);
    }

    #[test]
    fn the_cache_keeps_a_bounded_number_of_graphs() {
        let mut cache = GraphCache::default();
        for power in 0..(CAPACITY + 10) {
            let graph = GraphWidget {
                expression: math_notation::parse(&format!("x^{power}")),
                ..GraphWidget::default()
            };
            draw(&mut Blank, &graph, &card(0.0, 0.0, 2), 1.0, 0.0, &mut cache);
        }
        assert_eq!(cache.entries.len(), CAPACITY);
        assert_eq!(cache.builds() as usize, CAPACITY + 10);
    }

    #[test]
    fn tick_labels_use_a_minus_sign() {
        assert_eq!(number(-2.0), "−2");
        assert_eq!(number(0.5), "0.5");
        assert_eq!(number(-0.0), "0");
    }
}
