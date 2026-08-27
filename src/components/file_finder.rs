//! Inline vault file finder.

use std::path::PathBuf;

use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;

use crate::components::title_bar;
use crate::layout::Rect;
use crate::renderer::{Layer, Rounding};
use crate::theme::{self, TextStyle};
use crate::ui::{Component, Context, Dirty};
use crate::vault::VaultFile;

use super::popup::paint_shadow_slab;

/// Corner radius of a row highlight and of the panel itself — the finder is
/// a tall narrow card, so its shoulders round slightly less than a small
/// popup's would (see `popup::CARD_RADIUS`).
const PANEL_RADIUS: f32 = 8.0;
const ROW_RADIUS: f32 = 6.0;

const PANEL_WIDTH: f32 = 420.0;
const ROW_HEIGHT: f32 = 28.0;
const MAX_ROWS: usize = 10;

/// Indices into `files`, ranked by fuzzy score and then source order.
pub fn filter(files: &[VaultFile], query: &str) -> Vec<usize> {
    if query.is_empty() {
        return (0..files.len()).collect();
    }

    let matcher = SkimMatcherV2::default();
    let mut matches: Vec<(usize, i64)> = files
        .iter()
        .enumerate()
        .filter_map(|(index, file)| {
            matcher
                .fuzzy_indices(&file.name, query)
                .map(|(score, _)| (index, score))
        })
        .collect();
    matches.sort_by(|(left_index, left_score), (right_index, right_score)| {
        right_score
            .cmp(left_score)
            .then_with(|| left_index.cmp(right_index))
    });
    matches.into_iter().map(|(index, _)| index).collect()
}

pub struct FileFinder {
    files: Vec<VaultFile>,
    visible: Vec<usize>,
    selected: usize,
    first_visible: usize,
    /// The blurred layer this panel paints its shadow slab into.
    shadow: Option<Layer>,
    reveal: f32,
    /// Whether the shell granted this snapshot ownership of the shared
    /// shadow layer this frame (`Context::owns_shadow`); only an owner may
    /// paint or clear.
    owned: bool,
    dirty: Dirty,
}

impl FileFinder {
    pub fn new(files: Vec<VaultFile>, query: String, selected: usize) -> Self {
        let visible = filter(&files, &query);
        let selected = selected.min(visible.len().saturating_sub(1));
        Self {
            files,
            visible,
            selected,
            first_visible: selected.saturating_sub(MAX_ROWS.saturating_sub(1)),
            shadow: None,
            reveal: 0.0,
            owned: false,
            dirty: Dirty::new(),
        }
    }

    pub fn closed() -> Self {
        Self {
            files: Vec::new(),
            visible: Vec::new(),
            selected: 0,
            first_visible: 0,
            shadow: None,
            reveal: 1.0,
            owned: false,
            dirty: Dirty::new(),
        }
    }

    /// Attaches the shell's blurred shadow layer.
    pub fn with_shadow(mut self, shadow: Layer) -> Self {
        self.shadow = Some(shadow);
        self
    }

    /// Paints the shadow slab from sync. The finder is anchored to the
    /// title bar's search box; its resting geometry depends on that box's
    /// rect, so sync only clears here and draw paints after measuring. A
    /// closed snapshot still clears unconditionally.
    fn clear_shadow(&self) {
        if let Some(shadow) = &self.shadow {
            super::popup::paint_shadow_slab(shadow, Rect::default(), -1.0);
        }
    }
}

impl Component for FileFinder {
    fn measure(&mut self, _: &Layer) -> (f32, f32) {
        // An overlay bound to the title bar; it asks the layout for nothing.
        (0.0, 0.0)
    }

    fn sync(&mut self, context: &Context) {
        // The finder's resting geometry needs the search box rect, which
        // only draw can measure; sync therefore clears the shadow layer on
        // every reveal change and lets draw paint the slab. A closed
        // snapshot still clears unconditionally — the halo must leave with
        // whoever last drew it.
        let reveal_changed = self.reveal != context.reveal;
        if reveal_changed {
            self.reveal = context.reveal;
            self.dirty.set();
        }
        // Draw reads this — it paints the slab from measured geometry.
        self.owned = context.owns_shadow;
        // An owner with an empty list is the CLOSED snapshot the shell
        // builds when the finder closes — it exists purely to blank the
        // layer this frame. Without this the last halo would linger
        // forever: ownership normally pairs with a paint in draw, but an
        // empty snapshot draws no panel at all, and reveal never changes
        // again to re-arm the clear above.
        if self.owned && self.visible.is_empty() {
            self.clear_shadow();
        }
    }

    fn is_dirty(&self) -> bool {
        self.dirty.get()
    }

    fn clear_dirty(&mut self) {
        self.dirty.clear()
    }

    fn is_animating(&self) -> bool {
        self.reveal > 0.0
            && self.reveal < 1.0
            && (!self.files.is_empty() || !self.visible.is_empty())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn draw(&mut self, layer: &Layer, rect: Rect) {
        if self.files.is_empty() && self.visible.is_empty() && self.first_visible == 0 {
            return;
        }
        let ea = self.reveal.clamp(0.0, 1.0);
        let title = Rect::new(rect.x, rect.y, rect.width, title_bar::HEIGHT);
        let search = title_bar::search_box_rect(layer, title);
        let panel = Rect::new(
            search.right() - PANEL_WIDTH,
            search.bottom() + 4.0,
            PANEL_WIDTH,
            (self.visible.len().min(MAX_ROWS) as f32 * ROW_HEIGHT + 12.0).max(40.0),
        );

        // Shadow from draw: this component's slab depends on the measured
        // search-box geometry (see sync). Painted only while the shell has
        // granted this region ownership of the shared layer.
        if self.owned
            && let Some(shadow) = &self.shadow
        {
            paint_shadow_slab(shadow, panel, ea);
        }

        layer.set_clip_rect(Some((panel.position(), panel.size())));
        layer.draw_rectangle(
            panel.position(),
            panel.size(),
            theme::elevated_popup(ea),
            Rounding::uniform(PANEL_RADIUS),
        );
        theme::rounded_outline(
            layer,
            panel.inset(0.5),
            PANEL_RADIUS - 0.5,
            1.0,
            theme::fade(theme::non_text(), ea),
        );
        let row_style = TextStyle::serif(13.0, theme::fade(theme::ink(), ea));
        let row_content_width = (panel.width - 24.0).max(0.0);
        for (offset, &index) in self.visible[self.first_visible..]
            .iter()
            .take(MAX_ROWS)
            .enumerate()
        {
            let row = Rect::new(
                panel.x,
                panel.y + 6.0 + offset as f32 * ROW_HEIGHT,
                panel.width,
                ROW_HEIGHT,
            );
            if self.first_visible + offset == self.selected {
                layer.draw_rectangle(
                    (row.x + 3.0, row.y + 2.0),
                    (row.width - 6.0, row.height - 4.0),
                    theme::fade(theme::selection(), ea),
                    Rounding::uniform(ROW_RADIUS),
                );
            }
            let file = &self.files[index];
            let middle = row.y + row.height / 2.0;
            let label = compact_path(layer, &file.name, &row_style, row_content_width);
            theme::draw(
                layer,
                &label,
                (row.x + 12.0, middle),
                &row_style,
                theme::LEFT,
            );
        }
        if self.visible.is_empty() {
            theme::draw(
                layer,
                "No matching file",
                (panel.x + 12.0, panel.y + 20.0),
                &TextStyle::serif(13.0, theme::fade(theme::faint(), ea)),
                theme::LEFT,
            );
        }
        layer.set_clip_rect(None);
    }
}

pub fn compact_path(layer: &Layer, path: &str, style: &TextStyle, max_width: f32) -> String {
    compact_path_with_width(path, max_width, |text| theme::width(layer, text, style))
}

fn compact_path_with_width<F>(path: &str, max_width: f32, mut width: F) -> String
where
    F: FnMut(&str) -> f32,
{
    if max_width <= 0.0 {
        return String::new();
    }
    if width(path) <= max_width {
        return path.into();
    }

    let basename = path.rsplit('/').next().unwrap_or(path);
    if width(basename) > max_width {
        return compact_suffix(path, max_width, &mut width);
    }

    let mut compact = format!(".../{basename}");
    if width(&compact) > max_width {
        return basename.into();
    }

    let parent = &path[..path.len().saturating_sub(basename.len())];
    for folder in parent.trim_end_matches('/').rsplit('/') {
        let candidate = format!(".../{folder}/{}", compact.trim_start_matches(".../"));
        if width(&candidate) > max_width {
            break;
        }
        compact = candidate;
    }
    compact
}

fn compact_suffix<F>(path: &str, max_width: f32, width: &mut F) -> String
where
    F: FnMut(&str) -> f32,
{
    let marker = "...";
    if width(marker) <= max_width {
        let mut suffix = String::new();
        for character in path.chars().rev() {
            let candidate = format!("{marker}{character}{suffix}");
            if width(&candidate) > max_width {
                break;
            }
            suffix.insert(0, character);
        }
        return format!("{marker}{suffix}");
    }

    let mut suffix = String::new();
    for character in path.chars().rev() {
        let candidate = format!("{character}{suffix}");
        if width(&candidate) > max_width {
            break;
        }
        suffix.insert(0, character);
    }
    suffix
}

pub fn path_at(files: &[VaultFile], query: &str, selected: usize) -> Option<PathBuf> {
    filter(files, query)
        .get(selected)
        .map(|&index| files[index].path.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> VaultFile {
        VaultFile {
            name: name.into(),
            path: name.into(),
        }
    }

    #[test]
    fn fuzzy_ranking_prefers_score_and_keeps_source_order_on_ties() {
        let files = vec![
            file("notes/alpha.md"),
            file("notes/algebra.md"),
            file("notes/beta.md"),
        ];
        assert_eq!(filter(&files, "alg"), vec![1]);
        assert_eq!(filter(&files, "md"), vec![0, 1, 2]);
    }

    #[test]
    fn empty_query_keeps_all_files_in_source_order() {
        let files = vec![file("b.md"), file("a.md")];
        assert_eq!(filter(&files, ""), vec![0, 1]);
    }

    #[test]
    fn compact_path_preserves_basename() {
        let path = "lectures/algebra/limits.md";
        let compact = compact_path_with_width(path, 18.0, |text| text.len() as f32);
        assert!(compact.ends_with("limits.md"));
    }

    #[test]
    fn compact_path_shortens_long_paths() {
        let path = "lectures/algebra/very-long-name.md";
        let compact = compact_path_with_width(path, 20.0, |text| text.len() as f32);
        assert!(compact.len() < path.len());
        assert!(compact.ends_with("very-long-name.md"));
    }

    #[test]
    fn compact_path_keeps_short_paths_unchanged() {
        let path = "algebra/limits.md";
        assert_eq!(
            compact_path_with_width(path, 40.0, |text| text.len() as f32),
            path
        );
    }
}
