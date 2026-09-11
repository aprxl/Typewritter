//! Recursive box layout for math expressions, reducing the TeX model to what
//! fractions need. Every node measures to `(width, ascent, descent)` around a
//! shared anchor line, lists concatenate boxes, and fractions stack operands
//! around the math axis. A nested fraction is simply a tall child whose
//! ancestors grow to hold it. Layout is pure and measured through a closure,
//! so every rule remains testable without a renderer.
//!
//! Box origins are anchor-left. Every child tuple stores `(x, y, box)` with
//! `y` positive upward; a denominator therefore has a negative y offset.

use crate::document::math::{
    AccentKind, BigOp, MathCursor, MathList, MathNode, NodeAddress, Slot, Step, SymbolRole,
};
use crate::document::math_style::{self, SymbolStyle};
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
/// How wide every glyph set from the font is, as a fraction of the advance
/// the typeface gives it.
///
/// JuliaMono is a monospace face, and a monospace cell is sized for the
/// widest thing that has to fit in it — so math set at its natural advance
/// reads loose: letters sit in boxes wider than their ink, and an expression
/// gets long fast. One global squeeze is the fix, applied where the glyph
/// boxes are built rather than at draw time, so the reserved width and the
/// rendered ink are the same number by construction (see
/// [`TextStyle::condensed`]).
///
/// It multiplies a glyph's own ratio: `∫` and `∑` are condensed further than
/// this, and land narrower still. Tune this one by eye — it is the single
/// number that sets how tight every expression in the document reads.
pub const MATH_CONDENSE: f32 = 0.85;
/// Stroke width for scalable math geometry at level zero. Height changes do
/// not change it, so tall delimiters stay the same visual weight as short ones.
pub const SHAPE_STROKE: f32 = 1.25;
/// Vertical clearance between an accent and the body it annotates.
pub const ACCENT_GAP: f32 = 0.10;
/// Space inside the rounded background of a variable at level zero.
pub const VARIABLE_PAD_X: f32 = 3.0;
pub const VARIABLE_PAD_Y: f32 = 2.0;
/// The gap that separates a numeral's digit groups, as a fraction of the
/// current math size. A thin space rather than a full one: `1 000` has to
/// read as one number that is easy to count, not as two.
pub const DIGIT_GROUP_SPACE: f32 = 3.0 / 18.0;
/// How many digits a run needs before it is grouped at all. Four, so `1000`
/// is set `1 000` — the threshold the reader asked for, and the one every
/// three-digit number falls safely under.
pub const DIGIT_GROUP_MIN: usize = 4;
/// Optical overlap between adjacent integral-family signs at level zero, so
/// `∫∫` nests the way a double integral is set. About a sixth of the sign's
/// width — the condensed glyph is narrower than a normal operator, so the
/// pixel count came down with it rather than doubling the nesting.
pub const INTEGRAL_OVERLAP: f32 = 1.5;
/// How far the integral sign's glyph is condensed horizontally, as a faux
/// width ratio handed to [`TextStyle::condensed`]. JuliaMono's `∫` and `∮`
/// are full-width monospace cells, so at [`BIGOP_SCALE`] they read as a
/// wide, heavy swash rather than the tall narrow sign the notation has.
/// Condensing the outline re-narrows the ink and thins the spine while
/// keeping the font's own curve — so its edges stay exactly as smooth as
/// every glyph around it. Worth tuning by eye, beside [`MATH_CONDENSE`].
///
/// Relative to the typeface's natural advance, like every other ratio here:
/// [`MATH_CONDENSE`] applies on top, so the sign is drawn at
/// `INTEGRAL_CONDENSE * MATH_CONDENSE`.
pub const INTEGRAL_CONDENSE: f32 = 0.6;
/// How far the integral sign's glyph is lifted above the anchor line, as a
/// fraction of the operator's size. JuliaMono's `∫` is a text glyph whose
/// ink leans below the em box's centre (its bottom hook hangs like a
/// descender), so centring the box leaves the sign riding low and clipping
/// the lower limit. Lifting it re-centres the ink on the math axis. Tune by
/// eye, alongside [`INTEGRAL_CONDENSE`].
pub const INTEGRAL_RISE: f32 = 0.15;
/// JuliaMono's summation sign is broad enough at display size that its
/// diagonals read heavier than the surrounding notation. Narrowing the
/// outline gives it the same optical weight as the integral family.
pub const SUM_CONDENSE: f32 = 0.76;
/// The summation glyph is bottom-biased in its cell. Lift its ink onto the
/// math axis without moving the limits that are centred around its box.
pub const SUM_RISE: f32 = 0.08;
/// TeX's automatic inter-atom spacing, as fractions of the current math size
/// (an em, where `1mu = 1/18 em`). Thin sits after punctuation, medium braces
/// a binary operator, thick braces a relation. [`layout_inner`] reads them
/// from the classes of adjacent atoms.
pub const THIN_SPACE: f32 = 3.0 / 18.0;
pub const MED_SPACE: f32 = 4.0 / 18.0;
pub const THICK_SPACE: f32 = 5.0 / 18.0;

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
    /// Draw a semantic highlight around this whole box — the hue that says
    /// which symbol it is and the shape that says what role it plays.
    /// Resolved here rather than at paint time so a `MathBox` is two bytes
    /// heavier instead of one `String` per symbol heavier.
    pub highlight: Option<SymbolStyle>,
    pub kind: BoxKind,
}

/// The full area in which an expression can receive structural input.
/// Unlike [`MathBox`]'s visual measurements, this includes non-painting
/// children such as empty integral limits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InteractionBounds {
    pub left: f32,
    pub right: f32,
    pub ascent: f32,
    pub descent: f32,
}

/// Visual bounds of an addressed node, in the expression's coordinate space.
/// Math layout uses an upward-positive y axis, so `top >= bottom`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodeBounds {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

/// Renderer-independent geometry emitted by math layout.
#[derive(Clone, Debug, PartialEq)]
pub enum MathPrimitive {
    Stroke {
        path: String,
        thickness: f32,
        ink: MathInk,
    },
    Dots {
        centers: Vec<(f32, f32)>,
        radius: f32,
    },
}

/// Which ink a glyph is set in. Not a colour — the palette owns those —
/// but the kind of thing the glyph is, which is what chooses one.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum MathInk {
    /// A name: the thing an expression is about. Set in the document's ink.
    #[default]
    Term,
    /// A numeral.
    Number,
    /// An operator, relation, or punctuation mark — the grammar that joins
    /// terms rather than one of the terms.
    Operator,
}

/// Visual shape and children of a [`MathBox`].
#[derive(Clone, Debug, PartialEq)]
pub enum BoxKind {
    Glyph {
        text: String,
        size: f32,
        /// Which of the notation inks sets this glyph.
        ink: MathInk,
        /// Horizontal offset from the box edge to the glyph itself.
        offset_x: f32,
        /// Vertical offset of the glyph's centre from the box's anchor line,
        /// positive upward. The integral rides above the anchor because its
        /// ink is bottom-biased in JuliaMono's em box.
        offset_y: f32,
        /// Faux condense ratio (`< 1.0` narrower), handed to the renderer.
        condense: f32,
    },
    Bar {
        thickness: f32,
    },
    Slot {
        size: f32,
        visible: bool,
    },
    Primitive(MathPrimitive),
    /// Child y offsets are anchor-relative and positive upward.
    Row {
        children: Vec<(f32, f32, MathBox)>,
    },
}

/// Lay out `list` at display level `0`, script level `1`, or scriptscript
/// level `2+`, multiplied by `document_scale`. Scaling operands at the next
/// level keeps nested fractions bounded without conflating nesting with the
/// size of the whole expression.
pub fn layout(
    list: &MathList,
    level: usize,
    document_scale: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> MathBox {
    layout_inner(list, level, document_scale, measure, true)
}

fn layout_inner(
    list: &MathList,
    level: usize,
    document_scale: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
    semantic_highlights: bool,
) -> MathBox {
    if list.is_empty() {
        return slot_box(level, document_scale);
    }

    // TeX spacing: a binary operator on the edge of the list (a unary sign)
    // is demoted to ordinary, then each adjacent pair gets its classed gap.
    let classes: Vec<AtomClass> = list.iter().map(atom_class).collect();
    let spaced: Vec<AtomClass> = (0..list.len()).map(|i| demote(&classes, i)).collect();

    // Digit groups ride the same advance as TeX's inter-atom spacing rather
    // than a node of their own: a separator that existed in the tree would
    // be a place the caret could stop and the backspace key could delete.
    let groups = digit_group_gaps(list);

    let mut children = Vec::with_capacity(list.len());
    let mut x = 0.0;
    for (index, node) in list.iter().enumerate() {
        if index > 0 && integral_family(&list[index - 1]) && integral_family(node) {
            x -= INTEGRAL_OVERLAP * scale(level, document_scale);
        }
        if index > 0 {
            x += (space_between(spaced[index - 1], spaced[index]) + groups[index])
                * size(level, document_scale);
        }
        let child = layout_node(node, level, document_scale, measure, semantic_highlights);
        children.push((x, 0.0, child));
        x += children.last().expect("child was pushed").2.width;
    }
    row_box(children)
}

#[derive(Clone, Copy)]
struct LayoutOptions {
    document_scale: f32,
    semantic_highlights: bool,
}

fn scale(level: usize, document_scale: f32) -> f32 {
    LEVEL_SCALE[level.min(LEVEL_SCALE.len() - 1)] * document_scale
}

fn size(level: usize, document_scale: f32) -> f32 {
    BASE_SIZE * scale(level, document_scale)
}

fn integral_family(node: &MathNode) -> bool {
    matches!(
        node,
        MathNode::BigOp {
            kind: BigOp::Integral | BigOp::ContourIntegral,
            ..
        }
    )
}

/// The TeX atom class of a node, which decides the space around it. Only the
/// classes that carry spacing are distinguished; every composite node — a
/// fraction, group, script, accent, or big operator — is an ordinary atom.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum AtomClass {
    /// A variable, number, or composite. No space around it.
    Ord,
    /// A binary operator (`+`, `−`, `·`, `∪`…): medium space on both sides
    /// when it sits between operands.
    Bin,
    /// A relation (`=`, `≤`, `∈`, `⊂`…): thick space on both sides.
    Rel,
    /// Punctuation (`,` `;`): thin space after it, none before.
    Punct,
    /// A standalone opening/closing delimiter. These normally live inside a
    /// [`MathNode::Group`] and rarely appear as bare atoms.
    Open,
    Close,
}

fn atom_class(node: &MathNode) -> AtomClass {
    match node {
        MathNode::Sym(ch) => char_class(*ch),
        _ => AtomClass::Ord,
    }
}

fn char_class(ch: char) -> AtomClass {
    match ch {
        '+' | '-' | '−' | '±' | '∓' | '×' | '÷' | '·' | '∗' | '⋆' | '∘' | '⊕' | '⊗' | '⊖' | '⊙'
        | '⊎' | '⊔' | '∖' | '∪' | '∩' | '∧' | '∨' => AtomClass::Bin,
        '=' | '<' | '>' | '≤' | '≥' | '≠' | '≈' | '≡' | '∼' | '≃' | '≅' | '⊂' | '⊃' | '⊆' | '⊇'
        | '∈' | '∉' | '∝' | '≪' | '≫' | '→' | '←' | '↔' | '⇒' | '⇐' | '⇔' | '↦' | '∥' | '⊥'
        | '≺' | '≻' => AtomClass::Rel,
        ',' | ';' => AtomClass::Punct,
        '(' | '[' | '{' | '⟨' | '⌈' | '⌊' => AtomClass::Open,
        ')' | ']' | '}' | '⟩' | '⌉' | '⌋' => AtomClass::Close,
        _ => AtomClass::Ord,
    }
}

/// A binary operator that is not strictly between operands — it starts or
/// ends the list, or touches an operator, relation, or punctuation — reads as
/// a unary sign and is demoted to an ordinary atom so it gets no space. This
/// is TeX's rule that tells `x − y` (spaced) from `−x` (not).
fn demote(classes: &[AtomClass], index: usize) -> AtomClass {
    if classes[index] != AtomClass::Bin {
        return classes[index];
    }
    let left_demotes = match index.checked_sub(1).map(|i| classes[i]) {
        Some(c) => matches!(
            c,
            AtomClass::Bin | AtomClass::Rel | AtomClass::Open | AtomClass::Punct
        ),
        None => true,
    };
    let right_demotes = match classes.get(index + 1).copied() {
        Some(c) => matches!(c, AtomClass::Rel | AtomClass::Close | AtomClass::Punct),
        None => true,
    };
    if left_demotes || right_demotes {
        AtomClass::Ord
    } else {
        AtomClass::Bin
    }
}

/// The space between two adjacent atoms, as a fraction of the current math
/// size. Nothing sits before punctuation; a binary operator is braced with a
/// medium space; a relation with a thick space.
fn space_between(prev: AtomClass, next: AtomClass) -> f32 {
    use AtomClass::*;
    match (prev, next) {
        (_, Punct) => 0.0,
        (Bin, _) | (_, Bin) => MED_SPACE,
        (Rel, Rel) => 0.0,
        (Rel, _) | (_, Rel) => THICK_SPACE,
        (Punct, _) => THIN_SPACE,
        _ => 0.0,
    }
}

/// The extra advance before each atom that separates a numeral's digit
/// groups: `gaps[i]` is the space owed *before* `list[i]`, in ems.
///
/// Grouping is per literal, not per digit run, so a decimal point does not
/// split one number into two: `1234.56789` is grouped from the point in
/// both directions, which is what ISO 31-0 asks for and what makes a long
/// mantissa countable.
fn digit_group_gaps(list: &MathList) -> Vec<f32> {
    let mut gaps = vec![0.0; list.len()];
    let mut index = 0;
    while index < list.len() {
        if !is_digit(list, index) {
            index += 1;
            continue;
        }
        let integer_start = index;
        let mut integer_end = index;
        while is_digit(list, integer_end) {
            integer_end += 1;
        }
        index = integer_end;
        // A point only belongs to this literal if a digit follows it —
        // otherwise it is the end of a sentence that happens to touch a
        // number, and the digits after nothing are somebody else's.
        if is_point(list, integer_end) && is_digit(list, integer_end + 1) {
            let mut fraction_end = integer_end + 1;
            while is_digit(list, fraction_end) {
                fraction_end += 1;
            }
            mark_groups(&mut gaps, integer_end + 1, fraction_end, false);
            index = fraction_end;
        }
        mark_groups(&mut gaps, integer_start, integer_end, true);
    }
    gaps
}

/// Marks a group boundary every three digits across `start..end`, counting
/// from the decimal point — the right edge of an integer part, the left
/// edge of a fractional one. Runs shorter than [`DIGIT_GROUP_MIN`] are left
/// alone; `1 00` would be worse than `100`.
fn mark_groups(gaps: &mut [f32], start: usize, end: usize, from_right: bool) {
    if end - start < DIGIT_GROUP_MIN {
        return;
    }
    for (position, gap) in gaps.iter_mut().enumerate().take(end).skip(start + 1) {
        let from_point = if from_right {
            end - position
        } else {
            position - start
        };
        if from_point % 3 == 0 {
            *gap = DIGIT_GROUP_SPACE;
        }
    }
}

fn is_digit(list: &MathList, index: usize) -> bool {
    matches!(list.get(index), Some(MathNode::Sym(c)) if c.is_ascii_digit())
}

fn is_point(list: &MathList, index: usize) -> bool {
    matches!(list.get(index), Some(MathNode::Sym('.')))
}

/// Which ink sets `ch`. Read from the atom class the spacing rules already
/// computed, so an operator cannot be spaced as one and inked as something
/// else. A *unary* sign is still an operator here even though [`demote`]
/// strips its spacing — how it is inked is about what it is, not where it
/// happens to sit.
fn char_ink(ch: char) -> MathInk {
    if ch.is_ascii_digit() {
        return MathInk::Number;
    }
    // Self-paired bars cannot have separate Open/Close atom classes, and a
    // pasted large-operator glyph remains an ordinary atom for spacing. Both
    // are still grammar when choosing ink.
    if matches!(ch, '|' | '‖' | '∫' | '∮' | '∑' | '∏') {
        return MathInk::Operator;
    }
    match char_class(ch) {
        AtomClass::Bin | AtomClass::Rel | AtomClass::Punct | AtomClass::Open | AtomClass::Close => {
            MathInk::Operator
        }
        AtomClass::Ord => MathInk::Term,
    }
}

/// One typed character. A bare letter is a variable nobody has said
/// anything else about, so it is styled as one — under its own identity,
/// which is the letter itself.
fn glyph(
    ch: char,
    level: usize,
    document_scale: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
    semantic_highlights: bool,
) -> MathBox {
    let mut buffer = [0u8; 4];
    let highlight = (semantic_highlights && ch.is_alphabetic())
        .then(|| math_style::resolve(SymbolRole::Variable, ch.encode_utf8(&mut buffer)));
    glyph_with_highlight(ch, level, document_scale, measure, highlight)
}

fn glyph_with_highlight(
    ch: char,
    level: usize,
    document_scale: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
    highlight: Option<SymbolStyle>,
) -> MathBox {
    let size = size(level, document_scale);
    let text = ch.to_string();
    let half = size * 0.5;
    let pad_x = if highlight.is_some() {
        VARIABLE_PAD_X * scale(level, document_scale)
    } else {
        0.0
    };
    let pad_y = if highlight.is_some() {
        VARIABLE_PAD_Y * scale(level, document_scale)
    } else {
        0.0
    };
    // Measured with the style it will be drawn with, not a natural one: a
    // box that reserved the font's full advance while the painter condensed
    // the ink would leave every letter sitting in a gap of its own.
    let style = TextStyle::math(size, theme::ink()).condensed(MATH_CONDENSE);
    MathBox {
        width: measure(&text, &style) + pad_x * 2.0,
        ascent: half + pad_y,
        descent: half + pad_y,
        highlight,
        kind: BoxKind::Glyph {
            text,
            size,
            ink: char_ink(ch),
            offset_x: pad_x,
            offset_y: 0.0,
            condense: MATH_CONDENSE,
        },
    }
}

fn slot_box(level: usize, document_scale: f32) -> MathBox {
    let scale = scale(level, document_scale);
    let height = SLOT_H * scale;
    let half = height * 0.5;
    MathBox {
        width: SLOT_W * scale,
        ascent: half,
        descent: half,
        highlight: None,
        kind: BoxKind::Slot {
            size: height,
            visible: true,
        },
    }
}

fn invisible_slot_box(level: usize, document_scale: f32) -> MathBox {
    let mut box_ = slot_box(level, document_scale);
    let BoxKind::Slot { visible, .. } = &mut box_.kind else {
        unreachable!("slot_box must produce a slot");
    };
    *visible = false;
    box_
}

fn contributes_to_measure(box_: &MathBox) -> bool {
    !matches!(box_.kind, BoxKind::Slot { visible: false, .. })
}

fn measured_width(box_: &MathBox) -> f32 {
    if contributes_to_measure(box_) {
        box_.width
    } else {
        0.0
    }
}

fn layout_node(
    node: &MathNode,
    level: usize,
    document_scale: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
    semantic_highlights: bool,
) -> MathBox {
    match node {
        MathNode::Sym(ch) => glyph(*ch, level, document_scale, measure, semantic_highlights),
        MathNode::Resolved { id, role, body, .. } => {
            let body = layout_inner(body, level, document_scale, measure, false);
            if semantic_highlights {
                let style = math_style::resolve(*role, id);
                padded_highlight(body, level, document_scale, style)
            } else {
                body
            }
        }
        MathNode::Frac { num, den } => fraction(
            node,
            num,
            den,
            level,
            document_scale,
            measure,
            semantic_highlights,
        ),
        MathNode::Script { .. } => {
            script(node, level, document_scale, measure, semantic_highlights)
        }
        MathNode::Group { open, close, body } => group(
            node,
            *open,
            *close,
            body,
            level,
            measure,
            LayoutOptions {
                document_scale,
                semantic_highlights,
            },
        ),
        MathNode::Sqrt { body } => radical(
            node,
            body,
            level,
            document_scale,
            measure,
            semantic_highlights,
        ),
        MathNode::Accent { kind, body } => accent(
            node,
            *kind,
            body,
            level,
            document_scale,
            measure,
            semantic_highlights,
        ),
        MathNode::BigOp { kind, lower, upper } => big_op(
            node,
            kind,
            lower,
            upper,
            level,
            measure,
            LayoutOptions {
                document_scale,
                semantic_highlights,
            },
        ),
    }
}

/// A large operator set from the font rather than drawn. `ratio` is that
/// operator's own faux width ratio (`1.0` = the typeface's natural advance),
/// which [`MATH_CONDENSE`] squeezes like every other glyph's — so the ratio
/// this box ends up carrying is the product of the two, and the one the
/// painter will use. Measurement applies the same ratio, so rendered and
/// reserved widths stay in step.
///
/// `rise` lifts the glyph's centre above the anchor as a fraction of `size`,
/// which the integral needs because its ink is bottom-biased in JuliaMono's
/// em box.
fn text_glyph(
    text: &str,
    size: f32,
    ratio: f32,
    rise: f32,
    ink: MathInk,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> MathBox {
    let half = size * 0.5;
    let condense = ratio * MATH_CONDENSE;
    let style = TextStyle::math(size, theme::ink()).condensed(condense);
    MathBox {
        width: measure(text, &style),
        ascent: half,
        descent: half,
        highlight: None,
        kind: BoxKind::Glyph {
            text: text.to_owned(),
            size,
            ink,
            offset_x: 0.0,
            offset_y: size * rise,
            condense,
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
    options: LayoutOptions,
) -> MathBox {
    let document_scale = options.document_scale;
    let semantic_highlights = options.semantic_highlights;
    // Every operator is set from the font; the integral family is condensed
    // to the sign's tall, narrow shape. See `INTEGRAL_CONDENSE`.
    let operator = match kind {
        BigOp::Integral => text_glyph(
            "∫",
            size(level, document_scale) * BIGOP_SCALE,
            INTEGRAL_CONDENSE,
            INTEGRAL_RISE,
            MathInk::Operator,
            measure,
        ),
        BigOp::ContourIntegral => text_glyph(
            "∮",
            size(level, document_scale) * BIGOP_SCALE,
            INTEGRAL_CONDENSE,
            INTEGRAL_RISE,
            MathInk::Operator,
            measure,
        ),
        BigOp::Sum => text_glyph(
            "∑",
            size(level, document_scale) * BIGOP_SCALE,
            SUM_CONDENSE,
            SUM_RISE,
            MathInk::Operator,
            measure,
        ),
        BigOp::Prod => text_glyph(
            "∏",
            size(level, document_scale) * BIGOP_SCALE,
            1.0,
            0.0,
            MathInk::Operator,
            measure,
        ),
        BigOp::Limit => text_glyph(
            "lim",
            size(level, document_scale),
            1.0,
            0.0,
            MathInk::Operator,
            measure,
        ),
    };
    let operand_level = (level + 1).min(2);
    let hide_empty_limits = matches!(kind, BigOp::Integral | BigOp::ContourIntegral);
    let lower_visible = !lower.is_empty() || !hide_empty_limits;
    let lower = if lower_visible {
        layout_inner(
            lower,
            operand_level,
            document_scale,
            measure,
            semantic_highlights,
        )
    } else {
        invisible_slot_box(operand_level, document_scale)
    };
    let upper = if *kind == BigOp::Limit {
        None
    } else if upper.is_empty() && hide_empty_limits {
        Some(invisible_slot_box(operand_level, document_scale))
    } else {
        Some(layout_inner(
            upper,
            operand_level,
            document_scale,
            measure,
            semantic_highlights,
        ))
    };
    let width = operator
        .width
        .max(measured_width(&lower))
        .max(upper.as_ref().map_or(0.0, measured_width));
    let gap = size(level, document_scale) * BIGOP_GAP;
    let operator_ascent = operator.ascent;
    let operator_descent = operator.descent;
    let lower_y = -(operator_descent + gap + lower.ascent);
    let mut children = (0..(1 + node.slots().len()))
        .map(|_| None)
        .collect::<Vec<_>>();
    children[0] = Some(((width - operator.width) * 0.5, 0.0, operator));
    let lower_x = (width - lower.width) * 0.5;
    children[slot_child_index(node, Slot::Lower).expect("big operator lower slot index")] =
        Some((lower_x, lower_y, lower));
    if let Some(upper) = upper {
        let upper_y = operator_ascent + gap + upper.descent;
        let upper_x = (width - upper.width) * 0.5;
        children[slot_child_index(node, Slot::Upper).expect("big operator upper slot index")] =
            Some((upper_x, upper_y, upper));
    }
    row_box(
        children
            .into_iter()
            .map(|child| child.expect("every big operator child must be laid out"))
            .collect(),
    )
}

fn stretchy_size(body: &MathBox, level: usize, document_scale: f32) -> f32 {
    size(level, document_scale).max((body.ascent + body.descent) * DELIM_FILL)
}

/// `thickness` is passed rather than derived, because a delimiter and a big
/// operator are drawn at the same height but not at the same weight. Every
/// caller already has the scaled stroke it drew the path with, so taking it
/// here also stops the two from being computed twice and drifting.
fn stroked_box(
    width: f32,
    ascent: f32,
    descent: f32,
    path: String,
    thickness: f32,
    ink: MathInk,
) -> MathBox {
    MathBox {
        width,
        ascent,
        descent,
        highlight: None,
        kind: BoxKind::Primitive(MathPrimitive::Stroke {
            path,
            thickness,
            ink,
        }),
    }
}

fn delimiter(ch: char, height: f32, level: usize, document_scale: f32) -> MathBox {
    let width = size(level, document_scale)
        * match ch {
            '|' => 0.20,
            '‖' => 0.30,
            _ => 0.34,
        };
    let stroke = SHAPE_STROKE * scale(level, document_scale);
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
        '|' => format!("M {} {top} V {bottom}", width * 0.5),
        '‖' => format!(
            "M {} {top} V {bottom} M {} {top} V {bottom}",
            width * 0.3,
            width * 0.7,
        ),
        '⟨' => format!("M {width} {top} L {} 0 L {width} {bottom}", stroke * 0.5,),
        '⟩' => format!("M 0 {top} L {} 0 L 0 {bottom}", width - stroke * 0.5,),
        _ => format!("M {} {top} V {bottom}", width * 0.5),
    };
    stroked_box(
        width,
        height * 0.5,
        height * 0.5,
        path,
        stroke,
        MathInk::Operator,
    )
}

fn group(
    node: &MathNode,
    open: char,
    close: char,
    body: &MathList,
    level: usize,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
    options: LayoutOptions,
) -> MathBox {
    let document_scale = options.document_scale;
    let semantic_highlights = options.semantic_highlights;
    let body = layout_inner(body, level, document_scale, measure, semantic_highlights);
    let delimiter_height = stretchy_size(&body, level, document_scale);
    let opener = delimiter(open, delimiter_height, level, document_scale);
    let closer = delimiter(close, delimiter_height, level, document_scale);
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
    document_scale: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
    semantic_highlights: bool,
) -> MathBox {
    let body = layout_inner(body, level, document_scale, measure, semantic_highlights);
    let stroke = SHAPE_STROKE * scale(level, document_scale);
    let body_x = size(level, document_scale) * 0.48;
    let gap = size(level, document_scale) * RADICAL_GAP;
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
        stroke,
        MathInk::Term,
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
    document_scale: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
    semantic_highlights: bool,
) -> MathBox {
    let body = layout_inner(body, level, document_scale, measure, semantic_highlights);
    let stroke = SHAPE_STROKE * scale(level, document_scale);
    let height = size(level, document_scale) * 0.22;
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
                ink: MathInk::Term,
            }
        }
        AccentKind::Bar | AccentKind::Hat => {
            let path = match kind {
                AccentKind::Bar => {
                    format!("M {} {y} H {}", stroke * 0.5, body.width - stroke * 0.5,)
                }
                AccentKind::Hat => format!(
                    "M {} {} L {} {} L {} {}",
                    stroke * 0.5,
                    y + height * 0.35,
                    body.width * 0.5,
                    y - height * 0.45,
                    body.width - stroke * 0.5,
                    y + height * 0.35,
                ),
                AccentKind::Vector
                | AccentKind::Dot
                | AccentKind::DoubleDot
                | AccentKind::TripleDot => unreachable!(),
            };
            MathPrimitive::Stroke {
                path,
                thickness: stroke,
                ink: MathInk::Term,
            }
        }
        AccentKind::Dot | AccentKind::DoubleDot | AccentKind::TripleDot => {
            let count = match kind {
                AccentKind::Dot => 1,
                AccentKind::DoubleDot => 2,
                AccentKind::TripleDot => 3,
                AccentKind::Vector | AccentKind::Hat | AccentKind::Bar => unreachable!(),
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
        highlight: None,
        kind: BoxKind::Primitive(primitive),
    };
    let mark_y = body.ascent + size(level, document_scale) * ACCENT_GAP;
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

fn clear_highlights(box_: &mut MathBox) {
    box_.highlight = None;
    if let BoxKind::Row { children } = &mut box_.kind {
        for (_, _, child) in children {
            clear_highlights(child);
        }
    }
}

fn padded_highlight(
    mut box_: MathBox,
    level: usize,
    document_scale: f32,
    style: SymbolStyle,
) -> MathBox {
    clear_highlights(&mut box_);
    let pad_x = VARIABLE_PAD_X * scale(level, document_scale);
    let pad_y = VARIABLE_PAD_Y * scale(level, document_scale);
    if let BoxKind::Row { children } = &mut box_.kind {
        for (x, _, _) in children {
            *x += pad_x;
        }
    }
    box_.width += pad_x * 2.0;
    box_.ascent += pad_y;
    box_.descent += pad_y;
    box_.highlight = Some(style);
    box_
}

fn script(
    node: &MathNode,
    level: usize,
    document_scale: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
    semantic_highlights: bool,
) -> MathBox {
    let operand_level = (level + 1).min(2);
    let slots = node.slots();
    let mut children = (0..slots.len()).map(|_| None).collect::<Vec<_>>();
    let mut base_width = 0.0;
    // A script over a lone symbol highlights as one atom -- a squared `x` is
    // a thing, not an `x` wearing a decoration -- so the base's own identity
    // styles the whole box and the base is laid out without a second one.
    let base_style = semantic_highlights
        .then(|| match node {
            MathNode::Script { base, .. } => match base.as_slice() {
                [MathNode::Sym(ch)] if ch.is_alphabetic() => {
                    let mut buffer = [0u8; 4];
                    Some(math_style::resolve(
                        SymbolRole::Variable,
                        ch.encode_utf8(&mut buffer),
                    ))
                }
                [MathNode::Resolved { id, role, .. }] => Some(math_style::resolve(*role, id)),
                _ => None,
            },
            _ => None,
        })
        .flatten();

    for slot in slots {
        let child_list = node.slot(slot).expect("script slots must resolve");
        let child = layout_inner(
            child_list,
            if slot == Slot::Base {
                level
            } else {
                operand_level
            },
            document_scale,
            measure,
            semantic_highlights && base_style.is_none(),
        );
        let (x, y) = match slot {
            Slot::Base => {
                base_width = child.width;
                (0.0, 0.0)
            }
            Slot::Sup => (
                base_width,
                size(level, document_scale) * SCRIPT_RISE + child.descent,
            ),
            Slot::Sub => (
                base_width,
                -(size(level, document_scale) * SCRIPT_DROP + child.ascent),
            ),
            Slot::Num | Slot::Den | Slot::Body | Slot::Lower | Slot::Upper => {
                unreachable!("fraction slots cannot be scripts")
            }
        };
        let index = slot_child_index(node, slot).expect("script slot must have a child index");
        children[index] = Some((x, y, child));
    }

    // `slots` drives both this order and `slot_child_index`, so traversal sees
    // exactly the children this layout emitted.
    let box_ = row_box(
        children
            .into_iter()
            .map(|child| child.expect("every script slot must be laid out"))
            .collect(),
    );
    if let Some(style) = base_style {
        padded_highlight(box_, level, document_scale, style)
    } else {
        box_
    }
}

fn fraction(
    node: &MathNode,
    num: &MathList,
    den: &MathList,
    level: usize,
    document_scale: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
    semantic_highlights: bool,
) -> MathBox {
    let operand_level = (level + 1).min(2);
    let numerator = layout_inner(
        num,
        operand_level,
        document_scale,
        measure,
        semantic_highlights,
    );
    let denominator = layout_inner(
        den,
        operand_level,
        document_scale,
        measure,
        semantic_highlights,
    );
    let current_size = size(level, document_scale);
    let gap = current_size * FRAC_GAP;
    let pad = FRAC_PAD * scale(level, document_scale);
    let bar_size = BAR * scale(level, document_scale);
    let width = numerator.width.max(denominator.width) + pad * 2.0;

    let bar = MathBox {
        width,
        ascent: bar_size * 0.5,
        descent: bar_size * 0.5,
        highlight: None,
        kind: BoxKind::Bar {
            thickness: bar_size,
        },
    };
    // Anchor line is the inline prose middle, so placing the bar at zero
    // aligns it with the math axis instead of lifting the whole fraction.
    let bar_y = 0.0;
    // Like scripts, place each operand by the edge facing the bar so lopsided
    // boxes keep the requested clearance.
    let clearance = bar_size * 0.5 + gap;
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
        MathNode::Sym(_) | MathNode::Resolved { .. } | MathNode::Script { .. } => 0,
    };
    Some(slot_index + visual_offset)
}

fn row_box(children: Vec<(f32, f32, MathBox)>) -> MathBox {
    let width = children
        .iter()
        .filter(|(_, _, child)| contributes_to_measure(child))
        .map(|(x, _, child)| x + child.width)
        .fold(0.0, f32::max);
    let ascent = children
        .iter()
        .filter(|(_, _, child)| contributes_to_measure(child))
        .map(|(_, y, child)| y + child.ascent)
        .fold(0.0, f32::max);
    let descent = children
        .iter()
        .filter(|(_, _, child)| contributes_to_measure(child))
        .map(|(_, y, child)| child.descent - y)
        .fold(0.0, f32::max);
    MathBox {
        width,
        ascent,
        descent,
        highlight: None,
        kind: BoxKind::Row { children },
    }
}

/// Return the recursive hit-test envelope of a laid-out expression without
/// changing its visual width, ascent, or descent.
pub fn interaction_bounds(box_: &MathBox) -> InteractionBounds {
    let mut bounds = InteractionBounds {
        left: 0.0,
        right: box_.width,
        ascent: box_.ascent,
        descent: box_.descent,
    };
    if let BoxKind::Row { children } = &box_.kind {
        for (x, y, child) in children {
            let child = interaction_bounds(child);
            bounds.left = bounds.left.min(x + child.left);
            bounds.right = bounds.right.max(x + child.right);
            bounds.ascent = bounds.ascent.max(y + child.ascent);
            bounds.descent = bounds.descent.max(child.descent - y);
        }
    }
    bounds
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
    document_scale: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> (f32, f32, f32) {
    let box_ = layout(list, level, document_scale, measure);
    cursor_in(list, &box_, cursor, 0, level, document_scale, (0.0, 0.0))
}

fn cursor_in(
    list: &MathList,
    box_: &MathBox,
    cursor: &MathCursor,
    path_index: usize,
    level: usize,
    document_scale: f32,
    origin: (f32, f32),
) -> (f32, f32, f32) {
    let BoxKind::Row { children } = &box_.kind else {
        return (
            origin.0,
            origin.1,
            cursor_height(list, level, document_scale),
        );
    };
    let index = cursor.index.min(list.len()).min(children.len());
    if path_index == cursor.path.len() {
        let x = children
            .get(index)
            .map_or(box_.width, |(child_x, _, _)| *child_x);
        return (
            origin.0 + x,
            origin.1,
            cursor_height(list, level, document_scale),
        );
    }

    let step = cursor.path[path_index];
    let Some(node) = list.get(step.index) else {
        return (
            origin.0 + children.get(index).map_or(box_.width, |(x, _, _)| *x),
            origin.1,
            cursor_height(list, level, document_scale),
        );
    };
    let Some((_, _, parent)) = children.get(step.index) else {
        return (
            origin.0 + box_.width,
            origin.1,
            cursor_height(list, level, document_scale),
        );
    };
    let Some((slot_list, slot_box, slot_x, slot_y)) = structural_slot(node, parent, step.slot)
    else {
        return (
            origin.0 + box_.width,
            origin.1,
            cursor_height(list, level, document_scale),
        );
    };
    cursor_in(
        slot_list,
        slot_box,
        cursor,
        path_index + 1,
        child_level(node, step.slot, level),
        document_scale,
        (
            origin.0 + children[step.index].0 + slot_x,
            origin.1 + children[step.index].1 + slot_y,
        ),
    )
}

fn cursor_height(_list: &MathList, level: usize, document_scale: f32) -> f32 {
    size(level, document_scale)
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
    document_scale: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> MathCursor {
    let box_ = layout(list, level, document_scale, measure);
    let mut cursor = MathCursor::default();
    hit_list(list, &box_, point, level, &mut cursor);
    cursor
}

/// Hit-test one complete math node for Normal-mode context selection.
///
/// Atomic symbols beat their structural ancestors. A structure wins on its
/// own geometry, while empty space outside node boxes has no target.
pub fn hit_node(
    list: &MathList,
    point: (f32, f32),
    level: usize,
    document_scale: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> Option<NodeAddress> {
    let box_ = layout(list, level, document_scale, measure);
    hit_node_in(list, &box_, point, &mut Vec::new())
}

/// Return every smallest math node touched by a circular brush.
///
/// Structural children take precedence over their parent, and later siblings
/// are returned first to match paint order. A structure itself is returned
/// when the brush only touches its own geometry or one of its empty slots.
pub fn hit_nodes_in_circle(
    list: &MathList,
    point: (f32, f32),
    radius: f32,
    level: usize,
    document_scale: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> Vec<NodeAddress> {
    let box_ = layout(list, level, document_scale, measure);
    let mut hits = Vec::new();
    hit_nodes_in_circle_in(
        list,
        &box_,
        point,
        radius.max(0.0),
        &mut Vec::new(),
        &mut hits,
    );
    hits
}

fn hit_nodes_in_circle_in(
    list: &MathList,
    box_: &MathBox,
    point: (f32, f32),
    radius: f32,
    path: &mut Vec<Step>,
    hits: &mut Vec<NodeAddress>,
) {
    let BoxKind::Row { children } = &box_.kind else {
        return;
    };
    for (index, node) in list.iter().enumerate().rev() {
        let Some((child_x, child_y, child)) = children.get(index) else {
            continue;
        };
        let local = (point.0 - child_x, point.1 - child_y);
        if !circle_intersects_box(child, local, radius) {
            continue;
        }

        let address = NodeAddress {
            path: path.clone(),
            index,
        };
        if matches!(node, MathNode::Sym(_) | MathNode::Resolved { .. }) {
            hits.push(address);
            continue;
        }

        let before = hits.len();
        for slot in node.slots().into_iter().rev() {
            let Some((slot_list, slot_box, slot_x, slot_y)) = structural_slot(node, child, slot)
            else {
                continue;
            };
            let slot_point = (local.0 - slot_x, local.1 - slot_y);
            if slot_list.is_empty() || !circle_intersects_box(slot_box, slot_point, radius) {
                continue;
            }
            path.push(Step { index, slot });
            hit_nodes_in_circle_in(slot_list, slot_box, slot_point, radius, path, hits);
            path.pop();
        }
        if hits.len() == before {
            hits.push(address);
        }
    }
}

fn hit_node_in(
    list: &MathList,
    box_: &MathBox,
    point: (f32, f32),
    path: &mut Vec<Step>,
) -> Option<NodeAddress> {
    let BoxKind::Row { children } = &box_.kind else {
        return None;
    };
    // Later siblings paint on top of earlier ones when boxes overlap.
    for (index, node) in list.iter().enumerate().rev() {
        let (child_x, child_y, child) = children.get(index)?;
        let local = (point.0 - child_x, point.1 - child_y);
        if !contains(child, local) {
            continue;
        }
        let address = NodeAddress {
            path: path.clone(),
            index,
        };
        if matches!(node, MathNode::Sym(_) | MathNode::Resolved { .. }) {
            return Some(address);
        }
        for slot in node.slots().into_iter().rev() {
            let Some((slot_list, slot_box, slot_x, slot_y)) = structural_slot(node, child, slot)
            else {
                continue;
            };
            let slot_point = (local.0 - slot_x, local.1 - slot_y);
            if !contains(slot_box, slot_point) {
                continue;
            }
            if slot_list.is_empty() {
                return Some(address);
            }
            path.push(Step { index, slot });
            let hit = hit_node_in(slot_list, slot_box, slot_point, path);
            path.pop();
            return hit;
        }
        return Some(address);
    }
    None
}

fn contains(box_: &MathBox, point: (f32, f32)) -> bool {
    point.0 >= 0.0 && point.0 <= box_.width && point.1 >= -box_.descent && point.1 <= box_.ascent
}

fn circle_intersects_box(box_: &MathBox, point: (f32, f32), radius: f32) -> bool {
    let nearest_x = point.0.clamp(0.0, box_.width);
    let nearest_y = point.1.clamp(-box_.descent, box_.ascent);
    let dx = point.0 - nearest_x;
    let dy = point.1 - nearest_y;
    dx * dx + dy * dy <= radius * radius
}

/// Return the visual bounds of `address` in the expression's coordinate space.
pub fn node_bounds(
    list: &MathList,
    address: &NodeAddress,
    level: usize,
    document_scale: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) -> Option<NodeBounds> {
    let box_ = layout(list, level, document_scale, measure);
    node_bounds_in(list, &box_, address, 0.0, 0.0)
}

fn node_bounds_in(
    list: &MathList,
    box_: &MathBox,
    address: &NodeAddress,
    mut origin_x: f32,
    mut origin_y: f32,
) -> Option<NodeBounds> {
    let mut list = list;
    let mut box_ = box_;
    for step in &address.path {
        let BoxKind::Row { children } = &box_.kind else {
            return None;
        };
        let node = list.get(step.index)?;
        let (node_x, node_y, node_box) = children.get(step.index)?;
        let (slot_list, slot_box, slot_x, slot_y) = structural_slot(node, node_box, step.slot)?;
        origin_x += node_x + slot_x;
        origin_y += node_y + slot_y;
        list = slot_list;
        box_ = slot_box;
    }
    let BoxKind::Row { children } = &box_.kind else {
        return None;
    };
    list.get(address.index)?;
    let (x, y, node) = children.get(address.index)?;
    let anchor_x = origin_x + x;
    let anchor_y = origin_y + y;
    Some(NodeBounds {
        left: anchor_x,
        right: anchor_x + node.width,
        top: anchor_y + node.ascent,
        bottom: anchor_y - node.descent,
    })
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

    fn layout(list: &MathList, level: usize, measure: &dyn Fn(&str, &TextStyle) -> f32) -> MathBox {
        super::layout(list, level, 1.0, measure)
    }

    fn cursor_pos(
        list: &MathList,
        cursor: &MathCursor,
        level: usize,
        measure: &dyn Fn(&str, &TextStyle) -> f32,
    ) -> (f32, f32, f32) {
        super::cursor_pos(list, cursor, level, 1.0, measure)
    }

    fn hit(
        list: &MathList,
        point: (f32, f32),
        level: usize,
        measure: &dyn Fn(&str, &TextStyle) -> f32,
    ) -> MathCursor {
        super::hit(list, point, level, 1.0, measure)
    }

    fn hit_node(
        list: &MathList,
        point: (f32, f32),
        level: usize,
        measure: &dyn Fn(&str, &TextStyle) -> f32,
    ) -> Option<NodeAddress> {
        super::hit_node(list, point, level, 1.0, measure)
    }

    fn hit_nodes_in_circle(
        list: &MathList,
        point: (f32, f32),
        radius: f32,
        level: usize,
        measure: &dyn Fn(&str, &TextStyle) -> f32,
    ) -> Vec<NodeAddress> {
        super::hit_nodes_in_circle(list, point, radius, level, 1.0, measure)
    }

    fn node_bounds(
        list: &MathList,
        address: &NodeAddress,
        level: usize,
        measure: &dyn Fn(&str, &TextStyle) -> f32,
    ) -> Option<NodeBounds> {
        super::node_bounds(list, address, level, 1.0, measure)
    }

    fn symbols(text: &str) -> MathList {
        text.chars().map(MathNode::Sym).collect()
    }

    fn variable_width(level: usize) -> f32 {
        (BASE_SIZE * 0.5 + VARIABLE_PAD_X * 2.0) * super::scale(level, 1.0)
    }

    /// Every glyph set from the font is squeezed — that is the whole point of
    /// [`MATH_CONDENSE`], and the box is where the painter reads it from, so
    /// this is the number that has to be right.
    #[test]
    fn every_glyph_set_from_the_font_carries_the_global_squeeze() {
        let box_ = super::layout(&symbols("xy"), 0, 1.0, &fake_measure);
        let BoxKind::Row { children } = &box_.kind else {
            panic!("a symbol list must produce row");
        };
        for (_, _, child) in children {
            let BoxKind::Glyph { condense, .. } = child.kind else {
                panic!("a symbol must be a glyph");
            };
            assert!(
                (condense - MATH_CONDENSE).abs() < 0.0001,
                "an ordinary glyph must be set at the global squeeze, got {condense}"
            );
        }
    }

    /// Reserved width and rendered ink are the same number only if layout
    /// measures every glyph with the ratio it will be drawn at. A glyph
    /// measured naturally would reserve the monospace cell it is no longer
    /// painted into, and every letter would sit in a gap of its own.
    #[test]
    fn layout_measures_every_glyph_at_the_ratio_it_will_be_drawn_at() {
        let seen = std::cell::RefCell::new(Vec::new());
        let recording = |text: &str, style: &TextStyle| {
            seen.borrow_mut().push(style.width);
            fake_measure(text, style)
        };
        let _ = super::layout(&symbols("xy"), 0, 1.0, &recording);
        let widths = seen.into_inner();
        assert!(!widths.is_empty(), "the measure must have been asked");
        assert!(
            widths
                .iter()
                .all(|width| (*width - MATH_CONDENSE).abs() < 0.0001),
            "every glyph must be measured condensed, saw {widths:?}"
        );

        // A large operator is measured at its own ratio times the squeeze —
        // the same number its box carries.
        let seen = std::cell::RefCell::new(Vec::new());
        let recording = |text: &str, style: &TextStyle| {
            seen.borrow_mut().push(style.width);
            fake_measure(text, style)
        };
        let _ = layout(
            &vec![big_op(BigOp::Integral, Vec::new(), Vec::new())],
            0,
            &recording,
        );
        let widths = seen.into_inner();
        let expected = INTEGRAL_CONDENSE * MATH_CONDENSE;
        assert!(
            widths
                .iter()
                .all(|width| (*width - expected).abs() < 0.0001),
            "the integral must be measured at its drawn ratio, saw {widths:?}"
        );
    }

    #[test]
    fn document_scale_shrinks_math_width_and_height_by_same_factor() {
        let list = vec![fraction(symbols("xy"), symbols("z"))];
        let full = super::layout(&list, 0, 1.0, &fake_measure);
        let small = super::layout(&list, 0, 0.6, &fake_measure);

        assert!((small.width - full.width * 0.6).abs() < 0.0001);
        assert!((small.ascent - full.ascent * 0.6).abs() < 0.0001);
        assert!((small.descent - full.descent * 0.6).abs() < 0.0001);
    }

    #[test]
    fn fraction_operands_keep_level_ratio_at_document_scale() {
        let list = vec![fraction(symbols("x"), symbols("y"))];
        let box_ = super::layout(&list, 0, 0.6, &fake_measure);
        let root = super::layout(&symbols("x"), 0, 0.6, &fake_measure);
        let BoxKind::Row { children } = box_.kind else {
            panic!("fraction expression must produce row");
        };
        let BoxKind::Row { children: fraction } = &children[0].2.kind else {
            panic!("fraction must produce row");
        };
        let BoxKind::Row {
            children: numerator,
        } = &fraction[0].2.kind
        else {
            panic!("numerator must produce row");
        };
        let BoxKind::Glyph {
            size: numerator_size,
            ..
        } = numerator[0].2.kind
        else {
            panic!("numerator must contain glyph");
        };
        let BoxKind::Row { children: root_row } = root.kind else {
            panic!("root expression must produce row");
        };
        let BoxKind::Glyph {
            size: root_size, ..
        } = root_row[0].2.kind
        else {
            panic!("root must contain glyph");
        };

        assert!((numerator_size / root_size - LEVEL_SCALE[1]).abs() < 0.0001);
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

    fn resolved(role: SymbolRole, body: MathList) -> MathNode {
        MathNode::Resolved {
            id: "test.symbol".to_owned(),
            role,
            variant: "default".to_owned(),
            body,
        }
    }

    /// The x offset of every child in a laid-out list, which is where digit
    /// grouping shows up: a group boundary is an advance, not a node.
    fn offsets(box_: &MathBox) -> Vec<f32> {
        let BoxKind::Row { children } = &box_.kind else {
            panic!("list must produce row");
        };
        children.iter().map(|(x, _, _)| *x).collect()
    }

    /// The gap left before each child beyond the width of the one before it.
    fn gaps(box_: &MathBox) -> Vec<f32> {
        let BoxKind::Row { children } = &box_.kind else {
            panic!("list must produce row");
        };
        let offsets = offsets(box_);
        offsets
            .iter()
            .enumerate()
            .map(|(index, x)| match index.checked_sub(1) {
                None => 0.0,
                Some(previous) => x - offsets[previous] - children[previous].2.width,
            })
            .collect()
    }

    /// Which children a group gap was inserted before. Thresholded rather
    /// than compared against zero: the offsets are an f32 accumulation, so
    /// a child with no gap before it lands within an ulp of its neighbour's
    /// right edge rather than exactly on it.
    fn grouped_at(box_: &MathBox) -> Vec<usize> {
        let group = BASE_SIZE * DIGIT_GROUP_SPACE;
        gaps(box_)
            .into_iter()
            .enumerate()
            .filter(|(_, gap)| *gap > group * 0.5)
            .map(|(index, _)| index)
            .collect()
    }

    fn ink_at(box_: &MathBox, index: usize) -> MathInk {
        let BoxKind::Row { children } = &box_.kind else {
            panic!("list must produce row");
        };
        match &children[index].2.kind {
            BoxKind::Glyph { ink, .. } => *ink,
            other => panic!("child {index} is not a glyph: {other:?}"),
        }
    }

    #[test]
    fn a_long_numeral_is_grouped_in_threes_from_the_right() {
        assert_eq!(grouped_at(&layout(&symbols("1000"), 0, &fake_measure)), [1]);
        assert_eq!(
            grouped_at(&layout(&symbols("10000"), 0, &fake_measure)),
            [2]
        );
        assert_eq!(
            grouped_at(&layout(&symbols("1234567"), 0, &fake_measure)),
            [1, 4]
        );
    }

    /// `1 00` would be worse than `100`, so grouping starts at four digits.
    #[test]
    fn a_short_numeral_is_left_alone() {
        for short in ["1", "12", "123"] {
            assert!(grouped_at(&layout(&symbols(short), 0, &fake_measure)).is_empty());
        }
    }

    /// ISO 31-0: a decimal point is the origin, and the fractional part is
    /// grouped away from it in the other direction.
    #[test]
    fn a_decimal_groups_outward_from_its_point() {
        // 3.14159 -> 3.141 59; the one-digit integer part is left alone.
        assert_eq!(
            grouped_at(&layout(&symbols("3.14159"), 0, &fake_measure)),
            [5]
        );
        // 12345.6789 -> 12 345.678 9
        assert_eq!(
            grouped_at(&layout(&symbols("12345.6789"), 0, &fake_measure)),
            [2, 9]
        );
    }

    /// A point with no digits after it belongs to the sentence, not to the
    /// number, so it must not join two runs into one literal.
    #[test]
    fn a_trailing_point_does_not_join_two_numbers() {
        assert_eq!(
            grouped_at(&layout(&symbols("1234.x5678"), 0, &fake_measure)),
            [1, 7]
        );
    }

    /// The separator is an advance, never a node: the caret still steps
    /// through exactly four digits, and lands on the group boundary.
    #[test]
    fn a_group_boundary_is_spacing_rather_than_a_node() {
        let list = symbols("1000");
        let box_ = layout(&list, 0, &fake_measure);
        let BoxKind::Row { children } = &box_.kind else {
            panic!("list must produce row");
        };
        assert_eq!(children.len(), 4, "grouping adds no atoms");

        let cursor = MathCursor {
            path: Vec::new(),
            index: 1,
        };
        let (x, ..) = cursor_pos(&list, &cursor, 0, &fake_measure);
        assert_eq!(x, offsets(&box_)[1], "the caret follows the gap");
    }

    #[test]
    fn numerals_operators_and_terms_take_three_different_inks() {
        let box_ = layout(&symbols("2+x"), 0, &fake_measure);
        assert_eq!(ink_at(&box_, 0), MathInk::Number);
        assert_eq!(ink_at(&box_, 1), MathInk::Operator);
        assert_eq!(ink_at(&box_, 2), MathInk::Term);
    }

    /// Relations, punctuation, and delimiters are all structural grammar.
    #[test]
    fn relations_punctuation_and_delimiters_are_quiet() {
        let box_ = layout(&symbols("=,(|∑"), 0, &fake_measure);
        assert_eq!(ink_at(&box_, 0), MathInk::Operator);
        assert_eq!(ink_at(&box_, 1), MathInk::Operator);
        assert_eq!(ink_at(&box_, 2), MathInk::Operator);
        assert_eq!(ink_at(&box_, 3), MathInk::Operator);
        assert_eq!(ink_at(&box_, 4), MathInk::Operator);
    }

    /// TeX demotes a leading `-` to an ordinary atom so it gets no spacing.
    /// That is about where it sits; it is still an operator, and inking it
    /// as a term would make `-x` and a variable named minus look alike.
    #[test]
    fn a_unary_sign_is_still_inked_as_an_operator() {
        let box_ = layout(&symbols("-x"), 0, &fake_measure);
        assert_eq!(ink_at(&box_, 0), MathInk::Operator);
        assert!(gaps(&layout(&symbols("-x"), 0, &fake_measure))[1] < 0.5);
    }

    fn highlight_count(box_: &MathBox) -> usize {
        usize::from(box_.highlight.is_some())
            + match &box_.kind {
                BoxKind::Row { children } => children
                    .iter()
                    .map(|(_, _, child)| highlight_count(child))
                    .sum(),
                _ => 0,
            }
    }

    #[test]
    fn a_list_concatenates_widths_on_one_baseline() {
        let box_ = layout(&symbols("xy"), 0, &fake_measure);
        let BoxKind::Row { children } = box_.kind else {
            panic!("list must produce row");
        };
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].0, 0.0);
        assert_eq!(children[1].0, variable_width(0));
        assert_eq!(children[0].1, 0.0);
        assert_eq!(children[1].1, 0.0);
        assert_eq!(box_.width, variable_width(0) * 2.0);
    }

    #[test]
    fn a_fraction_is_as_wide_as_its_wider_operand_plus_padding() {
        let list = vec![fraction(symbols("x"), symbols("yz"))];
        let box_ = layout(&list, 0, &fake_measure);
        let BoxKind::Row { children } = box_.kind else {
            panic!("list must produce row");
        };
        let operand = variable_width(1) * 2.0;
        assert_eq!(children[0].2.width, operand + FRAC_PAD * 2.0);
    }

    #[test]
    fn operands_shrink_one_script_level() {
        let display = layout(&symbols("x"), 0, &fake_measure);
        let script = layout(&symbols("x"), 1, &fake_measure);
        assert!((script.width - display.width * LEVEL_SCALE[1]).abs() < 0.0001);
        assert!((script.ascent - display.ascent * LEVEL_SCALE[1]).abs() < 0.0001);

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
            matches!(box_.kind, BoxKind::Slot { size, visible: true } if (size - SLOT_H * LEVEL_SCALE[1]).abs() < 0.0001)
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
        assert_eq!(x, FRAC_PAD + variable_width(1));
        let expected_y = -(BAR * 0.5
            + BASE_SIZE * FRAC_GAP
            + (BASE_SIZE * 0.5 + VARIABLE_PAD_Y) * LEVEL_SCALE[1]);
        assert!((y - expected_y).abs() < 0.0001);

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
        let base_width = BASE_SIZE * 0.5;
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
        assert_eq!(sub_x, VARIABLE_PAD_X + base_width);
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
    fn new_delimiters_use_their_measured_widths_and_stroked_paths() {
        for (ch, factor, path_parts) in [
            ('|', 0.20, (1, 0)),
            ('‖', 0.30, (2, 0)),
            ('⟨', 0.34, (1, 2)),
            ('⟩', 0.34, (1, 2)),
        ] {
            let box_ = delimiter(ch, BASE_SIZE, 0, 1.0);
            assert_eq!(box_.width, BASE_SIZE * factor);
            let BoxKind::Primitive(MathPrimitive::Stroke {
                path,
                thickness,
                ink,
            }) = box_.kind
            else {
                panic!("delimiter must be stroked geometry");
            };
            assert_eq!(thickness, SHAPE_STROKE);
            assert_eq!(ink, MathInk::Operator);
            assert_eq!(path.matches('M').count(), path_parts.0);
            assert_eq!(path.matches('L').count(), path_parts.1);
        }
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
        let BoxKind::Primitive(MathPrimitive::Stroke {
            path,
            thickness,
            ink,
        }) = &sign.2.kind
        else {
            panic!("radical must be one connected stroke");
        };
        assert_eq!(*thickness, SHAPE_STROKE);
        assert_eq!(*ink, MathInk::Term);
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
            AccentKind::Hat,
            AccentKind::Bar,
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
            assert_eq!(body.2.width, variable_width(0) * 2.0);
        }
    }

    #[test]
    fn hat_and_bar_accents_use_stroked_paths_over_their_body() {
        for (kind, line_count) in [(AccentKind::Hat, 2), (AccentKind::Bar, 0)] {
            let list = layout(&vec![accent(kind, symbols("xy"))], 0, &fake_measure);
            let BoxKind::Row { children } = list.kind else {
                panic!("accent list must produce row");
            };
            let BoxKind::Row { children: accent } = &children[0].2.kind else {
                panic!("accent must produce row");
            };
            let BoxKind::Primitive(MathPrimitive::Stroke {
                path,
                thickness,
                ink,
            }) = &accent[0].2.kind
            else {
                panic!("accent must be stroked geometry");
            };
            assert_eq!(*thickness, SHAPE_STROKE);
            assert_eq!(*ink, MathInk::Term);
            assert_eq!(path.matches('L').count(), line_count);
            if kind == AccentKind::Bar {
                assert!(path.contains(" H "));
            }
        }
    }

    #[test]
    fn alphabetic_and_greek_symbols_are_highlighted() {
        let box_ = layout(
            &vec![
                MathNode::Sym('x'),
                MathNode::Sym('α'),
                MathNode::Sym('Ж'),
                MathNode::Sym('2'),
            ],
            0,
            &fake_measure,
        );
        let BoxKind::Row { children } = box_.kind else {
            panic!("list must produce row");
        };
        assert_eq!(
            children[0].2.highlight,
            Some(math_style::automatic(SymbolRole::Variable, "x"))
        );
        assert_eq!(
            children[1].2.highlight,
            Some(math_style::automatic(SymbolRole::Variable, "\u{03b1}"))
        );
        assert_eq!(
            children[2].2.highlight,
            Some(math_style::automatic(SymbolRole::Variable, "\u{0416}"))
        );
        assert_eq!(children[3].2.highlight, None, "a digit is not a name");
        assert_eq!(children[0].2.width, BASE_SIZE * 0.5 + VARIABLE_PAD_X * 2.0);
        assert_eq!(children[0].2.ascent, BASE_SIZE * 0.5 + VARIABLE_PAD_Y);
        assert_eq!(children[1].0, children[0].2.width);
        // Each letter is its own colour, which is what identity-carries-hue
        // buys: `x` and an alpha must not read as the same term.
        assert_ne!(children[0].2.highlight, children[1].2.highlight);
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
        assert_eq!(
            script.highlight,
            Some(math_style::automatic(SymbolRole::Variable, "x"))
        );
        assert!(script.descent > BASE_SIZE * 0.5);
        assert!(script.width > BASE_SIZE * 0.5);
        assert_eq!(highlight_count(script), 1);
        let BoxKind::Row { children } = &script.kind else {
            panic!("script must produce row");
        };
        assert_eq!(children[0].0, VARIABLE_PAD_X);
    }

    #[test]
    fn resolved_roles_color_one_pill_around_their_rendered_body() {
        for role in [
            SymbolRole::Variable,
            SymbolRole::Constant,
            SymbolRole::Function,
        ] {
            let list = layout(&vec![resolved(role, symbols("π"))], 0, &fake_measure);
            let BoxKind::Row { children } = list.kind else {
                panic!("list must produce row");
            };
            let symbol = &children[0].2;
            assert_eq!(
                symbol.highlight,
                Some(math_style::automatic(role, "test.symbol"))
            );
            assert_eq!(highlight_count(symbol), 1);
            assert_eq!(symbol.width, BASE_SIZE * 0.5 + VARIABLE_PAD_X * 2.0);
        }
    }

    #[test]
    fn an_external_script_joins_a_resolved_symbols_role_pill() {
        let list = layout(
            &vec![script(
                vec![resolved(SymbolRole::Constant, symbols("ε"))],
                None,
                Some(symbols("0")),
            )],
            0,
            &fake_measure,
        );
        let BoxKind::Row { children } = list.kind else {
            panic!("list must produce row");
        };
        let symbol = &children[0].2;
        assert_eq!(
            symbol.highlight,
            Some(math_style::automatic(SymbolRole::Constant, "test.symbol"))
        );
        assert_eq!(highlight_count(symbol), 1);
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

    /// The integral family is set from the font — like ∑, ∏ and lim —, and
    /// condensed, because JuliaMono's `∫`/`∮` are full-width monospace cells
    /// that read as a wide swash at [`BIGOP_SCALE`]. Rendering through the
    /// text pipeline is what keeps its edges as smooth as the glyphs around
    /// it; condensing is what keeps it tall and narrow, and rising clears its
    /// lower limit.
    #[test]
    fn the_integral_is_a_raised_condensed_glyph_not_a_drawn_path() {
        for (kind, text) in [(BigOp::Integral, "∫"), (BigOp::ContourIntegral, "∮")] {
            let list = layout(
                &vec![big_op(kind, Vec::new(), Vec::new())],
                0,
                &fake_measure,
            );
            let BoxKind::Row { children } = &list.kind else {
                panic!("list must produce row");
            };
            let BoxKind::Row { children: operator } = &children[0].2.kind else {
                panic!("operator must produce row");
            };
            let BoxKind::Glyph {
                text: glyph,
                size,
                condense,
                offset_y,
                ..
            } = &operator[0].2.kind
            else {
                panic!("the integral must be a glyph, not a drawn path");
            };
            assert_eq!(glyph.as_str(), text);
            assert!((*size - BASE_SIZE * BIGOP_SCALE).abs() < 0.0001);
            assert!(
                (*condense - INTEGRAL_CONDENSE * MATH_CONDENSE).abs() < 0.0001,
                "the integral keeps its own narrow shape, squeezed like the rest"
            );
            assert!(
                *condense < MATH_CONDENSE,
                "the integral must be narrower than an ordinary glyph"
            );
            assert!(
                (*offset_y - *size * INTEGRAL_RISE).abs() < 0.0001,
                "the integral glyph must ride above the anchor"
            );
        }

        // The sum receives its own optical width and rise just like the
        // integral family, while product and lim stay on the anchor.
        let sum = layout(
            &vec![big_op(BigOp::Sum, Vec::new(), Vec::new())],
            0,
            &fake_measure,
        );
        let BoxKind::Row { children } = &sum.kind else {
            panic!("list must produce row");
        };
        let BoxKind::Row { children: operator } = &children[0].2.kind else {
            panic!("operator must produce row");
        };
        let BoxKind::Glyph {
            offset_y,
            condense,
            ink,
            ..
        } = &operator[0].2.kind
        else {
            panic!("the sum must be a glyph");
        };
        assert!(
            (*condense - SUM_CONDENSE * MATH_CONDENSE).abs() < 0.0001,
            "the sum keeps its own ratio, squeezed by the global one"
        );
        assert!((*offset_y - BASE_SIZE * BIGOP_SCALE * SUM_RISE).abs() < 0.0001);
        assert_eq!(*ink, MathInk::Operator);

        for kind in [BigOp::Prod, BigOp::Limit] {
            let box_ = layout(
                &vec![big_op(kind, Vec::new(), Vec::new())],
                0,
                &fake_measure,
            );
            let BoxKind::Row { children } = &box_.kind else {
                panic!("list must produce row");
            };
            let BoxKind::Row { children: operator } = &children[0].2.kind else {
                panic!("operator must produce row");
            };
            let BoxKind::Glyph { offset_y, ink, .. } = operator[0].2.kind else {
                panic!("large operator must be a glyph");
            };
            assert_eq!(offset_y, 0.0);
            assert_eq!(ink, MathInk::Operator);
        }
    }

    #[test]
    fn operators_and_relations_are_classified_apart_from_ordinary_atoms() {
        use AtomClass::*;
        for ch in ['+', '-', '·', '×', '∪', '∩', '∧'] {
            assert_eq!(char_class(ch), Bin, "{ch} should be a binary operator");
        }
        for ch in ['=', '<', '>', '≤', '∈', '⊂', '→'] {
            assert_eq!(char_class(ch), Rel, "{ch} should be a relation");
        }
        for ch in ['x', 'X', '1', 'α', '∞', '∫', '∑'] {
            assert_eq!(char_class(ch), Ord, "{ch} should be ordinary");
        }
        assert_eq!(char_class(','), Punct);
        assert_eq!(char_class('('), Open);
        assert_eq!(char_class(')'), Close);
    }

    /// The space around operators follows TeX's atom classes: medium around a
    /// binary operator, thick around a relation, thin after punctuation, and
    /// none around ordinary atoms — with a leading/trailing binary operator
    /// (a unary sign) demoted so it gets no space.
    #[test]
    fn operators_are_spaced_from_operands_but_unary_signs_are_not() {
        let w = |s: &str| layout(&symbols(s), 0, &fake_measure).width;
        let xw = w("x");
        let pw = w("+");
        let ew = w("=");
        let mw = w("-");
        let cw = w(",");
        let med = MED_SPACE * BASE_SIZE;
        let thick = THICK_SPACE * BASE_SIZE;
        let thin = THIN_SPACE * BASE_SIZE;
        let eps = 0.0001;

        // x + y — medium space on both sides of the plus.
        let BoxKind::Row { children } = layout(&symbols("x+y"), 0, &fake_measure).kind else {
            panic!("expected row");
        };
        assert!((children[1].0 - (xw + med)).abs() < eps);
        assert!((children[2].0 - (children[1].0 + pw + med)).abs() < eps);

        // x = y — thick space on both sides of the relation.
        let BoxKind::Row { children } = layout(&symbols("x=y"), 0, &fake_measure).kind else {
            panic!("expected row");
        };
        assert!((children[1].0 - (xw + thick)).abs() < eps);
        assert!((children[2].0 - (children[1].0 + ew + thick)).abs() < eps);

        // xy — adjacent ordinary atoms stay flush.
        let BoxKind::Row { children } = layout(&symbols("xy"), 0, &fake_measure).kind else {
            panic!("expected row");
        };
        assert!((children[1].0 - xw).abs() < eps);

        // -x — a leading minus is a unary sign: no space before x.
        let BoxKind::Row { children } = layout(&symbols("-x"), 0, &fake_measure).kind else {
            panic!("expected row");
        };
        assert!((children[1].0 - mw).abs() < eps);

        // x- — a trailing minus is a unary sign too: no space after x.
        let BoxKind::Row { children } = layout(&symbols("x-"), 0, &fake_measure).kind else {
            panic!("expected row");
        };
        assert!((children[1].0 - xw).abs() < eps);

        // x, y — thin space after the comma, none before it.
        let BoxKind::Row { children } = layout(&symbols("x,y"), 0, &fake_measure).kind else {
            panic!("expected row");
        };
        assert!((children[1].0 - xw).abs() < eps);
        assert!((children[2].0 - (children[1].0 + cw + thin)).abs() < eps);
    }

    #[test]
    fn empty_integral_limits_are_invisible_without_changing_the_operator_box() {
        for kind in [BigOp::Integral, BigOp::ContourIntegral] {
            let source = vec![big_op(kind, Vec::new(), Vec::new())];
            let list = layout(&source, 0, &fake_measure);
            let bounds = interaction_bounds(&list);
            assert!(bounds.ascent > list.ascent);
            assert!(bounds.descent > list.descent);
            let BoxKind::Row { children } = &list.kind else {
                panic!("list must produce row");
            };
            let operator_box = &children[0].2;
            let BoxKind::Row { children: operator } = &operator_box.kind else {
                panic!("operator must produce row");
            };
            // Set from the font, condensed to the integral's narrow shape —
            // its own ratio, squeezed by the global one like every glyph.
            assert!(matches!(
                &operator[0].2.kind,
                BoxKind::Glyph { condense, .. }
                    if (*condense - INTEGRAL_CONDENSE * MATH_CONDENSE).abs() < 0.0001
            ));
            assert_eq!(operator_box.width, operator[0].2.width);
            assert_eq!(operator_box.ascent, operator[0].2.ascent);
            assert_eq!(operator_box.descent, operator[0].2.descent);
            for (index, slot) in [Slot::Lower, Slot::Upper].into_iter().enumerate() {
                let hidden = &operator[index + 1];
                assert!(matches!(
                    hidden.2.kind,
                    BoxKind::Slot { visible: false, .. }
                ));
                assert_eq!(hidden.2.width, SLOT_W * LEVEL_SCALE[1]);
                assert_eq!(hidden.2.ascent + hidden.2.descent, SLOT_H * LEVEL_SCALE[1]);
                let cursor = MathCursor {
                    path: vec![Step { index: 0, slot }],
                    index: 0,
                };
                let (_, y, _) = cursor_pos(&source, &cursor, 0, &fake_measure);
                assert_eq!(y, hidden.1);
                match slot {
                    Slot::Lower => assert!(y < 0.0),
                    Slot::Upper => assert!(y > 0.0),
                    _ => unreachable!(),
                }

                let hit_cursor = hit(
                    &source,
                    (hidden.0 + hidden.2.width * 0.5, hidden.1),
                    0,
                    &fake_measure,
                );
                assert_eq!(hit_cursor.path, vec![Step { index: 0, slot }]);
                assert_eq!(hit_cursor.index, 0);
            }
        }
    }

    /// Every path this module emits goes to `Layer::draw_path`, which
    /// `expect`s a successful parse — so a malformed one is a panic in the
    /// running app, on the frame the reader first types that structure.
    /// Parsed here through the very parser the renderer uses, so a path can
    /// never reach the screen untested.
    #[test]
    fn every_generated_path_parses_as_svg() {
        fn parses(d: &str) -> bool {
            let mut parser = lyon_extra::parser::PathParser::new();
            let mut builder = lyon::path::Path::builder();
            let mut source = lyon_extra::parser::Source::new(d.chars());
            parser
                .parse(
                    &lyon_extra::parser::ParserOptions::DEFAULT,
                    &mut source,
                    &mut builder,
                )
                .is_ok()
        }

        let mut checked = 0;
        let mut check = |box_: &MathBox| {
            fn walk(box_: &MathBox, out: &mut Vec<String>) {
                match &box_.kind {
                    BoxKind::Primitive(MathPrimitive::Stroke { path, .. }) => {
                        out.push(path.clone())
                    }
                    BoxKind::Row { children } => {
                        for (_, _, child) in children {
                            walk(child, out);
                        }
                    }
                    _ => {}
                }
            }
            let mut paths = Vec::new();
            walk(box_, &mut paths);
            assert!(!paths.is_empty(), "expected at least one stroked path");
            for path in paths {
                assert!(parses(&path), "lyon rejected: {path}");
                checked += 1;
            }
        };

        // Delimiters and the radical go through the same `draw_path`.
        for (open, close) in [('(', ')'), ('[', ']'), ('|', '|'), ('⟨', '⟩')] {
            let group = vec![MathNode::Group {
                open,
                close,
                body: vec![MathNode::Sym('x')],
            }];
            check(&layout(&group, 0, &fake_measure));
        }
        check(&layout(
            &vec![MathNode::Sqrt {
                body: vec![MathNode::Sym('x')],
            }],
            0,
            &fake_measure,
        ));
        assert!(checked >= 8, "only {checked} paths reached the parser");
    }

    #[test]
    fn empty_non_integral_limits_keep_their_visible_placeholders() {
        for kind in [BigOp::Sum, BigOp::Limit] {
            let source = vec![big_op(kind, Vec::new(), Vec::new())];
            let list = layout(&source, 0, &fake_measure);
            let BoxKind::Row { children } = &list.kind else {
                panic!("list must produce row");
            };
            let operator_box = &children[0].2;
            let BoxKind::Row { children: operator } = &operator_box.kind else {
                panic!("operator must produce row");
            };
            assert!(matches!(
                operator[1].2.kind,
                BoxKind::Slot { visible: true, .. }
            ));
            assert!(operator[1].2.width > 0.0);
            assert!(operator_box.descent > operator[0].2.descent);
            if kind == BigOp::Sum {
                assert!(matches!(
                    operator[2].2.kind,
                    BoxKind::Slot { visible: true, .. }
                ));
                assert!(operator_box.ascent > operator[0].2.ascent);
            }
        }
    }

    #[test]
    fn contour_integral_populated_limits_stack_normally() {
        let list = layout(
            &vec![big_op(
                BigOp::ContourIntegral,
                symbols("lower"),
                symbols("upper"),
            )],
            0,
            &fake_measure,
        );
        let BoxKind::Row { children } = list.kind else {
            panic!("list must produce row");
        };
        let BoxKind::Row { children: operator } = &children[0].2.kind else {
            panic!("operator must produce row");
        };
        assert!(operator[1].1 < 0.0);
        assert!(operator[2].1 > 0.0);
        assert!(operator[1].2.width > 0.0);
        assert!(operator[2].2.width > 0.0);
    }

    #[test]
    fn adjacent_integral_family_operators_overlap_only_each_other() {
        let empty = |kind| big_op(kind, Vec::new(), Vec::new());
        for left in [BigOp::Integral, BigOp::ContourIntegral] {
            for right in [BigOp::Integral, BigOp::ContourIntegral] {
                let left_width = layout(&vec![empty(left)], 0, &fake_measure).width;
                let right_width = layout(&vec![empty(right)], 0, &fake_measure).width;
                let pair = layout(&vec![empty(left), empty(right)], 0, &fake_measure);
                assert!(
                    (pair.width - (left_width + right_width - INTEGRAL_OVERLAP)).abs() < 0.0001
                );
            }
        }

        let integral = layout(&vec![empty(BigOp::Integral)], 0, &fake_measure).width;
        let sum = layout(&vec![empty(BigOp::Sum)], 0, &fake_measure).width;
        let mixed = layout(
            &vec![empty(BigOp::Integral), empty(BigOp::Sum)],
            0,
            &fake_measure,
        );
        assert!((mixed.width - (integral + sum)).abs() < 0.0001);

        let script_pair = layout(
            &vec![empty(BigOp::Integral), empty(BigOp::ContourIntegral)],
            1,
            &fake_measure,
        );
        let script_singles = layout(&vec![empty(BigOp::Integral)], 1, &fake_measure).width
            + layout(&vec![empty(BigOp::ContourIntegral)], 1, &fake_measure).width;
        assert!(
            (script_pair.width - (script_singles - INTEGRAL_OVERLAP * LEVEL_SCALE[1])).abs()
                < 0.0001
        );
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
            BoxKind::Glyph { ref text, size, .. } if text == "lim" && size == BASE_SIZE
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

    #[test]
    fn normal_hit_prefers_the_deepest_symbol_then_structural_geometry() {
        let source = vec![fraction(
            vec![resolved(SymbolRole::Constant, symbols("π"))],
            symbols("d"),
        )];
        let laid_out = layout(&source, 0, &fake_measure);
        let BoxKind::Row { children } = &laid_out.kind else {
            panic!("list must be a row");
        };
        let fraction = &children[0];
        let BoxKind::Row {
            children: fraction_children,
        } = &fraction.2.kind
        else {
            panic!("fraction must be a row");
        };
        let numerator = &fraction_children[0];

        assert_eq!(
            hit_node(
                &source,
                (
                    fraction.0 + numerator.0 + numerator.2.width * 0.5,
                    fraction.1 + numerator.1,
                ),
                0,
                &fake_measure,
            ),
            Some(NodeAddress {
                path: vec![Step {
                    index: 0,
                    slot: Slot::Num,
                }],
                index: 0,
            })
        );
        assert_eq!(
            hit_node(
                &source,
                (fraction.0 + fraction.2.width * 0.5, fraction.1),
                0,
                &fake_measure,
            ),
            Some(NodeAddress {
                path: Vec::new(),
                index: 0,
            })
        );
    }

    #[test]
    fn normal_hit_leaves_whitespace_untargeted_and_resolved_atomic() {
        let source = vec![resolved(
            SymbolRole::Variable,
            vec![script(symbols("x"), Some(symbols("2")), None)],
        )];
        let laid_out = layout(&source, 0, &fake_measure);

        assert_eq!(
            hit_node(&source, (laid_out.width * 0.5, 0.0), 0, &fake_measure,),
            Some(NodeAddress {
                path: Vec::new(),
                index: 0,
            })
        );
        assert_eq!(
            hit_node(&source, (laid_out.width + 1.0, 0.0), 0, &fake_measure),
            None
        );
    }

    #[test]
    fn normal_hit_uses_paint_order_for_overlapping_siblings() {
        let source = vec![
            big_op(BigOp::Integral, Vec::new(), Vec::new()),
            big_op(BigOp::ContourIntegral, Vec::new(), Vec::new()),
        ];
        let laid_out = layout(&source, 0, &fake_measure);
        let BoxKind::Row { children } = &laid_out.kind else {
            panic!("list must be a row");
        };
        let overlap_x = children[1].0 + INTEGRAL_OVERLAP * 0.5;

        assert_eq!(
            hit_node(&source, (overlap_x, 0.0), 0, &fake_measure),
            Some(NodeAddress {
                path: Vec::new(),
                index: 1,
            })
        );
    }

    #[test]
    fn addressed_bounds_follow_nested_layout_offsets() {
        let source = vec![fraction(symbols("ab"), symbols("c"))];
        let address = NodeAddress {
            path: vec![Step {
                index: 0,
                slot: Slot::Num,
            }],
            index: 1,
        };
        let laid_out = layout(&source, 0, &fake_measure);
        let BoxKind::Row { children } = &laid_out.kind else {
            panic!("list must be a row");
        };
        let fraction = &children[0];
        let BoxKind::Row {
            children: fraction_children,
        } = &fraction.2.kind
        else {
            panic!("fraction must be a row");
        };
        let numerator = &fraction_children[0];
        let BoxKind::Row {
            children: numerator_children,
        } = &numerator.2.kind
        else {
            panic!("numerator must be a row");
        };
        let symbol = &numerator_children[1];
        let expected_left = fraction.0 + numerator.0 + symbol.0;
        let expected_anchor = fraction.1 + numerator.1 + symbol.1;

        assert_eq!(
            node_bounds(&source, &address, 0, &fake_measure),
            Some(NodeBounds {
                left: expected_left,
                right: expected_left + symbol.2.width,
                top: expected_anchor + symbol.2.ascent,
                bottom: expected_anchor - symbol.2.descent,
            })
        );
        assert_eq!(
            node_bounds(
                &source,
                &NodeAddress {
                    path: Vec::new(),
                    index: 9,
                },
                0,
                &fake_measure,
            ),
            None
        );
    }

    #[test]
    fn circular_hit_includes_a_tangent_corner() {
        let source = symbols("x");
        let bounds = node_bounds(
            &source,
            &NodeAddress {
                path: Vec::new(),
                index: 0,
            },
            0,
            &fake_measure,
        )
        .expect("symbol bounds");
        let point = (bounds.left - 3.0, bounds.bottom - 4.0);

        assert_eq!(
            hit_nodes_in_circle(&source, point, 5.0, 0, &fake_measure),
            vec![NodeAddress {
                path: Vec::new(),
                index: 0,
            }]
        );
        assert!(hit_nodes_in_circle(&source, point, 4.99, 0, &fake_measure).is_empty());
    }

    #[test]
    fn circular_hit_returns_deep_siblings_in_paint_order() {
        let source = vec![fraction(symbols("ab"), symbols("d"))];
        let expression = layout(&source, 0, &fake_measure);
        let BoxKind::Row { children } = &expression.kind else {
            panic!("expression must be a row");
        };
        let fraction = &children[0];
        let BoxKind::Row {
            children: fraction_children,
        } = &fraction.2.kind
        else {
            panic!("fraction must be a row");
        };
        let numerator = &fraction_children[0];
        let BoxKind::Row {
            children: numerator_children,
        } = &numerator.2.kind
        else {
            panic!("numerator must be a row");
        };
        let boundary = numerator_children[1].0;
        let point = (
            fraction.0 + numerator.0 + boundary,
            fraction.1 + numerator.1,
        );

        assert_eq!(
            hit_nodes_in_circle(&source, point, 0.0, 0, &fake_measure),
            vec![
                NodeAddress {
                    path: vec![Step {
                        index: 0,
                        slot: Slot::Num,
                    }],
                    index: 1,
                },
                NodeAddress {
                    path: vec![Step {
                        index: 0,
                        slot: Slot::Num,
                    }],
                    index: 0,
                },
            ]
        );
    }
}
