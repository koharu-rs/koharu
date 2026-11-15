//! CEF's frames as shared GPU textures, where the `cef` crate can import them
//! into Koharu's device: CEF's software path reads each frame back through a
//! copy request whose delivery jitters by up to a frame, which scrolling shows,
//! while a shared texture arrives as soon as Chromium draws it.
//!
//! A shared texture is valid only during CEF's callback, and wgpu imports it
//! for sampling alone, so each frame is drawn into Koharu's own texture there,
//! with the page's transparency mask, which stands in for the bytes the
//! software path routes the pointer by.

use anyhow::{Context as _, Result, bail};
use cef::{AcceleratedPaintInfo, osr_texture_import::SharedTextureHandle};
use koharu_renderer::Gpu;

use super::paint::Mask;

/// The mask's texels per page pixel, each way.
const MASK_SCALE: u32 = 4;

/// Copies a texture pixel for pixel, and draws its transparency mask: each
/// texel the largest alpha of its block of the page, so a block counts as
/// painted when any of its pixels is.
const SHADER: &str = r"
@group(0) @binding(0) var source: texture_2d<f32>;

// One triangle over the whole target.
@vertex
fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
}

@fragment
fn blit(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    return textureLoad(source, vec2<i32>(at.xy), 0);
}

@fragment
fn mask(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let last = vec2<i32>(textureDimensions(source)) - 1;
    let origin = vec2<i32>(at.xy) * 4;
    var alpha = 0.0;
    for (var y = 0; y < 4; y++) {
        for (var x = 0; x < 4; x++) {
            alpha = max(alpha, textureLoad(source, min(origin + vec2<i32>(x, y), last), 0).a);
        }
    }
    return vec4<f32>(alpha, 0.0, 0.0, 1.0);
}
";

/// Whether CEF can hand its frames over as shared textures: on the backends
/// the `cef` crate imports them on, Vulkan only with the device's external
/// memory, and on Windows only once Chromium's GPU process is pinned to
/// Koharu's GPU, since a shared texture opens only on the adapter that made
/// it. `Some` holds the value of Chromium's `use-adapter-luid` switch on
/// Windows, and nothing elsewhere.
pub fn sharing(gpu: &Gpu) -> Option<Option<String>> {
    let adapter = gpu.adapter.get_info();
    let features = gpu.device.features();
    match adapter.backend {
        #[cfg(windows)]
        wgpu::Backend::Dx12 => pin(&adapter),
        #[cfg(windows)]
        wgpu::Backend::Vulkan
            if features.contains(wgpu::Features::VULKAN_EXTERNAL_MEMORY_WIN32) =>
        {
            pin(&adapter)
        }
        #[cfg(target_os = "linux")]
        wgpu::Backend::Vulkan
            if features.contains(wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF) =>
        {
            Some(None)
        }
        #[cfg(target_os = "macos")]
        wgpu::Backend::Metal => Some(None),
        _ => None,
    }
}

/// Chromium's `use-adapter-luid` value for `adapter`, as Chromium parses it:
/// `<HighPart>,<LowPart>`, in decimal.
#[cfg(windows)]
fn pin(adapter: &wgpu::AdapterInfo) -> Option<Option<String>> {
    let (high, low) = luid(adapter)?;
    Some(Some(format!("{high},{low}")))
}

/// The LUID of the DXGI adapter that is `adapter`'s GPU, found by its vendor
/// and device IDs, since wgpu names none, whichever backend it runs on.
#[cfg(windows)]
fn luid(adapter: &wgpu::AdapterInfo) -> Option<(i32, u32)> {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};

    // SAFETY: plain DXGI enumeration, holding no pointers past each call.
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.ok()?;
    (0..)
        .map_while(|index| unsafe { factory.EnumAdapters1(index) }.ok())
        .filter_map(|found| unsafe { found.GetDesc1() }.ok())
        .find(|desc| desc.VendorId == adapter.vendor && desc.DeviceId == adapter.device)
        .map(|desc| (desc.AdapterLuid.HighPart, desc.AdapterLuid.LowPart))
}

/// Takes CEF's shared textures into Koharu's own: the pipelines, and the
/// mask's targets, made at their first use and again at a size change.
pub struct Importer {
    layout: wgpu::BindGroupLayout,
    module: wgpu::ShaderModule,
    pipeline_layout: wgpu::PipelineLayout,
    blit: Option<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    mask: wgpu::RenderPipeline,
    /// The mask's texture and the buffer it's read back through.
    targets: Option<(wgpu::Texture, wgpu::Buffer)>,
}

impl Importer {
    pub fn new(gpu: &Gpu) -> Self {
        let device = &gpu.device;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("koharu shared frame"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("koharu shared frame"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("koharu shared frame"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let mask = pipeline(
            device,
            &pipeline_layout,
            &module,
            "mask",
            wgpu::TextureFormat::R8Unorm,
        );
        Self {
            layout,
            module,
            pipeline_layout,
            blit: None,
            mask,
            targets: None,
        }
    }

    /// Draws the frame `info` shares into `frame`, made again at a size or
    /// format change, and for the page its transparency into `mask`, and
    /// waits for the GPU, so CEF can reuse its texture once this returns.
    /// Fails when the texture couldn't be imported.
    pub fn take(
        &mut self,
        gpu: &Gpu,
        info: &AcceleratedPaintInfo,
        frame: &mut Option<wgpu::Texture>,
        mask: Option<&mut Option<Mask>>,
    ) -> Result<()> {
        let shared = SharedTextureHandle::new(info)
            .import_texture(&gpu.device)
            .map_err(|error| anyhow::anyhow!("{error:?}"))
            .context("CEF's shared texture couldn't be imported")?;
        // The `cef` crate answers a failed import with an empty texture it
        // made itself, the only one of its textures that takes copies.
        if shared.usage().contains(wgpu::TextureUsages::COPY_DST) {
            bail!("CEF's shared texture couldn't be imported on this GPU");
        }
        let (width, height, format) = (shared.width(), shared.height(), shared.format());
        let device = &gpu.device;
        let target = match frame.take() {
            Some(kept)
                if (kept.width(), kept.height(), kept.format()) == (width, height, format) =>
            {
                kept
            }
            _ => device.create_texture(&wgpu::TextureDescriptor {
                label: Some("koharu shared frame"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            }),
        };
        if self.blit.as_ref().is_none_or(|(made, _)| *made != format) {
            let blit = pipeline(device, &self.pipeline_layout, &self.module, "blit", format);
            self.blit = Some((format, blit));
        }

        let source = shared.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("koharu shared frame"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&source),
            }],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("koharu shared frame"),
        });
        let (_, blit) = self.blit.as_ref().expect("the blit pipeline was just made");
        draw(
            &mut encoder,
            blit,
            &bind_group,
            &target.create_view(&Default::default()),
        );
        let mask_size = (width.div_ceil(MASK_SCALE), height.div_ceil(MASK_SCALE));
        let padded = mask_size
            .0
            .next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        if mask.is_some() {
            self.size_targets(gpu, mask_size, padded);
            let (texture, buffer) = self.targets.as_ref().expect("the targets were just sized");
            draw(
                &mut encoder,
                &self.mask,
                &bind_group,
                &texture.create_view(&Default::default()),
            );
            encoder.copy_texture_to_buffer(
                texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded),
                        rows_per_image: Some(mask_size.1),
                    },
                },
                texture.size(),
            );
        }
        let index = gpu.queue.submit([encoder.finish()]);
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(index),
                timeout: None,
            })
            .context("the GPU didn't finish drawing CEF's frame")?;
        *frame = Some(target);

        if let Some(mask) = mask {
            let (_, buffer) = self.targets.as_ref().expect("the mask was just drawn");
            *mask = Some(read_mask(gpu, buffer, mask_size, padded)?);
        }
        Ok(())
    }

    /// Makes the mask's texture and readback buffer for a mask of `size`,
    /// whose rows are `padded` bytes in the buffer, unless they're that size.
    fn size_targets(&mut self, gpu: &Gpu, size: (u32, u32), padded: u32) {
        let sized = self
            .targets
            .as_ref()
            .is_some_and(|(texture, _)| (texture.width(), texture.height()) == size);
        if !sized {
            let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("koharu shared frame mask"),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("koharu shared frame mask"),
                size: u64::from(padded) * u64::from(size.1),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            self.targets = Some((texture, buffer));
        }
    }
}

fn pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    fragment: &str,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("koharu shared frame"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vertex"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fragment),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// One pass of `pipeline` over all of `target`.
fn draw(
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &wgpu::RenderPipeline,
    bind_group: &wgpu::BindGroup,
    target: &wgpu::TextureView,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("koharu shared frame"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bind_group, &[]);
    pass.draw(0..3, 0..1);
}

/// The mask the GPU just wrote to `buffer`, rows `padded` bytes apart.
fn read_mask(
    gpu: &Gpu,
    buffer: &wgpu::Buffer,
    (width, height): (u32, u32),
    padded: u32,
) -> Result<Mask> {
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    gpu.device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .context("the GPU didn't map the mask")?;
    let bytes = {
        let mapped = slice
            .get_mapped_range()
            .map_err(|error| anyhow::anyhow!("{error:?}"))
            .context("the mask's buffer didn't map")?;
        mapped
            .chunks(padded as usize)
            .flat_map(|row| &row[..width as usize])
            .copied()
            .collect()
    };
    buffer.unmap();
    Ok(Mask {
        width,
        height,
        scale: MASK_SCALE,
        bytes,
    })
}
