//! A plotters [`DrawingBackend`] that turns every plotters primitive into an
//! Atomos draw call on a [`Layer`] — lines and outlines become stroked SVG
//! paths, fills become rectangles, circles and polygons, and text is shaped
//! by Atomos in Typewritter's own fonts.
//!
//! plotters addresses whole pixels (`BackendCoord` is `(i32, i32)`), so the
//! backend reports its size in *physical* pixels and divides back down to
//! logical ones on the way out. That keeps curves smooth on a HiDPI screen;
//! the price is that plot code sizes its fonts and margins in physical
//! pixels too (see `plots::Scale`).

use std::fmt;

use plotters_backend::{
    BackendColor, BackendCoord, BackendStyle, BackendTextStyle, DrawingBackend, DrawingErrorKind,
    FontFamily, FontStyle, FontTransform,
    text_anchor::{HPos, VPos},
};
use typewritter::renderer::{
    Alignment, Color, Font, FontParameters, HorizontalAlign, Layer, LineJoin, PathPaint, Pixels,
    Rounding, Stroke, VerticalAlign,
};
use typewritter::theme;

/// An SVG path Atomos refused. Only reachable through a bug here, since
/// every path is generated from integer coordinates.
#[derive(Debug)]
pub struct PathError(String);

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Atomos rejected a generated path: {}", self.0)
    }
}

impl std::error::Error for PathError {}

type Result = std::result::Result<(), DrawingErrorKind<PathError>>;

pub struct AtomosBackend {
    layer: Layer,
    /// Logical top-left of the plot on the layer.
    origin: (f32, f32),
    /// Physical pixels, what plotters sees.
    size: (u32, u32),
    scale: f32,
}

impl AtomosBackend {
    pub fn new(layer: Layer, origin: (f32, f32), size: (f32, f32)) -> Self {
        let scale = layer.scale_factor();
        Self {
            layer,
            origin,
            size: (
                (size.0 * scale).round() as u32,
                (size.1 * scale).round() as u32,
            ),
            scale,
        }
    }

    /// A physical plotters coordinate as a logical layer position. `nudge`
    /// moves odd-width strokes onto pixel centres so a 1px line covers one
    /// row of pixels instead of blurring across two.
    fn point(&self, (x, y): BackendCoord, nudge: f32) -> (f32, f32) {
        (
            self.origin.0 + (x as f32 + nudge) / self.scale,
            self.origin.1 + (y as f32 + nudge) / self.scale,
        )
    }

    fn logical(&self, pixels: f32) -> f32 {
        pixels / self.scale
    }

    fn stroke<S: BackendStyle>(
        &self,
        points: impl IntoIterator<Item = BackendCoord>,
        close: bool,
        style: &S,
    ) -> Result {
        let width = style.stroke_width();
        if width == 0 || style.color().alpha == 0.0 {
            return Ok(());
        }
        let nudge = if width % 2 == 1 { 0.5 } else { 0.0 };
        let mut d = String::new();
        for (index, point) in points.into_iter().enumerate() {
            let (x, y) = self.point(point, nudge);
            let command = if index == 0 { 'M' } else { 'L' };
            d.push_str(&format!("{command}{x} {y}"));
        }
        if d.is_empty() {
            return Ok(());
        }
        if close {
            d.push('Z');
        }
        self.draw(&d, stroke(style.color(), self.logical(width as f32)))
    }

    fn draw(&self, d: &str, paint: PathPaint) -> Result {
        self.layer
            .draw_path(d, (0.0, 0.0), paint)
            .map_err(|error| DrawingErrorKind::DrawingError(PathError(format!("{error:?}"))))
    }

    /// Text plotters wants rotated: Atomos text has no rotation, so let
    /// plotters rasterize the glyphs itself, turn the bitmap, and upload it
    /// as an image. Mirrors plotters' own default `draw_text`.
    fn draw_rotated_text<T: BackendTextStyle>(
        &self,
        text: &str,
        style: &T,
        pos: BackendCoord,
    ) -> Result {
        let ((min_x, min_y), (max_x, max_y)) = style
            .layout_box(text)
            .map_err(|error| DrawingErrorKind::FontError(Box::new(error)))?;
        let (width, height) = (max_x - min_x, max_y - min_y);
        if width <= 0 || height <= 0 {
            return Ok(());
        }
        let dx = match style.anchor().h_pos {
            HPos::Left => 0,
            HPos::Right => -width,
            HPos::Center => -width / 2,
        };
        let dy = match style.anchor().v_pos {
            VPos::Top => 0,
            VPos::Center => -height / 2,
            VPos::Bottom => -height,
        };
        let transform = style.transform();
        let corners = [
            transform.transform(dx, dy),
            transform.transform(dx + width - 1, dy),
            transform.transform(dx, dy + height - 1),
            transform.transform(dx + width - 1, dy + height - 1),
        ];
        let left = corners.iter().map(|c| c.0).min().unwrap_or(0);
        let top = corners.iter().map(|c| c.1).min().unwrap_or(0);
        let (out_w, out_h) = match transform {
            FontTransform::Rotate90 | FontTransform::Rotate270 => (height, width),
            _ => (width, height),
        };

        let mut rgba = vec![0u8; (out_w * out_h * 4) as usize];
        let drawn = style.draw(text, (0, 0), |x, y, color| {
            let (tx, ty) = transform.transform(x - min_x + dx, y - min_y + dy);
            let (ox, oy) = (tx - left, ty - top);
            if (0..out_w).contains(&ox) && (0..out_h).contains(&oy) {
                let at = ((oy * out_w + ox) * 4) as usize;
                let (r, g, b) = color.rgb;
                rgba[at..at + 4].copy_from_slice(&[r, g, b, alpha(color.alpha)]);
            }
            Ok::<(), DrawingErrorKind<PathError>>(())
        });
        drawn.map_err(|error| DrawingErrorKind::FontError(Box::new(error)))??;

        self.layer.draw_image(
            Pixels::new(out_w as u32, out_h as u32, rgba),
            self.point((pos.0 + left, pos.1 + top), 0.0),
            (self.logical(out_w as f32), self.logical(out_h as f32)),
            Color::rgb(0xFF, 0xFF, 0xFF),
        );
        Ok(())
    }

    /// The Atomos font and parameters for a plotters text style.
    fn font<T: BackendTextStyle>(&self, style: &T) -> (Font, FontParameters) {
        let font = match style.family() {
            FontFamily::Monospace => theme::mono(),
            _ => theme::sans(),
        };
        // plotters sizes text by its line height (ascent + descent), Atomos
        // by its em square; convert so both paths set the same text size.
        let em = self.logical(style.size() as f32) / line_height_per_em(&font);
        let mut parameters = FontParameters::new(em);
        if matches!(style.style(), FontStyle::Bold) {
            parameters.weight = em * BOLD_WEIGHT;
        }
        (font, parameters)
    }
}

/// Faux-bold expansion as a fraction of size, matching `theme::TextStyle::bold`.
const BOLD_WEIGHT: f32 = 0.018;

impl DrawingBackend for AtomosBackend {
    type ErrorType = PathError;

    fn get_size(&self) -> (u32, u32) {
        self.size
    }

    fn ensure_prepared(&mut self) -> Result {
        Ok(())
    }

    fn present(&mut self) -> Result {
        Ok(())
    }

    fn draw_pixel(&mut self, point: BackendCoord, color: BackendColor) -> Result {
        let pixel = self.logical(1.0);
        self.layer.draw_rectangle(
            self.point(point, 0.0),
            (pixel, pixel),
            to_color(color),
            Rounding::NONE,
        );
        Ok(())
    }

    fn draw_line<S: BackendStyle>(
        &mut self,
        from: BackendCoord,
        to: BackendCoord,
        style: &S,
    ) -> Result {
        self.stroke([from, to], false, style)
    }

    fn draw_rect<S: BackendStyle>(
        &mut self,
        upper_left: BackendCoord,
        bottom_right: BackendCoord,
        style: &S,
        fill: bool,
    ) -> Result {
        if !fill {
            let (left, top) = upper_left;
            let (right, bottom) = bottom_right;
            let corners = [(left, top), (right, top), (right, bottom), (left, bottom)];
            return self.stroke(corners, true, style);
        }
        // plotters' rectangles include their bottom-right pixel.
        let top_left = self.point(upper_left, 0.0);
        let bottom_right = self.point((bottom_right.0 + 1, bottom_right.1 + 1), 0.0);
        self.layer.draw_rectangle(
            top_left,
            (bottom_right.0 - top_left.0, bottom_right.1 - top_left.1),
            to_color(style.color()),
            Rounding::NONE,
        );
        Ok(())
    }

    fn draw_path<S: BackendStyle, I: IntoIterator<Item = BackendCoord>>(
        &mut self,
        path: I,
        style: &S,
    ) -> Result {
        self.stroke(path, false, style)
    }

    fn draw_circle<S: BackendStyle>(
        &mut self,
        center: BackendCoord,
        radius: u32,
        style: &S,
        fill: bool,
    ) -> Result {
        let radius = self.logical(radius as f32);
        if fill {
            let center = self.point(center, 0.5);
            self.layer
                .draw_circle(center, radius, to_color(style.color()));
            return Ok(());
        }
        if style.stroke_width() == 0 {
            return Ok(());
        }
        let (x, y) = self.point(center, 0.5);
        let d = format!(
            "M{} {y}A{radius} {radius} 0 1 0 {} {y}A{radius} {radius} 0 1 0 {} {y}Z",
            x - radius,
            x + radius,
            x - radius,
        );
        let width = self.logical(style.stroke_width() as f32);
        self.draw(&d, stroke(style.color(), width))
    }

    fn fill_polygon<S: BackendStyle, I: IntoIterator<Item = BackendCoord>>(
        &mut self,
        vert: I,
        style: &S,
    ) -> Result {
        let vertices: Vec<_> = vert
            .into_iter()
            .map(|point| self.point(point, 0.0))
            .collect();
        if vertices.len() >= 3 {
            self.layer.draw_polygon(&vertices, to_color(style.color()));
        }
        Ok(())
    }

    fn draw_text<T: BackendTextStyle>(
        &mut self,
        text: &str,
        style: &T,
        pos: BackendCoord,
    ) -> Result {
        if style.color().alpha == 0.0 {
            return Ok(());
        }
        if !matches!(style.transform(), FontTransform::None) {
            return self.draw_rotated_text(text, style, pos);
        }
        let (font, parameters) = self.font(style);
        let anchor = style.anchor();
        let alignment = Alignment {
            horizontal: match anchor.h_pos {
                HPos::Left => HorizontalAlign::Left,
                HPos::Center => HorizontalAlign::Center,
                HPos::Right => HorizontalAlign::Right,
            },
            vertical: match anchor.v_pos {
                VPos::Top => VerticalAlign::Top,
                VPos::Center => VerticalAlign::Center,
                VPos::Bottom => VerticalAlign::Bottom,
            },
        };
        self.layer.draw_text(
            text,
            self.point(pos, 0.0),
            to_color(style.color()),
            alignment,
            font,
            parameters,
        );
        Ok(())
    }

    fn estimate_text_size<T: BackendTextStyle>(
        &self,
        text: &str,
        style: &T,
    ) -> std::result::Result<(u32, u32), DrawingErrorKind<PathError>> {
        if !matches!(style.transform(), FontTransform::None) {
            // Measured the way `draw_rotated_text` will draw it.
            let ((x0, y0), (x1, y1)) = style
                .layout_box(text)
                .map_err(|error| DrawingErrorKind::FontError(Box::new(error)))?;
            return Ok(((x1 - x0) as u32, (y1 - y0) as u32));
        }
        let (font, parameters) = self.font(style);
        let (width, height) = self.layer.get_text_size(text, &font, &parameters);
        Ok((
            (width * self.scale).ceil() as u32,
            (height * self.scale).ceil() as u32,
        ))
    }

    fn blit_bitmap(
        &mut self,
        pos: BackendCoord,
        (width, height): (u32, u32),
        src: &[u8],
    ) -> Result {
        let rgba = rgb_to_rgba(src);
        self.layer.draw_image(
            Pixels::new(width, height, rgba),
            self.point(pos, 0.0),
            (self.logical(width as f32), self.logical(height as f32)),
            Color::rgb(0xFF, 0xFF, 0xFF),
        );
        Ok(())
    }
}

/// Registers the app's embedded fonts under the family names plotters asks
/// for, so its own rasterizer (bitmap, SVG, rotated text) sets the same
/// faces Atomos does.
pub fn register_fonts() {
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

/// A font's ascent + descent in ems.
fn line_height_per_em(font: &Font) -> f32 {
    let Font::Bytes(bytes) = font else {
        return 1.0;
    };
    swash::FontRef::from_index(bytes, 0)
        .map(|face| {
            let metrics = face.metrics(&[]);
            (metrics.ascent + metrics.descent.abs()) / metrics.units_per_em as f32
        })
        .unwrap_or(1.0)
}

pub fn rgb_to_rgba(rgb: &[u8]) -> Vec<u8> {
    rgb.as_chunks::<3>()
        .0
        .iter()
        .flat_map(|&[r, g, b]| [r, g, b, 0xFF])
        .collect()
}

fn stroke(color: BackendColor, width: f32) -> PathPaint {
    PathPaint::Stroke(Stroke {
        join: LineJoin::Round,
        ..Stroke::new(to_color(color), width)
    })
}

fn to_color(color: BackendColor) -> Color {
    let (r, g, b) = color.rgb;
    Color::rgba(r, g, b, alpha(color.alpha))
}

fn alpha(alpha: f64) -> u8 {
    (alpha.clamp(0.0, 1.0) * 255.0).round() as u8
}
