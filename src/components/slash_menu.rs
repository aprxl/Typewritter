//! The slash command menu: a compact inline popup next to the text caret.
//!
//! Same relationship to the shell as [`Palette`](super::Palette) — the shell
//! owns the keystrokes and the query, this component holds a snapshot and
//! draws it. The card speaks the shared popup vocabulary — an elevated
//! surface that grows out of the caret with a gentle overshoot, a blurred
//! shadow painted into the shell's layer, and a hover pill that slides
//! between rows whether the pointer or the arrow keys moved it — and closes
//! as a ghost of its real rows falling away on a shared clock.

use crate::layout::Rect;
use crate::renderer::{Layer, Rounding};
use crate::theme::{self, TextStyle};
use crate::ui::{Component, Context, Dirty};

use super::palette::{self, Entry};
use super::popup::{MENU_SLIDE_EASING, Slide, paint_shadow_slab, revealed_card};

/// Corner radius of a row highlight — softer than the card itself, nested
/// smaller things always round less.
const ROW_RADIUS: f32 = 6.0;

const CARD_W: f32 = 260.0;
const QUERY_H: f32 = 56.0;
const ROW_HEIGHT: f32 = 34.0;
/// Derived from `QUERY_H` + 6 rows, so a constant and its derivation live
/// next to each other rather than drifting apart.
const CARD_H: f32 = QUERY_H + 6.0 * ROW_HEIGHT;
pub const MAX_ROWS: usize = ((CARD_H - QUERY_H) / ROW_HEIGHT) as usize;

/// The card's top-left corner sits at `anchor` by default. If it would
/// overflow the viewport bottom, the card flips upward (bottom-left corner
/// at anchor); if it would overflow the right edge, it shifts left.
pub fn card_anchored(viewport: Rect, anchor: (f32, f32)) -> Rect {
    let mut x = anchor.0;
    let mut y = anchor.1;

    if y + CARD_H > viewport.bottom() {
        y = anchor.1 - CARD_H;
    }

    if x + CARD_W > viewport.right() {
        x = viewport.right() - CARD_W;
    }

    Rect::new(x, y, CARD_W, CARD_H)
}

/// Everything a dismissal ghost needs to redraw the menu exactly as it
/// stood: cloneable real state, no placeholders. The shell captures this
/// at close time (`MenuDismiss::Slash`).
#[derive(Clone)]
pub struct Snapshot {
    pub entries: Vec<Entry>,
    pub visible: Vec<usize>,
    pub query: String,
    pub selected: usize,
    pub first_visible: usize,
}

impl SlashMenu {
    /// The ghost-rebuild data for this snapshot.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            entries: self.entries.clone(),
            visible: self.visible.clone(),
            query: self.query.clone(),
            selected: self.selected,
            first_visible: self.first_visible,
        }
    }
}

pub struct SlashMenu {
    entries: Vec<Entry>,
    visible: Vec<usize>,
    query: String,
    selected: usize,
    first_visible: usize,
    anchor: (f32, f32),
    caret_on: bool,
    open: bool,
    /// True while this snapshot is a dismissal ghost: dead input, reveal
    /// weight falling toward 0 through the same entrance curve.
    dismissing: bool,
    /// The row the slide pill sits on — pointer and keyboard both feed it.
    pointer_row: Option<usize>,
    /// The blurred layer this card paints its shadow slab into; `None` on
    /// snapshots built before the shell hands it over (tests).
    shadow: Option<Layer>,
    reveal: f32,
    slide: Slide,
    started: bool,
    dirty: Dirty,
}

impl SlashMenu {
    pub fn new(entries: Vec<Entry>, query: String, selected: usize, anchor: (f32, f32)) -> Self {
        let visible = palette::filter(&entries, &query);
        let selected = if visible.is_empty() {
            0
        } else {
            selected.min(visible.len() - 1)
        };
        let first_visible = selected.saturating_sub(MAX_ROWS.saturating_sub(1));
        Self {
            entries,
            visible,
            query,
            selected,
            first_visible,
            anchor,
            caret_on: true,
            open: true,
            dismissing: false,
            pointer_row: Some(selected),
            shadow: None,
            reveal: 0.0,
            slide: Slide::new(),
            started: false,
            dirty: Dirty::new(),
        }
    }

    /// A dismissal ghost rebuilt from the snapshot the shell captured at
    /// close time: same rows, same scroll, pill parked where it was. Input
    /// is dead; the reveal weight falls from outside.
    pub fn dismissing(snap: &Snapshot, anchor: (f32, f32), pointer_row: Option<usize>) -> Self {
        let ghost = Self {
            entries: snap.entries.clone(),
            visible: snap.visible.clone(),
            query: snap.query.clone(),
            selected: snap.selected,
            first_visible: snap.first_visible,
            anchor,
            caret_on: false,
            open: true,
            dismissing: true,
            pointer_row,
            shadow: None,
            reveal: 1.0,
            slide: Slide::new(),
            started: false,
            dirty: Dirty::new(),
        };
        // The pill is NOT parked here: `card_anchored(Rect::default(), ..)`
        // panics its own clamp (zero-width viewport ⇒ clamp max −CARD_W).
        // A ghost's first `sync` carries the region rect and parks it.
        ghost
    }

    pub fn closed() -> Self {
        Self {
            entries: Vec::new(),
            visible: Vec::new(),
            query: String::new(),
            selected: 0,
            first_visible: 0,
            anchor: (0.0, 0.0),
            caret_on: true,
            open: false,
            dismissing: false,
            pointer_row: None,
            shadow: None,
            reveal: 1.0,
            slide: Slide::new(),
            started: false,
            dirty: Dirty::new(),
        }
    }
}

impl SlashMenu {
    /// Attaches the shell's blurred shadow layer — same shape as the format
    /// bar's `with_shadow`. Every snapshot the shell builds carries it,
    /// closed ghosts included, so a closing menu always takes its halo away
    /// (`paint_shadow_slab` clears unconditionally).
    pub fn with_shadow(mut self, shadow: Layer) -> Self {
        self.shadow = Some(shadow);
        self
    }

    /// The screen-space row the `offset`th visible row occupies inside
    /// `card` — one geometry for the pill, hover hit-testing, and drawing.
    fn row_rect(&self, card: Rect, offset: usize) -> Rect {
        Rect::new(
            card.x,
            card.y + QUERY_H + offset as f32 * ROW_HEIGHT,
            card.width,
            ROW_HEIGHT,
        )
    }

    /// The absolute visible-row index under `point`, if it lands on the
    /// list area and within the drawn window.
    fn row_at(&self, card: Rect, point: (f32, f32)) -> Option<usize> {
        let hit = row_at_impl(card, self.first_visible, point)?;
        (hit < self.first_visible + self.window_len()).then_some(hit)
    }

    /// How many rows are actually drawn: a window of MAX_ROWS over the
    /// filtered list starting at `first_visible`.
    fn window_len(&self) -> usize {
        self.visible
            .len()
            .saturating_sub(self.first_visible)
            .min(MAX_ROWS)
    }

    /// Paints the shadow slab into the blurred layer from sync — which runs
    /// whether or not draw does — so a closing ghost takes its halo with it.
    /// The slab tracks the resting card: blur already softens what it lands
    /// on, and chasing the overshoot visually doubles it.
    fn paint_shadow(&mut self, owns: bool, viewport: Rect) {
        if !owns {
            return;
        }
        let Some(shadow) = self.shadow.clone() else {
            return;
        };
        if !self.open || self.reveal < 0.0 {
            paint_shadow_slab(&shadow, Rect::default(), -1.0);
            return;
        }
        let e = MENU_SLIDE_EASING.apply(self.reveal).clamp(0.0, 1.0);
        let card = revealed_card(card_anchored(viewport, self.anchor), self.anchor, 1.0);
        paint_shadow_slab(&shadow, card, e);
    }
}

impl Component for SlashMenu {
    fn measure(&mut self, _: &Layer) -> (f32, f32) {
        (0.0, 0.0)
    }

    fn sync(&mut self, context: &Context) {
        // Shadow upkeep precedes every other decision — a closing ghost must
        // still reach the blurred layer to take its halo with it.
        let reveal_changed = self.reveal != context.reveal;
        if reveal_changed {
            self.reveal = context.reveal;
        }
        self.paint_shadow(context.owns_shadow, context.self_rect);
        if !self.open {
            return;
        }
        if reveal_changed && !(self.dismissing && self.reveal <= 0.0) {
            self.dirty.set();
        }
        if self.dismissing {
            // Ghost: one lazy park of the pill with the REAL region rect —
            // the ctor couldn't (`dismissing`'s comment explains why) —
            // then freeze; a ghost's slide never advances.
            if !self.started && !self.visible.is_empty() {
                let e = MENU_SLIDE_EASING.apply(self.reveal).clamp(0.0, 1.0);
                let card = revealed_card(
                    card_anchored(context.self_rect, self.anchor),
                    self.anchor,
                    e,
                );
                let offset = self
                    .pointer_row
                    .unwrap_or(self.selected)
                    .saturating_sub(self.first_visible);
                if offset < MAX_ROWS {
                    let r = Rect::new(
                        card.x,
                        card.y + QUERY_H + offset as f32 * ROW_HEIGHT,
                        card.width,
                        ROW_HEIGHT,
                    );
                    self.slide.park(r);
                    self.started = true;
                    self.dirty.set();
                }
            }
        } else {
            self.dirty.write(&mut self.caret_on, context.caret_on);

            // Two feeds, one pill. Keyboard: the shell rebuilds this
            // snapshot on every selection change, and `new` seeds
            // `pointer_row` from the selection — so arrows drive the pill
            // across rebuilds. Pointer: hover lands here, frame by frame,
            // and persists once a row has been touched — sweeping into a
            // gap or off the card leaves the highlight where it was.
            let viewport = context.self_rect;
            let e = MENU_SLIDE_EASING.apply(self.reveal).clamp(0.0, 1.0);
            let card = revealed_card(card_anchored(viewport, self.anchor), self.anchor, e);
            if context.mouse.in_window && card.contains(context.mouse.position) {
                let over = self.row_at(card, context.mouse.position);
                if over != self.pointer_row {
                    self.pointer_row = over;
                    self.dirty.set();
                }
            }
            // Window-relative target for the drawn geometry.
            let target = self
                .pointer_row
                .and_then(|abs| abs.checked_sub(self.first_visible))
                .filter(|&offset| offset < self.window_len());
            if let Some(offset) = target {
                let rect = self.row_rect(card, offset);
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
        self.dirty.clear();
    }

    fn is_animating(&self) -> bool {
        // A closed snapshot's Slide is constructed playing but never
        // advanced; reporting it would pin the frame loop open forever (see
        // `popup`'s module docs). Only an open card animates.
        self.open && (self.slide.advancing() || self.reveal > 0.0 && self.reveal < 1.0)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn draw(&mut self, layer: &Layer, rect: Rect) {
        if !self.open {
            return;
        }

        // The whole card grows out of the caret: the reveal weight runs the
        // spring curve, overshooting ~6% before settling, and every alpha
        // below rides the same fade clamped into range.
        let e = MENU_SLIDE_EASING.apply(self.reveal);
        let resting = card_anchored(rect, self.anchor);
        let card = revealed_card(resting, self.anchor, e);
        let ea = e.clamp(0.0, 1.0);

        // Elevated gradient surface with soft shoulders — see `popup`.
        layer.draw_rectangle(
            card.position(),
            card.size(),
            crate::theme::elevated_popup(ea),
            Rounding::uniform(crate::components::popup::CARD_RADIUS),
        );
        crate::theme::rounded_outline(
            layer,
            card.inset(0.5),
            crate::components::popup::CARD_RADIUS - 0.5,
            1.0,
            crate::theme::fade(crate::theme::non_text(), ea),
        );

        self.draw_query(layer, card, ea);
        theme::rule(
            layer,
            (card.x + 20.0, card.y + QUERY_H),
            card.width - 40.0,
            1.0,
            theme::fade(theme::border(), ea),
        );

        if self.visible.is_empty() {
            theme::draw(
                layer,
                "No matching command",
                (card.x + card.width / 2.0, card.y + QUERY_H + 32.0),
                &TextStyle::sans(13.5, theme::fade(theme::faint(), ea)),
                theme::CENTER,
            );
        } else {
            self.draw_rows(layer, card, ea);
        }
    }
}

impl SlashMenu {
    fn draw_query(&self, layer: &Layer, card: Rect, ea: f32) {
        let style = TextStyle::sans(16.0, theme::fade(theme::ink(), ea));
        let middle = card.y + QUERY_H / 2.0;
        if self.query.is_empty() {
            theme::draw(
                layer,
                "Type a command",
                (card.x + 20.0, middle),
                &style.clone().color(theme::fade(theme::faint(), ea)),
                theme::LEFT,
            );
        } else {
            theme::draw(
                layer,
                &self.query,
                (card.x + 20.0, middle),
                &style,
                theme::LEFT,
            );
        }

        if self.caret_on {
            let x = card.x + 20.0 + theme::width(layer, &self.query, &style);
            layer.draw_rectangle(
                (x, middle - 10.0),
                (2.0, 20.0),
                theme::fade(theme::accent(), ea),
                Rounding::NONE,
            );
        }
    }

    fn draw_rows(&self, layer: &Layer, card: Rect, ea: f32) {
        let title_style = TextStyle::sans(13.0, theme::fade(theme::ink(), ea));
        let group_style = TextStyle::sans(10.5, theme::fade(theme::comment(), ea));
        let hint_style = TextStyle::mono(10.5, theme::fade(theme::faint(), ea));

        // The slide pill rides UNDER the text of whichever visible row was
        // last touched — pointer and keyboard feed one highlight (see
        // `sync`).
        let pill_offset = self.pointer_row.and_then(|abs| {
            let offset = abs.checked_sub(self.first_visible)?;
            (offset < MAX_ROWS).then_some(offset)
        });
        if let Some(offset) = pill_offset {
            let row = self.row_rect(card, offset);
            layer.draw_rectangle(
                (row.x + 4.0, row.y + 2.0),
                (row.width - 8.0, row.height - 4.0),
                theme::fade(theme::selection(), ea),
                Rounding::uniform(ROW_RADIUS),
            );
        }

        let window = self.visible[self.first_visible..]
            .iter()
            .take(MAX_ROWS)
            .enumerate();
        for (offset, &entry_index) in window {
            let entry = &self.entries[entry_index];
            let row = self.row_rect(card, offset);
            let middle = row.y + row.height / 2.0;

            if pill_offset.is_none() && self.first_visible + offset == self.selected {
                // Before the pill's first park lands (one frame), the flat
                // highlight keeps the selection visible.
                layer.draw_rectangle(
                    row.position(),
                    row.size(),
                    theme::fade(theme::selection(), ea),
                    Rounding::NONE,
                );
            }

            theme::draw(
                layer,
                &entry.title,
                (row.x + 20.0, middle),
                &title_style,
                theme::LEFT,
            );
            let group_x = row.x + 20.0 + theme::width(layer, &entry.title, &title_style) + 8.0;
            theme::draw(
                layer,
                &entry.group,
                (group_x, middle),
                &group_style,
                theme::LEFT,
            );

            if !entry.hint.is_empty() {
                theme::draw(
                    layer,
                    &entry.hint,
                    (row.right() - 20.0, middle),
                    &hint_style,
                    theme::RIGHT,
                );
            }
        }
    }
}

impl SlashMenu {
    /// The row the slide pill currently sits on, as an absolute index into
    /// `visible` — the shell reads this so a refresh cannot forget where the
    /// highlight was (same reason the format bar exposes `pointer_cell`).
    pub fn pill_row(&self) -> Option<usize> {
        self.pointer_row
    }
}

/// The visible row under `point` as an ABSOLUTE index into `visible`, or
/// `None`. Free function so tests can pin it without a full snapshot.
fn row_at_impl(card: Rect, first_visible: usize, point: (f32, f32)) -> Option<usize> {
    if !card.contains(point) || point.1 < card.y + QUERY_H {
        return None;
    }
    let offset = ((point.1 - card.y - QUERY_H) / ROW_HEIGHT) as usize;
    Some(first_visible + offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(title: &str, group: &str) -> Entry {
        Entry {
            title: title.into(),
            group: group.into(),
            hint: String::new(),
        }
    }

    #[test]
    fn anchor_opens_downward_by_default() {
        let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
        let r = card_anchored(viewport, (100.0, 200.0));
        assert_eq!(r.x, 100.0);
        assert_eq!(r.y, 200.0);
    }

    #[test]
    fn anchor_flips_upward_near_bottom() {
        let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
        let r = card_anchored(viewport, (100.0, 400.0));
        assert_eq!(r.x, 100.0);
        assert_eq!(r.y, 400.0 - CARD_H);
        // Bottom edge should be at the anchor point.
        assert_eq!(r.bottom(), 400.0);
    }

    #[test]
    fn anchor_clamps_near_right_edge() {
        let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
        let r = card_anchored(viewport, (700.0, 200.0));
        assert_eq!(r.x, 800.0 - CARD_W);
        assert_eq!(r.y, 200.0);
    }

    #[test]
    fn anchor_flips_and_clamps_near_bottom_right_corner() {
        let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
        let r = card_anchored(viewport, (750.0, 500.0));
        assert_eq!(r.x, 800.0 - CARD_W);
        assert_eq!(r.y, 500.0 - CARD_H);
    }

    #[test]
    fn closed_has_open_false() {
        let m = SlashMenu::closed();
        assert!(!m.open);
    }

    #[test]
    fn new_clamps_selected_into_bounds() {
        let entries = vec![entry("Bold", "Format"), entry("Italic", "Format")];
        let m = SlashMenu::new(entries, "".into(), 99, (0.0, 0.0));
        assert_eq!(m.selected, 1);
    }

    #[test]
    fn new_clamps_selected_to_zero_when_empty() {
        let m = SlashMenu::new(vec![], "x".into(), 0, (0.0, 0.0));
        assert_eq!(m.selected, 0);
        assert!(m.visible.is_empty());
    }
}
