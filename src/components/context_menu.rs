//! The right-click context menu: a small command card beside the pointer.
//!
//! Same snapshot relationship to the shell as [`Palette`](super::Palette) and
//! [`SlashMenu`](super::SlashMenu): the shell owns live state and keystrokes,
//! while this component holds a snapshot and draws it. The card speaks the
//! shared popup vocabulary — elevated surface springing from the pointer,
//! blurred shadow on the shell's layer, one sliding pill for pointer and
//! keyboard — and closes as a ghost of its real rows on the shared clock.

use crate::layout::Rect;
use crate::renderer::{Layer, Rounding};
use crate::theme::{self, TextStyle};
use crate::ui::{Component, Context, Dirty};

/// One row: reuse the palette's `Entry` — title, group, and the keybinding
/// hint, which is exactly what an OS menu shows on the right of a row.
pub use super::palette::Entry;

use super::popup::{MENU_SLIDE_EASING, Slide, paint_shadow_slab, revealed_card};

/// Corner radius of a row highlight — nested smaller things round less than
/// the card itself (see `popup::CARD_RADIUS`).
const ROW_RADIUS: f32 = 6.0;

/// Everything a dismissal ghost needs to redraw the menu exactly as it
/// stood: cloneable real state, no placeholders. The shell captures this at
/// close time (`MenuDismiss::Context`).
#[derive(Clone)]
pub struct Snapshot {
    pub entries: Vec<Entry>,
    pub checked: Vec<bool>,
}

impl ContextMenu {
    /// The row the pill sits on — the shell reads this at close time so the
    /// ghost parks its highlight where the user saw it.
    pub fn pill_row(&self) -> Option<usize> {
        (!self.entries.is_empty()).then_some(self.selected)
    }

    /// The ghost-rebuild data for this snapshot.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            entries: self.entries.clone(),
            checked: self.checked.clone(),
        }
    }
}

const CARD_W: f32 = 224.0;
const ROW_HEIGHT: f32 = 30.0;
/// The card's own padding above the first row and below the last.
const PAD_Y: f32 = 6.0;
const TITLE_X: f32 = 34.0;

/// The card for `rows` items, with its top-left corner at `anchor`. It
/// flips up when it would overflow the viewport's bottom and shifts left
/// when it would overflow the right edge, so a menu opened near a corner
/// stays fully on screen — same rule as the slash menu's card.
pub fn card_anchored(viewport: Rect, anchor: (f32, f32), rows: usize) -> Rect {
    let height = PAD_Y * 2.0 + rows as f32 * ROW_HEIGHT;
    let x = if anchor.0 + CARD_W > viewport.right() {
        viewport.right() - CARD_W
    } else {
        anchor.0
    }
    .clamp(viewport.x, viewport.right() - CARD_W);
    let y = if anchor.1 + height > viewport.bottom() {
        anchor.1 - height
    } else {
        anchor.1
    }
    .clamp(viewport.y, viewport.bottom() - height);
    Rect::new(x, y, CARD_W, height)
}

/// The row `point` is over, if any. The component owns this so the
/// hit-test and the drawing cannot drift apart.
pub fn row_at(card: Rect, rows: usize, point: (f32, f32)) -> Option<usize> {
    if !card.contains(point) || point.1 < card.y + PAD_Y {
        return None;
    }
    let index = ((point.1 - card.y - PAD_Y) / ROW_HEIGHT) as usize;
    (index < rows).then_some(index)
}

pub struct ContextMenu {
    entries: Vec<Entry>,
    checked: Vec<bool>,
    /// Indexes `entries`. Also what a hovered row sets, so keyboard and
    /// mouse drive one highlight rather than two.
    selected: usize,
    anchor: (f32, f32),
    open: bool,
    /// True while this snapshot is a dismissal ghost: dead input, reveal
    /// weight falling toward 0 through the entrance curve.
    dismissing: bool,
    /// Set once the pointer has hovered the card; cleared when it leaves.
    /// Only meaningful for the live snapshot's pill persistence.
    hovered_by_pointer: bool,
    /// The blurred layer this card paints its shadow slab into.
    shadow: Option<Layer>,
    reveal: f32,
    slide: Slide,
    started: bool,
    dirty: Dirty,
}

impl ContextMenu {
    pub fn new(
        entries: Vec<Entry>,
        checked: Vec<bool>,
        selected: usize,
        anchor: (f32, f32),
    ) -> Self {
        let selected = selected.min(entries.len().saturating_sub(1));
        Self {
            entries,
            checked,
            selected,
            anchor,
            open: true,
            hovered_by_pointer: false,
            dismissing: false,
            shadow: None,
            reveal: 0.0,
            slide: Slide::new(),
            started: false,
            dirty: Dirty::new(),
        }
    }

    /// A dismissal ghost rebuilt from the state captured at close time:
    /// same rows, same checkmarks, pill parked on the selection. Input is
    /// dead; the reveal weight falls from outside.
    pub fn dismissing(snap: &Snapshot, anchor: (f32, f32), pill_row: usize) -> Self {
        let ghost = Self {
            entries: snap.entries.clone(),
            checked: snap.checked.clone(),
            selected: pill_row.min(snap.entries.len().saturating_sub(1)),
            anchor,
            open: true,
            hovered_by_pointer: false,
            dismissing: true,
            shadow: None,
            reveal: 1.0,
            slide: Slide::new(),
            started: true,
            dirty: Dirty::new(),
        };
        // The pill is NOT parked here: card placement needs the REAL
        // viewport, and `card_anchored(Rect::default(), ..)` panics its own
        // clamp (zero-width viewport ⇒ clamp max −CARD_W). A ghost's first
        // `sync` carries the region rect and parks it.
        ghost
    }

    pub fn closed() -> Self {
        Self {
            entries: Vec::new(),
            checked: Vec::new(),
            selected: 0,
            anchor: (0.0, 0.0),
            open: false,
            hovered_by_pointer: false,
            dismissing: false,
            shadow: None,
            reveal: 1.0,
            slide: Slide::new(),
            started: false,
            dirty: Dirty::new(),
        }
    }
}

impl ContextMenu {
    /// Attaches the shell's blurred shadow layer — every snapshot the shell
    /// builds carries it, ghosts included (see `SlashMenu::with_shadow`).
    pub fn with_shadow(mut self, shadow: Layer) -> Self {
        self.shadow = Some(shadow);
        self
    }

    /// Paints the shadow slab from sync — see `SlashMenu::paint_shadow`.
    fn paint_shadow(&mut self, viewport: Rect) {
        let Some(shadow) = self.shadow.clone() else {
            return;
        };
        if !self.open || self.reveal < 0.0 {
            paint_shadow_slab(&shadow, Rect::default(), -1.0);
            return;
        }
        let e = MENU_SLIDE_EASING.apply(self.reveal).clamp(0.0, 1.0);
        let card = revealed_card(
            card_anchored(viewport, self.anchor, self.entries.len()),
            self.anchor,
            1.0,
        );
        paint_shadow_slab(&shadow, card, e);
    }

    /// The row the pill should ride. Keyboard selection seeds it at build;
    /// hover takes over from there and persists.
    fn pill_target(&self) -> Option<usize> {
        (!self.entries.is_empty()).then_some(self.selected)
    }
}

impl Component for ContextMenu {
    fn measure(&mut self, _: &Layer) -> (f32, f32) {
        (0.0, 0.0)
    }

    fn sync(&mut self, context: &Context) {
        // Shadow upkeep precedes everything — a closing ghost must still
        // reach the blurred layer to take its halo with it.
        let reveal_changed = self.reveal != context.reveal;
        if reveal_changed {
            self.reveal = context.reveal;
        }
        self.paint_shadow(context.self_rect);
        if !self.open {
            return;
        }
        if reveal_changed && !(self.dismissing && self.reveal <= 0.0) {
            self.dirty.set();
        }
        if self.dismissing {
            // Ghost: one lazy park of the pill with the REAL region rect —
            // the ctor couldn't (`dismissing`'s comment explains why) — then
            // freeze; a ghost's slide never advances.
            if !self.started && !self.entries.is_empty() {
                let card = card_anchored(context.self_rect, self.anchor, self.entries.len());
                let y = card.y + PAD_Y + self.selected as f32 * ROW_HEIGHT;
                self.slide.park(Rect::new(
                    card.x + 4.0,
                    y + 2.0,
                    card.width - 8.0,
                    ROW_HEIGHT - 4.0,
                ));
                self.started = true;
                self.dirty.set();
            }
        } else {
            let viewport = context.self_rect;
            let e = MENU_SLIDE_EASING.apply(self.reveal).clamp(0.0, 1.0);
            let card = revealed_card(
                card_anchored(viewport, self.anchor, self.entries.len()),
                self.anchor,
                e,
            );
            // Pointer hovers move the live selection (the shell reads it via
            // `row_at` in input.rs — actually the shell routes clicks, this
            // only needs the visual), so track it through the shared geometry.
            if context.mouse.in_window && card.contains(context.mouse.position) {
                if let Some(row) = row_at(card, self.entries.len(), context.mouse.position) {
                    if row != self.selected {
                        self.selected = row;
                        self.dirty.set();
                        self.hovered_by_pointer = true;
                    }
                } else if self.hovered_by_pointer {
                    // Left the card: keep the highlight where it was.
                    self.hovered_by_pointer = false;
                }
            }
            if let Some(row) = self.pill_target() {
                let y = card.y + PAD_Y + row as f32 * ROW_HEIGHT;
                let rect = Rect::new(card.x + 4.0, y + 2.0, card.width - 8.0, ROW_HEIGHT - 4.0);
                if !self.started {
                    self.slide.park(rect);
                    self.started = true;
                    self.dirty.set();
                } else if self.slide.slide_to(rect) {
                    self.dirty.set();
                }
            }
        }
        self.slide.advance(context.animation_dt);
        if self.slide.advancing() || reveal_changed {
            self.dirty.set();
        }
    }

    fn is_dirty(&self) -> bool {
        self.dirty.get()
    }

    fn clear_dirty(&mut self) {
        self.dirty.clear()
    }

    fn is_animating(&self) -> bool {
        // Never report a never-advanced Slide on a closed snapshot — that
        // pins the frame loop open (see popup's module docs).
        self.open && (self.slide.advancing() || (self.reveal > 0.0 && self.reveal < 1.0))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn draw(&mut self, layer: &Layer, rect: Rect) {
        if !self.open {
            return;
        }

        // Unlike a modal, a context menu is local to the pointer; dimming
        // the document behind it would make a small action menu noisy.
        let e = MENU_SLIDE_EASING.apply(self.reveal);
        let resting = card_anchored(rect, self.anchor, self.entries.len());
        let card = revealed_card(resting, self.anchor, e);
        let ea = e.clamp(0.0, 1.0);

        layer.draw_rectangle(
            card.position(),
            card.size(),
            theme::elevated_popup(ea),
            Rounding::uniform(super::popup::CARD_RADIUS),
        );
        theme::rounded_outline(
            layer,
            card.inset(0.5),
            super::popup::CARD_RADIUS - 0.5,
            1.0,
            theme::fade(theme::non_text(), ea),
        );

        let title_style = TextStyle::serif(14.5, theme::fade(theme::ink(), ea));
        let hint_style = TextStyle::mono(10.0, theme::fade(theme::faint(), ea));

        // The slide pill rides under the selected/hovered row — one
        // highlight for pointer and keyboard alike.
        if self.started {
            let pill = self.slide.rect();
            layer.draw_rectangle(
                pill.position(),
                pill.size(),
                theme::fade(theme::selection(), ea),
                Rounding::uniform(ROW_RADIUS),
            );
        }

        for (index, entry) in self.entries.iter().enumerate() {
            let row = Rect::new(
                card.x,
                card.y + PAD_Y + index as f32 * ROW_HEIGHT,
                card.width,
                ROW_HEIGHT,
            );
            let middle = row.y + row.height / 2.0;
            if self.checked.get(index).copied().unwrap_or(false) {
                theme::icon(
                    layer,
                    theme::icons::CHECK,
                    (row.x + 10.0, middle - 7.0),
                    14.0,
                    theme::fade(theme::accent(), ea),
                    1.8,
                );
            }
            theme::draw(
                layer,
                &entry.title,
                (row.x + TITLE_X, middle),
                &title_style,
                theme::LEFT,
            );
            if !entry.hint.is_empty() {
                theme::draw(
                    layer,
                    &entry.hint,
                    (row.right() - 14.0, middle),
                    &hint_style,
                    theme::RIGHT,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_menu_near_the_bottom_flips_up() {
        let viewport = Rect::new(0.0, 0.0, 400.0, 300.0);
        let anchor = (100.0, 290.0);
        let card = card_anchored(viewport, anchor, 3);
        assert!(card.bottom() <= anchor.1);
        assert!(card.y >= viewport.y);
        assert!(card.bottom() <= viewport.bottom());
    }

    #[test]
    fn a_menu_near_the_right_edge_shifts_left() {
        let viewport = Rect::new(0.0, 0.0, 400.0, 300.0);
        let anchor = (390.0, 100.0);
        let card = card_anchored(viewport, anchor, 3);
        assert!(card.x < anchor.0);
        assert!(card.x >= viewport.x);
        assert!(card.right() <= viewport.right());
    }

    #[test]
    fn row_at_maps_a_point_to_the_row_drawn_there() {
        let card = card_anchored(Rect::new(0.0, 0.0, 500.0, 400.0), (40.0, 40.0), 3);
        assert_eq!(
            row_at(card, 3, (card.x + 20.0, card.y + PAD_Y + ROW_HEIGHT / 2.0)),
            Some(0)
        );
        assert_eq!(
            row_at(card, 3, (card.x + 20.0, card.y + PAD_Y + ROW_HEIGHT * 2.5)),
            Some(2)
        );
        assert_eq!(row_at(card, 3, (card.x + 20.0, card.y + PAD_Y / 2.0)), None);
        assert_eq!(row_at(card, 3, (card.right() + 1.0, card.y + 15.0)), None);
    }

    #[test]
    fn a_check_is_drawn_on_the_row_with_the_same_entry_index() {
        let entries = vec![
            Entry {
                title: "First".into(),
                group: "".into(),
                hint: "".into(),
            },
            Entry {
                title: "Second".into(),
                group: "".into(),
                hint: "".into(),
            },
        ];
        let menu = ContextMenu::new(entries, vec![false, true], 0, (0.0, 0.0));
        assert!(!menu.checked[0]);
        assert!(menu.checked[1]);
    }
}
