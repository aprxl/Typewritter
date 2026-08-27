//! The command palette: one card over everything, a query and a ranked list.
//!
//! Same shape as [`Dialog`](super::Dialog) and for the same reason — the
//! shell is the one taking keystrokes (it owns the query text and the
//! selection index), so this component holds a snapshot and draws it. It
//! never mutates anything and never decides what is selected.

use crate::layout::Rect;
use crate::renderer::{Layer, Rounding};
use crate::theme::{self, TextStyle};
use crate::ui::{Component, Context, Dirty};

const CARD_W: f32 = 560.0;
/// Query line plus the rule beneath it.
const QUERY_H: f32 = 56.0;
const ROW_HEIGHT: f32 = 34.0;
/// The "↑↓ select · Enter runs · Esc closes" line.
const FOOTER_H: f32 = 34.0;
/// Tall enough for about ten rows. `MAX_ROWS` is derived from this, not the
/// other way round, so the two numbers cannot drift apart.
const CARD_H: f32 = 430.0;
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
    dirty: Dirty,
}

impl Palette {
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
            dirty: Dirty::new(),
        }
    }

    /// Closed. Draws nothing and never dirties itself.
    pub fn closed() -> Self {
        Self {
            entries: Vec::new(),
            visible: Vec::new(),
            query: String::new(),
            selected: 0,
            first_visible: 0,
            caret_on: true,
            open: false,
            dirty: Dirty::new(),
        }
    }
}

impl Component for Palette {
    fn measure(&mut self, _: &Layer) -> (f32, f32) {
        // An overlay bound to the viewport; it asks the layout for nothing.
        (0.0, 0.0)
    }

    fn sync(&mut self, context: &Context) {
        // A closed palette draws nothing, so tracking the caret would only
        // ever mark it dirty for a redraw nobody sees.
        if self.open {
            self.dirty.write(&mut self.caret_on, context.caret_on);
        }
    }

    fn is_dirty(&self) -> bool {
        self.dirty.get()
    }

    fn clear_dirty(&mut self) {
        self.dirty.clear();
    }

    fn draw(&mut self, layer: &Layer, rect: Rect) {
        if !self.open {
            return;
        }
        layer.draw_rectangle(
            rect.position(),
            rect.size(),
            theme::fade(theme::background(), 0.72),
            Rounding::NONE,
        );

        let card = card(rect);
        layer.draw_rectangle(card.position(), card.size(), theme::popup(), Rounding::NONE);
        theme::outline(layer, card, theme::border());

        self.draw_query(layer, card);
        theme::rule(
            layer,
            (card.x + 20.0, card.y + QUERY_H),
            card.width - 40.0,
            1.0,
            theme::border(),
        );

        if self.visible.is_empty() {
            theme::draw(
                layer,
                "No matching command",
                (card.x + card.width / 2.0, card.y + QUERY_H + 32.0),
                &TextStyle::serif(13.5, theme::faint()),
                theme::CENTER,
            );
        } else {
            self.draw_rows(layer, card);
        }

        theme::draw(
            layer,
            "\u{2191}\u{2193} select \u{b7} Enter runs \u{b7} Esc closes",
            (card.x + 20.0, card.bottom() - FOOTER_H / 2.0),
            &TextStyle::mono(10.0, theme::faint()),
            theme::LEFT,
        );
    }
}

impl Palette {
    fn draw_query(&self, layer: &Layer, card: Rect) {
        let style = TextStyle::serif(16.0, theme::ink());
        let middle = card.y + QUERY_H / 2.0;
        if self.query.is_empty() {
            theme::draw(
                layer,
                "Type a command",
                (card.x + 20.0, middle),
                &style.clone().color(theme::faint()),
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
                theme::accent(),
                Rounding::NONE,
            );
        }
    }

    fn draw_rows(&self, layer: &Layer, card: Rect) {
        let title_style = TextStyle::serif(15.0, theme::ink());
        let group_style = TextStyle::serif(11.5, theme::comment());
        let hint_style = TextStyle::mono(10.5, theme::faint());

        let window = self.visible[self.first_visible..]
            .iter()
            .take(MAX_ROWS)
            .enumerate();
        for (offset, &entry_index) in window {
            let entry = &self.entries[entry_index];
            let row = Rect::new(
                card.x,
                card.y + QUERY_H + offset as f32 * ROW_HEIGHT,
                card.width,
                ROW_HEIGHT,
            );
            let middle = row.y + row.height / 2.0;

            if self.first_visible + offset == self.selected {
                layer.draw_rectangle(
                    row.position(),
                    row.size(),
                    theme::selection(),
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
