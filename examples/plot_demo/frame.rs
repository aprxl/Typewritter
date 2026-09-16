//! The container a plot sits in, drawn by Typewritter rather than
//! plotters: a rounded surface, tick marks standing off its edges, and the
//! labels beyond them.
//!
//! Two layers per plot. The *data* layer holds the surface and whatever
//! plotters draws, clipped to the container's rounded shape. The *frame*
//! layer sits above it with the border, marks, labels and title, so the
//! border is always crisp over the data's edge.

use typewritter::canvas;
use typewritter::layout::Rect;
use typewritter::renderer::{
    Alignment, ClipShape, Color, FontParameters, HorizontalAlign, Layer, Rounding, VerticalAlign,
};
use typewritter::theme;

use crate::plots::{Frame, Tick, Ticks};

pub const RADIUS: f32 = 10.0;
const TITLE: f32 = 13.0;
const LABEL: f32 = 10.5;
const TITLE_ROW: f32 = 24.0;
const DESC_ROW: f32 = 16.0;
/// Room for a mark and one line of labels below or above the container.
const X_BAND: f32 = 20.0;
/// Room for a mark and a label beside the container.
const Y_BAND: f32 = 40.0;
/// Room on an unlabelled right edge for the last x label's overhang.
const OVERHANG: f32 = 10.0;
const MARK: f32 = 4.0;
const MARK_GAP: f32 = 4.0;
/// Least space between two x labels.
const LABEL_GAP: f32 = 6.0;

/// The inside of the container for a plot laid out in `cell`.
pub fn container(cell: Rect, frame: &Frame) -> Rect {
    let mut top = cell.y + TITLE_ROW;
    let mut bottom = cell.bottom();
    let mut left = cell.x;
    let mut right = cell.right();
    if frame.y_desc.is_some() || frame.y2_desc.is_some() {
        top += DESC_ROW;
    }
    if frame.x_desc.is_some() {
        bottom -= DESC_ROW;
    }
    if frame.ticks {
        if frame.x_on_top {
            top += X_BAND;
        } else {
            bottom -= X_BAND;
        }
        left += Y_BAND;
        right -= if frame.secondary { Y_BAND } else { OVERHANG };
    }
    Rect::new(
        left.round(),
        top.round(),
        (right - left).round().max(1.0),
        (bottom - top).round().max(1.0),
    )
}

/// Readies the data layer: clipped to the container's rounded shape.
pub fn clip(layer: &Layer, inside: Rect) {
    layer
        .set_clip_shape(Some(ClipShape::Rectangle {
            top_left: inside.position(),
            size: inside.size(),
            rounding: Rounding::uniform(RADIUS),
        }))
        .expect("a rectangle clip never fails to parse");
}

/// Border, ticks, labels and title, onto the frame layer. `cell` is what
/// [`container`] turned into `inside`.
pub fn draw(layer: &Layer, cell: Rect, inside: Rect, frame: &Frame, ticks: &Ticks) {
    let scale = layer.scale_factor();
    // Titles and labels never reach into a neighbouring plot.
    layer.set_clip_rect(Some((cell.position(), cell.size())));
    let mut pen: &Layer = layer;
    canvas::rounded_outline(&mut pen, inside, RADIUS, 1.0, theme::border());

    let mut title = FontParameters::new(TITLE);
    title.weight = TITLE * 0.018;
    layer.draw_text(
        frame.title,
        (inside.x, cell.y + 2.0),
        theme::ink(),
        Alignment::TOP_LEFT,
        theme::sans(),
        title,
    );
    let label = FontParameters::new(LABEL);
    let x_label_y = if frame.x_on_top {
        inside.y - MARK - MARK_GAP
    } else {
        inside.bottom() + MARK + MARK_GAP
    };
    // A label that would touch the one before it is left out; its mark
    // stays, so the rhythm of the axis still reads.
    let mut previous_right = f32::NEG_INFINITY;
    for tick in visible(&ticks.x, inside.width, scale) {
        let x = inside.x + tick.at as f32 / scale;
        let (mark_y, vertical) = if frame.x_on_top {
            (inside.y - MARK, VerticalAlign::Bottom)
        } else {
            (inside.bottom(), VerticalAlign::Top)
        };
        mark(layer, (x - 0.5, mark_y), (1.0, MARK));
        let (width, _) = layer.get_text_size(&tick.label, &theme::sans(), &label);
        if x - width / 2.0 < previous_right + LABEL_GAP {
            continue;
        }
        previous_right = x + width / 2.0;
        text(
            layer,
            &tick.label,
            (x, x_label_y),
            HorizontalAlign::Center,
            vertical,
            label,
        );
    }
    for tick in visible(&ticks.y, inside.height, scale) {
        let y = inside.y + tick.at as f32 / scale;
        mark(layer, (inside.x - MARK, y - 0.5), (MARK, 1.0));
        text(
            layer,
            &tick.label,
            (inside.x - MARK - MARK_GAP, y),
            HorizontalAlign::Right,
            VerticalAlign::Center,
            label,
        );
    }
    for tick in visible(&ticks.y2, inside.height, scale) {
        let y = inside.y + tick.at as f32 / scale;
        mark(layer, (inside.right(), y - 0.5), (MARK, 1.0));
        text(
            layer,
            &tick.label,
            (inside.right() + MARK + MARK_GAP, y),
            HorizontalAlign::Left,
            VerticalAlign::Center,
            label,
        );
    }

    // Axis names are horizontal, just outside the container at the far end
    // of their axis, lined up with its edges.
    let desc_y = if frame.ticks && frame.x_on_top {
        inside.y - X_BAND - 3.0
    } else {
        inside.y - 3.0
    };
    if let Some(desc) = frame.y_desc {
        text(
            layer,
            desc,
            (inside.x, desc_y),
            HorizontalAlign::Left,
            VerticalAlign::Bottom,
            label,
        );
    }
    if let Some(desc) = frame.y2_desc {
        text(
            layer,
            desc,
            (inside.right(), desc_y),
            HorizontalAlign::Right,
            VerticalAlign::Bottom,
            label,
        );
    }
    if let Some(desc) = frame.x_desc {
        let y = inside.bottom() + if frame.ticks { X_BAND } else { 0.0 } + 3.0;
        text(
            layer,
            desc,
            (inside.right(), y),
            HorizontalAlign::Right,
            VerticalAlign::Top,
            label,
        );
    }
}

/// Ticks that land on the container, not past its ends.
fn visible(ticks: &[Tick], length: f32, scale: f32) -> impl Iterator<Item = &Tick> {
    ticks.iter().filter(move |tick| {
        let at = tick.at as f32 / scale;
        (-0.5..=length + 0.5).contains(&at)
    })
}

fn mark(layer: &Layer, at: (f32, f32), size: (f32, f32)) {
    layer.draw_rectangle(at, size, theme::border(), Rounding::NONE);
}

fn text(
    layer: &Layer,
    text: &str,
    at: (f32, f32),
    horizontal: HorizontalAlign,
    vertical: VerticalAlign,
    parameters: FontParameters,
) {
    layer.draw_text(
        text,
        at,
        theme::dim(),
        Alignment {
            horizontal,
            vertical,
        },
        theme::sans(),
        parameters,
    );
}

/// The container's fill, in the theme's panel colour.
pub fn surface() -> Color {
    theme::popup()
}
