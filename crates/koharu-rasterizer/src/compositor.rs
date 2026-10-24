//! Ordered GPU composition for RGBA8 images and Vello vector batches.

use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
};

use vello::{AaConfig, RenderParams, Scene, kurbo::Affine, peniko::Color, wgpu};

#[cfg(target_arch = "wasm32")]
use crate::PreparedRasterTile;
use crate::{Error, RasterImage, ResourceId, Result};

pub const DEFAULT_RASTER_CACHE_BUDGET_BYTES: u64 = 256 * 1024 * 1024;

const SHADER: &str = r#"
struct DrawUniforms {
    linear: vec4<f32>,
    translation_source: vec4<f32>,
    target_options: vec4<f32>,
    format_options: vec4<f32>,
    sampling_options: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) screen_uv: vec2<f32>,
}

@group(0) @binding(0) var source_texture: texture_2d<f32>;
@group(0) @binding(1) var erase_texture: texture_2d<f32>;
@group(0) @binding(2) var texture_sampler: sampler;
@group(1) @binding(0) var<uniform> uniforms: DrawUniforms;

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> VertexOutput {
    let coordinates = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let uv = coordinates[index];
    let source = uv * uniforms.translation_source.zw;
    let screen = vec2<f32>(
        uniforms.linear.x * source.x + uniforms.linear.z * source.y,
        uniforms.linear.y * source.x + uniforms.linear.w * source.y,
    ) + uniforms.translation_source.xy;
    let target_size = uniforms.target_options.xy;
    var output: VertexOutput;
    output.position = vec4<f32>(
        screen.x / target_size.x * 2.0 - 1.0,
        1.0 - screen.y / target_size.y * 2.0,
        0.0,
        1.0,
    );
    output.uv = (uniforms.sampling_options.xy + uv * uniforms.translation_source.zw)
        / uniforms.sampling_options.zw;
    output.screen_uv = screen / target_size;
    return output;
}

@fragment
fn fragment(input: VertexOutput) -> @location(0) vec4<f32> {
    var color: vec4<f32>;
    if uniforms.format_options.y > 0.0 {
        let dimensions = textureDimensions(source_texture);
        let coordinate = clamp(
            vec2<i32>(floor(input.uv * vec2<f32>(dimensions))),
            vec2<i32>(0),
            vec2<i32>(dimensions) - vec2<i32>(1),
        );
        color = textureLoad(source_texture, coordinate, 0);
    } else {
        color = textureSample(source_texture, texture_sampler, input.uv);
    }
    // Vello stores straight (unpremultiplied) colour; the blend state expects premultiplied.
    if uniforms.format_options.x > 0.0 {
        color = vec4<f32>(color.rgb * color.a, color.a);
    }
    var erase = 0.0;
    if uniforms.target_options.w > 0.0 {
        erase = textureSample(erase_texture, texture_sampler, input.screen_uv).a;
    }
    return color * (uniforms.target_options.z * (1.0 - erase * uniforms.target_options.w));
}
"#;

pub enum CompositionCommand {
    Raster(RasterDraw),
    Vector(Box<Scene>),
}

pub struct RasterDraw {
    pub image: RasterImage,
    pub transform: Affine,
    pub opacity: f32,
    pub erase: bool,
}

struct CachedTexture {
    source: Option<ResourceId>,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// Samples this texture without an erase mask; built on first draw and valid for the
    /// texture's lifetime because the sampler and the empty mask outlive every cached texture.
    bind_group: Option<wgpu::BindGroup>,
    byte_len: u64,
    last_used: u64,
}

struct ScratchTarget {
    size: (u32, u32),
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    bind_group: Option<wgpu::BindGroup>,
}

const UNIFORM_FLOATS: usize = 20;
const UNIFORM_BYTES: u64 = (UNIFORM_FLOATS * size_of::<f32>()) as u64;

/// One textured quad of a composition pass.
struct Draw {
    textures: wgpu::BindGroup,
    uniforms: [f32; UNIFORM_FLOATS],
}

/// Per-draw uniforms for a whole pass, bound once with a dynamic offset per draw.
struct UniformArena {
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    capacity: usize,
    stride: usize,
    bytes: Vec<u8>,
}

impl UniformArena {
    fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, capacity: usize) -> Self {
        let stride = UNIFORM_BYTES.next_multiple_of(u64::from(
            device.limits().min_uniform_buffer_offset_alignment,
        )) as usize;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("koharu raster compositor uniforms"),
            size: (capacity * stride) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("koharu raster compositor uniform bind group"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(UNIFORM_BYTES),
                }),
            }],
        });
        Self {
            buffer,
            bind_group,
            capacity,
            stride,
            bytes: Vec::new(),
        }
    }
}

impl ScratchTarget {
    fn new(device: &wgpu::Device, label: &str, size: (u32, u32)) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            size,
            _texture: texture,
            view,
            bind_group: None,
        }
    }
}

pub struct GpuCompositor {
    pipeline: wgpu::RenderPipeline,
    texture_layout: wgpu::BindGroupLayout,
    uniform_layout: wgpu::BindGroupLayout,
    uniforms: UniformArena,
    draws: Vec<Draw>,
    sampler: wgpu::Sampler,
    empty_mask: CachedTexture,
    overlay: Option<ScratchTarget>,
    erase: Option<ScratchTarget>,
    images: HashMap<ResourceId, CachedTexture>,
    image_bytes: u64,
    image_budget: u64,
    image_clock: u64,
}

impl GpuCompositor {
    #[must_use]
    pub fn new(device: &wgpu::Device) -> Self {
        Self::with_cache_budget(device, DEFAULT_RASTER_CACHE_BUDGET_BYTES)
    }

    #[must_use]
    pub fn with_cache_budget(device: &wgpu::Device, image_budget: u64) -> Self {
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("koharu raster compositor sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("koharu raster compositor texture bind group layout"),
            entries: &[
                texture_layout(0),
                texture_layout(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("koharu raster compositor uniform bind group layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(UNIFORM_BYTES),
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("koharu raster compositor pipeline layout"),
            bind_group_layouts: &[Some(&texture_layout), Some(&uniform_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("koharu raster compositor shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("koharu raster compositor pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            uniforms: UniformArena::new(device, &uniform_layout, 64),
            texture_layout,
            uniform_layout,
            draws: Vec::new(),
            sampler,
            empty_mask: create_texture(device, "koharu empty erase mask", 1, 1, None),
            overlay: None,
            erase: None,
            images: HashMap::new(),
            image_bytes: 0,
            image_budget,
            image_clock: 0,
        }
    }

    #[must_use]
    pub const fn cache_budget(&self) -> u64 {
        self.image_budget
    }

    pub fn set_cache_budget(&mut self, budget: u64) {
        self.image_budget = budget;
        self.trim_cache();
    }

    #[must_use]
    pub const fn cached_resource_bytes(&self) -> u64 {
        self.image_bytes
    }

    #[must_use]
    pub fn cached_resource_count(&self) -> usize {
        self.images.len()
    }

    #[must_use]
    pub fn is_tile_cached(&self, id: ResourceId) -> bool {
        self.images.contains_key(&id)
    }

    #[cfg(target_arch = "wasm32")]
    pub fn cache_external_raster(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        source: ResourceId,
        source_size: (u32, u32),
        image: &wgpu::ExternalImageSource,
        tiles: &[PreparedRasterTile],
    ) -> Result<()> {
        if (image.width(), image.height()) != source_size {
            return Err(Error::invalid(format!(
                "encoded raster {source} decoded as {}x{}, expected {}x{}",
                image.width(),
                image.height(),
                source_size.0,
                source_size.1
            )));
        }
        let limit = device.limits().max_texture_dimension_2d;
        for tile in tiles {
            let id = tile.id(source);
            let (width, height) = tile.resource_size();
            let (x, y) = tile.source_origin();
            if width > limit || height > limit {
                return Err(Error::invalid(format!(
                    "raster tile {width}x{height} exceeds the device limit {limit}"
                )));
            }
            if x.saturating_add(width) > source_size.0 || y.saturating_add(height) > source_size.1 {
                return Err(Error::invalid(
                    "raster tile crop exceeds its encoded source",
                ));
            }
            self.image_clock = self.image_clock.wrapping_add(1);
            if let Some(cached) = self.images.get_mut(&id) {
                cached.last_used = self.image_clock;
                continue;
            }
            let mut texture = create_texture(
                device,
                "koharu decoded browser raster tile",
                width,
                height,
                Some(source),
            );
            queue.copy_external_image_to_texture(
                &wgpu::CopyExternalImageSourceInfo {
                    source: image.clone(),
                    origin: wgpu::Origin2d { x, y },
                    flip_y: false,
                },
                wgpu::CopyExternalImageDestInfo {
                    texture: &texture.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                    color_space: wgpu::PredefinedColorSpace::Srgb,
                    premultiplied_alpha: true,
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
            texture.last_used = self.image_clock;
            if let Some(previous) = self.images.insert(id, texture) {
                self.image_bytes = self.image_bytes.saturating_sub(previous.byte_len);
            }
            self.image_bytes = self
                .image_bytes
                .saturating_add(u64::from(width) * u64::from(height) * 4);
        }
        Ok(())
    }

    fn evict_tile(&mut self, id: ResourceId) -> bool {
        let Some(removed) = self.images.remove(&id) else {
            return false;
        };
        self.image_bytes = self.image_bytes.saturating_sub(removed.byte_len);
        true
    }

    pub fn evict_source(&mut self, source: ResourceId) -> bool {
        let previous = self.images.len();
        self.images.retain(|_, texture| {
            if texture.source == Some(source) {
                self.image_bytes = self.image_bytes.saturating_sub(texture.byte_len);
                false
            } else {
                true
            }
        });
        self.images.len() != previous
    }

    pub fn clear_resources(&mut self) {
        self.images.clear();
        self.image_bytes = 0;
    }

    pub fn trim_cache(&mut self) {
        self.trim_cache_protected(&HashSet::new());
    }

    fn trim_cache_protected(&mut self, protected: &HashSet<ResourceId>) {
        while self.image_bytes > self.image_budget {
            let Some(id) = self
                .images
                .iter()
                .filter(|(id, _)| !protected.contains(id))
                .min_by_key(|(id, texture)| (texture.last_used, **id))
                .map(|(id, _)| *id)
            else {
                break;
            };
            self.evict_tile(id);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        vello: &mut vello::Renderer,
        target: &wgpu::TextureView,
        size: (u32, u32),
        commands: &[CompositionCommand],
        erase_mask: Option<&Scene>,
        background: [u8; 4],
        clip: [u32; 4],
    ) -> Result<()> {
        if size.0 == 0 || size.1 == 0 {
            return Ok(());
        }
        if commands
            .iter()
            .any(|command| matches!(command, CompositionCommand::Vector(_)))
            && self
                .overlay
                .as_ref()
                .is_none_or(|target| target.size != size)
        {
            self.overlay = Some(ScratchTarget::new(device, "koharu vector overlay", size));
        }
        if erase_mask.is_some() && self.erase.as_ref().is_none_or(|target| target.size != size) {
            self.erase = Some(ScratchTarget::new(device, "koharu erase overlay", size));
        }
        let active = commands
            .iter()
            .flat_map(|command| match command {
                CompositionCommand::Raster(draw) => draw
                    .image
                    .tiles()
                    .iter()
                    .map(|tile| tile.id())
                    .collect::<Vec<_>>(),
                CompositionCommand::Vector(_) => Vec::new(),
            })
            .collect::<HashSet<_>>();
        for command in commands {
            if let CompositionCommand::Raster(draw) = command {
                self.upload(device, queue, &draw.image)?;
            }
        }
        if let Some(mask) = erase_mask {
            render_vello(
                vello,
                device,
                queue,
                mask,
                &self
                    .erase
                    .as_ref()
                    .expect("erase target created above")
                    .view,
                size,
            )?;
        }
        let clip = clipped_rect(clip, size);
        let mut clear = Some(background);
        self.draws.clear();
        if clip[2] == 0 || clip[3] == 0 {
            self.flush(device, queue, target, &mut clear, clip);
            self.trim_cache_protected(&active);
            return Ok(());
        }
        let mut overlay_pending = false;
        for command in commands {
            match command {
                CompositionCommand::Raster(draw) => {
                    let erased = draw.erase && erase_mask.is_some();
                    for tile in draw.image.tiles() {
                        let cached = self
                            .images
                            .get_mut(&tile.id())
                            .expect("raster tile uploaded above");
                        let textures = if erased {
                            texture_bind_group(
                                device,
                                &self.texture_layout,
                                &self.sampler,
                                &cached.view,
                                &self
                                    .erase
                                    .as_ref()
                                    .expect("erase target created above")
                                    .view,
                            )
                        } else {
                            cached
                                .bind_group
                                .get_or_insert_with(|| {
                                    texture_bind_group(
                                        device,
                                        &self.texture_layout,
                                        &self.sampler,
                                        &cached.view,
                                        &self.empty_mask.view,
                                    )
                                })
                                .clone()
                        };
                        let origin = tile.origin();
                        let gutter = tile.gutter();
                        self.draws.push(Draw {
                            textures,
                            uniforms: draw_uniforms(
                                draw.transform
                                    * Affine::translate((f64::from(origin.0), f64::from(origin.1))),
                                tile.size(),
                                (gutter[0], gutter[1]),
                                tile.resource_size(),
                                size,
                                draw.opacity.clamp(0.0, 1.0),
                                erased,
                                false,
                            ),
                        });
                    }
                }
                CompositionCommand::Vector(scene) => {
                    // Every vector batch renders into the same overlay texture, so a queued
                    // draw of its previous contents must reach the GPU before Vello replaces
                    // them.
                    if overlay_pending {
                        self.flush(device, queue, target, &mut clear, clip);
                    }
                    let overlay = self.overlay.as_mut().expect("vector target created above");
                    render_vello(vello, device, queue, scene, &overlay.view, size)?;
                    let textures = overlay
                        .bind_group
                        .get_or_insert_with(|| {
                            texture_bind_group(
                                device,
                                &self.texture_layout,
                                &self.sampler,
                                &overlay.view,
                                &self.empty_mask.view,
                            )
                        })
                        .clone();
                    self.draws.push(Draw {
                        textures,
                        uniforms: draw_uniforms(
                            Affine::IDENTITY,
                            size,
                            (0, 0),
                            size,
                            size,
                            1.0,
                            false,
                            // Vello's fine stage un-premultiplies before storing its target.
                            true,
                        ),
                    });
                    overlay_pending = true;
                }
            }
        }
        self.flush(device, queue, target, &mut clear, clip);
        // The budget is soft for the current frame: inactive least-recently-used
        // textures are victims, while every texture referenced by this render
        // remains resident even when the active working set exceeds the budget.
        self.trim_cache_protected(&active);
        Ok(())
    }

    /// Encodes the queued draws as one render pass and submits it. The first pass of a frame
    /// also clears the target, which is why it runs even without draws.
    fn flush(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        clear: &mut Option<[u8; 4]>,
        clip: [u32; 4],
    ) {
        if self.draws.is_empty() && clear.is_none() {
            return;
        }
        if self.draws.len() > self.uniforms.capacity {
            self.uniforms = UniformArena::new(
                device,
                &self.uniform_layout,
                self.draws.len().next_power_of_two(),
            );
        }
        let stride = self.uniforms.stride;
        if !self.draws.is_empty() {
            let bytes = &mut self.uniforms.bytes;
            bytes.clear();
            bytes.resize(self.draws.len() * stride, 0);
            for (draw, slot) in self.draws.iter().zip(bytes.chunks_exact_mut(stride)) {
                for (value, bytes) in draw.uniforms.iter().zip(slot.as_chunks_mut::<4>().0) {
                    *bytes = value.to_ne_bytes();
                }
            }
            queue.write_buffer(&self.uniforms.buffer, 0, bytes);
        }
        let load = clear.take().map_or(wgpu::LoadOp::Load, |[r, g, b, a]| {
            wgpu::LoadOp::Clear(wgpu::Color {
                r: f64::from(r) / 255.0,
                g: f64::from(g) / 255.0,
                b: f64::from(b) / 255.0,
                a: f64::from(a) / 255.0,
            })
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("koharu raster compositor encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("koharu raster compositor pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if !self.draws.is_empty() {
                pass.set_scissor_rect(clip[0], clip[1], clip[2], clip[3]);
                pass.set_pipeline(&self.pipeline);
                for (index, draw) in self.draws.iter().enumerate() {
                    pass.set_bind_group(0, &draw.textures, &[]);
                    pass.set_bind_group(1, &self.uniforms.bind_group, &[(index * stride) as u32]);
                    pass.draw(0..6, 0..1);
                }
            }
        }
        queue.submit([encoder.finish()]);
        self.draws.clear();
    }

    fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        image: &RasterImage,
    ) -> Result<()> {
        for tile in image.tiles() {
            self.image_clock = self.image_clock.wrapping_add(1);
            if let Some(cached) = self.images.get_mut(&tile.id()) {
                cached.last_used = self.image_clock;
                continue;
            }
            let pixels = image.pixels().ok_or_else(|| {
                Error::invalid(format!(
                    "raster tile {} is not resident on the GPU",
                    tile.id()
                ))
            })?;
            self.upload_native_tile(device, queue, image, tile, pixels)?;
        }
        Ok(())
    }

    fn upload_native_tile(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        image: &RasterImage,
        tile: &crate::RasterTile,
        pixels: &[u8],
    ) -> Result<()> {
        let (image_width, image_height) = image.size();
        let expected = usize::try_from(u64::from(image_width) * u64::from(image_height) * 4)
            .map_err(|_| Error::invalid("raster image byte length exceeds usize"))?;
        if pixels.len() != expected {
            return Err(Error::invalid(format!(
                "raster image {} has invalid dimensions or byte length",
                image.source()
            )));
        }
        let id = tile.id();
        let (width, height) = tile.resource_size();
        let limit = device.limits().max_texture_dimension_2d;
        if width > limit || height > limit {
            return Err(Error::invalid(format!(
                "raster image {width}x{height} exceeds the device limit {limit}"
            )));
        }
        let mut texture = create_texture(
            device,
            "koharu decoded native raster tile",
            width,
            height,
            Some(image.source()),
        );
        // Every raster layer kind uses straight RGBA resources through this upload
        // boundary. Premultiply before linear filtering so transparent texels cannot
        // contribute dark RGB at source, cleanup, paint, or embedded layer edges.
        let (pixels, bytes_per_row) = premultiply_raster_tile_rgba8(
            pixels,
            image_width,
            image_height,
            tile.source_origin(),
            (width, height),
        )?;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        texture.byte_len = u64::try_from(expected).unwrap_or(u64::MAX);
        texture.last_used = self.image_clock;
        if let Some(previous) = self.images.insert(id, texture) {
            self.image_bytes = self.image_bytes.saturating_sub(previous.byte_len);
        }
        self.image_bytes = self
            .image_bytes
            .saturating_add(u64::try_from(expected).unwrap_or(u64::MAX));
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_uniforms(
    transform: Affine,
    source_size: (u32, u32),
    sample_origin: (u32, u32),
    resource_size: (u32, u32),
    target_size: (u32, u32),
    opacity: f32,
    use_erase: bool,
    straight_alpha: bool,
) -> [f32; UNIFORM_FLOATS] {
    let [a, b, c, d, e, f] = transform.as_coeffs();
    let pixel_aligned =
        a == 1.0 && b == 0.0 && c == 0.0 && d == 1.0 && e.fract() == 0.0 && f.fract() == 0.0;
    [
        a as f32,
        b as f32,
        c as f32,
        d as f32,
        e as f32,
        f as f32,
        source_size.0 as f32,
        source_size.1 as f32,
        target_size.0 as f32,
        target_size.1 as f32,
        opacity,
        f32::from(use_erase),
        f32::from(straight_alpha),
        f32::from(pixel_aligned),
        0.0,
        0.0,
        sample_origin.0 as f32,
        sample_origin.1 as f32,
        resource_size.0 as f32,
        resource_size.1 as f32,
    ]
}

fn texture_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    source: &wgpu::TextureView,
    erase: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("koharu raster compositor texture bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(source),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(erase),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn texture_layout(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn create_texture(
    device: &wgpu::Device,
    label: &str,
    width: u32,
    height: u32,
    source: Option<ResourceId>,
) -> CachedTexture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    CachedTexture {
        source,
        texture,
        view,
        bind_group: None,
        byte_len: u64::from(width) * u64::from(height) * 4,
        last_used: 0,
    }
}

fn premultiply_raster_tile_rgba8(
    pixels: &[u8],
    image_width: u32,
    image_height: u32,
    origin: (u32, u32),
    size: (u32, u32),
) -> Result<(Cow<'_, [u8]>, u32)> {
    if size.0 == 0
        || size.1 == 0
        || origin.0.saturating_add(size.0) > image_width
        || origin.1.saturating_add(size.1) > image_height
    {
        return Err(Error::invalid("native raster tile crop is invalid"));
    }
    let row_bytes = image_width * 4;
    let tile_row_bytes = size.0 * 4;
    let start =
        usize::try_from(u64::from(origin.1) * u64::from(row_bytes) + u64::from(origin.0) * 4)
            .map_err(|_| Error::invalid("native raster tile offset exceeds usize"))?;
    let end = start
        .checked_add(
            usize::try_from(
                u64::from(size.1 - 1) * u64::from(row_bytes) + u64::from(tile_row_bytes),
            )
            .map_err(|_| Error::invalid("native raster tile byte length exceeds usize"))?,
        )
        .ok_or_else(|| Error::invalid("native raster tile byte range exceeds usize"))?;
    let pixels = pixels
        .get(start..end)
        .ok_or_else(|| Error::invalid("native raster pixels are truncated"))?;
    let row_bytes = row_bytes as usize;
    let tile_row_bytes = tile_row_bytes as usize;
    let opaque = (0..size.1 as usize).all(|row| {
        let start = row * row_bytes;
        pixels[start..start + tile_row_bytes]
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[3] == u8::MAX)
    });
    if opaque {
        return Ok((Cow::Borrowed(pixels), row_bytes as u32));
    }
    let mut premultiplied = Vec::with_capacity(tile_row_bytes * size.1 as usize);
    for row in 0..size.1 as usize {
        let source = row * row_bytes;
        let destination = premultiplied.len();
        premultiplied.extend_from_slice(&pixels[source..source + tile_row_bytes]);
        for pixel in premultiplied[destination..].as_chunks_mut::<4>().0 {
            let alpha = u16::from(pixel[3]);
            for channel in &mut pixel[..3] {
                *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
            }
        }
    }
    Ok((Cow::Owned(premultiplied), tile_row_bytes as u32))
}

fn render_vello(
    renderer: &mut vello::Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &Scene,
    target: &wgpu::TextureView,
    size: (u32, u32),
) -> Result<()> {
    renderer
        .render_to_texture(
            device,
            queue,
            scene,
            target,
            &RenderParams {
                base_color: Color::TRANSPARENT,
                width: size.0,
                height: size.1,
                antialiasing_method: AaConfig::Area,
            },
        )
        .map_err(Error::backend)
}

fn clipped_rect([x, y, width, height]: [u32; 4], size: (u32, u32)) -> [u32; 4] {
    let x = x.min(size.0);
    let y = y.min(size.1);
    [
        x,
        y,
        width.min(size.0.saturating_sub(x)),
        height.min(size.1.saturating_sub(y)),
    ]
}
