//! plotters on Atomos: twelve plots in a grid, drawn by Typewritter's own
//! renderer. plotters draws what is inside each plot; the rounded
//! container, tick marks and axis labels around it are Typewritter's
//! (`frame.rs`), in the current theme. The same plot code runs through two
//! in-app paths, switched with Tab:
//!
//! - **Vector** — the app's own `CanvasBackend` (`typewritter::plot`),
//!   the one graph widgets draw through, turns every plotters primitive
//!   into a canvas call. What a canvas cannot draw — the Mandelbrot plot's
//!   bitmap — is refused there, as it would be in a note.
//! - **Bitmap** — plotters' `BitMapBackend` rasterizes into an RGB buffer
//!   at the window's physical resolution, uploaded as one Atomos image.
//!
//! `S` writes what plotters draws — the insides, without the frame —
//! through two more backends, SVG and PNG, to `target/plot-demo/` (the
//! bitmap plot as PNG only). `D` switches light and dark. Arrow keys orbit
//! the 3D surface.
//!
//! `cargo run --example plot_demo`

mod frame;
mod plots;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use plots::{Palette, Plot, Style, Ticks, View};
use plotters::prelude::{BitMapBackend, IntoDrawingArea, RGBColor, SVGBackend};
use plotters_backend::FontStyle;
use typewritter::layout::Rect;
use typewritter::plot::{CanvasBackend, SUBPIXELS};
use typewritter::renderer::{
    Alignment, Color, Font, FontParameters, HorizontalAlign, Layer, LayerInvalidation, Pixels,
    Renderer, Rounding, VerticalAlign,
};
use typewritter::theme::{self, Theme};
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::{ElementState, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{Key, NamedKey},
    window::{Window, WindowId},
};

const COLUMNS: usize = 4;
const HEADER: f32 = 68.0;
const PADDING: f32 = 20.0;
const GAP: f32 = 22.0;
/// Size of the exported plot insides, in SVG units; PNGs are twice that.
const EXPORT_SIZE: (u32, u32) = (480, 340);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Path {
    Vector,
    Bitmap,
}

impl Path {
    fn describe(self) -> &'static str {
        match self {
            Path::Vector => "Vector — CanvasBackend → canvas paths, shapes and text",
            Path::Bitmap => "Bitmap — BitMapBackend → RGB buffer → one Atomos image per plot",
        }
    }
}

/// One plot's pair of layers — see `frame.rs`.
struct Panel {
    data: Layer,
    frame: Layer,
}

struct Scene {
    window: Arc<Window>,
    renderer: Renderer,
    /// Background and header text.
    chrome: Layer,
    panels: Vec<Panel>,
    stale: Vec<bool>,
    path: Path,
    view: View,
    status: String,
}

#[derive(Default)]
struct Demo {
    scene: Option<Scene>,
}

impl ApplicationHandler for Demo {
    fn resumed(&mut self, events: &ActiveEventLoop) {
        if self.scene.is_some() {
            return;
        }
        let window = Arc::new(
            events
                .create_window(
                    Window::default_attributes()
                        .with_title("Typewritter · plotters on Atomos")
                        .with_inner_size(LogicalSize::new(1560, 980)),
                )
                .expect("demo window"),
        );
        let mut renderer = pollster::block_on(Renderer::new(window.clone()));
        let chrome = renderer.new_layer_bottom(LayerInvalidation::Manual);
        // Every data layer first, then every frame layer, so frames stack
        // above all data.
        let data: Vec<Layer> = Plot::ALL
            .iter()
            .map(|_| renderer.new_layer_top(LayerInvalidation::Manual))
            .collect();
        let panels = data
            .into_iter()
            .map(|data| Panel {
                data,
                frame: renderer.new_layer_top(LayerInvalidation::Manual),
            })
            .collect();
        self.scene = Some(Scene {
            window,
            renderer,
            chrome,
            panels,
            stale: vec![true; Plot::ALL.len()],
            path: Path::Vector,
            view: View::default(),
            status: String::new(),
        });
    }

    fn window_event(&mut self, events: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(scene) = &mut self.scene else {
            return;
        };
        scene.renderer.handle_event(&event);
        match event {
            WindowEvent::CloseRequested => events.exit(),
            WindowEvent::Resized(size) => {
                scene.renderer.resize(size.width, size.height);
                scene.invalidate_all();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                scene.renderer.set_scale_factor(scale_factor);
                scene.invalidate_all();
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                scene.key(&event.logical_key);
            }
            WindowEvent::RedrawRequested => {
                scene.paint();
                scene.renderer.render().expect("render plot demo");
                if scene.renderer.has_pending_resize() {
                    scene.window.request_redraw();
                }
            }
            _ => {}
        }
    }
}

impl Scene {
    fn invalidate_all(&mut self) {
        self.stale.fill(true);
        self.window.request_redraw();
    }

    fn key(&mut self, key: &Key) {
        const STEP: f64 = 0.12;
        let surface = Plot::ALL.iter().position(|&p| p == Plot::Surface);
        match key {
            Key::Named(NamedKey::Tab) => {
                self.path = match self.path {
                    Path::Vector => Path::Bitmap,
                    Path::Bitmap => Path::Vector,
                };
                self.invalidate_all();
            }
            Key::Named(
                arrow @ (NamedKey::ArrowLeft
                | NamedKey::ArrowRight
                | NamedKey::ArrowUp
                | NamedKey::ArrowDown),
            ) => {
                match arrow {
                    NamedKey::ArrowLeft => self.view.yaw -= STEP,
                    NamedKey::ArrowRight => self.view.yaw += STEP,
                    NamedKey::ArrowUp => self.view.pitch = (self.view.pitch + STEP).min(1.5),
                    _ => self.view.pitch = (self.view.pitch - STEP).max(-1.5),
                }
                if let Some(index) = surface {
                    self.stale[index] = true;
                }
                self.window.request_redraw();
            }
            Key::Character(c) if c.eq_ignore_ascii_case("d") => {
                theme::set(theme::counterpart());
                self.invalidate_all();
            }
            Key::Character(c) if c.eq_ignore_ascii_case("s") => {
                let started = Instant::now();
                self.status = match export(self.view) {
                    Ok(dir) => {
                        println!("exported to {}", dir.display());
                        format!(
                            "exported {} files to target/plot-demo in {}",
                            Plot::ALL.len() * 2 - 1,
                            millis(started.elapsed()),
                        )
                    }
                    Err(error) => format!("export failed: {error}"),
                };
                self.window.request_redraw();
            }
            _ => {}
        }
    }

    fn paint(&mut self) {
        let size = self
            .window
            .inner_size()
            .to_logical::<f32>(self.window.scale_factor());
        let cells = grid(size.width, size.height);

        let mut repainted = 0;
        let started = Instant::now();
        for (index, (plot, panel)) in Plot::ALL.iter().zip(&self.panels).enumerate() {
            if !self.stale[index] {
                continue;
            }
            self.stale[index] = false;
            repainted += 1;
            let cell = cells[index];
            let spec = plot.frame();
            let inside = frame::container(cell, &spec);
            panel.data.clear();
            panel.frame.clear();
            frame::clip(&panel.data, inside);
            match paint_plot(&panel.data, inside, *plot, self.path, self.view) {
                Ok(ticks) => frame::draw(&panel.frame, cell, inside, &spec, &ticks),
                Err(error) => {
                    eprintln!("{}: {error}", plot.slug());
                    panel.frame.draw_text(
                        &format!("{}: {error}", plot.slug()),
                        (inside.x + 12.0, inside.y + 12.0),
                        theme::warning(),
                        Alignment::TOP_LEFT,
                        theme::sans(),
                        FontParameters::new(12.0),
                    );
                }
            }
        }
        if repainted > 0 {
            self.status = format!(
                "painted {repainted} plot{} in {}",
                if repainted == 1 { "" } else { "s" },
                millis(started.elapsed()),
            );
        }
        self.paint_chrome(size.width, size.height);
    }

    fn paint_chrome(&self, width: f32, height: f32) {
        let layer = &self.chrome;
        layer.clear();
        layer.draw_rectangle(
            (0.0, 0.0),
            (width, height),
            theme::background(),
            Rounding::NONE,
        );

        let left = Alignment {
            horizontal: HorizontalAlign::Left,
            vertical: VerticalAlign::Center,
        };
        let right = Alignment {
            horizontal: HorizontalAlign::Right,
            vertical: VerticalAlign::Center,
        };
        let mut title = FontParameters::new(17.0);
        title.weight = 17.0 * 0.018;
        layer.draw_text(
            "plotters on Atomos",
            (PADDING, 24.0),
            theme::ink(),
            left,
            theme::sans(),
            title,
        );
        layer.draw_text(
            self.path.describe(),
            (PADDING, 46.0),
            theme::accent(),
            left,
            theme::sans(),
            FontParameters::new(12.5),
        );
        layer.draw_text(
            "Tab  switch path   D  theme   ← → ↑ ↓  orbit 3D   S  export SVG + PNG",
            (width - PADDING, 24.0),
            theme::dim(),
            right,
            theme::mono(),
            FontParameters::new(12.0),
        );
        layer.draw_text(
            &self.status,
            (width - PADDING, 46.0),
            theme::dim(),
            right,
            theme::mono(),
            FontParameters::new(12.0),
        );
    }
}

/// Draws one plot's inside into `rect` through the chosen path.
fn paint_plot(
    layer: &Layer,
    rect: Rect,
    plot: Plot,
    path: Path,
    view: View,
) -> plots::Outcome<Ticks> {
    let scale = layer.scale_factor();
    match path {
        Path::Vector => {
            let style = Style {
                scale: f64::from(SUBPIXELS),
                palette: palette(),
            };
            let mut canvas = layer;
            let area = CanvasBackend::new(&mut canvas, rect).into_drawing_area();
            let ticks = plot.draw(&area, style, view)?;
            area.present()?;
            Ok(ticks)
        }
        Path::Bitmap => {
            let style = Style {
                scale: f64::from(scale),
                palette: palette(),
            };
            let width = (rect.width * scale).round() as u32;
            let height = (rect.height * scale).round() as u32;
            if width == 0 || height == 0 {
                return Ok(Ticks::default());
            }
            let mut rgb = vec![0u8; (width * height * 3) as usize];
            let ticks = {
                let area =
                    BitMapBackend::with_buffer(&mut rgb, (width, height)).into_drawing_area();
                let ticks = plot.draw(&area, style, view)?;
                area.present()?;
                ticks
            };
            layer.draw_image(
                Pixels::new(width, height, rgb_to_rgba(&rgb)),
                rect.position(),
                rect.size(),
                Color::rgb(0xFF, 0xFF, 0xFF),
            );
            Ok(ticks)
        }
    }
}

/// The current theme, as the colours plotters draws the insides with.
fn palette() -> Palette {
    let surface = rgb(frame::surface());
    let border = rgb(theme::border());
    Palette {
        surface,
        // Halfway from the border to the surface: grid lines sit under
        // the data without competing with the container's edge.
        grid: mix(border, surface, 0.5),
        border,
        ink: rgb(theme::ink()),
        dim: rgb(theme::dim()),
    }
}

fn rgb(color: Color) -> RGBColor {
    match color {
        Color::Solid([r, g, b, _]) => RGBColor(r, g, b),
        _ => unreachable!("theme colours are solid"),
    }
}

fn mix(a: RGBColor, b: RGBColor, amount: f64) -> RGBColor {
    let channel =
        |a: u8, b: u8| (f64::from(a) + (f64::from(b) - f64::from(a)) * amount).round() as u8;
    RGBColor(channel(a.0, b.0), channel(a.1, b.1), channel(a.2, b.2))
}

/// Every plot's inside as an SVG (1x) and a PNG (2x), through plotters'
/// own file-writing backends.
fn export(view: View) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("plot-demo");
    std::fs::create_dir_all(&dir)?;
    let (width, height) = EXPORT_SIZE;
    let style = |scale| Style {
        scale,
        palette: palette(),
    };
    for plot in Plot::ALL {
        // Without `plotters-svg/bitmap_encoder` (see Cargo.toml) the SVG
        // backend writes a bitmap as one <rect> per pixel.
        if plot != Plot::Mandelbrot {
            let svg = dir.join(format!("{}.svg", plot.slug()));
            let area = SVGBackend::new(&svg, (width, height)).into_drawing_area();
            plot.draw(&area, style(1.0), view)?;
            area.present()?;
        }

        let png = dir.join(format!("{}.png", plot.slug()));
        let area = BitMapBackend::new(&png, (width * 2, height * 2)).into_drawing_area();
        plot.draw(&area, style(2.0), view)?;
        area.present()?;
    }
    Ok(dir)
}

/// The plot cells below the header, `COLUMNS` across. Plots are packed in
/// order into the first free place their span fits, row by row.
fn grid(width: f32, height: f32) -> Vec<Rect> {
    let mut taken: Vec<[bool; COLUMNS]> = Vec::new();
    let mut places = Vec::new();
    for plot in Plot::ALL {
        let (span_x, span_y) = span(plot);
        let fits = |taken: &Vec<[bool; COLUMNS]>, column: usize, row: usize| {
            column + span_x <= COLUMNS
                && (row..row + span_y).all(|r| {
                    (column..column + span_x).all(|c| !taken.get(r).is_some_and(|cells| cells[c]))
                })
        };
        let (column, row) = (0..)
            .flat_map(|row| (0..COLUMNS).map(move |column| (column, row)))
            .find(|&(column, row)| fits(&taken, column, row))
            .expect("an empty row always fits");
        while taken.len() < row + span_y {
            taken.push([false; COLUMNS]);
        }
        for cells in &mut taken[row..row + span_y] {
            cells[column..column + span_x].fill(true);
        }
        places.push((column, row, span_x, span_y));
    }

    let rows = taken.len();
    let cell_width =
        ((width - 2.0 * PADDING - GAP * (COLUMNS - 1) as f32) / COLUMNS as f32).max(1.0);
    let cell_height =
        ((height - HEADER - PADDING - GAP * (rows - 1) as f32) / rows as f32).max(1.0);
    places
        .into_iter()
        .map(|(column, row, span_x, span_y)| {
            let (column, row) = (column as f32, row as f32);
            let (span_x, span_y) = (span_x as f32, span_y as f32);
            Rect::new(
                (PADDING + column * (cell_width + GAP)).round(),
                (HEADER + row * (cell_height + GAP)).round(),
                (span_x * cell_width + (span_x - 1.0) * GAP).floor(),
                (span_y * cell_height + (span_y - 1.0) * GAP).floor(),
            )
        })
        .collect()
}

/// How many grid cells a plot covers, across and down.
fn span(plot: Plot) -> (usize, usize) {
    match plot {
        Plot::TypedMath => (2, 2),
        _ => (1, 1),
    }
}

fn millis(duration: Duration) -> String {
    format!("{:.1} ms", duration.as_secs_f64() * 1000.0)
}

fn main() -> Result<(), winit::error::EventLoopError> {
    theme::set(if std::env::args().any(|arg| arg == "--dark") {
        Theme::DARK
    } else {
        Theme::LIGHT
    });
    register_fonts();
    EventLoop::new()?.run_app(&mut Demo::default())
}

/// Registers the app's embedded fonts under the family names plotters asks
/// for, so its own rasterizer (bitmap and SVG) sets the faces the app does.
fn register_fonts() {
    for (family, font) in [
        ("sans-serif", theme::sans()),
        ("serif", theme::sans()),
        ("monospace", theme::mono()),
    ] {
        let Font::Bytes(bytes) = font else {
            unreachable!("theme fonts are embedded");
        };
        for style in [FontStyle::Normal, FontStyle::Bold] {
            plotters::style::register_font(family, style, bytes)
                .unwrap_or_else(|_| panic!("{family} is not a valid font"));
        }
    }
}

fn rgb_to_rgba(rgb: &[u8]) -> Vec<u8> {
    rgb.as_chunks::<3>()
        .0
        .iter()
        .flat_map(|&[r, g, b]| [r, g, b, 0xFF])
        .collect()
}
