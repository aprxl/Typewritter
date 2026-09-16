//! A plot of one curve: the expression typeset as its heading, a rounded
//! plot area, and tick marks standing off the area's edges with their
//! values beyond them.
//!
//! plotters draws the inside of the plot area (grid, axes, curve) through
//! [`CanvasBackend`]; the area's border, marks and labels are drawn here,
//! in the theme's type and colour. A canvas cannot clip, so the curve is
//! clipped to the plot's value range before plotters sees it, and the data
//! sits a hair inside the area so no line reaches a rounded corner.

use plotters::coord::ranged1d::{BoldPoints, Ranged};
use plotters::coord::{CoordTranslate, Shift};
use plotters::prelude::{
    ChartBuilder, Color as _, DrawingArea, IntoDrawingArea, LineSeries, PathElement, TRANSPARENT,
};

use super::fitted_text;
use crate::canvas::{self, Canvas};
use crate::document::math_eval::Curve;
use crate::document::widget_layout::{WIDGET_PAD, WidgetCardLayout};
use crate::document::{math_layout, math_notation, math_paint};
use crate::layout::Rect;
use crate::plot::{CanvasBackend, SUBPIXELS, plotters_color};
use crate::renderer::Rounding;
use crate::theme::{self, TextStyle};

/// The showcase curve, in the notation a note stores math in:
/// y = 3 sin(2x) e^(−x²/8), the exponent's slash kept inline (`\/`) so the
/// heading stays one line tall.
const SHOWCASE: &str =
    "y=3sym{function|sin|plain}{sin}(2x)sym{constant|euler_number|plain}{e}^{-x^2\\/8}";
const X_RANGE: (f64, f64) = (-5.0, 5.0);
const Y_RANGE: (f64, f64) = (-3.0, 3.0);
const X_TICKS: usize = 6;
const Y_TICKS: usize = 4;

const PLOT_RADIUS: f32 = 8.0;
/// How far the data sits inside the plot area's border.
const DATA_INSET: f32 = 3.0;
const CAPTION: f32 = 12.0;
const HEADING_GAP: f32 = 6.0;
const LABEL: f32 = 9.5;
const MARK: f32 = 3.0;
const MARK_GAP: f32 = 3.0;
/// Room for the y labels beside the plot area.
const Y_BAND: f32 = 22.0;
/// Room for the x labels below it.
const X_BAND: f32 = 16.0;
const LABEL_GAP: f32 = 6.0;

/// A labelled position along one edge, in logical pixels from the data
/// rectangle's start.
struct Tick {
    at: f32,
    label: String,
}

pub(super) fn draw(canvas: &mut dyn Canvas, card: &WidgetCardLayout, scale: f32, hot: f32) {
    let inner = card.rect.inset(WIDGET_PAD * scale);
    fitted_text(
        canvas,
        "GRAPH",
        Rect::new(inner.x, inner.y, inner.width, CAPTION * scale),
        &TextStyle::sans(9.0 * scale, theme::comment()).tracked(0.08),
        false,
    );

    let tree = math_notation::parse(SHOWCASE);
    let equation = {
        let measure = |text: &str, style: &TextStyle| canvas.measure(text, style);
        math_layout::layout_plain(&tree, 0, scale, &measure)
    };
    let equation_top = inner.y + CAPTION * scale + 4.0 * scale;
    if equation.width <= inner.width {
        math_paint::draw(
            canvas,
            &equation,
            (inner.x, equation_top + equation.ascent),
            false,
        );
    }

    // The footer band stays clear for the drag hint every card shares.
    let plot_top = equation_top + equation.ascent + equation.descent + HEADING_GAP * scale;
    let plot = Rect::new(
        inner.x + Y_BAND * scale,
        plot_top,
        inner.width - Y_BAND * scale - 4.0 * scale,
        card.footer.y - 4.0 * scale - X_BAND * scale - plot_top,
    );
    if plot.width <= 0.0 || plot.height <= 0.0 {
        return;
    }
    let radius = PLOT_RADIUS * scale;
    canvas.draw_rectangle(
        plot.position(),
        plot.size(),
        theme::mix(theme::background(), theme::panel(), 0.08),
        Rounding::uniform(radius),
    );

    let data = plot.inset(DATA_INSET * scale);
    let (x_ticks, y_ticks) = match Curve::parse(&tree) {
        Ok(curve) => inside(canvas, data, &curve, scale).unwrap_or_default(),
        Err(error) => {
            fitted_text(
                canvas,
                &error.to_string(),
                Rect::new(data.x, data.y, data.width, 14.0 * scale),
                &TextStyle::sans(10.0 * scale, theme::warning()),
                false,
            );
            (Vec::new(), Vec::new())
        }
    };

    canvas::rounded_outline(
        canvas,
        plot,
        radius,
        scale,
        theme::mix(theme::border(), theme::accent(), hot * 0.12),
    );
    frame(canvas, plot, data, &x_ticks, &y_ticks, scale);
}

/// Grid, axes and curve through plotters. Returns where the ticks fall.
fn inside(
    canvas: &mut dyn Canvas,
    data: Rect,
    curve: &Curve,
    scale: f32,
) -> Option<(Vec<Tick>, Vec<Tick>)> {
    let area: DrawingArea<_, Shift> = CanvasBackend::new(canvas, data).into_drawing_area();
    let mut chart = ChartBuilder::on(&area)
        .build_cartesian_2d(X_RANGE.0..X_RANGE.1, Y_RANGE.0..Y_RANGE.1)
        .ok()?;
    let grid = plotters_color(&theme::mix(theme::border(), theme::background(), 0.45));
    let axis = plotters_color(&theme::border());
    let stroke = |width: f32| (width * scale * SUBPIXELS).round().max(1.0) as u32;
    chart
        .configure_mesh()
        .disable_axes()
        .x_labels(X_TICKS)
        .y_labels(Y_TICKS)
        .light_line_style(TRANSPARENT)
        .bold_line_style(grid.stroke_width(stroke(1.0)))
        .draw()
        .ok()?;
    chart
        .draw_series([
            PathElement::new(
                [(X_RANGE.0, 0.0), (X_RANGE.1, 0.0)],
                axis.stroke_width(stroke(1.0)),
            ),
            PathElement::new(
                [(0.0, Y_RANGE.0), (0.0, Y_RANGE.1)],
                axis.stroke_width(stroke(1.0)),
            ),
        ])
        .ok()?;

    // About two samples per logical pixel.
    let samples = (data.width * 2.0).max(2.0) as usize;
    let ink = plotters_color(&theme::accent()).stroke_width(stroke(2.0));
    for run in clipped_runs(curve, samples) {
        chart.draw_series(LineSeries::new(run, ink)).ok()?;
    }

    let spec = chart.as_coord_spec();
    let x_ticks = spec
        .x_spec()
        .key_points(BoldPoints(X_TICKS))
        .into_iter()
        .map(|x| Tick {
            at: spec.translate(&(x, 0.0)).0 as f32 / SUBPIXELS,
            label: number(x),
        })
        .collect();
    let y_ticks = spec
        .y_spec()
        .key_points(BoldPoints(Y_TICKS))
        .into_iter()
        .map(|y| Tick {
            at: spec.translate(&(0.0, y)).1 as f32 / SUBPIXELS,
            label: number(y),
        })
        .collect();
    Some((x_ticks, y_ticks))
}

/// The curve sampled across the x range, cut exactly where it leaves the
/// y range so nothing is drawn outside, and broken where it is undefined or
/// jumps by far more than the range between two samples — a pole, not a
/// steep stretch to join.
fn clipped_runs(curve: &Curve, samples: usize) -> Vec<Vec<(f64, f64)>> {
    let (bottom, top) = Y_RANGE;
    let within = |y: f64| (bottom..=top).contains(&y);
    let jump = 4.0 * (top - bottom);
    let mut runs = Vec::new();
    let mut run = Vec::new();
    let mut previous: Option<(f64, f64)> = None;
    for index in 0..=samples {
        let x = X_RANGE.0 + (X_RANGE.1 - X_RANGE.0) * index as f64 / samples as f64;
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
    let style = TextStyle::sans(LABEL * scale, theme::comment());
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
        let width = canvas.measure(&tick.label, &style);
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

    #[test]
    fn the_showcase_curve_evaluates() {
        let curve = Curve::parse(&math_notation::parse(SHOWCASE)).expect("showcase parses");
        assert_eq!(curve.input.as_deref(), Some("x"));
        let expected = |x: f64| 3.0 * (2.0 * x).sin() * (-x * x / 8.0).exp();
        for x in [-4.0, -0.7, 0.0, 1.3, 4.5] {
            assert!((curve.eval(x) - expected(x)).abs() < 1e-9, "at {x}");
        }
    }

    #[test]
    fn runs_stay_inside_the_value_range_and_break_at_poles() {
        let steep = Curve::parse(&math_notation::parse("y=x^3")).unwrap();
        let runs = clipped_runs(&steep, 400);
        assert_eq!(runs.len(), 1);
        let run = &runs[0];
        assert!(run.iter().all(|(_, y)| (Y_RANGE.0..=Y_RANGE.1).contains(y)));
        // Cut exactly at the bounds, not at the last sample inside.
        assert!((run[0].1 - Y_RANGE.0).abs() < 1e-9);
        assert!((run[run.len() - 1].1 - Y_RANGE.1).abs() < 1e-9);

        let pole = Curve::parse(&math_notation::parse("y=1/x")).unwrap();
        // 400 samples land on x = 0; 401 step over it.
        for samples in [400, 401] {
            let runs = clipped_runs(&pole, samples);
            assert_eq!(runs.len(), 2, "{samples} samples");
            for run in &runs {
                assert!(run.iter().all(|(_, y)| (Y_RANGE.0..=Y_RANGE.1).contains(y)));
                // No run joins the two branches across the pole.
                assert!(run.iter().all(|(x, _)| *x < 0.0) || run.iter().all(|(x, _)| *x > 0.0));
            }
        }
    }

    #[test]
    fn tick_labels_use_a_minus_sign() {
        assert_eq!(number(-2.0), "−2");
        assert_eq!(number(0.5), "0.5");
        assert_eq!(number(-0.0), "0");
    }
}
