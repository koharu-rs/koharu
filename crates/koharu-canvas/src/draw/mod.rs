//! How a redraw composes the canvas's texture: the page, then the overlay.

pub(crate) mod overlay;
mod target;

use koharu_scene::Rgba;
use vello_gpu::peniko::Color;

pub(crate) use target::Target;

pub(crate) fn color(Rgba(components): Rgba) -> Color {
    Color::new(components)
}
