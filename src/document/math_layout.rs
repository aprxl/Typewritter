//! Recursive box layout for math expressions, reducing the TeX model to what
//! fractions need. Every node measures to `(width, ascent, descent)` around a
//! shared anchor line, lists concatenate boxes, and fractions stack operands
//! around the math axis. A nested fraction is simply a tall child whose
//! ancestors grow to hold it. Layout is pure and measured through a closure,
//! so every rule remains testable without a renderer.
//!
//! Box origins are anchor-left. Every child tuple stores `(x, y, box)` with
//! `y` positive upward; a denominator therefore has a negative y offset.

use crate::document::math::{MathCursor, MathList, MathNode, Slot, Step};
use crate::theme::{self, TextStyle};

/// Math base size at script level 0. Matches body text so an inline expression
/// reads as part of the sentence it sits in.
pub const BASE_SIZE: f32 = 17.5;
/// Font scale per script level: full, script, scriptscript. Clamped at level
/// 2 because deeper TeX scripts do not shrink further.
pub const LEVEL_SCALE: [f32; 3] = [1.0, 0.78, 0.62];
/// Vertical clearance between the bar and each operand, as a fraction of the
/// current size.
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

// The renderer centers glyphs vertically, so the anchor line is a center line.
// Symmetric boxes make equal clearance above and below the line exact.

/// A laid-out node and its positioned children.
#[derive(Clone, Debug, PartialEq)]
pub struct MathBox {
    pub width: f32,
    /// Height above this box's anchor line.
    pub ascent: f32,
    /// Depth below this box's anchor line.
    pub descent: f32,
    pub kind: BoxKind,
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
        MathNode::Frac { num, den } => fraction(num, den, level, measure),
    }
}

fn fraction(
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
        kind: BoxKind::Bar { thickness: BAR },
    };
    // Anchor line is the inline prose middle, so placing the bar at zero
    // aligns it with the math axis instead of lifting the whole fraction.
    let bar_y = 0.0;
    let numerator_half = (numerator.ascent + numerator.descent) * 0.5;
    let denominator_half = (denominator.ascent + denominator.descent) * 0.5;
    let numerator_offset = BAR * 0.5 + gap + numerator_half;
    let denominator_offset = BAR * 0.5 + gap + denominator_half;
    let numerator_y = numerator_offset;
    let denominator_y = -denominator_offset;
    let children = vec![
        ((width - numerator.width) * 0.5, numerator_y, numerator),
        (0.0, bar_y, bar),
        (
            (width - denominator.width) * 0.5,
            denominator_y,
            denominator,
        ),
    ];
    row_box(children)
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
    let Some(MathNode::Frac { num, den }) = list.get(step.index) else {
        return (
            origin_x + children.get(index).map_or(box_.width, |(x, _, _)| *x),
            origin_y,
            cursor_height(list, level),
        );
    };
    let Some((_, _, parent)) = children.get(step.index) else {
        return (origin_x + box_.width, origin_y, cursor_height(list, level));
    };
    let Some((slot_list, slot_box, slot_x, slot_y)) = fraction_slot(parent, step.slot, num, den)
    else {
        return (origin_x + box_.width, origin_y, cursor_height(list, level));
    };
    cursor_in(
        slot_list,
        slot_box,
        cursor,
        path_index + 1,
        (level + 1).min(2),
        origin_x + children[step.index].0 + slot_x,
        origin_y + children[step.index].1 + slot_y,
    )
}

fn cursor_height(_list: &MathList, level: usize) -> f32 {
    size(level)
}

fn fraction_slot<'a>(
    parent: &'a MathBox,
    slot: Slot,
    num: &'a MathList,
    den: &'a MathList,
) -> Option<(&'a MathList, &'a MathBox, f32, f32)> {
    let BoxKind::Row { children } = &parent.kind else {
        return None;
    };
    let (list, child) = match slot {
        Slot::Num => (num, children.first()?),
        Slot::Den => (den, children.get(2)?),
    };
    Some((list, &child.2, child.0, child.1))
}

/// Hit-test a laid-out list and return its nearest model cursor.
///
/// A click descends into a fraction only when it lies inside one operand's
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
        let MathNode::Frac { num, den } = node else {
            continue;
        };
        let Some((child_x, child_y, parent)) = children.get(index) else {
            continue;
        };
        let Some((slot, slot_list, slot_box, slot_x, slot_y)) =
            clean_fraction_hit(parent, *child_x, *child_y, point, num, den)
        else {
            continue;
        };
        cursor.path.push(Step { index, slot });
        hit_list(
            slot_list,
            slot_box,
            (point.0 - child_x - slot_x, point.1 - child_y - slot_y),
            (level + 1).min(2),
            cursor,
        );
        return;
    }
}

fn clean_fraction_hit<'a>(
    parent: &'a MathBox,
    parent_x: f32,
    parent_y: f32,
    point: (f32, f32),
    num: &'a MathList,
    den: &'a MathList,
) -> Option<(Slot, &'a MathList, &'a MathBox, f32, f32)> {
    let BoxKind::Row { children } = &parent.kind else {
        return None;
    };
    for (slot, list, child) in [
        (Slot::Num, num, children.first()?),
        (Slot::Den, den, children.get(2)?),
    ] {
        let left = parent_x + child.0;
        let right = left + child.2.width;
        let top = parent_y + child.1 + child.2.ascent;
        let bottom = parent_y + child.1 - child.2.descent;
        if point.0 > left && point.0 < right && point.1 < top && point.1 > bottom {
            return Some((slot, list, &child.2, child.0, child.1));
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
            symbols("x"),
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
    fn a_fraction_is_centred_on_its_own_anchor_line() {
        let one_level = layout(
            &vec![fraction(symbols("x"), symbols("y"))],
            0,
            &fake_measure,
        );
        assert_eq!(one_level.ascent, one_level.descent);

        let two_deep = layout(
            &vec![fraction(
                vec![fraction(symbols("x"), symbols("y"))],
                vec![fraction(symbols("z"), symbols("w"))],
            )],
            0,
            &fake_measure,
        );
        assert_eq!(two_deep.ascent, two_deep.descent);
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
}
