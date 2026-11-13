//! The window's frame, drawn with the renderer's device on the window's
//! surface: the canvas's texture in its viewport, then CEF's paint over it,
//! then CEF's popup. CEF's bytes and the canvas's are already sRGB-encoded,
//! so the surface format isn't an sRGB one and nothing converts them.

use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use image::RgbaImage;
use koharu_renderer::Gpu;
use tokio::sync::oneshot;
use vello::{kurbo::Rect, wgpu};
use winit::window::Window;

use super::paint::{Paint, Painted};

/// Draws a texture into a rectangle given in normalized device coordinates,
/// as a four-vertex strip.
const SHADER: &str = r"
struct Placement {
    // left, top, right, bottom
    rect: vec4<f32>,
}

@group(0) @binding(0) var<uniform> placement: Placement;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var pixels: sampler;

struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> Vertex {
    let uv = vec2<f32>(f32(index & 1u), f32(index >> 1u));
    let rect = placement.rect;
    let position = vec2<f32>(mix(rect.x, rect.z, uv.x), mix(rect.y, rect.w, uv.y));
    return Vertex(vec4<f32>(position, 0.0, 1.0), uv);
}

@fragment
fn fragment(vertex: Vertex) -> @location(0) vec4<f32> {
    return textureSample(source, pixels, vertex.uv);
}
";

pub struct Compositor {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// The canvas, which covers its viewport.
    opaque: wgpu::RenderPipeline,
    /// CEF's premultiplied paints.
    blended: wgpu::RenderPipeline,
    canvas: Layer,
    view: Layer,
    popup: Layer,
}

/// One textured rectangle of the frame.
struct Layer {
    placement: wgpu::Buffer,
    source: Option<Source>,
    /// Where it goes this frame, in window pixels; `None` leaves it out.
    rect: Option<Rect>,
}

struct Source {
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

impl Compositor {
    pub fn new(gpu: &Gpu, window: Arc<dyn Window>) -> Result<Self> {
        let surface = gpu
            .instance
            .create_surface(window)
            .context("failed to create the window's surface")?;
        let capabilities = surface.get_capabilities(&gpu.adapter);
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(|format| !format.is_srgb())
            .context("the window's surface has no format without sRGB encoding")?;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: 0,
            height: 0,
            desired_maximum_frame_latency: 2,
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: Vec::new(),
        };

        let device = &gpu.device;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("koharu compositor"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        // Every layer is drawn at its own pixel size, so the nearest pixel
        // is the exact one.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("koharu compositor"),
            ..Default::default()
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("koharu compositor"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("koharu compositor"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |blend: wgpu::BlendState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("koharu compositor"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vertex"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fragment"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let opaque = pipeline(wgpu::BlendState::REPLACE);
        let blended = pipeline(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        let layer = || Layer {
            placement: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("koharu compositor placement"),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            source: None,
            rect: None,
        };
        Ok(Self {
            surface,
            config,
            layout,
            sampler,
            opaque,
            blended,
            canvas: layer(),
            view: layer(),
            popup: layer(),
        })
    }

    /// Draws a frame of the window's `size`, from CEF's latest paints and
    /// the canvas's texture in its viewport, and answers `capture` with it.
    pub fn present(
        &mut self,
        gpu: &Gpu,
        size: (u32, u32),
        painted: &mut Painted,
        canvas: Option<(&wgpu::Texture, Rect)>,
        capture: Option<oneshot::Sender<RgbaImage>>,
    ) -> Result<()> {
        if (self.config.width, self.config.height) != size {
            (self.config.width, self.config.height) = size;
            self.surface.configure(&gpu.device, &self.config);
        }
        self.view.rect = self.view.upload(
            gpu,
            &self.layout,
            &self.sampler,
            painted.view.as_mut(),
            (0, 0),
        );
        self.popup.rect = match painted.popup_origin {
            Some(origin) => self.popup.upload(
                gpu,
                &self.layout,
                &self.sampler,
                painted.popup.as_mut(),
                origin,
            ),
            None => None,
        };
        self.canvas.rect = None;
        if let Some((texture, viewport)) = canvas {
            self.canvas.show(gpu, &self.layout, &self.sampler, texture);
            self.canvas.rect = Some(viewport);
        }
        for layer in [&self.canvas, &self.view, &self.popup] {
            layer.place(gpu, size);
        }

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            // Nothing to draw on now; the next redraw tries again.
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&gpu.device, &self.config);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                bail!("the window's surface failed validation")
            }
        };
        let target = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.draw(gpu, &target);
        gpu.queue.present(frame);

        if let Some(capture) = capture {
            let _ = capture.send(self.capture(gpu, size)?);
        }
        Ok(())
    }

    fn draw(&self, gpu: &Gpu, target: &wgpu::TextureView) {
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("koharu compositor"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("koharu compositor"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            for (layer, pipeline) in [
                (&self.canvas, &self.opaque),
                (&self.view, &self.blended),
                (&self.popup, &self.blended),
            ] {
                if let (Some(source), Some(_)) = (&layer.source, layer.rect) {
                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(0, &source.bind_group, &[]);
                    pass.draw(0..4, 0..1);
                }
            }
        }
        gpu.queue.submit([encoder.finish()]);
    }

    /// The frame again, drawn into a texture and read back as RGBA.
    fn capture(&self, gpu: &Gpu, (width, height): (u32, u32)) -> Result<RgbaImage> {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("koharu capture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        self.draw(
            gpu,
            &texture.create_view(&wgpu::TextureViewDescriptor::default()),
        );
        let mut image = gpu.read_texture(&texture)?;
        match self.config.format {
            wgpu::TextureFormat::Rgba8Unorm => {}
            wgpu::TextureFormat::Bgra8Unorm => {
                for pixel in image.pixels_mut() {
                    pixel.0.swap(0, 2);
                }
            }
            format => bail!("can't read a {format:?} frame back"),
        }
        Ok(image)
    }
}

impl Layer {
    /// Uploads what changed of `paint`, drawn at `origin`, and answers where
    /// it goes; a new texture takes all of it.
    fn upload(
        &mut self,
        gpu: &Gpu,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        paint: Option<&mut Paint>,
        origin: (i32, i32),
    ) -> Option<Rect> {
        let paint = paint?;
        let sized = self.source.as_ref().is_some_and(|source| {
            (source.texture.width(), source.texture.height()) == (paint.width, paint.height)
        });
        if !sized {
            let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("koharu web paint"),
                size: wgpu::Extent3d {
                    width: paint.width,
                    height: paint.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Bgra8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            self.source = Some(self.source(gpu, layout, sampler, texture));
            paint.dirty.clear();
            paint.dirty.push(super::paint::PixelRect {
                x: 0,
                y: 0,
                width: paint.width,
                height: paint.height,
            });
        }
        let source = self.source.as_ref()?;
        let stride = paint.width * 4;
        for rect in paint.dirty.drain(..) {
            let start = (rect.y * stride + rect.x * 4) as usize;
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &source.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: rect.x,
                        y: rect.y,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &paint.bytes[start..],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width: rect.width,
                    height: rect.height,
                    depth_or_array_layers: 1,
                },
            );
        }
        let (x, y) = (f64::from(origin.0), f64::from(origin.1));
        Some(Rect::new(
            x,
            y,
            x + f64::from(paint.width),
            y + f64::from(paint.height),
        ))
    }

    /// Samples `texture`, which the canvas owns and replaces when its
    /// viewport's size changes.
    fn show(
        &mut self,
        gpu: &Gpu,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        texture: &wgpu::Texture,
    ) {
        if self
            .source
            .as_ref()
            .is_none_or(|source| source.texture != *texture)
        {
            self.source = Some(self.source(gpu, layout, sampler, texture.clone()));
        }
    }

    fn source(
        &self,
        gpu: &Gpu,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        texture: wgpu::Texture,
    ) -> Source {
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("koharu compositor"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.placement.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        Source {
            texture,
            bind_group,
        }
    }

    /// Writes where the layer goes, in normalized device coordinates of a
    /// frame of `size`; the rasterizer clips what lies outside.
    fn place(&self, gpu: &Gpu, (width, height): (u32, u32)) {
        let Some(rect) = self.rect else { return };
        let (width, height) = (f64::from(width), f64::from(height));
        let ndc = [
            rect.x0 / width * 2.0 - 1.0,
            1.0 - rect.y0 / height * 2.0,
            rect.x1 / width * 2.0 - 1.0,
            1.0 - rect.y1 / height * 2.0,
        ]
        .map(|value| value as f32);
        let bytes: Vec<u8> = ndc.iter().flat_map(|value| value.to_le_bytes()).collect();
        gpu.queue.write_buffer(&self.placement, 0, &bytes);
    }
}
