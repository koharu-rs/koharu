//! The canvas's texture and how a redraw composes it.

use koharu_renderer::{Gpu, View};
use vello::{
    Scene,
    kurbo::{Affine, Point, Rect, TranslateScale},
    wgpu::{
        self, BlendState, TextureFormat, TextureUsages,
        util::{TextureBlitter, TextureBlitterBuilder},
    },
};

/// Where a strip's corner lands in the viewport: where the camera shows the
/// view's corner, plus the strip's own place in the view's output.
pub(crate) fn strip_origin(camera: TranslateScale, view: View, strip: Rect) -> Point {
    camera * view.rect.origin() + strip.origin().to_vec2()
}

/// The canvas's texture, the viewport's size, with the scratch texture a
/// further pass renders into and the blitter that blends it over.
pub(crate) struct Target {
    pub texture: wgpu::Texture,
    view: wgpu::TextureView,
    scratch: Option<(wgpu::Texture, wgpu::TextureView)>,
    blitter: TextureBlitter,
}

impl Target {
    pub fn new(gpu: &Gpu, width: u32, height: u32) -> Self {
        // Storage for vello, a render attachment for blending, sampled by the
        // host, and copied out for a snapshot.
        let texture = texture(
            gpu,
            width,
            height,
            TextureUsages::STORAGE_BINDING
                | TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        // vello writes straight alpha.
        let blitter = TextureBlitterBuilder::new(&gpu.device, TextureFormat::Rgba8Unorm)
            .blend_state(BlendState::ALPHA_BLENDING)
            .build();
        Self {
            texture,
            view,
            scratch: None,
            blitter,
        }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.texture.width(), self.texture.height())
    }

    /// Draws `base`, then each strip at its origin, then `overlay`: in one
    /// pass when the page takes one strip. vello writes every pixel of its
    /// target, so a further strip, and then the overlay, render apart and
    /// are blended over.
    pub fn draw(
        &mut self,
        gpu: &Gpu,
        mut base: Scene,
        strips: Vec<(Point, Scene)>,
        overlay: &Scene,
    ) -> anyhow::Result<()> {
        let (width, height) = self.size();
        let mut strips = strips.into_iter();
        if let Some((origin, scene)) = strips.next() {
            base.append(&scene, Some(Affine::translate(origin.to_vec2())));
        }
        let rest: Vec<_> = strips.collect();
        if rest.is_empty() {
            base.append(overlay, None);
        }
        gpu.render(&base, &self.view, width, height)?;
        if rest.is_empty() {
            return Ok(());
        }
        for (origin, scene) in rest {
            let mut placed = Scene::new();
            placed.append(&scene, Some(Affine::translate(origin.to_vec2())));
            self.blend(gpu, &placed)?;
        }
        self.blend(gpu, overlay)
    }

    /// Renders `scene` apart and blends it over what's drawn.
    fn blend(&mut self, gpu: &Gpu, scene: &Scene) -> anyhow::Result<()> {
        let (width, height) = self.size();
        let (_, scratch) = self.scratch.get_or_insert_with(|| {
            let texture = texture(
                gpu,
                width,
                height,
                TextureUsages::STORAGE_BINDING | TextureUsages::TEXTURE_BINDING,
            );
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            (texture, view)
        });
        gpu.render(scene, scratch, width, height)?;
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("koharu canvas blend"),
            });
        self.blitter
            .copy(&gpu.device, &mut encoder, scratch, &self.view);
        gpu.queue.submit([encoder.finish()]);
        Ok(())
    }
}

fn texture(gpu: &Gpu, width: u32, height: u32, usage: TextureUsages) -> wgpu::Texture {
    gpu.device.create_texture(&wgpu::TextureDescriptor {
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
        usage,
        view_formats: &[],
    })
}

#[cfg(test)]
mod tests {
    use vello::kurbo::Vec2;

    use super::*;

    #[test]
    fn each_strip_lands_where_its_page_pixels_show() {
        let camera = TranslateScale::new(Vec2::new(10.5, -20.0), 2.0);
        let view = View {
            rect: Rect::new(100.0, 50.0, 400.0, 650.0),
            scale: 2.0,
        };
        let first = strip_origin(camera, view, Rect::new(0.0, 0.0, 600.0, 600.0));
        let second = strip_origin(camera, view, Rect::new(0.0, 600.0, 600.0, 1200.0));
        assert_eq!(first, Point::new(210.5, 80.0));
        assert_eq!(second, Point::new(210.5, 680.0));
    }
}
