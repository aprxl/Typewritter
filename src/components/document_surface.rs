//! The document sheet beneath the editor and its annotation margin.

use crate::layout::Rect;
use crate::renderer::Layer;
use crate::theme;
use crate::ui::Component;

pub const GAP: f32 = 12.0;
pub const RADIUS: f32 = 12.0;

pub struct DocumentSurface;

impl Component for DocumentSurface {
    fn draw(&mut self, layer: &Layer, rect: Rect) {
        theme::surface(layer, rect, theme::background(), RADIUS);
        theme::rounded_outline(layer, rect.inset(0.5), RADIUS - 0.5, 1.0, theme::border());
    }
}
