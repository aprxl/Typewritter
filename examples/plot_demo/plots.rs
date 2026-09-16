//! Twelve plots, each written once against plotters' generic
//! `DrawingArea`, so the same code draws through any backend.
//!
//! plotters draws only what is *inside* a plot — series, grid lines,
//! legends. The container, the tick marks and every axis label are
//! Typewritter's (see `frame.rs`); a plot hands back where its ticks fall
//! and what they read, in [`Ticks`], and the area it is given is exactly
//! the inside of that container.

use std::f64::consts::PI;

use plotters::coord::Shift;
use plotters::coord::ranged1d::{BoldPoints, Ranged};
use plotters::data::Quartiles;
use plotters::prelude::*;
use plotters::style::full_palette::{
    AMBER_700, BLUEGREY_400, DEEPORANGE_400, GREY_500, INDIGO_400, LIGHTBLUE_400, PINK_400,
    PURPLE_400, TEAL_400, TEAL_600,
};
use plotters::style::text_anchor::{HPos, Pos, VPos};
use typewritter::document::{math_eval::Curve, math_notation};

pub type Outcome<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plot {
    TypedMath,
    Waves,
    Scatter,
    Histogram,
    LogArea,
    TwoScales,
    Boxplots,
    Candles,
    ErrorBars,
    Pie,
    Surface,
    Heatmap,
    Mandelbrot,
}

/// What a plot's container carries around it. Fixed per plot, so the frame
/// can be laid out before anything is drawn.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub title: &'static str,
    /// Tick marks and labels on the left and bottom (or top) edges.
    pub ticks: bool,
    pub x_on_top: bool,
    /// A second set of y ticks on the right edge.
    pub secondary: bool,
    pub x_desc: Option<&'static str>,
    pub y_desc: Option<&'static str>,
    pub y2_desc: Option<&'static str>,
}

impl Frame {
    const fn ticked(title: &'static str) -> Self {
        Self {
            title,
            ticks: true,
            x_on_top: false,
            secondary: false,
            x_desc: None,
            y_desc: None,
            y2_desc: None,
        }
    }

    const fn bare(title: &'static str) -> Self {
        Self {
            ticks: false,
            ..Self::ticked(title)
        }
    }
}

/// A labelled position along one edge, in backend pixels from the
/// container's top-left.
#[derive(Clone, Debug)]
pub struct Tick {
    pub at: i32,
    pub label: String,
}

#[derive(Clone, Debug, Default)]
pub struct Ticks {
    pub x: Vec<Tick>,
    pub y: Vec<Tick>,
    pub y2: Vec<Tick>,
}

impl Plot {
    pub const ALL: [Plot; 13] = [
        Plot::TypedMath,
        Plot::Waves,
        Plot::Scatter,
        Plot::Histogram,
        Plot::LogArea,
        Plot::TwoScales,
        Plot::Boxplots,
        Plot::Candles,
        Plot::ErrorBars,
        Plot::Pie,
        Plot::Surface,
        Plot::Heatmap,
        Plot::Mandelbrot,
    ];

    pub fn slug(self) -> &'static str {
        match self {
            Plot::TypedMath => "typed-math",
            Plot::Waves => "waves",
            Plot::Scatter => "scatter",
            Plot::Histogram => "histogram",
            Plot::LogArea => "log-area",
            Plot::TwoScales => "two-scales",
            Plot::Boxplots => "boxplots",
            Plot::Candles => "candles",
            Plot::ErrorBars => "error-bars",
            Plot::Pie => "pie",
            Plot::Surface => "surface",
            Plot::Heatmap => "heatmap",
            Plot::Mandelbrot => "mandelbrot",
        }
    }

    pub fn frame(self) -> Frame {
        match self {
            Plot::TypedMath => Frame {
                x_desc: Some("x"),
                y_desc: Some("y"),
                ..Frame::ticked("Typed math · exmex")
            },
            Plot::Waves => Frame {
                x_desc: Some("t"),
                y_desc: Some("amplitude"),
                ..Frame::ticked("Line styles · legend")
            },
            Plot::Scatter => Frame::ticked("Scatter · markers · annotations"),
            Plot::Histogram => Frame {
                x_desc: Some("bucket"),
                y_desc: Some("count"),
                ..Frame::ticked("Histogram · segmented axis")
            },
            Plot::LogArea => Frame {
                x_desc: Some("n"),
                y_desc: Some("steps (log)"),
                ..Frame::ticked("Area series · log scale")
            },
            Plot::TwoScales => Frame {
                secondary: true,
                y_desc: Some("°C"),
                y2_desc: Some("mm"),
                ..Frame::ticked("Two y axes · custom ticks")
            },
            Plot::Boxplots => Frame::ticked("Box plots · categorical x"),
            Plot::Candles => Frame::ticked("Candlesticks · moving average"),
            Plot::ErrorBars => Frame::ticked("Error bars · sparse ticks"),
            Plot::Pie => Frame::bare("Pie · donut · no axes"),
            Plot::Surface => Frame::bare("3D surface · colormap · ← → ↑ ↓"),
            Plot::Heatmap => Frame {
                x_on_top: true,
                ..Frame::ticked("Heatmap · top axis · no grid")
            },
            Plot::Mandelbrot => Frame::ticked("Bitmap element · blit_bitmap"),
        }
    }

    /// Draws the inside of the container onto `area`, which is exactly
    /// that inside, and reports where the frame's ticks go.
    pub fn draw<DB: DrawingBackend>(
        self,
        area: &DrawingArea<DB, Shift>,
        style: Style,
        view: View,
    ) -> Outcome<Ticks>
    where
        DB::ErrorType: 'static,
    {
        area.fill(&style.palette.surface)?;
        match self {
            Plot::TypedMath => typed_math(area, style),
            Plot::Waves => waves(area, style),
            Plot::Scatter => scatter(area, style),
            Plot::Histogram => histogram(area, style),
            Plot::LogArea => log_area(area, style),
            Plot::TwoScales => two_scales(area, style),
            Plot::Boxplots => boxplots(area, style),
            Plot::Candles => candles(area, style),
            Plot::ErrorBars => error_bars(area, style),
            Plot::Pie => pie(area, style).map(|()| Ticks::default()),
            Plot::Surface => surface(area, style, view).map(|()| Ticks::default()),
            Plot::Heatmap => heatmap(area, style),
            Plot::Mandelbrot => mandelbrot(area, style),
        }
    }
}

/// The camera on the 3D surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub yaw: f64,
    pub pitch: f64,
}

impl Default for View {
    fn default() -> Self {
        Self {
            yaw: 0.6,
            pitch: 0.35,
        }
    }
}

/// The theme's colours, in plotters' terms.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub surface: RGBColor,
    pub grid: RGBColor,
    pub border: RGBColor,
    pub ink: RGBColor,
    pub dim: RGBColor,
}

/// How a plot is drawn: its palette, and the backend-pixel scale that
/// turns the logical sizes the plots are written in into what plotters
/// expects.
#[derive(Clone, Copy, Debug)]
pub struct Style {
    pub scale: f64,
    pub palette: Palette,
}

impl Style {
    fn px(self, logical: f64) -> u32 {
        (logical * self.scale).round() as u32
    }

    fn font(self, logical: f64) -> TextStyle<'static> {
        ("sans-serif", logical * self.scale)
            .into_font()
            .color(&self.palette.ink)
    }

    fn line(self, color: RGBColor, width: f64) -> ShapeStyle {
        color.stroke_width(self.px(width))
    }

    fn legend<'a, DB: DrawingBackend + 'a, CT: CoordTranslate>(
        self,
        chart: &mut ChartContext<'a, DB, CT>,
        position: SeriesLabelPosition,
    ) -> Outcome
    where
        DB::ErrorType: 'static,
    {
        chart
            .configure_series_labels()
            .position(position)
            .label_font(self.font(10.0))
            .background_style(self.palette.surface.mix(0.9))
            .border_style(self.palette.border)
            .margin(self.px(8.0))
            .legend_area_size(self.px(20.0))
            .draw()?;
        Ok(())
    }
}

// Plots hide minor grid lines with a transparent style, never
// `max_light_lines(0)`: on an integer axis that asks plotters for zero key
// points, and its search for a step that small multiplies until it
// overflows.

/// Where plotters' "nice" key values along x land, and what they read.
fn x_ticks<X: Ranged, Y: Ranged>(
    spec: &Cartesian2d<X, Y>,
    count: usize,
    label: impl Fn(&X::ValueType) -> String,
) -> Vec<Tick>
where
    Y::ValueType: Clone,
{
    let y = spec.y_spec().range().start;
    spec.x_spec()
        .key_points(BoldPoints(count))
        .into_iter()
        .map(|x| Tick {
            label: label(&x),
            at: spec.translate(&(x, y.clone())).0,
        })
        .collect()
}

fn y_ticks<X: Ranged, Y: Ranged>(
    spec: &Cartesian2d<X, Y>,
    count: usize,
    label: impl Fn(&Y::ValueType) -> String,
) -> Vec<Tick>
where
    X::ValueType: Clone,
{
    let x = spec.x_spec().range().start;
    spec.y_spec()
        .key_points(BoldPoints(count))
        .into_iter()
        .map(|y| Tick {
            label: label(&y),
            at: spec.translate(&(x.clone(), y)).1,
        })
        .collect()
}

/// A plain number, without float noise or a trailing `.0`.
fn number(value: &f64) -> String {
    let rounded = (value * 1000.0).round() / 1000.0;
    format!("{}", rounded + 0.0)
}

fn segment<T: ToString>(value: &SegmentValue<T>) -> String {
    match value {
        SegmentValue::Exact(v) | SegmentValue::CenterOf(v) => v.to_string(),
        SegmentValue::Last => String::new(),
    }
}

/// Curves typed in Typewritter's math notation, read by `math_eval` and
/// sampled here. The last entry has no numeric reading, and says so.
fn typed_math<DB: DrawingBackend>(area: &DrawingArea<DB, Shift>, s: Style) -> Outcome<Ticks>
where
    DB::ErrorType: 'static,
{
    const SIN: &str = "sym{function|sin|plain}{sin}";
    const PI: &str = "sym{constant|pi|plain}{π}";
    const RANGE: f64 = 4.0;
    const SAMPLES: usize = 800;
    // (notation, legend label, colour)
    let curves = [
        ("y=x^3/4-x".to_owned(), "y = x³/4 − x", INDIGO_400),
        (format!("y=2{SIN}({PI}x/2)"), "y = 2 sin(πx/2)", TEAL_600),
        ("y=1/x".to_owned(), "y = 1/x", DEEPORANGE_400),
        ("y=sqrt{x+3}-1".to_owned(), "y = √(x+3) − 1", PINK_400),
        ("y=int{0}{x}t".to_owned(), "y = ∫₀ˣ t", GREY_500),
    ];

    let mut chart = ChartBuilder::on(area).build_cartesian_2d(-RANGE..RANGE, -RANGE..RANGE)?;
    chart
        .configure_mesh()
        .disable_axes()
        .x_labels(9)
        .y_labels(9)
        .light_line_style(TRANSPARENT)
        .bold_line_style(s.palette.grid)
        .draw()?;
    // The axes through the origin, a step darker than the grid.
    chart.draw_series([
        PathElement::new([(-RANGE, 0.0), (RANGE, 0.0)], s.palette.border),
        PathElement::new([(0.0, -RANGE), (0.0, RANGE)], s.palette.border),
    ])?;

    let mut problems = Vec::new();
    for (notation, label, color) in &curves {
        let curve = match Curve::parse(&math_notation::parse(notation)) {
            Ok(curve) => curve,
            Err(error) => {
                problems.push(format!("{label}: {error}"));
                continue;
            }
        };
        // Sampled evenly; a run breaks where the curve is undefined or leaves
        // the view by a wide margin, so 1/x does not join across its pole.
        let mut runs: Vec<Vec<(f64, f64)>> = vec![Vec::new()];
        for i in 0..=SAMPLES {
            let x = -RANGE + 2.0 * RANGE * i as f64 / SAMPLES as f64;
            let y = curve.eval(x);
            let run = runs.last_mut().expect("runs starts non-empty");
            if y.is_finite() && y.abs() <= RANGE * 4.0 {
                run.push((x, y));
            } else if !run.is_empty() {
                runs.push(Vec::new());
            }
        }
        let color = *color;
        let mut first = true;
        for run in runs.into_iter().filter(|run| run.len() > 1) {
            let series = chart.draw_series(LineSeries::new(run, s.line(color, 2.0)))?;
            if std::mem::take(&mut first) {
                series.label(*label).legend(move |(x, y)| {
                    PathElement::new([(x, y), (x + 14, y)], s.line(color, 2.0))
                });
            }
        }
    }
    s.legend(&mut chart, SeriesLabelPosition::UpperLeft)?;
    for (line, problem) in problems.iter().enumerate() {
        let (_, height) = area.dim_in_pixel();
        area.draw(&Text::new(
            problem.clone(),
            (
                s.px(8.0) as i32,
                height as i32 - s.px(8.0 + 14.0 * (problems.len() - line) as f64) as i32,
            ),
            s.font(10.0).color(&s.palette.dim),
        ))?;
    }

    let spec = chart.as_coord_spec();
    Ok(Ticks {
        x: x_ticks(spec, 9, number),
        y: y_ticks(spec, 9, number),
        ..Ticks::default()
    })
}

/// Line series, dashed and dotted variants, π tick labels, a legend.
fn waves<DB: DrawingBackend>(area: &DrawingArea<DB, Shift>, s: Style) -> Outcome<Ticks>
where
    DB::ErrorType: 'static,
{
    const Y: usize = 6;
    let mut chart = ChartBuilder::on(area).build_cartesian_2d(0f64..4.0 * PI, -1.25f64..1.25)?;
    chart
        .configure_mesh()
        .disable_axes()
        .disable_x_mesh()
        .y_labels(Y)
        .light_line_style(TRANSPARENT)
        .bold_line_style(s.palette.grid)
        .draw()?;
    // The x grid follows the π ticks below.
    chart.draw_series((1..4).map(|k| {
        let x = k as f64 * PI;
        PathElement::new([(x, -1.25), (x, 1.25)], s.palette.grid)
    }))?;

    let t = || (0..=400).map(|i| i as f64 / 400.0 * 4.0 * PI);
    chart
        .draw_series(LineSeries::new(
            t().map(|t| (t, t.sin())),
            s.line(INDIGO_400, 2.0),
        ))?
        .label("sin t")
        .legend(move |(x, y)| PathElement::new([(x, y), (x + 18, y)], s.line(INDIGO_400, 2.0)));
    chart
        .draw_series(DashedLineSeries::new(
            t().map(|t| (t, t.cos())),
            s.px(6.0),
            s.px(4.0),
            s.line(TEAL_400, 2.0),
        ))?
        .label("cos t (dashed)")
        .legend(move |(x, y)| PathElement::new([(x, y), (x + 18, y)], s.line(TEAL_400, 2.0)));
    chart
        .draw_series(LineSeries::new(
            t().map(|t| (t, (-t / 4.0).exp() * (3.0 * t).sin())),
            s.line(DEEPORANGE_400, 2.0),
        ))?
        .label("e^(-t/4) sin 3t")
        .legend(move |(x, y)| PathElement::new([(x, y), (x + 18, y)], s.line(DEEPORANGE_400, 2.0)));
    let radius = s.px(1.5);
    chart
        .draw_series(DottedLineSeries::new(
            t().map(|t| (t, (-t / 4.0).exp())),
            0,
            s.px(7.0),
            move |point| Circle::new(point, radius, DEEPORANGE_400.filled()),
        ))?
        .label("envelope (dotted)")
        .legend(move |(x, y)| Circle::new((x + 9, y), radius, DEEPORANGE_400.filled()));
    s.legend(&mut chart, SeriesLabelPosition::UpperRight)?;

    let spec = chart.as_coord_spec();
    Ok(Ticks {
        // Multiples of π, not plotters' decimal key points.
        x: (0..=4)
            .map(|k| Tick {
                at: spec.translate(&(k as f64 * PI, 0.0)).0,
                label: match k {
                    0 => "0".to_string(),
                    1 => "π".to_string(),
                    k => format!("{k}π"),
                },
            })
            .collect(),
        y: y_ticks(spec, Y, number),
        ..Ticks::default()
    })
}

/// Three marker shapes, and an annotation composed from elements.
fn scatter<DB: DrawingBackend>(area: &DrawingArea<DB, Shift>, s: Style) -> Outcome<Ticks>
where
    DB::ErrorType: 'static,
{
    const X: usize = 6;
    const Y: usize = 5;
    let mut chart = ChartBuilder::on(area).build_cartesian_2d(-4f64..6.0, -3f64..5.0)?;
    chart
        .configure_mesh()
        .disable_axes()
        .x_labels(X)
        .y_labels(Y)
        .light_line_style(TRANSPARENT)
        .bold_line_style(s.palette.grid)
        .draw()?;

    let mut rng = Rng(7);
    let cluster = |rng: &mut Rng, (cx, cy): (f64, f64), spread: f64| -> Vec<(f64, f64)> {
        (0..70)
            .map(|_| (cx + rng.normal() * spread, cy + rng.normal() * spread))
            .collect()
    };
    let a = cluster(&mut rng, (-1.5, 2.5), 0.7);
    let b = cluster(&mut rng, (2.5, 3.0), 0.5);
    let c = cluster(&mut rng, (1.5, -0.5), 0.9);

    chart
        .draw_series(
            a.iter()
                .map(|&p| Circle::new(p, s.px(3.0), INDIGO_400.mix(0.7).filled())),
        )?
        .label("circles")
        .legend(move |(x, y)| Circle::new((x + 6, y), s.px(3.0), INDIGO_400.filled()));
    chart
        .draw_series(
            b.iter()
                .map(|&p| TriangleMarker::new(p, s.px(4.0), PINK_400.mix(0.8).filled())),
        )?
        .label("triangles")
        .legend(move |(x, y)| TriangleMarker::new((x + 6, y), s.px(4.0), PINK_400.filled()));
    chart
        .draw_series(
            c.iter()
                .map(|&p| Cross::new(p, s.px(3.0), s.line(TEAL_600, 1.5))),
        )?
        .label("crosses")
        .legend(move |(x, y)| Cross::new((x + 6, y), s.px(3.0), s.line(TEAL_600, 1.5)));

    // EmptyElement + shapes + text: one composed, pixel-offset annotation
    // anchored at a data point.
    for (points, name) in [(&a, "A"), (&b, "B"), (&c, "C")] {
        let n = points.len() as f64;
        let centroid = (
            points.iter().map(|p| p.0).sum::<f64>() / n,
            points.iter().map(|p| p.1).sum::<f64>() / n,
        );
        let ring = s.px(6.0) as i32;
        chart.draw_series(std::iter::once(
            EmptyElement::at(centroid)
                + Circle::new((0, 0), ring as u32, s.line(s.palette.ink, 1.5))
                + Text::new(
                    format!("{name} ({:.1}, {:.1})", centroid.0, centroid.1),
                    (ring + 3, -ring - 4),
                    s.font(10.0),
                ),
        ))?;
    }
    s.legend(&mut chart, SeriesLabelPosition::LowerLeft)?;

    let spec = chart.as_coord_spec();
    Ok(Ticks {
        x: x_ticks(spec, X, number),
        y: y_ticks(spec, Y, number),
        ..Ticks::default()
    })
}

/// Segmented (categorical) axis and a histogram built from raw samples.
fn histogram<DB: DrawingBackend>(area: &DrawingArea<DB, Shift>, s: Style) -> Outcome<Ticks>
where
    DB::ErrorType: 'static,
{
    const Y: usize = 5;
    let mut rng = Rng(42);
    let samples: Vec<u32> = (0..3000)
        .filter_map(|_| {
            let value = 10.0 + rng.normal() * 3.2;
            (0.0..20.0).contains(&value).then_some(value as u32)
        })
        .collect();

    let mut chart =
        ChartBuilder::on(area).build_cartesian_2d((0u32..19u32).into_segmented(), 0u32..500u32)?;
    chart
        .configure_mesh()
        .disable_axes()
        .disable_x_mesh()
        .y_labels(Y)
        .light_line_style(TRANSPARENT)
        .bold_line_style(s.palette.grid)
        .draw()?;
    chart.draw_series(
        Histogram::vertical(&chart)
            .style_func(|bucket, _| {
                if matches!(bucket, SegmentValue::CenterOf(8..=11)) {
                    TEAL_600.filled()
                } else {
                    TEAL_400.mix(0.55).filled()
                }
            })
            .margin(s.px(2.0))
            .data(samples.iter().map(|&bucket| (bucket, 1))),
    )?;

    let spec = chart.as_coord_spec();
    Ok(Ticks {
        // Every other bucket, labelled at its centre.
        x: x_ticks(spec, 20, segment)
            .into_iter()
            .filter(|tick| tick.label.parse::<u32>().is_ok_and(|b| b % 2 == 0))
            .collect(),
        y: y_ticks(spec, Y, |v| v.to_string()),
        ..Ticks::default()
    })
}

/// Logarithmic y axis, filled area series, y-only grid.
fn log_area<DB: DrawingBackend>(area: &DrawingArea<DB, Shift>, s: Style) -> Outcome<Ticks>
where
    DB::ErrorType: 'static,
{
    const X: usize = 5;
    const Y: usize = 5;
    let mut chart =
        ChartBuilder::on(area).build_cartesian_2d(1f64..64.0, (1f64..5000.0).log_scale())?;
    chart
        .configure_mesh()
        .disable_axes()
        .disable_x_mesh()
        .y_labels(Y)
        .light_line_style(TRANSPARENT)
        .bold_line_style(s.palette.grid)
        .draw()?;

    let n = || (1..=64).map(f64::from);
    for (name, color, f) in [
        ("n²", PURPLE_400, (|n: f64| n * n) as fn(f64) -> f64),
        ("n log n", INDIGO_400, |n: f64| (n * n.log2()).max(1.0)),
        ("n", TEAL_400, |n: f64| n),
    ] {
        chart
            .draw_series(
                AreaSeries::new(n().map(|x| (x, f(x))), 1.0, color.mix(0.18))
                    .border_style(s.line(color, 1.5)),
            )?
            .label(name)
            .legend(move |(x, y)| {
                Rectangle::new([(x, y - 4), (x + 12, y + 4)], color.mix(0.6).filled())
            });
    }
    s.legend(&mut chart, SeriesLabelPosition::UpperLeft)?;

    let spec = chart.as_coord_spec();
    Ok(Ticks {
        x: x_ticks(spec, X, number),
        y: y_ticks(spec, Y, number),
        ..Ticks::default()
    })
}

/// A secondary y axis, custom tick labels, bars drawn as rectangles.
fn two_scales<DB: DrawingBackend>(area: &DrawingArea<DB, Shift>, s: Style) -> Outcome<Ticks>
where
    DB::ErrorType: 'static,
{
    const MONTHS: [&str; 12] = ["J", "F", "M", "A", "M", "J", "J", "A", "S", "O", "N", "D"];
    const TEMPERATURE: [f64; 12] = [
        3.1, 4.0, 7.2, 10.9, 14.6, 18.0, 20.3, 19.9, 16.4, 11.8, 6.9, 3.8,
    ];
    const RAIN: [f64; 12] = [
        78.0, 60.0, 64.0, 52.0, 58.0, 49.0, 44.0, 57.0, 62.0, 88.0, 96.0, 91.0,
    ];
    const Y: usize = 5;

    let mut chart = ChartBuilder::on(area)
        .build_cartesian_2d(0f64..12.0, 0f64..25.0)?
        .set_secondary_coord(0f64..12.0, 0f64..125.0);
    chart
        .configure_mesh()
        .disable_axes()
        .disable_x_mesh()
        .y_labels(Y)
        .light_line_style(TRANSPARENT)
        .bold_line_style(s.palette.grid)
        .draw()?;

    chart
        .draw_secondary_series(RAIN.iter().enumerate().map(|(month, &rain)| {
            let x = month as f64;
            Rectangle::new(
                [(x + 0.2, 0.0), (x + 0.8, rain)],
                LIGHTBLUE_400.mix(0.35).filled(),
            )
        }))?
        .label("rain")
        .legend(|(x, y)| Rectangle::new([(x, y - 4), (x + 12, y + 4)], LIGHTBLUE_400.filled()));
    chart
        .draw_series(
            LineSeries::new(
                TEMPERATURE
                    .iter()
                    .enumerate()
                    .map(|(m, &t)| (m as f64 + 0.5, t)),
                s.line(DEEPORANGE_400, 2.0),
            )
            .point_size(s.px(3.0)),
        )?
        .label("temperature")
        .legend(move |(x, y)| PathElement::new([(x, y), (x + 12, y)], s.line(DEEPORANGE_400, 2.0)));
    s.legend(&mut chart, SeriesLabelPosition::UpperLeft)?;

    let spec = chart.as_coord_spec();
    let secondary = chart.secondary_plotting_area().as_coord_spec();
    Ok(Ticks {
        // One tick per month, at its centre rather than plotters' key points.
        x: MONTHS
            .iter()
            .enumerate()
            .map(|(month, name)| Tick {
                at: spec.translate(&(month as f64 + 0.5, 0.0)).0,
                label: name.to_string(),
            })
            .collect(),
        y: y_ticks(spec, Y, number),
        y2: y_ticks(secondary, Y, number),
    })
}

/// Box-and-whisker plots over a categorical axis.
fn boxplots<DB: DrawingBackend>(area: &DrawingArea<DB, Shift>, s: Style) -> Outcome<Ticks>
where
    DB::ErrorType: 'static,
{
    const Y: usize = 6;
    let mut rng = Rng(3);
    let groups = ["alpha", "beta", "gamma", "delta"];
    let data: Vec<Quartiles> = [(12.0, 2.0), (16.0, 4.5), (9.0, 1.2), (14.0, 3.0)]
        .iter()
        .map(|&(mean, spread)| {
            let values: Vec<f64> = (0..60).map(|_| mean + rng.normal() * spread).collect();
            Quartiles::new(&values)
        })
        .collect();

    let mut chart =
        ChartBuilder::on(area).build_cartesian_2d(groups[..].into_segmented(), 0f32..28f32)?;
    chart
        .configure_mesh()
        .disable_axes()
        .disable_x_mesh()
        .y_labels(Y)
        .light_line_style(TRANSPARENT)
        .bold_line_style(s.palette.grid)
        .draw()?;
    let colors = [INDIGO_400, PINK_400, TEAL_600, AMBER_700];
    chart.draw_series(groups.iter().zip(&data).zip(colors).map(
        |((group, quartiles), color)| {
            Boxplot::new_vertical(SegmentValue::CenterOf(group), quartiles)
                .width(s.px(26.0))
                .whisker_width(0.6)
                .style(s.line(color, 1.5))
        },
    ))?;

    let spec = chart.as_coord_spec();
    Ok(Ticks {
        x: x_ticks(spec, groups.len(), segment),
        y: y_ticks(spec, Y, |v| number(&f64::from(*v))),
        ..Ticks::default()
    })
}

/// Candlesticks with a moving average on top.
fn candles<DB: DrawingBackend>(area: &DrawingArea<DB, Shift>, s: Style) -> Outcome<Ticks>
where
    DB::ErrorType: 'static,
{
    const X: usize = 6;
    const Y: usize = 5;
    let mut rng = Rng(11);
    let mut price = 100.0;
    let days: Vec<(u32, f64, f64, f64, f64)> = (1..=36)
        .map(|day| {
            let open = price;
            let close = open + rng.normal() * 2.4 + 0.25;
            let high = open.max(close) + rng.uniform() * 1.8;
            let low = open.min(close) - rng.uniform() * 1.8;
            price = close;
            (day, open, high, low, close)
        })
        .collect();
    let low = days.iter().map(|d| d.3).fold(f64::MAX, f64::min) - 2.0;
    let high = days.iter().map(|d| d.2).fold(f64::MIN, f64::max) + 2.0;

    let mut chart = ChartBuilder::on(area).build_cartesian_2d(0u32..37u32, low..high)?;
    chart
        .configure_mesh()
        .disable_axes()
        .x_labels(X)
        .y_labels(Y)
        .light_line_style(TRANSPARENT)
        .bold_line_style(s.palette.grid)
        .draw()?;
    chart.draw_series(days.iter().map(|&(day, open, high, low, close)| {
        CandleStick::new(
            day,
            open,
            high,
            low,
            close,
            TEAL_600.filled(),
            DEEPORANGE_400.filled(),
            s.px(5.0),
        )
    }))?;
    let average: Vec<(u32, f64)> = days
        .windows(5)
        .map(|window| (window[4].0, window.iter().map(|d| d.4).sum::<f64>() / 5.0))
        .collect();
    chart
        .draw_series(LineSeries::new(average, s.line(BLUEGREY_400, 1.5)))?
        .label("5-day average")
        .legend(move |(x, y)| PathElement::new([(x, y), (x + 14, y)], s.line(BLUEGREY_400, 1.5)));
    s.legend(&mut chart, SeriesLabelPosition::UpperLeft)?;

    let spec = chart.as_coord_spec();
    Ok(Ticks {
        x: x_ticks(spec, X, |d| format!("d{d}")),
        y: y_ticks(spec, Y, |p| format!("${p:.0}")),
        ..Ticks::default()
    })
}

/// Noisy samples, down-sampled into error bars, with a fitted line.
fn error_bars<DB: DrawingBackend>(area: &DrawingArea<DB, Shift>, s: Style) -> Outcome<Ticks>
where
    DB::ErrorType: 'static,
{
    const X: usize = 5;
    const Y: usize = 5;
    let mut rng = Rng(5);
    let raw: Vec<(f64, f64)> = (0..400)
        .map(|i| {
            let x = i as f64 / 40.0;
            (x, 0.8 * x - 2.0 + rng.normal() * 1.1)
        })
        .collect();
    let bars: Vec<(f64, f64, f64, f64)> = raw
        .chunks(40)
        .map(|chunk| {
            let n = chunk.len() as f64;
            let x = chunk.iter().map(|p| p.0).sum::<f64>() / n;
            let mean = chunk.iter().map(|p| p.1).sum::<f64>() / n;
            let deviation = (chunk.iter().map(|p| (p.1 - mean).powi(2)).sum::<f64>() / n).sqrt();
            (x, mean - deviation, mean, mean + deviation)
        })
        .collect();

    let mut chart = ChartBuilder::on(area).build_cartesian_2d(0f64..10.0, -5f64..9.0)?;
    chart
        .configure_mesh()
        .disable_axes()
        .x_labels(X)
        .y_labels(Y)
        // Light lines between the labelled ones, still faint.
        .max_light_lines(1)
        .bold_line_style(s.palette.grid)
        .light_line_style(s.palette.grid.mix(0.45))
        .draw()?;
    chart
        .draw_series(
            raw.iter()
                .map(|&p| Circle::new(p, s.px(1.2), GREY_500.mix(0.7).filled())),
        )?
        .label("samples")
        .legend(move |(x, y)| Circle::new((x + 6, y), s.px(2.0), GREY_500.filled()));
    chart
        .draw_series(LineSeries::new(
            [(0.0, -2.0), (10.0, 6.0)],
            s.line(PINK_400, 1.5),
        ))?
        .label("y = 0.8x − 2")
        .legend(move |(x, y)| PathElement::new([(x, y), (x + 14, y)], s.line(PINK_400, 1.5)));
    chart
        .draw_series(bars.iter().map(|&(x, low, mean, high)| {
            ErrorBar::new_vertical(
                x,
                low,
                mean,
                high,
                s.line(INDIGO_400, 2.0).filled(),
                s.px(8.0),
            )
        }))?
        .label("mean ± σ")
        .legend(move |(x, y)| {
            ErrorBar::new_vertical(x + 6, y - 5, y, y + 5, INDIGO_400.filled(), s.px(8.0))
        });
    s.legend(&mut chart, SeriesLabelPosition::UpperLeft)?;

    let spec = chart.as_coord_spec();
    Ok(Ticks {
        x: x_ticks(spec, X, number),
        y: y_ticks(spec, Y, number),
        ..Ticks::default()
    })
}

/// A donut chart, straight on the drawing area, no coordinate system.
fn pie<DB: DrawingBackend>(area: &DrawingArea<DB, Shift>, s: Style) -> Outcome
where
    DB::ErrorType: 'static,
{
    let (width, height) = area.dim_in_pixel();
    let center = (width as i32 / 2, height as i32 / 2);
    let radius = f64::from(width.min(height)) * 0.32;
    let sizes = [38.0, 24.0, 17.0, 12.0, 9.0];
    let colors = [INDIGO_400, TEAL_400, PINK_400, AMBER_700, GREY_500];
    let labels = ["Rust", "Lua", "C++", "Python", "Other"];

    let mut pie = Pie::new(&center, &radius, &sizes, &colors, &labels);
    pie.start_angle(-90.0);
    pie.donut_hole(radius * 0.55);
    pie.label_offset(18.0);
    pie.label_style(s.font(11.0));
    pie.percentages(s.font(10.0).color(&WHITE));
    area.draw(&pie)?;
    area.draw(&Text::new(
        "lines of code",
        center,
        s.font(11.0)
            .color(&s.palette.dim)
            .pos(Pos::new(HPos::Center, VPos::Center)),
    ))?;
    Ok(())
}

/// A 3D surface coloured by a colormap, plus a helix; arrow keys orbit it.
/// Its axes live in 3D, so plotters keeps drawing them.
fn surface<DB: DrawingBackend>(area: &DrawingArea<DB, Shift>, s: Style, view: View) -> Outcome
where
    DB::ErrorType: 'static,
{
    let mut chart = ChartBuilder::on(area)
        .margin(s.px(8.0))
        .build_cartesian_3d(-3.0f64..3.0, -1.2f64..1.2, -3.0f64..3.0)?;
    chart.with_projection(|mut projection| {
        projection.yaw = view.yaw;
        projection.pitch = view.pitch;
        projection.scale = 0.8;
        projection.into_matrix()
    });
    chart
        .configure_axes()
        .light_grid_style(s.palette.grid.mix(0.5))
        .bold_grid_style(s.palette.grid)
        .axis_panel_style(s.palette.surface)
        .max_light_lines(2)
        .x_labels(4)
        .y_labels(3)
        .z_labels(4)
        .label_style(s.font(10.0).color(&s.palette.dim))
        .draw()?;

    let steps = |n: i32| (0..=n).map(move |i| -3.0 + 6.0 * i as f64 / n as f64);
    let color = |y: &f64| ViridisRGB.get_color_normalized(*y, -1.0, 1.0).filled();
    chart.draw_series(
        SurfaceSeries::xoz(steps(36), steps(36), |x, z| {
            let r = (x * x + z * z).sqrt();
            (r * 1.6).cos() * (-r / 3.0).exp()
        })
        .style_func(&color),
    )?;
    chart.draw_series(LineSeries::new(
        (0..=300).map(|i| {
            let t = i as f64 / 300.0 * 6.0 * PI;
            (2.6 * t.cos(), -1.1 + t / (6.0 * PI) * 2.2, 2.6 * t.sin())
        }),
        s.line(DEEPORANGE_400, 2.0),
    ))?;
    Ok(())
}

/// A matrix heatmap made of rectangles, labelled along the top.
fn heatmap<DB: DrawingBackend>(area: &DrawingArea<DB, Shift>, s: Style) -> Outcome<Ticks>
where
    DB::ErrorType: 'static,
{
    const COLUMNS: i32 = 24;
    const ROWS: i32 = 14;
    // plotters puts a range's start at the bottom; a descending y range
    // puts row 0 at the top, like a matrix.
    let chart = ChartBuilder::on(area).build_cartesian_2d(
        0i32..COLUMNS,
        std::ops::Range {
            start: ROWS,
            end: 0,
        },
    )?;
    let gap = s.px(1.0);
    chart.plotting_area().draw(&Rectangle::new(
        [(0, 0), (COLUMNS, ROWS)],
        s.palette.surface.filled(),
    ))?;
    for x in 0..COLUMNS {
        for y in 0..ROWS {
            let wave = ((x as f64 / 3.8).sin() + (y as f64 / 2.6).cos()) / 2.0;
            let value = wave * 0.5 + 0.5;
            let mut cell = Rectangle::new(
                [(x, y), (x + 1, y + 1)],
                ViridisRGB.get_color(value).filled(),
            );
            cell.set_margin(0, gap, 0, gap);
            chart.plotting_area().draw(&cell)?;
        }
    }

    // Ticks at cell centres, as a matrix is read.
    let spec = chart.as_coord_spec();
    let centre = |at: (i32, i32), other: (i32, i32)| ((at.0 + other.0) / 2, (at.1 + other.1) / 2);
    Ok(Ticks {
        x: (0..COLUMNS)
            .step_by(4)
            .map(|hour| Tick {
                at: centre(spec.translate(&(hour, 0)), spec.translate(&(hour + 1, 0))).0,
                label: format!("{hour}h"),
            })
            .collect(),
        y: (0..ROWS)
            .step_by(3)
            .map(|row| Tick {
                at: centre(spec.translate(&(0, row)), spec.translate(&(0, row + 1))).1,
                label: format!("r{row}"),
            })
            .collect(),
        ..Ticks::default()
    })
}

/// A bitmap computed by hand and handed to the backend as one image.
fn mandelbrot<DB: DrawingBackend>(area: &DrawingArea<DB, Shift>, _: Style) -> Outcome<Ticks>
where
    DB::ErrorType: 'static,
{
    const X: usize = 4;
    const Y: usize = 5;
    const LIMIT: u32 = 96;
    let chart = ChartBuilder::on(area).build_cartesian_2d(-2.2f64..0.8, -1.25f64..1.25)?;

    let (width, height) = chart.plotting_area().dim_in_pixel();
    let (x_range, y_range) = (chart.x_range(), chart.y_range());
    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    for py in 0..height {
        for px in 0..width {
            let cx = x_range.start + (x_range.end - x_range.start) * px as f64 / width as f64;
            let cy = y_range.end - (y_range.end - y_range.start) * py as f64 / height as f64;
            let (mut zx, mut zy, mut n) = (0.0f64, 0.0f64, 0);
            while n < LIMIT && zx * zx + zy * zy <= 4.0 {
                (zx, zy) = (zx * zx - zy * zy + cx, 2.0 * zx * zy + cy);
                n += 1;
            }
            let (r, g, b) = if n == LIMIT {
                (0x26, 0x23, 0x2B)
            } else {
                VulcanoHSL.get_color(n as f64 / LIMIT as f64).rgb()
            };
            rgb.extend_from_slice(&[r, g, b]);
        }
    }
    let image =
        BitMapElement::with_owned_buffer((x_range.start, y_range.end), (width, height), rgb)
            .ok_or("bitmap size mismatch")?;
    chart.plotting_area().draw(&image)?;

    let spec = chart.as_coord_spec();
    Ok(Ticks {
        x: x_ticks(spec, X, number),
        y: y_ticks(spec, Y, number),
        ..Ticks::default()
    })
}

/// A tiny deterministic generator, so every backend draws the same data.
struct Rng(u64);

impl Rng {
    fn uniform(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Box–Muller.
    fn normal(&mut self) -> f64 {
        let u = self.uniform().max(f64::MIN_POSITIVE);
        let v = self.uniform();
        (-2.0 * u.ln()).sqrt() * (2.0 * PI * v).cos()
    }
}
