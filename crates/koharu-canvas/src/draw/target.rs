//! The canvas's texture, which a redraw renders into whole.

use std::sync::Arc;

use koharu_renderer::Gpu;
use wgpu::{TextureFormat, TextureUsages};

/// The canvas's texture, the viewport's size, on the GPU that made it.
pub(crate) struct Target {
    pub gpu: Arc<Gpu>,
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
}

impl Target {
    pub fn new(gpu: &Arc<Gpu>, width: u32, height: u32) -> Self {
        // Rendered into by vello_gpu, sampled by the host, and copied out
        // for a snapshot.
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("koharu canvas"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            gpu: Arc::clone(gpu),
            texture,
            view,
        }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.texture.width(), self.texture.height())
    }
}
