//! The palette switch: a squircle that shows the theme you are on, and —
//! on hover — cuts itself in half to show the one you would get.
//!
//! Not a [`Component`](crate::ui::Component) of its own. It sits inside the
//! title bar's region because it is a 28-pixel button and a region costs a
//! full-surface render target; what lives here is the geometry and the
//! drawing, so [`title_bar`](super::title_bar) can host it and the shell
//! can hit-test it without either one knowing how it is built.
//!
//! **The split is geometry, not a clip.** A layer's clip shape is a
//! whole-layer mask (see `Layer::set_clip_shape`), and the switch shares its
//! layer with the rest of the title bar, so the diagonal is cut where the
//! shape is made instead: the squircle is sampled into a convex polygon and
//! clipped against a half-plane, which is a handful of dot products and
//! gives the same antialiased edge the MSAA resolve gives every other
//! polygon.
//!
//! The cut sweeps rather than appears. At rest the half-plane sits clear of
//! the button, so the current palette owns all of it; as the hover weight
//! rises the plane slides in until it passes through the centre. That is
//! what makes the reveal read as one motion instead of two states.

use crate::layout::Rect;
use crate::renderer::{Color, Layer};
use crate::theme::{self, Mode, Theme, icons};

/// Side of the squircle, logical pixels.
pub const SIZE: f32 = 30.0;
/// Corner radius. A third of the side: past a rounded rectangle, short of a
/// circle — the shape the design calls a squircle.
const RADIUS: f32 = 10.0;
/// Gap between the switch and the search box to its left.
pub const GAP: f32 = 12.0;
/// Distance from the right edge of the title bar, matching the inset the
/// rest of the bar's right-hand content uses.
const INSET: f32 = 18.0;

/// Icon side, logical pixels.
const ICON: f32 = 13.0;
/// How far each half's icon sits from the split, along the split's normal.
/// Both bounds are tight: less and the icon crosses the seam into the other
/// palette's half, more and it runs off the squircle's far edge. The unit
/// tests below pin the shape this is measured against.
const ICON_OFFSET: f32 = 7.0;
/// How far the icons tilt at full hover, radians. Small: a nudge that says
/// the button is live, not a spin.
const TILT: f32 = -0.22;
/// Where the half-plane starts, as a multiple of the button's side. Past the
/// far corner (which is at `1/sqrt(2)`), so at rest the other palette is not
/// merely invisible but absent — no geometry, no draw call.
const SWEEP_START: f32 = 0.78;

/// Segments per rounded corner. Eight is past the point where another one
/// changes a pixel at this radius.
const CORNER_SEGMENTS: usize = 8;

/// A polyline in logical pixels — closed when it is a shape, open when it
/// is a run of one shape's boundary. Both come out of [`clip`].
type Points = Vec<(f32, f32)>;

/// Where the switch sits inside the title bar's `bar` rect, against the same
/// vertical centre the rest of the bar lays out from.
pub fn switch_rect(bar: Rect) -> Rect {
    let middle = bar.y + (bar.height - 2.0) / 2.0;
    Rect::new(bar.right() - INSET - SIZE, middle - SIZE / 2.0, SIZE, SIZE)
}

/// Draw the switch. `hover` is the 0..1 hover weight; `locked` holds the
/// button at rest while a swap is still running, so the affordance stops
/// inviting a click that would be ignored.
pub fn draw(layer: &Layer, rect: Rect, hover: f32, locked: bool) {
    let hover = if locked { 0.0 } else { hover.clamp(0.0, 1.0) };
    let current = theme::current();
    let other = theme::counterpart();

    let outline = squircle(rect, RADIUS);
    let centre = (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
    // Normal of the split, pointing into the incoming half: down-right, so
    // the palette you would switch to arrives from the bottom-right corner
    // and the one you are on retreats to the top-left.
    const DIAGONAL: f32 = std::f32::consts::FRAC_1_SQRT_2;
    let normal = (DIAGONAL, DIAGONAL);
    let offset = SIZE * SWEEP_START * (1.0 - hover);

    // The half you are on: everything behind the plane. At rest that is the
    // whole button, and this is the only fill drawn.
    let (near, _) = clip(&outline, normal, centre, offset);
    layer.draw_polygon(&near, surface(&current, hover));

    // The half you would get. `arc` is the run of the squircle's own
    // boundary that survived the cut, which is what lets the outline be
    // drawn in two palettes without the seam being stroked as if it were
    // part of it.
    let far_normal = (-normal.0, -normal.1);
    let (far, arc) = clip(&outline, far_normal, centre, -offset);
    if far.len() >= 3 {
        layer.draw_polygon(&far, surface(&other, hover));
    }

    // Outline: the whole ring in the current palette, then the incoming
    // half's arc over the top of it in its own. Drawing the ring whole first
    // means the un-hovered button pays for one stroke, and the hovered one
    // never shows a gap where the two arcs meet.
    let mut ring = outline.clone();
    ring.push(outline[0]);
    theme::polyline(layer, &ring, edge(&current, hover), 1.2);
    if arc.len() >= 2 {
        theme::polyline(layer, &arc, edge(&other, hover), 1.2);
        // The seam itself, a hairline along the cut.
        let seam = [arc[0], arc[arc.len() - 1]];
        theme::polyline(layer, &seam, edge(&other, hover), 1.0);
    }

    // Icons. Each slides out along the split's normal as the cut arrives, so
    // at rest the current palette's icon is dead centre.
    let slide = ICON_OFFSET * hover;
    draw_icon(
        layer,
        (centre.0 - normal.0 * slide, centre.1 - normal.1 * slide),
        current.mode,
        current.ink.clone(),
        hover,
    );
    if hover > 0.0 {
        draw_icon(
            layer,
            (centre.0 + normal.0 * slide, centre.1 + normal.1 * slide),
            other.mode,
            theme::fade(other.ink.clone(), hover),
            hover,
        );
    }
}

/// The sun or the moon, centred on `at` and tilted by the hover weight.
fn draw_icon(layer: &Layer, at: (f32, f32), mode: Mode, color: Color, hover: f32) {
    let path = match mode {
        Mode::Light => icons::SUN,
        Mode::Dark => icons::MOON,
    };
    theme::icon_turned(
        layer,
        path,
        (at.0 - ICON / 2.0, at.1 - ICON / 2.0),
        ICON,
        TILT * hover,
        color,
        1.4,
    );
}

/// A half's fill. The page colour, not a panel colour: it is the most
/// recognisable thing about a palette, which is the whole point of showing
/// half of one. Hover warms it toward the accent — the "lights up" is a tint
/// rather than a brightness step, so it reads the same way in a light
/// palette as in a dark one.
fn surface(theme: &Theme, hover: f32) -> Color {
    theme::mix(theme.background.clone(), theme.accent.clone(), 0.10 * hover)
}

/// A half's outline, on the same terms.
///
/// `non_text` rather than `border`: a border is drawn *between* surfaces
/// and is therefore darker than both in a dark palette, which makes it
/// disappear when a button sits on the gutter. `non_text` is the palette's
/// name for an outline that has to be seen — empty-slot outlines, rules
/// inside a line of type — and it steps toward the foreground in both
/// palettes rather than away from it.
fn edge(theme: &Theme, hover: f32) -> Color {
    // Half way to the accent, not all the way: at full hover the two halves
    // still have to be outlined in colours of their own, or the preview
    // would come down to the fill alone.
    theme::mix(theme.non_text.clone(), theme.accent.clone(), 0.5 * hover)
}

/// The squircle as a closed, convex polygon in logical pixels.
///
/// Sampled rather than described as an SVG path because both things done
/// with it — filling a half and stroking an arc — need vertices, and a shape
/// this small is a few dozen points.
fn squircle(rect: Rect, radius: f32) -> Points {
    let radius = radius.min(rect.width / 2.0).min(rect.height / 2.0);
    // Each corner's arc centre, and the angle that arc starts at. Walked in
    // screen order (top-left, top-right, bottom-right, bottom-left), so the
    // points come out in one consistent winding.
    let corners = [
        ((rect.x + radius, rect.y + radius), std::f32::consts::PI),
        (
            (rect.right() - radius, rect.y + radius),
            std::f32::consts::PI * 1.5,
        ),
        ((rect.right() - radius, rect.bottom() - radius), 0.0),
        (
            (rect.x + radius, rect.bottom() - radius),
            std::f32::consts::FRAC_PI_2,
        ),
    ];
    let mut points = Vec::with_capacity(corners.len() * (CORNER_SEGMENTS + 1));
    for ((cx, cy), start) in corners {
        for step in 0..=CORNER_SEGMENTS {
            let angle =
                start + std::f32::consts::FRAC_PI_2 * (step as f32 / CORNER_SEGMENTS as f32);
            points.push((cx + radius * angle.cos(), cy + radius * angle.sin()));
        }
    }
    points
}

/// Clip convex polygon `poly` to the half-plane `dot(p - pivot, normal) <=
/// offset` — Sutherland–Hodgman against a single edge, which is all a
/// half-plane is.
///
/// Returns the clipped polygon and, separately, the run of `poly`'s *own*
/// boundary that survived: an open polyline from where the cut enters the
/// shape to where it leaves. The caller fills the first and strokes the
/// second, which is how one half gets outlined without the seam being
/// stroked as part of the outline.
fn clip(
    poly: &[(f32, f32)],
    normal: (f32, f32),
    pivot: (f32, f32),
    offset: f32,
) -> (Points, Points) {
    let side = |(x, y): (f32, f32)| (x - pivot.0) * normal.0 + (y - pivot.1) * normal.1 - offset;

    let mut clipped = Points::with_capacity(poly.len() + 2);
    // The surviving boundary in walk order, plus where in it the cut
    // *enters* the shape. A convex polygon meets a half-plane in one
    // contiguous run, but the walk can start partway through that run, so
    // the list is rotated onto the entry point at the end rather than
    // assumed to already begin there.
    let mut run = Points::with_capacity(poly.len() + 2);
    let mut entry: Option<usize> = None;

    for index in 0..poly.len() {
        let from = poly[index];
        let to = poly[(index + 1) % poly.len()];
        let (from_side, to_side) = (side(from), side(to));

        if from_side <= 0.0 {
            clipped.push(from);
            run.push(from);
        }
        if (from_side <= 0.0) != (to_side <= 0.0) {
            let t = from_side / (from_side - to_side);
            let crossing = (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
            clipped.push(crossing);
            if from_side > 0.0 {
                entry = Some(run.len());
            }
            run.push(crossing);
        }
    }

    if let Some(entry) = entry {
        run.rotate_left(entry);
    }
    (clipped, run)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Points {
        vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]
    }

    #[test]
    fn a_plane_clear_of_the_shape_keeps_all_of_it() {
        let (kept, _) = clip(&square(), (1.0, 0.0), (5.0, 5.0), 100.0);
        assert_eq!(kept.len(), 4);
    }

    #[test]
    fn a_plane_past_the_shape_keeps_none_of_it() {
        let (kept, arc) = clip(&square(), (1.0, 0.0), (5.0, 5.0), -100.0);
        assert!(kept.is_empty());
        assert!(arc.is_empty());
    }

    #[test]
    fn a_vertical_cut_halves_the_square() {
        // Keep x <= 5.
        let (kept, arc) = clip(&square(), (1.0, 0.0), (5.0, 5.0), 0.0);
        assert!(kept.iter().all(|(x, _)| *x <= 5.0 + 1e-4), "{kept:?}");
        assert!(kept.contains(&(0.0, 0.0)));
        assert!(kept.contains(&(0.0, 10.0)));
        // The run ends on the two crossings, both on x = 5 and distinct.
        assert!((arc[0].0 - 5.0).abs() < 1e-4, "{arc:?}");
        assert!((arc[arc.len() - 1].0 - 5.0).abs() < 1e-4, "{arc:?}");
        assert!(
            (arc[0].1 - arc[arc.len() - 1].1).abs() > 1e-4,
            "the ends are the two crossings, not one point twice: {arc:?}"
        );
    }

    #[test]
    fn the_run_is_the_boundary_and_never_the_seam() {
        // Cut the square on its own diagonal — the case the switch draws.
        let normal = (
            std::f32::consts::FRAC_1_SQRT_2,
            std::f32::consts::FRAC_1_SQRT_2,
        );
        let (_, arc) = clip(&square(), normal, (5.0, 5.0), 0.0);
        // Every point between the two ends is on the square's own boundary;
        // a point on the seam's interior would be strictly inside it.
        for point in &arc[1..arc.len() - 1] {
            let on_edge = point.0.abs() < 1e-4
                || point.1.abs() < 1e-4
                || (point.0 - 10.0).abs() < 1e-4
                || (point.1 - 10.0).abs() < 1e-4;
            assert!(on_edge, "{point:?} is not on the square's boundary");
        }
    }

    #[test]
    fn the_squircle_stays_inside_its_rect() {
        let rect = Rect::new(4.0, 6.0, SIZE, SIZE);
        for (x, y) in squircle(rect, RADIUS) {
            assert!(x >= rect.x - 1e-3 && x <= rect.right() + 1e-3, "{x}");
            assert!(y >= rect.y - 1e-3 && y <= rect.bottom() + 1e-3, "{y}");
        }
    }

    /// Is `point` inside convex polygon `poly`? Every edge of a convex
    /// polygon has the whole shape on one side of it, so one consistent
    /// cross-product sign is the whole test.
    fn inside(poly: &[(f32, f32)], point: (f32, f32)) -> bool {
        let mut sign = 0.0f32;
        for index in 0..poly.len() {
            let (ax, ay) = poly[index];
            let (bx, by) = poly[(index + 1) % poly.len()];
            let cross = (bx - ax) * (point.1 - ay) - (by - ay) * (point.0 - ax);
            if cross.abs() < 1e-6 {
                continue;
            }
            if sign == 0.0 {
                sign = cross.signum();
            } else if cross.signum() != sign {
                return false;
            }
        }
        true
    }

    /// The claim `ICON_OFFSET`'s doc makes, measured rather than asserted:
    /// at full hover an icon has to clear the seam on one side and the
    /// squircle's edge on the other. Both icons are bounded by a disc of
    /// half the icon size — the sun's rays reach the viewbox edge at the
    /// cardinal directions and nothing reaches the box's corners — so that
    /// disc is what gets checked.
    #[test]
    fn an_icon_clears_both_the_seam_and_the_edge() {
        let rect = Rect::new(0.0, 0.0, SIZE, SIZE);
        let outline = squircle(rect, RADIUS);
        let centre = (SIZE / 2.0, SIZE / 2.0);
        let normal = (
            std::f32::consts::FRAC_1_SQRT_2,
            std::f32::consts::FRAC_1_SQRT_2,
        );
        let reach = ICON / 2.0;
        assert!(
            ICON_OFFSET > reach,
            "an icon {ICON} across, {ICON_OFFSET} from the seam, crosses it"
        );

        let icon_centre = (
            centre.0 + normal.0 * ICON_OFFSET,
            centre.1 + normal.1 * ICON_OFFSET,
        );
        for step in 0..64 {
            let angle = std::f32::consts::TAU * (step as f32 / 64.0);
            let point = (
                icon_centre.0 + reach * angle.cos(),
                icon_centre.1 + reach * angle.sin(),
            );
            assert!(
                inside(&outline, point),
                "{point:?} falls outside the squircle"
            );
            let from_seam = (point.0 - centre.0) * normal.0 + (point.1 - centre.1) * normal.1;
            assert!(from_seam > 0.0, "{point:?} crosses into the other half");
        }
    }

    #[test]
    fn the_sweep_starts_clear_of_the_button() {
        // At rest the incoming half must be empty, or the button would show
        // a sliver of the other palette while nothing is hovered.
        let rect = Rect::new(0.0, 0.0, SIZE, SIZE);
        let outline = squircle(rect, RADIUS);
        let centre = (SIZE / 2.0, SIZE / 2.0);
        let normal = (
            -std::f32::consts::FRAC_1_SQRT_2,
            -std::f32::consts::FRAC_1_SQRT_2,
        );
        let (far, _) = clip(&outline, normal, centre, -SIZE * SWEEP_START);
        assert!(far.is_empty(), "{far:?}");
    }
}
