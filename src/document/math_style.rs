//! What a symbol is *drawn as*: a hue that carries its identity and a shape
//! that carries its role.
//!
//! Split from the palette deliberately. `theme.rs` says what teal is in the
//! appearance that is current; this says that `θ` is teal, which is true in
//! both appearances and in an export. One is a colour, the other is a fact
//! about a symbol, and only the second is worth writing into a config file.
//!
//! Two axes, chosen so they never compete:
//!
//! - **Hue is identity.** `x` and `y` are different colours because they are
//!   different symbols, and a reader learns them the way they learn a
//!   legend. The default comes from the letter itself rather than a hash of
//!   it, so the letters a reader actually reaches for together — `x y z`,
//!   `i j k`, `m n` — are guaranteed to land on different hues instead of
//!   colliding by luck.
//! - **Shape is role.** A variable is a filled wash, a constant an outline,
//!   a function both. That survives the hue varying per symbol, and it
//!   survives greyscale, a projector, and a printed page — none of which a
//!   colour-only distinction does.
//!
//! Either axis can be overridden per symbol; nothing else about a symbol is
//! configurable, and the overrides live in the vault config rather than in
//! the document, so one symbol looks the same in every note.

use std::collections::BTreeMap;
use std::sync::{PoisonError, RwLock, RwLockReadGuard};

use serde::{Deserialize, Serialize};

use super::math::SymbolRole;
use super::math_symbols;

/// The identity hues, in wheel order. Ten is the count a reader can still
/// tell apart at the size a superscript is set in — more would be more
/// names for the same three impressions.
///
/// Wheel order matters: [`automatic`] walks this list with the alphabet, so
/// consecutive letters get adjacent hues, which are the *most* separable
/// pairs there are.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MathHue {
    Rose,
    Coral,
    Amber,
    Olive,
    Green,
    Teal,
    Sky,
    Indigo,
    Violet,
    Magenta,
}

impl MathHue {
    pub const ALL: [Self; 10] = [
        Self::Rose,
        Self::Coral,
        Self::Amber,
        Self::Olive,
        Self::Green,
        Self::Teal,
        Self::Sky,
        Self::Indigo,
        Self::Violet,
        Self::Magenta,
    ];

    /// What the config file and the command table spell it.
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Rose => "rose",
            Self::Coral => "coral",
            Self::Amber => "amber",
            Self::Olive => "olive",
            Self::Green => "green",
            Self::Teal => "teal",
            Self::Sky => "sky",
            Self::Indigo => "indigo",
            Self::Violet => "violet",
            Self::Magenta => "magenta",
        }
    }

    /// What a menu calls it.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Rose => "Rose",
            Self::Coral => "Coral",
            Self::Amber => "Amber",
            Self::Olive => "Olive",
            Self::Green => "Green",
            Self::Teal => "Teal",
            Self::Sky => "Sky",
            Self::Indigo => "Indigo",
            Self::Violet => "Violet",
            Self::Magenta => "Magenta",
        }
    }

    pub fn from_keyword(keyword: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|hue| hue.keyword() == keyword)
    }

    /// The hue `steps` further round the wheel.
    fn nth(steps: usize) -> Self {
        Self::ALL[steps % Self::ALL.len()]
    }
}

/// How a highlight is drawn around its symbol.
///
/// The outline is never a colour of its own — it is [`theme::math_edge`], a
/// shade of the very fill it would sit on, so a symbol drawn with `Both`
/// reads as one object with an edge rather than two overlapping claims.
///
/// [`theme::math_edge`]: crate::theme::math_edge
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HighlightShape {
    /// A filled rounded rectangle, and nothing else.
    Fill,
    /// A rounded rectangle's border, and nothing inside it.
    Outline,
    /// Both: the fill, edged.
    Both,
}

impl HighlightShape {
    pub const ALL: [Self; 3] = [Self::Fill, Self::Outline, Self::Both];

    pub const fn fills(self) -> bool {
        matches!(self, Self::Fill | Self::Both)
    }

    pub const fn outlines(self) -> bool {
        matches!(self, Self::Outline | Self::Both)
    }

    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Fill => "fill",
            Self::Outline => "outline",
            Self::Both => "both",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Fill => "Filled",
            Self::Outline => "Outlined",
            Self::Both => "Filled and outlined",
        }
    }

    pub fn from_keyword(keyword: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|shape| shape.keyword() == keyword)
    }
}

/// The complete look of one highlight: which hue, drawn which way. Two
/// bytes and [`Copy`], because every laid-out symbol carries one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SymbolStyle {
    pub hue: MathHue,
    pub shape: HighlightShape,
}

/// The shape a role is drawn with unless the reader says otherwise.
pub const fn role_shape(role: SymbolRole) -> HighlightShape {
    match role {
        SymbolRole::Variable => HighlightShape::Fill,
        SymbolRole::Constant => HighlightShape::Outline,
        SymbolRole::Function => HighlightShape::Both,
    }
}

/// The look `id` gets with nothing configured.
///
/// A catalog name and the glyph it stands for are the same symbol, so both
/// spellings normalize to the glyph before the hue is picked: `pi` typed by
/// name and a `π` that arrived any other way are one colour, not two.
pub fn automatic(role: SymbolRole, id: &str) -> SymbolStyle {
    SymbolStyle {
        hue: automatic_hue(id),
        shape: role_shape(role),
    }
}

/// The hue an identity falls on before any override.
pub fn automatic_hue(id: &str) -> MathHue {
    let glyph = math_symbols::exact(id)
        .map(|symbol| symbol.glyph)
        .or_else(|| one_char(id));
    match glyph.and_then(alphabet_index) {
        Some(index) => MathHue::nth(index),
        // Anything with no place in an alphabet — `sin`, `exp`, a raw name —
        // takes a stable hash instead. FNV-1a: same answer on every machine
        // and every run, which a `DefaultHasher` does not promise.
        None => MathHue::nth(fnv1a(id) as usize % MathHue::ALL.len()),
    }
}

fn one_char(id: &str) -> Option<char> {
    let mut chars = id.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    }
}

/// Where `glyph` sits in its own alphabet, offset per case and script.
///
/// Ten hues cannot give a hundred symbols one each, so the offsets buy the
/// collisions that matter least. What they do guarantee: neighbours within
/// one alphabet always differ, the two cases of one letter always differ,
/// and the two symbols a reader is likeliest to have on screen together —
/// `x` and an alpha — are not the same colour. Everything past that is what
/// the per-symbol override is for.
fn alphabet_index(glyph: char) -> Option<usize> {
    /// How far each alphabet starts round the wheel from lowercase Latin.
    const LATIN_UPPER: usize = 5;
    const GREEK_UPPER: usize = 2;
    const GREEK_LOWER: usize = 7;

    if glyph.is_ascii_lowercase() {
        return Some(glyph as usize - 'a' as usize);
    }
    if glyph.is_ascii_uppercase() {
        return Some(glyph as usize - 'A' as usize + LATIN_UPPER);
    }
    greek_position(glyph, LOWER_GREEK)
        .map(|index| index + GREEK_LOWER)
        .or_else(|| greek_position(glyph, UPPER_GREEK).map(|index| index + GREEK_UPPER))
}

/// The Greek alphabets, minus final sigma — the same twenty-four letters,
/// in the same order, that `math_symbols` builds its variants from.
const LOWER_GREEK: &str = "αβγδεζηθικλμνξοπρστυφχψω";
const UPPER_GREEK: &str = "ΑΒΓΔΕΖΗΘΙΚΛΜΝΞΟΠΡΣΤΥΦΧΨΩ";

fn greek_position(glyph: char, alphabet: &str) -> Option<usize> {
    alphabet.chars().position(|candidate| candidate == glyph)
}

fn fnv1a(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// One symbol's departure from its automatic look. Both axes are optional,
/// so "coral, but keep the role's shape" is one field rather than a copy of
/// a default that would then stop tracking the role.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Override {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hue: Option<MathHue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<HighlightShape>,
}

impl Override {
    /// Whether this says anything at all. An override that has been reset on
    /// both axes is dropped rather than written as an empty table.
    pub fn is_empty(&self) -> bool {
        self.hue.is_none() && self.shape.is_none()
    }
}

/// Every symbol the reader has taken a position on, keyed by identity —
/// the `id` of a resolved symbol, or the character itself for a bare one.
#[derive(Clone, Default, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Overrides {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub symbols: BTreeMap<String, Override>,
}

impl Overrides {
    pub fn get(&self, id: &str) -> Override {
        self.symbols.get(id).copied().unwrap_or_default()
    }

    /// Applies `edit` to `id`, dropping the entry once it says nothing.
    pub fn set(&mut self, id: &str, edit: Override) {
        if edit.is_empty() {
            self.symbols.remove(id);
        } else {
            self.symbols.insert(id.to_owned(), edit);
        }
    }
}

/// Holds the overrides the application draws with, and reports its own
/// changes — the same contract as [`ThemeServer`], for the same reason: a
/// symbol's look is read from every math draw call, and a change to one
/// symbol invalidates every laid-out expression in the document.
///
/// [`ThemeServer`]: crate::theme::ThemeServer
#[derive(Clone, Default, Debug, PartialEq)]
pub struct StyleServer {
    overrides: Overrides,
    revision: u64,
    changed: bool,
}

impl StyleServer {
    pub const fn new() -> Self {
        Self {
            overrides: Overrides {
                symbols: BTreeMap::new(),
            },
            revision: 0,
            changed: false,
        }
    }

    pub fn overrides(&self) -> &Overrides {
        &self.overrides
    }

    /// Bumped on every real change, so a cache of laid-out boxes can compare
    /// against the revision it was built at.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Replaces the whole table, and says whether that changed anything.
    /// Installing what is already installed is not a change and must not
    /// cost a frame.
    pub fn install(&mut self, overrides: Overrides) -> bool {
        if self.overrides == overrides {
            return false;
        }
        self.overrides = overrides;
        self.revision = self.revision.wrapping_add(1);
        self.changed = true;
        true
    }

    pub fn take_change(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }
}

/// The application's server. Read from every math draw; written only by
/// [`install`].
static SERVER: RwLock<StyleServer> = RwLock::new(StyleServer::new());

/// Reads the live server. A poisoned lock still holds a perfectly good
/// table, so it is taken rather than panicked on.
pub fn server() -> RwLockReadGuard<'static, StyleServer> {
    SERVER.read().unwrap_or_else(PoisonError::into_inner)
}

/// A copy of the current table, for a caller about to edit one entry of it.
pub fn overrides() -> Overrides {
    server().overrides().clone()
}

/// Switches the application to `overrides`, arming a redraw if that is a
/// change. This is the only way symbol styling moves.
pub fn install(overrides: Overrides) -> bool {
    SERVER
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .install(overrides)
}

pub fn revision() -> u64 {
    server().revision()
}

pub fn take_change() -> bool {
    SERVER
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .take_change()
}

/// What `id` in `role` is drawn as right now: its automatic look with the
/// reader's overrides applied, axis by axis.
pub fn resolve(role: SymbolRole, id: &str) -> SymbolStyle {
    let edit = server().overrides().get(id);
    SymbolStyle {
        hue: edit.hue.unwrap_or_else(|| automatic_hue(id)),
        shape: edit.shape.unwrap_or_else(|| role_shape(role)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of walking the alphabet instead of hashing: the
    /// letters that appear in one expression are the ones a reader must be
    /// able to tell apart, and they are consecutive.
    #[test]
    fn neighbouring_letters_never_share_a_hue() {
        for window in ["xyz", "ijk", "mn", "abc", "pqr", "uvw"] {
            let hues: Vec<MathHue> = window
                .chars()
                .map(|c| automatic_hue(&c.to_string()))
                .collect();
            for (index, hue) in hues.iter().enumerate() {
                assert!(
                    !hues[..index].contains(hue),
                    "{window} reuses {hue:?} at {index}"
                );
            }
        }
    }

    /// The pair most likely to be on screen at the same time.
    #[test]
    fn latin_and_greek_do_not_shadow_each_other() {
        assert_ne!(automatic_hue("x"), automatic_hue("\u{03b1}"));
        assert_ne!(automatic_hue("a"), automatic_hue("\u{0391}"));
    }

    #[test]
    fn a_case_change_is_a_different_symbol() {
        assert_ne!(automatic_hue("x"), automatic_hue("X"));
        assert_ne!(automatic_hue("alpha"), automatic_hue("Alpha"));
    }

    /// A catalog name and its glyph are one symbol, so they must not be two
    /// colours — this is what `pi` typed by name and a pasted `π` share.
    #[test]
    fn a_name_and_its_glyph_agree() {
        assert_eq!(automatic_hue("pi"), automatic_hue("π"));
        assert_eq!(automatic_hue("Delta"), automatic_hue("Δ"));
        assert_eq!(automatic_hue("x"), automatic_hue("x"));
    }

    #[test]
    fn an_unalphabetic_name_still_gets_a_stable_hue() {
        assert_eq!(automatic_hue("sin"), automatic_hue("sin"));
        assert_ne!(automatic_hue("sin"), automatic_hue("cos"));
    }

    #[test]
    fn role_drives_shape_and_identity_drives_hue() {
        let x = automatic(SymbolRole::Variable, "x");
        let y = automatic(SymbolRole::Variable, "y");
        let x_constant = automatic(SymbolRole::Constant, "x");

        assert_ne!(x.hue, y.hue, "two symbols, two hues");
        assert_eq!(x.shape, y.shape, "one role, one shape");
        assert_eq!(x.hue, x_constant.hue, "the role must not move the hue");
        assert_ne!(x.shape, x_constant.shape);
    }

    /// The config file spells these; a rename that only changed one of the
    /// two spellings would silently drop every saved override.
    #[test]
    fn keywords_match_what_the_config_file_writes() {
        for hue in MathHue::ALL {
            let written = toml::to_string(&Override {
                hue: Some(hue),
                shape: None,
            })
            .expect("an override serializes");
            assert_eq!(written.trim(), format!("hue = \"{}\"", hue.keyword()));
            assert_eq!(MathHue::from_keyword(hue.keyword()), Some(hue));
        }
        for shape in HighlightShape::ALL {
            let written = toml::to_string(&Override {
                hue: None,
                shape: Some(shape),
            })
            .expect("an override serializes");
            assert_eq!(written.trim(), format!("shape = \"{}\"", shape.keyword()));
            assert_eq!(HighlightShape::from_keyword(shape.keyword()), Some(shape));
        }
    }

    #[test]
    fn a_table_round_trips_through_toml() {
        let mut overrides = Overrides::default();
        overrides.set(
            "epsilon",
            Override {
                hue: Some(MathHue::Coral),
                shape: Some(HighlightShape::Outline),
            },
        );
        overrides.set(
            "x",
            Override {
                hue: None,
                shape: Some(HighlightShape::Both),
            },
        );

        let text = toml::to_string_pretty(&overrides).expect("a table serializes");
        assert_eq!(toml::from_str::<Overrides>(&text).unwrap(), overrides);
    }

    #[test]
    fn an_override_replaces_one_axis_and_leaves_the_other_automatic() {
        let mut overrides = Overrides::default();
        overrides.set(
            "x",
            Override {
                hue: Some(MathHue::Coral),
                shape: None,
            },
        );

        assert_eq!(overrides.get("x").hue, Some(MathHue::Coral));
        assert_eq!(overrides.get("x").shape, None);
        assert_eq!(overrides.get("y"), Override::default());
    }

    #[test]
    fn clearing_both_axes_drops_the_entry() {
        let mut overrides = Overrides::default();
        overrides.set(
            "x",
            Override {
                hue: Some(MathHue::Teal),
                shape: None,
            },
        );
        overrides.set("x", Override::default());

        assert!(overrides.symbols.is_empty());
    }

    #[test]
    fn a_server_reports_one_change_per_real_edit() {
        let mut server = StyleServer::new();
        assert!(!server.take_change(), "a fresh server owes no redraw");

        let mut overrides = Overrides::default();
        overrides.set(
            "x",
            Override {
                hue: Some(MathHue::Sky),
                shape: None,
            },
        );
        assert!(server.install(overrides.clone()), "an edit is a change");
        assert!(server.take_change());
        assert!(!server.take_change(), "and only one");

        assert!(
            !server.install(overrides),
            "installing the same table is not"
        );
        assert!(!server.take_change());
    }
}
