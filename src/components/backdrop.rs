//! The window's floor.

use crate::layout::Rect;
use crate::renderer::{Layer, Rounding};
use crate::theme;
use crate::ui::Component;

/// Fills the window behind everything, so a resize never flashes.
pub struct Backdrop;

impl Component for Backdrop {
    fn draw(&mut self, layer: &Layer, rect: Rect) {
        layer.draw_rectangle(
            rect.position(),
            rect.size(),
            theme::background(),
            Rounding::NONE,
        );
    }
}
