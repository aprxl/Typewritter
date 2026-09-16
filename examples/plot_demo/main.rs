//! plotters on Atomos: twelve plots in a grid, drawn by Typewritter's own
//! renderer. The same plot code runs through two in-app paths, switched
//! with Tab:
//!
//! - **Vector** — `AtomosBackend` turns every plotters primitive into an
//!   Atomos draw call (paths, rectangles, polygons, Atomos-shaped text).
//! - **Bitmap** — plotters' `BitMapBackend` rasterizes into an RGB buffer
//!   at the window's physical resolution, uploaded as one Atomos image.
//!
//! `S` writes every plot through two more backends — SVG and PNG — to
//! `target/plot-demo/` (the bitmap plot as PNG only). Arrow keys orbit the 3D surface.
//!
//! `cargo run --example plot_demo`

mod atomos_backend;
mod plots;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use atomos_backend::AtomosBackend;
use plots::{Plot, Scale, View};
use plotters::prelude::{BitMapBackend, IntoDrawingArea, SVGBackend};
use typewritter::layout::Rect;
use typewritter::renderer::{
    Alignment, Color, FontParameters, HorizontalAlign, Layer, LayerInvalidation, Pixels, Renderer,
    Rounding, VerticalAlign,
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
const HEADER: f32 = 64.0;
const PADDING: f32 = 16.0;
const GAP: f32 = 12.0;
/// Size of the exported SVGs, in SVG units; PNGs are twice that.
const EXPORT_SIZE: (u32, u32) = (480, 340);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Path {
    Vector,
    Bitmap,
}

impl Path {
    fn describe(self) -> &'static str {
        match self {
            Path::Vector => "Vector — AtomosBackend → Atomos paths, shapes and text",
            Path::Bitmap => "Bitmap — BitMapBackend → RGB buffer → one Atomos image per plot",
        }
    }
}

struct Scene {
    window: Arc<Window>,
    renderer: Renderer,
    /// Background, cards and header text.
    chrome: Layer,
    /// One scissored layer per plot, so one plot can repaint alone.
    panels: Vec<Layer>,
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
        let panels = Plot::ALL
            .iter()
            .map(|_| renderer.new_layer_top(LayerInvalidation::Manual))
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
        for (index, (plot, layer)) in Plot::ALL.iter().zip(&self.panels).enumerate() {
            if !self.stale[index] {
                continue;
            }
            self.stale[index] = false;
            repainted += 1;
            let rect = cells[index];
            layer.clear();
            layer.set_clip_rect(Some((rect.position(), rect.size())));
            if let Err(error) = paint_plot(layer, rect, *plot, self.path, self.view) {
                eprintln!("{}: {error}", plot.slug());
                layer.draw_text(
                    &format!("{}: {error}", plot.slug()),
                    (rect.x + 12.0, rect.y + 12.0),
                    theme::warning(),
                    Alignment::TOP_LEFT,
                    theme::sans(),
                    FontParameters::new(12.0),
                );
            }
        }
        if repainted > 0 {
            self.status = format!(
                "painted {repainted} plot{} in {}",
                if repainted == 1 { "" } else { "s" },
                millis(started.elapsed()),
            );
        }
        self.paint_chrome(size.width, size.height, &cells);
    }

    fn paint_chrome(&self, width: f32, height: f32, cells: &[Rect]) {
        let layer = &self.chrome;
        layer.clear();
        layer.draw_rectangle(
            (0.0, 0.0),
            (width, height),
            theme::background(),
            Rounding::NONE,
        );
        for cell in cells {
            let card = cell.inset(-1.0);
            layer.draw_rectangle(
                card.position(),
                card.size(),
                theme::border(),
                Rounding::uniform(6.0),
            );
        }

        let left = |x: f32, y: f32| (PADDING + x, y);
        let mut title = FontParameters::new(17.0);
        title.weight = 17.0 * 0.018;
        layer.draw_text(
            "plotters on Atomos",
            left(0.0, 24.0),
            theme::ink(),
            Alignment {
                horizontal: HorizontalAlign::Left,
                vertical: VerticalAlign::Center,
            },
            theme::sans(),
            title,
        );
        layer.draw_text(
            self.path.describe(),
            left(0.0, 46.0),
            theme::accent(),
            Alignment {
                horizontal: HorizontalAlign::Left,
                vertical: VerticalAlign::Center,
            },
            theme::sans(),
            FontParameters::new(12.5),
        );
        let right = Alignment {
            horizontal: HorizontalAlign::Right,
            vertical: VerticalAlign::Center,
        };
        layer.draw_text(
            "Tab  switch path     ← → ↑ ↓  orbit 3D     S  export SVG + PNG",
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

/// Draws one plot into `rect` through the chosen path.
fn paint_plot(layer: &Layer, rect: Rect, plot: Plot, path: Path, view: View) -> plots::Outcome {
    let scale = layer.scale_factor();
    match path {
        Path::Vector => {
            let backend = AtomosBackend::new(layer.clone(), rect.position(), rect.size());
            let area = backend.into_drawing_area();
            plot.draw(&area, Scale(f64::from(scale)), view)?;
            area.present()?;
        }
        Path::Bitmap => {
            let width = (rect.width * scale).round() as u32;
            let height = (rect.height * scale).round() as u32;
            if width == 0 || height == 0 {
                return Ok(());
            }
            let mut rgb = vec![0u8; (width * height * 3) as usize];
            {
                let area =
                    BitMapBackend::with_buffer(&mut rgb, (width, height)).into_drawing_area();
                plot.draw(&area, Scale(f64::from(scale)), view)?;
                area.present()?;
            }
            layer.draw_image(
                Pixels::new(width, height, atomos_backend::rgb_to_rgba(&rgb)),
                rect.position(),
                rect.size(),
                Color::rgb(0xFF, 0xFF, 0xFF),
            );
        }
    }
    Ok(())
}

/// Every plot as an SVG (1x) and a PNG (2x), through plotters' own
/// file-writing backends.
fn export(view: View) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("plot-demo");
    std::fs::create_dir_all(&dir)?;
    let (width, height) = EXPORT_SIZE;
    for plot in Plot::ALL {
        // Without `plotters-svg/bitmap_encoder` (see Cargo.toml) the SVG
        // backend writes a bitmap as one <rect> per pixel.
        if plot != Plot::Mandelbrot {
            let svg = dir.join(format!("{}.svg", plot.slug()));
            let area = SVGBackend::new(&svg, (width, height)).into_drawing_area();
            plot.draw(&area, Scale(1.0), view)?;
            area.present()?;
        }

        let png = dir.join(format!("{}.png", plot.slug()));
        let area = BitMapBackend::new(&png, (width * 2, height * 2)).into_drawing_area();
        plot.draw(&area, Scale(2.0), view)?;
        area.present()?;
    }
    Ok(dir)
}

/// The plot cells below the header, `COLUMNS` across.
fn grid(width: f32, height: f32) -> Vec<Rect> {
    let rows = Plot::ALL.len().div_ceil(COLUMNS);
    let cell_width =
        ((width - 2.0 * PADDING - GAP * (COLUMNS - 1) as f32) / COLUMNS as f32).max(1.0);
    let cell_height =
        ((height - HEADER - PADDING - GAP * (rows - 1) as f32) / rows as f32).max(1.0);
    (0..Plot::ALL.len())
        .map(|index| {
            let (column, row) = ((index % COLUMNS) as f32, (index / COLUMNS) as f32);
            Rect::new(
                (PADDING + column * (cell_width + GAP)).round(),
                (HEADER + row * (cell_height + GAP)).round(),
                cell_width.floor(),
                cell_height.floor(),
            )
        })
        .collect()
}

fn millis(duration: Duration) -> String {
    format!("{:.1} ms", duration.as_secs_f64() * 1000.0)
}

fn main() -> Result<(), winit::error::EventLoopError> {
    theme::set(Theme::LIGHT);
    atomos_backend::register_fonts();
    EventLoop::new()?.run_app(&mut Demo::default())
}
