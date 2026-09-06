//! The in-math completion card: ordinary interpretations followed by a
//! compact grid of notation variants for an exact known symbol.
//!
//! The shell owns the query, selection, and input. This component owns all
//! menu geometry so drawing and mouse hit-testing use the same rectangles.
//! The card speaks the shared popup vocabulary — elevated surface springing
//! from the math caret, blurred shadow on the shell's layer, one sliding
//! pill for pointer and keyboard — and closes as a ghost of its real rows
//! on the shared clock.

use crate::layout::Rect;
use crate::renderer::{Layer, Rounding};
use crate::theme::{self, TextStyle};
use crate::ui::{Component, Context, Dirty};

use super::popup::{MENU_SLIDE_EASING, Slide, paint_shadow_slab, revealed_card};

/// Corner radius of a row highlight — nested smaller things round less than
/// the card itself (see `popup::CARD_RADIUS`).
const ROW_RADIUS: f32 = 6.0;

/// Everything a dismissal ghost needs to redraw the card exactly as it
/// stood: cloneable real state, no placeholders. The shell captures this at
/// close time (`MenuDismiss::Math`).
#[derive(Clone)]
pub struct Snapshot {
    pub rows: Vec<Row>,
    pub variant_start: usize,
    pub selected: usize,
    pub first_visible: usize,
}

impl MathMenu {
    /// The ghost-rebuild data for this snapshot.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            rows: self.rows.clone(),
            variant_start: self.variant_start,
            selected: self.selected,
            first_visible: self.first_visible,
        }
    }

    /// The row/cell the pill sits on — the shell reads this at close time.
    pub fn pill_row(&self) -> Option<usize> {
        (!self.rows.is_empty()).then_some(self.selected)
    }
}

const CARD_W: f32 = 320.0;
const ROW_HEIGHT: f32 = 36.0;
/// Keep the full variant grid beside a short window of interpretations.
/// Even the largest catalog entry fits the application's compact window.
const MAX_ROWS: usize = 6;
const MORE_HEIGHT: f32 = 24.0;
const PAD_Y: f32 = 8.0;
const GRID_PAD_X: f32 = 8.0;
const GRID_HEADER_HEIGHT: f32 = 22.0;
const GRID_CELL_HEIGHT: f32 = 34.0;
const GRID_GAP: f32 = 5.0;
const GRID_COLUMNS: usize = 5;

/// One offered completion as the card shows it.
#[derive(Clone, PartialEq)]
pub struct Row {
    pub name: String,
    pub group: String,
    pub preview: String,
}

fn split(items: usize, variant_start: usize) -> (usize, usize) {
    let regular = variant_start.min(items);
    (regular, items - regular)
}

fn grid_rows(variants: usize) -> usize {
    variants.div_ceil(GRID_COLUMNS)
}

fn row_window(regular: usize, first_visible: usize) -> std::ops::Range<usize> {
    let start = first_visible.min(regular.saturating_sub(MAX_ROWS));
    start..(start + MAX_ROWS).min(regular)
}

/// Scroll only when keyboard selection leaves the visible interpretations.
/// Pointer selection and the always-visible variant grid keep this window.
pub fn follow_selection(regular: usize, selected: usize, first_visible: usize) -> usize {
    let window = row_window(regular, first_visible);
    if selected >= regular {
        window.start
    } else if selected < window.start {
        selected
    } else if selected >= window.end {
        selected.saturating_sub(MAX_ROWS - 1)
    } else {
        window.start
    }
}

fn rows_height(regular: usize) -> f32 {
    regular.min(MAX_ROWS) as f32 * ROW_HEIGHT + if regular > MAX_ROWS { MORE_HEIGHT } else { 0.0 }
}

fn card_height(items: usize, variant_start: usize) -> f32 {
    let (regular, variants) = split(items, variant_start);
    let grid = grid_rows(variants);
    PAD_Y * 2.0
        + rows_height(regular)
        + if grid == 0 {
            0.0
        } else {
            GRID_HEADER_HEIGHT
                + grid as f32 * GRID_CELL_HEIGHT
                + grid.saturating_sub(1) as f32 * GRID_GAP
        }
}

/// The card for `items`, where `variant_start..items` is drawn as a grid.
pub fn card_anchored(
    viewport: Rect,
    anchor: (f32, f32),
    items: usize,
    variant_start: usize,
) -> Rect {
    let height = card_height(items, variant_start);
    let x = if anchor.0 + CARD_W > viewport.right() {
        viewport.right() - CARD_W
    } else {
        anchor.0
    }
    .clamp(viewport.x, (viewport.right() - CARD_W).max(viewport.x));
    let y = if anchor.1 + height > viewport.bottom() {
        anchor.1 - height
    } else {
        anchor.1
    }
    .clamp(viewport.y, (viewport.bottom() - height).max(viewport.y));
    Rect::new(x, y, CARD_W, height)
}

/// The exact rectangle used to draw a flat item index. Hidden interpretations
/// have no rectangle; selection scrolls the window without changing indices.
pub fn item_rect(
    card: Rect,
    items: usize,
    variant_start: usize,
    first_visible: usize,
    index: usize,
) -> Option<Rect> {
    if index >= items {
        return None;
    }
    let (regular, _) = split(items, variant_start);
    if index < regular {
        let window = row_window(regular, first_visible);
        if !window.contains(&index) {
            return None;
        }
        return Some(Rect::new(
            card.x,
            card.y + PAD_Y + (index - window.start) as f32 * ROW_HEIGHT,
            card.width,
            ROW_HEIGHT,
        ));
    }

    let variant = index - regular;
    let column = variant % GRID_COLUMNS;
    let row = variant / GRID_COLUMNS;
    let width =
        (card.width - GRID_PAD_X * 2.0 - GRID_GAP * (GRID_COLUMNS.saturating_sub(1)) as f32)
            / GRID_COLUMNS as f32;
    Some(Rect::new(
        card.x + GRID_PAD_X + column as f32 * (width + GRID_GAP),
        card.y
            + PAD_Y
            + rows_height(regular)
            + GRID_HEADER_HEIGHT
            + row as f32 * (GRID_CELL_HEIGHT + GRID_GAP),
        width,
        GRID_CELL_HEIGHT,
    ))
}

/// Flat item index under `point`, sharing [`item_rect`] with drawing.
pub fn item_at(
    card: Rect,
    items: usize,
    variant_start: usize,
    first_visible: usize,
    point: (f32, f32),
) -> Option<usize> {
    if !card.contains(point) {
        return None;
    }
    (0..items).find(|&index| {
        item_rect(card, items, variant_start, first_visible, index)
            .is_some_and(|rect| rect.contains(point))
    })
}

pub struct MathMenu {
    rows: Vec<Row>,
    variant_start: usize,
    selected: usize,
    first_visible: usize,
    anchor: (f32, f32),
    open: bool,
    /// True while this snapshot is a dismissal ghost: dead input, reveal
    /// weight falling toward 0 through the entrance curve.
    dismissing: bool,
    /// The blurred layer this card paints its shadow slab into.
    shadow: Option<Layer>,
    reveal: f32,
    slide: Slide,
    started: bool,
    dirty: Dirty,
}

impl MathMenu {
    pub fn new(
        rows: Vec<Row>,
        variant_start: usize,
        selected: usize,
        first_visible: usize,
        anchor: (f32, f32),
    ) -> Self {
        let selected = selected.min(rows.len().saturating_sub(1));
        let variant_start = variant_start.min(rows.len());
        Self {
            variant_start,
            first_visible: follow_selection(variant_start, selected, first_visible),
            rows,
            selected,
            anchor,
            open: true,
            dismissing: false,
            shadow: None,
            reveal: 0.0,
            slide: Slide::new(),
            started: false,
            dirty: Dirty::new(),
        }
    }

    /// A dismissal ghost rebuilt from the state captured at close time:
    /// same rows and grid, pill parked on the selection. Input is dead; the
    /// reveal weight falls from outside.
    pub fn dismissing(snap: &Snapshot, anchor: (f32, f32), pill_row: usize) -> Self {
        Self {
            rows: snap.rows.clone(),
            variant_start: snap.variant_start,
            selected: pill_row.min(snap.rows.len().saturating_sub(1)),
            first_visible: snap.first_visible,
            anchor,
            open: true,
            dismissing: true,
            shadow: None,
            reveal: 1.0,
            slide: Slide::new(),
            started: false,
            dirty: Dirty::new(),
        }
    }

    pub fn closed() -> Self {
        Self {
            rows: Vec::new(),
            variant_start: 0,
            selected: 0,
            first_visible: 0,
            anchor: (0.0, 0.0),
            open: false,
            dismissing: false,
            shadow: None,
            reveal: 1.0,
            slide: Slide::new(),
            started: false,
            dirty: Dirty::new(),
        }
    }

    /// Keep a selection's motion across shell snapshots. A different query,
    /// scroll window, or anchor starts in place rather than crossing new rows.
    pub fn resuming(mut self, previous: Option<&Self>) -> Self {
        if let Some(previous) = previous
            && previous.open
            && previous.started
            && !previous.dismissing
            && self.rows == previous.rows
            && self.variant_start == previous.variant_start
            && self.first_visible == previous.first_visible
            && self.anchor == previous.anchor
        {
            self.slide.park(previous.slide.rect());
            self.started = true;
        }
        self
    }

    /// Attaches the shell's blurred shadow layer — every snapshot the shell
    /// builds carries it, ghosts included.
    pub fn with_shadow(mut self, shadow: Layer) -> Self {
        self.shadow = Some(shadow);
        self
    }

    /// Paints the shadow slab from sync — see `SlashMenu::paint_shadow`.
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
        let card = revealed_card(
            card_anchored(viewport, self.anchor, self.rows.len(), self.variant_start),
            self.anchor,
            1.0,
        );
        paint_shadow_slab(&shadow, card, e);
    }
}

/// A row/cell rect shrunk to the pill that rides under it — one rule for
/// flat rows and grid cells alike.
fn inset_for_pill(rect: Rect) -> Rect {
    Rect::new(
        rect.x + 3.0,
        rect.y + 2.0,
        rect.width - 6.0,
        rect.height - 4.0,
    )
}

impl Component for MathMenu {
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
        self.paint_shadow(context.owns_shadow, context.self_rect);
        if !self.open || self.rows.is_empty() {
            return;
        }
        if reveal_changed && !(self.dismissing && self.reveal <= 0.0) {
            self.dirty.set();
        }
        if !self.dismissing || !self.started {
            // The pill is persistent: seeded by `new` from the keyboard
            // selection and parked on the first sync.
            if let Some(rect) = item_rect(
                card_anchored(
                    context.self_rect,
                    self.anchor,
                    self.rows.len(),
                    self.variant_start,
                ),
                self.rows.len(),
                self.variant_start,
                self.first_visible,
                self.selected,
            ) {
                let target = inset_for_pill(rect);
                if !self.started {
                    self.slide.park(target);
                    self.started = true;
                    self.dirty.set();
                } else if self.slide.slide_to(target) {
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

    fn draw(&mut self, layer: &Layer, viewport: Rect) {
        if !self.open || self.rows.is_empty() {
            return;
        }

        let e = MENU_SLIDE_EASING.apply(self.reveal);
        let resting = card_anchored(viewport, self.anchor, self.rows.len(), self.variant_start);
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

        let name_style = TextStyle::sans(13.0, theme::fade(theme::ink(), ea));
        let group_style = TextStyle::sans(11.5, theme::fade(theme::comment(), ea));
        let preview_style = TextStyle::math(17.0, theme::fade(theme::ink(), ea));

        // Cell surfaces go below the selection, so a selected variant is
        // just as visible as a selected interpretation.
        for index in self.variant_start..self.rows.len() {
            let rect = item_rect(
                card,
                self.rows.len(),
                self.variant_start,
                self.first_visible,
                index,
            )
            .expect("a variant must have geometry");
            layer.draw_rectangle(
                rect.position(),
                rect.size(),
                theme::fade(theme::alt(), ea),
                Rounding::uniform(5.0),
            );
        }
        if self.started {
            let pill = revealed_card(self.slide.rect(), self.anchor, e);
            layer.draw_rectangle(
                pill.position(),
                pill.size(),
                theme::fade(theme::selection(), ea),
                Rounding::uniform(ROW_RADIUS),
            );
        }
        let window = row_window(self.variant_start, self.first_visible);
        for index in window.clone() {
            let row = &self.rows[index];
            let rect = item_rect(
                card,
                self.rows.len(),
                self.variant_start,
                self.first_visible,
                index,
            )
            .expect("a menu row must have geometry");
            let middle = rect.y + rect.height / 2.0;
            theme::draw(
                layer,
                &row.name,
                (rect.x + 20.0, middle),
                &name_style,
                theme::LEFT,
            );
            let group_x = rect.x + 20.0 + theme::width(layer, &row.name, &name_style) + 8.0;
            theme::draw(
                layer,
                &row.group,
                (group_x, middle),
                &group_style,
                theme::LEFT,
            );
            if !row.preview.is_empty() {
                theme::draw(
                    layer,
                    &row.preview,
                    (rect.right() - 20.0, middle),
                    &preview_style,
                    theme::RIGHT,
                );
            }
        }

        if self.variant_start > MAX_ROWS {
            let middle = card.y + PAD_Y + MAX_ROWS as f32 * ROW_HEIGHT + MORE_HEIGHT / 2.0;
            let style = TextStyle::sans(11.0, theme::fade(theme::faint(), ea));
            theme::draw(
                layer,
                &format!(
                    "{}–{} of {} matches",
                    window.start + 1,
                    window.end,
                    self.variant_start
                ),
                (card.x + 20.0, middle),
                &style,
                theme::LEFT,
            );
            theme::draw(
                layer,
                "↑↓ to browse",
                (card.right() - 20.0, middle),
                &style,
                theme::RIGHT,
            );
        }
        if self.variant_start < self.rows.len() {
            let header_y =
                card.y + PAD_Y + rows_height(self.variant_start) + GRID_HEADER_HEIGHT / 2.0;
            theme::draw(
                layer,
                "VARIANTS",
                (card.x + GRID_PAD_X, header_y),
                &group_style,
                theme::LEFT,
            );
        }
        for (index, row) in self.rows[self.variant_start..].iter().enumerate() {
            let flat_index = self.variant_start + index;
            let rect = item_rect(
                card,
                self.rows.len(),
                self.variant_start,
                self.first_visible,
                flat_index,
            )
            .expect("a variant must have geometry");
            theme::draw(
                layer,
                &row.preview,
                (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0),
                &preview_style,
                theme::CENTER,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(count: usize) -> Vec<Row> {
        (0..count)
            .map(|index| Row {
                name: format!("name{index}"),
                group: "Group".into(),
                preview: format!("p{index}"),
            })
            .collect()
    }

    #[test]
    fn anchor_sizes_for_rows_and_variant_grid() {
        let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
        let rows_only = card_anchored(viewport, (100.0, 200.0), 3, 3);
        assert_eq!(rows_only.height, PAD_Y * 2.0 + 3.0 * ROW_HEIGHT);

        let mixed = card_anchored(viewport, (100.0, 200.0), 8, 2);
        assert_eq!(
            mixed.height,
            PAD_Y * 2.0 + 2.0 * ROW_HEIGHT + GRID_HEADER_HEIGHT + 2.0 * GRID_CELL_HEIGHT + GRID_GAP
        );
    }

    #[test]
    fn anchor_flips_and_clamps_at_viewport_edges() {
        let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
        let r = card_anchored(viewport, (700.0, 500.0), 8, 2);
        assert_eq!(r.x, 800.0 - CARD_W);
        assert_eq!(r.bottom(), 500.0);
    }

    #[test]
    fn hit_test_maps_rows_and_cells_to_one_flat_selection() {
        let card = card_anchored(Rect::new(0.0, 0.0, 800.0, 600.0), (100.0, 100.0), 8, 2);
        for index in 0..8 {
            let rect = item_rect(card, 8, 2, 0, index).unwrap();
            assert_eq!(
                item_at(
                    card,
                    8,
                    2,
                    0,
                    (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0)
                ),
                Some(index)
            );
        }
        let gap = item_rect(card, 8, 2, 0, 2).unwrap().right() + GRID_GAP / 2.0;
        assert_eq!(
            item_at(card, 8, 2, 0, (gap, item_rect(card, 8, 2, 0, 2).unwrap().y)),
            None
        );
    }

    #[test]
    fn new_clamps_selection_and_variant_boundary() {
        let menu = MathMenu::new(rows(3), 99, 99, 0, (0.0, 0.0));
        assert_eq!(menu.selected, 2);
        assert_eq!(menu.variant_start, 3);

        let menu = MathMenu::new(Vec::new(), 9, 9, 0, (0.0, 0.0));
        assert_eq!(menu.selected, 0);
        assert_eq!(menu.variant_start, 0);
    }

    #[test]
    fn scrolling_keeps_hits_stable_and_hidden_matches_unavailable() {
        let card = card_anchored(Rect::new(0.0, 0.0, 620.0, 484.0), (100.0, 250.0), 24, 11);
        let first = follow_selection(11, 8, 0);
        assert_eq!(first, 3);
        assert!(item_rect(card, 24, 11, first, 2).is_none());
        assert!(item_rect(card, 24, 11, first, 9).is_none());
        for index in 3..9 {
            let rect = item_rect(card, 24, 11, first, index).unwrap();
            assert_eq!(
                item_at(card, 24, 11, first, (rect.x + 10.0, rect.y + 10.0)),
                Some(index)
            );
            assert_eq!(follow_selection(11, index, first), first);
        }
        // Choosing any visible variant must not move the interpretations.
        assert_eq!(follow_selection(11, 23, first), first);
        assert_eq!(follow_selection(11, 2, first), 2);
    }

    #[test]
    fn every_catalog_prefix_fits_and_all_choices_can_be_reached() {
        use crate::document::{math, math_conversion, math_symbols};
        let viewport = Rect::new(0.0, 0.0, 620.0, 484.0);
        let names = math_symbols::SYMBOLS
            .iter()
            .map(|entry| entry.name)
            .chain(math::STRUCTURES.iter().map(|entry| entry.name));
        for name in names {
            for len in 1..=name.len() {
                let query = math_conversion::Query {
                    path: Vec::new(),
                    start: 0,
                    end: len,
                    source: name[..len].into(),
                };
                let offers = math_conversion::offers(&query).offers;
                let regular = offers
                    .iter()
                    .position(|offer| matches!(offer, math_conversion::Offer::Variant { .. }))
                    .unwrap_or(offers.len());
                for anchor in [(68.0, 250.0), (590.0, 470.0)] {
                    let card = card_anchored(viewport, anchor, offers.len(), regular);
                    assert!(
                        card.x >= viewport.x && card.right() <= viewport.right(),
                        "{}",
                        query.source
                    );
                    assert!(
                        card.y >= viewport.y && card.bottom() <= viewport.bottom(),
                        "{}",
                        query.source
                    );
                    let mut first = 0;
                    for index in (0..offers.len()).chain((0..offers.len()).rev()) {
                        first = follow_selection(regular, index, first);
                        let rect = item_rect(card, offers.len(), regular, first, index).unwrap();
                        assert!(rect.y >= card.y && rect.bottom() <= card.bottom());
                        let point = (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
                        assert_eq!(
                            item_at(card, offers.len(), regular, first, point),
                            Some(index)
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn dismissal_waits_for_the_real_viewport_and_preserves_scroll() {
        let menu = MathMenu::new(rows(24), 11, 8, 3, (100.0, 250.0));
        let mut ghost = MathMenu::dismissing(&menu.snapshot(), menu.anchor, 8);
        assert!(!ghost.started);
        ghost.sync(&Context {
            self_rect: Rect::new(0.0, 0.0, 620.0, 484.0),
            reveal: 0.5,
            ..Context::default()
        });
        assert!(ghost.started);
        assert_eq!(ghost.first_visible, 3);
        let card = card_anchored(Rect::new(0.0, 0.0, 620.0, 484.0), menu.anchor, 24, 11);
        assert_eq!(
            ghost.slide.rect(),
            inset_for_pill(item_rect(card, 24, 11, 3, 8).unwrap())
        );
    }
}
