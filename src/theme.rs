//! Colours, type, and icons.
//!
//! A [`Theme`] names every colour the application draws with, and
//! [`ThemeServer`] owns the one that is current. Nothing outside this file
//! names a colour or a font family, and nothing outside this file holds a
//! colour past the frame it drew with it — [`set`] swaps the palette and
//! every call site picks the new value up on its next read.
//!
//! Studio pairs chalk and mulberry surfaces with a warm coral focus accent.
//! Inter is embedded, so UI and prose have the same metrics on every OS.

use std::sync::{PoisonError, RwLock, RwLockReadGuard};

use crate::document::BadgeColor;
use crate::document::math_style::MathHue;
use crate::document::table::TableLines;
use crate::layout::Rect;
use crate::renderer::{
    Alignment, Color, Font, FontParameters, GradientDirection, HorizontalAlign, Layer, LineCap,
    LineJoin, PathPaint, Rounding, Stroke, VerticalAlign,
};

/// Whether a palette is a light one or a dark one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Light,
    Dark,
}

impl Mode {
    /// The other one.
    pub fn flipped(self) -> Self {
        match self {
            Mode::Light => Mode::Dark,
            Mode::Dark => Mode::Light,
        }
    }
}

/// Every colour the application draws with, one palette's worth. Sizes and
/// spacings are *not* here: a badge is the same size in every theme, so
/// swapping palettes can never move anything on the page.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    /// Whether this palette reads as a light or a dark one. Not a colour
    /// and not derived from one: it is the theme's own claim about itself,
    /// so the one affordance that has to know — the switch, which draws a
    /// sun or a moon and picks a counterpart to swap to — asks instead of
    /// guessing from a luminance threshold.
    pub mode: Mode,

    // Surfaces, from the document to the surrounding chrome and floating cards.
    pub background: Color,
    pub panel: Color,
    pub popup: Color,
    pub alt: Color,
    pub chrome: Color,
    pub selection: Color,
    pub code: Color,
    pub math: Color,
    pub border: Color,

    // Document annotations and mathematical roles.
    pub badge_ink: Color,
    pub badge_blue: Color,
    pub badge_green: Color,
    pub badge_purple: Color,
    pub highlight: Color,
    /// The ink a numeral is set in. Numbers are quantities, not names, and
    /// reading one is a different act from reading a variable — so they get
    /// an ink of their own rather than borrowing the prose's.
    pub math_number: Color,
    /// The ink an operator, relation, or piece of punctuation is set in.
    /// Quieter than [`Self::ink`] on purpose: `+` and `=` are the grammar
    /// of an expression, and grammar should not compete with the terms it
    /// joins.
    pub math_operator: Color,
    /// The identity washes behind symbols, in [`MathHue::ALL`] order. Fills
    /// only — an outline is derived from its fill by [`Theme::shade`], so a
    /// border can never drift off the hue it belongs to.
    pub math_hues: [Color; MathHue::ALL.len()],

    // Text and non-text hierarchy.
    pub faint: Color,
    pub non_text: Color,
    pub ink: Color,
    pub dim: Color,
    pub comment: Color,

    // Interaction and state.
    pub accent: Color,
    pub structure: Color,
    pub live: Color,
    pub cool: Color,
    pub warning: Color,
}

impl Theme {
    /// Studio Chalk: warm paper, lavender surroundings, coral gestures.
    pub const LIGHT: Self = Self {
        mode: Mode::Light,
        background: Color::rgb(0xFB, 0xF9, 0xF6),
        panel: Color::rgb(0xEF, 0xEB, 0xF0),
        popup: Color::rgb(0xFD, 0xFB, 0xF8),
        alt: Color::rgb(0xF3, 0xEC, 0xEE),
        chrome: Color::rgb(0xF5, 0xF1, 0xF3),
        selection: Color::rgb(0xEE, 0xDB, 0xD9),
        code: Color::rgb(0xEE, 0xE8, 0xEF),
        math: Color::rgb(0xF0, 0xEA, 0xF2),
        border: Color::rgb(0xDE, 0xD5, 0xE0),
        badge_ink: Color::rgb(0xA6, 0x55, 0x32),
        badge_blue: Color::rgb(0x4E, 0x6D, 0x9F),
        badge_green: Color::rgb(0x35, 0x76, 0x6A),
        badge_purple: Color::rgb(0x84, 0x5A, 0x9E),
        highlight: Color::rgb(0xDB, 0xAA, 0x52),
        math_number: Color::rgb(0x4A, 0x5C, 0x91),
        math_operator: Color::rgb(0x7D, 0x6E, 0x85),
        math_hues: [
            Color::rgb(0xF5, 0xDC, 0xDD),
            Color::rgb(0xF8, 0xDF, 0xCF),
            Color::rgb(0xF3, 0xE6, 0xC6),
            Color::rgb(0xE4, 0xEB, 0xC9),
            Color::rgb(0xD3, 0xEA, 0xD8),
            Color::rgb(0xCC, 0xE8, 0xE5),
            Color::rgb(0xD5, 0xE4, 0xF1),
            Color::rgb(0xDC, 0xDC, 0xF0),
            Color::rgb(0xE7, 0xD9, 0xF0),
            Color::rgb(0xF4, 0xD8, 0xE7),
        ],
        faint: Color::rgb(0x81, 0x71, 0x84),
        non_text: Color::rgb(0xC1, 0xAF, 0xBF),
        ink: Color::rgb(0x35, 0x2B, 0x3B),
        dim: Color::rgb(0x6D, 0x5C, 0x72),
        comment: Color::rgb(0x82, 0x71, 0x87),
        accent: Color::rgb(0xB8, 0x4F, 0x48),
        structure: Color::rgb(0x92, 0x64, 0x88),
        live: Color::rgb(0x44, 0x81, 0x6C),
        cool: Color::rgb(0x85, 0x61, 0x8D),
        warning: Color::rgb(0xA7, 0x71, 0x31),
    };

    /// Studio Mulberry: plum-black surfaces and apricot light.
    pub const DARK: Self = Self {
        mode: Mode::Dark,
        background: Color::rgb(0x27, 0x21, 0x2C),
        panel: Color::rgb(0x1C, 0x18, 0x21),
        popup: Color::rgb(0x35, 0x2C, 0x3C),
        alt: Color::rgb(0x32, 0x29, 0x37),
        chrome: Color::rgb(0x23, 0x1D, 0x2A),
        selection: Color::rgb(0x51, 0x37, 0x40),
        code: Color::rgb(0x33, 0x2A, 0x3A),
        math: Color::rgb(0x30, 0x29, 0x39),
        border: Color::rgb(0x3D, 0x31, 0x44),
        badge_ink: Color::rgb(0xEA, 0xB8, 0x82),
        badge_blue: Color::rgb(0xAB, 0xC0, 0xDF),
        badge_green: Color::rgb(0x97, 0xC8, 0xB4),
        badge_purple: Color::rgb(0xCF, 0xAB, 0xD9),
        highlight: Color::rgb(0xD5, 0xAB, 0x63),
        math_number: Color::rgb(0xA6, 0xB4, 0xE4),
        math_operator: Color::rgb(0xAC, 0x98, 0xB3),
        math_hues: [
            Color::rgb(0x52, 0x31, 0x38),
            Color::rgb(0x57, 0x3A, 0x2F),
            Color::rgb(0x50, 0x44, 0x2C),
            Color::rgb(0x3F, 0x49, 0x30),
            Color::rgb(0x2C, 0x4B, 0x3C),
            Color::rgb(0x28, 0x49, 0x4B),
            Color::rgb(0x32, 0x44, 0x58),
            Color::rgb(0x3B, 0x3A, 0x5C),
            Color::rgb(0x48, 0x35, 0x56),
            Color::rgb(0x52, 0x30, 0x49),
        ],
        faint: Color::rgb(0xA1, 0x8B, 0xA5),
        non_text: Color::rgb(0x66, 0x51, 0x6E),
        ink: Color::rgb(0xF0, 0xE8, 0xEA),
        dim: Color::rgb(0xC3, 0xB0, 0xC7),
        comment: Color::rgb(0xA4, 0x8F, 0xA9),
        accent: Color::rgb(0xF0, 0xA0, 0x88),
        structure: Color::rgb(0xD5, 0xAB, 0xC6),
        live: Color::rgb(0x98, 0xC7, 0xAE),
        cool: Color::rgb(0xD2, 0xB0, 0xDC),
        warning: Color::rgb(0xE7, 0xBD, 0x7B),
    };

    /// The built-in palette for `mode`. The pair the switch flips between,
    /// and the one place that mapping is written down.
    pub const fn of(mode: Mode) -> Self {
        match mode {
            Mode::Light => Self::LIGHT,
            Mode::Dark => Self::DARK,
        }
    }

    /// The wash behind a symbol carrying `hue`.
    pub fn math_fill(&self, hue: MathHue) -> Color {
        self.math_hues[hue as usize].clone()
    }

    /// The border around a symbol carrying `hue`: its own fill, shaded.
    pub fn math_edge(&self, hue: MathHue) -> Color {
        let index = usize::from(self.mode == Mode::Dark);
        self.shade(self.math_fill(hue), EDGE_STEP[index], EDGE_CHROMA[index])
    }

    /// The same colour, pushed `step` away from *this palette's* page and
    /// `chroma` times as saturated: darker on a light appearance, lighter
    /// on a dark one.
    ///
    /// Done in HSL rather than by mixing toward the ink, because mixing
    /// drags every hue toward the ink's own — on the mulberry palette that
    /// turns ten distinguishable borders into ten shades of plum-grey.
    /// Rotating value and saturation leaves the hue exactly where it was,
    /// which is the whole promise: a border is a shade of its fill, not a
    /// second colour.
    pub fn shade(&self, color: Color, step: f32, chroma: f32) -> Color {
        let Color::Solid([r, g, b, a]) = color else {
            return color;
        };
        let (hue, saturation, lightness) = to_hsl(r, g, b);
        let lightness = match self.mode {
            Mode::Light => lightness * (1.0 - step),
            Mode::Dark => lightness + (1.0 - lightness) * step,
        };
        let (r, g, b) = from_hsl(hue, (saturation * chroma).min(1.0), lightness);
        Color::rgba(r, g, b, a)
    }

    /// The ink a badge of `color` is drawn in — label and hairline box both.
    pub fn badge(&self, color: BadgeColor) -> Color {
        match color {
            BadgeColor::Orange => self.badge_ink.clone(),
            BadgeColor::Blue => self.badge_blue.clone(),
            BadgeColor::Green => self.badge_green.clone(),
            BadgeColor::Purple => self.badge_purple.clone(),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::LIGHT
    }
}

/// Type size of a badge's label. Small enough that the chip is shorter
/// than the line it sits on, so it never opens the leading up.
///
/// Sizes are constants, not theme entries: a palette may not move type.
pub const BADGE_SIZE: f32 = 9.0;
/// Space between a badge's label and its box, each side. Part of the run's
/// advance (see `document::layout::advance`), not just of the drawing —
/// a chip that flows as if it were only its label overlaps whatever comes
/// next.
pub const BADGE_PAD: f32 = 4.0;
/// Height of a badge's box. Fixed rather than derived from the line, so a
/// chip in a heading is the same chip as one in a paragraph.
pub const BADGE_HEIGHT: f32 = 16.0;

/// Holds the theme the application draws with, and answers for it.
///
/// Two things make it a server rather than a variable:
///
/// 1. **It is asked, never copied.** Drawing code reads its colour from here
///    at the moment it draws, so a swap reaches the whole application at
///    once and no widget can be left holding the old palette.
/// 2. **It reports its own changes.** A swap bumps [`revision`] and arms a
///    change flag; the shell takes that flag each frame and redraws every
///    region — see [`take_change`]. A theme change is the one event that
///    dirties everything at once, so nothing else has to notice it.
///
/// [`revision`]: ThemeServer::revision
/// [`take_change`]: ThemeServer::take_change
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeServer {
    theme: Theme,
    revision: u64,
    changed: bool,
}

impl ThemeServer {
    pub const fn new(theme: Theme) -> Self {
        Self {
            theme,
            revision: 0,
            changed: false,
        }
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// Bumped on every real change, so a holder of retained pixels can
    /// compare it against the revision those pixels were drawn at.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Swaps the palette, and says whether that changed anything. Setting
    /// the theme that is already current is not a change and must not cost
    /// a frame — the same rule [`crate::ui::Dirty::write`] keeps.
    pub fn set(&mut self, theme: Theme) -> bool {
        if self.theme == theme {
            return false;
        }
        self.theme = theme;
        self.revision = self.revision.wrapping_add(1);
        self.changed = true;
        true
    }

    /// Takes the pending redraw request, if there is one. The shell calls
    /// this once a frame; `true` means every region has to be redrawn,
    /// because every one of them may be drawn in different colours now.
    pub fn take_change(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    /// The ink a badge of `color` is drawn in.
    pub fn badge(&self, color: BadgeColor) -> Color {
        self.theme.badge(color)
    }
}

impl Default for ThemeServer {
    fn default() -> Self {
        Self::new(Theme::LIGHT)
    }
}

/// The application's server.
///
/// A palette is read from nearly every draw call in the program; threading
/// one down through the layout, the components, and the prose builders would
/// put a `&Theme` in a few hundred signatures to say one thing that is true
/// everywhere. It lives here instead, and is written only by [`set`].
static SERVER: RwLock<ThemeServer> = RwLock::new(ThemeServer::new(Theme::LIGHT));

/// Reads the live server. A poisoned lock still holds a perfectly good
/// palette, so it is taken rather than panicked on: a stale colour for one
/// frame beats no window at all.
pub fn server() -> RwLockReadGuard<'static, ThemeServer> {
    SERVER.read().unwrap_or_else(PoisonError::into_inner)
}

/// A copy of the current palette, for a caller that wants to compare or
/// derive one rather than read a single colour out of it.
pub fn current() -> Theme {
    server().theme().clone()
}

/// Switches the application to `theme`, arming a full redraw if that is a
/// change. This is the only way the palette moves.
pub fn set(theme: Theme) -> bool {
    SERVER
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .set(theme)
}

pub fn revision() -> u64 {
    server().revision()
}

/// Whether the application is currently on a light or a dark palette.
pub fn mode() -> Mode {
    server().theme().mode
}

/// The palette the switch would move to from here.
pub fn counterpart() -> Theme {
    Theme::of(mode().flipped())
}

/// Takes the pending redraw request — see [`ThemeServer::take_change`].
pub fn take_change() -> bool {
    SERVER
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .take_change()
}

/// Defines each colour twice: as a method on the server, and as a free
/// function reading the application's server. The free functions are what
/// drawing code calls (`theme::ink()`), so a call site names a role and
/// nothing else — not a palette, not a theme, and never a hex value.
macro_rules! palette {
    ($($name:ident => $field:ident),* $(,)?) => {
        impl ThemeServer {
            $(
                pub fn $name(&self) -> Color {
                    self.theme.$field.clone()
                }
            )*
        }

        $(
            pub fn $name() -> Color {
                server().theme().$field.clone()
            }
        )*
    };
}

palette! {
    background => background,
    panel => panel,
    popup => popup,
    alt => alt,
    chrome => chrome,
    selection => selection,
    code => code,
    // The surface behind display math needs a name of its own: `math` is
    // already the notation font, three functions down.
    math_surface => math,
    border => border,
    highlight => highlight,
    math_number => math_number,
    math_operator => math_operator,
    faint => faint,
    non_text => non_text,
    ink => ink,
    dim => dim,
    comment => comment,
    accent => accent,
    structure => structure,
    live => live,
    cool => cool,
    warning => warning,
}

/// The ink a badge of `color` is drawn in — label and hairline box both.
pub fn badge_ink(color: BadgeColor) -> Color {
    server().badge(color)
}

/// Faux-bold outline expansion, as a fraction of font size.
const FAUX_BOLD_WEIGHT_RATIO: f32 = 0.018;

/// Inter is embedded in the app and reused by both screen and PDF shaping.
pub fn sans() -> Font {
    Font::Bytes(include_bytes!("../resources/fonts/InterVariable.ttf"))
}

/// JetBrains Mono is embedded so code and monospace labels use the same
/// distinct letterforms on screen, in exports, and on every platform.
pub fn mono() -> Font {
    Font::Bytes(include_bytes!(
        "../resources/fonts/JetBrainsMono-Regular.ttf"
    ))
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

/// Surrounding text stays readable while the active line keeps full ink.
pub fn focus_opacity(amount: f32) -> f32 {
    // Alpha blends in linear light. Dark ink needs more coverage against
    // chalk to look as readable as pale ink against mulberry.
    let surrounding = match mode() {
        Mode::Light => 0.58,
        Mode::Dark => 0.28,
    };
    1.0 - (1.0 - surrounding) * amount.clamp(0.0, 1.0)
}

/// A font plus everything `draw_text` needs to reproduce one look.
#[derive(Clone, Debug)]
pub struct TextStyle {
    pub font: Font,
    pub size: f32,
    pub color: Color,
    /// Faux-bold outline expansion, in logical pixels.
    pub weight: f32,
    /// Faux condense/expand ratio: a horizontal scale applied to each
    /// glyph's outline at rasterization. `< 1.0` condenses, `> 1.0` expands.
    pub width: f32,
    /// Extra letter-spacing, in EM.
    pub tracking: f32,
    /// Faux-italic shear factor; `0.0` is upright.
    pub slant: f32,
}

impl TextStyle {
    pub fn sans(size: f32, color: Color) -> Self {
        Self {
            font: sans(),
            size,
            color,
            weight: 0.0,
            width: 1.0,
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
            width: 1.0,
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
            width: 1.0,
            tracking: 0.0,
            slant: 0.0,
        }
    }

    /// A light optical expansion, shared by screen and PDF glyph outlines.
    pub fn bold(mut self) -> Self {
        self.weight = self.size * FAUX_BOLD_WEIGHT_RATIO;
        self
    }

    /// A 12° italic shear, shared by screen and PDF glyph outlines.
    pub fn italic(mut self) -> Self {
        self.slant = 0.21;
        self
    }

    /// Faux condense/expand: a horizontal scale on each glyph's outline.
    pub fn condensed(mut self, ratio: f32) -> Self {
        self.width = ratio;
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
            width: self.width,
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

/// Shared syntax and DIY ink palette, following the current appearance.
pub fn code_ink(ink: crate::document::code::Ink) -> Color {
    use crate::document::code::{Ink, capture};
    match ink {
        Ink::Manual(hue) => math_edge(hue),
        Ink::Syntax(index) => match capture(index) {
            "comment" => comment(),
            "string" => math_edge(MathHue::Green),
            "number" | "constant" => math_edge(MathHue::Amber),
            "keyword" => math_edge(MathHue::Violet),
            "type" | "constructor" => math_edge(MathHue::Teal),
            "function" => math_edge(MathHue::Sky),
            "property" | "attribute" | "tag" => math_edge(MathHue::Rose),
            "operator" | "punctuation" => dim(),
            _ => self::ink(),
        },
    }
}

/// The same colour at a different opacity. Non-solid colours are returned
/// unchanged — there is no single alpha to set on a gradient.
pub fn fade(color: Color, alpha: f32) -> Color {
    match color {
        Color::Solid([r, g, b, _]) => Color::rgba(r, g, b, (alpha.clamp(0.0, 1.0) * 255.0) as u8),
        other => other,
    }
}

/// Scales a color's OWN alpha by `weight`, keeping its hue. Where [`fade`]
/// replaces the alpha outright — the right tool when the caller knows the
/// final opacity — this preserves how translucent the color already is,
/// for weights applied ON TOP of an authored value: a shadow ink authored
/// at 6% must still be 6% at rest, not 100% because its reveal weight is.
pub fn scale_alpha(color: Color, weight: f32) -> Color {
    match color {
        Color::Solid([r, g, b, a]) => Color::rgba(
            r,
            g,
            b,
            ((a as f32 / 255.0) * weight.clamp(0.0, 1.0) * 255.0) as u8,
        ),
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

/// How far [`math_edge`] pushes a fill away from the page. Higher in the
/// dark appearance because a border there is climbing out of a deep tint
/// rather than down out of a pale one, and the same step reads as less.
const EDGE_STEP: [f32; 2] = [0.50, 0.42];
/// How far [`math_edge`] lifts a fill's saturation. A one-pixel line has
/// almost no area to carry a hue with, so it is given a little more chroma
/// than the wash it edges — but only a little. The pale hues start out
/// light enough that a bigger lift sends amber and olive fluorescent, and a
/// highlighter pen is not what a page of mathematics wants.
const EDGE_CHROMA: [f32; 2] = [1.15, 1.25];

/// The wash behind a symbol carrying `hue`.
pub fn math_fill(hue: MathHue) -> Color {
    server().theme().math_fill(hue)
}

/// The border around a symbol carrying `hue`: its own fill, shaded. Never a
/// colour of its own — see [`Theme::shade`].
pub fn math_edge(hue: MathHue) -> Color {
    server().theme().math_edge(hue)
}

/// sRGB to HSL, hue in turns. Straight from the definition; the only care
/// needed is a grey, whose hue is undefined and is reported as zero.
fn to_hsl(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let lightness = (max + min) / 2.0;
    let chroma = max - min;
    if chroma <= f32::EPSILON {
        return (0.0, 0.0, lightness);
    }
    let saturation = chroma / (1.0 - (2.0 * lightness - 1.0).abs());
    let hue = if max == r {
        ((g - b) / chroma).rem_euclid(6.0)
    } else if max == g {
        (b - r) / chroma + 2.0
    } else {
        (r - g) / chroma + 4.0
    };
    (hue / 6.0, saturation, lightness)
}

/// The inverse of [`to_hsl`].
fn from_hsl(hue: f32, saturation: f32, lightness: f32) -> (u8, u8, u8) {
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let sector = hue.rem_euclid(1.0) * 6.0;
    let second = chroma * (1.0 - (sector.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match sector as u32 {
        0 => (chroma, second, 0.0),
        1 => (second, chroma, 0.0),
        2 => (0.0, chroma, second),
        3 => (0.0, second, chroma),
        4 => (second, 0.0, chroma),
        _ => (chroma, 0.0, second),
    };
    let base = lightness - chroma / 2.0;
    let channel = |v: f32| ((v + base).clamp(0.0, 1.0) * 255.0).round() as u8;
    (channel(r), channel(g), channel(b))
}

pub fn rule(layer: &Layer, at: (f32, f32), length: f32, thickness: f32, color: Color) {
    layer.draw_rectangle(at, (length, thickness), color, Rounding::NONE);
}

pub fn vertical_rule(layer: &Layer, at: (f32, f32), length: f32, thickness: f32, color: Color) {
    layer.draw_rectangle(at, (thickness, length), color, Rounding::NONE);
}

/// Draws a filled glyph from the Material Symbols set — these are filled
/// shapes authored in a `0 -960 960 960` viewbox, not the stroked feather
/// glyphs [`icon`] draws, so they need their own helper rather than a pen.
/// `at` is the top-left corner of the `size`-square the glyph fills.
pub fn material(layer: &Layer, d: &str, at: (f32, f32), size: f32, color: Color) {
    layer
        .draw_svg_icon(
            d,
            (0.0, -960.0, 960.0, 960.0),
            (size, size),
            at,
            crate::renderer::PathPaint::fill(color),
        )
        .expect("material icon paths are constants — a parse failure is a typo, not input");
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

/// The ink a floating surface's shadow is drawn in — one value for every
/// popup, tuned per palette. Deliberately translucent at full weight: the
/// blur's falloff ring is half-transparent by nature, and an opaque core
/// makes the page's own text ghost through that ring darkened — a grid of
/// noise around every popup. A soft shadow reads as depth; a hard one
/// reads as a smudge.
pub fn shadow_ink() -> Color {
    match mode() {
        Mode::Light => Color::rgba(0x35, 0x23, 0x3F, 0x30),
        Mode::Dark => Color::rgba(0x00, 0x00, 0x00, 0x80),
    }
}

/// An elevated floating card's surface: [`Theme::popup`]'s hue lifted into
/// a vertical gradient — a touch lighter above the content, settling back
/// to the flat colour below it. Reads as one raised object with light on it
/// rather than a tinted slab. `e` is an entrance weight; the stops' shared
/// alpha carries it, so a gradient fades without per-pixel work.
pub fn elevated_popup(e: f32) -> Color {
    let base = popup();
    let lift = if mode() == Mode::Light { 3 } else { 7 };
    let Color::Solid([r, g, b, _]) = base.clone() else {
        return fade(base, e);
    };
    let a = (e.clamp(0.0, 1.0) * 255.0) as u8;
    // Near-white surfaces clamp their highlight instead of wrapping a channel.
    let up = |c: u8| c.saturating_add(lift);
    Color::gradient(
        [up(r), up(g), up(b), a],
        [r, g, b, a],
        GradientDirection::Vertical,
    )
}

/// Draw `rect` as a rounded-rectangle outline by stroking its border path —
/// straight edges joined by real quarter arcs, so the stroke's corners
/// match a fill drawn at the same radius. ([`outline`]'s four axis-aligned
/// rules cannot do this: they square off any radius they are drawn
/// around.) One path, one pen; parse failures are impossible because the
/// geometry is generated here — and pinned by a unit test anyway.
pub fn rounded_outline(layer: &Layer, rect: Rect, radius: f32, width: f32, color: Color) {
    let d = rounded_rect_path(rect, radius);
    let mut pen = Stroke::new(color, width);
    pen.join = LineJoin::Round;
    layer
        .draw_path(&d, (0.0, 0.0), PathPaint::Stroke(pen))
        .expect("rounded_outline generates its own path data");
}

/// The SVG path data for `rect`'s rounded border, clockwise from the top
/// edge — the exact shape [`rounded_outline`] strokes. The radius is
/// clamped to half the shorter side, so degenerate rects come out as
/// stadia rather than nonsense arcs.
///
/// Public because the document painters stroke the same shape through a
/// [`Canvas`](crate::canvas::Canvas) rather than a [`Layer`], and two
/// generators for one border would be two borders.
pub fn rounded_rect_path(rect: Rect, radius: f32) -> String {
    let r = radius.max(0.0).min(rect.width / 2.0).min(rect.height / 2.0);
    let (x, y) = rect.position();
    let right = rect.right();
    let bottom = rect.bottom();
    // Arc flags: rx ry rotation large-arc sweep — quarter circles, the
    // short way round, clockwise (sweep 1 in SVG's y-down space).
    format!(
        "M{mx} {y} H{tx} A{r} {r} 0 0 1 {right} {ty} V{by} A{r} {r} 0 0 1 {tx} {bottom} H{mx} A{r} {r} 0 0 1 {x} {by} V{ty} A{r} {r} 0 0 1 {mx} {y} Z",
        mx = x + r,
        tx = right - r,
        ty = y + r,
        by = bottom - r,
    )
}

/// The SVG path data for a table's border: the enabled edges of the same
/// rounded rectangle [`rounded_rect_path`] traces, so a table whose reader
/// switched an edge off still follows the shape of the rounded panel behind it
/// instead of collapsing into a square. A corner curves only where the two
/// edges that meet there are both drawn — a lone edge ends on the flat part of
/// the rectangle rather than hooking into empty space.
pub fn table_border_path(rect: Rect, radius: f32, lines: TableLines) -> String {
    let r = radius.max(0.0).min(rect.width / 2.0).min(rect.height / 2.0);
    let (x, y) = rect.position();
    let right_edge = rect.right();
    let bottom_edge = rect.bottom();
    let (mx, tx) = (x + r, right_edge - r);
    let (ty, by) = (y + r, bottom_edge - r);
    let arc = |from: (f32, f32), to: (f32, f32)| {
        format!("M{} {} A{r} {r} 0 0 1 {} {} ", from.0, from.1, to.0, to.1)
    };
    let mut d = String::new();
    if lines.top {
        d.push_str(&format!("M{mx} {y} H{tx} "));
    }
    if lines.bottom {
        d.push_str(&format!("M{tx} {bottom_edge} H{mx} "));
    }
    if lines.left {
        d.push_str(&format!("M{x} {by} V{ty} "));
    }
    if lines.right {
        d.push_str(&format!("M{right_edge} {ty} V{by} "));
    }
    if lines.top && lines.left {
        d.push_str(&arc((x, ty), (mx, y)));
    }
    if lines.top && lines.right {
        d.push_str(&arc((tx, y), (right_edge, ty)));
    }
    if lines.bottom && lines.right {
        d.push_str(&arc((right_edge, by), (tx, bottom_edge)));
    }
    if lines.bottom && lines.left {
        d.push_str(&arc((mx, bottom_edge), (x, by)));
    }
    d.trim_end().to_string()
}

/// Stroke a table's border — [`table_border_path`] through a layer, the way
/// [`rounded_outline`] strokes a whole rounded rectangle.
pub fn table_border(
    layer: &Layer,
    rect: Rect,
    radius: f32,
    width: f32,
    color: Color,
    lines: TableLines,
) {
    let d = table_border_path(rect, radius, lines);
    if d.is_empty() {
        return;
    }
    let mut pen = Stroke::new(color, width);
    pen.join = LineJoin::Round;
    layer
        .draw_path(&d, (0.0, 0.0), PathPaint::Stroke(pen))
        .expect("table_border_path generates its own path data");
}

/// The hover surface every interactive row and button shares: one tint, one
/// opacity curve, so nothing in the interface highlights differently from
/// anything else. A weight of zero draws nothing at all.
pub fn hover_fill(layer: &Layer, rect: Rect, weight: f32) {
    if weight > 0.0 {
        layer.draw_rectangle(
            rect.position(),
            rect.size(),
            fade(selection(), 0.55 * weight),
            Rounding::uniform(7.0),
        );
    }
}

/// A softly lit surface. The gradient is geometry rendered by Atomos;
/// no texture allocation or continuously running effect is needed.
pub fn surface(layer: &Layer, rect: Rect, base: Color, radius: f32) {
    let top = mix(
        base.clone(),
        ink(),
        if mode() == Mode::Dark { 0.018 } else { 0.006 },
    );
    let (Color::Solid(top), Color::Solid(bottom)) = (top, base) else {
        return;
    };
    layer.draw_rectangle(
        rect.position(),
        rect.size(),
        Color::gradient(top, bottom, GradientDirection::Vertical),
        Rounding::uniform(radius),
    );
}

/// The app's handwritten tau, drawn as a native path at any scale.
pub fn app_mark(layer: &Layer, rect: Rect) {
    let top = [0xDA, 0x78, 0x5C, 0xFF];
    let bottom = [0xAF, 0x50, 0x68, 0xFF];
    layer.draw_rectangle(
        rect.position(),
        rect.size(),
        Color::gradient(top, bottom, GradientDirection::Vertical),
        Rounding::uniform(rect.width * 0.29),
    );
    icon(
        layer,
        icons::TAU_MARK,
        (rect.x + rect.width * 0.2, rect.y + rect.height * 0.2),
        rect.width * 0.6,
        mark_ink(),
        2.3,
    );
}

pub fn mark_ink() -> Color {
    Color::rgb(0xFF, 0xFF, 0xFF)
}

/// A keyboard hint with a restrained physical edge. Returns its width.
pub fn keycap(layer: &Layer, label: &str, at: (f32, f32), alpha: f32) -> f32 {
    let style = TextStyle::sans(11.0, fade(dim(), alpha));
    let width = width(layer, label, &style) + 12.0;
    let rect = Rect::new(at.0, at.1 - 10.0, width, 20.0);
    layer.draw_rectangle(
        rect.position(),
        rect.size(),
        fade(alt(), alpha),
        Rounding::uniform(5.0),
    );
    rounded_outline(layer, rect.inset(0.5), 4.5, 1.0, fade(border(), alpha));
    draw(layer, label, (at.0 + width / 2.0, at.1), &style, CENTER);
    width
}

/// Ellipsize before shaping the final label so it stays inside its control.
pub fn elide(layer: &Layer, text: &str, max_width: f32, style: &TextStyle) -> String {
    if width(layer, text, style) <= max_width {
        return text.into();
    }
    if width(layer, "…", style) > max_width {
        return String::new();
    }
    let mut ends: Vec<_> = text.char_indices().map(|(i, _)| i).collect();
    ends.push(text.len());
    let mut low = 0;
    let mut high = ends.len() - 1;
    while low < high {
        let mid = (low + high).div_ceil(2);
        let label = format!("{}…", &text[..ends[mid]]);
        if width(layer, &label, style) <= max_width {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    format!("{}…", &text[..ends[low]])
}

/// Draws one of the [`icons`] paths, `at` being its top-left corner.
///
/// `stroke` is in the icon's own 24-unit space, exactly as the `d` string's
/// source SVG means it, so the design's `stroke-width="1.8"` is `1.8` here
/// regardless of the size the icon is drawn at.
pub fn icon(layer: &Layer, path: &str, at: (f32, f32), size: f32, color: Color, stroke: f32) {
    icon_turned(layer, path, at, size, 0.0, color, stroke);
}

/// [`icon`], tilted by `rotation` radians clockwise about the icon's own
/// centre. `at` still names the upright icon's top-left corner, so a
/// caller can place the icon once and animate the tilt without also having
/// to re-derive where it sits.
// Seven params; the alternative is a struct for one call site — the same
// call `Layer::draw_svg_icon_rotated` already made.
#[allow(clippy::too_many_arguments)]
pub fn icon_turned(
    layer: &Layer,
    path: &str,
    at: (f32, f32),
    size: f32,
    rotation: f32,
    color: Color,
    stroke: f32,
) {
    let mut pen = Stroke::new(color, stroke);
    pen.cap = LineCap::Round;
    pen.join = LineJoin::Round;
    layer
        .draw_svg_icon_rotated(
            path,
            (0.0, 0.0, 24.0, 24.0),
            (size, size),
            at,
            rotation,
            PathPaint::Stroke(pen),
        )
        .expect("icon paths are constants — a parse failure is a typo, not input");
}

/// Stroke an open polyline in logical pixels — the arcs and seam the theme
/// switch draws, which are not any of the closed shapes `draw_*` names.
pub fn polyline(layer: &Layer, points: &[(f32, f32)], color: Color, thickness: f32) {
    let Some(((first_x, first_y), rest)) = points.split_first() else {
        return;
    };
    let mut d = format!("M{first_x} {first_y}");
    for (x, y) in rest {
        d.push_str(&format!("L{x} {y}"));
    }
    let mut pen = Stroke::new(color, thickness);
    pen.cap = LineCap::Round;
    pen.join = LineJoin::Round;
    layer
        .draw_path(&d, (0.0, 0.0), PathPaint::Stroke(pen))
        .expect("a polyline's own path data is generated here, not parsed from input");
}

/// Feather-style 24×24 stroked icons, taken from the design.
///
/// Circles and rects from the source SVG are written out as arcs and
/// closed subpaths: `draw_svg_icon` takes one path's `d`, not a document.
pub mod icons {
    pub const TAU_MARK: &str = "M4 7C6 5 8 6 11 6H20 M13 6L10.5 16C9.5 20 13.5 20 17 17";
    pub const NOTE_MARK: &str =
        "M7 3H16L20 7V21H7A3 3 0 0 1 4 18V6A3 3 0 0 1 7 3Z M8 3V21 M12 10H16 M12 14H16";
    pub const SIDEBAR: &str =
        "M4 4H20A1 1 0 0 1 21 5V19A1 1 0 0 1 20 20H4A1 1 0 0 1 3 19V5A1 1 0 0 1 4 4Z M9 4V20";
    pub const FOCUS: &str = "M8 3H3V8 M16 3H21V8 M3 16V21H8 M21 16V21H16";
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
    pub const CHEVRON_LEFT: &str = "M15 18L9 12L15 6";
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
    /// The light palette, on the theme switch. Not the usual sun: at the
    /// switch's size a stock icon's two-unit rays land under a pixel and
    /// read as dust around a disc. The disc here is smaller and the rays
    /// are more than twice as long, which is what keeps them rays.
    pub const SUN: &str = "M12 8.4a3.6 3.6 0 1 0 0 7.2a3.6 3.6 0 1 0 0-7.2 \
                           M12 1L12 5.4 M12 18.6L12 23 M1 12L5.4 12 M18.6 12L23 12 \
                           M4.22 4.22L7.33 7.33 M16.67 16.67L19.78 19.78 \
                           M19.78 4.22L16.67 7.33 M4.22 19.78L7.33 16.67";
    /// The dark palette, on the theme switch.
    pub const MOON: &str = "M12 3a6 6 0 0 0 9 9a9 9 0 1 1-9-9z";
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `shade` is what makes a border "a shade of its fill" true rather
    /// than aspirational: the hue must come out where it went in, and the
    /// value must move away from the page — down on chalk, up on mulberry.
    #[test]
    fn a_shade_keeps_its_hue_and_moves_away_from_the_page() {
        for (mode, brighter) in [(Mode::Light, false), (Mode::Dark, true)] {
            let theme = Theme::of(mode);
            for hue in MathHue::ALL {
                let Color::Solid([r, g, b, _]) = theme.math_fill(hue) else {
                    panic!("a hue is a solid colour");
                };
                let Color::Solid([sr, sg, sb, _]) = theme.math_edge(hue) else {
                    panic!("a shade of a solid colour is solid");
                };
                let (fill_hue, _, fill_light) = to_hsl(r, g, b);
                let (edge_hue, _, edge_light) = to_hsl(sr, sg, sb);

                let turned = (fill_hue - edge_hue)
                    .abs()
                    .min(1.0 - (fill_hue - edge_hue).abs());
                assert!(
                    turned < 0.02,
                    "{hue:?} in {mode:?} turned {turned} of a wheel"
                );
                assert_eq!(
                    edge_light > fill_light,
                    brighter,
                    "{hue:?} in {mode:?} moved the wrong way"
                );
            }
        }
    }

    /// Ten hues that a reader has to tell apart cannot be nine.
    #[test]
    fn every_hue_is_its_own_colour_in_both_appearances() {
        for mode in [Mode::Light, Mode::Dark] {
            let theme = Theme::of(mode);
            for (index, hue) in theme.math_hues.iter().enumerate() {
                assert!(
                    !theme.math_hues[..index].contains(hue),
                    "{mode:?} repeats a hue at {index}"
                );
            }
        }
    }

    /// `rounded_outline` strokes generated path data with `.expect` — a
    /// typo in the generator would panic at draw time, so the exact shape
    /// it emits is parsed here through the same `lyon_extra` parser the
    /// renderer uses.
    #[test]
    fn the_rounded_rect_path_parses_as_svg() {
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
        // A normal card, a hairline-thin one, and the degenerate cases the
        // clamp exists for.
        assert!(parses(&rounded_rect_path(
            Rect::new(10.0, 20.0, 200.0, 34.0),
            9.0
        )));
        assert!(parses(&rounded_rect_path(
            Rect::new(0.0, 0.0, 40.0, 2.0),
            9.0
        )));
        assert!(parses(&rounded_rect_path(
            Rect::new(0.0, 0.0, 8.0, 8.0),
            9.0
        )));
        assert!(parses(&rounded_rect_path(
            Rect::new(0.0, 0.0, 30.0, 24.0),
            0.0
        )));
    }
    /// Every combination of table edges, because a path that fails to parse
    /// panics at draw time and the editor strokes this one on every frame a
    /// table is on screen.
    #[test]
    fn the_table_border_path_parses_for_every_edge_combination() {
        fn parses(d: &str) -> bool {
            if d.is_empty() {
                return true;
            }
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
        let card = Rect::new(10.0, 20.0, 200.0, 34.0);
        // The four border edges as bits; the interior flags a table also
        // carries play no part in its outline.
        let edges = |case: u8| TableLines {
            top: case & 1 != 0,
            bottom: case & 2 != 0,
            left: case & 4 != 0,
            right: case & 8 != 0,
            horizontal: true,
            vertical: true,
        };
        for case in 0..16u8 {
            let lines = edges(case);
            for rect in [
                card,
                Rect::new(0.0, 0.0, 40.0, 2.0),
                Rect::new(0.0, 0.0, 8.0, 8.0),
            ] {
                let d = table_border_path(rect, 7.5, lines);
                assert!(parses(&d), "case {case} on {rect:?} produced {d:?}");
            }
        }
        // No edges is nothing to stroke, not an empty path.
        assert!(table_border_path(card, 7.5, edges(0)).is_empty());
        // One edge is a straight run, with no corner to curve.
        let lone = table_border_path(card, 7.5, edges(1));
        assert!(!lone.contains('A'), "a lone edge curves nowhere: {lone}");
        // Two edges that meet curve the corner between them.
        let corner = table_border_path(card, 7.5, edges(5));
        assert_eq!(corner.matches('A').count(), 1, "one corner: {corner}");
        // Opposite edges never meet, so they never curve.
        let rails = table_border_path(card, 7.5, edges(3));
        assert!(!rails.contains('A'), "parallel edges meet nowhere: {rails}");
        // All four is the same shape the background draws: four corners.
        let closed = table_border_path(card, 7.5, edges(15));
        assert_eq!(closed.matches('A').count(), 4, "four corners: {closed}");
    }

    /// The palette a widget reads has to be the one that was set, or a
    /// theme switch is only half a switch.
    #[test]
    fn a_swap_reaches_the_free_functions() {
        let mut server = ThemeServer::new(Theme::LIGHT);
        assert_eq!(server.ink(), Theme::LIGHT.ink);

        let mut other = Theme::LIGHT;
        other.ink = Color::rgb(0x11, 0x22, 0x33);
        assert!(server.set(other.clone()));
        assert_eq!(server.ink(), other.ink);
    }

    #[test]
    fn setting_the_current_theme_is_not_a_change() {
        let mut server = ThemeServer::new(Theme::LIGHT);
        assert!(!server.set(Theme::LIGHT), "same palette");
        assert_eq!(server.revision(), 0);
        assert!(!server.take_change(), "and so owes no redraw");

        let mut other = Theme::LIGHT;
        other.background = Color::rgb(0x00, 0x00, 0x00);
        assert!(server.set(other));
        assert_eq!(server.revision(), 1);
    }

    /// The switch flips between exactly two palettes and reads which one it
    /// is on off the palette itself, so the pair has to close: each mode's
    /// theme must claim that mode, and flipping twice must come home.
    #[test]
    fn the_two_built_in_palettes_are_each_others_counterpart() {
        for mode in [Mode::Light, Mode::Dark] {
            assert_eq!(Theme::of(mode).mode, mode);
            assert_eq!(mode.flipped().flipped(), mode);
            assert_eq!(
                Theme::of(mode.flipped()),
                Theme::of(Theme::of(mode).mode.flipped())
            );
        }
        assert_ne!(Theme::LIGHT, Theme::DARK);
        assert_eq!(Theme::of(Mode::Light), Theme::LIGHT);
        assert_eq!(Theme::of(Mode::Dark), Theme::DARK);
    }

    /// The redraw request is a one-shot: the shell takes it, redraws every
    /// region once, and the next frame is idle again.
    #[test]
    fn a_change_is_taken_exactly_once() {
        let mut server = ThemeServer::new(Theme::LIGHT);
        let mut other = Theme::LIGHT;
        other.panel = Color::rgb(0x0A, 0x0B, 0x0C);
        server.set(other);

        assert!(server.take_change(), "a swap owes a redraw");
        assert!(!server.take_change(), "and only one");
    }

    /// The two alpha helpers are not interchangeable, and a popup's shadow
    /// once shipped tuned-then-discarded because the slab painter reached
    /// for `fade` (replace) where it meant `scale_alpha` (multiply): the
    /// ink's authored byte never survived the reveal weight overwriting it.
    #[test]
    fn fade_replaces_but_scale_alpha_multiplies() {
        // A fresh value per use: Color is not Copy.
        let ink = || Color::rgba(0x3C, 0x38, 0x36, 0x20);
        let weight = 0.5;

        // fade answers "what is this color AT 50% opacity" — the authored
        // byte is irrelevant.
        let Color::Solid([r, g, b, _]) = fade(ink(), weight) else {
            panic!("fade must pass solids through");
        };
        assert_eq!((r, g, b), (0x3C, 0x38, 0x36));
        // (0.5 * 255) as u8 truncates to 127, not 128.
        assert_eq!(fade(ink(), weight), Color::rgba(r, g, b, 127));

        // scale_alpha answers "what is this color at HALF ITS authored
        // opacity" — an authored 0x20 lands on 0x10, not 0x80.
        assert_eq!(
            scale_alpha(ink(), weight),
            Color::rgba(0x3C, 0x38, 0x36, 0x10)
        );
        // The weight clamps and fully-transparent stays fully transparent.
        assert_eq!(scale_alpha(ink(), 1.5), scale_alpha(ink(), 1.0));
        assert_eq!(
            scale_alpha(Color::rgba(1, 2, 3, 0), weight),
            Color::rgba(1, 2, 3, 0)
        );
    }
}
