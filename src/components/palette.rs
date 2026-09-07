//! The command palette: one card over everything, a query and a ranked list.
//!
//! Same shape as [`Dialog`](super::Dialog) and for the same reason — the
//! shell is the one taking keystrokes (it owns the query text and the
//! selection index), so this component holds a snapshot and draws it. It
//! never mutates anything and never decides what is selected.

use crate::layout::Rect;
use crate::renderer::{Layer, Rounding};
use crate::theme::{self, TextStyle};
use crate::ui::{Component, Context, Dirty, Hover};

use super::popup::paint_shadow_slab;

const ROW_RADIUS: f32 = 6.0;
const CARD_W: f32 = 600.0;
/// Query line plus the rule beneath it.
const QUERY_H: f32 = 64.0;
const ROW_HEIGHT: f32 = 38.0;
/// The "↑↓ select · Enter runs · Esc closes" line.
const FOOTER_H: f32 = 44.0;
/// Tall enough for about ten rows. `MAX_ROWS` is derived from this, not the
/// other way round, so the two numbers cannot drift apart.
const CARD_H: f32 = 456.0;
const MAX_ROWS: usize = ((CARD_H - QUERY_H - FOOTER_H) / ROW_HEIGHT) as usize;

/// One command as the palette shows it.
#[derive(Clone)]
pub struct Entry {
    pub title: String,
    /// "File", "View", "Edit" — drawn dimmed after the title.
    pub group: String,
    /// The keybinding, e.g. "Ctrl+N". Empty when the command has none.
    pub hint: String,
}

/// Indices of `entries` matching `query`, best first.
///
/// A match is a case-insensitive subsequence of `"{title} {group}"` —
/// letters in order, not necessarily touching. Scoring is deliberately two
/// numbers rather than a formula: a contiguous run beats a scattered one,
/// and an earlier first match beats a later one. A real fuzzy matcher
/// (fzf-style scoring) is a dependency decision for later; this is the
/// upgrade path once "type a few letters" stops being enough.
pub fn filter(entries: &[Entry], query: &str) -> Vec<usize> {
    if query.is_empty() {
        return (0..entries.len()).collect();
    }
    let needle: Vec<char> = query.to_lowercase().chars().collect();

    let mut scored: Vec<(usize, bool, usize)> = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            let haystack: Vec<char> = format!("{} {}", entry.title, entry.group)
                .to_lowercase()
                .chars()
                .collect();
            let first = subsequence_start(&haystack, &needle)?;
            let scattered = !haystack[first..].starts_with(needle.as_slice());
            Some((index, scattered, first))
        })
        .collect();

    // Stable sort: ties keep the order `entries` was given in.
    scored.sort_by_key(|&(_, scattered, first)| (scattered, first));
    scored.into_iter().map(|(index, _, _)| index).collect()
}

/// The index `needle`'s first character occupies in `haystack`, if `needle`
/// occurs there as a subsequence.
fn subsequence_start(haystack: &[char], needle: &[char]) -> Option<usize> {
    let mut first = None;
    let mut rest = needle.iter();
    let mut want = rest.next();
    for (index, ch) in haystack.iter().enumerate() {
        let Some(&w) = want else { break };
        if *ch == w {
            first.get_or_insert(index);
            want = rest.next();
        }
    }
    if want.is_none() { first } else { None }
}

pub fn card(viewport: Rect) -> Rect {
    Rect::new(
        viewport.x + (viewport.width - CARD_W) / 2.0,
        viewport.y + (viewport.height - CARD_H) / 2.0,
        CARD_W,
        CARD_H,
    )
}

pub struct Palette {
    entries: Vec<Entry>,
    /// Indices into `entries`, already ranked by `filter`.
    visible: Vec<usize>,
    query: String,
    /// Indexes `visible`, not `entries`.
    selected: usize,
    /// First row of `visible` drawn, so the window follows `selected`.
    first_visible: usize,
    caret_on: bool,
    open: bool,
    /// The blurred layer this card paints its shadow slab into.
    shadow: Option<Layer>,
    reveal: f32,
    hovered: Option<usize>,
    hover: Hover,
    dirty: Dirty,
}

impl Palette {
    /// The command under the pointer, using the same rows as the painter.
    pub fn row_at(&self, viewport: Rect, point: (f32, f32)) -> Option<usize> {
        if !self.open {
            return None;
        }
        (0..self
            .visible
            .len()
            .saturating_sub(self.first_visible)
            .min(MAX_ROWS))
            .find(|&row| row_rect(card(viewport), row).contains(point))
            .map(|row| self.first_visible + row)
    }

    /// `selected` indexes into the *filtered* list.
    pub fn new(entries: Vec<Entry>, query: String, selected: usize) -> Self {
        let visible = filter(&entries, &query);
        let selected = if visible.is_empty() {
            0
        } else {
            selected.min(visible.len() - 1)
        };
        // Scroll only when the selection has run past the window; a
        // selection still inside it leaves the window exactly where it was.
        let first_visible = selected.saturating_sub(MAX_ROWS.saturating_sub(1));
        Self {
            entries,
            visible,
            query,
            selected,
            first_visible,
            caret_on: true,
            open: true,
            shadow: None,
            reveal: 0.0,
            hovered: None,
            hover: Hover::new(),
            dirty: Dirty::new(),
        }
    }

    /// Closed. Draws nothing and never dirties itself. Modals close
    /// instantly — no ghost (see the toolkit's motion table).
    pub fn closed() -> Self {
        Self {
            entries: Vec::new(),
            visible: Vec::new(),
            query: String::new(),
            selected: 0,
            first_visible: 0,
            caret_on: true,
            open: false,
            shadow: None,
            reveal: 1.0,
            hovered: None,
            hover: Hover::new(),
            dirty: Dirty::new(),
        }
    }

    /// Attaches the shell's blurred shadow layer.
    pub fn with_shadow(mut self, shadow: Layer) -> Self {
        self.shadow = Some(shadow);
        self
    }

    /// Paints the shadow slab from sync. A modal's halo fades in with the
    /// card; a closed palette clears the layer unconditionally so the last
    /// frame's halo always leaves with it.
    fn paint_shadow(&mut self, owns: bool, viewport: Rect) {
        if !owns {
            return;
        }
        let Some(shadow) = self.shadow.clone() else {
            return;
        };
        if !self.open || !(0.0..=1.0).contains(&self.reveal) {
            paint_shadow_slab(&shadow, Rect::default(), -1.0);
            return;
        }
        paint_shadow_slab(&shadow, card(viewport), self.reveal);
    }
}

impl Component for Palette {
    fn measure(&mut self, _: &Layer) -> (f32, f32) {
        // An overlay bound to the viewport; it asks the layout for nothing.
        (0.0, 0.0)
    }

    fn sync(&mut self, context: &Context) {
        // A closed palette still clears the shadow layer (the last halo has
        // to leave with it), so paint_shadow runs unconditionally — but the
        // caret is only tracked while anyone can see it.
        let reveal_changed = self.reveal != context.reveal;
        if reveal_changed {
            self.reveal = context.reveal;
            self.dirty.set();
        }
        self.paint_shadow(context.owns_shadow, context.self_rect);
        if self.open {
            self.dirty.write(&mut self.caret_on, context.caret_on);
            let hovered = context
                .mouse
                .in_window
                .then(|| self.row_at(context.self_rect, context.mouse.position))
                .flatten();
            if self
                .hover
                .track(&mut self.hovered, hovered, context.animation_dt)
            {
                self.dirty.set();
            }
        }
    }

    fn is_dirty(&self) -> bool {
        self.dirty.get()
    }

    fn clear_dirty(&mut self) {
        self.dirty.clear()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn is_animating(&self) -> bool {
        self.open && (self.hover.is_animating() || (self.reveal > 0.0 && self.reveal < 1.0))
    }

    fn draw(&mut self, layer: &Layer, rect: Rect) {
        if !self.open {
            return;
        }
        // The dim sits under everything and fades in with the card.
        let ea = self.reveal.clamp(0.0, 1.0);
        layer.draw_rectangle(
            rect.position(),
            rect.size(),
            theme::fade(theme::background(), 0.48 * ea),
            Rounding::NONE,
        );

        let resting = card(rect);
        // A modal does not scale or overshoot — a palette-sized surface
        // wobbling reads as lag. It just fades up (and the rounded surface
        // rises 8px into place on the same curve).
        let travel = (1.0 - ea) * 8.0;
        let card = Rect::new(resting.x, resting.y + travel, resting.width, resting.height);

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

        theme::rule(
            layer,
            (card.x + 20.0, card.bottom() - FOOTER_H),
            card.width - 40.0,
            1.0,
            theme::fade(theme::border(), ea),
        );
        let y = card.bottom() - FOOTER_H / 2.0;
        theme::draw(
            layer,
            "COMMANDS",
            (card.x + 22.0, y),
            &TextStyle::sans(9.0, theme::fade(theme::faint(), ea)).tracked(0.1),
            theme::LEFT,
        );
        let x = card.right() - 245.0;
        theme::keycap(layer, "↑ ↓", (x, y), ea);
        theme::draw(
            layer,
            "Navigate",
            (x + 41.0, y),
            &TextStyle::sans(10.5, theme::fade(theme::faint(), ea)),
            theme::LEFT,
        );
        theme::keycap(layer, "Enter", (x + 110.0, y), ea);
        theme::draw(
            layer,
            "Run",
            (x + 156.0, y),
            &TextStyle::sans(10.5, theme::fade(theme::faint(), ea)),
            theme::LEFT,
        );
    }
}

/// The rect of the `row`th drawn row inside `card` — shared by the pill and
/// the row loop.
fn row_rect(card: Rect, row: usize) -> Rect {
    Rect::new(
        card.x,
        card.y + QUERY_H + row as f32 * ROW_HEIGHT,
        card.width,
        ROW_HEIGHT,
    )
}

impl Palette {
    fn draw_query(&self, layer: &Layer, card: Rect, ea: f32) {
        let style = TextStyle::sans(16.0, theme::fade(theme::ink(), ea));
        let middle = card.y + QUERY_H / 2.0;
        theme::icon(
            layer,
            theme::icons::SEARCH,
            (card.x + 23.0, middle - 8.0),
            16.0,
            theme::fade(theme::accent(), ea),
            1.6,
        );
        let query = theme::elide(layer, &self.query, card.width - 130.0, &style);
        theme::draw(
            layer,
            if self.query.is_empty() {
                "What would you like to do?"
            } else {
                &query
            },
            (card.x + 52.0, middle),
            &style.clone().color(if self.query.is_empty() {
                theme::fade(theme::faint(), ea)
            } else {
                style.color.clone()
            }),
            theme::LEFT,
        );
        theme::keycap(layer, "Esc", (card.right() - 54.0, middle), ea);
        if self.caret_on {
            let x = card.x + 52.0 + theme::width(layer, &query, &style);
            layer.draw_rectangle(
                (x, middle - 10.0),
                (1.5, 20.0),
                theme::fade(theme::accent(), ea),
                Rounding::uniform(0.75),
            );
        }
    }

    fn draw_rows(&self, layer: &Layer, card: Rect, ea: f32) {
        let title_style = TextStyle::sans(13.0, theme::fade(theme::ink(), ea));
        let group_style = TextStyle::sans(10.5, theme::fade(theme::faint(), ea));
        for (offset, &entry_index) in self.visible[self.first_visible..]
            .iter()
            .take(MAX_ROWS)
            .enumerate()
        {
            let entry = &self.entries[entry_index];
            let row = row_rect(card, offset);
            let middle = row.y + row.height / 2.0;
            if self.first_visible + offset == self.selected {
                layer.draw_rectangle(
                    (row.x + 8.0, row.y + 2.0),
                    (row.width - 16.0, row.height - 4.0),
                    theme::fade(theme::selection(), ea),
                    Rounding::uniform(ROW_RADIUS),
                );
            } else if self.hovered == Some(self.first_visible + offset) {
                theme::hover_fill(layer, row.inset(4.0), self.hover.value() * ea);
            }
            let icon = match entry.group.as_str() {
                "File" => theme::icons::FILE_LINES,
                "View" => theme::icons::SIDEBAR,
                "Vault" => theme::icons::FOLDER,
                "Edit" => theme::icons::NOTE_MARK,
                _ => theme::icons::TOPICS,
            };
            theme::icon(
                layer,
                icon,
                (row.x + 22.0, middle - 7.0),
                14.0,
                theme::fade(theme::dim(), ea),
                1.5,
            );
            let label = theme::elide(layer, &entry.title, row.width - 230.0, &title_style);
            theme::draw(
                layer,
                &label,
                (row.x + 50.0, middle),
                &title_style,
                theme::LEFT,
            );
            theme::draw(
                layer,
                &entry.group,
                (row.right() - 150.0, middle),
                &group_style,
                theme::RIGHT,
            );
            if !entry.hint.is_empty() {
                let width =
                    theme::width(layer, &entry.hint, &TextStyle::sans(11.0, theme::dim())) + 12.0;
                theme::keycap(layer, &entry.hint, (row.right() - 20.0 - width, middle), ea);
            }
        }
    }
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
    fn clicking_a_scrolled_command_uses_its_filtered_index() {
        let entries = (0..25)
            .map(|i| entry(&format!("Command {i}"), "View"))
            .collect();
        let palette = Palette::new(entries, String::new(), 20);
        let viewport = Rect::new(30.0, 50.0, 1280.0, 800.0);
        let row = row_rect(card(viewport), MAX_ROWS - 1);
        assert_eq!(
            palette.row_at(viewport, (row.x + 20.0, row.y + 15.0)),
            Some(20)
        );
        assert_eq!(
            palette.row_at(viewport, (row.x + 20.0, card(viewport).y + 20.0)),
            None
        );
        assert_eq!(
            Palette::closed().row_at(viewport, (row.x + 20.0, row.y + 15.0)),
            None
        );
    }

    #[test]
    fn an_empty_query_returns_every_entry_in_order() {
        let entries = vec![entry("New note", "File"), entry("Delete note", "File")];
        assert_eq!(filter(&entries, ""), vec![0, 1]);
    }

    #[test]
    fn a_contiguous_match_outranks_a_scattered_one() {
        // "cat" is a subsequence of "Coat" (c..a.t, skipping the 'o') but a
        // contiguous run in "Cats".
        let entries = vec![entry("Coat", "Clothing"), entry("Cats", "Animals")];
        assert_eq!(filter(&entries, "cat"), vec![1, 0]);
    }

    #[test]
    fn a_query_matches_against_the_group_as_well_as_the_title() {
        let entries = vec![entry("Save", "File"), entry("Undo", "Edit")];
        assert_eq!(filter(&entries, "file"), vec![0]);
    }

    #[test]
    fn filtering_is_case_insensitive() {
        let entries = vec![entry("New Note", "File")];
        assert_eq!(filter(&entries, "NEW"), vec![0]);
    }

    #[test]
    fn no_match_returns_an_empty_list() {
        let entries = vec![entry("New note", "File")];
        assert!(filter(&entries, "zzz").is_empty());
    }
}
