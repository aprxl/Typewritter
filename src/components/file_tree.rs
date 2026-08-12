//! The vault as a tree, for real: directories on disk, read one folder at a
//! time as it is expanded. The region's scissor is the viewport, so
//! scrolling is just a drawing offset.
//!
//! Clicks open files: a single click previews (a transient tab, replaced by
//! the next open), a double-click pins the file in a full tab.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::layout::Rect;
use crate::renderer::{Layer, Rounding};
use crate::tabs::Tabs;
use crate::theme::{self, TextStyle, icons};
use crate::ui::{Component, Context, Dirty, Hover};
use crate::vault::Vault;

pub const WIDTH: f32 = 250.0;

/// What a right-click in the file tree landed on: a row, or the empty space
/// below the rows. The tree owns the row hit-test, so it is the only thing
/// that can tell them apart; the shell holds the other end of this and
/// clears it the frame it acts on it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuTarget {
    Row,
    Empty,
}

pub type MenuRequest = Rc<Cell<Option<((f32, f32), MenuTarget)>>>;

/// One row's height. Names are drawn at their baseline, so a row's visual
/// extent is `[y - ROW_HALF, y - ROW_HALF + ROW_HEIGHT)` and the hit-test
/// must use exactly that band — a click at a row's visual middle has to
/// land on that row.
const ROW_HEIGHT: f32 = 27.0;
const ROW_HALF: f32 = 13.0;
/// Depth indent per level.
const INDENT: f32 = 14.0;
/// The vault name is pinned at the top; scrolling starts below it.
const HEADER_Y: f32 = 24.0;
const CONTENT_TOP: f32 = HEADER_Y + 26.0;
/// Two clicks on one file inside this window count as a double-click.
const DOUBLE_CLICK: Duration = Duration::from_millis(300);

pub struct FileTree {
    vault: Option<Rc<RefCell<Vault>>>,
    docs: Rc<RefCell<Tabs>>,
    menu_request: MenuRequest,
    /// The row currently receiving the hover surface. Kept during fade-out.
    hovered: Option<PathBuf>,
    hover: Hover,
    /// Content offset in logical pixels, clamped to the content's own size.
    scroll: f32,
    divider_hover: f32,
    debug_rows: bool,
    /// The last file row clicked, for double-click detection.
    last_click: Option<(PathBuf, Instant)>,
    dirty: Dirty,
}

impl FileTree {
    pub fn new(
        vault: Option<Rc<RefCell<Vault>>>,
        docs: Rc<RefCell<Tabs>>,
        menu_request: MenuRequest,
    ) -> Self {
        Self {
            vault,
            docs,
            menu_request,
            hovered: None,
            hover: Hover::new(),
            scroll: 0.0,
            divider_hover: 0.0,
            debug_rows: false,
            last_click: None,
            dirty: Dirty::new(),
        }
    }

    /// How tall the scrollable content is; the ceiling for `scroll`.
    fn content_height(&self) -> f32 {
        let rows = self
            .vault
            .as_ref()
            .map_or(0, |v| v.borrow().visible().len());
        CONTENT_TOP + rows as f32 * ROW_HEIGHT
    }

    /// Which row a point falls on. Local is offset by `ROW_HALF` so the
    /// drawn band maps to `[0, ROW_HEIGHT)`.
    fn row_index(&self, rect: Rect, position: (f32, f32)) -> Option<usize> {
        let local = position.1 - rect.y - CONTENT_TOP + self.scroll;
        (local >= -ROW_HALF).then(|| ((local + ROW_HALF) / ROW_HEIGHT) as usize)
    }

    fn row_path(&self, rect: Rect, position: (f32, f32)) -> Option<PathBuf> {
        let index = self.row_index(rect, position)?;
        let vault = self.vault.as_ref()?;
        vault
            .borrow()
            .visible()
            .get(index)
            .map(|row| row.path.to_path_buf())
    }

    /// A click on a row: folders toggle, files open (once previews, twice
    /// pins).
    fn open_row(&mut self, index: usize) {
        let hit = match &self.vault {
            Some(vault) => vault
                .borrow()
                .visible()
                .get(index)
                .map(|row| (row.path.to_path_buf(), row.is_dir)),
            None => None,
        };
        let Some((path, is_dir)) = hit else {
            return;
        };

        self.docs.borrow_mut().tree_selected = Some(path.clone());
        if is_dir {
            if let Some(vault) = &self.vault {
                vault.borrow_mut().toggle(&path);
            }
            // A folder click is not part of a file double-click.
            self.last_click = None;
        } else {
            let double = self
                .last_click
                .as_ref()
                .is_some_and(|(p, at)| *p == path && at.elapsed() <= DOUBLE_CLICK);
            let mut docs = self.docs.borrow_mut();
            if double {
                docs.open_full(&path);
            } else {
                docs.open_preview(&path);
            }
            self.last_click = Some((path, Instant::now()));
        }
        self.dirty.set();
    }
}

impl Component for FileTree {
    fn measure(&mut self, layer: &Layer) -> (f32, f32) {
        let style = TextStyle::serif(14.5, theme::INK);
        let widest = self.vault.as_ref().map_or(0.0, |v| {
            v.borrow()
                .visible()
                .iter()
                .map(|row| theme::width(layer, row.name, &style))
                .fold(0.0, f32::max)
        });
        // A truncated filename is still a usable tree; the insets are not.
        ((widest * 0.55).max(90.0) + 96.0, 200.0)
    }

    fn sync(&mut self, context: &Context) {
        let rect = context.self_rect;
        let over = context.hovering(rect);
        let next = over
            .then(|| self.row_path(rect, context.mouse.position))
            .flatten();
        if self
            .hover
            .track(&mut self.hovered, next, context.animation_dt)
        {
            self.dirty.set();
        }

        if over && context.scroll_y != 0.0 {
            let max = (self.content_height() - rect.height).max(0.0);
            let next = (self.scroll - context.scroll_y * ROW_HEIGHT).clamp(0.0, max);
            self.dirty.write(&mut self.scroll, next);
        }

        if let Some(position) = context.click_position()
            && rect.contains(position)
            && let Some(index) = self.row_index(rect, position)
        {
            self.open_row(index);
            // Expanding can change how much there is to scroll.
            let max = (self.content_height() - rect.height).max(0.0);
            self.scroll = self.scroll.clamp(0.0, max);
        }

        if let Some(position) = context.right_click_position()
            && rect.contains(position)
        {
            let path = self.row_path(rect, position);
            match path {
                Some(path) => {
                    // Menu commands act on `tree_selected`, so right-click must target
                    // this row before the menu opens or New note would use another row.
                    self.docs.borrow_mut().tree_selected = Some(path);
                    self.menu_request.set(Some((position, MenuTarget::Row)));
                }
                None => {
                    // With nothing selected, New note creates at the vault root, which is
                    // what right-clicking the empty part of the panel should mean.
                    self.docs.borrow_mut().tree_selected = None;
                    self.menu_request.set(Some((position, MenuTarget::Empty)));
                }
            }
            self.dirty.set();
        }

        self.dirty
            .write(&mut self.divider_hover, context.divider_hover);
        self.dirty.write(&mut self.debug_rows, context.debug_rows);
    }

    fn is_dirty(&self) -> bool {
        self.dirty.get()
    }

    fn clear_dirty(&mut self) {
        self.dirty.clear();
    }

    fn is_animating(&self) -> bool {
        self.hover.is_animating()
    }

    fn draw(&mut self, layer: &Layer, rect: Rect) {
        layer.draw_rectangle(rect.position(), rect.size(), theme::PANEL, Rounding::NONE);
        theme::vertical_rule(
            layer,
            (rect.right() - 1.0, rect.y),
            rect.height,
            1.0,
            theme::BORDER,
        );

        let Some(vault_ref) = &self.vault else {
            theme::draw(
                layer,
                "No vault loaded",
                (rect.x + 18.0, rect.y + 48.0),
                &TextStyle::serif(14.5, theme::INK),
                theme::LEFT,
            );
            theme::draw(
                layer,
                "run with --onboard to choose one",
                (rect.x + 18.0, rect.y + 78.0),
                &TextStyle::mono(10.5, theme::COMMENT),
                theme::LEFT,
            );
            return;
        };
        let vault = vault_ref.borrow();
        let selected = self.docs.borrow().tree_selected.clone();

        // The vault name, pinned above the scroll.
        if let Some(name) = vault.root().file_name().and_then(|n| n.to_str()) {
            theme::icon(
                layer,
                icons::FOLDER,
                (rect.x + 18.0, rect.y + HEADER_Y - 6.0),
                12.0,
                theme::COMMENT,
                1.8,
            );
            theme::draw(
                layer,
                name,
                (rect.x + 36.0, rect.y + HEADER_Y),
                &TextStyle::mono(10.0, theme::COMMENT).tracked(0.18),
                theme::LEFT,
            );
            theme::rule(
                layer,
                (rect.x + 18.0, rect.y + HEADER_Y + 13.0),
                (rect.width - 36.0).max(0.0),
                1.0,
                theme::BORDER,
            );
        }

        let name_style = TextStyle::serif(14.5, theme::DIM);
        for (index, row) in vault.visible().iter().enumerate() {
            let y = rect.y + CONTENT_TOP - self.scroll + index as f32 * ROW_HEIGHT;
            let band = Rect::new(rect.x, y - ROW_HALF, rect.width - 1.0, ROW_HEIGHT);
            if self.debug_rows {
                // The exact band `sync` hit-tests, so a misaligned hit is
                // visible instead of mysterious.
                theme::outline(layer, band, theme::WARNING);
            }
            let selected = selected.as_deref() == Some(row.path);
            if selected {
                layer.draw_rectangle(
                    band.position(),
                    band.size(),
                    theme::SELECTION,
                    Rounding::NONE,
                );
            }
            // Selection takes priority over the hover surface: a clicked
            // row keeps its full selected treatment.
            if !selected && self.hovered.as_deref() == Some(row.path) {
                theme::hover_fill(layer, band, self.hover.value());
            }
            let chevron_x = rect.x + 18.0 + row.depth as f32 * INDENT;
            if row.is_dir {
                theme::icon(
                    layer,
                    if row.expanded {
                        icons::CHEVRON_DOWN
                    } else {
                        icons::CHEVRON_RIGHT
                    },
                    (chevron_x, y - 5.5),
                    11.0,
                    theme::COMMENT,
                    2.2,
                );
            }
            let (name, icon_color) = if selected {
                (name_style.clone().color(theme::INK), theme::ACCENT)
            } else {
                (name_style.clone(), theme::NON_TEXT)
            };
            theme::icon(
                layer,
                if row.is_dir {
                    icons::FOLDER
                } else {
                    icons::FILE
                },
                (chevron_x + 16.0, y - 6.0),
                12.0,
                icon_color,
                1.8,
            );
            theme::draw(layer, row.name, (chevron_x + 36.0, y), &name, theme::LEFT);
        }

        if self.divider_hover > 0.0 {
            theme::vertical_rule(
                layer,
                (rect.right() - 1.0, rect.y),
                rect.height,
                1.0,
                theme::fade(theme::ACCENT, self.divider_hover),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hit band is exactly the band `draw` fills: `[baseline -
    /// ROW_HALF, baseline - ROW_HALF + ROW_HEIGHT)`. A click anywhere in
    /// the highlight has to land on the row that highlight belongs to.
    #[test]
    fn rows_hit_the_band_they_are_drawn_in() {
        let tree = FileTree::new(
            None,
            Rc::new(RefCell::new(Tabs::new())),
            Rc::new(Cell::new(None::<((f32, f32), MenuTarget)>)),
        );
        let rect = Rect::new(0.0, 0.0, WIDTH, 600.0);
        let baseline_of = |row: usize| CONTENT_TOP + row as f32 * ROW_HEIGHT;

        assert_eq!(tree.row_index(rect, (10.0, baseline_of(0))), Some(0));
        assert_eq!(tree.row_index(rect, (10.0, baseline_of(3))), Some(3));
        assert_eq!(
            tree.row_index(rect, (10.0, baseline_of(3) - ROW_HALF)),
            Some(3),
            "the top edge of the drawn band belongs to that row"
        );
        assert_eq!(
            tree.row_index(rect, (10.0, baseline_of(3) - ROW_HALF + ROW_HEIGHT - 0.1)),
            Some(3),
            "and so does the last pixel of it"
        );
        assert_eq!(
            tree.row_index(rect, (10.0, baseline_of(3) - ROW_HALF + ROW_HEIGHT)),
            Some(4),
            "one pixel further is the next row"
        );
        assert_eq!(
            tree.row_index(rect, (10.0, 0.0)),
            None,
            "the pinned header is not a row"
        );
    }

    /// Scrolling moves the bands with the content, not just the drawing.
    #[test]
    fn a_scrolled_tree_hits_the_row_under_the_pointer() {
        let mut tree = FileTree::new(
            None,
            Rc::new(RefCell::new(Tabs::new())),
            Rc::new(Cell::new(None::<((f32, f32), MenuTarget)>)),
        );
        let rect = Rect::new(0.0, 0.0, WIDTH, 600.0);
        let baseline = CONTENT_TOP + 5.0 * ROW_HEIGHT;
        assert_eq!(tree.row_index(rect, (10.0, baseline)), Some(5));

        tree.scroll = 3.0 * ROW_HEIGHT;
        assert_eq!(
            tree.row_index(rect, (10.0, baseline)),
            Some(8),
            "three rows scrolled away puts row 8 where row 5 was"
        );
    }
}
