//! Recursive box layout for math expressions, reducing the TeX model to what
//! fractions need. Every node measures to `(width, ascent, descent)` around a
//! shared anchor line, lists concatenate boxes, and fractions stack operands
//! around the math axis. A nested fraction is simply a tall child whose
//! ancestors grow to hold it. Layout is pure and measured through a closure,
//! so every rule remains testable without a renderer.
//!
//! Box origins are anchor-left. Every child tuple stores `(x, y, box)` with
//! `y` positive upward; a denominator therefore has a negative y offset.

use crate::document::math::{AccentKind, BigOp, MathCursor, MathList, MathNode, Slot, Step};
use crate::theme::{self, TextStyle};

/// Math base size at script level 0. Matches body text so an inline expression
/// reads as part of the sentence it sits in.
pub const BASE_SIZE: f32 = 17.5;
/// Font scale per script level: full, script, scriptscript. Clamped at level
/// 2 because deeper TeX scripts do not shrink further.
pub const LEVEL_SCALE: [f32; 3] = [1.0, 0.78, 0.62];
/// Vertical clearance from the bar to each operand's near edge, as a fraction
/// of the current size.
pub const FRAC_GAP: f32 = 0.12;
/// The bar's horizontal overhang past the wider operand, in pixels at level 0
/// and scaled with the current level.
pub const FRAC_PAD: f32 = 3.0;
/// An empty slot's placeholder width before level scaling.
pub const SLOT_W: f32 = 12.0;
/// An empty slot's placeholder height before level scaling.
pub const SLOT_H: f32 = 16.0;
/// The fraction bar's thickness.
pub const BAR: f32 = 1.0;
/// How far a superscript's lower edge sits above the anchor line, as a
/// fraction of the current size — measured from the edge rather than the
/// centre so the gap does not change when the script grows taller.
pub const SCRIPT_RISE: f32 = 0.10;
/// The mirror for a subscript's upper edge below the anchor line.
pub const SCRIPT_DROP: f32 = 0.10;
/// How much of a group's body height its delimiters are drawn to fill.
pub const DELIM_FILL: f32 = 1.15;
/// Clearance between a radicand and the bar drawn over it.
pub const RADICAL_GAP: f32 = 0.10;
/// How much larger than surrounding text a large operator is drawn.
pub const BIGOP_SCALE: f32 = 1.6;
/// Clearance between a large operator and each of its limits.
pub const BIGOP_GAP: f32 = 0.12;
/// Stroke width for scalable math geometry at level zero. Height changes do
/// not change it, so tall delimiters stay the same visual weight as short ones.
pub const SHAPE_STROKE: f32 = 1.25;
/// Vertical clearance between an accent and the body it annotates.
pub const ACCENT_GAP: f32 = 0.10;

// The renderer centers glyphs vertically, so glyph boxes are symmetric about
// the anchor. Structural boxes use their near edge when clearance matters.

/// A laid-out node and its positioned children.
#[derive(Clone, Debug, PartialEq)]
pub struct MathBox {
    pub width: f32,
    /// Height above this box's anchor line.
    pub ascent: f32,
    /// Depth below this box's anchor line.
    pub descent: f32,
    /// Draw a rounded variable background behind this whole box.
    pub highlight: bool,
    pub kind: BoxKind,
}

/// Renderer-independent geometry emitted by math layout.
#[derive(Clone, Debug, PartialEq)]
pub enum MathPrimitive {
    Stroke {
        path: String,
        thickness: f32,
    },
    Dots {
        centers: Vec<(f32, f32)>,
        radius: f32,
    },
}

/// Visual shape and children of a [`MathBox`].
#[derive(Clone, Debug, PartialEq)]
pub enum BoxKind {
    Glyph {
        text: String,
        size: f32,
    },
    Bar {
        thickness: f32,
    },
    Slot {
        size: f32,
    },
    Primitive(MathPrimitive),
    /// Child y offsets are anchor-relative and positive upward.
    Row {
        children: Vec<(f32, f32, MathBox)>,
    },
}

/// Lay out `list` at display level `0`, script level `1`, or scriptscript
/// level `2+`. Scaling operands at the next level keeps nested fractions
/// bounded without introducing a second layout representation.
pub fn layout(list: &MathList, level: usize, measure: &dyn Fn(&str, &TextStyle) -> f32) -> MathBox {
    if list.is_empty() {
        return slot_box(level);
    }

    let mut children = Vec::with_capacity(list.len());
    let mut x = 0.0;
    for node in list {
        let child = layout_node(node, level, measure);
        children.push((x, 0.0, child));
        x += children.last().expect("child was pushed").2.width;
    }
    row_box(children)
}

fn scale(level: usize) -> f32 {
    LEVEL_SCALE[level.min(LEVEL_SCALE.len() - 1)]
}

fn size(level: usize) -> f32 {
    BASE_SIZE * scale(level)
}

fn glyph(ch: char, level: usize, measure: &dyn Fn(&str, &TextStyle) -> f32) -> MathBox {
    let size = size(level);
    let text = ch.to_string();
    let half = size * 0.5;
    MathBox {
        width: measure(&text, &TextStyle::math(size, theme::INK)),
        ascent: half,
        descent: half,
        highlight: ch.is_alphabetic(),
        kind: BoxKind::Glyph { text, size },
    }
}

fn slot_box(level: usize) -> MathBox {
    let scale = scale(level);
    let height = SLOT_H * scale;
    let half = height * 0.5;
    MathBox {
        width: SLOT_W * scale,
        ascent: half,
        descent: half,
        highlight: false,
        kind: BoxKind::Slot { size: height },
    }
}

fn layout_node(
    node: &MathNode,
    level: usize,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> MathBox {
    match node {
        MathNode::Sym(ch) => glyph(*ch, level, measure),
        MathNode::Frac { num, den } => fraction(node, num, den, level, measure),
        MathNode::Script { .. } => script(node, level, measure),
        MathNode::Group { open, close, body } => group(node, *open, *close, body, level, measure),
        MathNode::Sqrt { body } => radical(node, body, level, measure),
        MathNode::Accent { kind, body } => accent(node, *kind, body, level, measure),
        MathNode::BigOp { kind, lower, upper } => big_op(node, kind, lower, upper, level, measure),
    }
}

fn text_glyph(text: &str, size: f32, measure: &dyn Fn(&str, &TextStyle) -> f32) -> MathBox {
    let half = size * 0.5;
    MathBox {
        width: measure(text, &TextStyle::math(size, theme::INK)),
        ascent: half,
        descent: half,
        highlight: false,
        kind: BoxKind::Glyph {
            text: text.to_owned(),
            size,
        },
    }
}

fn big_op(
    node: &MathNode,
    kind: &BigOp,
    lower: &MathList,
    upper: &MathList,
    level: usize,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> MathBox {
    let (operator_text, operator_size) = match kind {
        BigOp::Sum => ("∑", size(level) * BIGOP_SCALE),
        BigOp::Prod => ("∏", size(level) * BIGOP_SCALE),
        BigOp::Integral => ("∫", size(level) * BIGOP_SCALE),
        BigOp::Limit => ("lim", size(level)),
    };
    let operator = text_glyph(operator_text, operator_size, measure);
    let operand_level = (level + 1).min(2);
    let lower = layout(lower, operand_level, measure);
    let upper = if *kind == BigOp::Limit {
        None
    } else {
        Some(layout(upper, operand_level, measure))
    };
    let width = operator
        .width
        .max(lower.width)
        .max(upper.as_ref().map_or(0.0, |upper| upper.width));
    let gap = size(level) * BIGOP_GAP;
    let operator_ascent = operator.ascent;
    let operator_descent = operator.descent;
    let lower_y = -(operator_descent + gap + lower.ascent);
    let mut children = (0..(1 + node.slots().len()))
        .map(|_| None)
        .collect::<Vec<_>>();
    children[0] = Some(((width - operator.width) * 0.5, 0.0, operator));
    children[slot_child_index(node, Slot::Lower).expect("big operator lower slot index")] =
        Some(((width - lower.width) * 0.5, lower_y, lower));
    if let Some(upper) = upper {
        let upper_y = operator_ascent + gap + upper.descent;
        children[slot_child_index(node, Slot::Upper).expect("big operator upper slot index")] =
            Some(((width - upper.width) * 0.5, upper_y, upper));
    }
    row_box(
        children
            .into_iter()
            .map(|child| child.expect("every big operator child must be laid out"))
            .collect(),
    )
}

fn stretchy_size(body: &MathBox, level: usize) -> f32 {
    size(level).max((body.ascent + body.descent) * DELIM_FILL)
}

fn stroked_box(width: f32, ascent: f32, descent: f32, path: String, level: usize) -> MathBox {
    MathBox {
        width,
        ascent,
        descent,
        highlight: false,
        kind: BoxKind::Primitive(MathPrimitive::Stroke {
            path,
            thickness: SHAPE_STROKE * scale(level),
        }),
    }
}

fn delimiter(ch: char, height: f32, level: usize) -> MathBox {
    let width = size(level) * 0.34;
    let stroke = SHAPE_STROKE * scale(level);
    let top = -height * 0.5 + stroke * 0.5;
    let bottom = height * 0.5 - stroke * 0.5;
    let path = match ch {
        '(' => format!(
            "M {width} {top} C 0 {top_half}, 0 {bottom_half}, {width} {bottom}",
            top_half = top * 0.45,
            bottom_half = bottom * 0.45,
        ),
        ')' => format!(
            "M 0 {top} C {width} {top_half}, {width} {bottom_half}, 0 {bottom}",
            top_half = top * 0.45,
            bottom_half = bottom * 0.45,
        ),
        '[' => format!("M {width} {top} H 0 V {bottom} H {width}"),
        ']' => format!("M 0 {top} H {width} V {bottom} H 0"),
        _ => format!("M {} {top} V {bottom}", width * 0.5),
    };
    stroked_box(width, height * 0.5, height * 0.5, path, level)
}

fn group(
    node: &MathNode,
    open: char,
    close: char,
    body: &MathList,
    level: usize,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> MathBox {
    let body = layout(body, level, measure);
    let delimiter_height = stretchy_size(&body, level);
    let opener = delimiter(open, delimiter_height, level);
    let closer = delimiter(close, delimiter_height, level);
    let body_x = opener.width;
    let closer_x = body_x + body.width;
    let mut children = (0..3).map(|_| None).collect::<Vec<_>>();
    children[0] = Some((0.0, 0.0, opener));
    children[slot_child_index(node, Slot::Body).expect("group body slot index")] =
        Some((body_x, 0.0, body));
    children[2] = Some((closer_x, 0.0, closer));
    row_box(
        children
            .into_iter()
            .map(|child| child.expect("every group child must be laid out"))
            .collect(),
    )
}

fn radical(
    node: &MathNode,
    body: &MathList,
    level: usize,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> MathBox {
    let body = layout(body, level, measure);
    let stroke = SHAPE_STROKE * scale(level);
    let body_x = size(level) * 0.48;
    let gap = size(level) * RADICAL_GAP;
    let top = -(body.ascent + gap + stroke * 0.5);
    let valley = body.descent * 0.65;
    let width = body_x + body.width;
    let path = format!(
        "M {} {} L {} {valley} L {} {} Q {} {top}, {body_x} {top} H {}",
        stroke * 0.5,
        body.descent * 0.1,
        body_x * 0.25,
        body_x * 0.48,
        -body.ascent * 0.12,
        body_x * 0.78,
        width - stroke * 0.5,
    );
    let sign = stroked_box(
        width,
        -top + stroke * 0.5,
        valley + stroke * 0.5,
        path,
        level,
    );
    let mut children = (0..2).map(|_| None).collect::<Vec<_>>();
    children[0] = Some((0.0, 0.0, sign));
    children[slot_child_index(node, Slot::Body).expect("radical body slot index")] =
        Some((body_x, 0.0, body));
    row_box(
        children
            .into_iter()
            .map(|child| child.expect("every radical child must be laid out"))
            .collect(),
    )
}

fn accent(
    node: &MathNode,
    kind: AccentKind,
    body: &MathList,
    level: usize,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> MathBox {
    let body = layout(body, level, measure);
    let stroke = SHAPE_STROKE * scale(level);
    let height = size(level) * 0.22;
    let y = -height * 0.45;
    let primitive = match kind {
        AccentKind::Vector => {
            let end = body.width - stroke * 0.5;
            let head = height * 0.75;
            MathPrimitive::Stroke {
                path: format!(
                    "M {} {y} H {end} M {} {} L {end} {y} L {} {}",
                    stroke * 0.5,
                    end - head,
                    y - head * 0.7,
                    end - head,
                    y + head * 0.7,
                ),
                thickness: stroke,
            }
        }
        AccentKind::Dot | AccentKind::DoubleDot | AccentKind::TripleDot => {
            let count = match kind {
                AccentKind::Dot => 1,
                AccentKind::DoubleDot => 2,
                AccentKind::TripleDot => 3,
                AccentKind::Vector => unreachable!(),
            };
            let spacing = stroke * 2.5;
            let start = body.width * 0.5 - spacing * (count as f32 - 1.0) * 0.5;
            MathPrimitive::Dots {
                centers: (0..count)
                    .map(|i| (start + i as f32 * spacing, y))
                    .collect(),
                radius: stroke,
            }
        }
    };
    let mark = MathBox {
        width: body.width,
        ascent: height,
        descent: 0.0,
        highlight: false,
        kind: BoxKind::Primitive(primitive),
    };
    let mark_y = body.ascent + size(level) * ACCENT_GAP;
    let mut children = vec![None, None];
    children[0] = Some((0.0, mark_y, mark));
    children[slot_child_index(node, Slot::Body).expect("accent body slot index")] =
        Some((0.0, 0.0, body));
    row_box(
        children
            .into_iter()
            .map(|child| child.expect("every accent child must be laid out"))
            .collect(),
    )
}

fn script(node: &MathNode, level: usize, measure: &dyn Fn(&str, &TextStyle) -> f32) -> MathBox {
    let operand_level = (level + 1).min(2);
    let slots = node.slots();
    let mut children = (0..slots.len()).map(|_| None).collect::<Vec<_>>();
    let mut base_width = 0.0;

    for slot in slots {
        let child_list = node.slot(slot).expect("script slots must resolve");
        let child = layout(
            child_list,
            if slot == Slot::Base {
                level
            } else {
                operand_level
            },
            measure,
        );
        let (x, y) = match slot {
            Slot::Base => {
                base_width = child.width;
                (0.0, 0.0)
            }
            Slot::Sup => (base_width, size(level) * SCRIPT_RISE + child.descent),
            Slot::Sub => (base_width, -(size(level) * SCRIPT_DROP + child.ascent)),
            Slot::Num | Slot::Den | Slot::Body | Slot::Lower | Slot::Upper => {
                unreachable!("fraction slots cannot be scripts")
            }
        };
        let index = slot_child_index(node, slot).expect("script slot must have a child index");
        children[index] = Some((x, y, child));
    }

    // `slots` drives both this order and `slot_child_index`, so traversal sees
    // exactly the children this layout emitted.
    let mut box_ = row_box(
        children
            .into_iter()
            .map(|child| child.expect("every script slot must be laid out"))
            .collect(),
    );
    box_.highlight = matches!(node, MathNode::Script { base, .. }
        if matches!(base.as_slice(), [MathNode::Sym(ch)] if ch.is_alphabetic()));
    box_
}

fn fraction(
    node: &MathNode,
    num: &MathList,
    den: &MathList,
    level: usize,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> MathBox {
    let operand_level = (level + 1).min(2);
    let numerator = layout(num, operand_level, measure);
    let denominator = layout(den, operand_level, measure);
    let current_size = size(level);
    let gap = current_size * FRAC_GAP;
    let pad = FRAC_PAD * scale(level);
    let width = numerator.width.max(denominator.width) + pad * 2.0;

    let bar = MathBox {
        width,
        ascent: BAR * 0.5,
        descent: BAR * 0.5,
        highlight: false,
        kind: BoxKind::Bar { thickness: BAR },
    };
    // Anchor line is the inline prose middle, so placing the bar at zero
    // aligns it with the math axis instead of lifting the whole fraction.
    let bar_y = 0.0;
    // Like scripts, place each operand by the edge facing the bar so lopsided
    // boxes keep the requested clearance.
    let clearance = BAR * 0.5 + gap;
    let numerator_offset = clearance + numerator.descent;
    let denominator_offset = clearance + denominator.ascent;
    let numerator_y = numerator_offset;
    let denominator_y = -denominator_offset;
    let mut children = (0..3).map(|_| None).collect::<Vec<_>>();
    children
        [slot_child_index(node, Slot::Num).expect("fraction numerator must have a child index")] =
        Some(((width - numerator.width) * 0.5, numerator_y, numerator));
    children[1] = Some((0.0, bar_y, bar));
    children[slot_child_index(node, Slot::Den)
        .expect("fraction denominator must have a child index")] = Some((
        (width - denominator.width) * 0.5,
        denominator_y,
        denominator,
    ));
    row_box(
        children
            .into_iter()
            .map(|child| child.expect("every fraction child must be laid out"))
            .collect(),
    )
}

fn slot_child_index(node: &MathNode, slot: Slot) -> Option<usize> {
    let slot_index = node
        .slots()
        .iter()
        .position(|candidate| *candidate == slot)?;
    // Structural slots follow each node's fixed visual children: delimiters
    // flank groups, a radical sign and bar precede its body, an operator
    // precedes limits, and a fraction bar sits between numerator and denominator.
    let visual_offset = match node {
        MathNode::Group { .. } => 1,
        MathNode::Sqrt { .. } | MathNode::Accent { .. } => 1,
        MathNode::BigOp { .. } => 1,
        MathNode::Frac { .. } => usize::from(slot_index > 0),
        MathNode::Sym(_) | MathNode::Script { .. } => 0,
    };
    Some(slot_index + visual_offset)
}

fn row_box(children: Vec<(f32, f32, MathBox)>) -> MathBox {
    let width = children
        .iter()
        .map(|(x, _, child)| x + child.width)
        .fold(0.0, f32::max);
    let ascent = children
        .iter()
        .map(|(_, y, child)| y + child.ascent)
        .fold(0.0, f32::max);
    let descent = children
        .iter()
        .map(|(_, y, child)| child.descent - y)
        .fold(0.0, f32::max);
    MathBox {
        width,
        ascent,
        descent,
        highlight: false,
        kind: BoxKind::Row { children },
    }
}

/// Resolve cursor to `(x, anchor_offset, height)`.
///
/// `anchor_offset` uses box coordinates: numerator anchors are positive,
/// denominator anchors are negative. Cursor height follows its current
/// list level rather than the total height of an enclosing fraction.
pub fn cursor_pos(
    list: &MathList,
    cursor: &MathCursor,
    level: usize,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> (f32, f32, f32) {
    let box_ = layout(list, level, measure);
    cursor_in(list, &box_, cursor, 0, level, 0.0, 0.0)
}

fn cursor_in(
    list: &MathList,
    box_: &MathBox,
    cursor: &MathCursor,
    path_index: usize,
    level: usize,
    origin_x: f32,
    origin_y: f32,
) -> (f32, f32, f32) {
    let BoxKind::Row { children } = &box_.kind else {
        return (origin_x, origin_y, cursor_height(list, level));
    };
    let index = cursor.index.min(list.len()).min(children.len());
    if path_index == cursor.path.len() {
        let x = children
            .get(index)
            .map_or(box_.width, |(child_x, _, _)| *child_x);
        return (origin_x + x, origin_y, cursor_height(list, level));
    }

    let step = cursor.path[path_index];
    let Some(node) = list.get(step.index) else {
        return (
            origin_x + children.get(index).map_or(box_.width, |(x, _, _)| *x),
            origin_y,
            cursor_height(list, level),
        );
    };
    let Some((_, _, parent)) = children.get(step.index) else {
        return (origin_x + box_.width, origin_y, cursor_height(list, level));
    };
    let Some((slot_list, slot_box, slot_x, slot_y)) = structural_slot(node, parent, step.slot)
    else {
        return (origin_x + box_.width, origin_y, cursor_height(list, level));
    };
    cursor_in(
        slot_list,
        slot_box,
        cursor,
        path_index + 1,
        child_level(node, step.slot, level),
        origin_x + children[step.index].0 + slot_x,
        origin_y + children[step.index].1 + slot_y,
    )
}

fn cursor_height(_list: &MathList, level: usize) -> f32 {
    size(level)
}

fn structural_slot<'a>(
    node: &'a MathNode,
    parent: &'a MathBox,
    slot: Slot,
) -> Option<(&'a MathList, &'a MathBox, f32, f32)> {
    let BoxKind::Row { children } = &parent.kind else {
        return None;
    };
    let list = node.slot(slot)?;
    let child = children.get(slot_child_index(node, slot)?)?;
    Some((list, &child.2, child.0, child.1))
}

fn child_level(node: &MathNode, slot: Slot, level: usize) -> usize {
    match node {
        MathNode::Script { .. } if slot == Slot::Base => level,
        MathNode::Group { .. } => level,
        MathNode::Sqrt { .. } => level,
        MathNode::Accent { .. } => level,
        _ => (level + 1).min(2),
    }
}

/// Hit-test a laid-out list and return its nearest model cursor.
///
/// A click descends into a structural slot only when it lies inside its
/// horizontal and vertical box. Bar and ambiguous points use the nearest
/// boundary in the current list, which keeps root-list fallback predictable.
pub fn hit(
    list: &MathList,
    point: (f32, f32),
    level: usize,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> MathCursor {
    let box_ = layout(list, level, measure);
    let mut cursor = MathCursor::default();
    hit_list(list, &box_, point, level, &mut cursor);
    cursor
}

fn hit_list(
    list: &MathList,
    box_: &MathBox,
    point: (f32, f32),
    level: usize,
    cursor: &mut MathCursor,
) {
    let BoxKind::Row { children } = &box_.kind else {
        cursor.index = 0;
        return;
    };
    cursor.index = nearest_boundary(children, point.0);

    for (index, node) in list.iter().enumerate() {
        if !node.is_structural() {
            continue;
        }
        let Some((child_x, child_y, parent)) = children.get(index) else {
            continue;
        };
        let Some((slot, slot_list, slot_box, slot_x, slot_y)) =
            clean_structural_hit(node, parent, *child_x, *child_y, point)
        else {
            continue;
        };
        cursor.path.push(Step { index, slot });
        hit_list(
            slot_list,
            slot_box,
            (point.0 - child_x - slot_x, point.1 - child_y - slot_y),
            child_level(node, slot, level),
            cursor,
        );
        return;
    }
}

fn clean_structural_hit<'a>(
    node: &'a MathNode,
    parent: &'a MathBox,
    parent_x: f32,
    parent_y: f32,
    point: (f32, f32),
) -> Option<(Slot, &'a MathList, &'a MathBox, f32, f32)> {
    for slot in node.slots() {
        let (list, child, child_x, child_y) = structural_slot(node, parent, slot)?;
        let left = parent_x + child_x;
        let right = left + child.width;
        let top = parent_y + child_y + child.ascent;
        let bottom = parent_y + child_y - child.descent;
        if point.0 > left && point.0 < right && point.1 < top && point.1 > bottom {
            return Some((slot, list, child, child_x, child_y));
        }
    }
    None
}

fn nearest_boundary(children: &[(f32, f32, MathBox)], x: f32) -> usize {
    children
        .iter()
        .position(|(child_x, _, child)| x < *child_x + child.width * 0.5)
        .unwrap_or(children.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_measure(text: &str, style: &TextStyle) -> f32 {
        text.chars().count() as f32 * style.size * 0.5
    }

    fn symbols(text: &str) -> MathList {
        text.chars().map(MathNode::Sym).collect()
    }

    fn fraction(num: MathList, den: MathList) -> MathNode {
        MathNode::Frac { num, den }
    }

    fn script(base: MathList, sup: Option<MathList>, sub: Option<MathList>) -> MathNode {
        MathNode::Script { base, sup, sub }
    }

    fn group(body: MathList) -> MathNode {
        MathNode::Group {
            open: '(',
            close: ')',
            body,
        }
    }

    fn radical(body: MathList) -> MathNode {
        MathNode::Sqrt { body }
    }

    fn accent(kind: AccentKind, body: MathList) -> MathNode {
        MathNode::Accent { kind, body }
    }

    fn big_op(kind: BigOp, lower: MathList, upper: MathList) -> MathNode {
        MathNode::BigOp { kind, lower, upper }
    }

    #[test]
    fn a_list_concatenates_widths_on_one_baseline() {
        let box_ = layout(&symbols("xy"), 0, &fake_measure);
        let BoxKind::Row { children } = box_.kind else {
            panic!("list must produce row");
        };
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].0, 0.0);
        assert_eq!(children[1].0, BASE_SIZE * 0.5);
        assert_eq!(children[0].1, 0.0);
        assert_eq!(children[1].1, 0.0);
        assert_eq!(box_.width, BASE_SIZE);
    }

    #[test]
    fn a_fraction_is_as_wide_as_its_wider_operand_plus_padding() {
        let list = vec![fraction(symbols("x"), symbols("yz"))];
        let box_ = layout(&list, 0, &fake_measure);
        let BoxKind::Row { children } = box_.kind else {
            panic!("list must produce row");
        };
        let operand = BASE_SIZE * LEVEL_SCALE[1];
        assert_eq!(children[0].2.width, operand + FRAC_PAD * 2.0);
    }

    #[test]
    fn operands_shrink_one_script_level() {
        let display = layout(&symbols("x"), 0, &fake_measure);
        let script = layout(&symbols("x"), 1, &fake_measure);
        assert_eq!(script.width, display.width * LEVEL_SCALE[1]);
        assert_eq!(script.ascent, display.ascent * LEVEL_SCALE[1]);

        let nested = layout(
            &vec![fraction(
                vec![fraction(symbols("x"), symbols("y"))],
                symbols("z"),
            )],
            0,
            &fake_measure,
        );
        let BoxKind::Row { children } = nested.kind else {
            panic!("fraction must produce row");
        };
        let BoxKind::Row {
            children: numerator,
        } = &children[0].2.kind
        else {
            panic!("nested fraction must produce row");
        };
        let BoxKind::Row {
            children: inner_num,
        } = &numerator[0].2.kind
        else {
            panic!("inner fraction must produce row");
        };
        let BoxKind::Row {
            children: inner_list,
        } = &inner_num[0].2.kind
        else {
            panic!("inner operand must produce row");
        };
        let BoxKind::Row { children: glyphs } = &inner_list[0].2.kind else {
            panic!("inner operand must produce row");
        };
        let BoxKind::Glyph { size, .. } = &glyphs[0].2.kind else {
            panic!("inner operand must be glyph");
        };
        assert_eq!(*size, BASE_SIZE * LEVEL_SCALE[2]);

        let three_deep = layout(
            &vec![fraction(
                vec![fraction(
                    vec![fraction(symbols("x"), symbols("y"))],
                    symbols("z"),
                )],
                symbols("w"),
            )],
            0,
            &fake_measure,
        );
        let BoxKind::Row { children } = three_deep.kind else {
            panic!("fraction must produce row");
        };
        let BoxKind::Row {
            children: level_one,
        } = &children[0].2.kind
        else {
            panic!("fraction must produce row");
        };
        let BoxKind::Row {
            children: level_two,
        } = &level_one[0].2.kind
        else {
            panic!("fraction must produce row");
        };
        let BoxKind::Row { children: inner } = &level_two[0].2.kind else {
            panic!("fraction must produce row");
        };
        let BoxKind::Row {
            children: inner_list,
        } = &inner[0].2.kind
        else {
            panic!("inner operand list must produce row");
        };
        let BoxKind::Row { children: deepest } = &inner_list[0].2.kind else {
            panic!("deepest fraction must produce row");
        };
        let BoxKind::Row { children: glyphs } = &deepest[0].2.kind else {
            panic!("deepest operand must produce row");
        };
        let BoxKind::Glyph { size, .. } = &glyphs[0].2.kind else {
            panic!("inner operand must be glyph");
        };
        assert_eq!(*size, BASE_SIZE * LEVEL_SCALE[2]);
    }

    #[test]
    fn a_nested_fraction_grows_its_ancestors() {
        let mut list = vec![fraction(symbols("a"), symbols("b"))];
        let mut previous = layout(&list, 0, &fake_measure);
        for _ in 0..3 {
            list = vec![fraction(list, symbols("c"))];
            let current = layout(&list, 0, &fake_measure);
            assert!(current.ascent + current.descent > previous.ascent + previous.descent);
            previous = current;
        }
    }

    #[test]
    fn an_empty_slot_is_a_fixed_box() {
        let root = layout(&Vec::new(), 0, &fake_measure);
        let box_ = layout(&Vec::new(), 1, &fake_measure);
        assert_eq!(box_.width / root.width, LEVEL_SCALE[1]);
        assert!((box_.ascent + box_.descent - SLOT_H * LEVEL_SCALE[1]).abs() < 0.0001);
        assert!(
            matches!(box_.kind, BoxKind::Slot { size } if (size - SLOT_H * LEVEL_SCALE[1]).abs() < 0.0001)
        );
    }

    #[test]
    fn num_and_den_are_centered_under_the_bar() {
        let list = vec![fraction(symbols("x"), symbols("yz"))];
        let box_ = layout(&list, 0, &fake_measure);
        let BoxKind::Row { children } = box_.kind else {
            panic!("list must produce row");
        };
        let BoxKind::Row { children: fraction } = &children[0].2.kind else {
            panic!("fraction must produce row");
        };
        assert_eq!(fraction[1].0, 0.0);
        assert_eq!(
            fraction[0].0 + fraction[0].2.width * 0.5,
            fraction[2].0 + fraction[2].2.width * 0.5
        );
    }

    #[test]
    fn the_bar_never_touches_the_operands() {
        let list = vec![fraction(
            vec![fraction(symbols("a"), symbols("b"))],
            vec![fraction(symbols("c"), symbols("d"))],
        )];
        let box_ = layout(&list, 0, &fake_measure);
        let BoxKind::Row { children } = box_.kind else {
            panic!("list must produce row");
        };
        let BoxKind::Row { children: fraction } = &children[0].2.kind else {
            panic!("fraction must produce row");
        };
        let num = &fraction[0];
        let bar = &fraction[1];
        let den = &fraction[2];
        let numerator_gap = num.1 - num.2.descent - (bar.1 + bar.2.ascent);
        let denominator_gap = bar.1 - bar.2.descent - (den.1 + den.2.ascent);
        assert!(numerator_gap > 0.0);
        assert!(denominator_gap > 0.0);
    }

    #[test]
    fn a_fraction_puts_its_operands_the_same_distance_from_the_bar() {
        let list = vec![fraction(
            vec![script(symbols("x"), None, Some(symbols("i")))],
            vec![fraction(symbols("yz"), symbols("w"))],
        )];
        let box_ = layout(&list, 0, &fake_measure);
        let BoxKind::Row { children } = box_.kind else {
            panic!("fraction must produce row");
        };
        let BoxKind::Row { children: fraction } = &children[0].2.kind else {
            panic!("fraction must produce row");
        };
        let numerator = &fraction[0];
        let bar = &fraction[1];
        let denominator = &fraction[2];
        let numerator_gap = numerator.1 - numerator.2.descent - (bar.1 + bar.2.ascent);
        let denominator_gap = bar.1 - bar.2.descent - (denominator.1 + denominator.2.ascent);
        assert!((numerator_gap - denominator_gap).abs() < 0.0001);
    }

    #[test]
    fn the_bar_sits_on_the_anchor_line_whatever_the_operands_are() {
        let list = vec![fraction(
            symbols("x"),
            vec![fraction(symbols("z"), symbols("w"))],
        )];
        let box_ = layout(&list, 0, &fake_measure);
        let BoxKind::Row { children } = box_.kind else {
            panic!("fraction must produce row");
        };
        let fraction_box = &children[0].2;
        let BoxKind::Row { children: fraction } = &fraction_box.kind else {
            panic!("fraction must produce row");
        };
        let numerator = &fraction[0];
        let bar = &fraction[1];
        let denominator = &fraction[2];
        let clearance = BAR * 0.5 + BASE_SIZE * FRAC_GAP;

        assert_eq!(bar.1, 0.0);
        assert!(
            (fraction_box.ascent - (clearance + numerator.2.ascent + numerator.2.descent)).abs()
                < 0.0001
        );
        assert!(
            (fraction_box.descent - (clearance + denominator.2.ascent + denominator.2.descent))
                .abs()
                < 0.0001
        );
    }

    #[test]
    fn a_scripted_operand_clears_the_bar() {
        let list = vec![fraction(
            vec![script(symbols("C"), None, Some(symbols("i")))],
            vec![script(symbols("M"), Some(symbols("s")), None)],
        )];
        let box_ = layout(&list, 0, &fake_measure);
        let BoxKind::Row { children } = box_.kind else {
            panic!("fraction must produce row");
        };
        let BoxKind::Row { children: fraction } = &children[0].2.kind else {
            panic!("fraction must produce row");
        };
        let numerator = &fraction[0];
        let bar = &fraction[1];
        let denominator = &fraction[2];
        let expected_gap = BASE_SIZE * FRAC_GAP;
        let numerator_gap = numerator.1 - numerator.2.descent - (bar.1 + bar.2.ascent);
        let denominator_gap = bar.1 - bar.2.descent - (denominator.1 + denominator.2.ascent);

        assert!((numerator_gap - expected_gap).abs() < 0.0001);
        assert!((denominator_gap - expected_gap).abs() < 0.0001);
    }

    #[test]
    fn a_lopsided_operand_is_placed_by_the_edge_that_faces_the_bar() {
        let subscript = layout(
            &vec![fraction(
                vec![script(symbols("x"), None, Some(symbols("i")))],
                symbols("y"),
            )],
            0,
            &fake_measure,
        );
        let superscript = layout(
            &vec![fraction(
                vec![script(symbols("x"), Some(symbols("i")), None)],
                symbols("y"),
            )],
            0,
            &fake_measure,
        );
        let BoxKind::Row {
            children: subscript_children,
        } = subscript.kind
        else {
            panic!("fraction must produce row");
        };
        let BoxKind::Row {
            children: superscript_children,
        } = superscript.kind
        else {
            panic!("fraction must produce row");
        };
        let BoxKind::Row {
            children: subscript_fraction,
        } = &subscript_children[0].2.kind
        else {
            panic!("fraction must produce row");
        };
        let BoxKind::Row {
            children: superscript_fraction,
        } = &superscript_children[0].2.kind
        else {
            panic!("fraction must produce row");
        };
        let subscript_numerator = &subscript_fraction[0].2;
        let superscript_numerator = &superscript_fraction[0].2;

        assert!(
            (subscript_numerator.ascent + subscript_numerator.descent
                - superscript_numerator.ascent
                - superscript_numerator.descent)
                .abs()
                < 0.0001
        );
        assert!(subscript_fraction[0].1 > superscript_fraction[0].1);
    }

    #[test]
    fn cursor_pos_walks_into_a_denominator() {
        let list = vec![fraction(symbols("x"), symbols("yz"))];
        let cursor = MathCursor {
            path: vec![Step {
                index: 0,
                slot: Slot::Den,
            }],
            index: 1,
        };
        let (x, y, _) = cursor_pos(&list, &cursor, 0, &fake_measure);
        assert_eq!(x, FRAC_PAD + BASE_SIZE * LEVEL_SCALE[1] * 0.5);
        assert_eq!(
            y,
            -(BAR * 0.5 + BASE_SIZE * FRAC_GAP + BASE_SIZE * LEVEL_SCALE[1] * 0.5)
        );

        let (root_x, root_y, _) = cursor_pos(&list, &MathCursor::default(), 0, &fake_measure);
        assert_eq!(root_x, 0.0);
        assert_eq!(root_y, 0.0);
        assert!(x <= layout(&list, 0, &fake_measure).width);
    }

    #[test]
    fn cursor_height_matches_the_level() {
        let root = vec![fraction(symbols("x"), symbols("y"))];
        let (_, _, root_height) = cursor_pos(&root, &MathCursor::default(), 0, &fake_measure);
        let in_operand = MathCursor {
            path: vec![Step {
                index: 0,
                slot: Slot::Num,
            }],
            index: 0,
        };
        let (_, _, operand_height) = cursor_pos(&root, &in_operand, 0, &fake_measure);
        assert_eq!(root_height, BASE_SIZE);
        assert_eq!(operand_height, BASE_SIZE * LEVEL_SCALE[1]);
        assert_eq!(operand_height / root_height, LEVEL_SCALE[1]);
    }

    #[test]
    fn a_superscript_sits_above_the_anchor_and_a_subscript_below() {
        let list = vec![script(symbols("x"), Some(symbols("s")), Some(symbols("i")))];
        let BoxKind::Row { children } = &layout(&list, 0, &fake_measure).kind else {
            panic!("list must produce row");
        };
        let BoxKind::Row { children: script } = &children[0].2.kind else {
            panic!("script must produce row");
        };
        let sup = &script[1];
        let sub = &script[2];
        assert!((sup.1 - sup.2.descent - BASE_SIZE * SCRIPT_RISE).abs() < 0.0001);
        assert!((-sub.1 - sub.2.ascent - BASE_SIZE * SCRIPT_DROP).abs() < 0.0001);
    }

    #[test]
    fn both_scripts_share_the_column_after_the_base() {
        let list = vec![script(
            symbols("xy"),
            Some(symbols("s")),
            Some(symbols("long")),
        )];
        let BoxKind::Row { children } = &layout(&list, 0, &fake_measure).kind else {
            panic!("list must produce row");
        };
        let script_box = &children[0].2;
        let BoxKind::Row { children: script } = &script_box.kind else {
            panic!("script must produce row");
        };
        let base_width = script[0].2.width;
        assert_eq!(script[1].0, base_width);
        assert_eq!(script[2].0, base_width);
        assert_eq!(
            script_box.width,
            base_width + script[1].2.width.max(script[2].2.width)
        );
    }

    #[test]
    fn a_script_grows_the_expression_that_holds_it() {
        let plain = layout(&symbols("x"), 0, &fake_measure);
        let scripted = layout(
            &vec![script(
                symbols("x"),
                Some(vec![fraction(symbols("a"), symbols("b"))]),
                None,
            )],
            0,
            &fake_measure,
        );
        let BoxKind::Row { children } = scripted.kind else {
            panic!("list must produce row");
        };
        assert!(children[0].2.ascent > plain.ascent);
    }

    #[test]
    fn an_absent_script_costs_nothing() {
        let base = layout(&symbols("xy"), 0, &fake_measure);
        let scripted = layout(&vec![script(symbols("xy"), None, None)], 0, &fake_measure);
        let BoxKind::Row { children } = scripted.kind else {
            panic!("list must produce row");
        };
        assert_eq!(children[0].2.width, base.width);
    }

    #[test]
    fn scripts_shrink_one_level() {
        let list = vec![script(
            symbols("x"),
            Some(vec![
                MathNode::Sym('s'),
                script(symbols("t"), Some(symbols("u")), None),
            ]),
            None,
        )];
        let BoxKind::Row { children } = &layout(&list, 0, &fake_measure).kind else {
            panic!("list must produce row");
        };
        let BoxKind::Row { children: outer } = &children[0].2.kind else {
            panic!("script must produce row");
        };
        let BoxKind::Row { children: sup } = &outer[1].2.kind else {
            panic!("superscript must produce row");
        };
        let BoxKind::Glyph { size: sup_size, .. } = &sup[0].2.kind else {
            panic!("superscript symbol must be a glyph");
        };
        assert_eq!(*sup_size, BASE_SIZE * LEVEL_SCALE[1]);
        let BoxKind::Row { children: nested } = &sup[1].2.kind else {
            panic!("nested script must produce row");
        };
        let BoxKind::Row {
            children: nested_sup,
        } = &nested[1].2.kind
        else {
            panic!("nested superscript must produce row");
        };
        let BoxKind::Glyph {
            size: nested_size, ..
        } = &nested_sup[0].2.kind
        else {
            panic!("nested superscript symbol must be a glyph");
        };
        assert_eq!(*nested_size, BASE_SIZE * LEVEL_SCALE[2]);
    }

    #[test]
    fn the_cursor_reaches_a_script_slot() {
        let list = vec![script(symbols("x"), Some(symbols("s")), Some(symbols("i")))];
        let base_width = layout(&symbols("x"), 0, &fake_measure).width;
        let sup_cursor = MathCursor {
            path: vec![Step {
                index: 0,
                slot: Slot::Sup,
            }],
            index: 1,
        };
        let sub_cursor = MathCursor {
            path: vec![Step {
                index: 0,
                slot: Slot::Sub,
            }],
            index: 0,
        };
        let (sup_x, sup_y, _) = cursor_pos(&list, &sup_cursor, 0, &fake_measure);
        let (sub_x, sub_y, _) = cursor_pos(&list, &sub_cursor, 0, &fake_measure);
        assert!(sup_x > base_width);
        assert_eq!(sub_x, base_width);
        assert!(sup_y > 0.0);
        assert!(sub_y < 0.0);
    }

    #[test]
    fn a_bracket_grows_with_what_it_holds() {
        let plain = layout(&symbols("x"), 0, &fake_measure);
        let single = layout(&vec![group(symbols("x"))], 0, &fake_measure);
        let nested = layout(
            &vec![group(vec![fraction(symbols("x"), symbols("y"))])],
            0,
            &fake_measure,
        );
        let BoxKind::Row {
            children: single_list,
        } = single.kind
        else {
            panic!("group list must produce row");
        };
        let BoxKind::Row {
            children: single_group,
        } = &single_list[0].2.kind
        else {
            panic!("group must produce row");
        };
        let BoxKind::Primitive(MathPrimitive::Stroke {
            thickness: single_stroke,
            ..
        }) = &single_group[0].2.kind
        else {
            panic!("group opener must be stroked geometry");
        };
        let BoxKind::Row {
            children: nested_list,
        } = nested.kind
        else {
            panic!("group list must produce row");
        };
        let BoxKind::Row {
            children: nested_group,
        } = &nested_list[0].2.kind
        else {
            panic!("group must produce row");
        };
        let BoxKind::Primitive(MathPrimitive::Stroke {
            thickness: nested_stroke,
            ..
        }) = &nested_group[0].2.kind
        else {
            panic!("group opener must be stroked geometry");
        };

        assert!(nested_group[0].2.ascent > single_group[0].2.ascent);
        assert_eq!(nested_stroke, single_stroke);
        assert!(nested_list[0].2.ascent + nested_list[0].2.descent > plain.ascent + plain.descent);
    }

    #[test]
    fn a_bracket_is_never_smaller_than_its_text() {
        let list = layout(&vec![group(symbols("x"))], 0, &fake_measure);
        let BoxKind::Row { children } = list.kind else {
            panic!("group list must produce row");
        };
        let BoxKind::Row { children: group } = &children[0].2.kind else {
            panic!("group must produce row");
        };
        assert!(group[0].2.ascent + group[0].2.descent >= BASE_SIZE);
        assert!(group[2].2.ascent + group[2].2.descent >= BASE_SIZE);
        assert!(matches!(group[0].2.kind, BoxKind::Primitive(_)));
        assert!(matches!(group[2].2.kind, BoxKind::Primitive(_)));
    }

    #[test]
    fn a_radical_covers_its_body() {
        let list = layout(&vec![radical(symbols("xy"))], 0, &fake_measure);
        let BoxKind::Row { children } = list.kind else {
            panic!("radical list must produce row");
        };
        let radical_box = &children[0].2;
        let BoxKind::Row { children: radical } = &radical_box.kind else {
            panic!("radical must produce row");
        };
        let sign = &radical[0];
        let body = &radical[1];
        let BoxKind::Primitive(MathPrimitive::Stroke { path, thickness }) = &sign.2.kind else {
            panic!("radical must be one connected stroke");
        };
        assert_eq!(*thickness, SHAPE_STROKE);
        assert_eq!(path.matches('M').count(), 1);
        assert_eq!(sign.2.width, body.0 + body.2.width);
        assert!(sign.2.ascent > body.2.ascent);
        assert!(radical_box.ascent >= sign.2.ascent);
    }

    #[test]
    fn accents_cover_their_body_without_changing_its_level() {
        for kind in [
            AccentKind::Vector,
            AccentKind::Dot,
            AccentKind::DoubleDot,
            AccentKind::TripleDot,
        ] {
            let list = layout(&vec![accent(kind, symbols("xy"))], 0, &fake_measure);
            let BoxKind::Row { children } = list.kind else {
                panic!("accent list must produce row");
            };
            let BoxKind::Row { children: accent } = &children[0].2.kind else {
                panic!("accent must produce row");
            };
            let mark = &accent[0];
            let body = &accent[1];
            assert_eq!(mark.2.width, body.2.width);
            assert!(mark.1 - mark.2.descent > body.1 + body.2.ascent);
            assert_eq!(body.2.width, BASE_SIZE);
        }
    }

    #[test]
    fn alphabetic_and_greek_symbols_are_highlighted() {
        let box_ = layout(
            &vec![MathNode::Sym('x'), MathNode::Sym('α'), MathNode::Sym('2')],
            0,
            &fake_measure,
        );
        let BoxKind::Row { children } = box_.kind else {
            panic!("list must produce row");
        };
        assert!(children[0].2.highlight);
        assert!(children[1].2.highlight);
        assert!(!children[2].2.highlight);
    }

    #[test]
    fn a_single_variable_base_highlights_the_whole_script_box() {
        let list = layout(
            &vec![script(symbols("x"), None, Some(symbols("12")))],
            0,
            &fake_measure,
        );
        let BoxKind::Row { children } = list.kind else {
            panic!("list must produce row");
        };
        let script = &children[0].2;
        assert!(script.highlight);
        assert!(script.descent > BASE_SIZE * 0.5);
        assert!(script.width > BASE_SIZE * 0.5);
    }

    #[test]
    fn a_big_operator_stacks_its_limits() {
        let list = layout(
            &vec![big_op(BigOp::Sum, symbols("long"), symbols("u"))],
            0,
            &fake_measure,
        );
        let BoxKind::Row { children } = list.kind else {
            panic!("big operator list must produce row");
        };
        let op_box = &children[0].2;
        let BoxKind::Row { children: op } = &op_box.kind else {
            panic!("big operator must produce row");
        };
        let operator = &op[0];
        let lower = &op[1];
        let upper = &op[2];
        let operator_center = operator.0 + operator.2.width * 0.5;
        assert!((operator_center - (op_box.width * 0.5)).abs() < 0.0001);
        assert!((lower.0 + lower.2.width * 0.5 - op_box.width * 0.5).abs() < 0.0001);
        assert!((upper.0 + upper.2.width * 0.5 - op_box.width * 0.5).abs() < 0.0001);
        assert!(upper.1 - upper.2.descent > operator.1 + operator.2.ascent);
        assert!(lower.1 + lower.2.ascent < operator.1 - operator.2.descent);
        assert!(matches!(
            operator.2.kind,
            BoxKind::Glyph { size, .. } if size == BASE_SIZE * BIGOP_SCALE
        ));
    }

    #[test]
    fn a_limit_has_no_upper_child() {
        let list = layout(
            &vec![big_op(BigOp::Limit, symbols("x"), symbols("ignored"))],
            0,
            &fake_measure,
        );
        let BoxKind::Row { children } = list.kind else {
            panic!("limit list must produce row");
        };
        let BoxKind::Row { children: limit } = &children[0].2.kind else {
            panic!("limit must produce row");
        };
        assert_eq!(limit.len(), 2);
        assert!(matches!(
            limit[0].2.kind,
            BoxKind::Glyph { ref text, size } if text == "lim" && size == BASE_SIZE
        ));
    }

    #[test]
    fn the_cursor_reaches_every_new_slot() {
        let group_list = vec![group(symbols("xy"))];
        let group_box = layout(&group_list, 0, &fake_measure);
        let BoxKind::Row { children } = &group_box.kind else {
            panic!("group list must produce row");
        };
        let BoxKind::Row { children: group } = &children[0].2.kind else {
            panic!("group must produce row");
        };
        let group_cursor = MathCursor {
            path: vec![Step {
                index: 0,
                slot: Slot::Body,
            }],
            index: 1,
        };
        let (group_x, _, _) = cursor_pos(&group_list, &group_cursor, 0, &fake_measure);
        assert!(group_x > group[1].0 && group_x < group[1].0 + group[1].2.width);

        let radical_list = vec![radical(symbols("xy"))];
        let radical_box = layout(&radical_list, 0, &fake_measure);
        let BoxKind::Row { children } = &radical_box.kind else {
            panic!("radical list must produce row");
        };
        let BoxKind::Row { children: radical } = &children[0].2.kind else {
            panic!("radical must produce row");
        };
        let radical_cursor = MathCursor {
            path: vec![Step {
                index: 0,
                slot: Slot::Body,
            }],
            index: 1,
        };
        let (radical_x, _, _) = cursor_pos(&radical_list, &radical_cursor, 0, &fake_measure);
        assert!(radical_x > radical[1].0 && radical_x < radical[1].0 + radical[1].2.width);

        let accent_list = vec![accent(AccentKind::Vector, symbols("xy"))];
        let accent_cursor = MathCursor {
            path: vec![Step {
                index: 0,
                slot: Slot::Body,
            }],
            index: 1,
        };
        let (accent_x, _, _) = cursor_pos(&accent_list, &accent_cursor, 0, &fake_measure);
        assert!(accent_x > 0.0 && accent_x < layout(&accent_list, 0, &fake_measure).width);

        let sum_list = vec![big_op(BigOp::Sum, symbols("lo"), symbols("up"))];
        let sum_box = layout(&sum_list, 0, &fake_measure);
        let BoxKind::Row { children } = &sum_box.kind else {
            panic!("sum list must produce row");
        };
        let BoxKind::Row { children: sum } = &children[0].2.kind else {
            panic!("sum must produce row");
        };
        for slot in [Slot::Lower, Slot::Upper] {
            let cursor = MathCursor {
                path: vec![Step { index: 0, slot }],
                index: 1,
            };
            let (x, _, _) = cursor_pos(&sum_list, &cursor, 0, &fake_measure);
            let child = &sum[slot_child_index(&sum_list[0], slot).expect("sum slot index")];
            assert!(x > child.0 && x < child.0 + child.2.width);
        }
    }
}
