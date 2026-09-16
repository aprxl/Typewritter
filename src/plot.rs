//! plotters on a [`Canvas`]: what a plot draws inside its frame reaches
//! the screen and the page through the same four calls as the rest of the
//! document.
//!
//! plotters addresses whole backend pixels. A document is drawn at
//! whatever density its surface has, so the backend works in
//! [`SUBPIXELS`]-per-logical-pixel units and divides back down on the way
//! out; a sampled curve then stays smooth at any zoom or DPI.
//!
//! Only what plots draw inside their frames is supported: lines, paths,
//! rectangles, circles and polygons. Text is drawn upright (a canvas has no
//! rotation for it), and bitmaps are refused.

use std::fmt;

use plotters_backend::{
    BackendColor, BackendCoord, BackendStyle, BackendTextStyle, DrawingBackend, DrawingErrorKind,
    FontFamily, FontStyle,
    text_anchor::{HPos, VPos},
};

use crate::canvas::Canvas;
use crate::layout::Rect;
use crate::renderer::{
    Alignment, Color, HorizontalAlign, LineJoin, PathPaint, Rounding, Stroke, VerticalAlign,
};
use crate::theme::TextStyle;

/// Backend units per logical pixel.
pub const SUBPIXELS: f32 = 4.0;

/// Something a canvas cannot draw.
#[derive(Debug)]
pub struct Unsupported(&'static str);

impl fmt::Display for Unsupported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "a canvas cannot draw {}", self.0)
    }
}

impl std::error::Error for Unsupported {}

type Result = std::result::Result<(), DrawingErrorKind<Unsupported>>;

/// Draws into `rect` of a canvas. Backend coordinates start at the rect's
/// top-left corner.
pub struct CanvasBackend<'a> {
    canvas: &'a mut dyn Canvas,
    rect: Rect,
}

impl<'a> CanvasBackend<'a> {
    pub fn new(canvas: &'a mut dyn Canvas, rect: Rect) -> Self {
        Self { canvas, rect }
    }

    fn point(&self, (x, y): BackendCoord) -> (f32, f32) {
        (
            self.rect.x + x as f32 / SUBPIXELS,
            self.rect.y + y as f32 / SUBPIXELS,
        )
    }

    fn stroke<S: BackendStyle>(
        &mut self,
        points: impl IntoIterator<Item = BackendCoord>,
        close: bool,
        style: &S,
    ) {
        if style.stroke_width() == 0 || style.color().alpha == 0.0 {
            return;
        }
        let d = self.path(points, close);
        if d.is_empty() {
            return;
        }
        let mut pen = Stroke::new(
            color(style.color()),
            style.stroke_width() as f32 / SUBPIXELS,
        );
        pen.join = LineJoin::Round;
        self.canvas
            .draw_path(&d, (0.0, 0.0), 0.0, &PathPaint::Stroke(pen));
    }

    fn path(&self, points: impl IntoIterator<Item = BackendCoord>, close: bool) -> String {
        let mut d = String::new();
        for (index, point) in points.into_iter().enumerate() {
            let (x, y) = self.point(point);
            d.push_str(&format!("{}{x} {y}", if index == 0 { 'M' } else { 'L' }));
        }
        if close && !d.is_empty() {
            d.push('Z');
        }
        d
    }

    fn text_style<T: BackendTextStyle>(style: &T) -> TextStyle {
        let size = style.size() as f32 / SUBPIXELS;
        let color = color(style.color());
        let text = match style.family() {
            FontFamily::Monospace => TextStyle::mono(size, color),
            _ => TextStyle::sans(size, color),
        };
        if matches!(style.style(), FontStyle::Bold) {
            text.bold()
        } else {
            text
        }
    }
}

impl DrawingBackend for CanvasBackend<'_> {
    type ErrorType = Unsupported;

    fn get_size(&self) -> (u32, u32) {
        (
            (self.rect.width * SUBPIXELS).round().max(0.0) as u32,
            (self.rect.height * SUBPIXELS).round().max(0.0) as u32,
        )
    }

    fn ensure_prepared(&mut self) -> Result {
        Ok(())
    }

    fn present(&mut self) -> Result {
        Ok(())
    }

    fn draw_pixel(&mut self, point: BackendCoord, color_: BackendColor) -> Result {
        let side = 1.0 / SUBPIXELS;
        let at = self.point(point);
        self.canvas
            .draw_rectangle(at, (side, side), color(color_), Rounding::NONE);
        Ok(())
    }

    fn draw_line<S: BackendStyle>(
        &mut self,
        from: BackendCoord,
        to: BackendCoord,
        style: &S,
    ) -> Result {
        self.stroke([from, to], false, style);
        Ok(())
    }

    fn draw_rect<S: BackendStyle>(
        &mut self,
        upper_left: BackendCoord,
        bottom_right: BackendCoord,
        style: &S,
        fill: bool,
    ) -> Result {
        let (left, top) = upper_left;
        let (right, bottom) = bottom_right;
        if fill {
            let from = self.point(upper_left);
            let to = self.point((right + 1, bottom + 1));
            self.canvas.draw_rectangle(
                from,
                (to.0 - from.0, to.1 - from.1),
                color(style.color()),
                Rounding::NONE,
            );
        } else {
            self.stroke(
                [(left, top), (right, top), (right, bottom), (left, bottom)],
                true,
                style,
            );
        }
        Ok(())
    }

    fn draw_path<S: BackendStyle, I: IntoIterator<Item = BackendCoord>>(
        &mut self,
        path: I,
        style: &S,
    ) -> Result {
        self.stroke(path, false, style);
        Ok(())
    }

    fn draw_circle<S: BackendStyle>(
        &mut self,
        center: BackendCoord,
        radius: u32,
        style: &S,
        fill: bool,
    ) -> Result {
        let center = self.point(center);
        let radius = radius as f32 / SUBPIXELS;
        if fill {
            self.canvas
                .draw_circle(center, radius, color(style.color()));
            return Ok(());
        }
        if style.stroke_width() == 0 {
            return Ok(());
        }
        let (x, y) = center;
        let d = format!(
            "M{} {y}A{radius} {radius} 0 1 0 {} {y}A{radius} {radius} 0 1 0 {} {y}Z",
            x - radius,
            x + radius,
            x - radius,
        );
        let pen = Stroke::new(
            color(style.color()),
            style.stroke_width() as f32 / SUBPIXELS,
        );
        self.canvas
            .draw_path(&d, (0.0, 0.0), 0.0, &PathPaint::Stroke(pen));
        Ok(())
    }

    fn fill_polygon<S: BackendStyle, I: IntoIterator<Item = BackendCoord>>(
        &mut self,
        vert: I,
        style: &S,
    ) -> Result {
        let d = self.path(vert, true);
        if !d.is_empty() {
            self.canvas
                .draw_path(&d, (0.0, 0.0), 0.0, &PathPaint::fill(color(style.color())));
        }
        Ok(())
    }

    fn draw_text<T: BackendTextStyle>(
        &mut self,
        text: &str,
        style: &T,
        pos: BackendCoord,
    ) -> Result {
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
        let at = self.point(pos);
        self.canvas
            .draw_text(text, at, &Self::text_style(style), alignment);
        Ok(())
    }

    fn estimate_text_size<T: BackendTextStyle>(
        &self,
        text: &str,
        style: &T,
    ) -> std::result::Result<(u32, u32), DrawingErrorKind<Unsupported>> {
        let text_style = Self::text_style(style);
        let width = self.canvas.measure(text, &text_style);
        Ok((
            (width * SUBPIXELS).ceil() as u32,
            (text_style.size * SUBPIXELS).ceil() as u32,
        ))
    }

    fn blit_bitmap(&mut self, _: BackendCoord, _: (u32, u32), _: &[u8]) -> Result {
        Err(DrawingErrorKind::DrawingError(Unsupported("bitmaps")))
    }
}

/// A theme colour as plotters takes it.
pub fn plotters_color(color: &Color) -> plotters::style::RGBAColor {
    match color {
        Color::Solid([r, g, b, a]) => plotters::style::RGBAColor(*r, *g, *b, f64::from(*a) / 255.0),
        _ => unreachable!("plot colours are solid"),
    }
}

fn color(color: BackendColor) -> Color {
    let (r, g, b) = color.rgb;
    Color::rgba(r, g, b, (color.alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
}
