//! The active file, drawn from a [`DocLayout`]: styled, wrapped, no markup
//! visible. The shell lays out the open document and hands it over as an
//! [`Rc`]; this component only reads it.

use std::collections::HashMap;
use std::rc::Rc;

use crate::document::decoration::{self, Painted};
use crate::document::layout::{
    self, CHEVRON_WIDTH, ContextHit, DocLayout, FOLD_INDICATOR_HEIGHT, NUMBER_GUTTER, NUMBER_SIZE,
    RangeKind,
};
use crate::document::math::{MathCursor, NodeAddress};
use crate::document::math_layout::{self, MathBox};
use crate::document::math_paint;
use crate::document::{ATOM, Block, Caret, FlatRange, Inline, Style};
use crate::layout::Rect;
use crate::renderer::{Layer, PathPaint, Rounding, ShaderEffect};
use crate::theme::{self, TextStyle};
use crate::ui::{Component, Context, Dirty};

/// Top padding of the content area (the filename title is gone — the
/// document's own H1 is the title).
pub const TOP: f32 = 48.0;
pub const INSET: f32 = 56.0;
/// The equation number's size — the same margin-annotation register as the
/// heading auto-number, never part of the math's own typography.
pub const EQ_NUMBER_SIZE: f32 = 11.0;
/// Room kept between the equation number and the band's right edge, so the
/// widest reading of the band never crowds it.
pub const EQ_NUMBER_INSET: f32 = 18.0;
/// The design's measure: the content column is never wider than this.
pub const MEASURE: f32 = 704.0;
/// Space kept past the right edge before a line may wrap.
pub const RIGHT_MARGIN: f32 = 24.0;
/// Radius of the Ctrl-drag selection brush in logical pixels.
pub const BRUSH_RADIUS: f32 = 9.0;
/// Padding of the tint behind a fenced code block, which is the generous
/// one; an inline span's tighter pad belongs to [`decoration`], with the
/// box that uses it.
pub const BLOCK_PAD: (f32, f32) = (10.0, 6.0);
pub const CODE_ROUNDING: Rounding = Rounding::uniform(10.0);
const MATH_SELECTION_ROUNDING: Rounding = Rounding::uniform(4.0);
/// The halo drawn on the glow layer: a fatter, fainter copy of the
/// highlight bar that the layer's blur turns into the falloff. One shader
/// pass over one layer, rather than a stack of hand-faded rectangles. Kept
/// close to the bar — the glow is meant to read as the mark's own light,
/// not as a second object under it.
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
/// The fold chevron's path: a filled triangle pointing down (the section
/// is open), centred on the origin so [`Layer::draw_path_rotated`] turns it
/// about its own centre — a quarter turn and it points right (folded).
const CHEVRON: &str = "M0 3 L-3.5 -3 L3.5 -3 Z";

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

/// The wash behind a selected expression, or behind one addressed node of
/// it. Editor-only: a page has no selection, which is why this stayed here
/// while the notation itself moved to [`math_paint`].
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
        let rect = math_paint::rect(box_, origin);
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
    focus_amount: f32,
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
    empty: super::empty_state::EmptyState,
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
            focus_amount: 0.0,
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
            empty: super::empty_state::EmptyState::default(),
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
                code_colors: Vec::new(),
                anchors: Vec::new(),
                equation_numbers: HashMap::new(),
            }),
            metrics: Metrics::PAGE,
            caret: None,
            scroll: 0.0,
            focus_amount: 0.0,
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
            empty: super::empty_state::EmptyState::default(),
            dirty: Dirty::new(),
        }
    }

    /// The page origin, shared by painting, mouse input and popup anchors.
    pub fn content_x(rect: Rect) -> f32 {
        rect.x + INSET + ((rect.width - INSET - RIGHT_MARGIN - MEASURE).max(0.0) / 2.0)
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
        if self.metrics.page {
            self.dirty
                .write(&mut self.focus_amount, context.focus_amount);
        }
        if !self.has_file && self.empty.sync(context) {
            self.dirty.set();
        }
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

    fn is_animating(&self) -> bool {
        !self.has_file && self.empty.is_animating()
    }

    fn draw(&mut self, layer: &Layer, rect: Rect) {
        // The glow layer is ours to manage: nothing else clears it, and a
        // stale halo would outlive the bar it belongs to. Clearing up here
        // covers the placeholder's early return too.
        if let Some(glow) = &self.glow {
            glow.clear();
            glow.set_clip_rect(Some((rect.position(), rect.size())));
        }
        // DocumentSurface owns the sheet beneath this region.
        let x = if self.metrics.page {
            Self::content_x(rect)
        } else {
            rect.x + self.metrics.inset
        };
        if !self.has_file {
            layer.set_effect(None);
            self.empty.draw(layer, rect);
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

        layer.set_effect((self.focus_amount > 0.0 && caret.is_some()).then(|| {
            ShaderEffect::FocusBand {
                top: content + band_top - self.scroll,
                bottom: content + band_bottom - self.scroll,
                feather: 4.0,
                opacity: theme::focus_opacity(self.focus_amount),
            }
        }));

        // The current-line band does not blink — it identifies the line the
        // caret is on, regardless of caret visibility. The page's spans the
        // whole column; a note's spans only its own content column, so the
        // marker and rule beside it stay visible.
        if caret.is_some() {
            let (band_x, band_width) = if self.metrics.page {
                (x - 12.0, self.metrics.content_width(rect) + 24.0)
            } else {
                (x, self.metrics.content_width(rect))
            };
            layer.draw_rectangle(
                (band_x, content + band_top - self.scroll),
                (band_width, band_bottom - band_top),
                theme::fade(theme::alt(), 0.65),
                Rounding::uniform(6.0),
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
            // Folded ground draws nothing at all — not the runs, not the
            // slabs, not a hint of what is hidden. The folded heading's own
            // indicator is the only trace.
            if block.hidden.is_some() {
                continue;
            }
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
                    let mut canvas = layer;
                    decoration::marker(
                        &mut canvas,
                        marker,
                        x + line.x,
                        baseline,
                        self.layout.scale,
                    );
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
                    let width =
                        segment.advance(run, &text, kind, self.layout.scale, &|text, style| {
                            theme::width(layer, text, style)
                        });
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
                    pieces.push(decoration::piece(
                        text,
                        segment.style,
                        cursor,
                        width,
                        self.layout.scale,
                        segment.padding,
                    ));
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
                        // The notation itself is drawn by `math_paint`, the
                        // one copy an export runs too — so a fraction's bar
                        // cannot land differently on a page than on screen.
                        // A layer is a handle, so a `Canvas` over it is a
                        // rebinding, not a conversion.
                        let mut canvas = layer;
                        math_paint::draw(
                            &mut canvas,
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
                // Every mark these runs carry — the box behind a code
                // span, a chip, a highlight's bar, a done task's strike —
                // drawn by the painter the page draws them with.
                let mut canvas = layer;
                decoration::runs(
                    &mut canvas,
                    &pieces,
                    kind,
                    top,
                    line.height,
                    baseline,
                    self.layout.scale,
                );
                // The crisp bar is `decoration`'s; the glow layer takes a
                // fatter, fainter copy of the same rect and its blur does
                // the falloff. A page has no such layer, which is why this
                // half stays here.
                if let Some(glow) = &self.glow {
                    for (at, size) in decoration::highlight_bars(&pieces, baseline) {
                        glow.draw_rectangle(
                            (at.0 - GLOW_SPREAD, at.1 - GLOW_SPREAD),
                            (size.0 + GLOW_SPREAD * 2.0, size.1 + GLOW_SPREAD * 2.0),
                            theme::fade(theme::highlight(), GLOW_ALPHA * (1.0 - self.focus_amount)),
                            decoration::HIGHLIGHT_ROUNDING,
                        );
                    }
                }
                for ((text, _, at, _), segment) in pieces.iter().zip(&line.segments) {
                    if matches!(
                        kind.inlines()[segment.inline],
                        Inline::Math(_) | Inline::Note(_) | Inline::EqRef(_)
                    ) {
                        continue;
                    }
                    let spans = crate::document::code::painted(&self.layout, bi, segment, text);
                    let spans: Vec<_> = spans
                        .iter()
                        .map(|(text, style)| {
                            crate::renderer::TextSpan::new(
                                *text,
                                style.font.clone(),
                                style.parameters(),
                                style.color.clone(),
                            )
                        })
                        .collect();
                    layer.draw_styled_text(&spans, (*at, baseline), theme::LEFT);
                }
                // The auto-number, hung in the margin: virtual, so it is
                // drawn rather than laid out — the caret cannot reach it and
                // it never shifts the heading it labels.
                if line_index == 0 && kind.is_heading() {
                    if let Some(number) = &self.numbers[bi] {
                        theme::draw(
                            layer,
                            number,
                            (x - NUMBER_GUTTER, baseline),
                            &TextStyle::mono(NUMBER_SIZE, theme::non_text()),
                            theme::RIGHT,
                        );
                    }
                    // The fold chevron, just left of the number: down while
                    // the section is open, turned to point right when it is
                    // folded. The turn itself is instant — a component
                    // recreated on every rebuild cannot hold a clock, and
                    // §11 forbids the *reflow* from animating anyway; the
                    // state change is the statement.
                    let folded = kind.is_folded();
                    let scale = self.layout.scale;
                    let number_width = self.numbers[bi].as_deref().map_or(0.0, |number| {
                        theme::width(layer, number, &TextStyle::mono(NUMBER_SIZE, theme::ink()))
                    });
                    let right = layout::chevron_right(number_width, scale);
                    let centre_x = x + right - CHEVRON_WIDTH * scale / 2.0;
                    let colour = if folded {
                        theme::dim()
                    } else {
                        theme::non_text()
                    };
                    layer
                        .draw_path_rotated(
                            CHEVRON,
                            (centre_x, baseline),
                            if folded {
                                -std::f32::consts::FRAC_PI_2
                            } else {
                                0.0
                            },
                            PathPaint::fill(colour),
                        )
                        .expect("chevron path parses");
                }
            }

            // The collapsed-body indicator: one quiet italic line standing
            // in for everything folded away, riding a hairline rule that
            // runs out to the column's right edge. The three midline dots
            // are drawn, not typed — Georgia has no U+22EF, and a missing
            // glyph would break the quiet this line exists to keep. The
            // whole band is a click target (see `DocLayout::
            // fold_indicator_at`): touching it unfolds the fold.
            if let Some(indicator) = &block.indicator {
                let scale = self.layout.scale;
                let height = FOLD_INDICATOR_HEIGHT * scale;
                let mid = content + indicator.y + height * 0.5 - self.scroll;
                let label = if indicator.lines == 1 {
                    "1 line hidden".to_string()
                } else {
                    format!("{} lines hidden", indicator.lines)
                };
                let style = TextStyle::sans(12.5 * scale, theme::comment()).italic();
                for dot in 0..3 {
                    layer.draw_circle(
                        (x + 4.0 * scale + dot as f32 * 4.5 * scale, mid),
                        1.2 * scale,
                        theme::faint(),
                    );
                }
                theme::draw(
                    layer,
                    &label,
                    (x + 22.0 * scale, mid + 4.5 * scale),
                    &style,
                    theme::LEFT,
                );
                let text_width = theme::width(layer, &label, &style);
                let rule_x = x + 22.0 * scale + text_width + 12.0 * scale;
                let rule_end = x + self.metrics.content_width(rect);
                if rule_end > rule_x {
                    theme::rule(
                        layer,
                        (rule_x, mid),
                        rule_end - rule_x,
                        1.0,
                        theme::border(),
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
            let width = theme::width(layer, &sample, &TextStyle::sans(17.5, theme::ink())).max(1.0);
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
                x += segment.advance(run, &text, block, scale, &|text, style| {
                    theme::width(layer, text, style)
                });
            } else {
                let count = flat.saturating_sub(cursor);
                x += segment.text_inset(scale);
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

/// Center the active visual line in the window. Negative offsets provide
/// breathing room above the first line; the same transform drives hits.
pub fn focus_scroll(band: (f32, f32), editor: Rect, window: Rect) -> f32 {
    editor.y + TOP + (band.0 + band.1) * 0.5 - (window.y + window.height * 0.5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_origin_centers_the_capped_measure_without_changing_narrow_insets() {
        let narrow = Rect::new(200.0, 100.0, 420.0, 600.0);
        assert_eq!(Editor::content_x(narrow), narrow.x + INSET);
        let wide = Rect::new(200.0, 100.0, 1200.0, 600.0);
        let left = Editor::content_x(wide) - wide.x - INSET;
        let right =
            wide.right() - RIGHT_MARGIN - Editor::content_x(wide) - Editor::content_width(wide);
        assert_eq!(left, right);
        assert_eq!(Editor::content_width(wide), MEASURE);
    }
    use crate::document::math::MathNode;
    use crate::document::math_layout::BoxKind;
    use crate::document::{Document, Text};
    use std::path::Path;

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
    fn focused_wrapped_lines_stay_centered_and_hit_the_same_caret_after_resize() {
        let measure = |s: &str, _: &TextStyle| s.chars().count() as f32 * 10.0;
        let doc = crate::document::markdown::parse(
            Path::new("focus.md"),
            "# First\n\nA paragraph long enough to wrap onto several visual lines and remain editable.\n\nLast line.",
        );
        for (width, height) in [(620.0, 520.0), (1600.0, 1000.0)] {
            let window = Rect::new(0.0, 0.0, width, height);
            let editor = Rect::new(12.0, 152.0, width - 24.0, height - 164.0);
            let laid = layout::layout(&doc, Editor::content_width(editor), &measure);
            for (block, source) in laid.source.iter().enumerate() {
                for offset in 0..source.inlines().first().map_or(0, |run| match run {
                    Inline::Text(t) => t.text.chars().count(),
                    _ => 0,
                }) {
                    let caret = Caret {
                        block,
                        inline: 0,
                        offset,
                        style: Style::PLAIN,
                    };
                    let band = laid.caret_band(caret);
                    let scroll = focus_scroll(band, editor, window);
                    let (cx, cy, _) = laid.caret_pos(caret, &measure);
                    let screen_y = editor.y + TOP + cy - scroll;
                    assert!((screen_y - height * 0.5).abs() < 0.01);
                    let hit = laid.hit(cx, screen_y - editor.y - TOP + scroll, &measure);
                    assert_eq!((hit.block, hit.inline, hit.offset), (block, 0, offset));
                }
            }
            let first = laid.caret_band(Caret {
                block: 0,
                inline: 0,
                offset: 0,
                style: Style::PLAIN,
            });
            assert!(
                focus_scroll(first, editor, window) < 0.0,
                "first line needs top breathing room"
            );
        }
    }

    #[test]
    fn focus_centers_tall_equations_and_releases_negative_scroll_on_exit() {
        let window = Rect::new(0.0, 0.0, 1200.0, 900.0);
        let editor = Rect::new(12.0, 152.0, 1176.0, 736.0);
        for band in [(0.0, 30.0), (2500.0, 2680.0)] {
            let scroll = focus_scroll(band, editor, window);
            let top = editor.y + TOP + band.0 - scroll;
            let bottom = editor.y + TOP + band.1 - scroll;
            assert_eq!((top + bottom) / 2.0, 450.0);
        }
        let scroll = focus_scroll((0.0, 30.0), editor, window);
        assert_eq!(
            follow_scroll(scroll, 0.0, 30.0, editor.y + TOP, editor.bottom(), 1000.0),
            0.0
        );
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
                folded: false,
                content: vec![text("First")],
            },
            Block::Heading {
                level: 2,
                folded: false,
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
        assert!(width > measure(&atom, &TextStyle::sans(17.5, theme::ink())));
    }

    /// The fold chevron's path is parsed by the renderer at draw time; the
    /// exact string the editor draws is pinned here.
    #[test]
    fn the_fold_chevron_path_parses() {
        let mut parser = lyon_extra::parser::PathParser::new();
        let mut builder = lyon::path::Path::builder();
        let mut source = lyon_extra::parser::Source::new(CHEVRON.chars());
        let ok = parser
            .parse(
                &lyon_extra::parser::ParserOptions::DEFAULT,
                &mut source,
                &mut builder,
            )
            .is_ok();
        assert!(
            ok,
            "the chevron path must parse or every heading draw panics"
        );
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
            math_paint::rect(&box_, (7.0, 31.0)),
            Rect {
                x: 7.0,
                y: 12.0,
                width: 42.0,
                height: 30.0,
            }
        );
    }
}
