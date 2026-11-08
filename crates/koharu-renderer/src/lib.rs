//! Renders Koharu scene pages with vello: layout, retained layer content,
//! and the GPU passes that draw it.

mod config;
mod error;
mod fonts;
mod frame;
mod gpu;
mod images;
mod renderer;
mod script;
mod text;
mod tiles;
mod types;

pub use config::RendererConfig;
pub use error::{Error, Result};
pub use frame::{Drawn, Frame, Strip, View};
pub use gpu::Gpu;
pub use renderer::{Raster, Renderer};
pub use text::{ResolvedText, TextRun};
pub use types::{FontFace, FontFamily, FontMetadata, FontSource, FontStyle};
