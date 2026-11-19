//! The GPU: one wgpu device and a vello_gpu renderer, which draws every page
//! Koharu shows or exports.

use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

use anyhow::{Context, Result, anyhow, bail};
use glifo::Glyph;
use image::RgbaImage;
use kurbo::{Affine, Stroke};
use parking_lot::{Mutex, MutexGuard};
use vello_gpu::{
    ImageSource, Pixmap, RenderOptions, RenderTarget, RenderTargetConfig, Renderer, Resources,
    Scene, TextureBindings, TextureId, TextureRegion,
};
use wgpu::{
    BufferDescriptor, BufferUsages, CommandEncoderDescriptor, Extent3d, TexelCopyBufferInfo,
    Texture, TextureDescriptor, TextureFormat, TextureUsages, TextureView, util::DeviceExt as _,
};

use crate::fonts::Font;

/// The process's GPU: started the first time it's asked for, and again the
/// first time after its device is lost, as a driver reset or a GPU out of
/// memory loses it.
static GPU: Mutex<Option<Arc<Gpu>>> = Mutex::new(None);

/// The device and its queue, which the desktop's compositor shares, and
/// vello_gpu's renderer. The instance and adapter stay, because a window's
/// surface must come from the instance the device came from, on an adapter
/// that presents to it.
pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    /// Set when wgpu reports the device lost.
    lost: Arc<AtomicBool>,
    /// What one drawing holds at a time.
    vello: Mutex<State>,
}

/// What the uploaded images may take on the GPU, in bytes. Kept only while
/// the retained content held their pixmaps, every page viewed held a
/// texture, up to that cache's 2 GiB, on the GPU the models use. Past this,
/// the coldest goes, and is uploaded again when it's next drawn.
const TEXTURE_BYTES: u64 = 256 << 20;

/// vello_gpu's renderer, the glyphs it keeps, and each image uploaded as a
/// texture of its own, by its pixmap's address, kept while the pixmap lives
/// and `TEXTURE_BYTES` has room.
struct State {
    renderer: Renderer,
    resources: Resources,
    textures: quick_cache::unsync::Cache<usize, Upload, Bytes>,
    /// The id the next uploaded image is named by.
    next_texture: u64,
}

/// An image's texture, and the pixmap it was uploaded from.
struct Upload {
    pixmap: Weak<Pixmap>,
    id: TextureId,
    view: TextureView,
    bytes: u64,
}

#[derive(Clone)]
struct Bytes;

impl quick_cache::Weighter<usize, Upload> for Bytes {
    fn weight(&self, _: &usize, upload: &Upload) -> u64 {
        upload.bytes
    }
}

impl Gpu {
    /// The process's GPU, which the canvas and the window draw with: a new
    /// one once the last was lost, so what a holder made on the last, it
    /// makes again on this one.
    pub fn get() -> Result<Arc<Self>> {
        let mut current = GPU.lock();
        if let Some(gpu) = current.as_ref().filter(|gpu| !gpu.lost()) {
            return Ok(Arc::clone(gpu));
        }
        let gpu = Arc::new(Self::new().context("the GPU didn't start")?);
        *current = Some(Arc::clone(&gpu));
        Ok(gpu)
    }

    /// Whether wgpu reported the device lost, after which nothing draws on
    /// it.
    pub fn lost(&self) -> bool {
        self.lost.load(Ordering::Acquire)
    }

    /// Creates the device with the adapter's texture and buffer limits, rather
    /// than `Limits::default()`, which caps a texture at 8192 pixels whatever
    /// the GPU supports.
    fn new() -> Result<Self> {
        // An instance of every backend sets GL up for each window's surface
        // too, whichever adapter draws, and on Linux EGL fails there against
        // the context GTK or CEF holds, which wgpu-hal panics on
        // (koharu-rs/koharu#1414). So GL is only for a machine with no
        // Vulkan, Metal, or DX12 adapter; `WGPU_BACKEND` still decides.
        let (instance, adapter) = match wgpu::Backends::from_env() {
            Some(backends) => open(backends),
            None => open(wgpu::Backends::PRIMARY).or_else(|_| open(wgpu::Backends::GL)),
        }
        .context("no GPU adapter is available")?;
        let supported = adapter.limits();
        let required_limits = wgpu::Limits {
            max_texture_dimension_2d: supported.max_texture_dimension_2d,
            max_buffer_size: supported.max_buffer_size,
            ..wgpu::Limits::default()
        };
        // External memory lets the window import the frames CEF shares.
        let optional = wgpu::Features::CLEAR_TEXTURE
            | wgpu::Features::PIPELINE_CACHE
            | wgpu::Features::VULKAN_EXTERNAL_MEMORY_WIN32;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("koharu renderer"),
            required_features: adapter.features() & optional,
            required_limits,
            ..Default::default()
        }))
        .context("no GPU adapter supports vello_gpu's features")?;
        // wgpu panics on an error no scope catches, and Koharu's panic hook
        // closes the app, so a GPU short of memory, or lost, ended Koharu in
        // the middle of a thumbnail. Reads fail through `checked` instead,
        // and the rest is logged.
        device.on_uncaptured_error(Arc::new(|error| tracing::error!("GPU error: {error}")));
        // wgpu calls it as it loses the device, before the call that lost it
        // returns.
        let lost = Arc::new(AtomicBool::new(false));
        device.set_device_lost_callback({
            let lost = Arc::clone(&lost);
            move |reason, message| {
                lost.store(true, Ordering::Release);
                tracing::error!("the GPU device was lost ({reason:?}): {message}");
            }
        });
        // A render takes its size from its target; this only sizes the
        // renderer's first buffers.
        let (renderer, resources) = Renderer::new(
            &device,
            &queue,
            &RenderTargetConfig {
                format: TextureFormat::Rgba8Unorm,
                width: 1,
                height: 1,
            },
        );
        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            lost,
            vello: Mutex::new(State {
                renderer,
                resources,
                textures: quick_cache::unsync::Cache::with_weighter(64, TEXTURE_BYTES, Bytes),
                next_texture: 0,
            }),
        })
    }

    /// The longest side a texture may have on this GPU, and so a drawing's.
    pub(crate) fn largest_texture(&self) -> u32 {
        self.device
            .limits()
            .max_texture_dimension_2d
            .min(u32::from(u16::MAX))
    }

    /// A drawing for a `width`×`height` target. It holds the renderer until
    /// it's rendered.
    pub fn drawing(&self, width: u16, height: u16) -> Drawing<'_> {
        let mut vello = self.vello.lock();
        // A texture whose pixmap has gone, as an evicted layer's, goes too.
        vello
            .textures
            .retain(|_, upload| upload.pixmap.strong_count() > 0);
        Drawing {
            gpu: self,
            vello,
            scene: Scene::new(width, height),
            bindings: TextureBindings::new(),
        }
    }

    /// Runs `work`, which records its GPU work and waits for it on this
    /// thread, inside error scopes, so a GPU error fails it rather than
    /// leaving what it reads back blank.
    fn checked<T>(&self, work: impl FnOnce() -> Result<T>) -> Result<T> {
        let memory = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let result = work();
        let invalid = pollster::block_on(validation.pop());
        let short = pollster::block_on(memory.pop());
        match invalid.or(short) {
            Some(error) => Err(anyhow!("the GPU failed: {error}")),
            None => result,
        }
    }

    /// `texture`, an `Rgba8Unorm` texture made with `COPY_SRC` that holds
    /// premultiplied RGBA, as vello_gpu draws it, read back as straight
    /// alpha.
    pub fn read_texture(&self, texture: &Texture) -> Result<RgbaImage> {
        self.checked(|| {
            let (width, height) = (texture.width(), texture.height());
            let padded_width = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
                * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            let readback = self.device.create_buffer(&BufferDescriptor {
                label: Some("koharu readback buffer"),
                size: u64::from(padded_width) * u64::from(height),
                usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mut encoder = self
                .device
                .create_command_encoder(&CommandEncoderDescriptor {
                    label: Some("koharu readback"),
                });
            encoder.copy_texture_to_buffer(
                texture.as_image_copy(),
                TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded_width),
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
            let slice = readback.slice(..);
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
                .chunks_exact(padded_width as usize)
                .take(height as usize)
            {
                pixels.extend_from_slice(&line[..row]);
            }
            drop(mapped);
            readback.unmap();
            for pixel in pixels.as_chunks_mut::<4>().0 {
                let alpha = u32::from(pixel[3]);
                if alpha != 0 && alpha != 255 {
                    for channel in &mut pixel[..3] {
                        *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
                    }
                }
            }
            RgbaImage::from_raw(width, height, pixels).context("the readback has the wrong size")
        })
    }
}

/// An instance of `backends` and the adapter `WGPU_ADAPTER_NAME`, or else
/// wgpu's default preference, picks among them.
fn open(backends: wgpu::Backends) -> Result<(wgpu::Instance, wgpu::Adapter)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        display: None,
        backends,
        flags: wgpu::InstanceFlags::from_build_config().with_env(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        backend_options: wgpu::BackendOptions::from_env_or_default(),
    });
    let adapter = pollster::block_on(wgpu::util::initialize_adapter_from_env_or_default(
        &instance, None,
    ))?;
    Ok((instance, adapter))
}

/// A scene for one render on the GPU, with the textures its images name.
pub struct Drawing<'a> {
    pub(crate) gpu: &'a Gpu,
    vello: MutexGuard<'a, State>,
    /// What the canvas draws over the page goes here too.
    pub scene: Scene,
    bindings: TextureBindings,
}

impl Drawing<'_> {
    /// Renders into a texture the drawing's size and reads it back as
    /// straight-alpha RGBA.
    pub(crate) fn read(self) -> Result<RgbaImage> {
        let gpu = self.gpu;
        gpu.checked(move || {
            let texture = gpu.device.create_texture(&TextureDescriptor {
                label: Some("koharu raster"),
                size: Extent3d {
                    width: u32::from(self.scene.width()),
                    height: u32::from(self.scene.height()),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: TextureFormat::Rgba8Unorm,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            self.render(&texture.create_view(&wgpu::TextureViewDescriptor::default()))?;
            gpu.read_texture(&texture)
        })
    }

    /// Renders into `target`, an `Rgba8Unorm` render attachment the drawing's
    /// size, as premultiplied RGBA.
    pub fn render(mut self, target: &TextureView) -> Result<()> {
        // vello_gpu uploads through wgpu's `create_buffer_init`, which panics
        // on a lost device's buffers.
        if self.gpu.lost() {
            bail!("the GPU device was lost");
        }
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("koharu render"),
            });
        let State {
            renderer,
            resources,
            ..
        } = &mut *self.vello;
        renderer
            .render(
                &self.scene,
                resources,
                &mut encoder,
                RenderTarget {
                    view: target,
                    depth_view: None,
                },
                &self.bindings,
                RenderOptions::default(),
            )
            .map_err(|error| anyhow!("the GPU couldn't render: {error}"))?;
        self.gpu.queue.submit([encoder.finish()]);
        Ok(())
    }

    /// Fills `glyphs` of `font` at `size` with the current paint and
    /// transform, each turned by `turn`, and strokes them too with `stroke`,
    /// whose width is in the run's units, when there is one.
    pub(crate) fn glyphs(
        &mut self,
        font: &Font,
        size: f32,
        turn: Option<Affine>,
        stroke: Option<Stroke>,
        glyphs: &[Glyph],
    ) -> Result<()> {
        let passes: &[bool] = match stroke {
            Some(stroke) => {
                self.scene.set_stroke(stroke);
                &[false, true]
            }
            None => &[false],
        };
        for &stroking in passes {
            let mut run = self
                .scene
                .glyph_run(&mut self.vello.resources, &font.data)
                .font_size(size)
                .normalized_coords(&font.coords)
                .hint(false);
            if let Some(turn) = turn {
                run = run.glyph_transform(turn);
            }
            let glyphs = glyphs.iter().copied();
            match stroking {
                true => run.stroke_glyphs(glyphs),
                false => run.fill_glyphs(glyphs),
            }
            .map_err(|error| anyhow!("glyphs couldn't be drawn: {error:?}"))?;
        }
        Ok(())
    }

    /// `pixmap` as an image the scene samples: uploaded as a texture of its
    /// own unless one is kept, and named by its id. The drawing holds the
    /// texture until it renders, kept or not.
    pub(crate) fn image(&mut self, pixmap: &Arc<Pixmap>) -> ImageSource {
        let address = Arc::as_ptr(pixmap) as usize;
        let uploaded = self
            .vello
            .textures
            .get(&address)
            .filter(|upload| upload.pixmap.as_ptr() == Arc::as_ptr(pixmap))
            .map(|upload| (upload.id, upload.view.clone()));
        let (id, view) = uploaded.unwrap_or_else(|| {
            let texture = self.gpu.device.create_texture_with_data(
                &self.gpu.queue,
                &TextureDescriptor {
                    label: Some("koharu image"),
                    size: Extent3d {
                        width: u32::from(pixmap.width()),
                        height: u32::from(pixmap.height()),
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: TextureFormat::Rgba8Unorm,
                    usage: TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                pixmap.data_as_u8_slice(),
            );
            let id = TextureId(self.vello.next_texture);
            self.vello.next_texture += 1;
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.vello.textures.insert(
                address,
                Upload {
                    pixmap: Arc::downgrade(pixmap),
                    id,
                    view: view.clone(),
                    bytes: pixmap.data_as_u8_slice().len() as u64,
                },
            );
            (id, view)
        });
        self.bindings.insert(id, view);
        ImageSource::external_texture(id, TextureRegion::Full, true)
    }
}

#[cfg(test)]
mod tests {
    use super::Gpu;

    #[test]
    fn a_read_the_gpu_rejects_fails_rather_than_panicking() {
        let Ok(gpu) = Gpu::get() else {
            return;
        };
        let limit = gpu.device.limits().max_texture_dimension_2d;
        let Ok(width) = u16::try_from(limit + 1) else {
            return;
        };
        assert!(gpu.drawing(width, 1).read().is_err());
    }
}
