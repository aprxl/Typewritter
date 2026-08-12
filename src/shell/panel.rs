//! A region that collapses to nothing and back.

use std::time::Duration;

use crate::animation::{Animation, Easing};
use crate::layout::NodeId;

/// The animation interpolates from wherever the panel *was* to wherever it
/// is going, rather than across the panel's full extent, so toggling
/// mid-flight continues from the current width instead of snapping.
pub struct Panel {
    pub node: NodeId,
    pub animation: Animation,
    pub open: bool,
    /// Extent when fully open. Mutable: the tree's is drag-resizable.
    extent: f32,
    from: f32,
    to: f32,
}

impl Panel {
    pub fn new(node: NodeId, extent: f32) -> Self {
        Self {
            node,
            animation: Animation::new(Duration::from_millis(140), Easing::EaseOut),
            open: true,
            extent,
            from: extent,
            to: extent,
        }
    }

    pub fn current(&self) -> f32 {
        self.animation.value(self.from, self.to)
    }

    pub fn toggle(&mut self) {
        self.from = self.current();
        self.open = !self.open;
        self.to = if self.open { self.extent } else { 0.0 };
        self.animation.restart();
    }

    pub fn set_open(&mut self, open: bool) {
        if open != self.open {
            self.toggle();
        }
    }

    /// Drag-resize: takes effect immediately, with no animation to fight.
    pub fn resize(&mut self, extent: f32) {
        self.extent = extent;
        self.from = extent;
        self.to = extent;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Layout;

    /// Longer than the animation, so it lands on its end state.
    fn settle(panel: &mut Panel) {
        panel.animation.advance(Duration::from_millis(500));
    }

    #[test]
    fn a_panel_collapses_to_nothing_and_back() {
        let mut panel = Panel::new(Layout::ROOT, 200.0);
        assert_eq!(panel.current(), 200.0);

        panel.toggle();
        settle(&mut panel);
        assert_eq!(panel.current(), 0.0);

        panel.toggle();
        settle(&mut panel);
        assert_eq!(panel.current(), 200.0);
    }

    #[test]
    fn toggling_mid_flight_continues_from_the_current_width() {
        let mut panel = Panel::new(Layout::ROOT, 200.0);
        panel.toggle();
        panel.animation.advance(Duration::from_millis(70));

        let width = panel.current();
        assert!(width > 0.0 && width < 200.0, "mid-collapse width {width}");

        // Re-opening has to start from where the panel actually is; the
        // naive version snaps back to full extent first, and it shows.
        panel.toggle();
        assert_eq!(panel.current(), width);
        settle(&mut panel);
        assert_eq!(panel.current(), 200.0);
    }

    #[test]
    fn a_resize_lands_immediately_and_survives_a_collapse() {
        let mut panel = Panel::new(Layout::ROOT, 200.0);
        panel.resize(320.0);
        assert_eq!(panel.current(), 320.0, "a drag has no animation to fight");

        panel.toggle();
        settle(&mut panel);
        assert_eq!(panel.current(), 0.0);
        panel.toggle();
        settle(&mut panel);
        assert_eq!(panel.current(), 320.0, "re-opens at the dragged width");
    }
}
