//! The symbol inspector: the face the context menu wears over a symbol in
//! an expression.
//!
//! The list it replaced said "Variable", "Bold italic", "Sans serif" — the
//! names of things whose whole content is how they *look*. So this card
//! shows them instead: every cell draws the symbol as choosing that cell
//! would leave it. Roles carry their shape, hues carry their wash, variants
//! carry their own glyph, and the head shows what the symbol is right now.
//!
//! Geometry, drawing, and hit-testing live together here so a click can
//! never land on a cell the card drew somewhere else. The shell owns the
//! state and the keystrokes and hands down a flat list of sections, whose
//! cells are in exactly the order of the commands behind them — so the one
//! selection index means the same thing to both sides.
//!
//! Speaks the shared popup vocabulary — elevated surface springing from the
//! pointer, blurred shadow on the shell's layer, one sliding pill for
//! keyboard and mouse alike — and closes as a ghost of its real cells on
//! the shared clock, the same as every other menu.

use crate::document::math_paint::{HIGHLIGHT_EDGE, HIGHLIGHT_RADIUS};
use crate::document::math_style::SymbolStyle;
use crate::layout::Rect;
use crate::renderer::{Layer, Rounding};
use crate::theme::{self, TextStyle};
use crate::ui::{Component, Context, Dirty};

use super::popup::{MENU_SLIDE_EASING, Slide, paint_shadow_slab, revealed_card};

const CARD_W: f32 = 320.0;
const PAD_X: f32 = 10.0;
const PAD_Y: f32 = 8.0;
/// The head: the symbol as it stands, big enough to judge a colour by.
const HEAD_HEIGHT: f32 = 46.0;
const HEAD_GLYPH: f32 = 24.0;
/// A section's label strip.
const SECTION_HEADER: f32 = 20.0;
const ROW_HEIGHT: f32 = 28.0;
const GRID_GAP: f32 = 5.0;
/// Corner radius of a row highlight — nested smaller things round less than
/// the card itself (see `popup::CARD_RADIUS`).
const ROW_RADIUS: f32 = 6.0;
/// How far a swatch or glyph sits inside its cell, so the selection pill
/// reads as a frame around it rather than a fill behind it.
const CELL_INSET: f32 = 6.0;
/// The ring that marks the cell a symbol is already on.
const CHECK_RING: f32 = 1.5;

/// How a section arranges its cells.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Shelf {
    /// Full-width rows: a chip, a name, a check.
    Rows,
    /// A grid of `columns` cells, each `height` tall. What a choice that is
    /// purely visual wants — ten hues read as a palette, not as ten lines
    /// of text spelling colours out.
    Grid { columns: usize, height: f32 },
}

/// What a cell draws to show what choosing it would do.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Art {
    /// The symbol, highlighted the way this cell would highlight it.
    Preview(SymbolStyle),
    /// The glyph on its own — a variant is a different letterform, and
    /// wrapping it in a wash would hide the very thing being chosen.
    Bare,
}

/// One offer: a command, drawn as its own outcome.
#[derive(Clone, PartialEq, Debug)]
pub struct Cell {
    /// Shown in a row shelf; empty in a grid, where the art is the label.
    pub label: String,
    pub glyph: String,
    pub art: Art,
    /// Whether the symbol already stands here.
    pub checked: bool,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Section {
    pub title: String,
    pub shelf: Shelf,
    pub cells: Vec<Cell>,
}

impl Section {
    fn rows(&self) -> usize {
        match self.shelf {
            Shelf::Rows => self.cells.len(),
            Shelf::Grid { columns, .. } => self.cells.len().div_ceil(columns.max(1)),
        }
    }

    fn shelf_height(&self) -> f32 {
        let rows = self.rows();
        match self.shelf {
            Shelf::Rows => rows as f32 * ROW_HEIGHT,
            Shelf::Grid { height, .. } => {
                rows as f32 * height + rows.saturating_sub(1) as f32 * GRID_GAP
            }
        }
    }

    fn height(&self) -> f32 {
        SECTION_HEADER + self.shelf_height()
    }
}

/// The symbol the card is about, as it stands.
#[derive(Clone, PartialEq, Debug)]
pub struct Head {
    pub glyph: String,
    /// The identity the styling is keyed on — what the config file would
    /// call this symbol.
    pub name: String,
    /// What it plays: the role's own name.
    pub role: String,
    pub style: SymbolStyle,
}

/// Everything a dismissal ghost needs to redraw the card exactly as it
/// stood: cloneable real state, no placeholders.
#[derive(Clone)]
pub struct Snapshot {
    pub head: Head,
    pub sections: Vec<Section>,
    pub selected: usize,
}

pub fn card_height(sections: &[Section]) -> f32 {
    PAD_Y * 2.0 + HEAD_HEIGHT + sections.iter().map(Section::height).sum::<f32>()
}

/// The card for `sections`, with its top-left corner at `anchor`. Flips up
/// past the viewport's bottom and shifts left past its right edge, so a
/// menu opened near a corner stays fully on screen — the same rule every
/// other card follows.
pub fn card_anchored(viewport: Rect, anchor: (f32, f32), sections: &[Section]) -> Rect {
    let height = card_height(sections);
    // This card is the tallest in the application, and a window can be
    // shorter than it. `max` on each clamp's ceiling keeps a card that does
    // not fit anchored to the viewport's top-left and overflowing off the
    // far edge, where the region's scissor cuts it — rather than handing
    // `clamp` a range whose ends have crossed over, which panics.
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

/// The top of each section's label strip, in card coordinates.
fn section_tops(card: Rect, sections: &[Section]) -> Vec<f32> {
    let mut y = card.y + PAD_Y + HEAD_HEIGHT;
    sections
        .iter()
        .map(|section| {
            let top = y;
            y += section.height();
            top
        })
        .collect()
}

/// The exact rectangle a flat cell index is drawn in — the one geometry
/// both the painter and the hit-test read.
pub fn cell_rect(card: Rect, sections: &[Section], index: usize) -> Option<Rect> {
    let tops = section_tops(card, sections);
    let mut seen = 0;
    for (section, top) in sections.iter().zip(tops) {
        let local = index.checked_sub(seen)?;
        if local >= section.cells.len() {
            seen += section.cells.len();
            continue;
        }
        let shelf = top + SECTION_HEADER;
        return Some(match section.shelf {
            Shelf::Rows => Rect::new(
                card.x,
                shelf + local as f32 * ROW_HEIGHT,
                card.width,
                ROW_HEIGHT,
            ),
            Shelf::Grid { columns, height } => {
                let columns = columns.max(1);
                let width = (card.width - PAD_X * 2.0 + GRID_GAP + -GRID_GAP * columns as f32)
                    / columns as f32;
                Rect::new(
                    card.x + PAD_X + (local % columns) as f32 * (width + GRID_GAP),
                    shelf + (local / columns) as f32 * (height + GRID_GAP),
                    width,
                    height,
                )
            }
        });
    }
    None
}

fn cell_count(sections: &[Section]) -> usize {
    sections.iter().map(|section| section.cells.len()).sum()
}

/// The flat cell index under `point`, sharing [`cell_rect`] with drawing.
pub fn cell_at(card: Rect, sections: &[Section], point: (f32, f32)) -> Option<usize> {
    (0..cell_count(sections))
        .find(|&index| cell_rect(card, sections, index).is_some_and(|rect| rect.contains(point)))
}

/// A row or cell rect shrunk to the pill that rides under it — one rule for
/// both shelves, so the selection is the same object everywhere.
fn inset_for_pill(rect: Rect) -> Rect {
    Rect::new(
        rect.x + 3.0,
        rect.y + 2.0,
        rect.width - 6.0,
        rect.height - 4.0,
    )
}

pub struct SymbolMenu {
    head: Head,
    sections: Vec<Section>,
    selected: usize,
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

impl SymbolMenu {
    pub fn new(head: Head, sections: Vec<Section>, selected: usize, anchor: (f32, f32)) -> Self {
        let selected = selected.min(cell_count(&sections).saturating_sub(1));
        Self {
            head,
            sections,
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

    /// A dismissal ghost rebuilt from the state captured at close time. The
    /// pill is not parked here: card placement needs the real viewport, and
    /// the first `sync` is the first place that exists.
    pub fn dismissing(snap: &Snapshot, anchor: (f32, f32), pill_cell: usize) -> Self {
        Self {
            head: snap.head.clone(),
            selected: pill_cell.min(cell_count(&snap.sections).saturating_sub(1)),
            sections: snap.sections.clone(),
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
            head: Head {
                glyph: String::new(),
                name: String::new(),
                role: String::new(),
                style: SymbolStyle {
                    hue: crate::document::math_style::MathHue::Rose,
                    shape: crate::document::math_style::HighlightShape::Fill,
                },
            },
            sections: Vec::new(),
            selected: 0,
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

    /// The ghost-rebuild data for this snapshot.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            head: self.head.clone(),
            sections: self.sections.clone(),
            selected: self.selected,
        }
    }

    /// The cell the pill sits on — the shell reads this at close time.
    pub fn pill_cell(&self) -> Option<usize> {
        (cell_count(&self.sections) > 0).then_some(self.selected)
    }

    /// Where the pill is standing, for a rebuild to carry over.
    pub fn resting(&self) -> Option<Rect> {
        (self.open && self.started).then(|| self.slide.rect())
    }

    /// Starts this card's pill where the last one left off, so it slides to
    /// the new selection instead of teleporting.
    ///
    /// The shell rebuilds the whole card on every hover and every arrow key
    /// — the sections have to be recomputed anyway, since choosing a colour
    /// changes what every other cell previews — so continuity has to be
    /// handed forward explicitly. This is the only thing that needs to be.
    pub fn resuming(mut self, from: Option<Rect>) -> Self {
        if let Some(rect) = from {
            self.slide.park(rect);
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
            card_anchored(viewport, self.anchor, &self.sections),
            self.anchor,
            1.0,
        );
        paint_shadow_slab(&shadow, card, e);
    }
}

/// Draws `glyph` inside `rect` the way `style` would highlight it in the
/// document — the same radius rule and the same hairline pen, so a preview
/// and the expression behind it cannot disagree about what a choice looks
/// like.
fn chip(layer: &Layer, rect: Rect, glyph: &str, style: SymbolStyle, e: f32) {
    let radius = rect.width.min(rect.height) * HIGHLIGHT_RADIUS;
    if style.shape.fills() {
        layer.draw_rectangle(
            rect.position(),
            rect.size(),
            theme::fade(theme::math_fill(style.hue), e),
            Rounding::uniform(radius),
        );
    }
    if style.shape.outlines() {
        let inset = HIGHLIGHT_EDGE * 0.5;
        theme::rounded_outline(
            layer,
            rect.inset(inset),
            radius - inset,
            HIGHLIGHT_EDGE,
            theme::fade(theme::math_edge(style.hue), e),
        );
    }
    if !glyph.is_empty() {
        theme::draw(
            layer,
            glyph,
            (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0),
            &TextStyle::math(rect.height * 0.62, theme::fade(theme::ink(), e)),
            theme::CENTER,
        );
    }
}

impl Component for SymbolMenu {
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
        if !self.open || self.sections.is_empty() {
            return;
        }
        if reveal_changed && !(self.dismissing && self.reveal <= 0.0) {
            self.dirty.set();
        }
        // A ghost parks once with the real viewport and then freezes; a live
        // card's pill follows the selection the shell is driving.
        if let Some(rect) = cell_rect(
            card_anchored(context.self_rect, self.anchor, &self.sections),
            &self.sections,
            self.selected,
        ) {
            let target = inset_for_pill(rect);
            if !self.started {
                self.slide.park(target);
                self.started = true;
                self.dirty.set();
            } else if !self.dismissing && self.slide.slide_to(target) {
                self.dirty.set();
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
        if !self.open || self.sections.is_empty() {
            return;
        }

        let e = MENU_SLIDE_EASING.apply(self.reveal);
        let resting = card_anchored(viewport, self.anchor, &self.sections);
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

        self.draw_head(layer, card, ea);

        // The pill rides under the selected cell, and every shelf inherits
        // it — one highlight for pointer and keyboard, on rows and grids
        // alike.
        if self.started {
            let pill = self.slide.rect();
            layer.draw_rectangle(
                pill.position(),
                pill.size(),
                theme::fade(theme::selection(), ea),
                Rounding::uniform(ROW_RADIUS),
            );
        }

        let label_style = TextStyle::sans(11.5, theme::fade(theme::comment(), ea));
        let name_style = TextStyle::sans(13.0, theme::fade(theme::ink(), ea));
        let mut index = 0;
        for (section, top) in self.sections.iter().zip(section_tops(card, &self.sections)) {
            theme::draw(
                layer,
                &section.title.to_uppercase(),
                (card.x + PAD_X + 4.0, top + SECTION_HEADER / 2.0),
                &label_style,
                theme::LEFT,
            );
            for cell in &section.cells {
                let rect = cell_rect(card, &self.sections, index)
                    .expect("a listed cell must have geometry");
                match section.shelf {
                    Shelf::Rows => self.draw_row(layer, rect, cell, &name_style, ea),
                    Shelf::Grid { .. } => self.draw_grid_cell(layer, rect, cell, ea),
                }
                index += 1;
            }
        }
    }
}

impl SymbolMenu {
    /// The symbol as it stands: its own highlight at reading size, the
    /// identity its styling is stored under, and the role it plays.
    fn draw_head(&self, layer: &Layer, card: Rect, ea: f32) {
        let box_ = Rect::new(
            card.x + PAD_X + 4.0,
            card.y + PAD_Y + (HEAD_HEIGHT - HEAD_GLYPH - 8.0) / 2.0,
            HEAD_GLYPH + 12.0,
            HEAD_GLYPH + 8.0,
        );
        chip(layer, box_, &self.head.glyph, self.head.style, ea);
        let middle = card.y + PAD_Y + HEAD_HEIGHT / 2.0;
        theme::draw(
            layer,
            &self.head.name,
            (box_.right() + 12.0, middle - 7.0),
            &TextStyle::sans(13.0, theme::fade(theme::ink(), ea)),
            theme::LEFT,
        );
        theme::draw(
            layer,
            &self.head.role,
            (box_.right() + 12.0, middle + 8.0),
            &TextStyle::sans(11.0, theme::fade(theme::comment(), ea)),
            theme::LEFT,
        );
        theme::rule(
            layer,
            (card.x + PAD_X, card.y + PAD_Y + HEAD_HEIGHT - 1.0),
            card.width - PAD_X * 2.0,
            1.0,
            theme::fade(theme::border(), ea),
        );
    }

    fn draw_row(&self, layer: &Layer, rect: Rect, cell: &Cell, name: &TextStyle, ea: f32) {
        let chip_box = Rect::new(rect.x + 14.0, rect.y + 5.0, 26.0, ROW_HEIGHT - 10.0);
        match cell.art {
            Art::Preview(style) => chip(layer, chip_box, &cell.glyph, style, ea),
            Art::Bare => theme::draw(
                layer,
                &cell.glyph,
                (
                    chip_box.x + chip_box.width / 2.0,
                    rect.y + rect.height / 2.0,
                ),
                &TextStyle::math(15.0, theme::fade(theme::ink(), ea)),
                theme::CENTER,
            ),
        }
        theme::draw(
            layer,
            &cell.label,
            (chip_box.right() + 12.0, rect.y + rect.height / 2.0),
            name,
            theme::LEFT,
        );
        if cell.checked {
            theme::icon(
                layer,
                theme::icons::CHECK,
                (rect.right() - 26.0, rect.y + rect.height / 2.0 - 7.0),
                14.0,
                theme::fade(theme::accent(), ea),
                1.8,
            );
        }
    }

    fn draw_grid_cell(&self, layer: &Layer, rect: Rect, cell: &Cell, ea: f32) {
        let inner = rect.inset(CELL_INSET);
        match cell.art {
            Art::Preview(style) => chip(layer, inner, &cell.glyph, style, ea),
            Art::Bare => {
                layer.draw_rectangle(
                    inner.position(),
                    inner.size(),
                    theme::fade(theme::alt(), ea),
                    Rounding::uniform(5.0),
                );
                theme::draw(
                    layer,
                    &cell.glyph,
                    (inner.x + inner.width / 2.0, inner.y + inner.height / 2.0),
                    &TextStyle::math(17.0, theme::fade(theme::ink(), ea)),
                    theme::CENTER,
                );
            }
        }
        if cell.checked {
            // A ring rather than a tick: at this size a glyph would cover
            // the very swatch it is marking. Drawn on the swatch's own
            // border, inside the selection pill's halo, so a cell that is
            // both current and selected shows two rings rather than one
            // ambiguous one.
            theme::rounded_outline(
                layer,
                inner,
                inner.width.min(inner.height) * HIGHLIGHT_RADIUS,
                CHECK_RING,
                theme::fade(theme::accent(), ea),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::math_style::{HighlightShape, MathHue};

    fn style() -> SymbolStyle {
        SymbolStyle {
            hue: MathHue::Teal,
            shape: HighlightShape::Fill,
        }
    }

    fn cells(count: usize) -> Vec<Cell> {
        (0..count)
            .map(|index| Cell {
                label: format!("cell{index}"),
                glyph: "x".into(),
                art: Art::Preview(style()),
                checked: false,
            })
            .collect()
    }

    fn sections() -> Vec<Section> {
        vec![
            Section {
                title: "Role".into(),
                shelf: Shelf::Rows,
                cells: cells(3),
            },
            Section {
                title: "Colour".into(),
                shelf: Shelf::Grid {
                    columns: 5,
                    height: 28.0,
                },
                cells: cells(10),
            },
        ]
    }

    #[test]
    fn a_card_reserves_its_head_and_every_section() {
        let sections = sections();
        let card = card_anchored(Rect::new(0.0, 0.0, 900.0, 900.0), (40.0, 40.0), &sections);

        assert_eq!(
            card.height,
            PAD_Y * 2.0
                + HEAD_HEIGHT
                + (SECTION_HEADER + 3.0 * ROW_HEIGHT)
                + (SECTION_HEADER + 2.0 * 28.0 + GRID_GAP)
        );
        assert_eq!(card.width, CARD_W);
    }

    /// One flat index across shelves of different shapes — the property the
    /// shell's single `selected` depends on.
    #[test]
    fn every_cell_hit_tests_back_to_its_own_index() {
        let sections = sections();
        let card = card_anchored(Rect::new(0.0, 0.0, 900.0, 900.0), (40.0, 40.0), &sections);

        for index in 0..13 {
            let rect = cell_rect(card, &sections, index).expect("every cell has geometry");
            let centre = (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
            assert_eq!(cell_at(card, &sections, centre), Some(index));
        }
        assert_eq!(cell_rect(card, &sections, 13), None);
    }

    /// A grid row must be inside the card and inside its own section, or a
    /// swatch would be drawn over the section header below it.
    #[test]
    fn a_grid_stays_inside_its_section() {
        let sections = sections();
        let card = card_anchored(Rect::new(0.0, 0.0, 900.0, 900.0), (40.0, 40.0), &sections);
        let tops = section_tops(card, &sections);

        let first = cell_rect(card, &sections, 3).expect("first swatch");
        let last = cell_rect(card, &sections, 12).expect("last swatch");
        assert!(first.y >= tops[1] + SECTION_HEADER);
        assert!(last.bottom() <= card.bottom() - PAD_Y + 0.001);
        assert!(first.x >= card.x + PAD_X);
        assert!(cell_rect(card, &sections, 7).unwrap().right() <= card.right() - PAD_X + 0.001);
    }

    #[test]
    fn the_gap_between_two_swatches_belongs_to_neither() {
        let sections = sections();
        let card = card_anchored(Rect::new(0.0, 0.0, 900.0, 900.0), (40.0, 40.0), &sections);
        let first = cell_rect(card, &sections, 3).expect("first swatch");

        let gap = (first.right() + GRID_GAP / 2.0, first.y + first.height / 2.0);
        assert_eq!(cell_at(card, &sections, gap), None);
    }

    /// A window can be shorter than this card, and `clamp` panics on a
    /// range whose ends have crossed. The card gives up on fitting rather
    /// than taking the window down with it.
    #[test]
    fn a_card_taller_than_the_window_anchors_to_the_top() {
        let sections = sections();
        let viewport = Rect::new(0.0, 0.0, 200.0, 80.0);
        let card = card_anchored(viewport, (150.0, 60.0), &sections);

        assert_eq!(card.position(), (viewport.x, viewport.y));
        assert_eq!(card.height, card_height(&sections));
    }

    #[test]
    fn a_card_near_the_bottom_right_stays_on_screen() {
        let sections = sections();
        let viewport = Rect::new(0.0, 0.0, 400.0, 500.0);
        let card = card_anchored(viewport, (390.0, 490.0), &sections);

        assert!(card.right() <= viewport.right());
        assert!(card.bottom() <= viewport.bottom());
        assert!(card.x >= viewport.x && card.y >= viewport.y);
    }

    #[test]
    fn new_clamps_a_selection_past_the_last_cell() {
        let menu = SymbolMenu::new(
            Head {
                glyph: "x".into(),
                name: "x".into(),
                role: "Variable".into(),
                style: style(),
            },
            sections(),
            99,
            (0.0, 0.0),
        );

        assert_eq!(menu.selected, 12);
        assert_eq!(menu.pill_cell(), Some(12));
    }
}
