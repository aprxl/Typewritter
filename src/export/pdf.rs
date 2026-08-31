//! [`PdfCanvas`] — the krilla backend.
//!
//! The only file that knows the output is a PDF, and the only one below
//! `geometry.rs` that says `pt`. Everything it is handed is in the editor's
//! logical pixels; [`PageGeometry::point`] is applied here and nowhere
//! else, so there is exactly one place a coordinate could be scaled twice.
//!
//! krilla's surface is top-left origin with y down, the same as the
//! editor's, so nothing in this pipeline flips an axis.

use std::collections::HashMap;

use krilla::color::rgb;
use krilla::geom::{Path, PathBuilder, Point, Rect, Transform};
use krilla::num::NormalizedF32;
use krilla::paint::{Fill, Stroke};
use krilla::surface::Surface;
use krilla::text::{Font, GlyphId, KrillaGlyph};

use crate::renderer::{
    Alignment, Color, FaceId, HorizontalAlign, Layer, PathPaint, Rounding, VerticalAlign,
};
use crate::theme::TextStyle;

use super::canvas::Canvas;
use super::geometry::PageGeometry;
use super::text::Shaper;

/// Bézier approximation of a quarter circle: the control points sit this
/// fraction of the radius along the tangents. Written as its derivation
/// rather than as `0.5522…`, so it reads as the constant it is.
const KAPPA: f32 = 4.0 / 3.0 * (std::f32::consts::SQRT_2 - 1.0);

/// One krilla [`Font`] per face the document actually drew with, built on
/// first sight from the bytes the renderer resolved. Shared across pages —
/// krilla subsets each embedded face once for the whole document.
#[derive(Default)]
pub struct Fonts {
    faces: HashMap<FaceId, Option<Font>>,
}

impl Fonts {
    /// The embeddable face behind `face`, or `None` if its bytes can't be
    /// read or parsed. A face that fails is skipped rather than fatal: the
    /// export loses those glyphs, not the document.
    fn get(&mut self, layer: &Layer, face: FaceId) -> Option<Font> {
        self.faces
            .entry(face)
            .or_insert_with(|| {
                let (data, index) = layer.face_data(face)?;
                Font::new(data.into(), index)
            })
            .clone()
    }
}

pub struct PdfCanvas<'a, 'b> {
    surface: Surface<'a>,
    shaper: &'b Shaper<'b>,
    layer: &'b Layer,
    fonts: &'b mut Fonts,
    geometry: PageGeometry,
}

impl<'a, 'b> PdfCanvas<'a, 'b> {
    pub fn new(
        surface: Surface<'a>,
        shaper: &'b Shaper<'b>,
        layer: &'b Layer,
        fonts: &'b mut Fonts,
        geometry: PageGeometry,
    ) -> Self {
        Self {
            surface,
            shaper,
            layer,
            fonts,
            geometry,
        }
    }

    fn point(&self, at: (f32, f32)) -> Point {
        let (x, y) = self.geometry.point(at);
        Point::from_xy(x, y)
    }

    fn paint(&mut self, path: &Path, color: &Color) {
        self.surface.set_fill(Some(fill(color)));
        self.surface.draw_path(path);
    }
}

/// A colour as krilla sees it: sRGB bytes, which is how the theme authors
/// them, plus alpha as coverage.
///
/// Only [`Color::Solid`] reaches a page — every call site in `paint.rs`
/// passes a `theme` colour and every one of those is solid. The others have
/// no single value to fill with, so they assert in a debug build and print
/// as ink rather than as nothing.
fn solid(color: &Color) -> (rgb::Color, f32) {
    let [r, g, b, a] = match color {
        Color::Solid(rgba) => *rgba,
        _ => {
            debug_assert!(false, "the page paints solid colours only");
            [0x00, 0x00, 0x00, 0xFF]
        }
    };
    (rgb::Color::new(r, g, b), f32::from(a) / 255.0)
}

fn fill(color: &Color) -> Fill {
    let (rgb, alpha) = solid(color);
    Fill {
        paint: rgb.into(),
        opacity: NormalizedF32::new(alpha).unwrap_or(NormalizedF32::ONE),
        ..Fill::default()
    }
}

/// A rounded rectangle, already in points.
///
/// Each radius is clamped to half the shorter side, so a small box with a
/// large radius reads as a stadium instead of turning inside out — the same
/// treatment `Rounding` gets on screen. A box with no rounding at all takes
/// the straight path, which is most of them.
fn rounded_rect(at: (f32, f32), size: (f32, f32), rounding: Rounding) -> Option<Path> {
    let (x, y) = at;
    let (w, h) = size;
    let mut path = PathBuilder::new();

    let limit = (w.min(h) * 0.5).max(0.0);
    let tl = rounding.top_left.clamp(0.0, limit);
    let tr = rounding.top_right.clamp(0.0, limit);
    let br = rounding.bottom_right.clamp(0.0, limit);
    let bl = rounding.bottom_left.clamp(0.0, limit);

    if tl == 0.0 && tr == 0.0 && br == 0.0 && bl == 0.0 {
        path.push_rect(Rect::from_xywh(x, y, w, h)?);
        return path.finish();
    }

    path.move_to(x + tl, y);
    path.line_to(x + w - tr, y);
    if tr > 0.0 {
        arc(&mut path, (x + w, y), (0.0, tr), (-tr, 0.0));
    }
    path.line_to(x + w, y + h - br);
    if br > 0.0 {
        arc(&mut path, (x + w, y + h), (-br, 0.0), (0.0, -br));
    }
    path.line_to(x + bl, y + h);
    if bl > 0.0 {
        arc(&mut path, (x, y + h), (0.0, -bl), (bl, 0.0));
    }
    path.line_to(x, y + tl);
    if tl > 0.0 {
        arc(&mut path, (x, y), (tl, 0.0), (0.0, tl));
    }
    path.close();
    path.finish()
}

/// A quarter turn around `corner`, from the point `corner + from` to
/// `corner + to`, with both control points pulled `KAPPA` of the way back
/// toward the corner.
fn arc(path: &mut PathBuilder, corner: (f32, f32), from: (f32, f32), to: (f32, f32)) {
    path.cubic_to(
        corner.0 + to.0 * KAPPA,
        corner.1 + to.1 * KAPPA,
        corner.0 + from.0 * KAPPA,
        corner.1 + from.1 * KAPPA,
        corner.0 + from.0,
        corner.1 + from.1,
    );
}

/// A circle, already in points. Four quarter turns — `PathBuilder` has no
/// ellipse of its own.
#[allow(dead_code)] // drawn once the list markers arrive — `PDF.md` §6
fn circle(center: (f32, f32), radius: f32) -> Option<Path> {
    let (cx, cy) = center;
    let r = radius;
    let k = r * KAPPA;
    let mut path = PathBuilder::new();
    path.move_to(cx + r, cy);
    path.cubic_to(cx + r, cy + k, cx + k, cy + r, cx, cy + r);
    path.cubic_to(cx - k, cy + r, cx - r, cy + k, cx - r, cy);
    path.cubic_to(cx - r, cy - k, cx - k, cy - r, cx, cy - r);
    path.cubic_to(cx + k, cy - r, cx + r, cy - k, cx + r, cy);
    path.close();
    path.finish()
}

impl Canvas for PdfCanvas<'_, '_> {
    fn draw_rectangle(
        &mut self,
        at: (f32, f32),
        size: (f32, f32),
        color: Color,
        rounding: Rounding,
    ) {
        let at = self.geometry.point(at);
        let size = (self.geometry.length(size.0), self.geometry.length(size.1));
        let rounding = Rounding {
            top_left: self.geometry.length(rounding.top_left),
            top_right: self.geometry.length(rounding.top_right),
            bottom_right: self.geometry.length(rounding.bottom_right),
            bottom_left: self.geometry.length(rounding.bottom_left),
        };
        if let Some(path) = rounded_rect(at, size, rounding) {
            self.paint(&path, &color);
        }
    }

    fn draw_circle(&mut self, center: (f32, f32), radius: f32, color: Color) {
        let center = self.geometry.point(center);
        if let Some(path) = circle(center, self.geometry.length(radius)) {
            self.paint(&path, &color);
        }
    }

    fn draw_path(
        &mut self,
        _d: &str,
        _at: (f32, f32),
        _scale: f32,
        _rotation: f32,
        _paint: &PathPaint,
    ) {
        // Arrives with the first element that needs one. The divider's rule
        // is a rectangle, and the fold chevron is chrome a page never
        // shows, so nothing painted today reaches here. See `PDF.md` §6.
    }

    fn draw_text(&mut self, text: &str, at: (f32, f32), style: &TextStyle, align: Alignment) {
        let shaped = self.shaper.runs(text, style);
        if shaped.runs.is_empty() {
            return;
        }

        // `at` names the point `align` picks out of the run's box. Glyphs
        // are placed from that box's top-left and sit on the baseline below
        // it — the same resolution `Layer::draw_text` does before drawing.
        let (width, height) = shaped.size;
        let left = at.0
            + match align.horizontal {
                HorizontalAlign::Left => 0.0,
                HorizontalAlign::Center => -width * 0.5,
                HorizontalAlign::Right => -width,
            };
        let top = at.1
            + match align.vertical {
                VerticalAlign::Top => 0.0,
                VerticalAlign::Center => -height * 0.5,
                VerticalAlign::Bottom => -height,
            };
        let baseline = top + shaped.baseline;

        let parameters = style.parameters();
        let size = self.geometry.length(parameters.size);
        self.surface.set_fill(Some(fill(&style.color)));
        // Faux bold is an outline dilated by `weight` px on every side, so
        // on the page it is the same run stroked as well as filled, at
        // twice that width.
        self.surface
            .set_stroke((parameters.weight > 0.0).then(|| Stroke {
                paint: solid(&style.color).0.into(),
                width: self.geometry.length(parameters.weight * 2.0),
                ..Stroke::default()
            }));

        // Faux condense and faux italic are an x-scale and a shear of every
        // glyph about its own origin. Every origin on a line sits on the
        // same baseline, so one transform about that baseline is the
        // identical result — see `PDF.md` §4. y is down here and up in the
        // font's own outlines, so the shear that leans a glyph forward
        // takes the opposite sign.
        let transformed = parameters.width != 1.0 || parameters.slant != 0.0;
        if transformed {
            let origin = self.geometry.point((left, baseline));
            self.surface
                .push_transform(&Transform::from_translate(origin.0, origin.1));
            self.surface.push_transform(&Transform::from_row(
                parameters.width,
                0.0,
                -parameters.slant,
                1.0,
                0.0,
                0.0,
            ));
            self.surface
                .push_transform(&Transform::from_translate(-origin.0, -origin.1));
        }

        for run in &shaped.runs {
            let Some(font) = self.fonts.get(self.layer, run.face) else {
                continue;
            };
            // krilla walks the pen itself, accumulating advances. Those are
            // each glyph's true width — what a reader's selection follows —
            // and `x_offset` then puts every glyph back exactly where the
            // shaper put it, so the two can never drift apart however the
            // advances round. Both are per em, hence the division; a
            // uniform px → pt scale cancels out of `length / size`.
            let mut pen = 0.0;
            let mut glyphs = Vec::with_capacity(run.glyphs.len());
            for (index, &glyph) in run.glyphs.iter().enumerate() {
                let offset = (run.positions[index] - run.positions[0]) - pen;
                pen += run.advances[index];
                glyphs.push(KrillaGlyph::new(
                    GlyphId::new(u32::from(glyph)),
                    run.advances[index] / parameters.size,
                    offset / parameters.size,
                    0.0,
                    0.0,
                    0..run.text.len(),
                    None,
                ));
            }
            let start = self.point((left + run.positions[0], baseline));
            self.surface
                .draw_glyphs(start, &glyphs, font, &run.text, size, false);
        }

        if transformed {
            self.surface.pop();
            self.surface.pop();
            self.surface.pop();
        }
        // The stroke is this call's, not the surface's: leaving it set
        // would outline the next rectangle drawn.
        self.surface.set_stroke(None);
    }

    fn measure(&self, text: &str, style: &TextStyle) -> f32 {
        self.shaper.width(text, style)
    }
}
