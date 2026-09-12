//! Reusable native WGPU readback and export supersampling.

use std::sync::mpsc;

use anyhow::{Context, Result as AnyResult, anyhow, bail};
use fast_image_resize::{FilterType, ResizeAlg, ResizeOptions, Resizer};
use image::RgbaImage;
use parking_lot::Mutex;
use vello::{
    AaConfig, AaSupport, RenderParams, RendererOptions, Scene,
    kurbo::Affine,
    peniko::Color,
    wgpu::{
        self, Buffer, BufferDescriptor, BufferUsages, CommandEncoderDescriptor, Extent3d,
        TexelCopyBufferInfo, Texture, TextureDescriptor, TextureFormat, TextureUsages, TextureView,
    },
};

use crate::{CompositionCommand, Error, Frame, GpuCompositor, RasterDraw, Result};

const MAX_SUPERSAMPLING_FACTOR: u32 = 4;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DownsampleFilter {
    Nearest,
    Triangle,
    CatmullRom,
    Gaussian,
    #[default]
    Lanczos3,
}

impl From<DownsampleFilter> for ResizeAlg {
    fn from(value: DownsampleFilter) -> Self {
        match value {
            DownsampleFilter::Nearest => ResizeAlg::Nearest,
            DownsampleFilter::Triangle => ResizeAlg::Convolution(FilterType::Bilinear),
            DownsampleFilter::CatmullRom => ResizeAlg::Convolution(FilterType::CatmullRom),
            DownsampleFilter::Gaussian => ResizeAlg::Convolution(FilterType::Gaussian),
            DownsampleFilter::Lanczos3 => ResizeAlg::Convolution(FilterType::Lanczos3),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RasterOptions {
    pub supersampling_factor: u32,
    pub downsample_filter: DownsampleFilter,
}

impl RasterOptions {
    #[must_use]
    pub fn supersampled(factor: u32) -> Self {
        Self {
            supersampling_factor: factor,
            ..Default::default()
        }
    }

    fn scale(self) -> u32 {
        self.supersampling_factor.clamp(1, MAX_SUPERSAMPLING_FACTOR)
    }
}

impl Default for RasterOptions {
    fn default() -> Self {
        Self {
            supersampling_factor: 1,
            downsample_filter: DownsampleFilter::Lanczos3,
        }
    }
}

#[derive(Debug)]
pub struct Raster {
    pub image: RgbaImage,
    pub left: i32,
    pub top: i32,
}

struct GpuState {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: vello::Renderer,
    compositor: GpuCompositor,
    targets: Vec<RenderTarget>,
}

struct RenderTarget {
    width: u32,
    height: u32,
    padded_width: u32,
    texture: Texture,
    view: TextureView,
    readback: Buffer,
}

/// Reusable headless Vello renderer with a bounded readback-target pool.
pub struct Rasterizer {
    adapter: wgpu::AdapterInfo,
    gpu: Mutex<GpuState>,
}

impl Rasterizer {
    pub fn new() -> Result<Self> {
        Self::try_new().map_err(Error::backend)
    }

    fn try_new() -> AnyResult<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::from_env().unwrap_or_default(),
            flags: wgpu::InstanceFlags::from_build_config().with_env(),
            display: None,
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            backend_options: wgpu::BackendOptions::from_env_or_default(),
        });
        let adapter = pollster::block_on(async {
            if std::env::var_os("WGPU_ADAPTER_NAME").is_some() {
                wgpu::util::initialize_adapter_from_env(&instance, None).await
            } else {
                instance
                    .request_adapter(&wgpu::RequestAdapterOptions {
                        power_preference: wgpu::PowerPreference::from_env()
                            .unwrap_or(wgpu::PowerPreference::HighPerformance),
                        ..Default::default()
                    })
                    .await
            }
        })
        .context("no WGPU adapter supports Vello's required features")?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: adapter.features()
                & (wgpu::Features::CLEAR_TEXTURE | wgpu::Features::PIPELINE_CACHE),
            ..Default::default()
        }))
        .context("failed to create the export GPU device")?;
        let renderer = vello::Renderer::new(
            &device,
            RendererOptions {
                antialiasing_support: AaSupport::area_only(),
                ..Default::default()
            },
        )
        .map_err(|error| anyhow!("failed to create Vello renderer: {error:?}"))?;
        let compositor = GpuCompositor::new(&device);
        Ok(Self {
            adapter: adapter.get_info(),
            gpu: Mutex::new(GpuState {
                device,
                queue,
                renderer,
                compositor,
                targets: Vec::new(),
            }),
        })
    }

    pub fn adapter_info(&self) -> &wgpu::AdapterInfo {
        &self.adapter
    }

    pub fn rasterize(&self, frame: &Frame, options: RasterOptions) -> Result<Raster> {
        let (width, height) = frame.size();
        let (left, top) = frame.origin();
        let image = self
            .rasterize_frame_inner(frame, width, height, options)
            .map_err(Error::backend)?;
        Ok(Raster { image, left, top })
    }

    fn rasterize_frame_inner(
        &self,
        frame: &Frame,
        width: u32,
        height: u32,
        options: RasterOptions,
    ) -> AnyResult<RgbaImage> {
        checked_surface(width, height)?;
        let scale = options.scale();
        let raster_width = width
            .checked_mul(scale)
            .context("supersampled render surface width overflow")?;
        let raster_height = height
            .checked_mul(scale)
            .context("supersampled render surface height overflow")?;
        let commands = frame.composition_commands(scale);
        let pixels = self.readback_commands(&commands, raster_width, raster_height)?;
        finish_raster(pixels, raster_width, raster_height, width, height, options)
    }

    pub fn rasterize_commands(
        &self,
        commands: &[CompositionCommand],
        width: u32,
        height: u32,
        options: RasterOptions,
    ) -> Result<RgbaImage> {
        self.rasterize_commands_inner(commands, width, height, options)
            .map_err(Error::backend)
    }

    fn rasterize_commands_inner(
        &self,
        commands: &[CompositionCommand],
        width: u32,
        height: u32,
        options: RasterOptions,
    ) -> AnyResult<RgbaImage> {
        checked_surface(width, height)?;
        let scale = options.scale();
        let raster_width = width
            .checked_mul(scale)
            .context("supersampled render surface width overflow")?;
        let raster_height = height
            .checked_mul(scale)
            .context("supersampled render surface height overflow")?;
        let scaled;
        let commands = if scale == 1 {
            commands
        } else {
            let transform = Affine::scale(f64::from(scale));
            scaled = commands
                .iter()
                .map(|command| match command {
                    CompositionCommand::Raster(draw) => CompositionCommand::Raster(RasterDraw {
                        image: draw.image.clone(),
                        transform: transform * draw.transform,
                        opacity: draw.opacity,
                        erase: draw.erase,
                    }),
                    CompositionCommand::Vector(scene) => {
                        let mut scaled = Scene::new();
                        scaled.append(scene, Some(transform));
                        CompositionCommand::Vector(Box::new(scaled))
                    }
                })
                .collect::<Vec<_>>();
            &scaled
        };
        let pixels = self.readback_commands(commands, raster_width, raster_height)?;
        finish_raster(pixels, raster_width, raster_height, width, height, options)
    }

    pub fn rasterize_scene(
        &self,
        scene: &Scene,
        width: u32,
        height: u32,
        background: [u8; 4],
        options: RasterOptions,
    ) -> Result<RgbaImage> {
        self.rasterize_scene_inner(scene, width, height, background, options)
            .map_err(Error::backend)
    }

    fn rasterize_scene_inner(
        &self,
        scene: &Scene,
        width: u32,
        height: u32,
        background: [u8; 4],
        options: RasterOptions,
    ) -> AnyResult<RgbaImage> {
        checked_surface(width, height)?;
        let scale = options.scale();
        let raster_width = width
            .checked_mul(scale)
            .context("supersampled render surface width overflow")?;
        let raster_height = height
            .checked_mul(scale)
            .context("supersampled render surface height overflow")?;
        let scaled;
        let scene = if scale == 1 {
            scene
        } else {
            scaled = {
                let mut scaled = Scene::new();
                scaled.append(scene, Some(Affine::scale(f64::from(scale))));
                scaled
            };
            &scaled
        };
        let pixels = self.readback_scene(scene, raster_width, raster_height, background)?;
        finish_raster(pixels, raster_width, raster_height, width, height, options)
    }

    fn readback_commands(
        &self,
        commands: &[CompositionCommand],
        width: u32,
        height: u32,
    ) -> AnyResult<Vec<u8>> {
        let size = Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let (device, submission, target) = {
            let mut gpu = self.gpu.lock();
            let GpuState {
                device,
                queue,
                renderer,
                compositor,
                targets,
            } = &mut *gpu;
            let device = device.clone();
            check_device_limit(&device, width, height)?;
            let target = take_target(targets, &device, width, height)?;
            compositor
                .render(
                    &device,
                    queue,
                    renderer,
                    &target.view,
                    (width, height),
                    commands,
                    None,
                    [0, 0, 0, 0],
                    [0, 0, width, height],
                )
                .map_err(|error| anyhow!(error.to_string()))?;
            let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
                label: Some("koharu frame readback encoder"),
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
                size,
            );
            let submission = queue.submit([encoder.finish()]);
            (device, submission, target)
        };
        self.finish_readback(device, submission, target)
    }

    fn readback_scene(
        &self,
        scene: &Scene,
        width: u32,
        height: u32,
        background: [u8; 4],
    ) -> AnyResult<Vec<u8>> {
        let size = Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let (device, submission, target) = {
            let mut gpu = self.gpu.lock();
            let GpuState {
                device,
                queue,
                renderer,
                compositor: _,
                targets,
            } = &mut *gpu;
            let device = device.clone();
            check_device_limit(&device, width, height)?;
            let target = take_target(targets, &device, width, height)?;
            renderer
                .render_to_texture(
                    &device,
                    queue,
                    scene,
                    &target.view,
                    &RenderParams {
                        base_color: rgba(background),
                        width,
                        height,
                        antialiasing_method: AaConfig::Area,
                    },
                )
                .map_err(|error| anyhow!("Vello rendering failed: {error:?}"))?;
            let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
                label: Some("koharu scene readback encoder"),
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
                size,
            );
            let submission = queue.submit([encoder.finish()]);
            (device, submission, target)
        };
        self.finish_readback(device, submission, target)
    }

    fn finish_readback(
        &self,
        device: wgpu::Device,
        submission: wgpu::SubmissionIndex,
        target: RenderTarget,
    ) -> AnyResult<Vec<u8>> {
        let width = target.width;
        let height = target.height;
        let slice = target.readback.slice(..);
        let (sender, receiver) = mpsc::sync_channel(1);
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .map_err(|error| anyhow!("WGPU device polling failed: {error:?}"))?;
        receiver
            .recv()
            .context("WGPU closed the readback channel")?
            .context("failed to map WGPU readback buffer")?;
        let mapped = slice.get_mapped_range();
        let row_len = (width * 4) as usize;
        let mut pixels = Vec::with_capacity(row_len * height as usize);
        for row in mapped
            .chunks_exact(target.padded_width as usize)
            .take(height as usize)
        {
            pixels.extend_from_slice(&row[..row_len]);
        }
        drop(mapped);
        target.readback.unmap();
        let mut gpu = self.gpu.lock();
        if gpu.targets.len() < 4 {
            gpu.targets.push(target);
        }
        Ok(pixels)
    }
}

fn checked_surface(width: u32, height: u32) -> AnyResult<()> {
    if width == 0 || height == 0 {
        bail!("invalid render surface {width}x{height}");
    }
    Ok(())
}

fn check_device_limit(device: &wgpu::Device, width: u32, height: u32) -> AnyResult<()> {
    let limit = device.limits().max_texture_dimension_2d;
    if width > limit || height > limit {
        bail!("render surface {width}x{height} exceeds the device limit {limit}");
    }
    Ok(())
}

fn take_target(
    targets: &mut Vec<RenderTarget>,
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> AnyResult<RenderTarget> {
    targets
        .iter()
        .position(|target| target.width == width && target.height == height)
        .map(|position| targets.swap_remove(position))
        .map_or_else(|| RenderTarget::new(device, width, height), Ok)
}

impl RenderTarget {
    fn new(device: &wgpu::Device, width: u32, height: u32) -> AnyResult<Self> {
        let size = Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("koharu rasterizer target"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::STORAGE_BINDING
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let row_bytes = width
            .checked_mul(4)
            .context("render target row size overflow")?;
        let padded_width = row_bytes.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer_size = u64::from(padded_width)
            .checked_mul(u64::from(height))
            .context("render target buffer size overflow")?;
        let readback = device.create_buffer(&BufferDescriptor {
            label: Some("koharu rasterizer readback"),
            size: buffer_size,
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            width,
            height,
            padded_width,
            texture,
            view,
            readback,
        })
    }
}

fn finish_raster(
    pixels: Vec<u8>,
    raster_width: u32,
    raster_height: u32,
    width: u32,
    height: u32,
    options: RasterOptions,
) -> AnyResult<RgbaImage> {
    let image = RgbaImage::from_raw(raster_width, raster_height, pixels)
        .context("WGPU returned an invalid RGBA buffer")?;
    if options.scale() == 1 {
        return Ok(image);
    }
    let mut downsampled = RgbaImage::new(width, height);
    let resize_options = ResizeOptions::new()
        .resize_alg(options.downsample_filter.into())
        .use_alpha(true);
    Resizer::new()
        .resize(&image, &mut downsampled, &resize_options)
        .context("failed to downsample WGPU render")?;
    Ok(downsampled)
}

fn rgba([r, g, b, a]: [u8; 4]) -> Color {
    Color::from_rgba8(r, g, b, a)
}
