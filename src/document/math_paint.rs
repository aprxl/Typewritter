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

use super::math::SymbolRole;
use super::math_layout::{BoxKind, MathBox, MathPrimitive};

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

/// `covered` is set once an ancestor has painted a role highlight: the
/// wash already says what the subtree is, and a second one inside it would
/// read as a different claim rather than the same one.
fn draw_inner(
    canvas: &mut dyn Canvas,
    box_: &MathBox,
    origin: (f32, f32),
    covered: bool,
    slots: bool,
) {
    if let Some(role) = box_.highlight.filter(|_| !covered) {
        let height = box_.ascent + box_.descent;
        let color = match role {
            SymbolRole::Variable => theme::variable(),
            SymbolRole::Constant => theme::constant(),
            SymbolRole::Function => theme::function(),
        };
        canvas.draw_rectangle(
            (origin.0, origin.1 - box_.ascent),
            (box_.width, height),
            color,
            Rounding::uniform(box_.width.min(height) * 0.45),
        );
    }
    let covered = covered || box_.highlight.is_some();
    match &box_.kind {
        BoxKind::Glyph {
            text,
            size,
            offset_x,
            offset_y,
            condense,
        } => {
            // Math boxes use glyph centres as their baseline for now; LEFT's
            // vertical centring therefore matches the prose baseline draw.
            canvas.draw_text(
                text,
                (origin.0 + offset_x, origin.1 - offset_y),
                &TextStyle::math(*size, theme::ink()).condensed(*condense),
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
