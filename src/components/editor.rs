//! The active file, drawn from a [`DocLayout`]: styled, wrapped, no markup
//! visible. The shell lays out the open document and hands it over as an
//! [`Rc`]; this component only reads it.

use std::collections::HashMap;
use std::rc::Rc;

use crate::document::layout::{self, ContextHit, DocLayout, RangeKind};
use crate::document::math::{MathCursor, NodeAddress, SymbolRole};
use crate::document::math_layout::{self, BoxKind, MathBox, MathPrimitive};
use crate::document::{ATOM, BadgeColor, Block, Caret, FlatRange, Inline, ListMarker, Style};
use crate::layout::Rect;
use crate::renderer::{Layer, LineCap, LineJoin, PathPaint, Rounding, Stroke};
use crate::theme::{self, TextStyle};
use crate::ui::{Component, Context, Dirty};

/// Top padding of the content area (the filename title is gone — the
/// document's own H1 is the title).
pub const TOP: f32 = 28.0;
pub const INSET: f32 = 56.0;
/// The gutter between a heading's auto-number and the text it belongs to.
/// The number is drawn right-aligned against this, so numbers of different
/// depths line up on their right edge instead of ragging.
const NUMBER_GUTTER: f32 = 12.0;
/// The auto-number's size. Constant rather than scaled per heading level:
/// it is a margin annotation, not part of the heading's own typography.
const NUMBER_SIZE: f32 = 11.0;
/// The equation number's size — the same margin-annotation register as the
/// heading auto-number, never part of the math's own typography.
const EQ_NUMBER_SIZE: f32 = 11.0;
/// Room kept between the equation number and the band's right edge, so the
/// widest reading of the band never crowds it.
const EQ_NUMBER_INSET: f32 = 18.0;
/// A bullet's dot: inset from the content column and sized to read as a
/// mark, not as a glyph. `BULLET_INSET` is from the column's left edge to
/// the dot's centre.
const BULLET_INSET: f32 = 11.0;
const BULLET_RADIUS: f32 = 2.5;
/// A task checkbox's corner radius — between the row radius and a code
/// span's, so it reads as a control, not as a box of text.
const CHECK_RADIUS: f32 = 4.5;
const CHECK_ROUNDING: Rounding = Rounding::uniform(CHECK_RADIUS);
/// How high a done task's strike sits above the line's centre — through
/// the x-height, like a pen, not along the baseline.
const STRIKE_RISE: f32 = 4.5;
const STRIKE_THICKNESS: f32 = 1.4;
/// The design's measure: the content column is never wider than this.
pub const MEASURE: f32 = 634.0;
/// Space kept past the right edge before a line may wrap.
pub const RIGHT_MARGIN: f32 = 24.0;
/// Radius of the Ctrl-drag selection brush in logical pixels.
pub const BRUSH_RADIUS: f32 = 9.0;
/// Padding of the tint behind code: a fenced block gets the generous one,
/// an inline span the tight one, so a `` `run` `` mid-sentence doesn't push
/// the line apart.
const BLOCK_PAD: (f32, f32) = (10.0, 6.0);
const INLINE_PAD: (f32, f32) = (8.0, 4.0);
const CODE_ROUNDING: Rounding = Rounding::uniform(6.0);
const MATH_SELECTION_ROUNDING: Rounding = Rounding::uniform(4.0);
/// The highlight bar: an underline, not a wash, so the glyphs keep the
/// page's own contrast. `DROP` is measured down from the line's centre.
const HIGHLIGHT_DROP: f32 = 8.0;
const HIGHLIGHT_THICKNESS: f32 = 2.5;
const HIGHLIGHT_PAD: f32 = 2.0;
const HIGHLIGHT_ROUNDING: Rounding = Rounding::uniform(1.25);
/// The halo drawn on the glow layer: a fatter, fainter copy of the bar that
/// the layer's blur turns into the falloff. One shader pass over one layer,
/// rather than a stack of hand-faded rectangles. Kept close to the bar —
/// the glow is meant to read as the mark's own light, not as a second
/// object under it.
const GLOW_SPREAD: f32 = 1.0;
const GLOW_ALPHA: f32 = 0.45;
pub const GLOW_RADIUS: f32 = 2.5;
/// Truncate a segment's drawing to this many characters before shaping it —
/// a single pathological line must cost the same as a normal one, not
/// O(line length).
pub const VIEW_CAP: usize = 8000;

/// How much empty space past the last line the editor may scroll into, as
/// a fraction of the visible height. Writing at the end of a document
/// should not mean writing on the bottom edge of the screen.
pub const OVERSCROLL: f32 = 0.5;

/// The numbers an editor draws with. The page has one value for each; a
/// note embedded in the margin has its own. Both are named, so the two
/// sites never drift into a shared "the" size that only one of them wants.
pub struct Metrics {
    /// Left padding of the content area.
    pub inset: f32,
    /// Top padding of the content area.
    pub top: f32,
    /// The content column is never wider than this.
    pub measure: f32,
    /// Space kept past the right edge before a line may wrap.
    pub right_margin: f32,
    /// Whether this editor is the page: it fills its rect with the page
    /// background and draws the current-line band and the caret. An editor
    /// embedded inside another component draws none of that — its container
    /// already painted, and a band or caret spanning someone else's rect is
    /// a second claim about what the rect is.
    pub page: bool,
}

impl Metrics {
    /// The text pane's metrics, unchanged from before these were instance
    /// state: the page editor draws by exactly the values it always did.
    pub const PAGE: Metrics = Metrics {
        inset: INSET,
        top: TOP,
        measure: MEASURE,
        right_margin: RIGHT_MARGIN,
        page: true,
    };

    /// The content column's width in `rect`: the measure, or whatever fits.
    pub fn content_width(&self, rect: Rect) -> f32 {
        (rect.width - self.inset - self.right_margin).clamp(0.0, self.measure)
    }
}

/// One measured piece of a visual line: `(text, style, x, width)`.
type Painted = (String, Style, f32, f32);

/// Draws a laid-out expression. `origin` is the box's baseline-left point
/// on screen; child offsets are baseline-relative with y positive upward,
/// so descending into a child subtracts its y.
///
/// `slots` draws the empty-slot placeholders. They are typing affordances —
/// they say where the next character lands — so they appear only while the
/// caret is actually inside this expression in Insert mode; read back later,
/// an expression shows its notation and nothing else. Their geometry is
/// reserved either way, so an expression never resizes as the caret enters
/// or leaves it.
fn draw_math(layer: &Layer, box_: &MathBox, origin: (f32, f32), slots: bool) {
    draw_math_inner(layer, box_, origin, false, slots);
}

fn math_rect(box_: &MathBox, origin: (f32, f32)) -> Rect {
    Rect {
        x: origin.0,
        y: origin.1 - box_.ascent,
        width: box_.width,
        height: box_.ascent + box_.descent,
    }
}

fn draw_math_selection(
    layer: &Layer,
    list: &crate::document::math::MathList,
    address: Option<&NodeAddress>,
    box_: &MathBox,
    origin: (f32, f32),
    scale: f32,
    measure: &dyn Fn(&str, &TextStyle) -> f32,
) {
    const PAD: f32 = 2.0;
    let rect = if let Some(address) = address {
        let Some(bounds) = math_layout::node_bounds(list, address, 0, scale, measure) else {
            return;
        };
        Rect {
            x: origin.0 + bounds.left - PAD,
            y: origin.1 - bounds.top - PAD,
            width: bounds.right - bounds.left + PAD * 2.0,
            height: bounds.top - bounds.bottom + PAD * 2.0,
        }
    } else {
        let rect = math_rect(box_, origin);
        Rect {
            x: rect.x - PAD,
            y: rect.y - PAD,
            width: rect.width + PAD * 2.0,
            height: rect.height + PAD * 2.0,
        }
    };
    layer.draw_rectangle(
        rect.position(),
        rect.size(),
        theme::selection(),
        MATH_SELECTION_ROUNDING,
    );
}

fn draw_brush(layer: &Layer, center: (f32, f32)) {
    layer.draw_circle(center, BRUSH_RADIUS, theme::fade(theme::accent(), 0.14));
}

fn draw_math_inner(layer: &Layer, box_: &MathBox, origin: (f32, f32), covered: bool, slots: bool) {
    if let Some(role) = box_.highlight.filter(|_| !covered) {
        let height = box_.ascent + box_.descent;
        let color = match role {
            SymbolRole::Variable => theme::variable(),
            SymbolRole::Constant => theme::constant(),
            SymbolRole::Function => theme::function(),
        };
        layer.draw_rectangle(
            (origin.0, origin.1 - box_.ascent),
            (box_.width, height),
            color,
            Rounding::uniform(box_.width.min(height) * 0.45),
        );
    }
    let covered = covered || box_.highlight.is_some();
    match &box_.kind {
        BoxKind::Glyph {
            text,
            size,
            offset_x,
            offset_y,
            condense,
        } => {
            // Math boxes use glyph centers as their baseline for now; LEFT's
            // vertical centering therefore matches the prose baseline draw.
            theme::draw(
                layer,
                text,
                (origin.0 + offset_x, origin.1 - offset_y),
                &TextStyle::math(*size, theme::ink()).condensed(*condense),
                theme::LEFT,
            );
        }
        BoxKind::Bar { thickness } => {
            // A fractional one-pixel rule smears across adjacent rows.
            theme::rule(
                layer,
                (origin.0, (origin.1 - thickness * 0.5).round()),
                box_.width,
                *thickness,
                theme::ink(),
            );
        }
        BoxKind::Slot { visible, .. } => {
            // `visible` is structural — an integral's unasked-for limits are
            // never drawn. `slots` is the mode gate on top of it.
            if !visible || !slots {
                return;
            }
            let rect = Rect {
                x: origin.0,
                y: origin.1 - box_.ascent,
                width: box_.width,
                height: box_.ascent + box_.descent,
            };
            layer.draw_rectangle(rect.position(), rect.size(), theme::alt(), Rounding::NONE);
            theme::outline(layer, rect, theme::non_text());
        }
        BoxKind::Primitive(primitive) => match primitive {
            MathPrimitive::Stroke { path, thickness } => {
                let mut stroke = Stroke::new(theme::ink(), *thickness);
                stroke.cap = LineCap::Round;
                stroke.join = LineJoin::Round;
                layer
                    .draw_path(path, origin, PathPaint::Stroke(stroke))
                    .expect("math paths are generated by layout");
            }
            MathPrimitive::Dots { centers, radius } => {
                for &(x, y) in centers {
                    layer.draw_circle((origin.0 + x, origin.1 + y), *radius, theme::ink());
                }
            }
        },
        BoxKind::Row { children } => {
            for (x, y, child) in children {
                draw_math_inner(layer, child, (origin.0 + x, origin.1 - y), covered, slots);
            }
        }
    }
}

/// A list item's marker, hung in the gutter left of the content column.
/// Virtual like a heading's auto-number: drawn, never laid out, so the
/// caret cannot reach it and it never shifts the text it labels. Bullets
/// are drawn glyphs, numbers are right-aligned mono in a fixed column, and
/// a task's box is a control — filled and ticked when done. `content_x` is
/// the content column the item's text hangs from; `baseline` the line's
/// vertical centre.
fn draw_list_marker(layer: &Layer, marker: &ListMarker, content_x: f32, baseline: f32, scale: f32) {
    match marker {
        ListMarker::Bullet => {
            // Optically centred: a dot a hair above the true centre reads
            // as aligned with lowercase text.
            layer.draw_circle(
                (content_x - BULLET_INSET * scale, baseline - 1.0),
                BULLET_RADIUS * scale,
                theme::accent(),
            );
        }
        ListMarker::Number(n) => {
            theme::draw(
                layer,
                &n.to_string(),
                (content_x - NUMBER_GUTTER, baseline),
                &TextStyle::mono(NUMBER_SIZE, theme::non_text()),
                theme::RIGHT,
            );
        }
        ListMarker::Task { done } => {
            let size = crate::document::layout::CHECK_SIZE * scale;
            let gap = crate::document::layout::CHECK_GAP * scale;
            let rect = Rect {
                x: content_x - gap - size,
                y: baseline - size * 0.5,
                width: size,
                height: size,
            };
            if *done {
                layer.draw_rectangle(
                    rect.position(),
                    rect.size(),
                    theme::fade(theme::accent(), 0.18),
                    CHECK_ROUNDING,
                );
                theme::rounded_outline(
                    layer,
                    rect.inset(0.5),
                    CHECK_RADIUS - 0.5,
                    1.0,
                    theme::accent(),
                );
                theme::polyline(
                    layer,
                    &[
                        (rect.x + size * 0.24, rect.y + size * 0.52),
                        (rect.x + size * 0.42, rect.y + size * 0.70),
                        (rect.x + size * 0.76, rect.y + size * 0.30),
                    ],
                    theme::accent(),
                    1.6,
                );
            } else {
                theme::rounded_outline(
                    layer,
                    rect.inset(0.5),
                    CHECK_RADIUS - 0.5,
                    1.0,
                    theme::non_text(),
                );
            }
        }
    }
}

/// Maximal runs of adjacent pieces matching `pred`, as `(start_x, end_x)`.
/// Adjacent pieces merge into one span so a decoration split across runs —
/// a bold word inside a highlight, two code spans touching — reads as a
/// single mark rather than beading up at every seam.
fn spans(pieces: &[Painted], pred: impl Fn(&Painted) -> bool) -> Vec<(f32, f32)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < pieces.len() {
        if !pred(&pieces[i]) {
            i += 1;
            continue;
        }
        let mut end = i;
        while pieces.get(end + 1).is_some_and(&pred) {
            end += 1;
        }
        out.push((pieces[i].2, pieces[end].2 + pieces[end].3));
        i = end + 1;
    }
    out
}

fn badge_spans(pieces: &[Painted]) -> Vec<(f32, f32, BadgeColor)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < pieces.len() {
        if !pieces[i].1.badge {
            i += 1;
            continue;
        }
        let color = pieces[i].1.badge_color;
        let mut end = i;
        while pieces
            .get(end + 1)
            .is_some_and(|piece| piece.1.badge && piece.1.badge_color == color)
        {
            end += 1;
        }
        out.push((pieces[i].2, pieces[end].2 + pieces[end].3, color));
        i = end + 1;
    }
    out
}

pub struct Editor {
    layout: Rc<DocLayout>,
    /// Where this editor draws: the page's metrics or a note's.
    metrics: Metrics,
    /// Model caret — its position is resolved against the layout at draw.
    /// `None` when this editor has no caret: the page while focus lives in a
    /// note, or an unfocused margin note. The value is the whole signal, so
    /// there is no second flag duplicating [`crate::document::Focus`].
    caret: Option<Caret>,
    /// Content offset in logical pixels.
    scroll: f32,
    /// False draws the "no file open" placeholder instead of a document.
    has_file: bool,
    /// Vim Normal mode: a block over the character under the caret, rather
    /// than Insert's bar between characters.
    block_caret: bool,
    /// The pending style context (SPEC §4.2): what typed text becomes. The
    /// edit showing it as a tiny mono marker is its only affordance.
    caret_style: Style,
    /// Each block's auto-number, indexed the same way `layout.source` is.
    /// Resolved once here rather than per redraw, and derived from the very
    /// blocks that are drawn, so a number can never disagree with the
    /// heading beside it.
    numbers: Vec<Option<String>>,
    selection: Option<FlatRange>,
    line_selection: bool,
    caret_on: bool,
    /// A second layer, composited directly above this one and carrying a
    /// blur — the highlight glow, and nothing else. It is redrawn in step
    /// with this component (see [`Component::draw`]), so it can never hold
    /// a halo for a bar that is no longer there.
    glow: Option<Layer>,
    /// Math cursor, when the caret is inside an atom. Its bar replaces the
    /// document caret — two blinking bars would be two claims about where
    /// typing goes.
    math: Option<MathCursor>,
    math_selection: Option<(usize, usize, NodeAddress)>,
    context_selections: Vec<ContextHit>,
    brush_point: Option<(f32, f32)>,
    dirty: Dirty,
}

impl Editor {
    pub fn new(
        layout: Rc<DocLayout>,
        caret: Option<Caret>,
        scroll: f32,
        block_caret: bool,
        caret_style: Style,
        metrics: Metrics,
    ) -> Self {
        let mut numbers = vec![None; layout.source.len()];
        for node in crate::document::outline::outline(&layout.source) {
            numbers[node.block] = Some(node.number);
        }
        Self {
            layout,
            metrics,
            caret,
            scroll,
            has_file: true,
            block_caret,
            caret_style,
            numbers,
            selection: None,
            line_selection: false,
            math: None,
            math_selection: None,
            context_selections: Vec::new(),
            brush_point: None,
            caret_on: true,
            glow: None,
            dirty: Dirty::new(),
        }
    }

    pub fn with_math(mut self, math: Option<MathCursor>) -> Self {
        self.math = math;
        self
    }

    pub fn with_math_selection(mut self, selection: Option<(usize, usize, NodeAddress)>) -> Self {
        self.math_selection = selection;
        self
    }

    pub fn with_context_selections(
        mut self,
        selections: Vec<ContextHit>,
        brush_point: Option<(f32, f32)>,
    ) -> Self {
        self.context_selections = selections;
        self.brush_point = brush_point;
        self
    }

    /// Attaches the glow layer. Separate from `new` because the shell keeps
    /// the layer across the component swaps that `rebuild_views` does.
    pub fn with_glow(mut self, glow: Layer) -> Self {
        self.glow = Some(glow);
        self
    }

    /// The "no file open" state.
    pub fn placeholder() -> Self {
        Self {
            layout: Rc::new(DocLayout {
                blocks: Vec::new(),
                height: 0.0,
                scale: 1.0,
                source: Vec::new(),
                anchors: Vec::new(),
                equation_numbers: HashMap::new(),
            }),
            metrics: Metrics::PAGE,
            caret: None,
            scroll: 0.0,
            has_file: false,
            block_caret: false,
            caret_style: Style::PLAIN,
            numbers: Vec::new(),
            selection: None,
            line_selection: false,
            math: None,
            math_selection: None,
            context_selections: Vec::new(),
            brush_point: None,
            caret_on: true,
            glow: None,
            dirty: Dirty::new(),
        }
    }

    /// The content column's width in `rect` — the page's, since the shell's
    /// own caret math is always about the text pane.
    pub fn content_width(rect: Rect) -> f32 {
        Metrics::PAGE.content_width(rect)
    }

    /// The laid-out content height: the page's scroll ceiling, and a note's
    /// own height when this editor is the margin showing it.
    pub fn content_height(&self) -> f32 {
        self.layout.height
    }

    /// Whether this editor draws a caret — the page while the body is
    /// focused, or the one focused note in the margin. The same signal the
    /// margin uses to mark a note as focused.
    pub fn has_caret(&self) -> bool {
        self.caret.is_some()
    }

    pub fn with_selection(mut self, selection: Option<FlatRange>, line: bool) -> Self {
        self.selection = selection;
        self.line_selection = line;
        self
    }
}

impl Component for Editor {
    fn measure(&mut self, _: &Layer) -> (f32, f32) {
        // Enough for a readable page; the region itself is flexible.
        (420.0, 320.0)
    }

    fn sync(&mut self, context: &Context) {
        // Only an editor with a caret blinks — the page while the body is
        // focused, or the focused note. The placeholder and an unfocused
        // note must not redraw twice a second for a caret neither draws.
        if self.has_file && self.caret.is_some() {
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
        // The glow layer is ours to manage: nothing else clears it, and a
        // stale halo would outlive the bar it belongs to. Clearing up here
        // covers the placeholder's early return too.
        if let Some(glow) = &self.glow {
            glow.clear();
            glow.set_clip_rect(Some((rect.position(), rect.size())));
        }
        // The page fills its whole rect; an editor embedded in a note does
        // not, because its container already painted.
        if self.metrics.page {
            layer.draw_rectangle(
                rect.position(),
                rect.size(),
                theme::background(),
                Rounding::NONE,
            );
        }

        let x = rect.x + self.metrics.inset;
        if !self.has_file {
            theme::draw(
                layer,
                "No file open",
                (x, rect.y + 56.0),
                &TextStyle::serif(17.5, theme::dim()),
                theme::LEFT,
            );
            theme::draw(
                layer,
                "click a note in the tree — one click previews, two pins it",
                (x, rect.y + 84.0),
                &TextStyle::mono(10.5, theme::comment()),
                theme::LEFT,
            );
            return;
        }

        let content = rect.y + self.metrics.top;

        // The current line's band and the caret are resolved against the same
        // layout, so they cannot drift apart. When there is no caret here —
        // the page while focus lives in a note, or an unfocused margin note —
        // the geometry is zero and neither is drawn.
        let caret = self.caret;
        let (caret_x, caret_baseline, caret_height, band_top, band_bottom) = match caret {
            Some(caret) => {
                let (cx, cb, ch) = self
                    .layout
                    .caret_pos(caret, &|text, style| theme::width(layer, text, style));
                let (bt, bb) = self.layout.caret_band(caret);
                (cx, cb, ch, bt, bb)
            }
            None => (0.0, 0.0, 0.0, 0.0, 0.0),
        };
        let math_focus = caret.and_then(|caret| {
            if self.math.is_some()
                && caret.block < self.layout.source.len()
                && caret.inline < self.layout.source[caret.block].inlines().len()
                && matches!(
                    self.layout.source[caret.block].inlines()[caret.inline],
                    Inline::Math(_)
                )
            {
                self.math.as_ref()
            } else {
                None
            }
        });

        // The current-line band does not blink — it identifies the line the
        // caret is on, regardless of caret visibility. The page's spans the
        // whole column; a note's spans only its own content column, so the
        // marker and rule beside it stay visible.
        if caret.is_some() {
            let (band_x, band_width) = if self.metrics.page {
                (rect.x, rect.width)
            } else {
                (x, self.metrics.content_width(rect))
            };
            layer.draw_rectangle(
                (band_x, content + band_top - self.scroll),
                (band_width, band_bottom - band_top),
                theme::alt(),
                Rounding::NONE,
            );
        }

        // A fenced block is tinted as one slab, not line by line: its lines
        // sit flush against each other, so per-line boxes would notch the
        // rounded corners at every join.
        let mut bi = 0;
        while bi < self.layout.blocks.len() {
            if self.layout.source[bi].is_math() {
                let first_line = self.layout.blocks[bi].lines.first();
                let last_line = self.layout.blocks[bi].lines.last();
                if let (Some(first_line), Some(last_line)) = (first_line, last_line) {
                    let top = content + first_line.y - self.scroll;
                    let bottom = content + last_line.y + last_line.height - self.scroll;
                    if bottom >= rect.y && top <= rect.bottom() {
                        layer.draw_rectangle(
                            (x - BLOCK_PAD.0, top - BLOCK_PAD.1),
                            (
                                self.metrics.content_width(rect) + BLOCK_PAD.0 * 2.0,
                                bottom - top + BLOCK_PAD.1 * 2.0,
                            ),
                            theme::math_surface(),
                            CODE_ROUNDING,
                        );
                        // The equation's number: virtual, hung flush right
                        // in the band with room to spare, so it annotates
                        // without ever crowding the expression. Only tagged
                        // blocks have one; the caret cannot reach it and it
                        // never reflows the math it labels.
                        if let Some(number) = self.layout.equation_numbers.get(&bi) {
                            theme::draw(
                                layer,
                                number,
                                (
                                    x + self.metrics.content_width(rect) - EQ_NUMBER_INSET,
                                    content + first_line.y + first_line.height * 0.5 - self.scroll,
                                ),
                                &TextStyle::mono(EQ_NUMBER_SIZE, theme::non_text()),
                                theme::RIGHT,
                            );
                        }
                    }
                }
                bi += 1;
                continue;
            }
            if !self.layout.source[bi].is_code() {
                bi += 1;
                continue;
            }
            let mut end = bi;
            while matches!(
                self.layout.source.get(end + 1),
                Some(Block::CodeLine { first: false, .. })
            ) {
                end += 1;
            }
            let first_line = self.layout.blocks[bi].lines.first();
            let last_line = self.layout.blocks[end].lines.last();
            if let (Some(first_line), Some(last_line)) = (first_line, last_line) {
                let top = content + first_line.y - self.scroll;
                let bottom = content + last_line.y + last_line.height - self.scroll;
                if bottom >= rect.y && top <= rect.bottom() {
                    layer.draw_rectangle(
                        (x - BLOCK_PAD.0, top - BLOCK_PAD.1),
                        (
                            self.metrics.content_width(rect) + BLOCK_PAD.0 * 2.0,
                            bottom - top + BLOCK_PAD.1 * 2.0,
                        ),
                        theme::code(),
                        CODE_ROUNDING,
                    );
                }
            }
            bi = end + 1;
        }

        self.draw_selection(layer, rect, x, content);
        for target in &self.context_selections {
            if let ContextHit::Range { range, kind } = target {
                self.draw_selection_range(
                    layer,
                    rect,
                    x,
                    content,
                    *range,
                    *kind == RangeKind::CodeBlock,
                );
            }
        }

        // (text, style, x, width) for one visual line — measured first so
        // the code tints can be painted underneath the text.
        let mut pieces: Vec<Painted> = Vec::new();
        for (bi, block) in self.layout.blocks.iter().enumerate() {
            let kind = &self.layout.source[bi];
            // A rule has no runs to paint, so it is drawn here rather than
            // in the text pass below: a hairline centred in its own short
            // band, spanning the text column and nothing more.
            if kind.is_divider() {
                if let Some(line) = block.lines.first() {
                    let top = content + line.y - self.scroll;
                    if top + line.height >= rect.y && top <= rect.bottom() {
                        // Whole pixel: a 1px rule on a fraction smears.
                        let y = (top + line.height * 0.5).round();
                        theme::rule(
                            layer,
                            (x, y),
                            self.metrics.content_width(rect),
                            1.0,
                            theme::border(),
                        );
                    }
                }
                continue;
            }
            // A done task is quiet twice over: dim ink (via `text_style`)
            // and a real drawn rule through the run.
            let done_task = matches!(
                kind,
                Block::ListItem {
                    marker: ListMarker::Task { done: true },
                    ..
                }
            );
            for (line_index, line) in block.lines.iter().enumerate() {
                let top = content + line.y - self.scroll;
                if top + line.height < rect.y || top > rect.bottom() {
                    continue;
                }
                let baseline = top + line.height * 0.5;
                // The marker hangs in the gutter of the item's first line
                // only; every wrapped line below keeps the indent.
                if line_index == 0
                    && let Block::ListItem { marker, .. } = kind
                {
                    draw_list_marker(layer, marker, x + line.x, baseline, self.layout.scale);
                }
                // The line's text starts at its own left edge — indented
                // for a list item's content, the gutter for everything else.
                let mut cursor = x + line.x;
                pieces.clear();
                for segment in &line.segments {
                    let run = &kind.inlines()[segment.inline];
                    let is_math = matches!(run, Inline::Math(_));
                    let is_note = matches!(run, Inline::Note(_));
                    let is_eq_ref = matches!(run, Inline::EqRef(_));
                    let text: String = match run {
                        Inline::Text(t) => t
                            .text
                            .chars()
                            .skip(segment.start)
                            .take(segment.len.min(VIEW_CAP))
                            .collect(),
                        Inline::Math(_) => ATOM.to_string(),
                        // An anchor draws its derived number, not the label
                        // the author stored — carried on the segment, so the
                        // drawing and `advance` read the same value. A
                        // reference likewise draws its `(n)`, or its raw
                        // spelling when nothing resolves.
                        Inline::Note(_) => segment.number.clone().unwrap_or_default(),
                        Inline::EqRef(_) => segment.number.clone().unwrap_or_default(),
                    };
                    let width = layout::advance(
                        run,
                        &text,
                        kind,
                        segment.style,
                        segment.number.as_deref(),
                        self.layout.scale,
                        &|text, style| theme::width(layer, text, style),
                    );
                    if is_note {
                        let style = layout::anchor_style();
                        theme::draw(
                            layer,
                            &text,
                            (cursor, baseline - layout::ANCHOR_RISE),
                            &style,
                            theme::LEFT,
                        );
                    }
                    if is_eq_ref {
                        let style = layout::eq_ref_style(&text, self.layout.scale);
                        theme::draw(layer, &text, (cursor, baseline), &style, theme::LEFT);
                    }
                    let label_x = cursor
                        + if segment.style.badge {
                            theme::BADGE_PAD * self.layout.scale
                        } else {
                            0.0
                        };
                    let painted_width = width
                        - if segment.style.badge {
                            theme::BADGE_PAD * 2.0 * self.layout.scale
                        } else {
                            0.0
                        };
                    pieces.push((text, segment.style, label_x, painted_width));
                    if is_math {
                        let Inline::Math(list) = run else {
                            unreachable!("math flag must match math run")
                        };
                        let measure =
                            |text: &str, style: &TextStyle| theme::width(layer, text, style);
                        let box_ = math_layout::layout(list, 0, self.layout.scale, &measure);
                        // An expression in prose gets no tint of its own: the
                        // notation is already distinct from the words around
                        // it, and a box behind every inline `$x$` reads as
                        // clutter down a page of them. A display block keeps
                        // its slab — that one is a block, not a phrase.
                        if let Some((selected_block, selected_inline, address)) =
                            &self.math_selection
                            && *selected_block == bi
                            && *selected_inline == segment.inline
                        {
                            draw_math_selection(
                                layer,
                                list,
                                Some(address),
                                &box_,
                                (cursor, baseline),
                                self.layout.scale,
                                &measure,
                            );
                        }
                        for target in &self.context_selections {
                            if let ContextHit::Math {
                                block,
                                inline,
                                node,
                            } = target
                                && *block == bi
                                && *inline == segment.inline
                            {
                                draw_math_selection(
                                    layer,
                                    list,
                                    node.as_ref(),
                                    &box_,
                                    (cursor, baseline),
                                    self.layout.scale,
                                    &measure,
                                );
                            }
                        }
                        // Typing into *this* expression, right now: the same
                        // condition the math cursor below is drawn on, minus
                        // Normal mode, where the slots have nothing to say.
                        let typing_here = matches!(
                            (math_focus, caret),
                            (Some(_), Some(caret))
                                if bi == caret.block && segment.inline == caret.inline
                        );
                        draw_math(
                            layer,
                            &box_,
                            (cursor, baseline),
                            typing_here && !self.block_caret,
                        );
                        if let (Some(math_cursor), Some(caret)) = (math_focus, caret)
                            && bi == caret.block
                            && segment.inline == caret.inline
                        {
                            let (cursor_x, cursor_y, cursor_height) = math_layout::cursor_pos(
                                list,
                                math_cursor,
                                0,
                                self.layout.scale,
                                &measure,
                            );
                            layer.draw_rectangle(
                                (
                                    cursor + cursor_x,
                                    baseline - cursor_y - cursor_height * 0.5 + 2.0,
                                ),
                                (2.0, cursor_height - 4.0),
                                theme::accent(),
                                Rounding::NONE,
                            );
                        }
                    }
                    cursor += width;
                }
                // A run inside a fenced block already sits on the slab drawn
                // above; only a code span in prose needs its own box.
                for (start, end) in spans(&pieces, |p| p.1.code && !kind.is_code()) {
                    layer.draw_rectangle(
                        (start - INLINE_PAD.0, top + INLINE_PAD.1),
                        (
                            end - start + INLINE_PAD.0 * 2.0,
                            line.height - INLINE_PAD.1 * 2.0,
                        ),
                        theme::code(),
                        CODE_ROUNDING,
                    );
                }
                // A chip's box is centred on the line rather than sized to
                // it: the label is 9pt, and a box grown to the leading
                // would read as a code block, not as a tag.
                for (start, end, color) in badge_spans(&pieces) {
                    theme::outline(
                        layer,
                        Rect {
                            x: start - theme::BADGE_PAD * self.layout.scale,
                            y: baseline - theme::BADGE_HEIGHT * 0.5,
                            width: end - start + theme::BADGE_PAD * 2.0 * self.layout.scale,
                            height: theme::BADGE_HEIGHT,
                        },
                        theme::badge_ink(color),
                    );
                }
                // The bar goes on this layer crisp; the glow layer takes a
                // fatter, fainter copy and its blur does the falloff.
                for (start, end) in spans(&pieces, |p| p.1.highlight) {
                    let at = (start - HIGHLIGHT_PAD, baseline + HIGHLIGHT_DROP);
                    let size = (end - start + HIGHLIGHT_PAD * 2.0, HIGHLIGHT_THICKNESS);
                    layer.draw_rectangle(at, size, theme::highlight(), HIGHLIGHT_ROUNDING);
                    if let Some(glow) = &self.glow {
                        glow.draw_rectangle(
                            (at.0 - GLOW_SPREAD, at.1 - GLOW_SPREAD),
                            (size.0 + GLOW_SPREAD * 2.0, size.1 + GLOW_SPREAD * 2.0),
                            theme::fade(theme::highlight(), GLOW_ALPHA),
                            HIGHLIGHT_ROUNDING,
                        );
                    }
                }
                if done_task && let (Some(first), Some(last)) = (pieces.first(), pieces.last()) {
                    theme::rule(
                        layer,
                        (first.2 - 1.0, baseline - STRIKE_RISE),
                        last.2 + last.3 - first.2 + 2.0,
                        STRIKE_THICKNESS,
                        theme::dim(),
                    );
                }
                for ((text, style, at, _), segment) in pieces.iter().zip(&line.segments) {
                    if matches!(
                        kind.inlines()[segment.inline],
                        Inline::Math(_) | Inline::Note(_) | Inline::EqRef(_)
                    ) {
                        continue;
                    }
                    let style = layout::text_style(kind, *style, self.layout.scale);
                    theme::draw(layer, text, (*at, baseline), &style, theme::LEFT);
                }
                // The auto-number, hung in the margin: virtual, so it is
                // drawn rather than laid out — the caret cannot reach it and
                // it never shifts the heading it labels.
                if line_index == 0
                    && kind.is_heading()
                    && let Some(number) = &self.numbers[bi]
                {
                    theme::draw(
                        layer,
                        number,
                        (x - NUMBER_GUTTER, baseline),
                        &TextStyle::mono(NUMBER_SIZE, theme::non_text()),
                        theme::RIGHT,
                    );
                }
            }
        }

        // The caret's glyph context, for Normal mode's block.
        let caret_char = caret.and_then(|caret| {
            self.layout
                .source
                .get(caret.block)
                .and_then(|b| b.inlines().get(caret.inline))
                .and_then(|run| match run {
                    Inline::Text(t) => t.text.chars().nth(caret.offset),
                    Inline::Math(_) => Some(ATOM),
                    Inline::Note(_) => Some(ATOM),
                    Inline::EqRef(_) => Some(ATOM),
                })
        });
        let screen_x = x + caret_x;
        let screen_y = content + caret_baseline - self.scroll;

        if let Some(point) = self.brush_point {
            draw_brush(layer, point);
        }

        if self.math.is_some() {
            return;
        }
        // No caret here — the page while focus lives in a note, or an
        // unfocused margin note. The text above already drew.
        if caret.is_none() {
            return;
        }
        if self.block_caret {
            // Faded rather than opaque, so whatever glyph is under it stays
            // legible. Falls back to a space's advance past the end of the
            // line, like the old line editor did.
            let sample = caret_char.map(String::from).unwrap_or_else(|| " ".into());
            let width =
                theme::width(layer, &sample, &TextStyle::serif(17.5, theme::ink())).max(1.0);
            layer.draw_rectangle(
                (screen_x, screen_y - caret_height * 0.5 + 2.0),
                (width, caret_height - 4.0),
                theme::fade(theme::accent(), 0.35),
                Rounding::NONE,
            );
        } else if self.caret_on {
            layer.draw_rectangle(
                (screen_x, screen_y - caret_height * 0.5 + 2.0),
                (2.0, caret_height - 4.0),
                theme::accent(),
                Rounding::NONE,
            );
            // The style context: a tiny mono marker on the bar's top-right,
            // one letter per active flag. The boxed styles exclude the rest
            // at the model layer, so `C` and `#` always stand alone.
            let mut marker = String::new();
            for (on, letter) in [
                (self.caret_style.code, 'C'),
                (self.caret_style.badge, '#'),
                (self.caret_style.bold, 'B'),
                (self.caret_style.italic, 'I'),
                (self.caret_style.highlight, 'H'),
            ] {
                if on {
                    marker.push(letter);
                }
            }
            if !marker.is_empty() {
                theme::draw(
                    layer,
                    &marker,
                    (screen_x + 3.0, screen_y - caret_height * 0.5),
                    &TextStyle::mono(9.0, theme::accent()),
                    theme::TOP_LEFT,
                );
            }
        }
    }
}

impl Editor {
    fn draw_selection(&self, layer: &Layer, rect: Rect, x: f32, content: f32) {
        let Some(selection) = self.selection else {
            return;
        };
        self.draw_selection_range(layer, rect, x, content, selection, self.line_selection);
    }

    fn draw_selection_range(
        &self,
        layer: &Layer,
        rect: Rect,
        x: f32,
        content: f32,
        selection: FlatRange,
        line_selection: bool,
    ) {
        let range = selection.normalized();
        for (bi, block) in self.layout.blocks.iter().enumerate() {
            let mut line_start = 0;
            for line in &block.lines {
                let top = content + line.y - self.scroll;
                let line_end = line_start + line.segments.iter().map(|s| s.len).sum::<usize>();
                if top + line.height >= rect.y && top <= rect.bottom() {
                    if line_selection {
                        if (range.start.block..=range.end.block).contains(&bi) {
                            layer.draw_rectangle(
                                (rect.x, top),
                                (rect.width, line.height),
                                theme::selection(),
                                Rounding::NONE,
                            );
                        }
                    } else {
                        let start = if range.start.block == bi {
                            range.start.offset
                        } else if range.start.block < bi {
                            0
                        } else {
                            line_end
                        };
                        let end = if range.end.block == bi {
                            range.end.offset
                        } else if range.end.block > bi {
                            line_end
                        } else {
                            0
                        };
                        let start = start.max(line_start).min(line_end);
                        let end = end.max(start).min(line_end);
                        if start < end {
                            let left = Self::line_x(
                                &self.layout.source[bi],
                                line,
                                line_start,
                                start,
                                self.layout.scale,
                                layer,
                            );
                            let right = Self::line_x(
                                &self.layout.source[bi],
                                line,
                                line_start,
                                end,
                                self.layout.scale,
                                layer,
                            );
                            layer.draw_rectangle(
                                (x + line.x + left, top),
                                ((right - left).max(1.0), line.height),
                                theme::selection(),
                                Rounding::NONE,
                            );
                        }
                    }
                }
                line_start = line_end;
            }
        }
    }

    fn line_x(
        block: &Block,
        line: &layout::VisLine,
        line_start: usize,
        flat: usize,
        scale: f32,
        layer: &Layer,
    ) -> f32 {
        let mut x = 0.0;
        let mut cursor = line_start;
        for segment in &line.segments {
            let run = &block.inlines()[segment.inline];
            let text: String = match run {
                Inline::Text(t) => t
                    .text
                    .chars()
                    .skip(segment.start)
                    .take(segment.len)
                    .collect(),
                Inline::Math(_) => ATOM.to_string(),
                Inline::Note(_) => ATOM.to_string(),
                Inline::EqRef(_) => ATOM.to_string(),
            };
            if flat >= cursor + segment.len {
                x += layout::advance(
                    run,
                    &text,
                    block,
                    segment.style,
                    segment.number.as_deref(),
                    scale,
                    &|text, style| theme::width(layer, text, style),
                );
            } else {
                let count = flat.saturating_sub(cursor);
                if segment.style.badge {
                    x += theme::BADGE_PAD * scale;
                }
                let prefix: String = text.chars().take(count).collect();
                x += theme::width(
                    layer,
                    &prefix,
                    &layout::text_style(block, segment.style, scale),
                );
                break;
            }
            cursor += segment.len;
        }
        x
    }
}

/// The scroll offset that brings the y-band `[band_top, band_bottom)` inside
/// `[view_top, view_bottom)` without moving further than it must — a band
/// already in view leaves the scroll untouched. Pure, so the "stuck at the
/// bottom" runaway is a test rather than a runtime surprise. Bands are
/// relative to content top; `view_top` is the content's top on screen, so
/// `current` cancels out exactly what the old line-based version did.
pub fn follow_scroll(
    current: f32,
    band_top: f32,
    band_bottom: f32,
    view_top: f32,
    view_bottom: f32,
    max: f32,
) -> f32 {
    let top = view_top + band_top - current;
    let mut out = current;
    if top < view_top {
        out -= view_top - top;
    } else if top + (band_bottom - band_top) > view_bottom {
        out += top + (band_bottom - band_top) - view_bottom;
    }
    out.clamp(0.0, max)
}

/// The editor's scroll ceiling: enough to bring the last line to the top
/// of the view, plus [`OVERSCROLL`] of the view's own height as empty
/// space past it. `view_height` is the visible content height (the
/// region's height minus [`TOP`]).
pub fn max_scroll(content_height: f32, view_height: f32) -> f32 {
    (content_height - view_height * (1.0 - OVERSCROLL)).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::math::MathNode;
    use crate::document::{Document, Text};
    use std::path::Path;

    #[test]
    fn adjacent_decorated_pieces_merge_into_one_span() {
        let mark = Style {
            highlight: true,
            ..Style::PLAIN
        };
        let bold_mark = Style { bold: true, ..mark };
        let piece = |style, x: f32, w: f32| (String::new(), style, x, w);
        let pieces = vec![
            piece(Style::PLAIN, 0.0, 10.0),
            // Two marked runs touching — a bold word inside the mark. One
            // bar, not two, or the seam shows as a notch.
            piece(mark, 10.0, 20.0),
            piece(bold_mark, 30.0, 15.0),
            piece(Style::PLAIN, 45.0, 5.0),
            piece(mark, 50.0, 8.0),
        ];
        assert_eq!(
            spans(&pieces, |p| p.1.highlight),
            vec![(10.0, 45.0), (50.0, 58.0)]
        );
        assert!(spans(&pieces, |p| p.1.code).is_empty());
    }

    #[test]
    fn badge_outlines_split_when_their_colors_differ() {
        let orange = Style {
            badge: true,
            ..Style::PLAIN
        };
        let blue = Style {
            badge: true,
            badge_color: BadgeColor::Blue,
            ..Style::PLAIN
        };
        let pieces = vec![
            ("A".into(), orange, 4.0, 10.0),
            ("B".into(), orange, 22.0, 10.0),
            ("C".into(), blue, 40.0, 10.0),
        ];
        assert_eq!(
            badge_spans(&pieces),
            vec![
                (4.0, 32.0, BadgeColor::Orange),
                (40.0, 50.0, BadgeColor::Blue),
            ]
        );
    }

    #[test]
    fn follow_scroll_brings_a_band_in_without_runaway() {
        let (top, bottom, max) = (0.0f32, 700.0, 10_000.0f32);
        // Band near the top: no movement needed.
        assert_eq!(follow_scroll(0.0, 0.0, 30.0, top, bottom, max), 0.0);
        // A band already visible is untouched — the click that used to
        // double the offset on every rebuild.
        assert_eq!(follow_scroll(0.0, 100.0, 130.0, top, bottom, max), 0.0);
        // Scrolled far down with the caret band at the top: snapped back up.
        assert_eq!(follow_scroll(3000.0, 0.0, 30.0, top, bottom, max), 0.0);
        // Band below the fold: scrolled down just enough to reveal it.
        assert_eq!(follow_scroll(0.0, 700.0, 730.0, top, bottom, max), 30.0);
        assert_eq!(follow_scroll(0.0, 800.0, 830.0, top, bottom, max), 130.0);
        // Clamped to the content ceiling.
        assert_eq!(
            follow_scroll(0.0, 80_000.0, 80_030.0, top, bottom, 1000.0),
            1000.0
        );
    }

    #[test]
    fn max_scroll_leaves_room_past_the_last_line() {
        // Document taller than half the view: positive ceiling.
        assert_eq!(max_scroll(2000.0, 800.0), 1600.0);
        // Empty document: ceiling is zero.
        assert_eq!(max_scroll(0.0, 800.0), 0.0);
        // Short document: ceiling is zero.
        assert_eq!(max_scroll(100.0, 800.0), 0.0);
        // Document exactly at the threshold: ceiling is zero.
        assert_eq!(max_scroll(400.0, 800.0), 0.0);
    }

    #[test]
    fn heading_numbers_are_indexed_by_block() {
        let text = |value: &str| {
            Inline::Text(Text {
                text: value.into(),
                style: Style::PLAIN,
            })
        };
        let blocks = vec![
            Block::Paragraph(vec![text("intro")]),
            Block::Heading {
                level: 1,
                content: vec![text("First")],
            },
            Block::Heading {
                level: 2,
                content: vec![text("Nested")],
            },
        ];
        let mut document = Document::new(Path::new("notes/test.md"));
        *document.body_mut() = blocks;
        let layout = layout::layout(&document, 1000.0, &|value, _| {
            value.chars().count() as f32 * 10.0
        });
        let editor = Editor::new(
            Rc::new(layout),
            Some(Caret {
                block: 0,
                inline: 0,
                offset: 0,
                style: Style::PLAIN,
            }),
            0.0,
            false,
            Style::PLAIN,
            Metrics::PAGE,
        );

        assert_eq!(
            editor.numbers,
            vec![None, Some("1".into()), Some("1.1".into())]
        );
    }

    #[test]
    fn editor_keeps_brush_targets_and_pointer_together() {
        let mut document = Document::new(Path::new("notes/test.md"));
        *document.body_mut() = vec![Block::Paragraph(vec![Inline::Text(Text {
            text: "x".into(),
            style: Style::PLAIN,
        })])];
        let target = ContextHit::Range {
            range: FlatRange::new(
                crate::document::FlatPos {
                    block: 0,
                    offset: 0,
                },
                crate::document::FlatPos {
                    block: 0,
                    offset: 1,
                },
            ),
            kind: RangeKind::Word,
        };
        let layout = layout::layout(&document, 1000.0, &|value, _| {
            value.chars().count() as f32 * 10.0
        });
        let editor = Editor::new(
            Rc::new(layout),
            Some(document.caret),
            0.0,
            false,
            Style::PLAIN,
            Metrics::PAGE,
        )
        .with_context_selections(vec![target.clone()], Some((42.0, 24.0)));

        assert_eq!(editor.context_selections, vec![target]);
        assert_eq!(editor.brush_point, Some((42.0, 24.0)));
    }

    #[test]
    fn an_atom_measures_as_its_box_not_as_a_glyph() {
        let run = Inline::Math(vec![MathNode::Frac {
            num: vec![MathNode::Sym('1')],
            den: vec![MathNode::Sym('2')],
        }]);
        let block = Block::Paragraph(vec![run.clone()]);
        let atom = ATOM.to_string();
        let measure =
            |text: &str, style: &TextStyle| text.chars().count() as f32 * style.size * 0.5;
        let width = layout::advance(&run, &atom, &block, Style::PLAIN, None, 1.0, &measure);
        let box_width = match &run {
            Inline::Math(list) => math_layout::layout(list, 0, 1.0, &measure).width,
            Inline::Text(_) => unreachable!(),
            Inline::Note(_) | Inline::EqRef(_) => unreachable!(),
        };
        assert_eq!(width, box_width);
        assert!(width > measure(&atom, &TextStyle::serif(17.5, theme::ink())));
    }

    /// `math_rect` is what a whole-expression selection is drawn to now that
    /// inline math has no background of its own — it must still cover the
    /// box exactly, ascent above the baseline and descent below.
    #[test]
    fn a_math_rect_covers_the_whole_box_around_its_baseline() {
        let box_ = MathBox {
            width: 42.0,
            ascent: 19.0,
            descent: 11.0,
            highlight: None,
            kind: BoxKind::Row { children: vec![] },
        };
        assert_eq!(
            math_rect(&box_, (7.0, 31.0)),
            Rect {
                x: 7.0,
                y: 12.0,
                width: 42.0,
                height: 30.0,
            }
        );
    }
}
