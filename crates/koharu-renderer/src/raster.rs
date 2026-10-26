//! The GPU: one wgpu device and vello renderer, rendering scenes to textures
//! and reading them back.

use std::{
    collections::HashSet,
    sync::{LazyLock, mpsc},
};

use anyhow::{Context, Result, anyhow};
use image::RgbaImage;
use parking_lot::Mutex;
use vello::{
    AaConfig, AaSupport, RenderParams, RendererOptions, Scene,
    kurbo::{Affine, Rect},
    low_level::vello_encoding::Patch,
    peniko::{Blob, Color, Fill, ImageAlphaType, ImageBrush, ImageData, ImageFormat},
    wgpu::{
        self, Buffer, BufferDescriptor, BufferUsages, CommandEncoderDescriptor, Extent3d,
        TexelCopyBufferInfo, Texture, TextureDescriptor, TextureFormat, TextureUsages, TextureView,
    },
};

/// The pixels vello 0.11's image atlas holds at most.
const ATLAS: u64 = 8192 * 8192;

/// One transparent pixel, drawn so a render always has an image.
static PIXEL: LazyLock<ImageData> = LazyLock::new(|| ImageData {
    data: Blob::from(vec![0; 4]),
    format: ImageFormat::Rgba8,
    alpha_type: ImageAlphaType::Alpha,
    width: 1,
    height: 1,
});

/// The device, its queue, and the vello renderer, shared by export and the
/// desktop canvas.
pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    vello: Mutex<Vello>,
    targets: Mutex<Vec<Target>>,
}

struct Vello {
    renderer: vello::Renderer,
    /// The image pixels each of the last two renders drew.
    recent: [u64; 2],
}

struct Target {
    width: u32,
    height: u32,
    padded_width: u32,
    texture: Texture,
    view: TextureView,
    readback: Buffer,
}

impl Gpu {
    pub(crate) fn new() -> Result<Self> {
        let (device, queue) = pollster::block_on(request_device())?;
        let renderer = vello::Renderer::new(
            &device,
            RendererOptions {
                antialiasing_support: AaSupport::area_only(),
                ..Default::default()
            },
        )
        .map_err(|error| anyhow!("failed to create the vello renderer: {error:?}"))?;
        Ok(Self {
            device,
            queue,
            vello: Mutex::new(Vello {
                renderer,
                recent: [0; 2],
            }),
            targets: Mutex::new(Vec::new()),
        })
    }

    /// Renders `scene` onto `target`, a transparent `width`×`height`
    /// `Rgba8Unorm` storage texture. Every vello render goes through here,
    /// which works around two faults of vello 0.11's image atlas:
    ///
    /// - A render with no image, gradient, or glyph frees the atlas while
    ///   vello still counts its images as uploaded, so they draw transparent
    ///   from then on [vello#1948]. Such a scene gets a transparent pixel.
    /// - vello never evicts an image either of the last two renders drew
    ///   [vello#2011], so a render whose images don't fit beside theirs loses
    ///   some, as consecutive strips of an export would. Two renders of the
    ///   pixel alone come first, so those images can go.
    ///
    /// [vello#1948]: https://github.com/linebender/vello/issues/1948
    /// [vello#2011]: https://github.com/linebender/vello/issues/2011
    pub fn render(
        &self,
        scene: &Scene,
        target: &TextureView,
        width: u32,
        height: u32,
    ) -> Result<()> {
        let mut vello = self.vello.lock();
        let pixels = image_pixels(scene);
        if pixels > 0 && vello.recent.iter().sum::<u64>() + pixels > ATLAS {
            let spacer = self.target(1, 1)?;
            let mut pixel = Scene::new();
            draw_pixel(&mut pixel);
            for _ in 0..2 {
                vello.draw(&self.device, &self.queue, &pixel, &spacer.view, 1, 1)?;
            }
            self.release(spacer);
            vello.recent = [0; 2];
        }
        if scene.encoding().resources.patches.is_empty() {
            let mut padded = Scene::new();
            padded.append(scene, None);
            draw_pixel(&mut padded);
            vello.draw(&self.device, &self.queue, &padded, target, width, height)?;
        } else {
            vello.draw(&self.device, &self.queue, scene, target, width, height)?;
        }
        vello.recent = [vello.recent[1], pixels];
        Ok(())
    }

    /// Renders `scene` onto a transparent `width`×`height` surface and reads
    /// it back as straight-alpha RGBA.
    pub(crate) fn read(&self, scene: &Scene, width: u32, height: u32) -> Result<RgbaImage> {
        let limit = self.device.limits().max_texture_dimension_2d;
        if width > limit || height > limit {
            anyhow::bail!(
                "a {width}×{height} pass is larger than this GPU's {limit}-pixel textures"
            );
        }
        let target = self.target(width, height)?;
        self.render(scene, &target.view, width, height)?;
        let mut encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("koharu readback"),
            });
        encoder.copy_texture_to_buffer(
            target.texture.as_image_copy(),
            TexelCopyBufferInfo {
                buffer: &target.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(target.padded_width),
                    rows_per_image: None,
                },
            },
            Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let submission = self.queue.submit([encoder.finish()]);
        let slice = target.readback.slice(..);
        let (sender, receiver) = mpsc::sync_channel(1);
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .map_err(|error| anyhow!("waiting for the GPU failed: {error:?}"))?;
        receiver
            .recv()
            .context("the GPU closed the readback")?
            .context("failed to map the readback buffer")?;
        let mapped = slice
            .get_mapped_range()
            .context("failed to read the readback buffer")?;
        let row = (width * 4) as usize;
        let mut pixels = Vec::with_capacity(row * height as usize);
        for line in mapped
            .chunks_exact(target.padded_width as usize)
            .take(height as usize)
        {
            pixels.extend_from_slice(&line[..row]);
        }
        drop(mapped);
        target.readback.unmap();
        self.release(target);
        RgbaImage::from_raw(width, height, pixels).context("the readback has the wrong size")
    }

    fn release(&self, target: Target) {
        let mut targets = self.targets.lock();
        if targets.len() < 4 {
            targets.push(target);
        }
    }

    fn target(&self, width: u32, height: u32) -> Result<Target> {
        let mut targets = self.targets.lock();
        if let Some(at) = targets
            .iter()
            .position(|target| target.width == width && target.height == height)
        {
            return Ok(targets.swap_remove(at));
        }
        drop(targets);
        let texture = self.device.create_texture(&TextureDescriptor {
            label: Some("koharu render target"),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::STORAGE_BINDING | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let padded_width = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = self.device.create_buffer(&BufferDescriptor {
            label: Some("koharu readback buffer"),
            size: u64::from(padded_width) * u64::from(height),
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Target {
            width,
            height,
            padded_width,
            texture,
            view,
            readback,
        })
    }
}

impl Vello {
    fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &Scene,
        target: &TextureView,
        width: u32,
        height: u32,
    ) -> Result<()> {
        let params = RenderParams {
            base_color: Color::TRANSPARENT,
            width,
            height,
            antialiasing_method: AaConfig::Area,
        };
        self.renderer
            .render_to_texture(device, queue, scene, target, &params)
            .map_err(|error| anyhow!("vello rendering failed: {error:?}"))
    }
}

/// The pixels of the distinct images `scene` draws.
fn image_pixels(scene: &Scene) -> u64 {
    let mut seen = HashSet::new();
    scene
        .encoding()
        .resources
        .patches
        .iter()
        .filter_map(|patch| match patch {
            Patch::Image { image, .. } if seen.insert(image.data.id()) => {
                Some(u64::from(image.width) * u64::from(image.height))
            }
            _ => None,
        })
        .sum()
}

fn draw_pixel(scene: &mut Scene) {
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        &ImageBrush::new(PIXEL.clone()),
        None,
        &Rect::new(0.0, 0.0, 1.0, 1.0),
    );
}

/// Creates the device as vello's `RenderContext` does, but with the
/// adapter's texture and buffer limits: vello asks for `Limits::default()`,
/// which caps a surface at 8192 pixels whatever the GPU supports.
async fn request_device() -> Result<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        display: None,
        backends: wgpu::Backends::from_env().unwrap_or_default(),
        flags: wgpu::InstanceFlags::from_build_config().with_env(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        backend_options: wgpu::BackendOptions::from_env_or_default(),
    });
    let adapter = wgpu::util::initialize_adapter_from_env_or_default(&instance, None)
        .await
        .context("no GPU adapter is available")?;
    let supported = adapter.limits();
    let required_limits = wgpu::Limits {
        max_texture_dimension_2d: supported.max_texture_dimension_2d,
        max_buffer_size: supported.max_buffer_size,
        ..wgpu::Limits::default()
    };
    let optional = wgpu::Features::CLEAR_TEXTURE | wgpu::Features::PIPELINE_CACHE;
    adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("koharu renderer"),
            required_features: adapter.features() & optional,
            required_limits,
            ..Default::default()
        })
        .await
        .context("no GPU adapter supports vello's features")
}
