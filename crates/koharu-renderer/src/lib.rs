//! Renders Koharu scene pages with vello: layout, retained layer content,
//! and the GPU passes that draw it.

use serde::{Deserialize, Serialize};
use specta::Type;

pub mod fonts;
mod frame;
mod gpu;
mod pixels;
mod renderer;
mod script;
mod text;

pub use frame::{Drawn, Frame, Raster};
pub use gpu::{Drawing, Gpu};
pub use renderer::Renderer;
pub use text::{ResolvedText, TextRun};

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, Type)]
#[serde(default)]
pub struct RendererConfig {
    pub font_families: Vec<String>,
}

impl Default for RendererConfig {
    fn default() -> Self {
        Self {
            font_families: vec!["CCWildWords".to_owned(), "Adobe 黑体 Std".to_owned()],
        }
    }
}

impl koharu_config::Section for RendererConfig {
    const NAME: &'static str = "renderer";
}
