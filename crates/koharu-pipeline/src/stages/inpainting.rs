use std::{
    collections::BTreeMap,
    io::Cursor,
    sync::{Arc, Mutex},
};

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use async_trait::async_trait;
use image::{
    DynamicImage, GenericImageView as _, GrayImage, ImageBuffer, ImageFormat, Luma, Rgb, RgbImage,
    Rgba, RgbaImage,
};
use imageproc::{
    distance_transform::{Norm, distance_transform},
    morphology::dilate,
    region_labelling::{Connectivity, connected_components},
};
use koharu_ml::{
    aot_inpainting::AotInpainting,
    flux2_klein::{Flux2KleinInpaint, Flux2KleinInpaintOptions},
    lama::{InpaintRequest, LaMa},
    rorem_mixed::{DEFAULT_NEGATIVE_PROMPT, DEFAULT_PROMPT, RoremMixed, RoremMixedOptions},
};
use koharu_scene::{
    AssetInput, AssetMetadata, AssetRole, At, BubbleRegion, EntityOrigin, Geometry, Origin,
    RasterLayer, RasterLayerKind, Region, RegionSpec,
};
use serde::{Deserialize, Serialize};
use specta::Type;

use super::{StageInput, StageProcessor, finish, generation};
use crate::{InpaintingModel, ModelCell};

const PRODUCER: &str = "dev.koharu.pipeline.inpainting";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(default)]
pub struct Flux2KleinConfig {
    pub prompt: String,
}

impl Default for Flux2KleinConfig {
    fn default() -> Self {
        Self {
            prompt: "Remove the text and reconstruct the background.".to_owned(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(default)]
pub struct RoremMixedConfig {
    pub prompt: String,
    pub negative_prompt: String,
}

impl Default for RoremMixedConfig {
    fn default() -> Self {
        Self {
            prompt: DEFAULT_PROMPT.to_owned(),
            negative_prompt: DEFAULT_NEGATIVE_PROMPT.to_owned(),
        }
    }
}

pub(super) struct Processor {
    config: InpaintingModel,
    device: koharu_ml::Device,
    model: ModelCell<Model>,
}

impl Processor {
    pub(super) fn new(config: InpaintingModel, device: koharu_ml::Device) -> Result<Self> {
        match &config {
            InpaintingModel::LaMa {} | InpaintingModel::AotInpainting {} => {}
            InpaintingModel::Flux2Klein(settings) => {
                ensure!(
                    !settings.prompt.contains('\0'),
                    "FLUX.2 prompt contains NUL"
                );
            }
            InpaintingModel::RoremMixed(settings) => {
                ensure!(
                    !settings.prompt.contains('\0') && !settings.negative_prompt.contains('\0'),
                    "RORem prompt contains NUL"
                );
            }
        }

        Ok(Self {
            config,
            device,
            model: ModelCell::new(),
        })
    }
}

#[async_trait]
impl StageProcessor for Processor {
    fn model(&self) -> &'static str {
        match self.config {
            InpaintingModel::LaMa {} => "lama",
            InpaintingModel::AotInpainting {} => "aot-inpainting",
            InpaintingModel::Flux2Klein(_) => "flux2-klein",
            InpaintingModel::RoremMixed(_) => "rorem-mixed",
        }
    }

    fn skip(&self, input: &StageInput) -> Result<bool> {
        if input.inpainting_mask.is_some() {
            return Ok(false);
        }
        let source = AssetRole::new("source")?;
        for entity in input.scene.children(input.page)? {
            if input
                .scene
                .component::<RasterLayer>(entity)?
                .is_some_and(|layer| layer.kind == RasterLayerKind::Cleanup)
                && input.scene.asset(entity, &source)?.is_some()
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn unload(&self) -> bool {
        self.model.unload()
    }

    async fn load(&self) -> Result<()> {
        self.model
            .ensure(|| Model::load(self.device.clone(), &self.config))
            .await
    }

    async fn process(&self, input: StageInput) -> Result<koharu_scene::Patch> {
        self.model
            .lock()
            .await
            .as_ref()
            .ok_or_else(|| anyhow!("inpainting model is not loaded"))?
            .run(input)
            .await
    }
}

enum Model {
    LaMa(Arc<Mutex<LaMa>>),
    Aot(Arc<Mutex<AotInpainting>>),
    Flux {
        model: Arc<Mutex<Flux2KleinInpaint>>,
        config: Flux2KleinConfig,
    },
    Rorem {
        model: Arc<Mutex<RoremMixed>>,
        config: RoremMixedConfig,
    },
}

impl Model {
    async fn load(device: koharu_ml::Device, config: &InpaintingModel) -> Result<Self> {
        match config {
            InpaintingModel::LaMa {} => {
                Ok(Self::LaMa(Arc::new(Mutex::new(LaMa::load(device).await?))))
            }
            InpaintingModel::AotInpainting {} => Ok(Self::Aot(Arc::new(Mutex::new(
                AotInpainting::load(device).await?,
            )))),
            InpaintingModel::Flux2Klein(config) => Ok(Self::Flux {
                model: Arc::new(Mutex::new(Flux2KleinInpaint::load(device).await?)),
                config: config.clone(),
            }),
            InpaintingModel::RoremMixed(config) => Ok(Self::Rorem {
                model: Arc::new(Mutex::new(RoremMixed::load(device).await?)),
                config: config.clone(),
            }),
        }
    }

    async fn run(&self, input: StageInput) -> Result<koharu_scene::Patch> {
        let mut prepared = prepare(&input).await?;
        if prepared.mask.as_raw().iter().all(|value| *value == 0) {
            return finish(input.scene.edit());
        }
        let mask = prepared.mask.clone();
        let original = prepared.original.clone();
        let cleanup = prepared.cleanup.take();
        let cleanup_entity = prepared.cleanup_entity;
        let (model_name, image) = match self {
            Self::LaMa(model) => {
                let model = model.clone();
                (
                    "lama",
                    tokio_rayon::spawn(move || -> Result<DynamicImage> {
                        let model = model
                            .lock()
                            .map_err(|_| anyhow!("LaMa model lock is poisoned"))?;
                        inpaint_tiled(
                            &prepared.image,
                            &prepared.mask,
                            &prepared.text_mask,
                            &prepared.flat_fill_regions,
                            |image, mask| {
                                Ok(DynamicImage::ImageRgb8(model.inference(
                                    image,
                                    mask,
                                    &InpaintRequest::default(),
                                )?))
                            },
                        )
                    })
                    .await?,
                )
            }
            Self::Aot(model) => {
                let model = model.clone();
                (
                    "aot-inpainting",
                    tokio_rayon::spawn(move || -> Result<DynamicImage> {
                        let model = model
                            .lock()
                            .map_err(|_| anyhow!("AOT model lock is poisoned"))?;
                        inpaint_tiled(
                            &prepared.image,
                            &prepared.mask,
                            &prepared.text_mask,
                            &prepared.flat_fill_regions,
                            |image, mask| {
                                Ok(DynamicImage::ImageRgb8(model.inference(image, mask)?))
                            },
                        )
                    })
                    .await?,
                )
            }
            Self::Flux { model, config } => {
                let model = model.clone();
                let config = config.clone();
                (
                    "flux2-klein",
                    tokio_rayon::spawn(move || -> Result<DynamicImage> {
                        let model = model
                            .lock()
                            .map_err(|_| anyhow!("FLUX model lock is poisoned"))?;
                        inpaint_tiled(
                            &prepared.image,
                            &prepared.mask,
                            &prepared.text_mask,
                            &prepared.flat_fill_regions,
                            |image, mask| {
                                model.inference(
                                    &config.prompt,
                                    image,
                                    None,
                                    &DynamicImage::ImageLuma8(mask.clone()),
                                    &Flux2KleinInpaintOptions::default(),
                                )
                            },
                        )
                    })
                    .await?,
                )
            }
            Self::Rorem { model, config } => {
                let model = model.clone();
                let config = config.clone();
                (
                    "rorem-mixed",
                    tokio_rayon::spawn(move || -> Result<DynamicImage> {
                        let model = model
                            .lock()
                            .map_err(|_| anyhow!("RORem model lock is poisoned"))?;
                        inpaint_tiled(
                            &prepared.image,
                            &prepared.mask,
                            &prepared.text_mask,
                            &prepared.flat_fill_regions,
                            |image, mask| {
                                Ok(DynamicImage::ImageRgb8(model.inference(
                                    image,
                                    mask,
                                    &config.prompt,
                                    &config.negative_prompt,
                                    &RoremMixedOptions::default(),
                                )?))
                            },
                        )
                    })
                    .await?,
                )
            }
        };
        let page = input.page;
        let manual = input.inpainting_mask.is_some();
        let mut edit = if manual {
            input.scene.edit()
        } else {
            input.scene.edit_as(generation(PRODUCER, model_name)?)
        };
        edit.observe_assets(page)?;
        if let Some(entity) = cleanup_entity {
            edit.observe::<RasterLayer>(entity)?;
            edit.observe_assets(entity)?;
        }
        let image = image.to_rgba8();
        if image.dimensions() != original.dimensions() || image.dimensions() != mask.dimensions() {
            bail!("inpainted image dimensions do not match page {page}");
        }
        let mut overlay = if manual {
            cleanup.unwrap_or_else(|| RgbaImage::new(image.width(), image.height()))
        } else {
            RgbaImage::new(image.width(), image.height())
        };
        for (x, y, target) in overlay.enumerate_pixels_mut() {
            if mask.get_pixel(x, y)[0] < 127 {
                continue;
            }
            let generated = image.get_pixel(x, y);
            // Keep the complete cleanup mask opaque. Transparent texels next to
            // edited pixels let linear canvas sampling reveal the artwork below.
            *target = Rgba([generated[0], generated[1], generated[2], 255]);
        }
        let mut bytes = Cursor::new(Vec::new());
        let width = overlay.width();
        let height = overlay.height();
        DynamicImage::ImageRgba8(overlay).write_to(&mut bytes, ImageFormat::Png)?;
        let cleanup_entity = if let Some(entity) = cleanup_entity {
            if manual {
                let mut layer = input
                    .scene
                    .component::<RasterLayer>(entity)?
                    .context("cleanup entity has no raster layer component")?;
                let generated = layer.origin != Origin::User
                    || input
                        .scene
                        .component::<EntityOrigin>(entity)?
                        .is_some_and(|origin| origin.origin != Origin::User);
                if generated {
                    edit.promote_entity_to_user(entity)?;
                    layer.origin = Origin::User;
                    edit.set(entity, &layer)?;
                }
            }
            entity
        } else {
            let entity = edit.add_entity(page, At::Start)?;
            edit.set(
                entity,
                &RasterLayer {
                    origin: Origin::User,
                    name: "Cleanup".to_owned(),
                    kind: RasterLayerKind::Cleanup,
                },
            )?;
            entity
        };
        edit.set_asset(
            cleanup_entity,
            &AssetRole::new("source")?,
            AssetInput::new(
                Arc::<[u8]>::from(bytes.into_inner()),
                "image/png",
                AssetMetadata {
                    width: Some(width),
                    height: Some(height),
                    attributes: BTreeMap::new(),
                },
            ),
        )?;
        finish(edit)
    }
}

#[derive(Clone, Debug)]
struct FlatFillRegion {
    bounds: [u32; 4],
    polygon: Vec<(f32, f32)>,
}

fn flat_fill_regions(input: &StageInput, width: u32, height: u32) -> Result<Vec<FlatFillRegion>> {
    let mut regions = Vec::new();
    for entity in input.scene.descendants(input.page)? {
        let id = entity.id();
        let is_bubble = input
            .scene
            .component::<Region>(id)?
            .is_some_and(|region| region.kind == BubbleRegion::kind());
        if !is_bubble {
            continue;
        }
        let Some(geometry) = input.scene.component::<Geometry>(id)? else {
            continue;
        };
        let polygon = geometry
            .points
            .iter()
            .map(|point| (point.x as f32, point.y as f32))
            .collect::<Vec<_>>();
        if polygon.len() < 3 {
            continue;
        }
        let (mut left, mut top) = (f32::INFINITY, f32::INFINITY);
        let (mut right, mut bottom) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
        for &(x, y) in &polygon {
            left = left.min(x);
            top = top.min(y);
            right = right.max(x);
            bottom = bottom.max(y);
        }
        let bounds = [
            left.floor().clamp(0.0, width as f32) as u32,
            top.floor().clamp(0.0, height as f32) as u32,
            right.ceil().clamp(0.0, width as f32) as u32,
            bottom.ceil().clamp(0.0, height as f32) as u32,
        ];
        if bounds[2] > bounds[0] && bounds[3] > bounds[1] {
            regions.push(FlatFillRegion { bounds, polygon });
        }
    }
    Ok(regions)
}

struct InpaintInput {
    image: Arc<DynamicImage>,
    original: Arc<DynamicImage>,
    cleanup_entity: Option<koharu_scene::EntityId>,
    cleanup: Option<RgbaImage>,
    mask: GrayImage,
    text_mask: GrayImage,
    flat_fill_regions: Vec<FlatFillRegion>,
}

async fn prepare(input: &StageInput) -> Result<InpaintInput> {
    let page = input.page;
    let original = input
        .images
        .get(&input.scene, page, "source")
        .await?
        .ok_or_else(|| anyhow!("page {page} has no source image"))?;
    let cleanup_entity = input.scene.children(page)?.find(|entity| {
        input
            .scene
            .component::<RasterLayer>(*entity)
            .ok()
            .flatten()
            .is_some_and(|layer| layer.kind == RasterLayerKind::Cleanup)
    });
    let cleanup = if let Some(entity) = cleanup_entity {
        input
            .images
            .get(&input.scene, entity, "source")
            .await?
            .map(|image| image.to_rgba8())
    } else {
        None
    };
    if cleanup
        .as_ref()
        .is_some_and(|image| image.dimensions() != original.dimensions())
    {
        bail!("cleanup layer dimensions do not match page {page}");
    }
    let source = if input.inpainting_mask.is_some() {
        if let Some(cleanup) = cleanup.as_ref() {
            let mut composite = original.to_rgba8();
            image::imageops::overlay(&mut composite, cleanup, 0, 0);
            Arc::new(DynamicImage::ImageRgba8(composite))
        } else {
            original.clone()
        }
    } else {
        original.clone()
    };
    let mut mask = GrayImage::new(source.width(), source.height());
    let mut text_mask = GrayImage::new(source.width(), source.height());
    if let Some(transient) = &input.inpainting_mask {
        let layer = image::load_from_memory(&transient.png)?.to_luma8();
        if layer.dimensions() != mask.dimensions() {
            bail!("inpainting mask dimensions do not match page {page}");
        }
        mask = layer;
    } else {
        for role in ["text-mask", "coo-mask"] {
            if let Some(image) = input.images.get(&input.scene, page, role).await? {
                let layer = image.to_luma8();
                if layer.dimensions() != mask.dimensions() {
                    bail!("{role} dimensions do not match page {page}");
                }
                for (target, source) in mask.iter_mut().zip(layer.as_raw()) {
                    *target = (*target).max(*source);
                }
                if role == "text-mask" {
                    text_mask = layer;
                }
            }
        }
    }
    if let Some(bounds) = input.region {
        for (x, y, pixel) in mask.enumerate_pixels_mut() {
            if f64::from(x + 1) <= bounds.x
                || f64::from(y + 1) <= bounds.y
                || f64::from(x) >= bounds.x + bounds.width
                || f64::from(y) >= bounds.y + bounds.height
            {
                *pixel = Luma([0]);
                text_mask.put_pixel(x, y, Luma([0]));
            }
        }
    }
    let flat_fill_regions = flat_fill_regions(input, source.width(), source.height())?;
    Ok(InpaintInput {
        image: source,
        original,
        cleanup_entity,
        cleanup,
        mask,
        text_mask,
        flat_fill_regions,
    })
}

const TILE_SIZE: u32 = 512;
const TILE_CONTEXT: u32 = 128;
const UNIFORM_BACKGROUND_MIN_PIXELS: usize = 16;
const FLAT_FILL_EDGE_MARGIN: f32 = 3.0;
const UNIFORM_OUTLIER_DISTANCE: u8 = 48;
const UNIFORM_OUTLIER_SHARE: f64 = 0.03;
// The tile and flat-fill sizes above are tuned for pages up to this long side.
// A larger page is worked on as if it were scaled down to it: its tiles cover the
// same share of the artwork and reach the model at the size the model expects, so
// a glyph's hole doesn't grow past what LaMa and AOT can fill.
const REFERENCE_PAGE_SIDE: u32 = 2048;
// A hole reaching further than this from its nearest edge, in working pixels, is
// more than LaMa and AOT fill well. Each hole is shrunk only until it fits, up to
// the page's own scale, so thin text over detailed art keeps its full resolution.
const MAX_HOLE_RADIUS: f32 = 24.0;
// Text outside any detected bubble is flat-filled when a ring around it is one
// colour. The ring starts a little away from the hole to skip glyph antialiasing.
const SURROUNDING_GAP: f32 = 2.0;
const SURROUNDING_RING: f32 = 6.0;
// Every pixel of that ring, and of the ring around text in a bubble, must be this
// close to the fill colour. A spread measure alone lets a thin line crossing the
// hole through, and filling would cut the line.
const RING_PIXEL_TOLERANCE: u8 = 40;

#[derive(Clone, Copy, Debug, PartialEq)]
struct TileGeometry {
    /// Page pixels per working pixel; 1 for pages up to `REFERENCE_PAGE_SIDE`.
    scale: f32,
    size: u32,
    context: u32,
}

impl TileGeometry {
    fn for_page(width: u32, height: u32) -> Self {
        Self::with_scale((width.max(height) as f32 / REFERENCE_PAGE_SIDE as f32).max(1.0))
    }

    fn with_scale(scale: f32) -> Self {
        Self {
            scale,
            size: (TILE_SIZE as f32 * scale).round() as u32,
            context: (TILE_CONTEXT as f32 * scale).round() as u32,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct InpaintTile {
    components: Vec<u32>,
    core: [u32; 4],
    crop: [u32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MaskComponent {
    label: u32,
    bounds: [u32; 4],
}

// BallonsTranslator avoids model inference for a text block when the non-text
// pixels inside its balloon are nearly uniform, and otherwise sends an enlarged
// block crop to the inpainter:
// https://github.com/dmMaze/BallonsTranslator/blob/4bcc635c19f6c63a902872cf77b3d554e14ed1b7/ballontranslator/modules/inpaint/base.py#L168-L200
// Koharu uses the detected bubble polygons as those blocks. Uniform bubbles are
// filled first; only the remaining mask is split into bounded model crops.
fn inpaint_tiled(
    image: &DynamicImage,
    mask: &GrayImage,
    text_mask: &GrayImage,
    flat_fill_regions: &[FlatFillRegion],
    mut inference: impl FnMut(&DynamicImage, &GrayImage) -> Result<DynamicImage>,
) -> Result<DynamicImage> {
    ensure!(
        image.dimensions() == mask.dimensions() && mask.dimensions() == text_mask.dimensions(),
        "image and mask dimensions differ: image={:?}, mask={:?}, text_mask={:?}",
        image.dimensions(),
        mask.dimensions(),
        text_mask.dimensions()
    );

    let mut output = image.to_rgb8();
    let mut pending_mask = mask.clone();
    let page_scale = TileGeometry::for_page(image.width(), image.height()).scale;
    fill_uniform_regions(
        &mut output,
        &mut pending_mask,
        text_mask,
        flat_fill_regions,
        page_scale,
    );
    let (component_labels, components) = mask_components(&pending_mask);
    let components = fill_uniform_surroundings(
        &mut output,
        &mut pending_mask,
        &component_labels,
        components,
        page_scale,
    );
    if components.is_empty() {
        return Ok(DynamicImage::ImageRgb8(output));
    }
    for (scale, components) in
        group_by_hole_scale(&pending_mask, &component_labels, components, page_scale)
    {
        let geometry = TileGeometry::with_scale(scale);
        let tiles = inpaint_tiles(&component_labels, components, geometry);
        for tile in &tiles {
            let [left, top, right, bottom] = tile.crop;
            let crop_width = right - left;
            let crop_height = bottom - top;
            let crop_image =
                image::imageops::crop_imm(&output, left, top, crop_width, crop_height).to_image();
            let crop_mask = crop_tile_mask(&pending_mask, tile);
            let (crop_image, crop_mask) = downscale_tile(crop_image, crop_mask, geometry.scale);

            let generated = inference(&DynamicImage::ImageRgb8(crop_image), &crop_mask)?;
            let generated = if generated.dimensions() == (crop_width, crop_height) {
                generated.to_rgb8()
            } else {
                generated
                    .resize_exact(
                        crop_width,
                        crop_height,
                        image::imageops::FilterType::Lanczos3,
                    )
                    .to_rgb8()
            };
            composite_generated(&mut output, &component_labels, tile, &generated);
        }
    }
    Ok(DynamicImage::ImageRgb8(output))
}

/// Groups mask components by how far each must be shrunk for its thickest point to
/// stay within `MAX_HOLE_RADIUS`, in half-step scales from 1 up to the page scale.
fn group_by_hole_scale(
    mask: &GrayImage,
    labels: &ImageBuffer<Luma<u32>, Vec<u32>>,
    components: Vec<MaskComponent>,
    page_scale: f32,
) -> Vec<(f32, Vec<MaskComponent>)> {
    if page_scale <= 1.0 {
        return vec![(1.0, components)];
    }
    // Distance from each masked pixel to the nearest unmasked one.
    let unmasked = GrayImage::from_fn(mask.width(), mask.height(), |x, y| {
        Luma([u8::from(mask.get_pixel(x, y)[0] < 127) * u8::MAX])
    });
    let distances = distance_transform(&unmasked, Norm::LInf);
    let mut radii = vec![
        0u8;
        components
            .iter()
            .map(|c| c.label as usize + 1)
            .max()
            .unwrap_or(0)
    ];
    for (label, distance) in labels.pixels().zip(distances.pixels()) {
        if let Some(radius) = radii.get_mut(label[0] as usize) {
            *radius = (*radius).max(distance[0]);
        }
    }
    let mut groups = BTreeMap::<u32, Vec<MaskComponent>>::new();
    for component in components {
        let radius = f32::from(radii[component.label as usize]);
        let scale = (radius / MAX_HOLE_RADIUS).clamp(1.0, page_scale);
        let half_steps = ((scale * 2.0).ceil() as u32).min((page_scale * 2.0).ceil() as u32);
        groups.entry(half_steps).or_default().push(component);
    }
    groups
        .into_iter()
        .map(|(half_steps, components)| ((half_steps as f32 / 2.0).min(page_scale), components))
        .collect()
}

/// Flat-fills each remaining hole whose surroundings are one colour, such as text
/// on a plain background outside any detected bubble. Returns the holes still
/// left for the model.
fn fill_uniform_surroundings(
    output: &mut RgbImage,
    pending_mask: &mut GrayImage,
    labels: &ImageBuffer<Luma<u32>, Vec<u32>>,
    components: Vec<MaskComponent>,
    scale: f32,
) -> Vec<MaskComponent> {
    let (gap, reach) = ring_reach(scale);
    let (width, height) = pending_mask.dimensions();
    let mut remaining = Vec::new();
    for component in components {
        let [left, top, right, bottom] =
            expand_bounds(component.bounds, width, height, u32::from(reach));
        let hole = GrayImage::from_fn(right - left, bottom - top, |x, y| {
            Luma([u8::from(labels.get_pixel(left + x, top + y)[0] == component.label) * u8::MAX])
        });
        let ring = ring_around(&hole, gap, reach)
            .into_iter()
            .filter(|&(x, y)| pending_mask.get_pixel(left + x, top + y)[0] < 127)
            .map(|(x, y)| output.get_pixel(left + x, top + y).0)
            .collect();
        // Outside a bubble a thin line through the ring is artwork, not a stray
        // speck, so no outliers are dropped and every ring pixel has to be flat.
        let Some(color) = uniform_color(ring, false) else {
            remaining.push(component);
            continue;
        };
        let [left, top, right, bottom] = component.bounds;
        for y in top..bottom {
            for x in left..right {
                if labels.get_pixel(x, y)[0] == component.label {
                    output.put_pixel(x, y, color);
                    pending_mask.put_pixel(x, y, Luma([0]));
                }
            }
        }
    }
    remaining
}

/// The gap and reach, in page pixels, of the ring sampled around a hole.
fn ring_reach(scale: f32) -> (u8, u8) {
    let gap = (SURROUNDING_GAP * scale)
        .ceil()
        .min(f32::from(u8::MAX) / 2.0) as u8;
    let reach = (gap as f32 + (SURROUNDING_RING * scale).ceil()).min(f32::from(u8::MAX)) as u8;
    (gap, reach)
}

/// Pixels of `hole`'s image between `gap` and `reach` pixels from the hole.
fn ring_around(hole: &GrayImage, gap: u8, reach: u8) -> Vec<(u32, u32)> {
    let inner = dilate(hole, Norm::LInf, gap);
    let outer = dilate(hole, Norm::LInf, reach);
    outer
        .enumerate_pixels()
        .filter(|&(x, y, pixel)| pixel[0] != 0 && inner.get_pixel(x, y)[0] == 0)
        .map(|(x, y, _)| (x, y))
        .collect()
}

fn near_color(pixel: [u8; 3], color: [u8; 3]) -> bool {
    (0..3).all(|channel| pixel[channel].abs_diff(color[channel]) <= RING_PIXEL_TOLERANCE)
}

/// Shrinks a tile from page pixels to working pixels. Any mask coverage keeps a
/// working pixel masked, so thin strokes aren't dropped from the hole.
fn downscale_tile(image: RgbImage, mask: GrayImage, scale: f32) -> (RgbImage, GrayImage) {
    if scale <= 1.0 {
        return (image, mask);
    }
    let width = ((image.width() as f32 / scale).round() as u32).max(1);
    let height = ((image.height() as f32 / scale).round() as u32).max(1);
    let image =
        image::imageops::resize(&image, width, height, image::imageops::FilterType::Triangle);
    let mut mask =
        image::imageops::resize(&mask, width, height, image::imageops::FilterType::Triangle);
    for pixel in mask.pixels_mut() {
        pixel[0] = if pixel[0] > 0 { u8::MAX } else { 0 };
    }
    (image, mask)
}

fn fill_uniform_regions(
    output: &mut RgbImage,
    pending_mask: &mut GrayImage,
    text_mask: &GrayImage,
    regions: &[FlatFillRegion],
    scale: f32,
) {
    for region in regions {
        let [left, top, right, bottom] = region.bounds;
        let mut targets = Vec::new();
        for y in top..bottom {
            for x in left..right {
                if pending_mask.get_pixel(x, y)[0] >= 127
                    && text_mask.get_pixel(x, y)[0] >= 127
                    && point_in_polygon((x as f32 + 0.5, y as f32 + 0.5), &region.polygon)
                {
                    targets.push((x, y));
                }
            }
        }
        if targets.is_empty() {
            continue;
        }
        let Some(color) = uniform_region_color(output, pending_mask, region, scale) else {
            continue;
        };
        // The bubble's colour tolerates a few stray pixels, but the area right
        // around the text must not: a line running into the text is artwork.
        if !bubble_ring_is_flat(output, pending_mask, region, &targets, color, scale) {
            continue;
        }
        for (x, y) in targets {
            output.put_pixel(x, y, color);
            pending_mask.put_pixel(x, y, Luma([0]));
        }
    }
}

fn inpaint_tiles(
    labels: &ImageBuffer<Luma<u32>, Vec<u32>>,
    components: Vec<MaskComponent>,
    geometry: TileGeometry,
) -> Vec<InpaintTile> {
    let TileGeometry {
        size: tile_size,
        context,
        ..
    } = geometry;
    let mut bounded_tiles: Vec<InpaintTile> = Vec::new();
    let mut split_tiles = Vec::new();

    for component in components {
        let [left, top, right, bottom] = component.bounds;
        if right - left <= tile_size && bottom - top <= tile_size {
            let best = bounded_tiles
                .iter()
                .enumerate()
                .filter_map(|(index, tile)| {
                    let bounds = union_bounds(tile.core, component.bounds);
                    ((bounds[2] - bounds[0] <= tile_size) && (bounds[3] - bounds[1] <= tile_size))
                        .then_some((bounds_area(bounds) - bounds_area(tile.core), index, bounds))
                })
                .min_by_key(|(growth, index, _)| (*growth, *index));
            if let Some((_, index, bounds)) = best {
                let tile = &mut bounded_tiles[index];
                tile.components.push(component.label);
                tile.core = bounds;
                tile.crop = expand_bounds(bounds, labels.width(), labels.height(), context);
            } else {
                bounded_tiles.push(InpaintTile {
                    components: vec![component.label],
                    core: component.bounds,
                    crop: expand_bounds(component.bounds, labels.width(), labels.height(), context),
                });
            }
            continue;
        }

        let mut core_top = top;
        while core_top < bottom {
            let core_bottom = core_top.saturating_add(tile_size).min(bottom);
            let mut core_left = left;
            while core_left < right {
                let core_right = core_left.saturating_add(tile_size).min(right);
                let core = [core_left, core_top, core_right, core_bottom];
                if let Some(owned_bounds) = component_bounds_in(labels, component.label, core) {
                    split_tiles.push(InpaintTile {
                        components: vec![component.label],
                        core,
                        crop: expand_bounds(owned_bounds, labels.width(), labels.height(), context),
                    });
                }
                core_left = core_right;
            }
            core_top = core_bottom;
        }
    }

    bounded_tiles.append(&mut split_tiles);
    bounded_tiles.sort_by_key(|tile| (tile.core[1], tile.core[0], tile.core[3], tile.core[2]));
    bounded_tiles
}

fn mask_components(mask: &GrayImage) -> (ImageBuffer<Luma<u32>, Vec<u32>>, Vec<MaskComponent>) {
    let binary = GrayImage::from_fn(mask.width(), mask.height(), |x, y| {
        Luma([(mask.get_pixel(x, y)[0] >= 127) as u8 * u8::MAX])
    });
    let labels = connected_components(&binary, Connectivity::Eight, Luma([0]));
    let mut bounds = Vec::<Option<[u32; 4]>>::new();
    for (x, y, pixel) in labels.enumerate_pixels() {
        let label = pixel[0] as usize;
        if label == 0 {
            continue;
        }
        if label >= bounds.len() {
            bounds.resize(label + 1, None);
        }
        if let Some(bounds) = &mut bounds[label] {
            bounds[0] = bounds[0].min(x);
            bounds[1] = bounds[1].min(y);
            bounds[2] = bounds[2].max(x + 1);
            bounds[3] = bounds[3].max(y + 1);
        } else {
            bounds[label] = Some([x, y, x + 1, y + 1]);
        }
    }
    let components = bounds
        .into_iter()
        .enumerate()
        .skip(1)
        .filter_map(|(label, bounds)| {
            bounds.map(|bounds| MaskComponent {
                label: label as u32,
                bounds,
            })
        })
        .collect();
    (labels, components)
}

fn union_bounds(left: [u32; 4], right: [u32; 4]) -> [u32; 4] {
    [
        left[0].min(right[0]),
        left[1].min(right[1]),
        left[2].max(right[2]),
        left[3].max(right[3]),
    ]
}

fn bounds_area([left, top, right, bottom]: [u32; 4]) -> u64 {
    u64::from(right - left) * u64::from(bottom - top)
}

fn expand_bounds(
    [left, top, right, bottom]: [u32; 4],
    width: u32,
    height: u32,
    context: u32,
) -> [u32; 4] {
    [
        left.saturating_sub(context),
        top.saturating_sub(context),
        right.saturating_add(context).min(width),
        bottom.saturating_add(context).min(height),
    ]
}

fn component_bounds_in(
    labels: &ImageBuffer<Luma<u32>, Vec<u32>>,
    label: u32,
    [region_left, region_top, region_right, region_bottom]: [u32; 4],
) -> Option<[u32; 4]> {
    let mut left = region_right;
    let mut top = region_bottom;
    let mut right = 0;
    let mut bottom = 0;
    for y in region_top..region_bottom {
        for x in region_left..region_right {
            if labels.get_pixel(x, y)[0] == label {
                left = left.min(x);
                top = top.min(y);
                right = right.max(x + 1);
                bottom = bottom.max(y + 1);
            }
        }
    }
    (right > left && bottom > top).then_some([left, top, right, bottom])
}

fn crop_tile_mask(mask: &GrayImage, tile: &InpaintTile) -> GrayImage {
    let [left, top, right, bottom] = tile.crop;
    let mut crop = GrayImage::new(right - left, bottom - top);
    for y in top..bottom {
        for x in left..right {
            if mask.get_pixel(x, y)[0] >= 127 {
                crop.put_pixel(x - left, y - top, Luma([u8::MAX]));
            }
        }
    }
    crop
}

fn bubble_ring_is_flat(
    image: &RgbImage,
    mask: &GrayImage,
    region: &FlatFillRegion,
    targets: &[(u32, u32)],
    color: Rgb<u8>,
    scale: f32,
) -> bool {
    let (gap, reach) = ring_reach(scale);
    let [left, top, right, bottom] = expand_bounds(
        region.bounds,
        image.width(),
        image.height(),
        u32::from(reach),
    );
    let mut hole = GrayImage::new(right - left, bottom - top);
    for &(x, y) in targets {
        hole.put_pixel(x - left, y - top, Luma([u8::MAX]));
    }
    let edge_margin = FLAT_FILL_EDGE_MARGIN * scale;
    ring_around(&hole, gap, reach).into_iter().all(|(x, y)| {
        let (x, y) = (left + x, top + y);
        let point = (x as f32 + 0.5, y as f32 + 0.5);
        // Only the part of the ring the bubble's colour was measured on counts.
        mask.get_pixel(x, y)[0] >= 127
            || !point_in_polygon(point, &region.polygon)
            || polygon_edge_distance_squared(point, &region.polygon) < edge_margin * edge_margin
            || near_color(image.get_pixel(x, y).0, color.0)
    })
}

fn uniform_region_color(
    image: &RgbImage,
    mask: &GrayImage,
    region: &FlatFillRegion,
    scale: f32,
) -> Option<Rgb<u8>> {
    let [left, top, right, bottom] = region.bounds;
    let edge_margin = FLAT_FILL_EDGE_MARGIN * scale;
    let mut pixels = Vec::new();
    for y in top..bottom {
        for x in left..right {
            let point = (x as f32 + 0.5, y as f32 + 0.5);
            if mask.get_pixel(x, y)[0] >= 127
                || !point_in_polygon(point, &region.polygon)
                || polygon_edge_distance_squared(point, &region.polygon) < edge_margin * edge_margin
            {
                continue;
            }
            pixels.push(image.get_pixel(x, y).0);
        }
    }
    uniform_color(pixels, true)
}

/// The median colour of `pixels` if they are flat enough to fill a hole with.
/// With `tolerate_outliers`, a small share of pixels far from it is ignored;
/// without, every pixel must be within `RING_PIXEL_TOLERANCE` of it.
fn uniform_color(mut pixels: Vec<[u8; 3]>, tolerate_outliers: bool) -> Option<Rgb<u8>> {
    if pixels.len() < UNIFORM_BACKGROUND_MIN_PIXELS {
        return None;
    }
    // A few stray pixels (a bit of outline inside the detected polygon, a speck)
    // shouldn't disqualify an otherwise flat area, so the spread is measured
    // after dropping a small share of pixels far from the median colour.
    let medians = std::array::from_fn::<_, 3, _>(|channel| {
        median(
            &mut pixels
                .iter()
                .map(|pixel| pixel[channel])
                .collect::<Vec<_>>(),
        )
    });
    if tolerate_outliers {
        let total = pixels.len();
        pixels.retain(|pixel| {
            (0..3).all(|channel| {
                pixel[channel].abs_diff(medians[channel]) <= UNIFORM_OUTLIER_DISTANCE
            })
        });
        if (total - pixels.len()) as f64 > total as f64 * UNIFORM_OUTLIER_SHARE {
            return None;
        }
    } else if !pixels.iter().all(|&pixel| near_color(pixel, medians)) {
        return None;
    }
    let channels: [Vec<u8>; 3] =
        std::array::from_fn(|channel| pixels.iter().map(|pixel| pixel[channel]).collect());
    let deviations = std::array::from_fn::<_, 3, _>(|channel| {
        standard_deviation(&channels[channel], f64::from(medians[channel]))
    });
    let mean_deviation = deviations.iter().sum::<f64>() / deviations.len() as f64;
    let channel_spread = (deviations
        .iter()
        .map(|deviation| (deviation - mean_deviation).powi(2))
        .sum::<f64>()
        / deviations.len() as f64)
        .sqrt();
    let threshold = if channel_spread > 1.0 { 7.0 } else { 10.0 };
    (deviations.iter().copied().fold(0.0, f64::max) < threshold).then_some(Rgb(medians))
}

fn point_in_polygon(point: (f32, f32), polygon: &[(f32, f32)]) -> bool {
    let mut inside = false;
    let mut previous = polygon[polygon.len() - 1];
    for &current in polygon {
        if (current.1 > point.1) != (previous.1 > point.1) {
            let intersection_x = (previous.0 - current.0) * (point.1 - current.1)
                / (previous.1 - current.1)
                + current.0;
            if point.0 < intersection_x {
                inside = !inside;
            }
        }
        previous = current;
    }
    inside
}

fn polygon_edge_distance_squared(point: (f32, f32), polygon: &[(f32, f32)]) -> f32 {
    let mut minimum = f32::INFINITY;
    let mut start = polygon[polygon.len() - 1];
    for &end in polygon {
        let segment = (end.0 - start.0, end.1 - start.1);
        let length_squared = segment.0 * segment.0 + segment.1 * segment.1;
        let projection = if length_squared > 0.0 {
            ((point.0 - start.0) * segment.0 + (point.1 - start.1) * segment.1) / length_squared
        } else {
            0.0
        }
        .clamp(0.0, 1.0);
        let closest = (
            start.0 + segment.0 * projection,
            start.1 + segment.1 * projection,
        );
        let dx = point.0 - closest.0;
        let dy = point.1 - closest.1;
        minimum = minimum.min(dx * dx + dy * dy);
        start = end;
    }
    minimum
}

fn median(values: &mut [u8]) -> u8 {
    values.sort_unstable();
    let middle = values.len() / 2;
    if values.len().is_multiple_of(2) {
        ((u16::from(values[middle - 1]) + u16::from(values[middle])) / 2) as u8
    } else {
        values[middle]
    }
}

fn standard_deviation(values: &[u8], center: f64) -> f64 {
    (values
        .iter()
        .map(|value| (f64::from(*value) - center).powi(2))
        .sum::<f64>()
        / values.len() as f64)
        .sqrt()
}

fn composite_generated(
    output: &mut RgbImage,
    labels: &ImageBuffer<Luma<u32>, Vec<u32>>,
    tile: &InpaintTile,
    generated: &RgbImage,
) {
    let [left, top, _, _] = tile.crop;
    let [core_left, core_top, core_right, core_bottom] = tile.core;
    for y in core_top..core_bottom {
        for x in core_left..core_right {
            if tile
                .components
                .binary_search(&labels.get_pixel(x, y)[0])
                .is_ok()
            {
                output.put_pixel(x, y, *generated.get_pixel(x - left, y - top));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn transient_inpainting_mask_replaces_persistent_page_masks() {
        let mut session = koharu_scene::Session::memory().await.unwrap();
        let mut page = None;
        let source = DynamicImage::new_rgb8(8, 8);
        let persistent = DynamicImage::ImageLuma8(GrayImage::from_pixel(8, 8, Luma([255])));
        let encode = |image: &DynamicImage| {
            let mut bytes = Cursor::new(Vec::new());
            image.write_to(&mut bytes, ImageFormat::Png).unwrap();
            Arc::<[u8]>::from(bytes.into_inner())
        };
        let patch = session
            .snapshot()
            .patch(|edit| {
                let id = edit.add_page(
                    koharu_scene::PageDraft::new("page", 8.0, 8.0),
                    koharu_scene::At::End,
                )?;
                for (role, image) in [("source", &source), ("text-mask", &persistent)] {
                    edit.set_asset(
                        id,
                        &AssetRole::new(role)?,
                        AssetInput::new(
                            encode(image),
                            "image/png",
                            AssetMetadata {
                                width: Some(8),
                                height: Some(8),
                                attributes: BTreeMap::new(),
                            },
                        ),
                    )?;
                }
                page = Some(id);
                Ok(())
            })
            .unwrap();
        let snapshot = session.commit(patch).await.unwrap().snapshot;
        let page = page.unwrap();
        let mut transient = GrayImage::new(8, 8);
        transient.put_pixel(3, 4, Luma([255]));
        let input = StageInput::new(
            snapshot,
            page,
            None,
            None,
            Arc::new(crate::ImageCache::default()),
            Some(crate::InpaintingMask {
                page,
                png: encode(&DynamicImage::ImageLuma8(transient)),
            }),
        );

        let prepared = prepare(&input).await.unwrap();
        assert_eq!(prepared.mask.get_pixel(3, 4), &Luma([255]));
        assert_eq!(prepared.mask.get_pixel(0, 0), &Luma([0]));
        assert!(prepared.text_mask.pixels().all(|pixel| pixel[0] == 0));
    }

    #[tokio::test]
    async fn automatic_inpainting_skips_committed_cleanup() {
        let mut session = koharu_scene::Session::memory().await.unwrap();
        let mut page = None;
        let patch = session
            .snapshot()
            .patch(|edit| {
                let id = edit.add_page(
                    koharu_scene::PageDraft::new("page", 8.0, 8.0),
                    koharu_scene::At::End,
                )?;
                let cleanup = edit.add_entity(id, At::Start)?;
                edit.set(
                    cleanup,
                    &RasterLayer {
                        origin: Origin::User,
                        name: "Cleanup".to_owned(),
                        kind: RasterLayerKind::Cleanup,
                    },
                )?;
                edit.set_asset(
                    cleanup,
                    &AssetRole::new("source")?,
                    AssetInput::new(
                        Arc::<[u8]>::from([0]),
                        "image/png",
                        AssetMetadata {
                            width: Some(8),
                            height: Some(8),
                            attributes: BTreeMap::new(),
                        },
                    ),
                )?;
                page = Some(id);
                Ok(())
            })
            .unwrap();
        let snapshot = session.commit(patch).await.unwrap().snapshot;
        let page = page.unwrap();
        let automatic = StageInput::new(
            snapshot.clone(),
            page,
            None,
            None,
            Arc::new(crate::ImageCache::default()),
            None,
        );
        let manual = StageInput::new(
            snapshot,
            page,
            None,
            None,
            Arc::new(crate::ImageCache::default()),
            Some(crate::InpaintingMask {
                page,
                png: Arc::<[u8]>::from([]),
            }),
        );
        let processor = Processor::new(InpaintingModel::LaMa {}, koharu_ml::Device::cpu()).unwrap();

        assert!(processor.skip(&automatic).unwrap());
        assert!(!processor.skip(&manual).unwrap());
    }

    fn rectangle_region([left, top, right, bottom]: [u32; 4]) -> FlatFillRegion {
        FlatFillRegion {
            bounds: [left, top, right, bottom],
            polygon: vec![
                (left as f32, top as f32),
                (right as f32, top as f32),
                (right as f32, bottom as f32),
                (left as f32, bottom as f32),
            ],
        }
    }

    #[test]
    fn uniform_mask_background_is_filled_without_inference() {
        let mut image = RgbImage::from_pixel(96, 96, Rgb([240, 241, 242]));
        let mut mask = GrayImage::new(96, 96);
        for y in 40..56 {
            for x in 32..64 {
                image.put_pixel(x, y, Rgb([20, 20, 20]));
                mask.put_pixel(x, y, Luma([u8::MAX]));
            }
        }
        let mut calls = 0;

        let output = inpaint_tiled(
            &DynamicImage::ImageRgb8(image),
            &mask,
            &mask,
            &[rectangle_region([0, 0, 96, 96])],
            |_, _| {
                calls += 1;
                Ok(DynamicImage::new_rgb8(1, 1))
            },
        )
        .unwrap()
        .to_rgb8();

        assert_eq!(calls, 0);
        assert_eq!(output.get_pixel(48, 48), &Rgb([240, 241, 242]));
    }

    #[test]
    fn uniform_bubbles_are_filled_independently_inside_one_textured_tile() {
        let mut image = RgbImage::new(200, 100);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            *pixel = if (x + y).is_multiple_of(2) {
                Rgb([20, 80, 140])
            } else {
                Rgb([220, 80, 120])
            };
        }
        for y in 10..90 {
            for x in 10..90 {
                image.put_pixel(x, y, Rgb([245, 245, 245]));
            }
            for x in 110..190 {
                image.put_pixel(x, y, Rgb([250, 240, 220]));
            }
        }
        let mut mask = GrayImage::new(200, 100);
        for y in 40..60 {
            for x in 30..60 {
                image.put_pixel(x, y, Rgb([10, 10, 10]));
                mask.put_pixel(x, y, Luma([u8::MAX]));
            }
            for x in 135..165 {
                image.put_pixel(x, y, Rgb([10, 10, 10]));
                mask.put_pixel(x, y, Luma([u8::MAX]));
            }
        }
        let mut calls = 0;

        let output = inpaint_tiled(
            &DynamicImage::ImageRgb8(image),
            &mask,
            &mask,
            &[
                rectangle_region([10, 10, 90, 90]),
                rectangle_region([110, 10, 190, 90]),
            ],
            |_, _| {
                calls += 1;
                Ok(DynamicImage::new_rgb8(1, 1))
            },
        )
        .unwrap()
        .to_rgb8();

        assert_eq!(calls, 0);
        assert_eq!(output.get_pixel(45, 50), &Rgb([245, 245, 245]));
        assert_eq!(output.get_pixel(150, 50), &Rgb([250, 240, 220]));
    }

    #[test]
    fn connected_mask_crossing_both_page_grid_axes_is_inferred_once() {
        let mut image = RgbImage::new(900, 900);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            *pixel = if (x + y).is_multiple_of(2) {
                Rgb([20, 40, 60])
            } else {
                Rgb([220, 200, 180])
            };
        }
        let mut mask = GrayImage::new(900, 900);
        for y in 500..525 {
            for x in 500..525 {
                mask.put_pixel(x, y, Luma([u8::MAX]));
            }
        }
        let mut calls = 0;

        let output = inpaint_tiled(
            &DynamicImage::ImageRgb8(image),
            &mask,
            &mask,
            &[],
            |tile, tile_mask| {
                calls += 1;
                assert_eq!(
                    tile_mask.pixels().filter(|pixel| pixel[0] >= 127).count(),
                    25 * 25
                );
                Ok(DynamicImage::ImageRgb8(RgbImage::from_pixel(
                    tile.width(),
                    tile.height(),
                    Rgb([1, 2, 3]),
                )))
            },
        )
        .unwrap()
        .to_rgb8();

        assert_eq!(calls, 1);
        assert_eq!(output.get_pixel(500, 500), &Rgb([1, 2, 3]));
        assert_eq!(output.get_pixel(524, 524), &Rgb([1, 2, 3]));
        assert_eq!(output.get_pixel(499, 499), &Rgb([20, 40, 60]));
    }

    #[test]
    fn oversized_component_tiles_mask_their_continuation_context() {
        let mut mask = GrayImage::new(1000, 300);
        for y in 100..120 {
            for x in 50..850 {
                mask.put_pixel(x, y, Luma([u8::MAX]));
            }
        }

        let geometry = TileGeometry::for_page(mask.width(), mask.height());
        let (labels, components) = mask_components(&mask);
        let tiles = inpaint_tiles(&labels, components, geometry);

        assert_eq!(tiles.len(), 2);
        for tile in &tiles {
            assert!(tile.crop[2] - tile.crop[0] <= TILE_SIZE + TILE_CONTEXT * 2);
            assert!(tile.crop[3] - tile.crop[1] <= TILE_SIZE + TILE_CONTEXT * 2);
            let label = tile.components[0];
            let [left, top, right, bottom] = tile.core;
            let mut owned = 0;
            for y in top..bottom {
                for x in left..right {
                    if labels.get_pixel(x, y)[0] == label {
                        owned += 1;
                    }
                }
            }
            let masked = crop_tile_mask(&mask, tile)
                .pixels()
                .filter(|pixel| pixel[0] >= 127)
                .count();
            assert!(masked > owned);
        }
    }

    fn bubble_with_soft_edge(width: u32, height: u32) -> (RgbImage, GrayImage) {
        // A white bubble whose outline fades through an 8 px grey band, as a large
        // scan's antialiased outline does, with one dark glyph in the middle.
        let mut image = RgbImage::from_pixel(width, height, Rgb([30, 30, 30]));
        for y in 100..400 {
            for x in 100..400 {
                let edge = (x - 100).min(399 - x).min(y - 100).min(399 - y);
                let pixel = if edge < 8 {
                    Rgb([128, 128, 128])
                } else {
                    Rgb([255, 255, 255])
                };
                image.put_pixel(x, y, pixel);
            }
        }
        let mut mask = GrayImage::new(width, height);
        for y in 200..300 {
            for x in 220..280 {
                image.put_pixel(x, y, Rgb([10, 10, 10]));
                mask.put_pixel(x, y, Luma([u8::MAX]));
            }
        }
        (image, mask)
    }

    #[test]
    fn soft_bubble_edges_grow_with_the_page() {
        for (width, filled) in [(512, false), (6144, true)] {
            let (image, mask) = bubble_with_soft_edge(width, 512);
            let mut output = image;
            let mut pending = mask.clone();
            let scale = TileGeometry::for_page(width, 512).scale;

            fill_uniform_regions(
                &mut output,
                &mut pending,
                &mask,
                &[rectangle_region([100, 100, 400, 400])],
                scale,
            );

            assert_eq!(
                pending.get_pixel(250, 250)[0] == 0,
                filled,
                "page width {width}"
            );
            if filled {
                assert_eq!(output.get_pixel(250, 250), &Rgb([255, 255, 255]));
            }
        }
    }

    #[test]
    fn text_on_a_plain_background_is_filled_without_a_bubble() {
        let mut image = RgbImage::from_pixel(300, 300, Rgb([12, 12, 12]));
        let mut mask = GrayImage::new(300, 300);
        for y in 100..160 {
            for x in 120..140 {
                image.put_pixel(x, y, Rgb([250, 250, 250]));
                mask.put_pixel(x, y, Luma([u8::MAX]));
            }
            // An antialiased glyph edge just outside the mask is skipped.
            image.put_pixel(119, y, Rgb([128, 128, 128]));
            image.put_pixel(140, y, Rgb([128, 128, 128]));
        }
        let mut calls = 0;

        let output = inpaint_tiled(
            &DynamicImage::ImageRgb8(image),
            &mask,
            &mask,
            &[],
            |_, _| {
                calls += 1;
                Ok(DynamicImage::new_rgb8(1, 1))
            },
        )
        .unwrap()
        .to_rgb8();

        assert_eq!(calls, 0);
        assert_eq!(output.get_pixel(130, 130), &Rgb([12, 12, 12]));
    }

    #[test]
    fn thin_lines_through_plain_surroundings_are_not_erased() {
        let mut image = RgbImage::from_pixel(300, 300, Rgb([250, 250, 250]));
        let mut mask = GrayImage::new(300, 300);
        // A one-pixel rule crosses the text; it is under 1% of the sampled ring.
        for y in 0..300 {
            image.put_pixel(150, y, Rgb([0, 0, 0]));
        }
        for y in 120..180 {
            for x in 130..170 {
                mask.put_pixel(x, y, Luma([u8::MAX]));
            }
        }
        let mut calls = 0;

        let output = inpaint_tiled(
            &DynamicImage::ImageRgb8(image),
            &mask,
            &mask,
            &[],
            |image, _| {
                calls += 1;
                Ok(image.clone())
            },
        )
        .unwrap()
        .to_rgb8();

        assert_eq!(calls, 1);
        assert_eq!(output.get_pixel(150, 150), &Rgb([0, 0, 0]));
    }

    #[test]
    fn bubble_flat_fill_does_not_break_a_continuous_rule() {
        let mut image = RgbImage::from_pixel(300, 300, Rgb([250, 250, 250]));
        let mut mask = GrayImage::new(300, 300);
        for y in 0..300 {
            image.put_pixel(150, y, Rgb([0, 0, 0]));
        }
        for y in 120..180 {
            for x in 130..170 {
                mask.put_pixel(x, y, Luma([u8::MAX]));
            }
        }
        let mut calls = 0;

        let output = inpaint_tiled(
            &DynamicImage::ImageRgb8(image),
            &mask,
            &mask,
            &[rectangle_region([0, 0, 300, 300])],
            |image, _| {
                calls += 1;
                Ok(image.clone())
            },
        )
        .unwrap()
        .to_rgb8();

        assert_eq!(calls, 1);
        for y in 120..180 {
            assert_eq!(output.get_pixel(150, y), &Rgb([0, 0, 0]));
        }
    }

    fn sparse_crossing_rule(rule: Rgb<u8>, transpose: bool) {
        let (width, height) = if transpose { (1400, 200) } else { (200, 1400) };
        let mut image = RgbImage::from_pixel(width, height, Rgb([250, 250, 250]));
        let mut mask = GrayImage::new(width, height);
        // A long, narrow hole: the rule is under 0.1% of its ring, too little to
        // move the ring's spread past the uniform threshold.
        let at = |along: u32, across: u32| {
            if transpose {
                (along, across)
            } else {
                (across, along)
            }
        };
        for across in 0..200 {
            let (x, y) = at(700, across);
            image.put_pixel(x, y, rule);
        }
        for along in 100..1300 {
            for across in 90..110 {
                let (x, y) = at(along, across);
                mask.put_pixel(x, y, Luma([u8::MAX]));
            }
        }
        let mut calls = 0;

        let output = inpaint_tiled(
            &DynamicImage::ImageRgb8(image),
            &mask,
            &mask,
            &[],
            |image, _| {
                calls += 1;
                Ok(image.clone())
            },
        )
        .unwrap()
        .to_rgb8();

        // The 1200 px hole is longer than a tile, so it reaches the model in pieces.
        assert!(calls > 0);
        for across in 90..110 {
            let (x, y) = at(700, across);
            assert_eq!(output.get_pixel(x, y), &rule);
        }
    }

    #[test]
    fn sparse_crossing_rule_is_not_hidden_by_ring_averaging() {
        sparse_crossing_rule(Rgb([0, 0, 0]), false);
        sparse_crossing_rule(Rgb([0, 0, 0]), true);
    }

    #[test]
    fn sparse_grey_rule_is_not_hidden_by_ring_averaging() {
        sparse_crossing_rule(Rgb([160, 160, 160]), false);
        sparse_crossing_rule(Rgb([160, 160, 160]), true);
    }

    #[test]
    fn stray_dark_pixels_do_not_block_a_flat_fill() {
        let mut image = RgbImage::from_pixel(300, 300, Rgb([250, 250, 250]));
        let mut mask = GrayImage::new(300, 300);
        // About 1% of the bubble is a dark speck, e.g. outline caught inside the
        // detected polygon. Its spread alone (~25) would fail the uniform check.
        for y in 20..50 {
            for x in 20..50 {
                image.put_pixel(x, y, Rgb([0, 0, 0]));
            }
        }
        for y in 120..180 {
            for x in 130..170 {
                image.put_pixel(x, y, Rgb([10, 10, 10]));
                mask.put_pixel(x, y, Luma([u8::MAX]));
            }
        }
        let mut calls = 0;

        let output = inpaint_tiled(
            &DynamicImage::ImageRgb8(image),
            &mask,
            &mask,
            &[rectangle_region([0, 0, 300, 300])],
            |_, _| {
                calls += 1;
                Ok(DynamicImage::new_rgb8(1, 1))
            },
        )
        .unwrap()
        .to_rgb8();

        assert_eq!(calls, 0);
        assert_eq!(output.get_pixel(150, 150), &Rgb([250, 250, 250]));
        assert_eq!(output.get_pixel(30, 30), &Rgb([0, 0, 0]));
    }

    #[test]
    fn large_pages_shrink_only_thick_holes() {
        let geometry = TileGeometry::for_page(4096, 1024);
        assert_eq!(
            geometry,
            TileGeometry {
                scale: 2.0,
                size: TILE_SIZE * 2,
                context: TILE_CONTEXT * 2,
            }
        );
        assert_eq!(TileGeometry::for_page(1500, 2048).scale, 1.0);

        let mut image = RgbImage::new(4096, 1024);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            *pixel = if (x + y).is_multiple_of(2) {
                Rgb([0, 40, 80])
            } else {
                Rgb([255, 215, 175])
            };
        }
        let original = image.clone();
        let mut mask = GrayImage::new(4096, 1024);
        // A thick hole (about 50 px from its centre to an edge) and a thin stroke.
        for y in 400..600 {
            for x in 2000..2101 {
                mask.put_pixel(x, y, Luma([u8::MAX]));
            }
            for x in 2600..2604 {
                mask.put_pixel(x, y, Luma([u8::MAX]));
            }
        }
        let mut tiles = Vec::new();

        let output = inpaint_tiled(
            &DynamicImage::ImageRgb8(image),
            &mask,
            &mask,
            &[],
            |tile, tile_mask| {
                assert_eq!(tile.dimensions(), tile_mask.dimensions());
                tiles.push(tile.dimensions());
                Ok(DynamicImage::ImageRgb8(RgbImage::from_pixel(
                    tile.width(),
                    tile.height(),
                    Rgb([1, 2, 3]),
                )))
            },
        )
        .unwrap()
        .to_rgb8();

        tiles.sort_unstable();
        // The stroke keeps full resolution with ordinary context; the thick hole's
        // 613x712 page-pixel crop reaches the model at half size.
        assert_eq!(
            tiles,
            [(4 + TILE_CONTEXT * 2, 200 + TILE_CONTEXT * 2), (307, 356)]
        );
        assert_eq!(output.get_pixel(2050, 500), &Rgb([1, 2, 3]));
        assert_eq!(output.get_pixel(2601, 500), &Rgb([1, 2, 3]));
        assert_eq!(output.get_pixel(2300, 500), original.get_pixel(2300, 500));
        assert_eq!(output.get_pixel(10, 10), original.get_pixel(10, 10));
    }

    #[test]
    fn downscaled_masks_keep_thin_strokes() {
        let image = RgbImage::new(100, 100);
        let mut mask = GrayImage::new(100, 100);
        for y in 0..100 {
            mask.put_pixel(51, y, Luma([u8::MAX]));
        }

        let (image, mask) = downscale_tile(image, mask, 4.0);

        assert_eq!(image.dimensions(), (25, 25));
        assert!(
            (0..25).all(|y| (0..25).any(|x| mask.get_pixel(x, y)[0] == u8::MAX)),
            "every row keeps part of the stroke"
        );
    }

    #[test]
    fn textured_mask_regions_are_inferred_as_bounded_tiles() {
        let mut image = RgbImage::new(1200, 700);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            *pixel = if (x + y).is_multiple_of(2) {
                Rgb([0, 40, 80])
            } else {
                Rgb([255, 215, 175])
            };
        }
        let original = image.clone();
        let mut mask = GrayImage::new(1200, 700);
        mask.put_pixel(10, 10, Luma([u8::MAX]));
        mask.put_pixel(1100, 600, Luma([u8::MAX]));
        let mut calls = 0;

        let output = inpaint_tiled(
            &DynamicImage::ImageRgb8(image),
            &mask,
            &mask,
            &[],
            |tile, _| {
                calls += 1;
                assert!(tile.width() <= TILE_CONTEXT * 2 + 1);
                assert!(tile.height() <= TILE_CONTEXT * 2 + 1);
                Ok(DynamicImage::ImageRgb8(RgbImage::from_pixel(
                    tile.width(),
                    tile.height(),
                    Rgb([1, 2, 3]),
                )))
            },
        )
        .unwrap()
        .to_rgb8();

        assert_eq!(calls, 2);
        assert_eq!(output.get_pixel(10, 10), &Rgb([1, 2, 3]));
        assert_eq!(output.get_pixel(1100, 600), &Rgb([1, 2, 3]));
        assert_eq!(output.get_pixel(500, 300), original.get_pixel(500, 300));
    }
}
