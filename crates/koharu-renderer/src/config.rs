use serde::{Deserialize, Serialize};
use specta::Type;

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
