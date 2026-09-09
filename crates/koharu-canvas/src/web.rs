use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::{Rc, Weak},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
};

use futures_channel::oneshot;
use js_sys::{Array, Function, Uint8Array};
use koharu_rasterizer::{
    CompositionCommand, Frame as PreparedFrame, GpuCompositor, LayerId, LayerKind, PreparedContent,
    PreparedFrameManifest, PreparedRasterTile, PreparedResource, PreparedResourcePacket,
    PreparedResourceStore, Presentation, RasterDraw, ResourceId, Revision,
};
use serde::{Deserialize, Serialize};
use vello::{
    AaSupport, RendererOptions, Scene,
    kurbo::{Affine, BezPath, Circle, Rect, Stroke},
    peniko::{Color, Fill, Mix},
    wgpu,
};
use wasm_bindgen::{JsCast as _, prelude::*};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    Blob, BlobPropertyBag, ColorSpaceConversion, HtmlCanvasElement, ImageBitmap,
    ImageBitmapOptions, ImageOrientation, PremultiplyAlpha, Window,
};

use crate::{cache::ResourceUsage, surface::SurfaceBlitter};

const MAX_BRUSH_DIAMETER: f32 = 128.0;
const RESOURCE_CACHE_BUDGET: u64 = 512 * 1024 * 1024;
const MAX_CACHED_RESOURCES: usize = 1_024;
const SAMPLE_ROW_BYTES: u64 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as u64;
static NEXT_DEVICE_ID: AtomicU32 = AtomicU32::new(1);

thread_local! {
    static DEVICE_TARGETS: RefCell<HashMap<u32, Weak<Shared>>> = RefCell::new(HashMap::new());
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct PhysicalSize {
    width: u32,
    height: u32,
}

impl PhysicalSize {
    const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
struct Point {
    x: f64,
    y: f64,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct ElementFrame {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    #[serde(default)]
    angle_degrees: f32,
}

impl ElementFrame {
    fn is_valid(self) -> bool {
        [self.x, self.y, self.width, self.height, self.angle_degrees]
            .into_iter()
            .all(f32::is_finite)
            && self.width > 0.0
            && self.height > 0.0
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ElementFrameDto {
    element: String,
    frame: ElementFrame,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TransformCommitDto {
    page: String,
    revision: u64,
    elements: Vec<ElementFrameDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StagedManifestDto {
    token: u32,
    missing: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
enum StrokeCommitDto {
    Paint {
        page: String,
        revision: u64,
        layer: Option<String>,
        diameter: f32,
        color: [u8; 4],
        points: Vec<Point>,
    },
    Erase {
        page: String,
        revision: u64,
        layer: String,
        diameter: f32,
        points: Vec<Point>,
    },
    Inpaint {
        page: String,
        revision: u64,
        diameter: f32,
        points: Vec<Point>,
    },
}

#[derive(Clone, Copy)]
struct Camera {
    zoom: f64,
    translation: [f64; 2],
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            translation: [0.0, 0.0],
        }
    }
}

impl Camera {
    fn new(zoom: f64, translation: [f64; 2]) -> Result<Self, JsValue> {
        if !zoom.is_finite() || zoom <= 0.0 || !translation.into_iter().all(f64::is_finite) {
            return Err(js_message(
                "camera values must be finite and zoom must be positive",
            ));
        }
        Ok(Self { zoom, translation })
    }

    fn affine(self) -> Affine {
        Affine::new([
            self.zoom,
            0.0,
            0.0,
            self.zoom,
            self.translation[0],
            self.translation[1],
        ])
    }
}

struct TransformEdit {
    page: LayerId,
    revision: Revision,
    order: Vec<LayerId>,
    originals: HashMap<LayerId, ElementFrame>,
    previews: HashMap<LayerId, ElementFrame>,
    last_sequence: Option<u64>,
    pending: bool,
}

impl TransformEdit {
    fn affine(&self, layer: LayerId) -> Option<Affine> {
        let original = self.originals.get(&layer)?;
        let preview = self.previews.get(&layer)?;
        Some(frame_transform(*original, *preview))
    }

    fn is_changed(&self) -> bool {
        self.order
            .iter()
            .any(|id| self.originals[id] != self.previews[id])
    }
}

#[derive(Clone, Copy, PartialEq)]
enum StrokeKind {
    Paint,
    Erase,
    Inpaint,
}

struct StrokeEdit {
    page: LayerId,
    revision: Revision,
    kind: StrokeKind,
    layer: Option<LayerId>,
    diameter: f32,
    color: [u8; 4],
    points: Vec<Point>,
    preview: Scene,
    pending: bool,
}

struct StagedFrame {
    token: u32,
    manifest: PreparedFrameManifest,
    resources: HashSet<ResourceId>,
}

struct RenderTarget {
    requested: PhysicalSize,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl RenderTarget {
    fn new(device: &wgpu::Device, requested: PhysicalSize) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("koharu browser viewport target"),
            size: wgpu::Extent3d {
                width: requested.width.max(1),
                height: requested.height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            requested,
            texture,
            view,
        }
    }
}

enum SampleStatus {
    Pending,
    Ready(Result<(), String>),
}

struct PendingSample {
    buffer: wgpu::Buffer,
    status: Arc<Mutex<SampleStatus>>,
    complete: oneshot::Sender<Result<[u8; 4], String>>,
}

struct CanvasRenderer {
    device: Rc<wgpu::Device>,
    queue: Rc<wgpu::Queue>,
    vello: vello::Renderer,
    compositor: GpuCompositor,
    target: RenderTarget,
    sample: Option<PendingSample>,
    wake: Arc<AtomicBool>,
}

impl CanvasRenderer {
    fn new(
        device: Rc<wgpu::Device>,
        queue: Rc<wgpu::Queue>,
        size: PhysicalSize,
        wake: Arc<AtomicBool>,
    ) -> Result<Self, JsValue> {
        let vello = vello::Renderer::new(
            &device,
            RendererOptions {
                antialiasing_support: AaSupport::area_only(),
                ..Default::default()
            },
        )
        .map_err(js_error)?;
        let compositor = GpuCompositor::with_cache_budget(&device, u64::MAX);
        let target = RenderTarget::new(&device, size);
        Ok(Self {
            device,
            queue,
            vello,
            compositor,
            target,
            sample: None,
            wake,
        })
    }

    fn resize(&mut self, size: PhysicalSize) {
        self.cancel_sample("color sample was cancelled by resize");
        self.target = RenderTarget::new(&self.device, size);
    }

    fn render(
        &mut self,
        commands: &[CompositionCommand],
        erase_mask: Option<&Scene>,
        background: [u8; 4],
        clip: [u32; 4],
    ) -> Result<(), JsValue> {
        if self.target.requested.is_empty() {
            return Ok(());
        }
        self.compositor
            .render(
                &self.device,
                &self.queue,
                &mut self.vello,
                &self.target.view,
                (self.target.requested.width, self.target.requested.height),
                commands,
                erase_mask,
                background,
                clip,
            )
            .map_err(js_error)
    }

    fn request_sample(
        &mut self,
        point: Point,
        complete: oneshot::Sender<Result<[u8; 4], String>>,
    ) -> Result<(), JsValue> {
        if self.sample.is_some() {
            return Err(js_message("a color sample is already pending"));
        }
        if !point.x.is_finite() || !point.y.is_finite() || point.x < 0.0 || point.y < 0.0 {
            return Err(js_message("sample point is outside the canvas"));
        }
        let x = point.x.floor() as u32;
        let y = point.y.floor() as u32;
        if x >= self.target.requested.width || y >= self.target.requested.height {
            return Err(js_message("sample point is outside the canvas"));
        }
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("koharu browser color sample"),
            size: SAMPLE_ROW_BYTES,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("koharu browser color sample encoder"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.target.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        let status = Arc::new(Mutex::new(SampleStatus::Pending));
        let callback_status = Arc::clone(&status);
        let wake = Arc::clone(&self.wake);
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                *callback_status.lock().expect("sample state poisoned") = SampleStatus::Ready(
                    result.map_err(|error| format!("failed to map color sample: {error}")),
                );
                wake.store(true, Ordering::Release);
            });
        self.sample = Some(PendingSample {
            buffer,
            status,
            complete,
        });
        Ok(())
    }

    fn poll_sample(&mut self) {
        let _ = self.device.poll(wgpu::PollType::Poll);
        let ready = self.sample.as_ref().and_then(|sample| {
            let mut status = sample.status.lock().expect("sample state poisoned");
            match std::mem::replace(&mut *status, SampleStatus::Pending) {
                SampleStatus::Pending => None,
                SampleStatus::Ready(result) => Some(result),
            }
        });
        let Some(ready) = ready else {
            return;
        };
        let sample = self.sample.take().expect("ready sample exists");
        let result = ready.and_then(|()| {
            let mapped = sample.buffer.slice(..).get_mapped_range();
            let color = (mapped.len() >= 4)
                .then(|| [mapped[0], mapped[1], mapped[2], mapped[3]])
                .ok_or_else(|| "mapped color sample is truncated".to_owned());
            drop(mapped);
            color
        });
        sample.buffer.unmap();
        let _ = sample.complete.send(result);
    }

    fn cancel_sample(&mut self, message: &str) {
        if let Some(sample) = self.sample.take() {
            sample.buffer.unmap();
            let _ = sample.complete.send(Err(message.to_owned()));
        }
    }

    fn cache_external_raster(
        &mut self,
        source: ResourceId,
        source_size: (u32, u32),
        bitmap: &ImageBitmap,
        tiles: &[PreparedRasterTile],
    ) -> Result<(), JsValue> {
        let image = wgpu::ExternalImageSource::ImageBitmap(bitmap.clone());
        self.compositor
            .cache_external_raster(
                &self.device,
                &self.queue,
                source,
                source_size,
                &image,
                tiles,
            )
            .map_err(js_error)
    }

    fn evict_source(&mut self, resource: ResourceId) {
        self.compositor.evict_source(resource);
    }
}

struct CanvasState {
    gpu: CanvasRenderer,
    frame: Option<PreparedFrame>,
    staged: Option<StagedFrame>,
    manifests: HashMap<LayerId, PreparedFrameManifest>,
    resources: PreparedResourceStore,
    resource_usage: ResourceUsage,
    active_resources: HashSet<ResourceId>,
    next_stage_token: u32,
    size: PhysicalSize,
    camera: Camera,
    workspace_color: [u8; 4],
    opacity: HashMap<LayerId, f32>,
    transform: Option<TransformEdit>,
    stroke: Option<StrokeEdit>,
    dirty: bool,
    generation: u64,
}

impl CanvasState {
    fn new(gpu: CanvasRenderer, size: PhysicalSize) -> Self {
        Self {
            gpu,
            frame: None,
            staged: None,
            manifests: HashMap::new(),
            resources: PreparedResourceStore::default(),
            resource_usage: ResourceUsage::default(),
            active_resources: HashSet::new(),
            next_stage_token: 0,
            size,
            camera: Camera::default(),
            workspace_color: [245, 245, 245, 255],
            opacity: HashMap::new(),
            transform: None,
            stroke: None,
            dirty: true,
            generation: 0,
        }
    }

    fn stage_manifest(&mut self, bytes: &[u8]) -> Result<StagedManifestDto, JsValue> {
        let manifest = PreparedFrameManifest::decode(bytes).map_err(js_error)?;
        let resources = manifest
            .required_resources()
            .iter()
            .map(|resource| resource.id)
            .collect::<HashSet<_>>();
        let missing = manifest
            .missing_resources(&self.resources)
            .into_iter()
            .map(|id| id.to_string())
            .collect();
        for resource in &resources {
            if self.resources.contains(*resource) {
                self.touch_resource(*resource);
            }
        }
        self.next_stage_token = self.next_stage_token.wrapping_add(1).max(1);
        let token = self.next_stage_token;
        self.staged = Some(StagedFrame {
            token,
            manifest,
            resources,
        });
        self.evict_resources();
        Ok(StagedManifestDto { token, missing })
    }

    fn wants_resource(&self, resource: ResourceId) -> bool {
        self.staged
            .as_ref()
            .is_some_and(|staged| staged.resources.contains(&resource))
    }

    fn has_resource(&self, resource: ResourceId) -> bool {
        self.resources.contains(resource)
    }

    fn install_resource(
        &mut self,
        expected: ResourceId,
        resource: PreparedResourcePacket,
        bitmap: Option<&ImageBitmap>,
    ) -> Result<(), JsValue> {
        if resource.id() != expected {
            return Err(js_message(
                "prepared resource packet does not match the requested content id",
            ));
        }
        let Some(reference) = self
            .staged
            .as_ref()
            .and_then(|staged| {
                staged
                    .manifest
                    .required_resources()
                    .iter()
                    .find(|resource| resource.id == expected)
            })
            .copied()
        else {
            return Ok(());
        };
        if resource.kind() != reference.kind || resource.byte_len() != reference.byte_len {
            return Err(js_message(
                "prepared resource packet does not match the staged manifest",
            ));
        }
        match resource.resource() {
            PreparedResource::Font { .. } => {
                if bitmap.is_some() {
                    return Err(js_message("font resource unexpectedly has decoded pixels"));
                }
            }
            PreparedResource::EncodedRaster { width, height, .. } => {
                let bitmap = bitmap.ok_or_else(|| {
                    js_message("encoded raster resource has not been decoded for the GPU")
                })?;
                let tiles = raster_tiles_for_source(
                    &self
                        .staged
                        .as_ref()
                        .expect("staged reference was resolved above")
                        .manifest,
                    expected,
                );
                if tiles.is_empty() {
                    return Err(js_message(
                        "encoded raster resource has no staged tile crops",
                    ));
                }
                self.gpu
                    .cache_external_raster(expected, (*width, *height), bitmap, &tiles)?;
            }
        }
        let _ = self.resources.insert(resource);
        self.resource_usage.insert(expected);
        self.evict_resources();
        Ok(())
    }

    fn activate_frame(&mut self, token: u32) -> Result<bool, JsValue> {
        let Some(staged) = self.staged.as_ref().filter(|staged| staged.token == token) else {
            return Ok(false);
        };
        let manifest = staged.manifest.clone();
        self.activate_manifest(manifest, true)
    }

    fn activate_page(
        &mut self,
        page: LayerId,
        expected_revision: Revision,
    ) -> Result<bool, JsValue> {
        let Some(manifest) = self.manifests.get(&page).cloned() else {
            return Ok(false);
        };
        if manifest.frame.revision != expected_revision {
            return Ok(false);
        }
        if !manifest.missing_resources(&self.resources).is_empty() {
            return Ok(false);
        }
        self.activate_manifest(manifest, false)
    }

    fn cache_frame(&mut self, token: u32) -> Result<bool, JsValue> {
        let Some(staged) = self.staged.as_ref().filter(|staged| staged.token == token) else {
            return Ok(false);
        };
        let manifest = staged.manifest.clone();
        let missing = manifest.missing_resources(&self.resources);
        if !missing.is_empty() {
            return Err(js_message(format!(
                "staged canvas manifest is missing {} resource(s)",
                missing.len()
            )));
        }
        let frame = manifest.compile(&self.resources).map_err(js_error)?;
        if frame.origin() != (0, 0) {
            return Err(js_message("canvas requires a complete frame at origin 0,0"));
        }
        if self
            .manifests
            .get(&frame.page())
            .is_some_and(|cached| cached.frame.revision > frame.revision())
        {
            self.staged = None;
            self.evict_resources();
            return Ok(false);
        }
        self.manifests.insert(frame.page(), manifest);
        self.staged = None;
        self.evict_resources();
        Ok(true)
    }

    fn activate_manifest(
        &mut self,
        manifest: PreparedFrameManifest,
        consume_staged: bool,
    ) -> Result<bool, JsValue> {
        let missing = manifest.missing_resources(&self.resources);
        if !missing.is_empty() {
            return Err(js_message(format!(
                "cached canvas manifest is missing {} resource(s)",
                missing.len()
            )));
        }
        let frame = manifest.compile(&self.resources).map_err(js_error)?;
        let resources = manifest
            .required_resources()
            .iter()
            .map(|resource| resource.id)
            .collect::<HashSet<_>>();
        if frame.origin() != (0, 0) {
            return Err(js_message("canvas requires a complete frame at origin 0,0"));
        }
        let page_changed = self
            .frame
            .as_ref()
            .is_some_and(|current| current.page() != frame.page());
        if self.frame.as_ref().is_some_and(|current| {
            current.page() == frame.page() && current.revision() > frame.revision()
        }) {
            if consume_staged {
                self.staged = None;
            }
            self.evict_resources();
            return Ok(false);
        }
        if page_changed {
            self.transform = None;
            self.stroke = None;
        } else {
            if self.transform.as_ref().is_some_and(|edit| {
                !edit.pending || (edit.page == frame.page() && edit.revision < frame.revision())
            }) {
                self.transform = None;
            }
            if self.stroke.as_ref().is_some_and(|edit| {
                !edit.pending || (edit.page == frame.page() && edit.revision < frame.revision())
            }) {
                self.stroke = None;
            }
        }
        self.opacity.clear();
        self.manifests.insert(frame.page(), manifest);
        self.frame = Some(frame);
        self.active_resources = resources;
        if consume_staged {
            self.staged = None;
        }
        let active = self.active_resources.iter().copied().collect::<Vec<_>>();
        for resource in active {
            self.touch_resource(resource);
        }
        self.evict_resources();
        self.dirty = true;
        Ok(true)
    }

    fn touch_resource(&mut self, resource: ResourceId) {
        self.resource_usage.touch(resource);
    }

    fn evict_resources(&mut self) {
        let mut protected = self.active_resources.clone();
        if let Some(staged) = self.staged.as_ref() {
            protected.extend(staged.resources.iter().copied());
        }
        for manifest in self.manifests.values() {
            protected.extend(
                manifest
                    .required_resources()
                    .iter()
                    .map(|resource| resource.id),
            );
        }
        while self
            .resources
            .total_bytes()
            .saturating_add(self.gpu.compositor.cached_resource_bytes())
            > RESOURCE_CACHE_BUDGET
            || self.resource_usage.len() > MAX_CACHED_RESOURCES
        {
            let candidate = self.resource_usage.oldest_unprotected(&protected);
            let Some(candidate) = candidate else {
                break;
            };
            self.resources.remove(candidate);
            self.resource_usage.remove(candidate);
            self.gpu.evict_source(candidate);
        }
    }

    fn clear(&mut self) {
        self.gpu
            .cancel_sample("color sample was cancelled by clear");
        self.frame = None;
        self.staged = None;
        self.manifests.clear();
        self.active_resources.clear();
        self.evict_resources();
        self.opacity.clear();
        self.transform = None;
        self.stroke = None;
        self.dirty = true;
    }

    fn render(&mut self) -> Result<(u64, bool), JsValue> {
        self.gpu.poll_sample();
        if self.dirty {
            let composition = self.composition();
            self.gpu.render(
                &composition.commands,
                composition.erase_mask.as_ref(),
                self.workspace_color,
                composition.clip,
            )?;
            if !self.size.is_empty() {
                self.generation = self.generation.wrapping_add(1).max(1);
            }
            self.dirty = false;
        }
        Ok((self.generation, self.gpu.sample.is_some()))
    }

    fn composition(&self) -> Composition {
        let Some(frame) = self.frame.as_ref() else {
            return Composition {
                commands: Vec::new(),
                erase_mask: None,
                clip: [0, 0, self.size.width, self.size.height],
            };
        };
        let page_size = frame.size();
        let page_rect = Rect::new(0.0, 0.0, f64::from(page_size.0), f64::from(page_size.1));
        let viewport_rect = Rect::new(
            0.0,
            0.0,
            f64::from(self.size.width),
            f64::from(self.size.height),
        );
        let camera = self.camera.affine();
        let normalize =
            Affine::translate((-f64::from(frame.origin().0), -f64::from(frame.origin().1)));
        let erase = self
            .stroke
            .as_ref()
            .filter(|stroke| stroke.kind == StrokeKind::Erase);
        let mut commands = Vec::new();
        let mut vectors = Scene::new();
        let mut vectors_pending = false;
        for layer in frame.layers() {
            let transform = normalize
                * self
                    .transform
                    .as_ref()
                    .and_then(|transform| transform.affine(layer.id()))
                    .unwrap_or(Affine::IDENTITY);
            let mut presentation = layer.presentation();
            if let Some(opacity) = self.opacity.get(&layer.id()) {
                presentation = Presentation {
                    opacity: *opacity,
                    ..presentation
                };
            }
            if let Some(image) = layer.raster_image() {
                flush_vectors(
                    &mut commands,
                    &mut vectors,
                    &mut vectors_pending,
                    camera,
                    page_rect,
                    viewport_rect,
                );
                if presentation.visible
                    && presentation.opacity.is_finite()
                    && presentation.opacity > 0.0
                {
                    commands.push(CompositionCommand::Raster(RasterDraw {
                        image: image.clone(),
                        transform: camera * transform * layer.placement(),
                        opacity: presentation.opacity.clamp(0.0, 1.0),
                        erase: erase.and_then(|edit| edit.layer) == Some(layer.id()),
                    }));
                }
            } else {
                layer.append_vector_with_presentation(&mut vectors, Some(transform), presentation);
                vectors_pending |= presentation.visible
                    && presentation.opacity.is_finite()
                    && presentation.opacity > 0.0;
            }
        }
        if let Some(stroke) = self
            .stroke
            .as_ref()
            .filter(|stroke| stroke.kind != StrokeKind::Erase)
        {
            let opacity = match stroke.kind {
                StrokeKind::Paint => f32::from(stroke.color[3]) / 255.0,
                StrokeKind::Inpaint => 116.0 / 255.0,
                StrokeKind::Erase => 1.0,
            };
            if opacity < 1.0 {
                vectors.push_layer(
                    Fill::NonZero,
                    Mix::Normal,
                    opacity,
                    Affine::IDENTITY,
                    &page_rect,
                );
            }
            vectors.append(&stroke.preview, None);
            if opacity < 1.0 {
                vectors.pop_layer();
            }
            vectors_pending = true;
        }
        flush_vectors(
            &mut commands,
            &mut vectors,
            &mut vectors_pending,
            camera,
            page_rect,
            viewport_rect,
        );
        Composition {
            commands,
            erase_mask: erase
                .map(|stroke| viewport_scene(&stroke.preview, camera, page_rect, viewport_rect)),
            clip: page_clip(self.camera, page_size, self.size),
        }
    }
}

fn raster_tiles_for_source(
    manifest: &PreparedFrameManifest,
    source: ResourceId,
) -> Vec<PreparedRasterTile> {
    let mut seen = HashSet::new();
    manifest
        .frame
        .layers
        .iter()
        .filter_map(|layer| match &layer.content {
            PreparedContent::Raster(raster) if raster.source == source => Some(&raster.tiles),
            _ => None,
        })
        .flatten()
        .copied()
        .filter(|tile| seen.insert(*tile))
        .collect()
}

async fn decode_encoded_raster(
    window: &Window,
    resource: &PreparedResource,
) -> Result<ImageBitmap, JsValue> {
    let PreparedResource::EncodedRaster {
        media_type, bytes, ..
    } = resource
    else {
        return Err(js_message("prepared resource is not an encoded raster"));
    };
    let parts = Array::new();
    parts.push(&Uint8Array::from(bytes.as_ref()));
    let blob_options = BlobPropertyBag::new();
    blob_options.set_type(media_type);
    let blob = Blob::new_with_u8_array_sequence_and_options(&parts, &blob_options)?;
    let bitmap_options = ImageBitmapOptions::new();
    bitmap_options.set_color_space_conversion(ColorSpaceConversion::None);
    bitmap_options.set_image_orientation(ImageOrientation::FromImage);
    bitmap_options.set_premultiply_alpha(PremultiplyAlpha::None);
    JsFuture::from(
        window.create_image_bitmap_with_blob_and_image_bitmap_options(&blob, &bitmap_options)?,
    )
    .await?
    .dyn_into::<ImageBitmap>()
    .map_err(|_| js_message("browser returned an invalid decoded raster"))
}

struct Composition {
    commands: Vec<CompositionCommand>,
    erase_mask: Option<Scene>,
    clip: [u32; 4],
}

struct BrowserGpu {
    _instance: wgpu::Instance,
    device: Rc<wgpu::Device>,
    queue: Rc<wgpu::Queue>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    surface_size: PhysicalSize,
    blitter: SurfaceBlitter,
    element: HtmlCanvasElement,
    canvas: CanvasState,
}

impl BrowserGpu {
    fn configure_surface(&mut self, size: PhysicalSize) {
        self.surface_size = size;
        if size.is_empty() {
            return;
        }
        if self.config.width != size.width || self.config.height != size.height {
            self.config.width = size.width;
            self.config.height = size.height;
            self.surface.configure(&self.device, &self.config);
        }
    }

    fn present(&mut self) -> Result<(u64, bool), JsValue> {
        let (generation, needs_redraw) = self.canvas.render()?;
        if self.surface_size.is_empty() {
            return Ok((generation, needs_redraw));
        }
        let (surface_texture, suboptimal) = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture) => (texture, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(texture) => (texture, true),
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok((generation, true));
            }
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                return Ok((generation, true));
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(js_message("browser canvas surface validation failed"));
            }
        };
        let target = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("koharu browser present encoder"),
            });
        let source = (!self.canvas.size.is_empty()).then_some(&self.canvas.gpu.target.view);
        self.blitter
            .copy(&self.device, &mut encoder, source, &target);
        self.queue.submit([encoder.finish()]);
        surface_texture.present();
        if suboptimal {
            self.surface.configure(&self.device, &self.config);
        }
        Ok((generation, needs_redraw || suboptimal))
    }
}

struct Shared {
    browser: RefCell<Option<BrowserGpu>>,
    window: Window,
    animation_frame: Cell<Option<i32>>,
    callback: Closure<dyn FnMut(f64)>,
    wake: Arc<AtomicBool>,
    disposed: Cell<bool>,
    device_id: u32,
    loss_reason: RefCell<Option<String>>,
    device_lost_callback: RefCell<Option<Function>>,
}

impl Shared {
    fn request_render(&self) -> Result<(), JsValue> {
        if self.disposed.get() || self.animation_frame.get().is_some() {
            return Ok(());
        }
        let handle = self
            .window
            .request_animation_frame(self.callback.as_ref().unchecked_ref())?;
        self.animation_frame.set(Some(handle));
        Ok(())
    }

    fn present(&self) -> Result<u64, JsValue> {
        if self.disposed.get() {
            return Err(js_message("browser canvas is disposed"));
        }
        if let Some(reason) = self.loss_reason.borrow().as_ref() {
            return Err(js_message(format!("WebGPU device is lost: {reason}")));
        }
        let (generation, needs_redraw) = {
            let mut browser = self.browser.borrow_mut();
            browser
                .as_mut()
                .ok_or_else(|| js_message("browser canvas is disposed"))?
                .present()?
        };
        if needs_redraw || self.wake.swap(false, Ordering::AcqRel) {
            self.request_render()?;
        }
        Ok(generation)
    }

    fn mark_device_lost(&self, reason: String) {
        if self.disposed.get() || self.loss_reason.borrow().is_some() {
            return;
        }
        *self.loss_reason.borrow_mut() = Some(reason.clone());
        if let Some(callback) = self.device_lost_callback.borrow().as_ref() {
            let _ = callback.call1(&JsValue::UNDEFINED, &JsValue::from_str(&reason));
        }
    }

    fn dispose(&self) {
        if self.disposed.replace(true) {
            return;
        }
        if let Some(handle) = self.animation_frame.take() {
            let _ = self.window.cancel_animation_frame(handle);
        }
        DEVICE_TARGETS.with(|targets| {
            targets.borrow_mut().remove(&self.device_id);
        });
        if let Some(mut browser) = self.browser.borrow_mut().take() {
            browser
                .canvas
                .gpu
                .cancel_sample("color sample was cancelled by dispose");
        }
        self.device_lost_callback.borrow_mut().take();
    }
}

/// Browser WebGPU viewport over native-prepared Koharu frames.
#[wasm_bindgen]
pub struct WebCanvas {
    shared: Rc<Shared>,
}

/// Initializes a browser WebGPU canvas.
#[wasm_bindgen(js_name = createCanvas)]
pub async fn create_canvas(element: HtmlCanvasElement) -> Result<WebCanvas, JsValue> {
    let window = web_sys::window().ok_or_else(|| js_message("browser window is unavailable"))?;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::BROWSER_WEBGPU,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let surface = instance
        .create_surface(wgpu::SurfaceTarget::Canvas(element.clone()))
        .map_err(js_error)?;
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
        })
        .await
        .map_err(js_error)?;
    if !adapter
        .get_downlevel_capabilities()
        .flags
        .contains(wgpu::DownlevelFlags::UNRESTRICTED_EXTERNAL_TEXTURE_COPIES)
    {
        return Err(js_message(
            "WebGPU adapter cannot upload premultiplied raster tile crops",
        ));
    }
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("koharu browser WebGPU device"),
            ..Default::default()
        })
        .await
        .map_err(js_error)?;
    let device = Rc::new(device);
    let queue = Rc::new(queue);
    let initial_size = PhysicalSize::new(element.width(), element.height());
    let config = surface
        .get_default_config(
            &adapter,
            initial_size.width.max(1),
            initial_size.height.max(1),
        )
        .ok_or_else(|| js_message("WebGPU adapter cannot present to the canvas"))?;
    surface.configure(&device, &config);
    let wake = Arc::new(AtomicBool::new(false));
    let renderer = CanvasRenderer::new(
        Rc::clone(&device),
        Rc::clone(&queue),
        initial_size,
        Arc::clone(&wake),
    )?;
    let browser = BrowserGpu {
        _instance: instance,
        device: Rc::clone(&device),
        queue,
        surface,
        config: config.clone(),
        surface_size: initial_size,
        blitter: SurfaceBlitter::new(&device, config.format),
        element,
        canvas: CanvasState::new(renderer, initial_size),
    };
    let device_id = NEXT_DEVICE_ID.fetch_add(1, Ordering::Relaxed);
    let shared = Rc::new_cyclic(|shared: &Weak<Shared>| {
        let shared = shared.clone();
        let callback = Closure::new(move |_timestamp: f64| {
            let Some(shared) = shared.upgrade() else {
                return;
            };
            shared.animation_frame.set(None);
            if let Err(error) = shared.present() {
                shared.mark_device_lost(js_value_message(&error));
            }
        });
        Shared {
            browser: RefCell::new(Some(browser)),
            window,
            animation_frame: Cell::new(None),
            callback,
            wake,
            disposed: Cell::new(false),
            device_id,
            loss_reason: RefCell::new(None),
            device_lost_callback: RefCell::new(None),
        }
    });
    DEVICE_TARGETS.with(|targets| {
        targets
            .borrow_mut()
            .insert(device_id, Rc::downgrade(&shared));
    });
    device.set_device_lost_callback(move |reason, message| {
        let reason = if message.is_empty() {
            format!("{reason:?}")
        } else {
            format!("{reason:?}: {message}")
        };
        DEVICE_TARGETS.with(|targets| {
            if let Some(shared) = targets.borrow().get(&device_id).and_then(Weak::upgrade) {
                shared.mark_device_lost(reason);
            }
        });
    });
    shared.request_render()?;
    Ok(WebCanvas { shared })
}

#[wasm_bindgen]
impl WebCanvas {
    pub fn resize(
        &self,
        css_width: f64,
        css_height: f64,
        device_pixel_ratio: f64,
        background: &Uint8Array,
    ) -> Result<(), JsValue> {
        let size = physical_size(css_width, css_height, device_pixel_ratio)?;
        let background = rgba(background)?;
        let mut browser = self.browser_mut()?;
        let size_changed = browser.canvas.size != size;
        let background_changed = browser.canvas.workspace_color != background;
        if size_changed {
            browser.element.set_width(size.width);
            browser.element.set_height(size.height);
            browser.configure_surface(size);
            browser.canvas.size = size;
            browser.canvas.gpu.resize(size);
        }
        browser.canvas.workspace_color = background;
        browser.canvas.dirty |= size_changed || background_changed;
        drop(browser);
        self.request_render()
    }

    #[wasm_bindgen(js_name = setView)]
    pub fn set_view(
        &self,
        zoom: f64,
        translation_x: f64,
        translation_y: f64,
    ) -> Result<(), JsValue> {
        let camera = Camera::new(zoom, [translation_x, translation_y])?;
        let mut browser = self.browser_mut()?;
        if browser.canvas.camera.zoom != camera.zoom
            || browser.canvas.camera.translation != camera.translation
        {
            browser.canvas.camera = camera;
            browser.canvas.dirty = true;
        }
        drop(browser);
        self.request_render()
    }

    #[wasm_bindgen(js_name = stageManifest)]
    pub fn stage_manifest(&self, bytes: &Uint8Array) -> Result<JsValue, JsValue> {
        let staged = self.browser_mut()?.canvas.stage_manifest(&bytes.to_vec())?;
        serde_wasm_bindgen::to_value(&staged).map_err(js_error)
    }

    #[wasm_bindgen(js_name = installResource)]
    pub async fn install_resource(&self, id: String, bytes: Uint8Array) -> Result<(), JsValue> {
        let id = parse_resource_id(&id)?;
        {
            let mut browser = self.browser_mut()?;
            if browser.canvas.has_resource(id) {
                browser.canvas.touch_resource(id);
                return Ok(());
            }
            if !browser.canvas.wants_resource(id) {
                return Ok(());
            }
        }
        let bytes = bytes.to_vec();
        let resource = PreparedResourcePacket::decode(&bytes).map_err(js_error)?;
        if resource.id() != id {
            return Err(js_message(
                "prepared resource packet does not match the requested content id",
            ));
        }
        let bitmap = match resource.resource() {
            PreparedResource::Font { .. } => None,
            PreparedResource::EncodedRaster { .. } => {
                Some(decode_encoded_raster(&self.shared.window, resource.resource()).await?)
            }
        };
        let result = self
            .browser_mut()?
            .canvas
            .install_resource(id, resource, bitmap.as_ref());
        if let Some(bitmap) = bitmap {
            bitmap.close();
        }
        result
    }

    #[wasm_bindgen(js_name = activateFrame)]
    pub fn activate_frame(&self, token: u32) -> Result<bool, JsValue> {
        {
            let browser = self.browser_mut()?;
            if browser
                .canvas
                .staged
                .as_ref()
                .is_none_or(|staged| staged.token != token)
            {
                return Ok(false);
            }
        }
        let activated = self.browser_mut()?.canvas.activate_frame(token)?;
        if activated {
            self.shared.present()?;
        }
        Ok(activated)
    }

    #[wasm_bindgen(js_name = activatePage)]
    pub fn activate_page(&self, page: &str, expected_revision: u64) -> Result<bool, JsValue> {
        let page = parse_layer_id(page)?;
        let activated = self
            .browser_mut()?
            .canvas
            .activate_page(page, Revision::new(expected_revision))?;
        if activated {
            self.shared.present()?;
        }
        Ok(activated)
    }

    #[wasm_bindgen(js_name = cacheFrame)]
    pub fn cache_frame(&self, token: u32) -> Result<bool, JsValue> {
        self.browser_mut()?.canvas.cache_frame(token)
    }

    pub fn clear(&self) -> Result<(), JsValue> {
        self.browser_mut()?.canvas.clear();
        self.shared.present()?;
        Ok(())
    }

    #[wasm_bindgen(js_name = previewOpacity)]
    pub fn preview_opacity(&self, element: &str, opacity: Option<f32>) -> Result<(), JsValue> {
        let id = parse_layer_id(element)?;
        let mut browser = self.browser_mut()?;
        let frame = browser
            .canvas
            .frame
            .as_ref()
            .ok_or_else(|| js_message("no prepared frame is installed"))?;
        let layer = frame
            .layer(id)
            .ok_or_else(|| js_message("opacity target is not in the prepared frame"))?;
        let changed = match opacity {
            Some(opacity) => {
                if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
                    return Err(js_message("opacity must be between zero and one"));
                }
                if opacity == layer.presentation().opacity {
                    browser.canvas.opacity.remove(&id).is_some()
                } else {
                    browser.canvas.opacity.insert(id, opacity) != Some(opacity)
                }
            }
            None => browser.canvas.opacity.remove(&id).is_some(),
        };
        browser.canvas.dirty |= changed;
        drop(browser);
        self.request_render()
    }

    #[wasm_bindgen(js_name = beginTransform)]
    pub fn begin_transform(&self, elements: JsValue) -> Result<(), JsValue> {
        let requested: Vec<ElementFrameDto> =
            serde_wasm_bindgen::from_value(elements).map_err(js_error)?;
        let mut browser = self.browser_mut()?;
        let frame = browser
            .canvas
            .frame
            .as_ref()
            .ok_or_else(|| js_message("no prepared frame is installed"))?;
        if browser.canvas.transform.is_some() || browser.canvas.stroke.is_some() {
            return Err(js_message("another canvas edit is already active"));
        }
        if requested.is_empty() {
            return Err(js_message("a transform requires at least one element"));
        }
        let mut order = Vec::with_capacity(requested.len());
        let mut originals = HashMap::with_capacity(requested.len());
        for element in requested {
            if !element.frame.is_valid() {
                return Err(js_message(
                    "transform control frame must be finite and non-empty",
                ));
            }
            let id = parse_layer_id(&element.element)?;
            if !originals.is_empty() && originals.contains_key(&id) {
                return Err(js_message("transform selection repeats an element"));
            }
            let layer = frame
                .layer(id)
                .ok_or_else(|| js_message("transform element is not in the prepared frame"))?;
            let presentation = layer.presentation();
            if layer.kind() != LayerKind::Text
                || !presentation.visible
                || presentation.opacity <= 0.0
            {
                return Err(js_message(
                    "transform element is not selectable and visible",
                ));
            }
            let control = layer
                .element_frame()
                .ok_or_else(|| js_message("transform element has no control frame"))?;
            let control = ElementFrame {
                x: control.x,
                y: control.y,
                width: control.width,
                height: control.height,
                angle_degrees: control.angle_degrees,
            };
            order.push(id);
            originals.insert(id, control);
        }
        browser.canvas.transform = Some(TransformEdit {
            page: frame.page(),
            revision: frame.revision(),
            order,
            previews: originals.clone(),
            originals,
            last_sequence: None,
            pending: false,
        });
        Ok(())
    }

    #[wasm_bindgen(js_name = updateTransform)]
    pub fn update_transform(&self, sequence: f64, elements: JsValue) -> Result<(), JsValue> {
        let sequence = sequence_number(sequence)?;
        let supplied: Vec<ElementFrameDto> =
            serde_wasm_bindgen::from_value(elements).map_err(js_error)?;
        let mut browser = self.browser_mut()?;
        let edit = browser
            .canvas
            .transform
            .as_mut()
            .ok_or_else(|| js_message("no transform is active"))?;
        if edit.pending {
            return Err(js_message("transform is waiting for its replacement frame"));
        }
        if edit
            .last_sequence
            .is_some_and(|previous| sequence <= previous)
        {
            return Ok(());
        }
        if supplied.len() != edit.order.len() {
            return Err(js_message("transform update is incomplete"));
        }
        let mut previews = HashMap::with_capacity(supplied.len());
        for element in supplied {
            if !element.frame.is_valid() {
                return Err(js_message("transform frame must be finite and non-empty"));
            }
            let id = parse_layer_id(&element.element)?;
            if !edit.originals.contains_key(&id) || previews.insert(id, element.frame).is_some() {
                return Err(js_message(
                    "transform update has unknown or repeated elements",
                ));
            }
        }
        edit.previews = previews;
        edit.last_sequence = Some(sequence);
        browser.canvas.dirty = true;
        drop(browser);
        self.request_render()
    }

    #[wasm_bindgen(js_name = finishTransform)]
    pub fn finish_transform(&self) -> Result<JsValue, JsValue> {
        let mut browser = self.browser_mut()?;
        let edit = browser
            .canvas
            .transform
            .as_mut()
            .ok_or_else(|| js_message("no transform is active"))?;
        if edit.pending {
            return Err(js_message("transform is already waiting for replacement"));
        }
        if !edit.is_changed() {
            browser.canvas.transform = None;
            return Ok(JsValue::NULL);
        }
        edit.pending = true;
        let output = TransformCommitDto {
            page: format_layer_id(edit.page),
            revision: edit.revision.get(),
            elements: edit
                .order
                .iter()
                .map(|id| ElementFrameDto {
                    element: format_layer_id(*id),
                    frame: edit.previews[id],
                })
                .collect(),
        };
        serde_wasm_bindgen::to_value(&output).map_err(js_error)
    }

    #[wasm_bindgen(js_name = cancelTransform)]
    pub fn cancel_transform(&self) -> Result<(), JsValue> {
        let mut browser = self.browser_mut()?;
        if browser.canvas.transform.take().is_some() {
            browser.canvas.dirty = true;
        }
        drop(browser);
        self.request_render()
    }

    #[wasm_bindgen(js_name = beginStroke)]
    pub fn begin_stroke(
        &self,
        kind: &str,
        layer: Option<String>,
        point: JsValue,
        diameter: f32,
        color: &Uint8Array,
    ) -> Result<(), JsValue> {
        let point: Point = serde_wasm_bindgen::from_value(point).map_err(js_error)?;
        validate_brush(diameter)?;
        let color = rgba(color)?;
        let mut browser = self.browser_mut()?;
        if browser.canvas.stroke.is_some() || browser.canvas.transform.is_some() {
            return Err(js_message("another canvas edit is already active"));
        }
        let frame = browser
            .canvas
            .frame
            .as_ref()
            .ok_or_else(|| js_message("no prepared frame is installed"))?;
        validate_page_point(point, frame.size())?;
        let kind = match kind {
            "paint" => StrokeKind::Paint,
            "erase" => StrokeKind::Erase,
            "inpaint" => StrokeKind::Inpaint,
            _ => return Err(js_message("unknown canvas stroke kind")),
        };
        let layer = layer.as_deref().map(parse_layer_id).transpose()?;
        if kind == StrokeKind::Erase {
            let layer = layer.ok_or_else(|| js_message("erase requires a raster layer"))?;
            let target = frame
                .layer(layer)
                .ok_or_else(|| js_message("erase target is not in the prepared frame"))?;
            if target.kind() != LayerKind::Raster {
                return Err(js_message("erase target is not a raster layer"));
            }
        } else if kind == StrokeKind::Paint {
            if let Some(layer) = layer {
                let target = frame
                    .layer(layer)
                    .ok_or_else(|| js_message("paint target is not in the prepared frame"))?;
                if target.kind() != LayerKind::Raster {
                    return Err(js_message("paint target is not a raster layer"));
                }
            }
        } else if layer.is_some() {
            return Err(js_message("inpaint preview does not accept a layer target"));
        }
        let preview_color = match kind {
            StrokeKind::Erase => [0, 0, 0, 255],
            StrokeKind::Inpaint => [168, 85, 247, 255],
            StrokeKind::Paint => [color[0], color[1], color[2], 255],
        };
        let mut preview = Scene::new();
        draw_dot(&mut preview, point, diameter, preview_color);
        browser.canvas.stroke = Some(StrokeEdit {
            page: frame.page(),
            revision: frame.revision(),
            kind,
            layer,
            diameter,
            color,
            points: vec![point],
            preview,
            pending: false,
        });
        browser.canvas.dirty = true;
        drop(browser);
        self.request_render()
    }

    #[wasm_bindgen(js_name = extendStroke)]
    pub fn extend_stroke(&self, points: JsValue) -> Result<(), JsValue> {
        let points: Vec<Point> = serde_wasm_bindgen::from_value(points).map_err(js_error)?;
        let mut browser = self.browser_mut()?;
        let size = browser
            .canvas
            .frame
            .as_ref()
            .ok_or_else(|| js_message("no prepared frame is installed"))?
            .size();
        let zoom = browser.canvas.camera.zoom.max(f64::EPSILON);
        let stroke = browser
            .canvas
            .stroke
            .as_mut()
            .ok_or_else(|| js_message("no stroke is active"))?;
        if stroke.pending {
            return Err(js_message("stroke is waiting for its replacement frame"));
        }
        let mut changed = false;
        for point in points {
            if !point.x.is_finite() || !point.y.is_finite() {
                return Err(js_message("stroke points must be finite"));
            }
            let point = Point {
                x: point.x.clamp(0.0, f64::from(size.0)),
                y: point.y.clamp(0.0, f64::from(size.1)),
            };
            let previous = *stroke.points.last().expect("stroke has an initial point");
            if (previous.x - point.x).hypot(previous.y - point.y) < 0.25 / zoom {
                continue;
            }
            let preview_color = match stroke.kind {
                StrokeKind::Erase => [0, 0, 0, 255],
                StrokeKind::Inpaint => [168, 85, 247, 255],
                StrokeKind::Paint => [stroke.color[0], stroke.color[1], stroke.color[2], 255],
            };
            draw_segment(
                &mut stroke.preview,
                previous,
                point,
                stroke.diameter,
                preview_color,
            );
            stroke.points.push(point);
            changed = true;
        }
        browser.canvas.dirty |= changed;
        drop(browser);
        self.request_render()
    }

    #[wasm_bindgen(js_name = finishStroke)]
    pub fn finish_stroke(&self) -> Result<JsValue, JsValue> {
        let mut browser = self.browser_mut()?;
        let stroke = browser
            .canvas
            .stroke
            .as_mut()
            .ok_or_else(|| js_message("no stroke is active"))?;
        if stroke.pending {
            return Err(js_message("stroke is already waiting for replacement"));
        }
        stroke.pending = true;
        let page = format_layer_id(stroke.page);
        let revision = stroke.revision.get();
        let output = match stroke.kind {
            StrokeKind::Paint => StrokeCommitDto::Paint {
                page,
                revision,
                layer: stroke.layer.map(format_layer_id),
                diameter: stroke.diameter,
                color: stroke.color,
                points: stroke.points.clone(),
            },
            StrokeKind::Erase => StrokeCommitDto::Erase {
                page,
                revision,
                layer: format_layer_id(stroke.layer.expect("erase target validated")),
                diameter: stroke.diameter,
                points: stroke.points.clone(),
            },
            StrokeKind::Inpaint => StrokeCommitDto::Inpaint {
                page,
                revision,
                diameter: stroke.diameter,
                points: stroke.points.clone(),
            },
        };
        serde_wasm_bindgen::to_value(&output).map_err(js_error)
    }

    #[wasm_bindgen(js_name = cancelStroke)]
    pub fn cancel_stroke(&self) -> Result<(), JsValue> {
        let mut browser = self.browser_mut()?;
        if browser.canvas.stroke.take().is_some() {
            browser.canvas.dirty = true;
        }
        drop(browser);
        self.request_render()
    }

    #[wasm_bindgen(js_name = sampleColor)]
    pub async fn sample_color(&self, x: f64, y: f64) -> Result<Uint8Array, JsValue> {
        let (send, receive) = oneshot::channel();
        {
            let mut browser = self.browser_mut()?;
            if browser.canvas.generation == 0 || browser.canvas.dirty {
                browser.canvas.render()?;
            }
            browser.canvas.gpu.request_sample(Point { x, y }, send)?;
        }
        self.request_render()?;
        let sampled = receive
            .await
            .map_err(|_| js_message("color sample was cancelled"))?
            .map_err(js_message)?;
        Ok(Uint8Array::from(sampled.as_slice()))
    }

    /// Presents immediately and resolves after the browser reaches its next frame.
    pub async fn render(&self) -> Result<u64, JsValue> {
        let generation = self.shared.present()?;
        next_animation_frame(&self.shared.window).await?;
        Ok(generation)
    }

    #[wasm_bindgen(js_name = requestRender)]
    pub fn request_render(&self) -> Result<(), JsValue> {
        self.shared.request_render()
    }

    #[wasm_bindgen(js_name = setDeviceLostCallback)]
    pub fn set_device_lost_callback(&self, callback: Option<Function>) {
        *self.shared.device_lost_callback.borrow_mut() = callback;
        if let (Some(callback), Some(reason)) = (
            self.shared.device_lost_callback.borrow().as_ref(),
            self.shared.loss_reason.borrow().as_ref(),
        ) {
            let _ = callback.call1(&JsValue::UNDEFINED, &JsValue::from_str(reason));
        }
    }

    #[wasm_bindgen(getter, js_name = deviceLostReason)]
    pub fn device_lost_reason(&self) -> Option<String> {
        self.shared.loss_reason.borrow().clone()
    }

    pub fn dispose(&self) {
        self.shared.dispose();
    }
}

impl WebCanvas {
    fn browser_mut(&self) -> Result<std::cell::RefMut<'_, BrowserGpu>, JsValue> {
        if self.shared.disposed.get() {
            return Err(js_message("browser canvas is disposed"));
        }
        if let Some(reason) = self.shared.loss_reason.borrow().as_ref() {
            return Err(js_message(format!("WebGPU device is lost: {reason}")));
        }
        std::cell::RefMut::filter_map(self.shared.browser.borrow_mut(), Option::as_mut)
            .map_err(|_| js_message("browser canvas is disposed"))
    }
}

impl Drop for WebCanvas {
    fn drop(&mut self) {
        self.shared.dispose();
    }
}

fn flush_vectors(
    commands: &mut Vec<CompositionCommand>,
    vectors: &mut Scene,
    pending: &mut bool,
    camera: Affine,
    page_rect: Rect,
    viewport_rect: Rect,
) {
    if !*pending {
        return;
    }
    let page = std::mem::replace(vectors, Scene::new());
    commands.push(CompositionCommand::Vector(Box::new(viewport_scene(
        &page,
        camera,
        page_rect,
        viewport_rect,
    ))));
    *pending = false;
}

fn viewport_scene(page: &Scene, camera: Affine, page_rect: Rect, viewport_rect: Rect) -> Scene {
    let mut scene = Scene::new();
    scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &viewport_rect);
    scene.push_clip_layer(Fill::NonZero, camera, &page_rect);
    scene.append(page, Some(camera));
    scene.pop_layer();
    scene.pop_layer();
    scene
}

fn page_clip(camera: Camera, page: (u32, u32), viewport: PhysicalSize) -> [u32; 4] {
    let left = camera.translation[0]
        .floor()
        .clamp(0.0, f64::from(viewport.width)) as u32;
    let top = camera.translation[1]
        .floor()
        .clamp(0.0, f64::from(viewport.height)) as u32;
    let right = (camera.translation[0] + f64::from(page.0) * camera.zoom)
        .ceil()
        .clamp(0.0, f64::from(viewport.width)) as u32;
    let bottom = (camera.translation[1] + f64::from(page.1) * camera.zoom)
        .ceil()
        .clamp(0.0, f64::from(viewport.height)) as u32;
    [
        left,
        top,
        right.saturating_sub(left),
        bottom.saturating_sub(top),
    ]
}

fn draw_dot(scene: &mut Scene, point: Point, diameter: f32, color: [u8; 4]) {
    let color = Color::from_rgba8(color[0], color[1], color[2], color[3]);
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        color,
        None,
        &Circle::new((point.x, point.y), f64::from(diameter) * 0.5),
    );
}

fn draw_segment(scene: &mut Scene, from: Point, to: Point, diameter: f32, color: [u8; 4]) {
    let color = Color::from_rgba8(color[0], color[1], color[2], color[3]);
    let mut path = BezPath::new();
    path.move_to((from.x, from.y));
    path.line_to((to.x, to.y));
    scene.stroke(
        &Stroke::new(f64::from(diameter)),
        Affine::IDENTITY,
        color,
        None,
        &path,
    );
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        color,
        None,
        &Circle::new((to.x, to.y), f64::from(diameter) * 0.5),
    );
}

fn frame_transform(original: ElementFrame, preview: ElementFrame) -> Affine {
    let original_angle = f64::from(original.angle_degrees).to_radians();
    let preview_angle = f64::from(preview.angle_degrees).to_radians();
    let (original_sin, original_cos) = original_angle.sin_cos();
    let (preview_sin, preview_cos) = preview_angle.sin_cos();
    let scale_x = f64::from(preview.width / original.width);
    let scale_y = f64::from(preview.height / original.height);
    let a = preview_cos * scale_x * original_cos + preview_sin * scale_y * original_sin;
    let b = preview_sin * scale_x * original_cos - preview_cos * scale_y * original_sin;
    let c = preview_cos * scale_x * original_sin - preview_sin * scale_y * original_cos;
    let d = preview_sin * scale_x * original_sin + preview_cos * scale_y * original_cos;
    let original_center_x = f64::from(original.x + original.width * 0.5);
    let original_center_y = f64::from(original.y + original.height * 0.5);
    let preview_center_x = f64::from(preview.x + preview.width * 0.5);
    let preview_center_y = f64::from(preview.y + preview.height * 0.5);
    Affine::new([
        a,
        b,
        c,
        d,
        preview_center_x - a * original_center_x - c * original_center_y,
        preview_center_y - b * original_center_x - d * original_center_y,
    ])
}

fn parse_layer_id(value: &str) -> Result<LayerId, JsValue> {
    let bytes_value = value.as_bytes();
    let valid_layout = bytes_value.len() == 32
        || (bytes_value.len() == 36
            && [8, 13, 18, 23]
                .into_iter()
                .all(|index| bytes_value[index] == b'-'));
    if !valid_layout {
        return Err(js_message("layer id is not a UUID"));
    }
    let mut bytes = [0; 16];
    let mut nibble = None;
    let mut index = 0;
    for byte in value.bytes().filter(|byte| *byte != b'-') {
        let value = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => return Err(js_message("layer id is not a UUID")),
        };
        if let Some(high) = nibble.take() {
            if index >= bytes.len() {
                return Err(js_message("layer id is not a UUID"));
            }
            bytes[index] = high << 4 | value;
            index += 1;
        } else {
            nibble = Some(value);
        }
    }
    if nibble.is_some() || index != bytes.len() {
        return Err(js_message("layer id is not a UUID"));
    }
    Ok(LayerId::from_bytes(bytes))
}

fn parse_resource_id(value: &str) -> Result<ResourceId, JsValue> {
    value
        .parse()
        .map_err(|error| js_message(format!("invalid prepared resource id: {error}")))
}

fn format_layer_id(id: LayerId) -> String {
    let bytes = id.as_bytes();
    let mut output = String::with_capacity(36);
    for (index, byte) in bytes.iter().enumerate() {
        if matches!(index, 4 | 6 | 8 | 10) {
            output.push('-');
        }
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing a String cannot fail");
    }
    output
}

fn validate_page_point(point: Point, size: (u32, u32)) -> Result<(), JsValue> {
    if point.x.is_finite()
        && point.y.is_finite()
        && point.x >= 0.0
        && point.y >= 0.0
        && point.x <= f64::from(size.0)
        && point.y <= f64::from(size.1)
    {
        Ok(())
    } else {
        Err(js_message("stroke must begin inside the page"))
    }
}

fn validate_brush(diameter: f32) -> Result<(), JsValue> {
    if diameter.is_finite() && diameter > 0.0 && diameter <= MAX_BRUSH_DIAMETER {
        Ok(())
    } else {
        Err(js_message(format!(
            "brush diameter must be in (0, {MAX_BRUSH_DIAMETER}]"
        )))
    }
}

fn physical_size(css_width: f64, css_height: f64, dpr: f64) -> Result<PhysicalSize, JsValue> {
    if !css_width.is_finite()
        || !css_height.is_finite()
        || !dpr.is_finite()
        || css_width < 0.0
        || css_height < 0.0
        || dpr <= 0.0
    {
        return Err(js_message("canvas size or device-pixel ratio is invalid"));
    }
    let width = (css_width * dpr).round();
    let height = (css_height * dpr).round();
    if width > f64::from(u32::MAX) || height > f64::from(u32::MAX) {
        return Err(js_message("canvas physical size exceeds u32"));
    }
    Ok(PhysicalSize::new(width as u32, height as u32))
}

fn rgba(value: &Uint8Array) -> Result<[u8; 4], JsValue> {
    if value.length() != 4 {
        return Err(js_message("colors require exactly four RGBA bytes"));
    }
    let mut color = [0; 4];
    value.copy_to(&mut color);
    Ok(color)
}

fn sequence_number(value: f64) -> Result<u64, JsValue> {
    const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
    if !value.is_finite() || value < 0.0 || value.fract() != 0.0 || value > MAX_SAFE_INTEGER {
        return Err(js_message(
            "transform sequence must be a non-negative safe integer",
        ));
    }
    Ok(value as u64)
}

async fn next_animation_frame(window: &Window) -> Result<(), JsValue> {
    let (send, receive) = oneshot::channel();
    let send = Rc::new(RefCell::new(Some(send)));
    let callback = {
        let send = Rc::clone(&send);
        Closure::<dyn FnMut(f64)>::new(move |_| {
            if let Some(send) = send.borrow_mut().take() {
                let _ = send.send(());
            }
        })
    };
    window.request_animation_frame(callback.as_ref().unchecked_ref())?;
    receive
        .await
        .map_err(|_| js_message("animation frame was cancelled"))?;
    Ok(())
}

fn js_error(error: impl std::fmt::Display) -> JsValue {
    js_message(error.to_string())
}

fn js_message(message: impl AsRef<str>) -> JsValue {
    js_sys::Error::new(message.as_ref()).into()
}

fn js_value_message(value: &JsValue) -> String {
    value
        .dyn_ref::<js_sys::Error>()
        .map(js_sys::Error::message)
        .map(String::from)
        .or_else(|| value.as_string())
        .unwrap_or_else(|| "unknown WebGPU canvas error".into())
}
