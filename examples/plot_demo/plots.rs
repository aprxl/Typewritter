//! Twelve plots, each written once against plotters' generic
//! `DrawingArea`, so the same code draws through any backend.

use std::f64::consts::PI;

use plotters::coord::Shift;
use plotters::data::Quartiles;
use plotters::prelude::*;
use plotters::style::full_palette::{
    AMBER_700, BLUEGREY_700, DEEPORANGE_400, GREY_100, GREY_200, GREY_400, GREY_600, GREY_800,
    INDIGO_400, LIGHTBLUE_400, PINK_400, PURPLE_400, TEAL_400, TEAL_600,
};
use plotters::style::text_anchor::{HPos, Pos, VPos};

pub type Outcome = Result<(), Box<dyn std::error::Error>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plot {
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

impl Plot {
    pub const ALL: [Plot; 12] = [
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

    pub fn draw<DB: DrawingBackend>(
        self,
        area: &DrawingArea<DB, Shift>,
        s: Scale,
        view: View,
    ) -> Outcome
    where
        DB::ErrorType: 'static,
    {
        area.fill(&WHITE)?;
        match self {
            Plot::Waves => waves(area, s),
            Plot::Scatter => scatter(area, s),
            Plot::Histogram => histogram(area, s),
            Plot::LogArea => log_area(area, s),
            Plot::TwoScales => two_scales(area, s),
            Plot::Boxplots => boxplots(area, s),
            Plot::Candles => candles(area, s),
            Plot::ErrorBars => error_bars(area, s),
            Plot::Pie => pie(area, s),
            Plot::Surface => surface(area, s, view),
            Plot::Heatmap => heatmap(area, s),
            Plot::Mandelbrot => mandelbrot(area, s),
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

/// plotters sizes everything in backend pixels; this turns the logical
/// sizes the plots are written in into those.
#[derive(Clone, Copy, Debug)]
pub struct Scale(pub f64);

impl Scale {
    fn px(self, logical: f64) -> u32 {
        (logical * self.0).round() as u32
    }

    fn font(self, logical: f64) -> TextStyle<'static> {
        ("sans-serif", logical * self.0)
            .into_font()
            .color(&GREY_800)
    }

    fn caption(self) -> TextStyle<'static> {
        ("sans-serif", 15.0 * self.0, FontStyle::Bold)
            .into_font()
            .color(&GREY_800)
    }

    fn label(self) -> TextStyle<'static> {
        self.font(11.0).color(&GREY_600)
    }

    fn line(self, color: RGBColor, width: f64) -> ShapeStyle {
        color.stroke_width(self.px(width))
    }
}

fn chart<'a, 'b, DB: DrawingBackend>(
    area: &'a DrawingArea<DB, Shift>,
    title: &str,
    s: Scale,
) -> ChartBuilder<'a, 'b, DB> {
    let mut builder = ChartBuilder::on(area);
    builder
        .caption(title, s.caption())
        .margin(s.px(10.0))
        .x_label_area_size(s.px(30.0))
        .y_label_area_size(s.px(42.0));
    builder
}

/// Line series, dashed and dotted variants, π tick labels, a legend.
fn waves<DB: DrawingBackend>(area: &DrawingArea<DB, Shift>, s: Scale) -> Outcome
where
    DB::ErrorType: 'static,
{
    let mut chart = chart(area, "Line styles · legend", s)
        .y_label_area_size(s.px(52.0))
        .build_cartesian_2d(0f64..4.0 * PI, -1.25f64..1.25)?;
    chart
        .configure_mesh()
        .x_labels(5)
        .x_label_formatter(&|x| format!("{:.1}π", x / PI))
        .x_desc("t")
        .y_desc("amplitude")
        .label_style(s.label())
        .axis_desc_style(s.font(12.0))
        .bold_line_style(GREY_200)
        .light_line_style(GREY_100)
        .draw()?;

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

    chart
        .configure_series_labels()
        .position(SeriesLabelPosition::UpperRight)
        .label_font(s.font(10.0))
        .background_style(WHITE.mix(0.85))
        .border_style(GREY_400)
        .margin(s.px(6.0))
        .legend_area_size(s.px(22.0))
        .draw()?;
    Ok(())
}

/// Three marker shapes, and an annotation composed from elements.
fn scatter<DB: DrawingBackend>(area: &DrawingArea<DB, Shift>, s: Scale) -> Outcome
where
    DB::ErrorType: 'static,
{
    let mut chart = chart(area, "Scatter · markers · annotations", s)
        .build_cartesian_2d(-4f64..6.0, -3f64..5.0)?;
    chart
        .configure_mesh()
        .x_labels(6)
        .label_style(s.label())
        // Only the bold grid, drawn faintly.
        .max_light_lines(0)
        .bold_line_style(GREY_200)
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
                + Circle::new((0, 0), ring as u32, s.line(GREY_800, 1.5))
                + Text::new(
                    format!("{name} ({:.1}, {:.1})", centroid.0, centroid.1),
                    (ring + 3, -ring - 4),
                    s.font(10.0),
                ),
        ))?;
    }

    chart
        .configure_series_labels()
        .position(SeriesLabelPosition::LowerLeft)
        .label_font(s.font(10.0))
        .background_style(WHITE.mix(0.85))
        .border_style(GREY_400)
        .draw()?;
    Ok(())
}

/// Segmented (categorical) axis and a histogram built from raw samples.
fn histogram<DB: DrawingBackend>(area: &DrawingArea<DB, Shift>, s: Scale) -> Outcome
where
    DB::ErrorType: 'static,
{
    let mut rng = Rng(42);
    let samples: Vec<u32> = (0..3000)
        .filter_map(|_| {
            let value = 10.0 + rng.normal() * 3.2;
            (0.0..20.0).contains(&value).then_some(value as u32)
        })
        .collect();

    let mut chart = chart(area, "Histogram · segmented axis", s)
        .build_cartesian_2d((0u32..19u32).into_segmented(), 0u32..500u32)?;
    chart
        .configure_mesh()
        .disable_x_mesh()
        .bold_line_style(GREY_200)
        .light_line_style(TRANSPARENT)
        .y_desc("count")
        .x_desc("bucket")
        .label_style(s.label())
        .axis_desc_style(s.font(12.0))
        .draw()?;
    chart.draw_series(
        Histogram::vertical(&chart)
            .style_func(|bucket, _| {
                let middle = matches!(bucket, SegmentValue::CenterOf(8..=11));
                if middle {
                    TEAL_600.filled()
                } else {
                    TEAL_400.mix(0.55).filled()
                }
            })
            .margin(s.px(2.0))
            .data(samples.iter().map(|&bucket| (bucket, 1))),
    )?;
    Ok(())
}

/// Logarithmic y axis, filled area series, y-only grid.
fn log_area<DB: DrawingBackend>(area: &DrawingArea<DB, Shift>, s: Scale) -> Outcome
where
    DB::ErrorType: 'static,
{
    let mut chart = chart(area, "Area series · log scale", s)
        .y_label_area_size(s.px(48.0))
        .build_cartesian_2d(1f64..64.0, (1f64..5000.0).log_scale())?;
    chart
        .configure_mesh()
        .disable_x_mesh()
        .bold_line_style(GREY_200)
        .light_line_style(GREY_100)
        .x_labels(5)
        .x_label_formatter(&|x| format!("{x:.0}"))
        .y_label_formatter(&|y| format!("{y:.0}"))
        .x_desc("n")
        .label_style(s.label())
        .axis_desc_style(s.font(12.0))
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
    chart
        .configure_series_labels()
        .position(SeriesLabelPosition::UpperLeft)
        .label_font(s.font(10.0))
        .border_style(GREY_400)
        .background_style(WHITE.mix(0.85))
        .draw()?;
    Ok(())
}

/// A secondary y axis, custom tick labels, bars drawn as rectangles.
fn two_scales<DB: DrawingBackend>(area: &DrawingArea<DB, Shift>, s: Scale) -> Outcome
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

    let mut chart = chart(area, "Two y axes · custom ticks", s)
        .right_y_label_area_size(s.px(42.0))
        .build_cartesian_2d(0f64..12.0, 0f64..25.0)?
        .set_secondary_coord(0f64..12.0, 0f64..120.0);
    chart
        .configure_mesh()
        .disable_x_mesh()
        .x_labels(12)
        .x_label_formatter(&|x| {
            MONTHS
                .get(x.floor() as usize)
                .map_or(String::new(), |m| m.to_string())
        })
        .x_label_offset(s.px(9.0) as i32)
        .y_desc("°C")
        .y_label_style(s.label().color(&DEEPORANGE_400))
        .x_label_style(s.label())
        .axis_desc_style(s.font(12.0).color(&DEEPORANGE_400))
        .bold_line_style(GREY_200)
        .light_line_style(TRANSPARENT)
        .draw()?;
    chart
        .configure_secondary_axes()
        .y_desc("rain (mm)")
        .label_style(s.label().color(&LIGHTBLUE_400))
        .axis_desc_style(s.font(12.0).color(&LIGHTBLUE_400))
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
    chart
        .configure_series_labels()
        .position(SeriesLabelPosition::UpperLeft)
        .label_font(s.font(10.0))
        .background_style(WHITE.mix(0.85))
        .border_style(GREY_400)
        .draw()?;
    Ok(())
}

/// Box-and-whisker plots over a categorical axis.
fn boxplots<DB: DrawingBackend>(area: &DrawingArea<DB, Shift>, s: Scale) -> Outcome
where
    DB::ErrorType: 'static,
{
    let mut rng = Rng(3);
    let groups = ["alpha", "beta", "gamma", "delta"];
    let data: Vec<Quartiles> = [(12.0, 2.0), (16.0, 4.5), (9.0, 1.2), (14.0, 3.0)]
        .iter()
        .map(|&(mean, spread)| {
            let values: Vec<f64> = (0..60).map(|_| mean + rng.normal() * spread).collect();
            Quartiles::new(&values)
        })
        .collect();

    let mut chart = chart(area, "Box plots · categorical x", s)
        .build_cartesian_2d(groups[..].into_segmented(), 0f32..28f32)?;
    chart
        .configure_mesh()
        .disable_x_mesh()
        .bold_line_style(GREY_200)
        .light_line_style(TRANSPARENT)
        .x_label_formatter(&|group| match group {
            SegmentValue::Exact(name) | SegmentValue::CenterOf(name) => name.to_string(),
            SegmentValue::Last => String::new(),
        })
        .label_style(s.label())
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
    Ok(())
}

/// Candlesticks with a moving average on top.
fn candles<DB: DrawingBackend>(area: &DrawingArea<DB, Shift>, s: Scale) -> Outcome
where
    DB::ErrorType: 'static,
{
    let mut rng = Rng(11);
    let mut price = 100.0;
    let days: Vec<(u32, f64, f64, f64, f64)> = (0..36)
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

    let mut chart = chart(area, "Candlesticks · moving average", s)
        .build_cartesian_2d(0u32..36u32, low..high)?;
    chart
        .configure_mesh()
        .light_line_style(TRANSPARENT)
        .bold_line_style(GREY_200)
        .x_labels(6)
        .x_label_formatter(&|d| format!("d{d}"))
        .y_label_formatter(&|p| format!("${p:.0}"))
        .label_style(s.label())
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
        .draw_series(LineSeries::new(average, s.line(BLUEGREY_700, 1.5)))?
        .label("5-day average")
        .legend(move |(x, y)| PathElement::new([(x, y), (x + 14, y)], s.line(BLUEGREY_700, 1.5)));
    chart
        .configure_series_labels()
        .position(SeriesLabelPosition::UpperLeft)
        .label_font(s.font(10.0))
        .background_style(WHITE.mix(0.85))
        .border_style(GREY_400)
        .draw()?;
    Ok(())
}

/// Noisy samples, down-sampled into error bars, with a fitted line.
fn error_bars<DB: DrawingBackend>(area: &DrawingArea<DB, Shift>, s: Scale) -> Outcome
where
    DB::ErrorType: 'static,
{
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

    let mut chart =
        chart(area, "Error bars · sparse ticks", s).build_cartesian_2d(0f64..10.0, -5f64..9.0)?;
    chart
        .configure_mesh()
        .x_labels(5)
        .y_labels(5)
        .max_light_lines(1)
        .bold_line_style(GREY_200)
        .light_line_style(GREY_100)
        .label_style(s.label())
        .draw()?;
    chart
        .draw_series(
            raw.iter()
                .map(|&p| Circle::new(p, s.px(1.2), GREY_400.filled())),
        )?
        .label("samples")
        .legend(move |(x, y)| Circle::new((x + 6, y), s.px(2.0), GREY_400.filled()));
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
    chart
        .configure_series_labels()
        .position(SeriesLabelPosition::UpperLeft)
        .label_font(s.font(10.0))
        .background_style(WHITE.mix(0.85))
        .border_style(GREY_400)
        .draw()?;
    Ok(())
}

/// A donut chart, placed on a titled drawing area, no coordinate system.
fn pie<DB: DrawingBackend>(area: &DrawingArea<DB, Shift>, s: Scale) -> Outcome
where
    DB::ErrorType: 'static,
{
    let area = area.titled("Pie · donut · no chart", s.caption())?;
    let (width, height) = area.dim_in_pixel();
    let center = (width as i32 / 2, height as i32 / 2);
    let radius = f64::from(width.min(height)) * 0.3;
    let sizes = [38.0, 24.0, 17.0, 12.0, 9.0];
    let colors = [INDIGO_400, TEAL_400, PINK_400, AMBER_700, GREY_400];
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
            .color(&GREY_600)
            .pos(Pos::new(HPos::Center, VPos::Center)),
    ))?;
    Ok(())
}

/// A 3D surface coloured by a colormap, plus a helix; arrow keys orbit it.
fn surface<DB: DrawingBackend>(area: &DrawingArea<DB, Shift>, s: Scale, view: View) -> Outcome
where
    DB::ErrorType: 'static,
{
    let mut chart = ChartBuilder::on(area)
        .caption("3D surface · colormap · ← → ↑ ↓", s.caption())
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
        .light_grid_style(BLACK.mix(0.06))
        .bold_grid_style(BLACK.mix(0.15))
        .max_light_lines(2)
        .x_labels(4)
        .y_labels(3)
        .z_labels(4)
        .label_style(s.label())
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
fn heatmap<DB: DrawingBackend>(area: &DrawingArea<DB, Shift>, s: Scale) -> Outcome
where
    DB::ErrorType: 'static,
{
    const COLUMNS: i32 = 24;
    const ROWS: i32 = 14;
    let mut chart = ChartBuilder::on(area)
        .caption("Heatmap · top axis · no mesh", s.caption())
        .margin(s.px(10.0))
        .top_x_label_area_size(s.px(22.0))
        .y_label_area_size(s.px(34.0))
        // plotters puts a range's start at the bottom; a descending y
        // range puts row 0 at the top, like a matrix.
        .build_cartesian_2d(
            0i32..COLUMNS,
            std::ops::Range {
                start: ROWS,
                end: 0,
            },
        )?;
    chart
        .configure_mesh()
        .disable_mesh()
        .x_labels(8)
        .y_labels(7)
        .x_label_formatter(&|hour| format!("{hour}h"))
        .y_label_formatter(&|row| format!("r{row}"))
        .label_style(s.label())
        .draw()?;
    let gap = s.px(1.0);
    chart.draw_series(
        (0..COLUMNS)
            .flat_map(|x| (0..ROWS).map(move |y| (x, y)))
            .map(|(x, y)| {
                let wave = ((x as f64 / 3.8).sin() + (y as f64 / 2.6).cos()) / 2.0;
                let value = wave * 0.5 + 0.5;
                let mut cell = Rectangle::new(
                    [(x, y), (x + 1, y + 1)],
                    MandelbrotHSL.get_color(value * 0.8).filled(),
                );
                cell.set_margin(0, gap, 0, gap);
                cell
            }),
    )?;
    Ok(())
}

/// A bitmap computed by hand and handed to the backend as one image.
fn mandelbrot<DB: DrawingBackend>(area: &DrawingArea<DB, Shift>, s: Scale) -> Outcome
where
    DB::ErrorType: 'static,
{
    let mut chart = chart(area, "Bitmap element · blit_bitmap", s)
        .build_cartesian_2d(-2.2f64..0.8, -1.25f64..1.25)?;
    chart
        .configure_mesh()
        .disable_mesh()
        .x_labels(4)
        .label_style(s.label())
        .draw()?;

    let (width, height) = chart.plotting_area().dim_in_pixel();
    let (x_range, y_range) = (chart.x_range(), chart.y_range());
    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    const LIMIT: u32 = 96;
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
                GREY_800.rgb()
            } else {
                VulcanoHSL.get_color(n as f64 / LIMIT as f64).rgb()
            };
            rgb.extend_from_slice(&[r, g, b]);
        }
    }
    let image =
        BitMapElement::with_owned_buffer((x_range.start, y_range.end), (width, height), rgb)
            .ok_or("bitmap size mismatch")?;
    chart.draw_series(std::iter::once(image))?;
    Ok(())
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
