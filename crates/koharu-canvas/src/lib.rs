//! The page viewport: draws a page with the renderer and turns window-space
//! pointer events into edits for the host to apply.

mod canvas;
mod control;
mod draw;
mod effect;
mod gesture;
mod selection;
mod settings;
mod snapshot;
#[cfg(test)]
mod tests;
mod tools;
mod view;

pub use canvas::Canvas;
pub use effect::{Effect, Notice};
pub use settings::{Brush, Colors, Tool};
pub use vello::kurbo::{Point, Rect, Size, TranslateScale, Vec2};
pub use view::Camera;
