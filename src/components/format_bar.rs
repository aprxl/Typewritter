//! The word-format bar: a small toolbar that opens beside a word clicked
//! in Normal mode.
//!
//! Unlike the generic [`ContextMenu`](super::ContextMenu), which is a list,
//! this is a horizontal strip of affordances — a text-editor style format
//! bar. The iconic options (Bold, Italic) are letterform buttons; the rest
//! are Material Symbol icons drawn with the very effect they would apply,
//! so "Highlight" is a marker on a highlight wash, "Inline code" a
//! `</>` on a code slab, and "Badge" a badge glyph in its own ink. A close
//! affordance sits at the far edge. The tool feels alive on purpose: the
//! bar fades up from the clicked word, the hovered cell's pill *slides*
//! across as the pointer moves, and the word's active formats get a bright
//! accent ring around their cell.
//!
//! The drop shadow is not drawn into the card's own layer either: shader
//! effects apply per layer, so the shell owns one extra blurred layer (the
//! same trick as the editor's highlight glow), hands it to every snapshot,
//! and [`FormatBar::paint_shadow`] fills it from [`Component::sync`] — one
//! fat faint slab the blur turns into a halo.
//!
//! Same snapshot relationship as the palette and the context menu: the
//! shell owns the live selection, checked states, and keystrokes; this
//! component holds a snapshot and draws it. The one animation it cannot own
//! — the entrance reveal — lives in the shell and arrives through
//! [`Context::reveal`](crate::ui::Context::reveal), so a refreshed snapshot
//! never re-triggers the pop (see that field's comment).

use std::time::Duration;

use crate::animation::{Animation, Easing};
use crate::document::BadgeColor;
use crate::layout::Rect;
use crate::renderer::{Color, Layer, Rounding};
use crate::theme::{self, TextStyle};
use crate::ui::{Component, Context, Dirty};

/// What a bar cell is, and therefore how it is drawn and how wide it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The letter B drawn bold — the icon for bold.
    Bold,
    /// The letter I drawn italic — the icon for italic.
    Italic,
    /// A highlighter marker, drawn on a highlight wash.
    Highlight,
    /// A `</>` glyph, drawn on a code slab.
    InlineCode,
    /// A badge glyph, drawn in the badge ink.
    Badge,
    /// The close affordance at the bar's far edge.
    Dismiss,
}

/// Map a formatting command id (from the shell's `WORD_MENU`) to its cell.
/// A format id the bar does not know opens no cell; the shell keeps the two
/// in step by never asking the bar to draw what it can't.
pub fn from_id(id: &str) -> Option<Kind> {
    match id {
        "context.bold" => Some(Kind::Bold),
        "context.italic" => Some(Kind::Italic),
        "context.highlight" => Some(Kind::Highlight),
        "context.inline_code" => Some(Kind::InlineCode),
        "context.badge" => Some(Kind::Badge),
        _ => None,
    }
}

/// The one row the bar keeps for each affordance — a kind plus whether it
/// is currently active on the clicked word.
pub struct Item {
    pub kind: Kind,
    pub checked: bool,
}

/// The Material Symbols this bar draws, read from `resources/`. Kept here —
/// next to the widget that owns them — rather than in the shared icon set,
/// because they are filled glyphs only this bar uses.
mod glyphs {
    /// The highlighter marker — the icon for Highlight.
    pub const MARKER: &str = "m272-104-38-38-42 42q-19 19-46.5 19.5T100-100q-19-19-19-46t19-46l42-42-38-40 554-554q12-12 29-12t29 12l112 112q12 12 12 29t-12 29L272-104Zm172-396L216-274l58 58 226-228-56-56Z";
    /// The `</>` code icon — the icon for Inline code.
    pub const CODE: &str = "M320-240 80-480l240-240 57 57-184 184 183 183-56 56Zm320 0-57-57 184-184-183-183 56-56 240 240-240 240Z";
    /// The badge tag — the icon for Badge.
    pub const BADGE: &str = "M160-160q-33 0-56.5-23.5T80-240v-480q0-33 23.5-56.5T160-800h440q19 0 36 8.5t28 23.5l216 288-216 288q-11 15-28 23.5t-36 8.5H160Zm0-80h440l180-240-180-240H160v480Zm220-240Z";
    /// The close X — the dismiss affordance.
    pub const CLOSE: &str = "m336-280-56-56 144-144-144-143 56-56 144 144 143-144 56 56-144 143 144 144-56 56-143-144-144 144Z";
}

const CARD_PAD_X: f32 = 6.0;
const CARD_PAD_Y: f32 = 5.0;
const CELL_H: f32 = 24.0;
const ICON_W: f32 = 30.0;
const CHIP_W: f32 = 38.0;
const GAP: f32 = 4.0;
/// The wider channel where a divider rule sits — between the letterforms
/// and the effect chips, and between the chips and the close affordance.
const DIV_GAP: f32 = 10.0;
/// Corner radius of the card and of each cell — a bar reads as modern when
/// its shoulders are soft rather than square.
const RADIUS: f32 = 9.0;
/// Thickness of the bright accent ring around a checked/active cell.
const RING: f32 = 1.6;
/// How far the shadow slab spreads past the resting card on every side,
/// before its blur. Wide enough that the blurred halo fades out before its
/// own edge arrives — a shadow with a visible border reads as a second
/// card behind the first.
pub const SHADOW_SPREAD: f32 = 6.0;
/// The blur radius the shell sets on the bar-shadow layer at creation and
/// never touches again — see `Shell::new`.
pub const SHADOW_BLUR_RADIUS: f32 = 9.0;

/// Which visual rank a cell belongs to, for the divider placement — 0 the
/// letterforms, 1 the effect chips, 2 the dismiss. Moving between ranks
/// takes the wider channel.
fn rank(kind: Kind) -> u8 {
    match kind {
        Kind::Bold | Kind::Italic => 0,
        Kind::Highlight | Kind::InlineCode | Kind::Badge => 1,
        Kind::Dismiss => 2,
    }
}

/// How wide a cell is, by kind. Deliberately a constant rather than a
/// measure: the drawing and the shell's hit-test share these numbers, and
/// two pieces of geometry can disagree only if one of them measures text
/// and the other doesn't.
fn cell_width(kind: Kind) -> f32 {
    match kind {
        Kind::Bold | Kind::Italic | Kind::Dismiss => ICON_W,
        Kind::Highlight | Kind::InlineCode | Kind::Badge => CHIP_W,
    }
}

/// The combined width of every cell plus the gap and divider channels.
fn content_width(items: &[Item]) -> f32 {
    let mut x = 0.0;
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            x += if rank(items[index - 1].kind) != rank(item.kind) {
                DIV_GAP
            } else {
                GAP
            };
        }
        x += cell_width(item.kind);
    }
    x
}

/// The card for `items` with its top-left at `anchor`, flipping up when it
/// would cross the viewport's bottom and left when it would cross the
/// right — the same rule the context and slash menus use.
pub fn card_anchored(viewport: Rect, anchor: (f32, f32), items: &[Item]) -> Rect {
    let width = content_width(items) + CARD_PAD_X * 2.0;
    let height = CARD_PAD_Y * 2.0 + CELL_H;
    let x = if anchor.0 + width > viewport.right() {
        viewport.right() - width
    } else {
        anchor.0
    }
    .clamp(viewport.x, viewport.right() - width);
    let y = if anchor.1 + height > viewport.bottom() {
        anchor.1 - height
    } else {
        anchor.1
    }
    .clamp(viewport.y, viewport.bottom() - height);
    Rect::new(x, y, width, height)
}

/// How long the entrance spring takes to settle, and its curve: a fast
/// ease that overshoots by ~6% and swings back — a materialize with a
/// little life in it, not a linear grow. `CubicBezier`'s y is unclamped,
/// which is exactly what an overshoot needs.
pub const REVEAL_DURATION: Duration = Duration::from_millis(180);
pub const REVEAL_EASING: Easing = Easing::CubicBezier(0.34, 1.32, 0.64, 1.0);

/// The pill's hover chase: nearly the same spring as the entrance, shorter
/// and with a subtler overshoot — fast enough to feel attached to the
/// pointer. A function rather than a constant: `Animation::new` is not
/// `const`, and each caller needs a fresh timer anyway.
fn slide_animation() -> Animation {
    Animation::new(
        Duration::from_millis(110),
        Easing::CubicBezier(0.3, 1.18, 0.5, 1.0),
    )
}

/// The card as it is *revealed*: `card` scaled from 88% up to full size
/// around the anchor point (the clicked word) while `e` climbs, so the bar
/// grows out of the word it serves instead of appearing beside it.
///
/// Shared by the drawing and the shell's shadow layer — one function, so
/// the two can never disagree about where the floating surface sits.
pub fn revealed_card(card: Rect, anchor: (f32, f32), e: f32) -> Rect {
    let e = e.clamp(0.0, 1.0);
    let scale = 0.88 + 0.12 * e;
    let lift = (1.0 - e) * 7.0;
    let cx = anchor.0;
    let cy = anchor.1;
    let x = cx + (card.x - cx) * scale - (1.0 - e) * 4.0;
    let y = cy + (card.y - cy) * scale - lift;
    let width = card.width * scale;
    let height = card.height * scale;
    Rect::new(x, y, width, height)
}

/// The cell each item occupies inside `card` — the drawing and the shell's
/// hit-test are one geometry.
pub fn cell_rects(card: Rect, items: &[Item]) -> Vec<Rect> {
    let mut rects = Vec::new();
    let mut x = card.x + CARD_PAD_X;
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            x += if rank(items[index - 1].kind) != rank(item.kind) {
                DIV_GAP
            } else {
                GAP
            };
        }
        rects.push(Rect::new(
            x,
            card.y + CARD_PAD_Y,
            cell_width(item.kind),
            CELL_H,
        ));
        x += cell_width(item.kind);
    }
    rects
}

/// The item under `point`, if any, sharing [`cell_rects`] with the draw.
pub fn cell_at(card: Rect, items: &[Item], point: (f32, f32)) -> Option<usize> {
    (0..items.len()).find(|index| cell_rects(card, items)[*index].contains(point))
}

/// A rounded rect that slides between its `from` and `to` targets as its
/// [`Animation`] plays — the hover pill chasing the pointer across the bar.
struct Slide {
    animation: Animation,
    from: Rect,
    to: Rect,
}

impl Slide {
    /// A fresh slide parked on top of `rect`. Snappy on purpose: a hover
    /// chase should feel immediate, not laggy — an ease-out with a whisper
    /// of overshoot lands like a magnet, not like a fade.
    fn park(&mut self, rect: Rect) {
        self.from = rect;
        self.to = rect;
        self.animation = slide_animation();
    }

    /// Point the slide at `rect`, leaving from wherever it currently is so
    /// the pill travels rather than teleports. Returns whether it moved.
    fn slide_to(&mut self, rect: Rect) -> bool {
        if self.to == rect {
            return false;
        }
        self.from = self.rect();
        self.to = rect;
        self.animation.restart();
        true
    }

    fn rect(&self) -> Rect {
        lerp_rect(self.from, self.to, self.animation.weight())
    }

    fn advancing(&self) -> bool {
        self.animation.is_playing()
    }
}

fn lerp_rect(from: Rect, to: Rect, t: f32) -> Rect {
    Rect::new(
        from.x + (to.x - from.x) * t,
        from.y + (to.y - from.y) * t,
        from.width + (to.width - from.width) * t,
        from.height + (to.height - from.height) * t,
    )
}

/// Which cell the hover pill should sit on — `None` meaning "hold where it
/// is", not "go to a default". A hovered cell always wins; the keyboard
/// `selected` only applies once the pointer has left the bar (`over_bar`
/// false). When the pointer is over the bar but in a gap between cells, the
/// pill holds rather than snapping back to the selection — that snap read
/// as a twitch when the pointer crossed the wide divider channels.
fn slide_target(
    hovered: Option<usize>,
    over_bar: bool,
    selected: usize,
    len: usize,
) -> Option<usize> {
    if len == 0 {
        return None;
    }
    hovered
        .map(|h| h.min(len - 1))
        .or_else(|| (!over_bar).then(|| selected.min(len - 1)))
}

pub struct FormatBar {
    items: Vec<Item>,
    selected: usize,
    anchor: (f32, f32),
    open: bool,
    /// The shell-owned blurred layer the drop shadow paints into, if the
    /// bar has been given one. `None` draws no shadow — a bar without its
    /// halo is correct everywhere the shell hasn't wired one.
    shadow: Option<Layer>,
    dirty: Dirty,
    /// Which cell the pointer is over, if any — the pointer drives the
    /// slide; the shell's `selected` is the keyboard fallback.
    hovered: Option<usize>,
    /// Whether the slide has been given its first rect yet.
    started: bool,
    slide: Slide,
    /// The entrance reveal the shell animates; copied out of the context so
    /// `draw` can fade the whole card with it. See the module comment.
    reveal: f32,
}

impl FormatBar {
    pub fn open(items: Vec<Item>, selected: usize, anchor: (f32, f32)) -> Self {
        let selected = selected.min(items.len().saturating_sub(1));
        Self {
            items,
            selected,
            anchor,
            open: true,
            shadow: None,
            dirty: Dirty::new(),
            hovered: None,
            started: false,
            slide: Slide {
                animation: slide_animation(),
                from: Rect::default(),
                to: Rect::default(),
            },
            reveal: 0.0,
        }
    }

    pub fn closed() -> Self {
        Self {
            items: Vec::new(),
            selected: 0,
            anchor: (0.0, 0.0),
            open: false,
            shadow: None,
            dirty: Dirty::new(),
            hovered: None,
            started: false,
            slide: Slide {
                animation: slide_animation(),
                from: Rect::default(),
                to: Rect::default(),
            },
            reveal: 1.0,
        }
    }

    /// Attaches the shell's blurred layer for `self` to paint its drop
    /// shadow into. Same shape as the editor's `with_glow`: the layer is
    /// created once by the shell (its blur is set at creation and never
    /// changes) and handed over here.
    pub fn with_shadow(mut self, shadow: Layer) -> Self {
        self.shadow = Some(shadow);
        self
    }

    /// Paints the bar's shadow onto the blurred layer: one fat, faint
    /// rounded slab spread past the card, from which the layer's blur makes
    /// a soft halo. Runs from [`Component::sync`] — which has this frame's
    /// viewport and runs whether or not `draw` will — clearing first, so a
    /// bar that closes takes its shadow with it without help from anyone.
    ///
    /// This component is recreated on every shell refresh; whatever a dead
    /// snapshot drew stays in the Manual-mode layer until a live one
    /// clears here, which is why the clear is unconditional.
    fn paint_shadow(&mut self, viewport: Rect) {
        let Some(shadow) = self.shadow.clone() else {
            return;
        };
        shadow.clear();
        if !self.open || self.items.is_empty() || self.reveal <= 0.0 {
            return;
        }
        // The entrance weight after the spring curve — the halo swells in
        // step with the card rather than fading in ahead of it. The slab
        // tracks the *resting* card: blur already softens what it lands
        // on, and chasing the overshoot visually doubles it.
        let e = REVEAL_EASING.apply(self.reveal).clamp(0.0, 1.0);
        let card = revealed_card(
            card_anchored(viewport, self.anchor, &self.items),
            self.anchor,
            1.0,
        );
        if card.is_empty() {
            return;
        }
        shadow.draw_rectangle(
            (card.x - SHADOW_SPREAD, card.y - SHADOW_SPREAD),
            (
                card.width + SHADOW_SPREAD * 2.0,
                card.height + SHADOW_SPREAD * 2.0,
            ),
            theme::fade(theme::shadow_ink(), e),
            Rounding::uniform(RADIUS + SHADOW_SPREAD),
        );
    }
}

impl Component for FormatBar {
    fn measure(&mut self, _: &Layer) -> (f32, f32) {
        (0.0, 0.0)
    }

    fn sync(&mut self, context: &Context) {
        // Shadow upkeep precedes every other decision — a closing bar must
        // still reach the blurred layer to take its halo with it.
        if self.reveal != context.reveal {
            self.reveal = context.reveal;
            self.dirty.set();
        }
        self.paint_shadow(context.self_rect);
        if !self.open {
            return;
        }
        let card = card_anchored(context.self_rect, self.anchor, &self.items);
        let rects = cell_rects(card, &self.items);

        let over_bar = context.mouse.in_window && card.contains(context.mouse.position);
        let hovered = if over_bar {
            cell_at(card, &self.items, context.mouse.position)
        } else {
            None
        };
        if hovered != self.hovered {
            self.hovered = hovered;
            self.dirty.set();
        }

        // Where the pill should sit. A hovered cell wins. When the pointer
        // is over the bar but in a gap between cells (the 10px divider
        // channels), the pill holds where it is — snapping back to the
        // keyboard selection across a gap reads as a twitch. The keyboard
        // selection only drives the pill once the pointer leaves the bar.
        let target = slide_target(hovered, over_bar, self.selected, rects.len()).map(|h| rects[h]);
        if let Some(target) = target {
            if !self.started {
                self.slide.park(target);
                self.started = true;
                self.dirty.set();
            } else if self.slide.slide_to(target) {
                self.dirty.set();
            }
        }
        if self.slide.advancing() {
            self.dirty.set();
        }
        self.slide.animation.advance(context.animation_dt);
    }

    fn is_dirty(&self) -> bool {
        self.dirty.get()
    }

    fn clear_dirty(&mut self) {
        self.dirty.clear();
    }

    fn is_animating(&self) -> bool {
        self.slide.advancing()
    }

    fn draw(&mut self, layer: &Layer, rect: Rect) {
        if !self.open || self.items.is_empty() {
            return;
        }

        // The whole card grows out of the word it serves: the reveal weight
        // runs through the spring curve, so the scale overshoots ~6% before
        // it settles, and every alpha in what follows rides the same fade.
        let e = REVEAL_EASING.apply(self.reveal);
        let resting = card_anchored(rect, self.anchor, &self.items);
        let card = revealed_card(resting, self.anchor, e);
        // The floor of the shadow sandwich goes to the shell-owned blurred
        // layer (see `paint_shadow`) — a slab here would be a hard rim.

        // An elevated surface with light on it: popup hue lifted at the top,
        // settling to the flat colour below — see `theme::elevated_popup`.
        layer.draw_rectangle(
            card.position(),
            card.size(),
            theme::elevated_popup(e.clamp(0.0, 1.0)),
            Rounding::uniform(RADIUS),
        );

        // A hairline stroke around the gradient: round joins keep the
        // corners as soft as the fill's radius, which four axis-aligned
        // rules squared off before.
        theme::rounded_outline(
            layer,
            card.inset(0.5),
            RADIUS - 0.5,
            1.0,
            theme::fade(theme::non_text(), e.clamp(0.0, 1.0)),
        );

        // The divider(s) separating the ranks.
        let rects = cell_rects(card, &self.items);
        for index in 1..rects.len() {
            if rank(self.items[index - 1].kind) != rank(self.items[index].kind) {
                let x = (rects[index - 1].right() + rects[index].x) / 2.0;
                theme::vertical_rule(
                    layer,
                    (x, card.y + CARD_PAD_Y + 3.0),
                    CELL_H - 6.0,
                    1.0,
                    theme::fade(theme::non_text(), e),
                );
            }
        }

        // Each effect chip carries its own slab (highlight wash, code tint,
        // badge fill) so it reads as the very form it would apply. A checked
        // cell warms its slab toward the accent before any ring lands on it.
        for (index, item) in self.items.iter().enumerate() {
            if let Some(fill) = chip_fill(item.kind, item.checked, e) {
                layer.draw_rectangle(
                    rects[index].position(),
                    rects[index].size(),
                    fill,
                    Rounding::uniform(RADIUS),
                );
            }
        }

        // The hover pill slides under the active cell: warm ink at low
        // weight, not a grey veil — hovering should feel like heat, and it
        // sits under whatever slab or ring it lands on without hiding the
        // glyph drawn on top of it.
        let pill = self.slide.rect();
        layer.draw_rectangle(
            pill.position(),
            pill.size(),
            theme::fade(
                theme::mix(theme::selection(), theme::accent(), 0.35),
                0.26 * e,
            ),
            Rounding::uniform(RADIUS),
        );

        // The bright accent ring around each checked/active affordance —
        // the one unambiguous "this format is on" cue.
        for (index, item) in self.items.iter().enumerate() {
            if item.checked {
                ring(layer, rects[index], e);
            }
        }

        for (index, item) in self.items.iter().enumerate() {
            paint(layer, rects[index], item.kind, item.checked, e);
        }
    }
}

/// A rounded accent ring of `RING` px stroked just inside `cell`'s edge —
/// a real pen with round joins (see [`theme::rounded_outline`]), not a fill
/// cut back over its centre, so it stays crisp over any slab and there is
/// no chance of the card showing through a seam.
fn ring(layer: &Layer, cell: Rect, e: f32) {
    theme::rounded_outline(
        layer,
        cell.inset(RING / 2.0),
        RADIUS - RING / 2.0,
        RING,
        theme::fade(theme::accent(), e),
    );
}

/// The translucent fill a chip sits on, or `None` for cells that need no
/// backing slab. `e` is the entrance weight — the slab fades in too.
fn chip_fill(kind: Kind, checked: bool, e: f32) -> Option<Color> {
    // A checked cell glows faintly of the accent even when its own effect
    // has a slab, so the active state reads before the ring lands on it.
    let checked_wash = if checked {
        Some(theme::fade(theme::accent(), 0.10 * e))
    } else {
        None
    };
    let slab = match kind {
        Kind::Bold | Kind::Italic | Kind::Dismiss => None,
        Kind::Highlight => Some(theme::fade(theme::highlight(), 0.30 * e)),
        Kind::InlineCode => Some(theme::fade(theme::code(), e)),
        Kind::Badge => Some(theme::fade(theme::badge_ink(BadgeColor::Orange), 0.16 * e)),
    };
    match (slab, checked_wash) {
        (None, None) => None,
        (Some(c), None) | (None, Some(c)) => Some(c),
        (Some(slab), Some(wash)) => Some(theme::mix(slab, wash, 0.35)),
    }
}

/// Draws one cell's content — the B/I letterform, the effect glyph, or the
/// close X.
fn paint(layer: &Layer, cell: Rect, kind: Kind, checked: bool, e: f32) {
    let middle = cell.y + cell.height / 2.0;
    let cx = cell.x + cell.width / 2.0;
    let ink = theme::fade(
        if checked {
            theme::accent()
        } else {
            theme::ink()
        },
        e,
    );
    match kind {
        Kind::Bold => {
            theme::draw(
                layer,
                "B",
                (cx, middle),
                &TextStyle::serif(15.0, ink).bold(),
                theme::CENTER,
            );
        }
        Kind::Italic => {
            theme::draw(
                layer,
                "I",
                (cx, middle),
                &TextStyle::serif(15.0, ink).italic(),
                theme::CENTER,
            );
        }
        Kind::Highlight => {
            theme::material(layer, glyphs::MARKER, (cx - 9.0, middle - 9.0), 18.0, ink);
        }
        Kind::InlineCode => {
            theme::material(layer, glyphs::CODE, (cx - 9.0, middle - 9.0), 18.0, ink);
        }
        Kind::Badge => {
            let color = theme::fade(theme::badge_ink(BadgeColor::Orange), e);
            theme::material(layer, glyphs::BADGE, (cx - 9.0, middle - 9.0), 18.0, color);
        }
        Kind::Dismiss => {
            theme::material(layer, glyphs::CLOSE, (cx - 8.0, middle - 8.0), 16.0, ink);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every icon this widget draws is a constant in this file; a path that
    /// fails to parse would panic at draw time, so they are all checked
    /// here — before any has reached a live menu — through the same
    /// `lyon_extra` parser the renderer uses.
    #[test]
    fn every_builtin_icon_parses_as_svg() {
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
        for (name, d) in [
            ("MARKER", glyphs::MARKER),
            ("CODE", glyphs::CODE),
            ("BADGE", glyphs::BADGE),
            ("CLOSE", glyphs::CLOSE),
        ] {
            assert!(parses(d), "{name} failed to parse: {d}");
        }
    }

    fn word_items() -> Vec<Item> {
        vec![
            Item {
                kind: Kind::Bold,
                checked: false,
            },
            Item {
                kind: Kind::Italic,
                checked: true,
            },
            Item {
                kind: Kind::Highlight,
                checked: false,
            },
            Item {
                kind: Kind::InlineCode,
                checked: false,
            },
            Item {
                kind: Kind::Badge,
                checked: false,
            },
        ]
    }

    #[test]
    fn every_word_menu_id_maps_to_a_cell_kind() {
        for id in [
            "context.bold",
            "context.italic",
            "context.highlight",
            "context.inline_code",
            "context.badge",
        ] {
            assert!(from_id(id).is_some(), "{id} should map");
        }
        assert!(from_id("context.symbol.variable").is_none());
        assert!(from_id("edit.cut").is_none());
    }

    #[test]
    fn a_bar_near_the_bottom_flips_up() {
        let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
        let card = card_anchored(viewport, (100.0, 590.0), &word_items());
        assert!(card.bottom() <= 590.0);
        assert!(card.y >= viewport.y);
        assert!(card.bottom() <= viewport.bottom());
    }

    #[test]
    fn a_bar_near_the_right_edge_shifts_left() {
        let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
        let anchor = (790.0, 100.0);
        let card = card_anchored(viewport, anchor, &word_items());
        assert!(card.x < anchor.0);
        assert!(card.x >= viewport.x);
        assert!(card.right() <= viewport.right());
    }

    #[test]
    fn cells_hit_test_to_the_same_item_the_draw_uses() {
        let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
        let items = word_items();
        let card = card_anchored(viewport, (40.0, 40.0), &items);
        let rects = cell_rects(card, &items);
        assert_eq!(rects.len(), items.len());
        for (index, rect) in rects.iter().enumerate() {
            let center = (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
            assert_eq!(cell_at(card, &items, center), Some(index));
        }
        // The 4px channel between two cells belongs to neither.
        let gap = (rects[0].right() + rects[1].x) / 2.0;
        assert_eq!(cell_at(card, &items, (gap, rects[0].y + 2.0)), None);
    }

    #[test]
    fn the_letterforms_chips_and_dismiss_are_separated() {
        let items = word_items();
        let mut items = items;
        items.push(Item {
            kind: Kind::Dismiss,
            checked: false,
        });
        let rects = cell_rects(Rect::new(0.0, 0.0, 800.0, 600.0), &items);
        // The divider channel between the letterforms and the first chip,
        // and between the last chip and the dismiss, is wider than a gap.
        assert!(rects[2].x - rects[1].right() > GAP);
        assert!(rects[5].x - rects[4].right() > GAP);
    }

    #[test]
    fn the_pill_holds_in_a_gap_instead_of_snapping_to_the_selection() {
        // Hovering a cell always wins.
        assert_eq!(slide_target(Some(3), true, 0, 6), Some(3));
        // Over the bar but in a gap (no hover): hold, don't snap to Bold.
        assert_eq!(slide_target(None, true, 0, 6), None);
        assert_eq!(slide_target(None, true, 2, 6), None);
        // Off the bar: the keyboard selection drives the pill.
        assert_eq!(slide_target(None, false, 2, 6), Some(2));
        assert_eq!(slide_target(None, false, 0, 6), Some(0));
        // An empty bar has no pill at all.
        assert_eq!(slide_target(None, false, 0, 0), None);
        assert_eq!(slide_target(Some(2), true, 0, 0), None);
        // Bound the selection.
        assert_eq!(slide_target(None, false, 99, 6), Some(5));
        assert_eq!(slide_target(Some(99), true, 0, 6), Some(5));
    }

    #[test]
    fn the_slide_travels_to_its_new_target() {
        let mut slide = Slide {
            animation: slide_animation(),
            from: Rect::default(),
            to: Rect::default(),
        };
        let first = Rect::new(0.0, 0.0, 30.0, 24.0);
        slide.park(first);
        assert_eq!(slide.rect(), first);

        // Same target again is not a move.
        assert!(!slide.slide_to(first));

        let second = Rect::new(40.0, 0.0, 38.0, 24.0);
        assert!(slide.slide_to(second));
        assert!(slide.advancing());
        slide.animation.advance(Duration::from_millis(200));
        assert!(!slide.advancing());
        assert_eq!(slide.rect(), second);
    }
}
