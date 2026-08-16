//! Guara (light) — colours, type, and icons.
//!
//! Palette is transcribed from `~/.dev/Guara/COLOR.md`; the role names are
//! that document's, so a value here can be checked against it directly.
//! Nothing else in the app names a colour or a font family.

use crate::document::BadgeColor;
use crate::layout::Rect;
use crate::renderer::{
    Alignment, Color, Font, FontParameters, HorizontalAlign, Layer, LineCap, LineJoin, PathPaint,
    Rounding, Stroke, VerticalAlign,
};

// -- Surfaces --
pub const BACKGROUND: Color = Color::rgb(0xF8, 0xED, 0xD0);
/// Panels and chrome that sit behind the page (Guara's gutter).
pub const PANEL: Color = Color::rgb(0xF0, 0xE3, 0xBE);
/// Cursor line / alternate background — also the math slot fill.
pub const ALT: Color = Color::rgb(0xEF, 0xE0, 0xA8);
/// Chrome / statusline. Lighter than `PANEL`: the tab strip and breadcrumb.
pub const CHROME: Color = Color::rgb(0xFA, 0xF1, 0xDB);
pub const SELECTION: Color = Color::rgb(0xD2, 0xB2, 0x6E);
/// Behind code, inline and fenced. A clear step down from `BACKGROUND` —
/// the boundary of a code span has to read at a glance, since nothing else
/// marks it.
pub const CODE: Color = Color::rgb(0xD9, 0xC9, 0x9E);
/// Behind a display math block. A sibling of `CODE` rather than the same
/// tint: both are slabs of machinery on a page of prose, and telling one
/// from the other at a glance is the whole point of tinting them at all.
/// Cooler and greyer than code's warm tan, which is what separates them
/// without introducing a colour the palette does not already live in.
pub const MATH: Color = Color::rgb(0xD5, 0xCD, 0xB4);
/// Subtle expression-level tint for math embedded in prose. Kept translucent
/// so the current-line and selection bands remain legible underneath it.
pub const INLINE_MATH: Color = Color::rgba(0xDD, 0xD4, 0xBA, 0x88);
pub const BORDER: Color = Color::rgb(0xDD, 0xD0, 0xA0);

// -- Badges and highlights --
/// Type size of a badge's label. Small enough that the chip is shorter
/// than the line it sits on, so it never opens the leading up.
pub const BADGE_SIZE: f32 = 9.0;
/// A badge's label and its outline share one colour, as in the design —
/// the box is a hairline, not a filled tag.
pub const BADGE_INK: Color = STRUCTURE;
pub const BADGE_BLUE: Color = Color::rgb(0x1F, 0x70, 0xA0);
pub const BADGE_GREEN: Color = Color::rgb(0x3E, 0x7D, 0x59);
pub const BADGE_PURPLE: Color = Color::rgb(0x7B, 0x52, 0xA0);

pub fn badge_ink(color: BadgeColor) -> Color {
    match color {
        BadgeColor::Orange => BADGE_INK,
        BadgeColor::Blue => BADGE_BLUE,
        BadgeColor::Green => BADGE_GREEN,
        BadgeColor::Purple => BADGE_PURPLE,
    }
}
/// Space between a badge's label and its box, each side. Part of the run's
/// advance (see `document::layout::advance`), not just of the drawing —
/// a chip that flows as if it were only its label overlaps whatever comes
/// next.
pub const BADGE_PAD: f32 = 4.0;
/// Height of a badge's box. Fixed rather than derived from the line, so a
/// chip in a heading is the same chip as one in a paragraph.
pub const BADGE_HEIGHT: f32 = 16.0;
/// Underline highlight. Drawn as a bar *below* the text rather than a
/// wash behind it, so the glyphs keep the page's own contrast.
pub const HIGHLIGHT: Color = Color::rgb(0xE0, 0xA8, 0x2C);
/// Saturated role colors behind resolved mathematical symbols. They sit
/// outside the page's warm surface palette so semantic tokens stand out.
pub const VARIABLE: Color = Color::rgb(0x6E, 0x9A, 0xF5);
pub const CONSTANT: Color = Color::rgb(0xEE, 0x91, 0x45);
pub const FUNCTION: Color = Color::rgb(0x65, 0xB8, 0x78);
/// Quietest ink that is still ink: dates, hints, disabled glyphs.
pub const FAINT: Color = Color::rgb(0xA0, 0x91, 0x83);
/// Non-text: separators inside a line of type, empty-slot outlines.
pub const NON_TEXT: Color = Color::rgb(0xC4, 0xB7, 0x9E);

// -- Foreground --
pub const INK: Color = Color::rgb(0x3C, 0x38, 0x36);
pub const DIM: Color = Color::rgb(0x7C, 0x6F, 0x64);
pub const COMMENT: Color = Color::rgb(0x92, 0x83, 0x74);

// -- Semantic accents (Guara's syntax hues, reused as UI roles) --
/// Keyword orange: the one loud colour. Mode badge, active markers, caret.
pub const ACCENT: Color = Color::rgb(0xC2, 0x4F, 0x1A);
/// Type orange, a shade deeper: structural annotations.
pub const STRUCTURE: Color = Color::rgb(0xA8, 0x4C, 0x00);
/// Function green: things that are healthy or live.
pub const LIVE: Color = Color::rgb(0x3E, 0x7D, 0x59);
/// Global teal — the deliberate cool outlier in a warm palette.
pub const COOL: Color = Color::rgb(0x0D, 0x66, 0x78);
pub const WARNING: Color = Color::rgb(0xB0, 0x7B, 0x18);

/// Faux-bold outline expansion, as a fraction of font size.
const FAUX_BOLD_WEIGHT_RATIO: f32 = 0.025;

/// Prose. Georgia is what the design specifies and what is installed.
pub fn serif() -> Font {
    Font::Named("Georgia".into())
}

/// Labels, numbers, and anything that wants to read as machinery.
pub fn mono() -> Font {
    Font::Named("Essential PragmataPro".into())
}

/// Math. Stays separate from `mono` because the two answer different
/// needs — `mono` is for labels and machinery, this one is for notation.
/// JuliaMono is chosen for glyph coverage: Greek, blackboard bold,
/// operators, and the wide symbol range mathematical notation reaches
/// for, which a UI monospace does not carry. Its monospace advance keeps
/// every glyph's width uniform, so an expression's boxes stay predictable
/// as it is edited.
pub fn math() -> Font {
    Font::Named("JuliaMono".into())
}

/// A font plus everything `draw_text` needs to reproduce one look.
#[derive(Clone, Debug)]
pub struct TextStyle {
    pub font: Font,
    pub size: f32,
    pub color: Color,
    /// Faux-bold outline expansion, in logical pixels.
    pub weight: f32,
    /// Extra letter-spacing, in EM.
    pub tracking: f32,
    /// Faux-italic shear factor; `0.0` is upright.
    pub slant: f32,
}

impl TextStyle {
    pub fn serif(size: f32, color: Color) -> Self {
        Self {
            font: serif(),
            size,
            color,
            weight: 0.0,
            tracking: 0.0,
            slant: 0.0,
        }
    }

    pub fn mono(size: f32, color: Color) -> Self {
        Self {
            font: mono(),
            size,
            color,
            weight: 0.0,
            tracking: 0.0,
            slant: 0.0,
        }
    }

    pub fn math(size: f32, color: Color) -> Self {
        Self {
            font: math(),
            size,
            color,
            weight: 0.0,
            tracking: 0.0,
            slant: 0.0,
        }
    }

    /// Faux bold. Georgia ships as a single regular cut here, so weight is
    /// synthesised; a light expansion reads as 600 rather than filling in
    /// the counters at body sizes.
    pub fn bold(mut self) -> Self {
        self.weight = self.size * FAUX_BOLD_WEIGHT_RATIO;
        self
    }

    /// Faux italic: a 12° shear. Georgia ships no italic cut, so — like
    /// faux bold — the slant is synthesized by the renderer.
    pub fn italic(mut self) -> Self {
        self.slant = 0.21;
        self
    }

    pub fn tracked(mut self, em: f32) -> Self {
        self.tracking = em;
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    pub fn size(mut self, size: f32) -> Self {
        // Keep a bold style bold when it is resized.
        let bold = self.weight > 0.0;
        self.size = size;
        if bold {
            self.weight = size * FAUX_BOLD_WEIGHT_RATIO;
        }
        self
    }

    pub fn parameters(&self) -> FontParameters {
        FontParameters {
            size: self.size,
            weight: self.weight,
            width: 1.0,
            tracking: self.tracking,
            slant: self.slant,
        }
    }
}

pub const LEFT: Alignment = Alignment {
    horizontal: HorizontalAlign::Left,
    vertical: VerticalAlign::Center,
};
pub const RIGHT: Alignment = Alignment {
    horizontal: HorizontalAlign::Right,
    vertical: VerticalAlign::Center,
};
pub const CENTER: Alignment = Alignment::CENTER;
pub const TOP_LEFT: Alignment = Alignment::TOP_LEFT;

/// Draws `text` with `at` as the point named by `align`.
pub fn draw(layer: &Layer, text: &str, at: (f32, f32), style: &TextStyle, align: Alignment) {
    layer.draw_text(
        text,
        at,
        style.color.clone(),
        align,
        style.font.clone(),
        style.parameters(),
    );
}

pub fn width(layer: &Layer, text: &str, style: &TextStyle) -> f32 {
    layer
        .get_text_size(text, &style.font, &style.parameters())
        .0
}

/// The same colour at a different opacity. Non-solid colours are returned
/// unchanged — there is no single alpha to set on a gradient.
pub fn fade(color: Color, alpha: f32) -> Color {
    match color {
        Color::Solid([r, g, b, _]) => Color::rgba(r, g, b, (alpha.clamp(0.0, 1.0) * 255.0) as u8),
        other => other,
    }
}

/// Linear blend between two solid colors, `t` in 0..=1. Non-solid inputs
/// are returned as `from` — there is no single channel blend for a gradient.
pub fn mix(from: Color, to: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t) as u8;
    match (from.clone(), to) {
        (Color::Solid(a), Color::Solid(b)) => {
            Color::rgba(lerp(a[0], b[0]), lerp(a[1], b[1]), lerp(a[2], b[2]), a[3])
        }
        _ => from,
    }
}

pub fn rule(layer: &Layer, at: (f32, f32), length: f32, thickness: f32, color: Color) {
    layer.draw_rectangle(at, (length, thickness), color, Rounding::NONE);
}

pub fn vertical_rule(layer: &Layer, at: (f32, f32), length: f32, thickness: f32, color: Color) {
    layer.draw_rectangle(at, (thickness, length), color, Rounding::NONE);
}

/// A 1px outline, drawn as four rules.
pub fn outline(layer: &Layer, rect: Rect, color: Color) {
    rule(layer, rect.position(), rect.width, 1.0, color.clone());
    rule(
        layer,
        (rect.x, rect.bottom() - 1.0),
        rect.width,
        1.0,
        color.clone(),
    );
    vertical_rule(layer, rect.position(), rect.height, 1.0, color.clone());
    vertical_rule(layer, (rect.right() - 1.0, rect.y), rect.height, 1.0, color);
}

/// The hover surface every interactive row and button shares: one tint, one
/// opacity curve, so nothing in the interface highlights differently from
/// anything else. A weight of zero draws nothing at all.
pub fn hover_fill(layer: &Layer, rect: Rect, weight: f32) {
    if weight > 0.0 {
        layer.draw_rectangle(
            rect.position(),
            rect.size(),
            fade(SELECTION, 0.32 * weight),
            Rounding::NONE,
        );
    }
}

/// Draws one of the [`icons`] paths, `at` being its top-left corner.
///
/// `stroke` is in the icon's own 24-unit space, exactly as the `d` string's
/// source SVG means it, so the design's `stroke-width="1.8"` is `1.8` here
/// regardless of the size the icon is drawn at.
pub fn icon(layer: &Layer, path: &str, at: (f32, f32), size: f32, color: Color, stroke: f32) {
    let mut pen = Stroke::new(color, stroke);
    pen.cap = LineCap::Round;
    pen.join = LineJoin::Round;
    layer
        .draw_svg_icon(
            path,
            (0.0, 0.0, 24.0, 24.0),
            (size, size),
            at,
            PathPaint::Stroke(pen),
        )
        .expect("icon paths are constants — a parse failure is a typo, not input");
}

/// Feather-style 24×24 stroked icons, taken from the design.
///
/// Circles and rects from the source SVG are written out as arcs and
/// closed subpaths: `draw_svg_icon` takes one path's `d`, not a document.
pub mod icons {
    pub const CHECK: &str = "M20 6L9 17l-5-5";
    pub const FILE: &str =
        "M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7z M14 2L14 8L20 8";
    pub const FILE_LINES: &str = "M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7z \
                                  M14 2L14 8L20 8 M16 13H8 M16 17H8";
    pub const PLUS: &str = "M5 12h14M12 5v14";
    pub const X: &str = "M18 6L6 18M6 6l12 12";
    pub const FOLDER: &str = "M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 \
                              3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2z";
    pub const CHEVRON_DOWN: &str = "M6 9L12 15L18 9";
    pub const CHEVRON_RIGHT: &str = "M9 18L15 12L9 6";
    pub const SORT: &str = "M11 5h10M11 12h10M11 19h10M3 8l3-3 3 3M6 5v14";
    pub const CALENDAR: &str = "M3 4H21V22H3Z M16 2v4M8 2v4M3 10h18";
    pub const CLOCK: &str = "M3 12a9 9 0 1 0 18 0a9 9 0 1 0-18 0 M12 7v5l3 2";
    pub const BOOK: &str = "M4 19.5A2.5 2.5 0 0 1 6.5 17H20 M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z";
    pub const SEARCH: &str = "M3 11a8 8 0 1 0 16 0a8 8 0 1 0-16 0 M21 21l-4.3-4.3";
    pub const ALERT: &str = "M12 8v5M12 17h.01";
    pub const TOPICS: &str = "M4 9h16M4 15h16M10 3L8 21M16 3l-2 18";
    pub const BRANCH: &str = "M6 3V15 M15 6a3 3 0 1 0 6 0a3 3 0 1 0-6 0 \
                              M3 18a3 3 0 1 0 6 0a3 3 0 1 0-6 0 M18 9a9 9 0 0 1-9 9";
    /// The "you are inside a structure" marker in the status line.
    pub const NEXT_SLOT: &str = "M18 7V4H6l6 8-6 8h12v-3";
}
