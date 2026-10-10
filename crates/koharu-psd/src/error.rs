use thiserror::Error;

#[derive(Debug, Error)]
pub enum PsdExportError {
    #[error("classic PSD only supports dimensions up to 30000x30000, got {width}x{height}")]
    UnsupportedDimensions { width: u32, height: u32 },
    #[error("page {page} is missing the {role:?} scene asset")]
    MissingAsset {
        page: koharu_scene::EntityId,
        role: &'static str,
    },
    #[error("renderer did not produce pixels for scene entity {0}")]
    MissingRenderedEntity(koharu_scene::EntityId),
    #[error("PSD layer count exceeds the classic format limit: {0}")]
    TooManyLayers(usize),
    #[error("mask dimensions differ: {left_width}x{left_height} and {right_width}x{right_height}")]
    MismatchedMaskDimensions {
        left_width: u32,
        left_height: u32,
        right_width: u32,
        right_height: u32,
    },
    #[error("invalid layer bounds for {layer}: {width}x{height}")]
    InvalidLayerBounds {
        layer: String,
        width: i32,
        height: i32,
    },
    #[error("RLE row {row} for {layer} exceeded PSD limits ({length} bytes)")]
    InvalidChannelEncoding {
        layer: String,
        row: usize,
        length: usize,
    },
    #[error("invalid descriptor data: {0}")]
    InvalidDescriptor(String),
    #[error(transparent)]
    Scene(#[from] koharu_scene::Error),
    #[error(transparent)]
    Renderer(#[from] koharu_renderer::Error),
    #[error(transparent)]
    Rasterizer(#[from] koharu_rasterizer::Error),
    #[error(transparent)]
    Image(#[from] image::ImageError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
