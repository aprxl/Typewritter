//! The shell: the layout tree, the regions that fill it, and the frame loop
//! that drives both.
//!
//! This is `design/Main View.dc.html`, rebuilt against the real renderer.
//! The left tree reads the actual vault on disk (see [`crate::vault`]) and
//! the centre column is a working buffer: files open into tabs (a click
//! previews, a double-click pins), the text is editable, and `Ctrl+S` saves.
//! Still mock: the sidenote margin, which belongs to document anchors.
//!
//! Everything visible is a [`Component`] in its own [`Region`], so each part
//! measures itself, redraws only when it changes, and is scissored to its
//! own rect. The widgets live in [`crate::components`]; what is here is the
//! wiring — which region sits on which node, what the shared state is, and
//! when a view has to be rebuilt.
//!
//! Two pieces of state are shared through [`Rc`]s so the shell and the
//! interactive components can both see them: the vault (read and mutated by
//! the tree, pruned by the shell on delete) and the open tabs (mutated by
//! the tree, the tab strip, and the shell). *View* regions — the tab strip,
//! the breadcrumb, the editor, the status line — are rebuilt from a live
//! snapshot whenever [`Tabs::revision`] moves, which is what
//! [`Region::set_component`] is for.

mod commands;
mod input;
mod panel;
mod stepped;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use winit::event::MouseButton;

use crate::animation::{Animation, Easing};
use crate::components::dialog::Prompt;
use crate::components::sidenotes::Note;
use crate::components::tab_strip::TabView;
use crate::components::topics::Entry;
use crate::components::{
    Backdrop, Breadcrumb, ContextMenu, Dialog, Editor, FileFinder, FileTree, MathMenu, Onboarding,
    Palette, SidenoteMargin, SlashMenu, StatusLine, TabStrip, TitleBar, Topics, breadcrumb, editor,
    file_tree, sidenotes, status_line, tab_strip, title_bar, topics,
};
use crate::config::Config;
use crate::document::Caret;
use crate::document::Focus;
use crate::document::layout::{ContextHit, DocLayout, RangeKind, layout_blocks};
use crate::document::math::{MathCursor, NodeAddress};
use crate::document::math_conversion;
use crate::document::outline;
use crate::frame::FrameScheduler;
use crate::input::Input;
use crate::layout::{Layout, NodeId, Rect, Size, Style};
use crate::renderer::{Layer, LayerInvalidation, Renderer, ShaderEffect};
use crate::tabs::Tabs;
use crate::theme::{self, TextStyle};
use crate::ui::{Component, Context, Hover, Mouse, Region};
use crate::vault::Vault;
use crate::vim::{Edit, Operator, OperatorTarget, Vim, VimMode, VisualMode};

use panel::Panel;
use stepped::Stepped;

/// The palette's live query text and selection index while it's open — the
/// shell owns these (it's the one taking keystrokes), the same reason
/// `Dialog`'s prompt lives here instead of in the component.
struct PaletteState {
    query: String,
    selected: usize,
}

struct FileFinderState {
    files: Vec<crate::vault::VaultFile>,
    query: String,
    selected: usize,
}

/// The slash menu's live query text, selection index, and anchor point
/// while it's open — the shell owns these (it's the one taking
/// keystrokes), the same reason `PaletteState` lives here instead of in
/// the component.
#[allow(dead_code)] // wired up in a later task
struct SlashMenuState {
    query: String,
    selected: usize,
    /// Screen position the menu opens next to — the text caret's position
    /// at the moment `/` was typed, captured once and not recomputed while
    /// the menu stays open (the underlying document caret doesn't move
    /// while the user is filtering, since the query lives here, not in the
    /// document).
    anchor: (f32, f32),
}

/// The open context menu's rows and selection, if it's open. The rows are
/// resolved commands rather than ids so running one cannot re-look-up a
/// different entry than the one that was drawn.
struct ContextMenuState {
    items: Vec<&'static commands::Command>,
    selected: usize,
    anchor: (f32, f32),
    target: Option<ContextHit>,
}

/// The in-math completion card while it is showing: the precise tree query,
/// selected row, and anchor point. It has no open/closed state of its own.
struct MathMenuState {
    query: math_conversion::Query,
    selected: usize,
    anchor: (f32, f32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SearchState {
    query: String,
    forward: bool,
}

/// How long a palette swap takes, from the click to the last frame of the
/// wipe. Short enough to stay out of the way, long enough to read as a
/// transition rather than a flicker.
const SWAP_DURATION: Duration = Duration::from_millis(240);
/// Width of the wipe's soft edge, logical pixels. A hard circle sweeping
/// across a page of text shows every stair-step in its own boundary; this
/// is wide enough to hide them and narrow enough to still read as an edge.
const SWAP_FEATHER: f32 = 48.0;

/// A palette swap in flight — the state the switch is locked by.
///
/// The whole point of the sequence is that the two palettes never exist as
/// two live interfaces at once. The frame that was on screen when the
/// switch was clicked is copied into a layer of its own and held there as a
/// still image (see [`Renderer::capture_into`]); the palette then moves,
/// and the entire window redraws itself underneath that image while the
/// image is wiped away from the switch outward. One live interface, one
/// picture, and the picture costs a texture for a quarter of a second.
enum ThemeSwap {
    /// Asked for, and waiting for a frame. The capture can only be armed
    /// from inside [`Shell::update`], which is where the renderer is, so a
    /// click leaves the request here for the same frame's sync to pick up.
    Requested { origin: (f32, f32) },
    /// Armed. The frame being drawn right now — still in the old palette,
    /// because nothing has moved yet — is the one that will be held.
    Capturing { origin: (f32, f32), layer: Layer },
    /// `layer` holds that frame; the new palette is live underneath it and
    /// the wipe is eating it from `origin` outward. Dropping the layer at
    /// the end frees the captured frame and the wipe's own output texture
    /// together.
    Wiping {
        origin: (f32, f32),
        layer: Layer,
        animation: Animation,
    },
}

#[derive(Clone)]
enum RepeatOp {
    Sequence(Vec<RepeatOp>),
    Visual {
        operator: Operator,
        line: bool,
        range: crate::document::FlatRange,
    },
    Edit(Edit, usize),
    Operate(Operator, OperatorTarget, usize),
    Substitute(usize),
    Paste {
        before: bool,
        count: usize,
    },
    Insert(Vec<InsertEvent>),
}

#[derive(Clone)]
enum InsertEvent {
    Text(char),
    Backspace,
    Delete,
    Enter,
}

/// Half-width of the grab zone around a divider, in logical pixels.
const GRAB: f32 = 3.0;

/// How long the document must sit unchanged before it is written.
/// Long enough not to fire between words, short enough that a crash
/// costs a sentence rather than a lecture.
const AUTOSAVE_IDLE: Duration = Duration::from_secs(2);

/// Which panel a divider drag resizes. The two right-hand panels grow
/// leftwards from their own right edge; the tree grows rightwards from
/// its left edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Divider {
    Tree,
    Sidenotes,
    Topics,
}

pub struct Shell {
    layout: Layout,
    regions: Vec<Region>,
    tree: Panel,
    sidenotes: Panel,
    topics: Panel,
    status: Panel,
    text_column: NodeId,
    /// `None` until a vault is chosen (first run / `--onboard`).
    config: Option<Config>,
    vault: Option<Rc<RefCell<Vault>>>,
    /// The open tabs — shared with the tree and the tab strip.
    docs: Rc<RefCell<Tabs>>,
    /// The last text this app published to the OS clipboard. Compared each
    /// frame against the live yank register so a yank — from any of the
    /// vim paths, the operators, or the chords — is exported exactly once,
    /// from one place, instead of at every site that writes the register.
    exported_yank: String,
    /// First run: show the onboarding splash instead of the vault.
    onboarding: bool,
    /// Vim mode and pending state, driving the editor.
    vim: Vim,
    /// The open palette's query and selection, if it's open.
    palette: Option<PaletteState>,
    /// The open file finder's files, query, and selection, if it's open.
    finder: Option<FileFinderState>,
    /// The open slash menu's query, selection, and anchor point, if it's open.
    slash_menu: Option<SlashMenuState>,
    /// The open context menu's rows, selection, and anchor point, if open.
    context_menu: Option<ContextMenuState>,
    /// Persistent Ctrl-brush selection, plus the targets currently under the
    /// brush so a held stroke toggles each target only on entry.
    brush_selected: Vec<ContextHit>,
    brush_inside: Vec<ContextHit>,
    brush_point: Option<(f32, f32)>,
    brush_revision: u64,
    last_brush_revision: u64,
    /// The in-math completion card while it is showing.
    math_menu: Option<MathMenuState>,
    /// The exact query dismissed by the reader. A changed path, range, or
    /// source is a new query and brings the card back.
    math_dismissed: Option<math_conversion::Query>,
    tree_menu_request: file_tree::MenuRequest,
    /// Indices into `regions` of the regions the shell rebuilds.
    title_region: usize,
    tree_region: usize,
    onboard_region: usize,
    tab_region: usize,
    breadcrumb_region: usize,
    topics_region: usize,
    text_region: usize,
    sidenote_region: usize,
    status_region: usize,
    dialog_region: usize,
    palette_region: usize,
    finder_region: usize,
    #[allow(dead_code)] // wired up in a later task
    slash_region: usize,
    menu_region: usize,
    math_menu_region: usize,
    /// Blinks the caret in the editor and the name prompt.
    caret: Stepped,
    /// Fades the writing indicator.
    pulse: Stepped,
    /// When the next animation step is due, for the frames the shell does
    /// *not* ask for. A stepped animation reports nothing through the flat
    /// middle of a step, so without this the loop would sleep past the
    /// blink — see [`stepped`].
    wake_at: Option<Instant>,
    /// The wall clock of the last time the autosave timer restarted —
    /// either an edit or a save attempt. Restarted whenever
    /// [`Tabs::revision`] moves and whenever a save is attempted, so the
    /// save debounces from the last activity rather than a fixed period.
    /// Restarting on attempt is what keeps a tab that cannot be saved — a
    /// failed write, or a dirty tab with no path — from retrying every
    /// frame.
    autosave_last_attempt: Instant,
    /// The revision `autosave_last_attempt` was captured at.
    autosave_revision: u64,
    show_stats: bool,
    /// Draws each tree row's hit band (`d` toggles).
    debug_rows: bool,
    dragging: Option<Divider>,
    divider_hot: bool,
    divider_hover: Hover,
    layouts: u32,
    redraws: (usize, usize),
    /// The open dialog, if any. While it is set it owns the keyboard.
    dialog: Option<Prompt>,
    /// Last [`Tabs::revision`] the views were rebuilt from.
    last_revision: u64,
    /// Last vim mode the views were rebuilt from — mode changes (Escape
    /// out of Insert, with nothing else touched) don't move `Tabs::revision`
    /// on their own, but the status badge and the editor's caret shape both
    /// depend on it.
    last_mode: VimMode,
    last_visual_state: (
        Option<VisualMode>,
        Option<crate::document::FlatPos>,
        Option<crate::document::FlatRange>,
        bool,
    ),
    /// Where the caret-follow last ran: `(active tab, focus, caret)`. The
    /// editor only snaps to the caret when this changes, so a manual wheel
    /// scroll can take the caret out of view freely. Focus is part of the
    /// key: moving focus into a note must follow the note, even when the
    /// caret value happens not to move.
    followed: (usize, Focus, Caret),
    /// The laid-out active document, keyed by `(revision, content width)`.
    /// Rebuilt in `rebuild_views`; the editor and the shell's own caret
    /// math (click, j/k) read the same `Rc`.
    doc_layout: Option<(u64, f32, Rc<DocLayout>)>,
    /// The goal x (pixels) vertical motion aims at — the vim `goal_col`,
    /// now in pixels. Some until a non-vertical caret change clears it.
    goal_x: Option<f32>,
    /// Shell owns selection geometry; Vim owns only parser state.
    visual_anchor: Option<crate::document::FlatPos>,
    visual_override: Option<crate::document::FlatRange>,
    visual_line_mode: bool,
    search: Option<SearchState>,
    last_search: Option<(String, bool)>,
    rendered_search: Option<(String, bool)>,
    repeat: Option<RepeatOp>,
    insert_repeat: Vec<InsertEvent>,
    insert_prefix: Option<RepeatOp>,
    /// The text region's width views were last rebuilt from. A panel toggle
    /// resizes it without moving `Tabs::revision`, and the wrap must follow.
    last_width: f32,
    /// The blurred layer above the editor, carrying highlight glow. Held
    /// here because `rebuild_views` swaps the editor component out and the
    /// layer has to outlive that.
    glow: Layer,
    /// The palette swap in flight, if there is one — see [`ThemeSwap`].
    /// `Some` is also what locks the switch: a swap cannot be spammed,
    /// because a second one would capture a frame mid-wipe and hold *that*
    /// as its still image.
    theme_swap: Option<ThemeSwap>,
}

impl Shell {
    /// `config` is `None` on first run and with `--onboard`, in which case
    /// the shell starts on the onboarding splash until a vault is picked.
    pub fn new(renderer: &mut Renderer, config: Option<Config>) -> Self {
        let vault = config
            .as_ref()
            .and_then(|config| Vault::open(&config.vault))
            .map(|vault| Rc::new(RefCell::new(vault)));
        let docs = Rc::new(RefCell::new(Tabs::new()));
        let tree_menu_request = Rc::new(Cell::new(None));

        let mut layout = Layout::new(Style::default());
        let title = layout.add_child(Layout::ROOT, Style::fixed(title_bar::HEIGHT));
        let tabs = layout.add_child(Layout::ROOT, Style::fixed(tab_strip::HEIGHT));
        let breadcrumb = layout.add_child(Layout::ROOT, Style::fixed(breadcrumb::HEIGHT));
        let body = layout.add_child(Layout::ROOT, Style::flex(1.0).row());
        let status = layout.add_child(Layout::ROOT, Style::fixed(status_line::HEIGHT));

        // The sidenote margin lives inside the canvas, which is what makes
        // it part of the document rather than a fourth panel.
        let tree = layout.add_child(body, Style::fixed(file_tree::WIDTH));
        let canvas = layout.add_child(body, Style::flex(1.0).row());
        let topics = layout.add_child(body, Style::fixed(topics::WIDTH));
        let text_column = layout.add_child(canvas, Style::flex(1.0));
        let sidenotes = layout.add_child(canvas, Style::fixed(sidenotes::WIDTH));

        // One layer each, created bottom-to-top.
        let region = |renderer: &mut Renderer, node, component: Box<dyn Component>| {
            Region::new(
                node,
                renderer.new_layer_top(LayerInvalidation::Manual),
                component,
            )
        };
        let mut regions = vec![
            region(renderer, Layout::ROOT, Box::new(Backdrop)),
            region(
                renderer,
                title,
                Box::new(TitleBar::new("Typewritter", "LECTURE CAPTURE", None)),
            ),
            region(
                renderer,
                tabs,
                Box::new(TabStrip::new(docs.clone(), Vec::new(), None)),
            ),
            region(
                renderer,
                breadcrumb,
                Box::new(Breadcrumb::new(vault_name(&vault))),
            ),
            region(
                renderer,
                tree,
                Box::new(FileTree::new(
                    vault.clone(),
                    docs.clone(),
                    tree_menu_request.clone(),
                )),
            ),
            region(renderer, text_column, Box::new(Editor::placeholder())),
        ];

        // Not a region: a bare layer the editor draws its highlight halos
        // into, created here so it composites directly above the text and
        // below everything else. The blur is set once — it is the effect,
        // and it never changes.
        //
        // ponytail: the blur runs over the whole layer every frame, even
        // with no highlight on screen. Gate it on `set_effect` when a
        // document with no marks costs measurably more than one without.
        let glow = renderer.new_layer_top(LayerInvalidation::Manual);
        glow.set_effect(Some(ShaderEffect::Blur {
            radius: editor::GLOW_RADIUS,
        }));

        regions.extend([
            region(
                renderer,
                sidenotes,
                Box::new(SidenoteMargin::new(Vec::new(), editor::TOP, 0.0)),
            ),
            region(renderer, topics, Box::new(Topics::new(Vec::new(), 0))),
            region(
                renderer,
                status,
                Box::new(StatusLine::new(
                    "NORMAL",
                    theme::cool(),
                    "no file open".into(),
                    String::new(),
                    String::new(),
                    true,
                    String::new(),
                )),
            ),
        ]);

        let idx = |node| {
            regions
                .iter()
                .position(|region| region.node() == node)
                .expect("region built")
        };
        let title_region = idx(title);
        let (tree_region, tab_region) = (idx(tree), idx(tabs));
        let (breadcrumb_region, topics_region, text_region) =
            (idx(breadcrumb), idx(topics), idx(text_column));
        let sidenote_region = idx(sidenotes);
        let status_region = idx(status);

        // The overlays span the viewport by sitting on the root node, each
        // on its own layer above everything else — the splash over the
        // shell, and a dialog over both.
        //
        // They start *detached*: a closed overlay holds no layer, and
        // `sync_overlay_layers` gives it one the frame it opens, on top of
        // whatever is already there. Stacking therefore follows what the
        // user opened last rather than this list's order.
        let onboarding = config.is_none();
        regions.push(Region::detached(
            Layout::ROOT,
            Box::new(Onboarding::new(onboarding)),
        ));
        let onboard_region = regions.len() - 1;
        regions.push(Region::detached(Layout::ROOT, Box::new(Dialog::new(None))));
        let dialog_region = regions.len() - 1;
        regions.push(Region::detached(Layout::ROOT, Box::new(Palette::closed())));
        let palette_region = regions.len() - 1;
        regions.push(Region::detached(
            Layout::ROOT,
            Box::new(SlashMenu::closed()),
        ));
        let slash_region = regions.len() - 1;
        regions.push(Region::detached(
            Layout::ROOT,
            Box::new(FileFinder::closed()),
        ));
        let finder_region = regions.len() - 1;
        regions.push(Region::detached(
            Layout::ROOT,
            Box::new(ContextMenu::closed()),
        ));
        let menu_region = regions.len() - 1;
        regions.push(Region::detached(Layout::ROOT, Box::new(MathMenu::closed())));
        let math_menu_region = regions.len() - 1;

        Self {
            layout,
            regions,
            tree: Panel::new(tree, file_tree::WIDTH),
            sidenotes: Panel::new(sidenotes, sidenotes::WIDTH),
            topics: Panel::new(topics, topics::WIDTH),
            status: Panel::new(status, status_line::HEIGHT),
            text_column,
            config,
            vault,
            docs,
            exported_yank: String::new(),
            onboarding,
            vim: Vim::new(),
            palette: None,
            finder: None,
            slash_menu: None,
            context_menu: None,
            brush_selected: Vec::new(),
            brush_inside: Vec::new(),
            brush_point: None,
            brush_revision: 0,
            last_brush_revision: 0,
            math_menu: None,
            math_dismissed: None,
            tree_menu_request,
            title_region,
            tree_region,
            onboard_region,
            tab_region,
            breadcrumb_region,
            topics_region,
            text_region,
            sidenote_region,
            status_region,
            dialog_region,
            palette_region,
            finder_region,
            slash_region,
            menu_region,
            math_menu_region,
            // Two steps, because a caret is on or off: every frame between
            // two flips repaints the same pixels. The writing indicator is
            // a fade, but `Topics` already rounds it to sixteenths, so
            // sixteen steps is every value that reaches the screen.
            caret: Stepped::new(Duration::from_millis(1050), Easing::Linear, 2),
            pulse: Stepped::new(Duration::from_millis(1200), Easing::EaseInOut, 16).ping_pong(),
            wake_at: None,
            autosave_last_attempt: Instant::now(),
            autosave_revision: 0,
            show_stats: true,
            debug_rows: false,
            dragging: None,
            divider_hot: false,
            divider_hover: Hover::new(),
            layouts: 0,
            redraws: (0, 0),
            dialog: None,
            last_revision: 0,
            last_mode: VimMode::Normal,
            last_visual_state: (None, None, None, false),
            followed: (
                usize::MAX,
                Focus::Body,
                Caret {
                    block: 0,
                    inline: 0,
                    offset: 0,
                    style: crate::document::Style::PLAIN,
                },
            ),
            doc_layout: None,
            goal_x: None,
            visual_anchor: None,
            visual_override: None,
            visual_line_mode: false,
            search: None,
            last_search: None,
            rendered_search: None,
            repeat: None,
            insert_repeat: Vec::new(),
            insert_prefix: None,
            last_width: 0.0,
            glow,
            theme_swap: None,
        }
    }

    /// One frame of shell work. Returns whether an animation still wants
    /// frames *now*; see [`Shell::wake_at`] for the ones that want one later.
    ///
    /// `renderer` is here for the overlay layers, which are attached on open
    /// and dropped on close rather than held for the session, and for the
    /// palette swap, which captures a frame into a layer of its own.
    pub fn update(
        &mut self,
        input: &Input,
        viewport: Rect,
        frametime: Duration,
        renderer: &mut Renderer,
    ) -> bool {
        // Every animation, every frame, accumulated with `|=` — `||` would
        // short-circuit and stop advancing the rest.
        let dt = FrameScheduler::animation_delta(frametime);

        self.handle_input(input, viewport);
        self.autosave();
        self.sync_math_menu();
        self.export_yank();
        self.sync_overlay_layers(renderer);
        // Before `sync_theme`, and on purpose: the step that swaps the
        // palette is in here, and the redraw it arms has to be taken in the
        // same frame or the window would spend one frame in the old colours
        // underneath a still image of the old colours.
        let mut animating = self.sync_theme_swap(renderer, viewport, dt);
        self.sync_theme();

        // The two stepped ones report a *step change*, not "still running",
        // so they fall silent between steps and the loop can sleep. They
        // keep their own wall clock; `dt` is clamped, and clamped time
        // cannot drive something that sleeps longer than the clamp.
        animating |= self.caret.advance();
        animating |= self.pulse.advance();
        let animation_wake = Instant::now() + self.caret.wake_in().min(self.pulse.wake_in());
        // A pending autosave is a deadline too: it must wake the loop from
        // its sleep even though no animation is asking for a frame. Without
        // this the save fires while typing and then never while idle.
        self.wake_at = Some(
            self.autosave_deadline()
                .map_or(animation_wake, |at| at.min(animation_wake)),
        );
        animating |= self.divider_hover.update(self.divider_hot, dt);
        animating |= self.divider_hover.is_animating();
        for panel in self.panels_mut() {
            animating |= panel.animation.advance(dt);
        }

        // Push the animated extents into the tree. Below half a pixel the
        // panel is hidden outright.
        let extents = self.panels().map(|p| (p.node, p.current()));
        for (node, extent) in extents {
            let visible = extent >= 0.5;
            self.layout.set_style(node, |s| {
                s.size = Size::Fixed(extent);
                s.visible = visible;
            });
        }

        let context = Context {
            frametime,
            animation_dt: dt,
            layouts: self.layouts,
            redraws: self.redraws,
            divider_hot: self.divider_hot || self.dragging == Some(Divider::Tree),
            // Step-end, not a fade: a caret that fades looks like a bug.
            caret_on: self.caret.is_on(),
            pulse: self.pulse.weight(),
            show_stats: self.show_stats,
            mouse: Mouse {
                position: input.mouse_position(),
                left_pressed: input.is_mouse_pressed(MouseButton::Left),
                right_pressed: input.is_mouse_pressed(MouseButton::Right),
                in_window: input.is_cursor_in_window(),
            },
            scroll_y: input.scroll_delta().1,
            self_rect: Rect::default(),
            divider_hover: self.divider_hover.value(),
            debug_rows: self.debug_rows,
            overlay_open: self.dialog.is_some()
                || self.onboarding
                || self.palette.is_some()
                || self.slash_menu.is_some()
                || self.context_menu.is_some()
                || self.math_menu.is_some()
                || self.finder.is_some(),
            theme_locked: self.theme_swap.is_some(),
        };
        for region in &mut self.regions {
            let mut context = context;
            // A component hit-tests against its own rect; the shell is the
            // only one that knows where that rect is, so it hands it over.
            context.self_rect = self.layout.rect(region.node());
            region.sync(&context);
            animating |= region.is_animating();
            region.measure_into(&mut self.layout);
        }

        if self.layout.compute(viewport) {
            self.layouts += 1;
        }

        let redrawn = self
            .regions
            .iter_mut()
            .fold(0, |n, region| n + region.update(&self.layout) as usize);
        self.redraws = (redrawn, self.regions.len());

        // Whatever changed this frame — typing, a click that opened a file,
        // a tab closed, Escape out of Insert with nothing else touched —
        // lands in the views as one rebuild. Runs after the draw pass, so
        // the fresh components paint on the next frame. A panel toggle
        // resizes the text column without moving `Tabs::revision`, but the
        // wrap depends on that width, so it rebuilds too.
        let revision = self.docs.borrow().revision();
        let mode = self.vim.current_mode();
        let visual_state = (
            self.vim.visual_mode(),
            self.visual_anchor,
            self.visual_override,
            self.visual_line_mode,
        );
        let width = self.layout.rect(self.text_column).width;
        let search = self
            .search
            .as_ref()
            .map(|state| (state.query.clone(), state.forward));
        if revision != self.last_revision
            || mode != self.last_mode
            || visual_state != self.last_visual_state
            || width != self.last_width
            || search != self.rendered_search
            || self.brush_revision != self.last_brush_revision
        {
            self.last_revision = revision;
            self.last_mode = mode;
            self.last_visual_state = visual_state;
            self.last_width = width;
            self.rendered_search = search;
            self.last_brush_revision = self.brush_revision;
            self.rebuild_views();
        }
        animating
    }

    /// The smallest the window may be before regions start overlapping.
    pub fn min_window_size(&self) -> (f32, f32) {
        self.layout.min_size(Layout::ROOT)
    }

    /// When the next stepped animation changes what it draws. The event loop
    /// sleeps until then when nothing else wants a frame — a caret asks for
    /// two frames a second, not for every frame between two blinks.
    pub fn wake_at(&self) -> Option<Instant> {
        self.wake_at
    }

    /// Runs the idle autosave: restart the clock whenever [`Tabs::revision`]
    /// moves or a save is attempted, and write every dirty tab once the
    /// document has sat still for [`AUTOSAVE_IDLE`]. Failures are logged; a
    /// failing tab is retried on the next idle window, not every frame.
    ///
    /// The clock is restarted on attempt, not only on edit, so that a tab
    /// that stays dirty — a failed write, or a dirty tab with no path that
    /// `save_all` skips without touching — retries once per
    /// [`AUTOSAVE_IDLE`], never at frame rate.
    fn autosave(&mut self) {
        let revision = self.docs.borrow().revision();
        if revision != self.autosave_revision {
            self.autosave_revision = revision;
            self.autosave_last_attempt = Instant::now();
            return;
        }
        let dirty = self.docs.borrow().any_dirty();
        let now = Instant::now();
        if autosave_due(self.autosave_last_attempt, now, dirty) {
            let failed = self.docs.borrow_mut().save_all();
            for (path, error) in failed {
                eprintln!("autosave failed for {}: {error}", path.display());
            }
            self.autosave_last_attempt = now;
        }
    }

    /// The instant the idle autosave is next due, if a dirty tab is waiting.
    /// `None` when nothing is dirty — there is no deadline to wake for.
    fn autosave_deadline(&self) -> Option<Instant> {
        if !self.docs.borrow().any_dirty() {
            return None;
        }
        Some(self.autosave_last_attempt + AUTOSAVE_IDLE)
    }

    /// Writes every dirty tab now, for the window-close path. Failures are
    /// logged to stderr and swallowed — the window must always close, and
    /// there is nothing left to retry once the loop has exited.
    pub fn save_all(&mut self) {
        for (path, error) in self.docs.borrow_mut().save_all() {
            eprintln!("save failed for {}: {error}", path.display());
        }
    }

    /// Attaches a layer to each overlay that is open and drops the layer of
    /// each that is not.
    ///
    /// Every layer is a full-surface render target regardless of what is
    /// drawn into it, and these overlays are closed for essentially the whole
    /// session. Held for the session they were the largest block of dead GPU memory in the
    /// app; the renderer's stack holds only a `Weak`, so dropping the handle
    /// is the whole of the deallocation.
    ///
    /// A re-attached layer goes back on *top* of the stack rather than to
    /// its original position, which is the right place for an overlay: the
    /// one opened last draws over the rest.
    fn sync_overlay_layers(&mut self, renderer: &mut Renderer) {
        let overlays = [
            (self.onboard_region, self.onboarding),
            (self.dialog_region, self.dialog.is_some()),
            (self.palette_region, self.palette.is_some()),
            (self.slash_region, self.slash_menu.is_some()),
            (self.finder_region, self.finder.is_some()),
            (self.menu_region, self.context_menu.is_some()),
            (self.math_menu_region, self.math_menu.is_some()),
        ];
        for (index, open) in overlays {
            match (open, self.regions[index].is_attached()) {
                (true, false) => {
                    let layer = renderer.new_layer_top(LayerInvalidation::Manual);
                    self.regions[index].attach(layer);
                }
                (false, true) => self.regions[index].detach(),
                _ => {}
            }
        }
    }

    /// Asks for a palette swap, expanding from `origin`. Ignored while one
    /// is already running — see [`ThemeSwap`] for why that lock exists.
    pub(super) fn request_theme_swap(&mut self, origin: (f32, f32)) {
        if self.theme_swap.is_none() {
            self.theme_swap = Some(ThemeSwap::Requested { origin });
        }
    }

    /// Drives the palette swap one frame, and reports whether it wants
    /// another. Each arm is one step of the sequence [`ThemeSwap`]
    /// describes; the state is taken rather than borrowed so every arm can
    /// end the swap simply by not putting one back.
    fn sync_theme_swap(&mut self, renderer: &mut Renderer, viewport: Rect, dt: Duration) -> bool {
        let Some(swap) = self.theme_swap.take() else {
            return false;
        };
        match swap {
            ThemeSwap::Requested { origin } => {
                // No capture, no still image to wipe, and a wipe over a
                // window already in the new palette would be a grey circle
                // sweeping over nothing. Swap outright instead.
                if !renderer.supports_capture() {
                    theme::set(theme::counterpart());
                    return false;
                }
                // Empty, and stays empty: the layer exists to be blitted
                // into, and its own render pass is skipped from the moment
                // the capture lands.
                let layer = renderer.new_layer_top(LayerInvalidation::Manual);
                renderer.capture_into(&layer);
                self.theme_swap = Some(ThemeSwap::Capturing { origin, layer });
                true
            }
            ThemeSwap::Capturing { origin, layer } => {
                theme::set(theme::counterpart());
                // The capture can be refused — a resize between the request
                // and the frame that served it leaves the layer the wrong
                // size. The palette still moves; it just moves at once.
                if !layer.is_frozen() {
                    return false;
                }
                // Radius zero on the first frame: the still image covers the
                // window exactly, which is what makes the swap underneath it
                // invisible.
                wipe(&layer, origin, viewport, 0.0);
                self.theme_swap = Some(ThemeSwap::Wiping {
                    origin,
                    layer,
                    animation: Animation::new(SWAP_DURATION, Easing::EaseOut),
                });
                true
            }
            ThemeSwap::Wiping {
                origin,
                layer,
                mut animation,
            } => {
                let running = animation.advance(dt);
                wipe(&layer, origin, viewport, animation.weight());
                if running {
                    self.theme_swap = Some(ThemeSwap::Wiping {
                        origin,
                        layer,
                        animation,
                    });
                }
                // Not putting the swap back drops the layer, and with it the
                // captured frame, the wipe's output texture, and its place
                // in the composite. Nothing is left allocated for a
                // transition that has finished.
                running
            }
        }
    }

    /// Answers a palette swap, if one is pending.
    ///
    /// A theme change is the one event that invalidates the whole window at
    /// once, and it invalidates it in three places:
    ///
    /// 1. the cached document layout, which carries a colour in every
    ///    [`TextStyle`] it holds;
    /// 2. the view components, which are built from that layout and from
    ///    colours of their own (the mode badge's fill, for one);
    /// 3. every region's retained layer, which holds pixels painted in the
    ///    palette that just went away.
    ///
    /// None of the three can notice on its own — a swap moves no revision, no
    /// rect, and no component's state — so the flag from
    /// [`ThemeServer::take_change`] is what stands in for all of them. Taking
    /// it here, before the frame's sync and draw passes, means the new
    /// palette lands on the same frame the swap was asked for.
    ///
    /// [`TextStyle`]: crate::theme::TextStyle
    /// [`ThemeServer::take_change`]: crate::theme::ThemeServer::take_change
    fn sync_theme(&mut self) {
        if !theme::take_change() {
            return;
        }
        self.doc_layout = None;
        self.rebuild_views();
        for region in &mut self.regions {
            region.poke();
        }
    }

    fn panels(&self) -> [&Panel; 4] {
        [&self.tree, &self.sidenotes, &self.topics, &self.status]
    }

    fn panels_mut(&mut self) -> [&mut Panel; 4] {
        [
            &mut self.tree,
            &mut self.sidenotes,
            &mut self.topics,
            &mut self.status,
        ]
    }

    /// Every live divider and the grab zone straddling it. A collapsed panel
    /// has no divider — there is nothing there to drag.
    fn dividers(&self) -> Vec<(Divider, Rect)> {
        let mut out = Vec::new();
        for (which, panel, from_right) in [
            (Divider::Tree, &self.tree, false),
            (Divider::Sidenotes, &self.sidenotes, true),
            (Divider::Topics, &self.topics, true),
        ] {
            let rect = self.layout.rect(panel.node);
            if !panel.open || rect.width < 1.0 {
                continue;
            }
            let edge = if from_right { rect.x } else { rect.right() };
            out.push((which, grab_zone(edge, rect)));
        }
        out
    }

    /// The divider under `point`, if any.
    fn divider_at(&self, point: (f32, f32)) -> Option<Divider> {
        self.dividers()
            .into_iter()
            .find(|(_, zone)| zone.contains(point))
            .map(|(which, _)| which)
    }

    fn panel_mut(&mut self, which: Divider) -> &mut Panel {
        match which {
            Divider::Tree => &mut self.tree,
            Divider::Sidenotes => &mut self.sidenotes,
            Divider::Topics => &mut self.topics,
        }
    }

    /// The active document laid out at the given width, cached by
    /// `(revision, width)`. The measure source is the editor region's own
    /// layer, so what is measured here is exactly what the editor draws.
    fn current_layout(&mut self, width: f32) -> Rc<DocLayout> {
        let revision = self.docs.borrow().revision();
        if let Some((r, w, layout)) = &self.doc_layout
            && *r == revision
            && *w == width
        {
            return layout.clone();
        }
        let layout = {
            let docs = self.docs.borrow();
            match docs.active() {
                Some(tab) => {
                    let layer = self.regions[self.text_region].layer();
                    crate::document::layout::layout(&tab.document, width, &|text, style| {
                        theme::width(layer, text, style)
                    })
                }
                None => DocLayout {
                    blocks: Vec::new(),
                    height: 0.0,
                    scale: 1.0,
                    source: Vec::new(),
                    anchors: Vec::new(),
                },
            }
        };
        let layout = Rc::new(layout);
        self.doc_layout = Some((revision, width, layout.clone()));
        layout
    }

    /// How far the editor can scroll — the layout's total height, which the
    /// old line-counting estimate could not see.
    fn editor_max_scroll(&mut self) -> f32 {
        let rect = self.layout.rect(self.text_column);
        match &self.doc_layout {
            Some((_, _, layout)) => editor::max_scroll(layout.height, rect.height - editor::TOP),
            None => 0.0,
        }
    }

    /// Brings the caret's visual line into the editor's visible band, if
    /// scrolling is needed to do it. Only called when the caret actually
    /// moved — a wheel scroll must be able to take the caret out of view.
    /// When focus is a note, the thing that must stay on screen is that note,
    /// so its placed rectangle is fed in instead of the page caret; the
    /// margin and the page share one scroll offset, so this is a change of
    /// which band is followed, not a second scroll.
    fn ensure_caret_visible(&mut self) {
        let rect = self.layout.rect(self.text_column);
        let band = match self.focused_note_band() {
            Some(band) => band,
            None => {
                let docs = self.docs.borrow();
                let Some(layout) = self
                    .doc_layout
                    .as_ref()
                    .map(|(_, _, layout)| layout.clone())
                else {
                    return;
                };
                let Some(tab) = docs.active() else {
                    return;
                };
                layout.caret_band(tab.document.caret)
            }
        };
        let scroll = self.docs.borrow().editor_scroll;
        let max = self.editor_max_scroll();
        let scroll = editor::follow_scroll(
            scroll,
            band.0,
            band.1,
            rect.y + editor::TOP,
            rect.bottom(),
            max,
        );
        self.docs.borrow_mut().set_editor_scroll(scroll);
    }

    /// The focused note's placed band in document coordinates — `(top,
    /// bottom)` — or `None` when focus is the body. Runs the same stacking
    /// pass the margin draws from, so the band is the note's real rectangle.
    fn focused_note_band(&mut self) -> Option<(f32, f32)> {
        let label = {
            let docs = self.docs.borrow();
            let tab = docs.active()?;
            let Focus::Note(i) = tab.document.focus else {
                return None;
            };
            tab.document.notes.get(i).map(|note| note.label.clone())?
        };
        self.stacked_notes()
            .into_iter()
            .find(|(note_label, _, _, _)| *note_label == label)
            .map(|(_, _, y, layout)| (y, y + layout.height))
    }

    fn editor_point(&self, rect: Rect, mouse: (f32, f32)) -> Option<(f32, f32)> {
        let docs = self.docs.borrow();
        docs.active()?;
        let local_x = (mouse.0 - (rect.x + editor::INSET)).max(0.0);
        let local_y = mouse.1 - rect.y - editor::TOP + docs.editor_scroll;
        if local_y < 0.0 {
            return None;
        }
        Some((local_x, local_y))
    }

    /// Mouse position within the editor: the caret nearest the cursor.
    fn caret_at(&mut self, rect: Rect, mouse: (f32, f32)) -> Option<Caret> {
        let layout = self.current_layout(Editor::content_width(rect));
        let (local_x, local_y) = self.editor_point(rect, mouse)?;
        let layer = self.regions[self.text_region].layer();
        let measure = |text: &str, style: &TextStyle| theme::width(layer, text, style);
        Some(layout.hit(local_x, local_y, &measure))
    }

    /// Mouse position within a rendered math atom and its nearest cursor.
    fn math_at(&mut self, rect: Rect, mouse: (f32, f32)) -> Option<(usize, usize, MathCursor)> {
        let layout = self.current_layout(Editor::content_width(rect));
        let (local_x, local_y) = self.editor_point(rect, mouse)?;
        let layer = self.regions[self.text_region].layer();
        let measure = |text: &str, style: &TextStyle| theme::width(layer, text, style);
        layout.hit_math(local_x, local_y, &measure)
    }

    fn context_at(&mut self, rect: Rect, mouse: (f32, f32)) -> Option<ContextHit> {
        let layout = self.current_layout(Editor::content_width(rect));
        let (local_x, local_y) = self.editor_point(rect, mouse)?;
        let layer = self.regions[self.text_region].layer();
        let measure = |text: &str, style: &TextStyle| theme::width(layer, text, style);
        layout.hit_context(local_x, local_y, &measure)
    }

    /// Temporary Normal-click highlight. It exists only as part of the open
    /// context menu state, so closing the popup also clears the selection.
    fn context_selection(
        &self,
    ) -> (
        Option<crate::document::FlatRange>,
        bool,
        Option<(usize, usize, NodeAddress)>,
    ) {
        let target = self
            .context_menu
            .as_ref()
            .and_then(|state| state.target.as_ref());
        match target {
            Some(ContextHit::Range { range, kind }) => {
                (Some(*range), *kind == RangeKind::CodeBlock, None)
            }
            Some(ContextHit::Math {
                block,
                inline,
                node: Some(node),
            }) => (None, false, Some((*block, *inline, node.clone()))),
            Some(ContextHit::Math {
                block,
                inline,
                node: None,
            }) => {
                let range = self.docs.borrow().active().and_then(|tab| {
                    let block_ref = tab.document.body().get(*block)?;
                    matches!(
                        block_ref.inlines().get(*inline),
                        Some(crate::document::Inline::Math(_))
                    )
                    .then(|| {
                        let offset = block_ref.inlines()[..*inline]
                            .iter()
                            .map(|run| match run {
                                crate::document::Inline::Text(text) => text.text.chars().count(),
                                crate::document::Inline::Math(_) => 1,
                                crate::document::Inline::Note(_) => 1,
                            })
                            .sum();
                        crate::document::FlatRange::new(
                            crate::document::FlatPos {
                                block: *block,
                                offset,
                            },
                            crate::document::FlatPos {
                                block: *block,
                                offset: offset + 1,
                            },
                        )
                    })
                });
                (range, false, None)
            }
            None => (None, false, None),
        }
    }

    /// The breadcrumb for the active tab: vault name, then the file's
    /// folders, then the file and the heading trail under the caret. Falls
    /// back to the vault name.
    fn crumb(&self) -> Vec<String> {
        let mut out = vault_name(&self.vault);
        let root = self
            .vault
            .as_ref()
            .map(|vault| vault.borrow().root().to_path_buf());
        let active = {
            let docs = self.docs.borrow();
            docs.active()
                .map(|tab| (tab.path().to_path_buf(), tab.document.caret.block))
        };
        if let Some((path, _)) = active.as_ref()
            && let Some(root) = root
            && let Ok(relative) = path.strip_prefix(&root)
        {
            out.extend(
                relative
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned()),
            );
        }
        // Headings join same list because last crumb is current place, and
        // deepest heading is exactly that place inside document.
        let nodes = self.outline();
        if let Some((_, caret_block)) = active {
            out.extend(
                outline::trail(&nodes, caret_block)
                    .iter()
                    .filter(|node| !node.text.is_empty())
                    .map(|node| format!("{} {}", node.number, node.text).trim().to_owned()),
            );
        }
        out
    }

    /// The active document's heading outline. Derived on demand — nothing
    /// in the model stores it, and it is O(blocks) over a document that is
    /// already walked every rebuild.
    fn outline(&self) -> Vec<outline::Node> {
        self.docs
            .borrow()
            .active()
            .map(|tab| outline::outline(tab.document.body()))
            .unwrap_or_default()
    }

    /// The margin's notes stacked beside their anchors, as `(label, number,
    /// placed y, layout)`. Runs in `rebuild_views`, so the resolution may lag
    /// the text a frame but never animates (spec §12.2): a note jumps to its
    /// place rather than sliding. Shared by `sidenote_notes` and the margin's
    /// click handler, so a click can never land on a note the view drew at a
    /// different y.
    fn stacked_notes(&mut self) -> Vec<(String, String, f32, Rc<DocLayout>)> {
        let width = Editor::content_width(self.layout.rect(self.text_column));
        let layout = self.current_layout(width);

        // Resolve every borrow into a plain local first: a `Ref` from
        // `docs.borrow()` held across the stacking below would still be
        // alive there, and the `set_component` call this feeds would then
        // find the `RefCell` already borrowed.
        let anchored: Vec<(String, String, f32)> = layout
            .anchors
            .iter()
            .map(|anchor| (anchor.label.clone(), anchor.number.clone(), anchor.y))
            .collect();
        let bodies: Vec<Vec<crate::document::Block>> = {
            let docs = self.docs.borrow();
            match docs.active() {
                Some(tab) => anchored
                    .iter()
                    .filter_map(|(label, _, _)| {
                        tab.document
                            .notes
                            .iter()
                            .find(|note| note.label == *label)
                            .map(|note| note.body.clone())
                    })
                    .collect(),
                None => Vec::new(),
            }
        };

        // Each note is laid out at the margin's width and scale with the same
        // measure the margin draws with, so a note's height is its own
        // layout's, not a second wrapper's guess.
        let layer = self.regions[self.text_region].layer();
        let measure = |text: &str, style: &TextStyle| theme::width(layer, text, style);
        let mut laid: Vec<(Rc<DocLayout>, f32)> = Vec::with_capacity(anchored.len());
        for (body, (_, _, y)) in bodies.iter().zip(&anchored) {
            let note_layout = Rc::new(layout_blocks(
                body,
                sidenotes::NOTE_WIDTH,
                sidenotes::SCALE,
                &measure,
            ));
            laid.push((note_layout, *y));
        }
        let wanted: Vec<(f32, f32)> = laid.iter().map(|(layout, y)| (*y, layout.height)).collect();
        let ys = sidenotes::stack(&wanted, sidenotes::GAP);

        anchored
            .into_iter()
            .zip(laid)
            .zip(ys)
            .map(|(((label, number, _), (layout, _)), y)| (label, number, y, layout))
            .collect()
    }

    /// The margin's notes as components. The focused note — the one whose
    /// label `Document::focus` names — is the only one handed a caret, so it
    /// is the only one that draws one.
    fn sidenote_notes(&mut self) -> Vec<Note> {
        let (focused_label, note_caret, block_caret) = {
            let docs = self.docs.borrow();
            let block_caret =
                !matches!(self.vim.current_mode(), VimMode::Insert | VimMode::Command);
            match docs.active() {
                Some(tab) => match tab.document.focus {
                    Focus::Note(i) => (
                        tab.document.notes.get(i).map(|note| note.label.clone()),
                        Some(tab.document.caret),
                        block_caret,
                    ),
                    Focus::Body => (None, None, block_caret),
                },
                None => (None, None, false),
            }
        };
        self.stacked_notes()
            .into_iter()
            .map(|(label, number, y, layout)| {
                let caret = if focused_label.as_deref() == Some(label.as_str()) {
                    note_caret
                } else {
                    None
                };
                Note::new(&number, layout, y, caret, block_caret)
            })
            .collect()
    }

    /// Rebuilds the view regions (tab strip, breadcrumb, editor, status
    /// line) from a live snapshot. Called when [`Tabs::revision`] moves, and
    /// after the picker swaps the vault.
    fn rebuild_views(&mut self) {
        // The fresh layout first: `ensure_caret_visible` below reads it.
        let width = Editor::content_width(self.layout.rect(self.text_column));
        let _ = self.current_layout(width);

        // Snap to the caret only when the caret (or the active tab) moved.
        // A pure wheel scroll must keep its offset — following here would
        // fight the user's scrolling every frame.
        let caret_key = {
            let docs = self.docs.borrow();
            (
                docs.active_index().unwrap_or(usize::MAX),
                docs.active().map_or(Focus::Body, |tab| tab.document.focus),
                docs.active().map_or(
                    Caret {
                        block: 0,
                        inline: 0,
                        offset: 0,
                        style: crate::document::Style::PLAIN,
                    },
                    |tab| tab.document.caret,
                ),
            )
        };
        if caret_key != self.followed {
            self.followed = caret_key;
            self.ensure_caret_visible();
            // A caret being moved or typed at should read as steady, not
            // strobing — restarting snaps the blink to weight 0, which
            // `Context::caret_on` reads as solid-on, for a full half-cycle.
            self.caret.restart();
        }

        let (tabs, active) = {
            let docs = self.docs.borrow();
            let tabs = docs
                .tabs
                .iter()
                .map(|tab| TabView {
                    name: tab.name().to_string(),
                    preview: tab.preview,
                    unsaved: tab.document.is_dirty(),
                })
                .collect();
            (tabs, docs.active_index())
        };
        self.regions[self.tab_region].set_component(Box::new(TabStrip::new(
            self.docs.clone(),
            tabs,
            active,
        )));

        let crumb = self.crumb();
        self.regions[self.breadcrumb_region].set_component(Box::new(Breadcrumb::new(crumb)));

        let caret_block = {
            let docs = self.docs.borrow();
            docs.active()
                .map(|tab| tab.document.caret.block)
                .unwrap_or(0)
        };
        let nodes = self.outline();
        let active = outline::active(&nodes, caret_block).unwrap_or(0);
        let entries = nodes
            .iter()
            .map(|node| Entry::new(&node.number, &node.text, node.depth))
            .collect();
        self.regions[self.topics_region].set_component(Box::new(Topics::new(entries, active)));

        // The badge and the caret shape both read peripherally, so both get
        // a colour/shape pair rather than just a label.
        let (mode_label, mode_color) = match self.vim.current_mode() {
            VimMode::Normal => ("NORMAL", theme::cool()),
            VimMode::Insert => ("INSERT", theme::accent()),
            VimMode::VisualChar => ("VISUAL", theme::accent()),
            VimMode::VisualLine => ("V-LINE", theme::accent()),
            VimMode::Command => ("COMMAND", theme::accent()),
        };
        let block_caret = !matches!(self.vim.current_mode(), VimMode::Insert | VimMode::Command);
        let command = self.search.as_ref().map_or_else(String::new, |search| {
            format!("{}{}", if search.forward { "/" } else { "?" }, search.query)
        });
        let visual_selection = self.current_selection();
        let (context_selection, context_line, math_selection) = self.context_selection();
        let selection = visual_selection.or(context_selection);
        let line_selection = if visual_selection.is_some() {
            self.vim.visual_mode() == Some(VisualMode::Line)
        } else {
            context_line
        };

        let (editor, status) = {
            // The layout cache is fresh from the top of this function; grab
            // the shared `Rc` before borrowing the tabs.
            let width = Editor::content_width(self.layout.rect(self.text_column));
            let layout = self.current_layout(width);
            let scroll = self.docs.borrow().editor_scroll;
            let mut docs = self.docs.borrow_mut();
            match docs.active_mut() {
                Some(tab) => {
                    let caret = tab.document.caret;
                    // The page draws the caret only while the body is focused;
                    // when a note is focused the caret is note-relative and the
                    // focused note's editor draws it instead.
                    let page_caret = match tab.document.focus {
                        Focus::Body => Some(caret),
                        Focus::Note(_) => None,
                    };
                    let math_path = if tab.document.math.is_some() {
                        let mut path = vec!["math"];
                        path.extend(tab.document.math_path_names());
                        path.join(" › ")
                    } else {
                        String::new()
                    };
                    let math = tab.document.math.clone();
                    (
                        Editor::new(
                            layout,
                            page_caret,
                            scroll,
                            block_caret,
                            caret.style,
                            editor::Metrics::PAGE,
                        )
                        .with_math(math)
                        .with_math_selection(math_selection)
                        .with_context_selections(self.brush_selected.clone(), self.brush_point)
                        .with_selection(selection, line_selection),
                        StatusLine::new(
                            mode_label,
                            mode_color,
                            tab.name().to_string(),
                            if tab.document.is_dirty() {
                                "unsaved changes".into()
                            } else {
                                "saved".into()
                            },
                            format!("{} words", tab.document.word_count()),
                            self.show_stats,
                            command.clone(),
                        )
                        .with_math_path(math_path),
                    )
                }
                None => (
                    Editor::placeholder(),
                    StatusLine::new(
                        mode_label,
                        mode_color,
                        "no file open".into(),
                        String::new(),
                        String::new(),
                        self.show_stats,
                        command,
                    ),
                ),
            }
        };
        self.regions[self.text_region].set_component(Box::new(editor.with_glow(self.glow.clone())));
        self.regions[self.status_region].set_component(Box::new(status));

        let sidenote_notes = self.sidenote_notes();
        let scroll = self.docs.borrow().editor_scroll;
        self.regions[self.sidenote_region].set_component(Box::new(SidenoteMargin::new(
            sidenote_notes,
            editor::TOP,
            scroll,
        )));
    }
}

/// Whether the idle autosave is due: a dirty document whose last activity —
/// an edit or a save attempt — was at least [`AUTOSAVE_IDLE`] ago. Kept as a
/// pure function so the debounce is testable without running frames.
/// Set the wipe on the layer holding the captured frame: a hole centred on
/// `origin` that has eaten none of it at weight 0 and all of it at weight 1.
///
/// The radius runs from just under zero to past the furthest corner, both
/// by half the feather, so the soft edge is fully outside the window at
/// each end — otherwise weight 0 would show a faint disc at the switch
/// before the wipe had started, and weight 1 would leave a smudge in the
/// far corner after it had finished.
///
/// Re-setting the same effect with a new radius patches a uniform rather
/// than rebuilding the layer's effect, so this costs a buffer write per
/// frame — see `Layer::set_effect`.
fn wipe(layer: &Layer, origin: (f32, f32), viewport: Rect, weight: f32) {
    let corners = [
        (viewport.x, viewport.y),
        (viewport.right(), viewport.y),
        (viewport.right(), viewport.bottom()),
        (viewport.x, viewport.bottom()),
    ];
    let reach = corners
        .iter()
        .map(|(x, y)| (x - origin.0).hypot(y - origin.1))
        .fold(0.0f32, f32::max);
    let span = reach + SWAP_FEATHER;
    layer.set_effect(Some(ShaderEffect::RadialWipe {
        center: origin,
        radius: weight * span - SWAP_FEATHER / 2.0,
        feather: SWAP_FEATHER,
        keep_inside: false,
    }));
}

fn autosave_due(last_attempt: Instant, now: Instant, dirty: bool) -> bool {
    dirty && now.saturating_duration_since(last_attempt) >= AUTOSAVE_IDLE
}

/// The vault's display name, for the breadcrumb.
fn vault_name(vault: &Option<Rc<RefCell<Vault>>>) -> Vec<String> {
    match vault {
        Some(vault) => vault
            .borrow()
            .root()
            .file_name()
            .map(|n| vec![n.to_string_lossy().into_owned()])
            .unwrap_or_default(),
        None => vec!["no vault".into()],
    }
}

/// The grab zone straddling a vertical edge at `x`, spanning `rect`'s height.
fn grab_zone(x: f32, rect: Rect) -> Rect {
    Rect::new(x - GRAB, rect.y, GRAB * 2.0 + 1.0, rect.height)
}

/// The width a drag to `mouse_x` is asking for. A panel on the right of the
/// canvas is measured from its own right edge, so dragging left makes
/// it wider — the mirror image of the tree.
fn dragged_width(rect: Rect, mouse_x: f32, from_right: bool) -> f32 {
    if from_right {
        rect.right() - mouse_x
    } else {
        mouse_x - rect.x
    }
}

#[cfg(test)]
mod tests {
    use super::{AUTOSAVE_IDLE, autosave_due, dragged_width, grab_zone};
    use crate::layout::Rect;
    use std::time::{Duration, Instant};

    #[test]
    fn an_edit_restarts_the_idle_clock() {
        let first = Instant::now();
        // A fresh edit is not yet due well inside the idle window.
        assert!(!autosave_due(
            first,
            first + Duration::from_millis(500),
            true
        ));
        // A second edit later restarts the clock: even though the first edit
        // is now older than AUTOSAVE_IDLE, the document has only just
        // changed again.
        let second = first + Duration::from_millis(1500);
        assert!(!autosave_due(
            second,
            second + Duration::from_millis(500),
            true
        ));
        // Once that second edit sits still, it fires.
        assert!(autosave_due(second, second + AUTOSAVE_IDLE, true));
    }

    #[test]
    fn autosave_fires_once_the_document_sits_still() {
        let edit = Instant::now();
        assert!(!autosave_due(edit, edit, true));
        assert!(!autosave_due(
            edit,
            edit + AUTOSAVE_IDLE - Duration::from_millis(1),
            true
        ));
        assert!(autosave_due(edit, edit + AUTOSAVE_IDLE, true));
        assert!(autosave_due(
            edit,
            edit + AUTOSAVE_IDLE + Duration::from_secs(1),
            true
        ));
    }

    #[test]
    fn a_clean_document_never_schedules_a_save() {
        let edit = Instant::now();
        assert!(!autosave_due(edit, edit + AUTOSAVE_IDLE * 10, false));
    }

    #[test]
    fn a_failed_save_waits_a_full_idle_window_before_retrying() {
        // A save attempt restarts the clock exactly like an edit does, so a
        // tab that stays dirty after a failed write is not due again until a
        // full idle window has passed — not the very next frame.
        let attempt = Instant::now();
        assert!(!autosave_due(
            attempt,
            attempt + AUTOSAVE_IDLE - Duration::from_millis(1),
            true
        ));
        assert!(autosave_due(attempt, attempt + AUTOSAVE_IDLE, true));
    }

    #[test]
    fn a_dirty_tab_with_no_path_does_not_retry_every_frame() {
        // A pathless dirty tab is skipped and never written, so it stays
        // dirty with nothing failed — but the attempt still restarts the
        // clock, so it is not due again until a full idle window has passed.
        let attempt = Instant::now();
        assert!(!autosave_due(
            attempt,
            attempt + Duration::from_millis(1),
            true
        ));
        assert!(!autosave_due(
            attempt,
            attempt + AUTOSAVE_IDLE - Duration::from_millis(1),
            true
        ));
        assert!(autosave_due(attempt, attempt + AUTOSAVE_IDLE, true));
    }

    #[test]
    fn a_right_hand_panel_grows_when_its_edge_is_dragged_left() {
        assert_eq!(
            dragged_width(Rect::new(100.0, 0.0, 250.0, 800.0), 300.0, false),
            200.0
        );
        assert_eq!(
            dragged_width(Rect::new(1000.0, 0.0, 250.0, 800.0), 1100.0, true),
            150.0
        );
        assert_eq!(
            dragged_width(Rect::new(1000.0, 0.0, 250.0, 800.0), 900.0, true),
            350.0
        );
        let zone = grab_zone(400.0, Rect::new(0.0, 0.0, 250.0, 800.0));
        assert!(zone.contains((400.0, 10.0)));
        assert!(zone.contains((397.0, 10.0)));
        assert!(zone.contains((403.0, 10.0)));
        assert!(!zone.contains((390.0, 10.0)));
    }
}
