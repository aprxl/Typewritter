//! Drawing a laid-out expression. The counterpart to [`math_layout`], and
//! the reason it sits beside it: layout decides where every box goes, this
//! decides what ink lands there, and both are pure functions of the
//! expression.
//!
//! Drawn through a [`Canvas`], so the editor and a PDF export run *this*
//! code rather than two copies of it. That matters more here than anywhere
//! else in the document: a fraction's bar, a radical's vinculum and a
//! script's offsets are not four values a second painter could plausibly
//! rediscover — they are a recursive walk over a tree, and two walks that
//! agree today would not stay agreeing.
//!
//! [`math_layout`]: super::math_layout

use crate::canvas::{self, Canvas};
use crate::layout::Rect;
use crate::renderer::{LineCap, LineJoin, PathPaint, Rounding, Stroke};
use crate::theme::{self, TextStyle};

use super::math_layout::{BoxKind, MathBox, MathInk, MathPrimitive};

/// Draw `box_` with its anchor line's left end at `origin`.
///
/// Child offsets are baseline-relative with y positive upward, so
/// descending into a child subtracts its y.
///
/// `slots` draws the empty-slot placeholders. They are typing affordances —
/// they say where the next character lands — so they appear only while the
/// caret is actually inside this expression in Insert mode; read back later
/// (or printed), an expression shows its notation and nothing else. Their
/// geometry is reserved either way, so an expression never resizes as the
/// caret enters or leaves it, and a page is the same width as the screen.
pub fn draw(canvas: &mut dyn Canvas, box_: &MathBox, origin: (f32, f32), slots: bool) {
    draw_inner(canvas, box_, origin, false, slots);
}

/// The box's full area, anchored at `origin` — what a selection or a hit
/// test covers.
pub fn rect(box_: &MathBox, origin: (f32, f32)) -> Rect {
    Rect {
        x: origin.0,
        y: origin.1 - box_.ascent,
        width: box_.width,
        height: box_.ascent + box_.descent,
    }
}

/// How round a highlight's corners are, as a fraction of its shorter side.
/// A single letter comes out a stadium and a three-letter function name a
/// generously rounded rectangle, which is the same corner rather than two.
const HIGHLIGHT_RADIUS: f32 = 0.45;
/// The weight of a highlight's border. A hairline: the shape is the signal,
/// and a heavier line would start competing with the glyph inside it.
const HIGHLIGHT_EDGE: f32 = 1.0;

/// `covered` is set once an ancestor has painted a highlight: the wash
/// already says what the subtree is, and a second one inside it would read
/// as a different claim rather than the same one.
fn draw_inner(
    canvas: &mut dyn Canvas,
    box_: &MathBox,
    origin: (f32, f32),
    covered: bool,
    slots: bool,
) {
    if let Some(style) = box_.highlight.filter(|_| !covered) {
        let area = rect(box_, origin);
        let radius = area.width.min(area.height) * HIGHLIGHT_RADIUS;
        if style.shape.fills() {
            canvas.draw_rectangle(
                area.position(),
                area.size(),
                theme::math_fill(style.hue),
                Rounding::uniform(radius),
            );
        }
        if style.shape.outlines() {
            // Inset by half the pen so the stroke lands inside the box the
            // fill covers; a centred stroke would widen the symbol by a
            // pixel that layout never reserved.
            let inset = HIGHLIGHT_EDGE * 0.5;
            canvas::rounded_outline(
                canvas,
                area.inset(inset),
                radius - inset,
                HIGHLIGHT_EDGE,
                theme::math_edge(style.hue),
            );
        }
    }
    let covered = covered || box_.highlight.is_some();
    match &box_.kind {
        BoxKind::Glyph {
            text,
            size,
            ink,
            offset_x,
            offset_y,
            condense,
        } => {
            // Math boxes use glyph centres as their baseline for now; LEFT's
            // vertical centring therefore matches the prose baseline draw.
            canvas.draw_text(
                text,
                (origin.0 + offset_x, origin.1 - offset_y),
                &TextStyle::math(*size, notation_ink(*ink)).condensed(*condense),
                theme::LEFT,
            );
        }
        BoxKind::Bar { thickness } => {
            // A fractional one-pixel rule smears across adjacent rows.
            canvas::rule(
                canvas,
                (origin.0, (origin.1 - thickness * 0.5).round()),
                box_.width,
                *thickness,
                theme::ink(),
            );
        }
        BoxKind::Slot { visible, .. } => {
            // `visible` is structural — an integral's unasked-for limits are
            // never drawn. `slots` is the mode gate on top of it.
            if !visible || !slots {
                return;
            }
            let rect = rect(box_, origin);
            canvas.draw_rectangle(rect.position(), rect.size(), theme::alt(), Rounding::NONE);
            canvas::outline(canvas, rect, theme::non_text());
        }
        BoxKind::Primitive(primitive) => match primitive {
            MathPrimitive::Stroke { path, thickness } => {
                let mut stroke = Stroke::new(theme::ink(), *thickness);
                stroke.cap = LineCap::Round;
                stroke.join = LineJoin::Round;
                canvas.draw_path(path, origin, 0.0, &PathPaint::Stroke(stroke));
            }
            MathPrimitive::Dots { centers, radius } => {
                for &(x, y) in centers {
                    canvas.draw_circle((origin.0 + x, origin.1 + y), *radius, theme::ink());
                }
            }
        },
        BoxKind::Row { children } => {
            for (x, y, child) in children {
                draw_inner(canvas, child, (origin.0 + x, origin.1 - y), covered, slots);
            }
        }
    }
}

/// The colour each notation ink names. The one place the three of them meet
/// a palette, so a numeral is the same blue wherever it is set.
fn notation_ink(ink: MathInk) -> crate::renderer::Color {
    match ink {
        MathInk::Term => theme::ink(),
        MathInk::Number => theme::math_number(),
        MathInk::Operator => theme::math_operator(),
    }
}
