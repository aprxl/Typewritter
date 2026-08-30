//! The vault finder: one centered card over everything, Telescope-style.
//!
//! A query line, a result list on the left, a rendered preview of the
//! selected row on the right. The shell owns the query text and the
//! selection (it is the one taking keystrokes), so this component holds a
//! snapshot and draws it — the same shape as [`Palette`](super::Palette).
//!
//! Results are files and text hits in one ranked list (`search::search`).
//! `Ctrl+1..5` pick one of the first five rows outright; the shell claims
//! those chords before this panel is even drawn, so it only ever needs to
//! render the hints.

use crate::layout::Rect;
use crate::renderer::{Layer, Rounding};
use crate::search::Row;
use crate::theme::{self, TextStyle};
use crate::ui::{Component, Context, Dirty};

use super::popup::{CARD_RADIUS, paint_shadow_slab};

const ROW_RADIUS: f32 = 6.0;
/// Query line plus the rule beneath it.
const QUERY_H: f32 = 56.0;
const ROW_HEIGHT: f32 = 34.0;
/// The "↑↓ select · ↵ open · esc closes" line.
const FOOTER_H: f32 = 34.0;
/// The result list's width; the preview takes the rest of the card.
const LIST_W: f32 = 340.0;
/// Tall enough for about nine rows. `MAX_ROWS` is derived from this, not
/// the other way round, so the two numbers cannot drift apart.
const CARD_H: f32 = 420.0;
const MAX_ROWS: usize = ((CARD_H - QUERY_H - FOOTER_H) / ROW_HEIGHT) as usize;
const CARD_W: f32 = LIST_W + 360.0;

/// The rect the card rests at, centered in `viewport`.
pub fn card(viewport: Rect) -> Rect {
    Rect::new(
        viewport.x + (viewport.width - CARD_W) / 2.0,
        viewport.y + (viewport.height - CARD_H) / 2.0,
        CARD_W,
        CARD_H,
    )
}

pub struct Finder {
    rows: Vec<Row>,
    /// Indexes `rows`.
    selected: usize,
    /// First row of `rows` drawn, so the window follows `selected`.
    first_visible: usize,
    query: String,
    caret_on: bool,
    open: bool,
    /// The blurred layer this card paints its shadow slab into.
    shadow: Option<Layer>,
    reveal: f32,
    dirty: Dirty,
}

impl Finder {
    /// `selected` indexes into the *filtered* list.
    pub fn new(rows: Vec<Row>, query: String, selected: usize) -> Self {
        let selected = selected.min(rows.len().saturating_sub(1));
        // Scroll only when the selection has run past the window; a
        // selection still inside it leaves the window exactly where it was.
        let first_visible = selected.saturating_sub(MAX_ROWS.saturating_sub(1));
        Self {
            rows,
            selected,
            first_visible,
            query,
            caret_on: true,
            open: true,
            shadow: None,
            reveal: 0.0,
            dirty: Dirty::new(),
        }
    }

    /// Closed. Draws nothing and never dirties itself. Modals close
    /// instantly — no ghost (see the toolkit's motion table).
    pub fn closed() -> Self {
        Self {
            rows: Vec::new(),
            selected: 0,
            first_visible: 0,
            query: String::new(),
            caret_on: true,
            open: false,
            shadow: None,
            reveal: 1.0,
            dirty: Dirty::new(),
        }
    }

    /// Attaches the shell's blurred shadow layer.
    pub fn with_shadow(mut self, shadow: Layer) -> Self {
        self.shadow = Some(shadow);
        self
    }

    /// Paints the shadow slab from sync. A modal's halo fades in with the
    /// card; a closed finder clears the layer unconditionally so the last
    /// frame's halo always leaves with it.
    fn paint_shadow(&mut self, owns: bool) {
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
        paint_shadow_slab(&shadow, card(Rect::default()), self.reveal);
    }
}

impl Component for Finder {
    fn measure(&mut self, _: &Layer) -> (f32, f32) {
        // An overlay bound to the viewport; it asks the layout for nothing.
        (0.0, 0.0)
    }

    fn sync(&mut self, context: &Context) {
        // A closed finder still clears the shadow layer (the last halo has
        // to leave with it), so paint_shadow runs unconditionally — but the
        // caret is only tracked while anyone can see it.
        let reveal_changed = self.reveal != context.reveal;
        if reveal_changed {
            self.reveal = context.reveal;
            self.dirty.set();
        }
        self.paint_shadow(context.owns_shadow);
        if self.open {
            self.dirty.write(&mut self.caret_on, context.caret_on);
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
        self.open && self.reveal > 0.0 && self.reveal < 1.0
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
            theme::fade(theme::background(), 0.72 * ea),
            Rounding::NONE,
        );

        let resting = card(rect);
        // A modal does not scale or overshoot — a finder-sized surface
        // wobbling reads as lag. It just fades up (and the rounded surface
        // rises 4px into place on the same curve).
        let travel = (1.0 - ea) * -4.0;
        let card = Rect::new(resting.x, resting.y + travel, resting.width, resting.height);

        layer.draw_rectangle(
            card.position(),
            card.size(),
            theme::elevated_popup(ea),
            Rounding::uniform(CARD_RADIUS),
        );
        theme::rounded_outline(
            layer,
            card.inset(0.5),
            CARD_RADIUS - 0.5,
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

        if self.rows.is_empty() {
            self.draw_empty(layer, card, ea);
        } else {
            self.draw_list(layer, card, ea);
            self.draw_preview(layer, card, ea);
        }
        // The rule between the list and the preview.
        theme::rule(
            layer,
            (card.x + LIST_W, card.y + QUERY_H),
            1.0,
            CARD_H - QUERY_H - FOOTER_H,
            theme::fade(theme::border(), ea),
        );

        theme::draw(
            layer,
            "\u{2191}\u{2193} select \u{b7} \u{21b5} open \u{b7} ctrl+1-5 pick \u{b7} esc closes",
            (card.x + 20.0, card.bottom() - FOOTER_H / 2.0),
            &TextStyle::mono(10.0, theme::fade(theme::faint(), ea)),
            theme::LEFT,
        );
    }
}

/// The rect of the `row`th drawn row in the list column inside `card` —
/// shared by the selection pill and the row loop.
fn row_rect(card: Rect, row: usize) -> Rect {
    Rect::new(
        card.x,
        card.y + QUERY_H + row as f32 * ROW_HEIGHT,
        LIST_W,
        ROW_HEIGHT,
    )
}

impl Finder {
    fn draw_query(&self, layer: &Layer, card: Rect, ea: f32) {
        let style = TextStyle::serif(16.0, theme::fade(theme::ink(), ea));
        let middle = card.y + QUERY_H / 2.0;
        if self.query.is_empty() {
            theme::draw(
                layer,
                "Search files and text",
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

    fn draw_empty(&self, layer: &Layer, card: Rect, ea: f32) {
        theme::draw(
            layer,
            "No matching file or text",
            (card.x + LIST_W / 2.0, card.y + QUERY_H + 32.0),
            &TextStyle::serif(13.5, theme::fade(theme::faint(), ea)),
            theme::CENTER,
        );
    }

    fn draw_list(&self, layer: &Layer, card: Rect, ea: f32) {
        let name_style = TextStyle::serif(14.0, theme::fade(theme::ink(), ea));
        let kind_style = TextStyle::mono(10.0, theme::fade(theme::faint(), ea));

        // The selection highlight is a static band: the card is rebuilt
        // per keystroke/move anyway, so an animated pill would re-park on
        // every snapshot instead of gliding.
        if self.selected >= self.first_visible {
            let band = row_rect(card, self.selected - self.first_visible);
            if band.y < card.y + CARD_H - FOOTER_H {
                layer.draw_rectangle(
                    (band.x + 4.0, band.y + 2.0),
                    (LIST_W - 12.0, band.height - 4.0),
                    theme::fade(theme::selection(), ea),
                    Rounding::uniform(ROW_RADIUS),
                );
            }
        }

        let window = self.rows[self.first_visible..]
            .iter()
            .take(MAX_ROWS)
            .enumerate();
        for (offset, row) in window {
            let row_rect = row_rect(card, self.first_visible + offset);
            let middle = row_rect.y + row_rect.height / 2.0;

            // The name first, then a small annotation: "file" for a file
            // row, the hit's parent folder for a text hit (the filename is
            // usually repeated next to it in the list already).
            theme::draw(
                layer,
                row.name(),
                (row_rect.x + 12.0, middle),
                &name_style,
                theme::LEFT,
            );
            let annotation = match row {
                Row::File { .. } => "file".to_string(),
                Row::Text(hit) => {
                    let parent = hit
                        .path
                        .parent()
                        .and_then(|p| p.file_name())
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "note".into());
                    format!("{parent} \u{b7} text")
                }
            };
            let name_w = theme::width(layer, row.name(), &name_style);
            let max_annotation = LIST_W - 12.0 - 8.0 - name_w;
            let annotation = compact(&annotation, max_annotation, layer, &kind_style);
            theme::draw(
                layer,
                &annotation,
                (row_rect.right() - 12.0, middle),
                &kind_style,
                theme::RIGHT,
            );
        }
    }

    /// The right pane: the selected row's content, rendered as it reads on
    /// the page. A file row shows the note's first lines; a text hit shows
    /// the line it matched.
    fn draw_preview(&self, layer: &Layer, card: Rect, ea: f32) {
        let title_style = TextStyle::serif(13.0, theme::fade(theme::comment(), ea));
        let body_style = TextStyle::serif(13.0, theme::fade(theme::ink(), ea));
        let preview = Rect::new(
            card.x + LIST_W + 16.0,
            card.y + QUERY_H + 14.0,
            card.width - LIST_W - 32.0,
            0.0,
        );
        let max_width = preview.width - 8.0;
        let mut y = preview.y;

        let Some(row) = self.rows.get(self.selected) else {
            return;
        };
        let header = match row {
            Row::File { .. } => row.name().to_string(),
            Row::Text(hit) => hit.name.clone(),
        };
        theme::draw(layer, &header, (preview.x, y), &title_style, theme::LEFT);
        y += 26.0;

        let lines: Vec<String> = match row {
            Row::File { .. } => self.file_lines(row),
            Row::Text(hit) => vec![hit.line.clone()],
        };
        for line in lines {
            if y > card.bottom() - FOOTER_H - 16.0 {
                break;
            }
            let line = compact(&line, max_width, layer, &body_style);
            theme::draw(layer, &line, (preview.x, y), &body_style, theme::LEFT);
            y += 19.0;
        }
    }

    /// The first lines of a file row's note, as they read on the page.
    fn file_lines(&self, row: &Row) -> Vec<String> {
        let Some(text) = std::fs::read_to_string(row.path()).ok() else {
            return Vec::new();
        };
        let doc = crate::document::markdown::parse(row.path(), &text);
        doc.body()
            .iter()
            .map(|block| {
                block
                    .inlines()
                    .iter()
                    .map(|run| match run {
                        crate::document::Inline::Text(t) => t.text.clone(),
                        crate::document::Inline::Math(list) => {
                            format!("${}$", crate::document::math_notation::print(list))
                        }
                        crate::document::Inline::Note(_) => String::new(),
                    })
                    .collect::<String>()
            })
            .collect()
    }
}

/// `line` shortened to `max_width` pixels, an ellipsis taking the bite.
/// Purely a draw-time trim: the underlying rows are never rewritten.
fn compact(line: &str, max_width: f32, layer: &Layer, style: &TextStyle) -> String {
    if max_width <= 0.0 || theme::width(layer, line, style) <= max_width {
        return line.to_string();
    }
    let ellipsis = "\u{2026}";
    let mut out = String::new();
    for ch in line.chars() {
        let candidate = format!("{out}{ch}{ellipsis}");
        if theme::width(layer, &candidate, style) > max_width {
            break;
        }
        out.push(ch);
    }
    format!("{out}{ellipsis}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn file_row(name: &str) -> Row {
        Row::File {
            path: PathBuf::from(name),
            name: name.into(),
        }
    }

    #[test]
    fn a_card_fills_the_viewport_from_its_center() {
        let viewport = Rect::new(0.0, 0.0, 1280.0, 800.0);
        let card = card(viewport);
        assert_eq!(card.x, (1280.0 - CARD_W) / 2.0);
        assert_eq!(card.y, (800.0 - CARD_H) / 2.0);
        assert_eq!(card.width, CARD_W);
        assert_eq!(card.height, CARD_H);
    }

    #[test]
    fn the_selection_window_follows_a_selection_that_runs_past_it() {
        let rows: Vec<Row> = (0..20).map(|i| file_row(&format!("n{i}.md"))).collect();
        let finder = Finder::new(rows, String::new(), 15);
        assert_eq!(finder.selected, 15);
        assert_eq!(
            finder.first_visible,
            15 + 1 - MAX_ROWS,
            "the selected row must be the last one drawn"
        );
    }

    #[test]
    fn a_selection_inside_the_window_leaves_the_scroll_alone() {
        let rows: Vec<Row> = (0..20).map(|i| file_row(&format!("n{i}.md"))).collect();
        let finder = Finder::new(rows, String::new(), 2);
        assert_eq!(finder.first_visible, 0);
    }

    #[test]
    fn a_selection_beyond_the_result_count_clamps_to_the_last_row() {
        let rows: Vec<Row> = (0..3).map(|i| file_row(&format!("n{i}.md"))).collect();
        let finder = Finder::new(rows, String::new(), 9);
        assert_eq!(finder.selected, 2);
    }

    #[test]
    fn an_empty_result_list_clamps_to_zero() {
        let finder = Finder::new(Vec::new(), String::new(), 4);
        assert_eq!(finder.selected, 0);
    }

    #[test]
    fn a_closed_finder_draws_nothing_and_reports_no_animation() {
        let mut finder = Finder::closed();
        assert!(!finder.open);
        assert!(!finder.is_animating());
        finder.clear_dirty();
        assert!(!finder.is_dirty());
    }

    #[test]
    fn file_lines_read_math_as_inline_notation() {
        let dir = std::env::temp_dir().join("tw-finder-preview");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("preview.md");
        std::fs::write(&path, "gain is $R_2 / R_1$ plus one\n").unwrap();
        let row = Row::File {
            path: path.clone(),
            name: "preview.md".into(),
        };
        let rows = vec![row.clone()];
        let finder = Finder::new(rows, String::new(), 0);
        let lines = finder.file_lines(&row);
        // Math reads as its canonical notation, the same form the search
        // index matches against.
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("gain is $R_2"));
        assert!(lines[0].contains("R_1"));
        std::fs::remove_file(&path).ok();
    }
}
