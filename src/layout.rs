//! Retained layout tree — rectangles for the editor shell.
//!
//! The shell is a handful of nested rows and columns: panels with a fixed
//! extent, panels that take what is left, and panels that collapse away
//! entirely. That is the whole problem, so that is the whole model —
//! [`Size::Fixed`], [`Size::Flex`], [`Style::visible`], and a minimum.
//!
//! The layout does no measuring of its own. A node's [`Style::min_size`] is
//! pushed in by whoever owns the content (see `crate::ui::Component`); the
//! solver only ever adds numbers up. Everything below follows from that:
//!
//! **Minimums are honoured before flexibility.** A flexible child that
//! would be squeezed under its minimum is frozen at that minimum and the
//! remaining space is redistributed among the rest — the same freeze pass
//! flexbox uses. Since nothing in the renderer clips, a panel squeezed
//! below its content's minimum would draw over its neighbour, so this is a
//! correctness rule, not a niceness.
//!
//! **Minimums propagate up.** [`Layout::min_size`] on any node reports what
//! that whole subtree needs, so the window can refuse to be resized below
//! the shell's floor and a drag can stop at the exact pixel where the next
//! panel would start losing space.
//!
//! **It only solves when something changed.** [`Layout::compute`] is called
//! every frame and returns immediately unless a style actually changed or
//! the viewport moved. Styles are compared by value, so an animation
//! writing the same width twice does not cause a second solve, and a static
//! shell costs one `bool` test per frame.
//!
//! **Adjacent rects share an edge exactly.** Child positions are snapped to
//! whole logical pixels and each child's far edge is the next one's near
//! edge, so no seam or overlap appears between panels at any width.
//!
//! ```ignore
//! let mut layout = Layout::new(Style::default());          // root, a column
//! let body = layout.add_child(Layout::ROOT, Style::flex(1.0).row());
//! let tree = layout.add_child(body, Style::fixed(200.0));
//! let text = layout.add_child(body, Style::flex(1.0).min(320.0, 0.0));
//!
//! layout.compute(viewport);                                 // solves
//! layout.compute(viewport);                                 // no-op
//! layout.set_style(tree, |s| s.visible = false);            // dirty
//! layout.compute(viewport);                                 // solves; `text` fills
//! ```

/// A laid-out rectangle in logical pixels — the same space every
/// `Layer::draw_*` method and `Input::mouse_position` use, so a rect is
/// directly drawable and directly hit-testable.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn position(&self) -> (f32, f32) {
        (self.x, self.y)
    }

    pub fn size(&self) -> (f32, f32) {
        (self.width, self.height)
    }

    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    pub fn is_empty(&self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }

    pub fn contains(&self, (px, py): (f32, f32)) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }

    /// Shrinks by `by` on every side, clamped at zero extent.
    pub fn inset(&self, by: f32) -> Self {
        Self {
            x: self.x + by,
            y: self.y + by,
            width: (self.width - by * 2.0).max(0.0),
            height: (self.height - by * 2.0).max(0.0),
        }
    }
}

/// How a node is sized along its parent's axis. The cross axis always
/// fills the parent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Size {
    /// An exact extent in logical pixels — exact, *not* floored by
    /// [`Style::min_size`]. A caller that sets one is stating the extent
    /// outright, and the commonest reason to state one below the content's
    /// minimum is a collapse animation: flooring it there strands the panel
    /// at its minimum width for most of the transition and then snaps it to
    /// nothing when it finally hides. A drag that must respect the minimum
    /// clamps against [`Layout::min_size`] itself, at the point where the
    /// user's intent is known.
    Fixed(f32),
    /// A share of whatever the fixed children left behind, by weight.
    Flex(f32),
}

/// The axis a node stacks its *children* along.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Row,
    Column,
}

/// Everything the solver knows about one node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub size: Size,
    pub axis: Axis,
    /// Space between children, in logical pixels. Only counted between
    /// *visible* children, so collapsing a panel takes its divider with it.
    pub gap: f32,
    /// A hidden node is skipped by the solver entirely: it takes no space,
    /// contributes no gap, and keeps whatever rect it last had.
    pub visible: bool,
    /// The smallest `(width, height)` this node's own content can live in.
    /// Set from the content's own measurement — the solver never derives
    /// it. A container's effective minimum is this or what its children
    /// need, whichever is larger.
    pub min_size: (f32, f32),
}

impl Default for Style {
    fn default() -> Self {
        Self {
            size: Size::Flex(1.0),
            axis: Axis::Column,
            gap: 0.0,
            visible: true,
            min_size: (0.0, 0.0),
        }
    }
}

impl Style {
    pub fn fixed(extent: f32) -> Self {
        Self {
            size: Size::Fixed(extent),
            ..Self::default()
        }
    }

    pub fn flex(weight: f32) -> Self {
        Self {
            size: Size::Flex(weight),
            ..Self::default()
        }
    }

    pub fn row(mut self) -> Self {
        self.axis = Axis::Row;
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    pub fn min(mut self, width: f32, height: f32) -> Self {
        self.min_size = (width, height);
        self
    }
}

/// Handle to a node. Stable for the life of the [`Layout`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(usize);

#[derive(Debug)]
struct Node {
    style: Style,
    children: Vec<NodeId>,
    rect: Rect,
    /// What this subtree needs, filled in by the measure pass.
    min: (f32, f32),
}

/// The tree. Built once at startup, mutated by the app, solved on demand.
#[derive(Debug)]
pub struct Layout {
    nodes: Vec<Node>,
    dirty: bool,
    viewport: Rect,
}

impl Layout {
    /// The root node, created by [`Layout::new`].
    pub const ROOT: NodeId = NodeId(0);

    pub fn new(root: Style) -> Self {
        Self {
            nodes: vec![Node {
                style: root,
                children: Vec::new(),
                rect: Rect::default(),
                min: (0.0, 0.0),
            }],
            // Nothing has been solved yet, so the first `compute` must run
            // whatever viewport it is handed.
            dirty: true,
            viewport: Rect::default(),
        }
    }

    pub fn add_child(&mut self, parent: NodeId, style: Style) -> NodeId {
        let id = NodeId(self.nodes.len());
        self.nodes.push(Node {
            style,
            children: Vec::new(),
            rect: Rect::default(),
            min: (0.0, 0.0),
        });
        self.nodes[parent.0].children.push(id);
        self.dirty = true;
        id
    }

    pub fn style(&self, id: NodeId) -> &Style {
        &self.nodes[id.0].style
    }

    /// Edits a node's style, marking the tree dirty **only if the edit
    /// changed something**. That check is what lets an animation write a
    /// width every frame without forcing a solve once it has settled.
    pub fn set_style(&mut self, id: NodeId, edit: impl FnOnce(&mut Style)) {
        let before = self.nodes[id.0].style;
        edit(&mut self.nodes[id.0].style);
        if self.nodes[id.0].style != before {
            self.dirty = true;
        }
    }

    /// The solved rect. Zero-sized until the first [`Layout::compute`].
    pub fn rect(&self, id: NodeId) -> Rect {
        self.nodes[id.0].rect
    }

    /// What this node's whole subtree needs, in logical pixels. Valid after
    /// [`Layout::compute`]. On [`Layout::ROOT`] this is the smallest the
    /// window may be without regions starting to overlap.
    pub fn min_size(&self, id: NodeId) -> (f32, f32) {
        self.nodes[id.0].min
    }

    /// Solves the tree into `viewport`, or does nothing if neither the
    /// styles nor the viewport have changed since the last solve. Returns
    /// whether it actually solved — useful for asserting in a test or
    /// showing a counter in a status line.
    pub fn compute(&mut self, viewport: Rect) -> bool {
        if !self.dirty && viewport == self.viewport {
            return false;
        }
        self.viewport = viewport;
        self.dirty = false;
        self.measure(Self::ROOT);
        self.solve(Self::ROOT, viewport);
        true
    }

    /// Bottom-up: what does each subtree need? A child contributes its own
    /// minimum, or its fixed extent if that is larger — a fixed child never
    /// shrinks, so its parent has to account for all of it.
    fn measure(&mut self, id: NodeId) -> (f32, f32) {
        let Style {
            axis,
            gap,
            min_size,
            ..
        } = self.nodes[id.0].style;
        let children = self.visible_children(id);

        let (mut main, mut cross) = (0.0f32, 0.0f32);
        for child in &children {
            let (width, height) = self.measure(*child);
            let (child_main, child_cross) = match axis {
                Axis::Row => (width, height),
                Axis::Column => (height, width),
            };
            let child_main = match self.nodes[child.0].style.size {
                Size::Fixed(v) => child_main.max(v),
                Size::Flex(_) => child_main,
            };
            main += child_main;
            cross = cross.max(child_cross);
        }
        if children.len() > 1 {
            main += gap * (children.len() - 1) as f32;
        }

        let derived = match axis {
            Axis::Row => (main, cross),
            Axis::Column => (cross, main),
        };
        let min = (derived.0.max(min_size.0), derived.1.max(min_size.1));
        self.nodes[id.0].min = min;
        min
    }

    fn solve(&mut self, id: NodeId, rect: Rect) {
        self.nodes[id.0].rect = rect;

        let Style { axis, gap, .. } = self.nodes[id.0].style;
        let children = self.visible_children(id);
        if children.is_empty() {
            return;
        }

        let extent = match axis {
            Axis::Row => rect.width,
            Axis::Column => rect.height,
        };
        let available = (extent - gap * (children.len() - 1) as f32).max(0.0);
        let sizes = self.distribute(&children, axis, available);

        let mut cursor = match axis {
            Axis::Row => rect.x,
            Axis::Column => rect.y,
        };
        for (i, child) in children.iter().enumerate() {
            // Snap both edges: one child's far edge is the next one's near
            // edge, so rounding can never open a seam or an overlap.
            let start = cursor.round();
            let end = (cursor + sizes[i]).round();
            let child_rect = match axis {
                Axis::Row => Rect::new(start, rect.y, end - start, rect.height),
                Axis::Column => Rect::new(rect.x, start, rect.width, end - start),
            };
            cursor += sizes[i];
            if i + 1 < children.len() {
                cursor += gap;
            }
            self.solve(*child, child_rect);
        }
    }

    /// Main-axis extent per child. Fixed children take their extent
    /// *exactly* — see [`Size::Fixed`] — while flexible ones split the
    /// remainder by weight and are never pushed under their minimum: one
    /// that would be is frozen there and the rest re-split what is left.
    /// Each pass freezes at least one child, so this terminates in at most
    /// `children.len()` passes.
    fn distribute(&self, children: &[NodeId], axis: Axis, available: f32) -> Vec<f32> {
        let min_of = |id: &NodeId| {
            let (w, h) = self.nodes[id.0].min;
            match axis {
                Axis::Row => w,
                Axis::Column => h,
            }
        };

        let mut sizes: Vec<f32> = Vec::with_capacity(children.len());
        let mut frozen: Vec<bool> = Vec::with_capacity(children.len());
        for child in children {
            match self.nodes[child.0].style.size {
                Size::Fixed(v) => {
                    sizes.push(v.max(0.0));
                    frozen.push(true);
                }
                Size::Flex(_) => {
                    sizes.push(min_of(child));
                    frozen.push(false);
                }
            }
        }

        loop {
            let taken: f32 = sizes
                .iter()
                .zip(&frozen)
                .filter(|(_, f)| **f)
                .map(|(s, _)| *s)
                .sum();
            let weights: f32 = children
                .iter()
                .zip(&frozen)
                .filter(|(_, f)| !**f)
                .map(|(c, _)| match self.nodes[c.0].style.size {
                    Size::Flex(w) => w.max(0.0),
                    Size::Fixed(_) => 0.0,
                })
                .sum();
            if weights <= 0.0 {
                // Everything is frozen. If the minimums do not fit, the
                // container overflows — the window's minimum size exists to
                // keep that off screen.
                break;
            }

            let leftover = (available - taken).max(0.0);
            let mut violated = false;
            for (i, child) in children.iter().enumerate() {
                if frozen[i] {
                    continue;
                }
                let Size::Flex(weight) = self.nodes[child.0].style.size else {
                    continue;
                };
                let share = leftover * weight.max(0.0) / weights;
                let min = min_of(child);
                if share < min {
                    sizes[i] = min;
                    frozen[i] = true;
                    violated = true;
                } else {
                    sizes[i] = share;
                }
            }
            if !violated {
                break;
            }
        }
        sizes
    }

    fn visible_children(&self, id: NodeId) -> Vec<NodeId> {
        self.nodes[id.0]
            .children
            .iter()
            .copied()
            .filter(|c| self.nodes[c.0].style.visible)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEWPORT: Rect = Rect::new(0.0, 0.0, 1000.0, 600.0);

    /// Root column with a fixed header, a flexible body, a fixed footer.
    fn shell() -> (Layout, NodeId, NodeId, NodeId) {
        let mut layout = Layout::new(Style::default());
        let header = layout.add_child(Layout::ROOT, Style::fixed(30.0));
        let body = layout.add_child(Layout::ROOT, Style::flex(1.0).row());
        let footer = layout.add_child(Layout::ROOT, Style::fixed(20.0));
        (layout, header, body, footer)
    }

    #[test]
    fn flex_takes_what_fixed_children_left() {
        let (mut layout, header, body, footer) = shell();
        layout.compute(VIEWPORT);

        assert_eq!(layout.rect(header), Rect::new(0.0, 0.0, 1000.0, 30.0));
        assert_eq!(layout.rect(body), Rect::new(0.0, 30.0, 1000.0, 550.0));
        assert_eq!(layout.rect(footer), Rect::new(0.0, 580.0, 1000.0, 20.0));
    }

    #[test]
    fn siblings_share_edges_exactly_at_fractional_widths() {
        let mut layout = Layout::new(Style::default().row());
        // Thirds of 1000 do not land on whole pixels.
        let a = layout.add_child(Layout::ROOT, Style::flex(1.0));
        let b = layout.add_child(Layout::ROOT, Style::flex(1.0));
        let c = layout.add_child(Layout::ROOT, Style::flex(1.0));
        layout.compute(VIEWPORT);

        assert_eq!(layout.rect(a).right(), layout.rect(b).x);
        assert_eq!(layout.rect(b).right(), layout.rect(c).x);
        assert_eq!(layout.rect(c).right(), VIEWPORT.right());
    }

    #[test]
    fn a_hidden_child_takes_no_space_and_no_gap() {
        let mut layout = Layout::new(Style::default().row().gap(10.0));
        let side = layout.add_child(Layout::ROOT, Style::fixed(200.0));
        let main = layout.add_child(Layout::ROOT, Style::flex(1.0));
        layout.compute(VIEWPORT);
        assert_eq!(layout.rect(main), Rect::new(210.0, 0.0, 790.0, 600.0));

        layout.set_style(side, |s| s.visible = false);
        layout.compute(VIEWPORT);
        // The gap goes with the panel: no 10px dead strip on the left.
        assert_eq!(layout.rect(main), VIEWPORT);
    }

    #[test]
    fn nested_axes_flip() {
        let (mut layout, _, body, _) = shell();
        let tree = layout.add_child(body, Style::fixed(200.0));
        let text = layout.add_child(body, Style::flex(1.0));
        layout.compute(VIEWPORT);

        assert_eq!(layout.rect(tree), Rect::new(0.0, 30.0, 200.0, 550.0));
        assert_eq!(layout.rect(text), Rect::new(200.0, 30.0, 800.0, 550.0));
    }

    #[test]
    fn it_only_solves_when_something_changed() {
        let (mut layout, header, ..) = shell();
        assert!(layout.compute(VIEWPORT), "first solve always runs");
        assert!(!layout.compute(VIEWPORT), "nothing changed");

        // An animation writing the width it already has must not resolve.
        layout.set_style(header, |s| s.size = Size::Fixed(30.0));
        assert!(!layout.compute(VIEWPORT), "style written but unchanged");

        layout.set_style(header, |s| s.size = Size::Fixed(31.0));
        assert!(layout.compute(VIEWPORT), "style changed");

        assert!(
            layout.compute(Rect::new(0.0, 0.0, 800.0, 600.0)),
            "viewport changed"
        );
    }

    #[test]
    fn rects_hit_test_in_the_same_space_the_mouse_reports() {
        let (mut layout, header, body, _) = shell();
        layout.compute(VIEWPORT);

        assert!(layout.rect(header).contains((500.0, 10.0)));
        assert!(!layout.rect(header).contains((500.0, 30.0)));
        assert!(layout.rect(body).contains((500.0, 30.0)));
    }

    #[test]
    fn a_flexible_child_stops_at_its_minimum() {
        let mut layout = Layout::new(Style::default().row());
        let side = layout.add_child(Layout::ROOT, Style::fixed(200.0));
        let text = layout.add_child(Layout::ROOT, Style::flex(1.0).min(320.0, 0.0));

        // Plenty of room: the minimum does not bind.
        layout.compute(VIEWPORT);
        assert_eq!(layout.rect(text).width, 800.0);

        // Squeezed: the text column keeps its 320 and overflows the window
        // rather than being drawn over.
        layout.compute(Rect::new(0.0, 0.0, 400.0, 600.0));
        assert_eq!(layout.rect(text).width, 320.0);
        assert_eq!(layout.rect(side).width, 200.0);
    }

    #[test]
    fn one_frozen_child_does_not_starve_the_others() {
        let mut layout = Layout::new(Style::default().row());
        let pinned = layout.add_child(Layout::ROOT, Style::flex(1.0).min(400.0, 0.0));
        let free_a = layout.add_child(Layout::ROOT, Style::flex(1.0));
        let free_b = layout.add_child(Layout::ROOT, Style::flex(1.0));

        // Even thirds would give 333 each, under `pinned`'s 400. It freezes
        // at 400 and the other two split the remaining 600 evenly — not
        // 333/333 with 267 left unused.
        layout.compute(VIEWPORT);
        assert_eq!(layout.rect(pinned).width, 400.0);
        assert_eq!(layout.rect(free_a).width, 300.0);
        assert_eq!(layout.rect(free_b).width, 300.0);
    }

    #[test]
    fn minimums_add_up_through_the_tree() {
        let (mut layout, header, body, footer) = shell();
        layout.set_style(header, |s| s.min_size = (260.0, 0.0));
        layout.set_style(footer, |s| s.min_size = (300.0, 0.0));
        let tree = layout.add_child(body, Style::fixed(200.0).min(130.0, 0.0));
        let text = layout.add_child(body, Style::flex(1.0).min(320.0, 200.0));
        layout.compute(VIEWPORT);

        // Row: minimums side by side, and a fixed child counts for all of
        // its extent. Column: they stack, and the widest wins.
        assert_eq!(layout.min_size(body), (520.0, 200.0));
        assert_eq!(layout.min_size(Layout::ROOT), (520.0, 250.0));
        assert_eq!(layout.min_size(tree).0, 130.0);
        assert_eq!(layout.min_size(text).0, 320.0);

        // Collapsing a panel lowers the floor by exactly its share.
        layout.set_style(tree, |s| s.visible = false);
        layout.compute(VIEWPORT);
        assert_eq!(layout.min_size(body), (320.0, 200.0));
    }

    #[test]
    fn a_fixed_extent_under_the_content_minimum_is_honoured_exactly() {
        // The collapse-animation bug: a panel driven from 200 down to 0 by
        // an animation used to stick at its content minimum for most of the
        // travel and then vanish in one step when it finally hid. Every
        // frame below the minimum has to solve to the extent it was given.
        let (mut layout, _, body, _) = shell();
        let tree = layout.add_child(body, Style::fixed(200.0).min(130.0, 0.0));
        let text = layout.add_child(body, Style::flex(1.0));

        for extent in [200.0, 130.0, 129.0, 64.0, 1.0, 0.0] {
            layout.set_style(tree, |s| s.size = Size::Fixed(extent));
            layout.compute(VIEWPORT);
            assert_eq!(
                layout.rect(tree).width,
                extent,
                "a fixed {extent} was floored at the content minimum"
            );
        }
        // What the panel gave up went to its flexible neighbour, rather
        // than being left as a hole.
        assert_eq!(layout.rect(text).width, VIEWPORT.width);
    }
}
