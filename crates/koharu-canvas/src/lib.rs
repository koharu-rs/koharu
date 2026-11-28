//! The page viewport: draws a page with the renderer and turns window-space
//! pointer events into edits for the host to apply.

mod area;
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
mod typing;
mod view;

pub use canvas::{Canvas, Facing, Side};
pub use effect::{Effect, Notice, PainterPress};
pub use kurbo::{Point, Rect, Size, TranslateScale, Vec2};
pub use settings::{Brush, Colors, Story, Tool};
pub use snapshot::Snapshot;
pub use typing::{CharKey, RubyBase, RubyEdit, Typed};
pub use view::Camera;
