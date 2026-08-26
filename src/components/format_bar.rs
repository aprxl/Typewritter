//! The word-format bar: a small toolbar that opens beside a word clicked
//! in Normal mode.
//!
//! Unlike the generic [`ContextMenu`](super::ContextMenu), which is a list,
//! this is a horizontal strip of affordances — a text-editor style format
//! bar. The iconic options (Bold, Italic) are letterform buttons; the rest
//! are drawn with the very effect they would apply, so "Highlight" sits on
//! a highlight wash, "Inline code" on a code slab, and "Badge" in a badge
//! chip. The tool feels alive on purpose: the bar fades up from the clicked
//! word, and the hovered cell's pill *slides* across as the pointer moves.
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
    /// The text on a highlight wash, drawn using the highlight effect.
    Highlight,
    /// The text on a code slab, drawn like an inline code span.
    InlineCode,
    /// The text as a badge chip.
    Badge,
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

const CARD_PAD_X: f32 = 6.0;
const CARD_PAD_Y: f32 = 5.0;
const CELL_H: f32 = 24.0;
const ICON_W: f32 = 30.0;
const GAP: f32 = 4.0;
/// The wider channel between the icon buttons and the effect chips; a thin
/// rule sits in it so the two ranks read as groups rather than one row.
const DIV_GAP: f32 = 10.0;
/// Corner radius of the card and of each cell — a bar reads as modern when
/// its shoulders are soft rather than square.
const RADIUS: f32 = 9.0;
/// Height of the accent bar under a checked affordance.
const CHECK_BAR: f32 = 2.5;

fn is_icon(kind: Kind) -> bool {
    kind == Kind::Bold || kind == Kind::Italic
}

/// How wide a cell is, by kind. Icon cells are squares; a text chip is wide
/// enough for its label in the bar's own fonts. Deliberately a constant
/// rather than a measure: the drawing and the shell's hit-test share these
/// numbers, and two pieces of geometry can disagree only if one of them
/// measures text and the other doesn't.
fn cell_width(kind: Kind) -> f32 {
    match kind {
        Kind::Bold => ICON_W,
        Kind::Italic => ICON_W,
        Kind::Highlight => 84.0,
        Kind::InlineCode => 114.0,
        Kind::Badge => 78.0,
    }
}

/// The combined width of every cell plus the gap and divider channels.
fn content_width(items: &[Item]) -> f32 {
    let mut x = 0.0;
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            x += if is_icon(items[index - 1].kind) && !is_icon(item.kind) {
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

/// The cell each item occupies inside `card` — the drawing and the shell's
/// hit-test are one geometry.
pub fn cell_rects(card: Rect, items: &[Item]) -> Vec<Rect> {
    let mut rects = Vec::new();
    let mut x = card.x + CARD_PAD_X;
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            x += if is_icon(items[index - 1].kind) && !is_icon(item.kind) {
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
    /// A fresh slide parked on top of `rect` — used the frame the bar opens
    /// so the hover pill does not glide in from nowhere.
    fn park(&mut self, rect: Rect) {
        self.from = rect;
        self.to = rect;
        self.animation = Animation::new(Duration::from_millis(160), Easing::EaseInOut);
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

pub struct FormatBar {
    items: Vec<Item>,
    selected: usize,
    anchor: (f32, f32),
    open: bool,
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
            dirty: Dirty::new(),
            hovered: None,
            started: false,
            slide: Slide {
                animation: Animation::new(Duration::from_millis(160), Easing::EaseInOut),
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
            dirty: Dirty::new(),
            hovered: None,
            started: false,
            slide: Slide {
                animation: Animation::new(Duration::from_millis(160), Easing::EaseInOut),
                from: Rect::default(),
                to: Rect::default(),
            },
            reveal: 1.0,
        }
    }
}

impl Component for FormatBar {
    fn measure(&mut self, _: &Layer) -> (f32, f32) {
        (0.0, 0.0)
    }

    fn sync(&mut self, context: &Context) {
        if !self.open {
            return;
        }
        if self.reveal != context.reveal {
            self.reveal = context.reveal;
            self.dirty.set();
        }
        let card = card_anchored(context.self_rect, self.anchor, &self.items);
        let rects = cell_rects(card, &self.items);

        let hovered = if context.mouse.in_window && card.contains(context.mouse.position) {
            cell_at(card, &self.items, context.mouse.position)
        } else {
            None
        };
        if hovered != self.hovered {
            self.hovered = hovered;
            self.dirty.set();
        }

        // The keyboard selection is the fallback; a hovered cell wins. The
        // highlight travels to the focused cell rather than jumping.
        if !rects.is_empty() {
            let focused = hovered.unwrap_or(self.selected).min(rects.len() - 1);
            let target = rects[focused];
            if !self.started {
                self.slide.park(target);
                self.started = true;
                self.dirty.set();
            } else if self.slide.slide_to(target) {
                self.dirty.set();
            }
            if self.slide.advancing() {
                self.dirty.set();
            }
            self.slide.animation.advance(context.animation_dt);
        }
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

        // The whole card fades up from the word it serves and lifts a few
        // pixels into place — a materialize, not a pop.
        let e = self.reveal;
        let card = card_anchored(rect, self.anchor, &self.items);
        let card = Rect::new(card.x, card.y - (1.0 - e) * 6.0, card.width, card.height);

        layer.draw_rectangle(
            card.position(),
            card.size(),
            theme::fade(theme::popup(), e),
            Rounding::uniform(RADIUS),
        );

        // The divider between the icon buttons and the effect chips.
        let rects = cell_rects(card, &self.items);
        if rects.len() > 2 {
            let divider_x = (rects[1].right() + rects[2].x) / 2.0;
            theme::vertical_rule(
                layer,
                (divider_x, card.y + CARD_PAD_Y + 3.0),
                CELL_H - 6.0,
                1.0,
                theme::fade(theme::non_text(), e),
            );
        }

        // Each effect chip carries its own slab (highlight wash, code tint,
        // badge fill) so it reads as the very form it would apply.
        for (index, item) in self.items.iter().enumerate() {
            if let Some(fill) = chip_fill(item.kind, e) {
                layer.draw_rectangle(
                    rects[index].position(),
                    rects[index].size(),
                    fill,
                    Rounding::uniform(RADIUS),
                );
            }
        }

        // The hover pill slides under the active cell, dimming whatever
        // slab it lands on without hiding the label drawn on top of it.
        let pill = self.slide.rect();
        layer.draw_rectangle(
            pill.position(),
            pill.size(),
            theme::fade(theme::selection(), 0.30 * e),
            Rounding::uniform(RADIUS),
        );

        for (index, item) in self.items.iter().enumerate() {
            paint(layer, rects[index], item.kind, item.checked, e);
            if item.checked {
                // The one persistent "on" cue, shared by every affordance:
                // a short accent bar under the cell.
                theme::rule(
                    layer,
                    (
                        rects[index].x + 4.0,
                        rects[index].bottom() - CHECK_BAR - 1.0,
                    ),
                    rects[index].width - 8.0,
                    CHECK_BAR,
                    theme::fade(theme::accent(), e),
                );
            }
        }
    }
}

/// The translucent fill a chip sits on, or `None` for the icon letters which
/// need no backing slab. `e` is the entrance weight — the slab fades in too.
fn chip_fill(kind: Kind, e: f32) -> Option<Color> {
    match kind {
        Kind::Bold => None,
        Kind::Italic => None,
        Kind::Highlight => Some(theme::fade(theme::highlight(), 0.30 * e)),
        Kind::InlineCode => Some(theme::fade(theme::code(), e)),
        Kind::Badge => Some(theme::fade(theme::badge_ink(BadgeColor::Orange), 0.16 * e)),
    }
}

/// Draws one cell's content — the B/I letterform or the effect label.
fn paint(layer: &Layer, cell: Rect, kind: Kind, checked: bool, e: f32) {
    let middle = cell.y + cell.height / 2.0;
    let center = (cell.x + cell.width / 2.0, middle);
    let color = theme::fade(
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
                center,
                &TextStyle::serif(15.0, color).bold(),
                theme::CENTER,
            );
        }
        Kind::Italic => {
            theme::draw(
                layer,
                "I",
                center,
                &TextStyle::serif(15.0, color).italic(),
                theme::CENTER,
            );
        }
        Kind::Highlight => theme::draw(
            layer,
            "Highlight",
            center,
            &TextStyle::serif(12.0, theme::fade(theme::ink(), e)),
            theme::CENTER,
        ),
        Kind::InlineCode => theme::draw(
            layer,
            "Inline code",
            center,
            &TextStyle::mono(12.0, theme::fade(theme::ink(), e)),
            theme::CENTER,
        ),
        Kind::Badge => theme::draw(
            layer,
            "Badge",
            center,
            &TextStyle::serif(11.5, theme::fade(theme::badge_ink(BadgeColor::Orange), e)),
            theme::CENTER,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn the_icon_group_is_separated_from_the_chips() {
        let rects = cell_rects(Rect::new(0.0, 0.0, 800.0, 600.0), &word_items());
        assert_eq!(rects[0].width, ICON_W, "bold is an icon square");
        assert_eq!(rects[1].width, ICON_W, "italic is an icon square");
        // The divider channel between the icons and the first chip is wider
        // than a plain gap — the two ranks read as groups.
        assert!(rects[2].x - rects[1].right() > GAP);
    }

    #[test]
    fn the_slide_travels_to_its_new_target() {
        let mut slide = Slide {
            animation: Animation::new(Duration::from_millis(160), Easing::EaseInOut),
            from: Rect::default(),
            to: Rect::default(),
        };
        let first = Rect::new(0.0, 0.0, 30.0, 24.0);
        slide.park(first);
        assert_eq!(slide.rect(), first);

        // Same target again is not a move.
        assert!(!slide.slide_to(first));

        let second = Rect::new(40.0, 0.0, 84.0, 24.0);
        assert!(slide.slide_to(second));
        assert!(slide.advancing());
        slide.animation.advance(Duration::from_millis(200));
        assert!(!slide.advancing());
        assert_eq!(slide.rect(), second);
    }
}
