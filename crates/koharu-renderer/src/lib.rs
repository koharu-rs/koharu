//! Renders Koharu scene pages with vello: layout, retained layer content,
//! and the GPU passes that draw it.

mod config;
mod error;
mod fonts;
mod frame;
mod images;
mod layout;
mod raster;
mod renderer;
mod script;
mod segment;
mod shape;
mod text;
mod tiles;
mod types;

pub use config::TypesettingConfig;
pub use error::{Error, Result};
pub use frame::{Frame, Layer, Strip, View};
pub use raster::Gpu;
pub use renderer::{Raster, Renderer};
pub use text::{ResolvedText, TextRun};
pub use types::{FontFace, FontFamily, FontMetadata, FontRange, FontSource, FontStyle};
