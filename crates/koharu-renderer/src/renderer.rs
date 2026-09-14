//! Renderer ownership, scene interpretation, resources, and retained updates.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    sync::{Arc, OnceLock, Weak},
};

use anyhow::{Context, anyhow};
use arc_swap::ArcSwap;
use futures::future::try_join_all;
use koharu_rasterizer::{RasterOptions, Rasterizer};
use koharu_scene::{
    Asset, AssetRole, BlobId, Change, Component, ComponentOwner, EntityChange, EntityId, FitsTo,
    FlowsIn, Geometry, Group, OcrAnalysis, Origin, Page, Presents, RasterLayer, RasterLayerKind,
    RecognizedFrom, Region, RelationChange, RelationId, RelationSpec, Revision, Snapshot,
    TextAlignment, TextDirection, TextLayout as SceneTextLayout, TextLayoutKind, Translation,
    Typography, Visibility,
};
use parking_lot::Mutex;
use rayon::prelude::*;
use skrifa::{
    GlyphId, MetadataProvider,
    instance::Size,
    outline::{DrawSettings, OutlinePen},
};
use tokio_rayon::AsyncThreadPool as _;
use vello::{
    Scene,
    kurbo::{Affine, BezPath, Rect, Vec2},
    peniko::Fill,
};

use crate::{
    Error, FontFamily, FontStyle, Frame, ImageKind, ImageMetadata, Layer, LayerKind, Presentation,
    RasterImage, RenderBounds, RenderDependency, RenderDiagnostic, Result, RetentionStats,
    TextAlign, TextMetadata, TypesettingConfig, WritingMode,
    bubble::{GeometryFrame, LayoutBox, contour, flow_cells, geometry_bounds, geometry_frame},
    fonts::{FontPreview, FontRequest, Fonts},
    frame::{
        FrameData, ImageNodeDescriptor, LayerData, LocalTextMetadata, NodeDescriptor, RetainedNode,
        prepare_frame,
    },
    images::{DecodedImage, ImageCache, decode},
    script::{is_chinese_or_japanese_text, shaping_direction_for_text},
    text_renderer::{StrokeOptions, TextNodeDescriptor, TextRenderer},
};

const MAX_SURFACE_DIMENSION: u32 = 32_768;
const MAX_SURFACE_PIXELS: u64 = 268_435_456;
const DEFAULT_RETAINED_NODES: usize = 2_048;
const MAX_RESOURCE_READS: usize = 8;
const ASSETS_KIND: &str = "dev.koharu.assets";
const MINIMUM_FONT_SIZE: f32 = 9.0;

#[derive(Clone)]
pub struct Renderer {
    inner: Arc<RendererInner>,
}

struct RendererInner {
    typesetting: Arc<ArcSwap<TypesettingConfig>>,
    fonts: Arc<Fonts>,
    images: Mutex<ImageCache>,
    image_loads: Mutex<HashMap<BlobId, Weak<ImageLoad>>>,
    nodes: Mutex<NodeCache>,
    workers: OnceLock<Arc<rayon::ThreadPool>>,
}

impl std::fmt::Debug for Renderer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Renderer").finish_non_exhaustive()
    }
}

impl Renderer {
    /// Creates a renderer without discovering fonts, reading blobs, or initializing a GPU.
    pub fn new() -> Result<Self> {
        Self::from_config(TypesettingConfig::load().map_err(Error::Backend)?)
    }

    pub fn from_config(config: koharu_config::Config<TypesettingConfig>) -> Result<Self> {
        let renderer = Self::with_typesetting(config.read().map_err(Error::Backend)?.clone());
        let watched = renderer.inner.typesetting.clone();
        let _watcher = tokio::runtime::Handle::try_current()
            .context("renderer requires a Tokio runtime")
            .map_err(Error::Backend)?
            .spawn(async move {
                let mut changes = config.subscribe();
                while changes.changed().await.is_ok() {
                    match config.read() {
                        Ok(config) => {
                            watched.store(Arc::new(config.clone()));
                        }
                        Err(error) => tracing::error!(%error, "failed to reload typesetting"),
                    }
                }
            });
        Ok(renderer)
    }

    fn with_typesetting(typesetting: TypesettingConfig) -> Self {
        Self {
            inner: Arc::new(RendererInner {
                typesetting: Arc::new(ArcSwap::from_pointee(typesetting)),
                fonts: Arc::new(Fonts::new()),
                images: Mutex::new(ImageCache::new()),
                image_loads: Mutex::new(HashMap::new()),
                nodes: Mutex::new(NodeCache::new(DEFAULT_RETAINED_NODES)),
                workers: OnceLock::new(),
            }),
        }
    }

    /// Completely renders one page, including resources needed by retained nodes.
    #[tracing::instrument(level = "info", skip_all, fields(page = %page, revision = %snapshot.revision()))]
    pub async fn render(&self, snapshot: &Snapshot, page: EntityId) -> Result<Frame> {
        let compiled = self.compile(snapshot, page)?;
        self.finish(snapshot, compiled, None, None).await
    }

    /// Renders a contiguous revision while retaining unchanged vector nodes.
    #[tracing::instrument(level = "info", skip_all, fields(page = %previous.page(), from = %change.from, to = %change.to))]
    pub async fn update(
        &self,
        previous: &Frame,
        snapshot: &Snapshot,
        change: &Change,
    ) -> Result<Frame> {
        if previous.revision() != change.from || snapshot.revision() != change.to {
            return Err(Error::invalid(format!(
                "renderer update is not contiguous: frame {}, change {} -> {}, snapshot {}",
                previous.revision(),
                change.from,
                change.to,
                snapshot.revision()
            )));
        }
        if change.from == change.to {
            return Ok(previous.clone());
        }
        let affected = AffectedDependencies::new(snapshot, change)?;
        if !affected.intersects(previous.dependencies()) {
            return Ok(previous.at_revision(
                change.to,
                RetentionStats {
                    candidate_layers: 0,
                    reused_layers: previous.layers().len(),
                    rebuilt_layers: 0,
                },
            ));
        }
        let compiled = self.compile(snapshot, previous.page())?;
        self.finish(snapshot, compiled, Some(previous), Some(&affected))
            .await
    }

    pub async fn available_fonts(&self) -> Result<Vec<FontFamily>> {
        self.inner
            .fonts
            .families()
            .await
            .map_err(Error::FontResource)
    }

    pub async fn font_preview(
        &self,
        family_name: &str,
        rasterizer: Arc<Rasterizer>,
    ) -> Result<Vec<u8>> {
        const FONT_SIZE: f32 = 24.0;
        const PREVIEW_HEIGHT: u32 = 96;

        let font = match self
            .inner
            .fonts
            .preview(family_name)
            .await
            .map_err(Error::FontResource)?
        {
            FontPreview::Webp(bytes) => return Ok(bytes),
            FontPreview::System(font) => *font,
        };
        let label = family_name.to_owned();
        self.inner
            .fonts
            .prepare(&[FontRequest {
                family: "Arial".to_owned(),
                weight: Some(400),
                style: Some(FontStyle::Normal),
            }])
            .await
            .map_err(Error::FontResource)?;
        let fonts = self.inner.fonts.clone();
        let (scene, width) = tokio_rayon::spawn(move || {
            let preview_fonts = if font.renders(&label, FONT_SIZE) {
                vec![font]
            } else {
                fonts.resolve(Some("Arial"), Some(400), None, &[], &label, None)?
            };
            let measured = crate::TextLayout::new(&preview_fonts[0])
                .with_fallback_fonts(&preview_fonts[1..])
                .with_font_size(FONT_SIZE)
                .run(&label)?;
            let size = FONT_SIZE * PREVIEW_HEIGHT as f32 / measured.height.max(1.0);
            let layout = crate::TextLayout::new(&preview_fonts[0])
                .with_fallback_fonts(&preview_fonts[1..])
                .with_font_size(size)
                .run(&label)?;
            let width = layout.width.ceil().max(1.0) as u32;
            let mut scene = Scene::new();
            draw_font_preview(&mut scene, &layout)?;
            Ok::<_, anyhow::Error>((scene, width))
        })
        .await
        .map_err(Error::FontResource)?;
        tokio_rayon::spawn(move || {
            let image = rasterizer
                .rasterize_scene(
                    &scene,
                    width,
                    PREVIEW_HEIGHT,
                    [0, 0, 0, 0],
                    RasterOptions::default(),
                )
                .context("failed to rasterize the font preview")?;
            Ok::<_, anyhow::Error>(
                webp::Encoder::from_rgba(image.as_raw(), image.width(), image.height())
                    .encode(85.0)
                    .to_vec(),
            )
        })
        .await
        .map_err(Error::FontResource)
    }

    /// Discards retained Vello nodes after their presentation resource lifetime ends.
    pub fn discard_retained_nodes(&self) {
        self.inner.nodes.lock().entries.clear();
    }

    async fn finish(
        &self,
        snapshot: &Snapshot,
        compiled: CompiledPage,
        previous: Option<&Frame>,
        affected: Option<&AffectedDependencies>,
    ) -> Result<Frame> {
        let mut nodes = vec![None; compiled.layers.len()];
        let mut pending = Vec::new();
        let mut reused = 0;
        let mut candidates = 0;
        let clock = {
            let mut node_cache = self.inner.nodes.lock();
            let clock = node_cache.next_clock();

            for (index, draft) in compiled.layers.iter().enumerate() {
                let candidate = previous.is_none()
                    || previous
                        .and_then(|frame| frame.layer(draft.entity))
                        .is_none()
                    || affected.is_some_and(|affected| affected.intersects(&draft.dependencies));
                candidates += usize::from(candidate);
                let previous_node = previous
                    .and_then(|frame| frame.layer(draft.entity))
                    .map(|layer| &layer.0.node)
                    .filter(|node| node.descriptor == draft.descriptor);
                if let Some(node) = previous_node {
                    nodes[index] = Some(node.clone());
                    reused += 1;
                    continue;
                }
                if let Some(node) =
                    node_cache.get((compiled.page, draft.entity), &draft.descriptor, clock)
                {
                    nodes[index] = Some(node);
                    reused += 1;
                } else {
                    pending.push((index, draft.descriptor.clone()));
                }
            }
            clock
        };

        let font_requests = font_requests(&pending);
        self.inner
            .fonts
            .prepare(&font_requests)
            .await
            .map_err(Error::FontResource)?;
        let image_ids = pending
            .iter()
            .filter_map(|(_, descriptor)| match descriptor {
                NodeDescriptor::Image(image) => Some(image.blob),
                NodeDescriptor::Text(_) => None,
            })
            .collect::<BTreeSet<_>>();
        let images = Arc::new(self.load_images(snapshot, image_ids).await?);
        if !pending.is_empty() {
            let workers = self.workers()?;
            let fonts = self.inner.fonts.clone();
            let built = workers
                .spawn_async(move || {
                    pending
                        .into_par_iter()
                        .map(|(index, descriptor)| {
                            build_node(descriptor, &fonts, &images).map(|node| (index, node))
                        })
                        .collect::<Result<Vec<_>>>()
                })
                .await?;
            for (index, node) in built {
                let node = Arc::new(node);
                self.inner.nodes.lock().insert(
                    (compiled.page, compiled.layers[index].entity),
                    node.clone(),
                    clock,
                );
                nodes[index] = Some(node);
            }
        }

        let rebuilt = compiled.layers.len().saturating_sub(reused);
        let nodes = nodes
            .into_iter()
            .map(|node| node.expect("every retained node must be resolved"))
            .collect();
        let stats = RetentionStats {
            candidate_layers: candidates,
            reused_layers: reused,
            rebuilt_layers: rebuilt,
        };
        let workers = self.workers()?;
        workers
            .spawn_async(move || assemble_frame(compiled, nodes, stats))
            .await
    }

    fn workers(&self) -> Result<Arc<rayon::ThreadPool>> {
        if let Some(workers) = self.inner.workers.get() {
            return Ok(workers.clone());
        }
        let threads = std::thread::available_parallelism()
            .map_or(2, usize::from)
            .clamp(2, 8);
        let workers = Arc::new(
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .thread_name(|index| format!("koharu-render-{index}"))
                .build()
                .map_err(|source| Error::Backend(anyhow!(source)))?,
        );
        let _ = self.inner.workers.set(workers);
        Ok(self
            .inner
            .workers
            .get()
            .expect("renderer worker pool was initialized")
            .clone())
    }

    async fn load_images(
        &self,
        snapshot: &Snapshot,
        ids: BTreeSet<BlobId>,
    ) -> Result<HashMap<BlobId, Arc<DecodedImage>>> {
        let mut output = HashMap::with_capacity(ids.len());
        let mut missing = Vec::new();
        {
            let mut cache = self.inner.images.lock();
            for id in ids {
                if let Some(image) = cache.get(id) {
                    output.insert(id, image);
                } else {
                    missing.push(id);
                }
            }
        }
        for chunk in missing.chunks(MAX_RESOURCE_READS) {
            let loads = try_join_all(chunk.iter().map(|&id| self.load_image(snapshot, id))).await?;
            for (id, image) in loads {
                output.insert(id, image);
            }
        }
        Ok(output)
    }

    async fn load_image(
        &self,
        snapshot: &Snapshot,
        id: BlobId,
    ) -> Result<(BlobId, Arc<DecodedImage>)> {
        let load = {
            let mut loads = self.inner.image_loads.lock();
            if let Some(load) = loads.get(&id).and_then(Weak::upgrade) {
                load
            } else {
                let load = Arc::new(ImageLoad {
                    lock: tokio::sync::Mutex::new(()),
                    image: Mutex::new(Weak::new()),
                });
                loads.insert(id, Arc::downgrade(&load));
                load
            }
        };
        let guard = load.lock.lock().await;
        if let Some(image) = self.inner.images.lock().get(id) {
            drop(guard);
            self.release_image_load(id, &load);
            return Ok((id, image));
        }
        if let Some(image) = load.image.lock().upgrade() {
            drop(guard);
            self.release_image_load(id, &load);
            return Ok((id, image));
        }
        let result = async {
            let bytes = snapshot.read_blob(id).await?;
            let bytes: Arc<[u8]> = Arc::from(bytes.as_ref());
            let workers = self.workers()?;
            let (id, image) = workers.spawn_async(move || decode(id, bytes, None)).await?;
            self.inner.images.lock().insert(id, image.clone());
            *load.image.lock() = Arc::downgrade(&image);
            Ok((id, image))
        }
        .await;
        drop(guard);
        self.release_image_load(id, &load);
        result
    }

    fn release_image_load(&self, id: BlobId, load: &Arc<ImageLoad>) {
        if Arc::strong_count(load) == 1 {
            let mut loads = self.inner.image_loads.lock();
            if loads
                .get(&id)
                .is_some_and(|current| current.ptr_eq(&Arc::downgrade(load)))
            {
                loads.remove(&id);
            }
        }
    }
}

impl Default for Renderer {
    fn default() -> Self {
        Self::with_typesetting(TypesettingConfig::default())
    }
}

struct ImageLoad {
    lock: tokio::sync::Mutex<()>,
    image: Mutex<Weak<DecodedImage>>,
}

#[derive(Clone)]
struct LayerDraft {
    entity: EntityId,
    geometry: Geometry,
    frame: GeometryFrame,
    presentation: Presentation,
    ancestry: Arc<[EntityId]>,
    descriptor: NodeDescriptor,
    metadata: DraftMetadata,
    dependencies: Arc<[RenderDependency]>,
}

#[derive(Clone)]
enum DraftMetadata {
    Image(ImageMetadata),
    Text {
        text: String,
        language: Option<koharu_scene::LanguageTag>,
        alignment: TextAlign,
        writing_mode: WritingMode,
        angle_degrees: f32,
    },
}

struct CompiledPage {
    revision: Revision,
    page: EntityId,
    width: u32,
    height: u32,
    layers: Vec<LayerDraft>,
    dependencies: Arc<[RenderDependency]>,
    diagnostics: Vec<RenderDiagnostic>,
}

impl Renderer {
    fn compile(&self, snapshot: &Snapshot, page: EntityId) -> Result<CompiledPage> {
        let typesetting = self.inner.typesetting.load();
        let page_value = snapshot.page(page)?.page()?;
        let (width, height) = surface_size(&page_value)?;
        let source_role = AssetRole::new("source")?;
        let flow_plan = resolve_balloon_flows(snapshot, page)?;
        let mut traversal = Traversal {
            snapshot,
            page,
            width,
            height,
            source_role: &source_role,
            font_families: &typesetting.font_families,
            balloon_flows: flow_plan.placements,
            layers: Vec::new(),
            dependencies: BTreeSet::from([
                RenderDependency::Entity(page),
                RenderDependency::Hierarchy(page),
                component_dependency::<Page>(page),
                RenderDependency::Component {
                    entity: page,
                    kind: ASSETS_KIND.to_owned(),
                },
            ]),
            diagnostics: Vec::new(),
        };
        traversal.dependencies.extend(flow_plan.dependencies);
        if let Some(asset) = snapshot.asset(page, &source_role)? {
            let geometry = Geometry::rectangle(0.0, 0.0, f64::from(width), f64::from(height));
            let mut dependencies = BTreeSet::from([
                RenderDependency::Entity(page),
                component_dependency::<Page>(page),
                RenderDependency::Component {
                    entity: page,
                    kind: ASSETS_KIND.to_owned(),
                },
                RenderDependency::Blob(asset.blob),
            ]);
            traversal.dependencies.extend(dependencies.iter().cloned());
            traversal.layers.push(image_draft(
                page,
                geometry,
                ImageNodeDescriptor::from_asset(asset, Some((width, height))),
                ImageMetadata {
                    name: None,
                    kind: ImageKind::Source,
                },
                Presentation {
                    visible: true,
                    opacity: 1.0,
                },
                Arc::from([]),
                &mut dependencies,
            )?);
        } else {
            traversal.diagnostics.push(RenderDiagnostic::MissingAsset {
                entity: page,
                role: source_role.as_str().to_owned(),
            });
        }
        traversal.visit_children(
            page,
            Presentation {
                visible: true,
                opacity: 1.0,
            },
            &[],
        )?;
        Ok(CompiledPage {
            revision: snapshot.revision(),
            page,
            width,
            height,
            layers: traversal.layers,
            dependencies: traversal.dependencies.into_iter().collect(),
            diagnostics: traversal.diagnostics,
        })
    }
}

struct Traversal<'a> {
    snapshot: &'a Snapshot,
    page: EntityId,
    width: u32,
    height: u32,
    source_role: &'a AssetRole,
    font_families: &'a [String],
    balloon_flows: HashMap<EntityId, ResolvedPlacement>,
    layers: Vec<LayerDraft>,
    dependencies: BTreeSet<RenderDependency>,
    diagnostics: Vec<RenderDiagnostic>,
}

impl Traversal<'_> {
    fn visit_children(
        &mut self,
        parent: EntityId,
        inherited: Presentation,
        ancestry: &[EntityId],
    ) -> Result<()> {
        self.dependencies
            .insert(RenderDependency::Hierarchy(parent));
        let children = self.snapshot.children(parent)?.collect::<Vec<_>>();
        for entity in children {
            let mut common = BTreeSet::from([
                RenderDependency::Entity(entity),
                RenderDependency::Hierarchy(parent),
                RenderDependency::Hierarchy(entity),
                component_dependency::<Visibility>(entity),
            ]);
            let visibility = self
                .snapshot
                .component::<Visibility>(entity)?
                .unwrap_or(Visibility {
                    origin: Origin::User,
                    visible: true,
                    opacity: 1.0,
                });
            let presentation = Presentation {
                visible: inherited.visible && visibility.visible,
                opacity: inherited.opacity * visibility.opacity,
            };
            self.dependencies.extend(common.iter().cloned());
            if self.snapshot.component::<Group>(entity)?.is_some() {
                common.insert(component_dependency::<Group>(entity));
                self.dependencies.extend(common);
                let mut nested = ancestry.to_vec();
                nested.push(entity);
                self.visit_children(entity, presentation, &nested)?;
                continue;
            }
            if let Some(raster) = self.snapshot.component::<RasterLayer>(entity)? {
                common.insert(component_dependency::<RasterLayer>(entity));
                common.insert(RenderDependency::Component {
                    entity,
                    kind: ASSETS_KIND.to_owned(),
                });
                if let Some(asset) = self.snapshot.asset(entity, self.source_role)? {
                    common.insert(RenderDependency::Blob(asset.blob));
                    let geometry = Geometry::rectangle(
                        0.0,
                        0.0,
                        f64::from(self.width),
                        f64::from(self.height),
                    );
                    let kind = match raster.kind {
                        RasterLayerKind::Cleanup => ImageKind::Cleanup,
                        RasterLayerKind::Paint => ImageKind::Paint,
                    };
                    self.layers.push(image_draft(
                        entity,
                        geometry,
                        ImageNodeDescriptor::from_asset(asset, None),
                        ImageMetadata {
                            name: Some(raster.name),
                            kind,
                        },
                        presentation,
                        Arc::from(ancestry),
                        &mut common,
                    )?);
                } else {
                    self.diagnostics.push(RenderDiagnostic::MissingAsset {
                        entity,
                        role: self.source_role.as_str().to_owned(),
                    });
                }
                self.dependencies.extend(common);
                continue;
            }
            if let Some(layout) = self.snapshot.component::<SceneTextLayout>(entity)? {
                common.insert(component_dependency::<SceneTextLayout>(entity));
                common.insert(component_dependency::<Typography>(entity));
                common.insert(component_dependency::<Geometry>(entity));
                for kind in [Presents::KIND, FitsTo::KIND, FlowsIn::KIND] {
                    common.insert(RenderDependency::RelationQuery {
                        source: entity,
                        kind: kind.to_owned(),
                    });
                }
                if let Some(draft) = self.text_draft(
                    entity,
                    layout,
                    presentation,
                    Arc::from(ancestry),
                    &mut common,
                )? {
                    self.layers.push(draft);
                }
                self.dependencies.extend(common);
                continue;
            }
            if self.snapshot.component::<Region>(entity)?.is_some() {
                common.insert(component_dependency::<Region>(entity));
                self.dependencies.extend(common);
                continue;
            }
            common.insert(component_dependency::<Geometry>(entity));
            common.insert(RenderDependency::Component {
                entity,
                kind: ASSETS_KIND.to_owned(),
            });
            let geometry = self.snapshot.component::<Geometry>(entity)?;
            let asset = self.snapshot.asset(entity, self.source_role)?;
            if let (Some(geometry), Some(asset)) = (geometry, asset) {
                common.insert(RenderDependency::Blob(asset.blob));
                self.layers.push(image_draft(
                    entity,
                    geometry,
                    ImageNodeDescriptor::from_asset(asset, None),
                    ImageMetadata {
                        name: None,
                        kind: ImageKind::Embedded,
                    },
                    presentation,
                    Arc::from(ancestry),
                    &mut common,
                )?);
            }
            self.dependencies.extend(common);
        }
        Ok(())
    }

    fn text_draft(
        &mut self,
        entity: EntityId,
        layout: SceneTextLayout,
        presentation: Presentation,
        ancestry: Arc<[EntityId]>,
        dependencies: &mut BTreeSet<RenderDependency>,
    ) -> Result<Option<LayerDraft>> {
        let Some(presents) = self.snapshot.relation_from::<Presents>(entity)? else {
            return Ok(None);
        };
        dependencies.insert(RenderDependency::Relation(presents.id()));
        let content = presents.value().target;
        dependencies.insert(RenderDependency::Entity(content));
        dependencies.insert(component_dependency::<Translation>(content));
        dependencies.insert(RenderDependency::RelationQuery {
            source: content,
            kind: RecognizedFrom::KIND.to_owned(),
        });
        let Some(translation) = self.snapshot.component::<Translation>(content)? else {
            return Ok(None);
        };
        let text = translation.text.value;
        if text.trim().is_empty() {
            return Ok(None);
        }
        let authored = self.snapshot.component::<Geometry>(entity)?;
        let placement = if let Some(placement) = self.balloon_flows.get(&entity) {
            dependencies.extend(placement.dependencies.iter().cloned());
            Some(placement.clone())
        } else if let Some(placement) = self.balloon_flow(entity, dependencies)? {
            Some(placement)
        } else {
            self.fit(entity, dependencies)?
        };
        let flow_contour = if authored.is_none() {
            placement
                .as_ref()
                .and_then(|placement| placement.flow_contour.clone())
        } else {
            None
        };
        let (geometry, frame, balloon_contour) = if let Some(geometry) = authored {
            let Some(frame) = geometry_frame(&geometry) else {
                return Ok(None);
            };
            let balloon = placement
                .as_ref()
                .and_then(|placement| placement.balloon_contour.as_ref())
                .map(|_| contour(&geometry, frame));
            (geometry, frame, balloon)
        } else {
            let Some(placement) = placement else {
                return Ok(None);
            };
            (
                placement.geometry,
                placement.frame,
                placement.balloon_contour,
            )
        };
        let typography = self.snapshot.component::<Typography>(entity)?;
        let analysis =
            if let Some(recognized) = self.snapshot.relation_from::<RecognizedFrom>(content)? {
                dependencies.insert(RenderDependency::Relation(recognized.id()));
                dependencies.insert(RenderDependency::Entity(recognized.value().target));
                dependencies.insert(component_dependency::<OcrAnalysis>(
                    recognized.value().target,
                ));
                self.snapshot
                    .component::<OcrAnalysis>(recognized.value().target)?
            } else {
                None
            };
        let writing_mode =
            resolve_writing_mode(&text, frame.bounds, typography.as_ref(), analysis.as_ref());
        let (direction, _) = shaping_direction_for_text(&text, writing_mode);
        let alignment = resolve_alignment(
            typography.as_ref().and_then(|value| value.alignment),
            writing_mode,
            direction == harfrust::Direction::RightToLeft,
        );
        let preferred_font = typography
            .as_ref()
            .and_then(|value| value.preferred_font.clone());
        let font_families = self.font_families.to_vec();
        for family in preferred_font.iter().chain(font_families.iter()) {
            dependencies.insert(RenderDependency::Font(family.clone()));
        }
        let is_bubble = balloon_contour.is_some();
        let descriptor = TextNodeDescriptor {
            entity,
            text: text.clone(),
            language: translation.language.clone(),
            width: frame.bounds.width,
            height: frame.bounds.height,
            balloon_contour,
            flow_contour,
            preferred_font,
            font_families,
            font_weight: typography.as_ref().and_then(|value| value.font_weight),
            font_style: typography
                .as_ref()
                .and_then(|value| value.font_style)
                .map(Into::into),
            font_size: typography.as_ref().and_then(|value| value.size),
            minimum_font_size: MINIMUM_FONT_SIZE,
            auto_fit: typography.as_ref().is_none_or(|value| value.auto_fit),
            alignment,
            writing_mode,
            foreground_color: typography
                .as_ref()
                .and_then(|value| value.color)
                .unwrap_or([0, 0, 0, 255]),
            stroke: resolve_stroke(typography.as_ref()),
            line_height: 1.2,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            text_inset: [4.0; 4],
            point_text: !is_bubble && layout.kind == TextLayoutKind::Point,
        };
        Ok(Some(LayerDraft {
            entity,
            geometry,
            frame,
            presentation,
            ancestry,
            descriptor: NodeDescriptor::Text(Box::new(descriptor)),
            metadata: DraftMetadata::Text {
                text,
                language: translation.language,
                alignment,
                writing_mode,
                angle_degrees: frame.angle_degrees,
            },
            dependencies: dependencies.iter().cloned().collect(),
        }))
    }

    fn fit(
        &self,
        entity: EntityId,
        dependencies: &mut BTreeSet<RenderDependency>,
    ) -> Result<Option<ResolvedPlacement>> {
        let Some(relation) = self.snapshot.relation_from::<FitsTo>(entity)? else {
            return Ok(None);
        };
        let target = relation.value().target;
        if !belongs_to_page(self.snapshot, target, self.page)? {
            return Ok(None);
        }
        dependencies.insert(RenderDependency::Relation(relation.id()));
        dependencies.insert(RenderDependency::Entity(target));
        dependencies.insert(component_dependency::<Geometry>(target));
        let geometry = self.snapshot.analysis_region(target)?.geometry()?;
        let Some(frame) = geometry_frame(&geometry) else {
            return Ok(None);
        };
        Ok(Some(ResolvedPlacement {
            geometry,
            frame,
            balloon_contour: None,
            flow_contour: None,
            dependencies: Arc::from([]),
        }))
    }

    fn balloon_flow(
        &self,
        entity: EntityId,
        dependencies: &mut BTreeSet<RenderDependency>,
    ) -> Result<Option<ResolvedPlacement>> {
        let Some(relation) = self.snapshot.relation_from::<FlowsIn>(entity)? else {
            return Ok(None);
        };
        let target = relation.value().target;
        if !belongs_to_page(self.snapshot, target, self.page)? {
            return Ok(None);
        }
        dependencies.insert(RenderDependency::Relation(relation.id()));
        dependencies.insert(RenderDependency::Entity(target));
        dependencies.insert(component_dependency::<Geometry>(target));
        let geometry = self.snapshot.analysis_region(target)?.geometry()?;
        let Some(frame) = geometry_frame(&geometry) else {
            return Ok(None);
        };
        Ok(Some(ResolvedPlacement {
            balloon_contour: Some(contour(&geometry, frame)),
            geometry,
            frame,
            flow_contour: None,
            dependencies: Arc::from([]),
        }))
    }
}

#[derive(Clone)]
struct ResolvedPlacement {
    geometry: Geometry,
    frame: GeometryFrame,
    balloon_contour: Option<Vec<(f32, f32)>>,
    flow_contour: Option<Vec<(f32, f32)>>,
    dependencies: Arc<[RenderDependency]>,
}

struct BalloonFlowPlan {
    placements: HashMap<EntityId, ResolvedPlacement>,
    dependencies: BTreeSet<RenderDependency>,
}

struct FlowSeed {
    layer: EntityId,
    anchor: Option<(f32, f32)>,
    dependencies: BTreeSet<RenderDependency>,
}

fn resolve_balloon_flows(snapshot: &Snapshot, page: EntityId) -> Result<BalloonFlowPlan> {
    let mut groups = BTreeMap::<EntityId, Vec<FlowSeed>>::new();
    if let Some(group) = snapshot.page(page)?.text_group()? {
        for layer in group.text_layers()? {
            let entity = layer.id();
            if snapshot.component::<Geometry>(entity)?.is_some() {
                continue;
            }
            let Some(relation) = snapshot.relation_from::<FlowsIn>(entity)? else {
                continue;
            };
            let balloon = relation.value().target;
            if !belongs_to_page(snapshot, balloon, page)? {
                continue;
            }
            let mut dependencies = BTreeSet::from([
                RenderDependency::Entity(entity),
                RenderDependency::Relation(relation.id()),
                RenderDependency::RelationQuery {
                    source: entity,
                    kind: FlowsIn::KIND.to_owned(),
                },
            ]);
            let Some(presents) = snapshot.relation_from::<Presents>(entity)? else {
                continue;
            };
            dependencies.insert(RenderDependency::Relation(presents.id()));
            let content = presents.value().target;
            dependencies.insert(RenderDependency::Entity(content));
            dependencies.insert(component_dependency::<Translation>(content));
            dependencies.insert(RenderDependency::RelationQuery {
                source: content,
                kind: RecognizedFrom::KIND.to_owned(),
            });
            let Some(translation) = snapshot.component::<Translation>(content)? else {
                continue;
            };
            if translation.text.value.trim().is_empty() {
                continue;
            }
            let anchor = flow_anchor(snapshot, content, &mut dependencies)?;
            groups.entry(balloon).or_default().push(FlowSeed {
                layer: entity,
                anchor,
                dependencies,
            });
        }
    }

    let mut placements = HashMap::new();
    let mut all_dependencies = BTreeSet::new();
    for (balloon, seeds) in groups {
        let region = snapshot.analysis_region(balloon)?;
        let geometry = region.geometry()?;
        let Some(frame) = geometry_frame(&geometry) else {
            continue;
        };
        let balloon_contour = contour(&geometry, frame);
        let mut dependencies = BTreeSet::from([
            RenderDependency::Entity(balloon),
            component_dependency::<Geometry>(balloon),
            component_dependency::<Region>(balloon),
            RenderDependency::RelationTargetQuery {
                target: balloon,
                kind: FlowsIn::KIND.to_owned(),
            },
        ]);
        for seed in &seeds {
            dependencies.extend(seed.dependencies.iter().cloned());
        }
        let center = (
            frame.bounds.x + frame.bounds.width * 0.5,
            frame.bounds.y + frame.bounds.height * 0.5,
        );
        let anchors = seeds
            .iter()
            .map(|seed| seed.anchor.unwrap_or(center))
            .collect::<Vec<_>>();
        let cells = (seeds.len() > 1).then(|| flow_cells(frame, &balloon_contour, &anchors));
        let dependencies: Arc<[RenderDependency]> = dependencies.iter().cloned().collect();
        for (index, seed) in seeds.into_iter().enumerate() {
            placements.insert(
                seed.layer,
                ResolvedPlacement {
                    geometry: geometry.clone(),
                    frame,
                    balloon_contour: Some(balloon_contour.clone()),
                    flow_contour: cells.as_ref().and_then(|cells| cells.get(index).cloned()),
                    dependencies: dependencies.clone(),
                },
            );
        }
        all_dependencies.extend(dependencies.iter().cloned());
    }
    Ok(BalloonFlowPlan {
        placements,
        dependencies: all_dependencies,
    })
}

fn flow_anchor(
    snapshot: &Snapshot,
    content: EntityId,
    dependencies: &mut BTreeSet<RenderDependency>,
) -> Result<Option<(f32, f32)>> {
    let Some(recognized) = snapshot.relation_from::<RecognizedFrom>(content)? else {
        return Ok(None);
    };
    dependencies.insert(RenderDependency::Relation(recognized.id()));
    let region = recognized.value().target;
    dependencies.insert(RenderDependency::Entity(region));
    dependencies.insert(component_dependency::<Geometry>(region));
    let Some(geometry) = snapshot.component::<Geometry>(region)? else {
        return Ok(None);
    };
    Ok(geometry_bounds(&geometry).map(|bounds| {
        (
            bounds.x + bounds.width * 0.5,
            bounds.y + bounds.height * 0.5,
        )
    }))
}

impl ImageNodeDescriptor {
    fn from_asset(asset: Asset, require_size: Option<(u32, u32)>) -> Self {
        Self {
            blob: asset.blob,
            media_type: asset.media_type,
            expected_size: asset.metadata.width.zip(asset.metadata.height),
            require_size,
        }
    }
}

fn image_draft(
    entity: EntityId,
    geometry: Geometry,
    descriptor: ImageNodeDescriptor,
    metadata: ImageMetadata,
    presentation: Presentation,
    ancestry: Arc<[EntityId]>,
    dependencies: &mut BTreeSet<RenderDependency>,
) -> Result<LayerDraft> {
    let frame = geometry_bounds(&geometry)
        .map(|bounds| GeometryFrame {
            bounds,
            angle_degrees: 0.0,
        })
        .ok_or_else(|| Error::invalid(format!("invalid image geometry for entity {entity}")))?;
    dependencies.insert(RenderDependency::Blob(descriptor.blob));
    Ok(LayerDraft {
        entity,
        geometry,
        frame,
        presentation,
        ancestry,
        descriptor: NodeDescriptor::Image(descriptor),
        metadata: DraftMetadata::Image(metadata),
        dependencies: dependencies.iter().cloned().collect(),
    })
}

fn build_node(
    descriptor: NodeDescriptor,
    fonts: &Fonts,
    images: &HashMap<BlobId, Arc<DecodedImage>>,
) -> Result<RetainedNode> {
    match &descriptor {
        NodeDescriptor::Image(image) => {
            let decoded = images.get(&image.blob).ok_or_else(|| {
                Error::invalid(format!("image blob {} was not loaded", image.blob))
            })?;
            if image
                .expected_size
                .is_some_and(|size| size != (decoded.width, decoded.height))
            {
                return Err(Error::invalid(format!(
                    "blob {} decoded as {}x{}, expected {:?}",
                    image.blob, decoded.width, decoded.height, image.expected_size
                )));
            }
            if image
                .require_size
                .is_some_and(|size| size != (decoded.width, decoded.height))
            {
                return Err(Error::invalid(format!(
                    "base image blob {} is {}x{}, expected {:?}",
                    image.blob, decoded.width, decoded.height, image.require_size
                )));
            }
            let raster = RasterImage {
                blob: image.blob,
                source: koharu_rasterizer::ResourceId::for_encoded_raster(
                    decoded.width,
                    decoded.height,
                    &image.media_type,
                    &decoded.encoded,
                ),
                width: decoded.width,
                height: decoded.height,
                media_type: image.media_type.clone(),
                encoded: decoded.encoded.clone(),
                pixels: decoded.pixels.clone(),
            };
            Ok(RetainedNode {
                descriptor,
                scene: Arc::new(koharu_rasterizer::PreparedScene::default()),
                resources: Arc::from([]),
                local_bounds: RenderBounds {
                    x: 0.0,
                    y: 0.0,
                    width: decoded.width as f32,
                    height: decoded.height as f32,
                },
                image: Some(raster),
                text: None,
                diagnostics: Arc::from([]),
            })
        }
        NodeDescriptor::Text(text) => {
            let rendered = TextRenderer::new().render_descriptor(text, fonts)?;
            Ok(RetainedNode {
                descriptor,
                scene: rendered.scene,
                resources: rendered.resources,
                local_bounds: rendered.local_bounds,
                image: None,
                text: Some(LocalTextMetadata {
                    rendered_bounds: rendered.metadata.rendered_bounds,
                    layout_bounds: rendered.metadata.layout_bounds,
                    post_script_fonts: rendered.metadata.post_script_fonts,
                    font_size: rendered.metadata.font_size,
                    color: rendered.metadata.color,
                }),
                diagnostics: rendered.diagnostics.into(),
            })
        }
    }
}

fn assemble_frame(
    compiled: CompiledPage,
    nodes: Vec<Arc<RetainedNode>>,
    stats: RetentionStats,
) -> Result<Frame> {
    let mut layers = Vec::with_capacity(compiled.layers.len());
    let mut diagnostics = compiled.diagnostics;
    for (draft, node) in compiled.layers.into_iter().zip(nodes) {
        let placement = placement(&draft, &node);
        let bounds = transform_bounds(node.local_bounds, placement);
        let kind = match draft.metadata {
            DraftMetadata::Image(metadata) => LayerKind::Image(metadata),
            DraftMetadata::Text {
                text,
                language,
                alignment,
                writing_mode,
                angle_degrees,
            } => {
                let metadata = node
                    .text
                    .as_ref()
                    .expect("text descriptor must build text metadata");
                LayerKind::Text(TextMetadata {
                    text,
                    language,
                    rendered_bounds: transform_bounds(metadata.rendered_bounds, placement),
                    layout_bounds: transform_bounds(metadata.layout_bounds, placement),
                    post_script_fonts: metadata.post_script_fonts.clone(),
                    font_size: metadata.font_size,
                    color: metadata.color,
                    alignment,
                    writing_mode,
                    angle_degrees,
                })
            }
        };
        diagnostics.extend(
            node.diagnostics
                .iter()
                .cloned()
                .map(|diagnostic| transform_diagnostic(diagnostic, placement)),
        );
        layers.push(Layer(Arc::new(LayerData {
            entity: draft.entity,
            geometry: draft.geometry,
            bounds,
            presentation: draft.presentation,
            ancestry: draft.ancestry,
            kind,
            placement,
            node,
            dependencies: draft.dependencies,
        })));
    }
    let layers: Arc<[Layer]> = layers.into();
    let layer_index = layers
        .iter()
        .enumerate()
        .map(|(index, layer)| (layer.entity(), index))
        .collect();
    let prepared = prepare_frame(
        compiled.revision,
        compiled.page,
        compiled.width,
        compiled.height,
        (0, 0),
        Affine::IDENTITY,
        &layers,
    )?;
    Ok(Frame(Arc::new(FrameData {
        revision: compiled.revision,
        page: compiled.page,
        width: compiled.width,
        height: compiled.height,
        origin: (0, 0),
        normalization: Affine::IDENTITY,
        layers,
        layer_index: Arc::new(layer_index),
        dependencies: compiled.dependencies,
        diagnostics: diagnostics.into(),
        stats,
        prepared: Arc::new(prepared),
    })))
}

fn placement(draft: &LayerDraft, node: &RetainedNode) -> Affine {
    match draft.descriptor {
        NodeDescriptor::Image(_) => Affine::scale_non_uniform(
            f64::from(draft.frame.bounds.width / node.local_bounds.width),
            f64::from(draft.frame.bounds.height / node.local_bounds.height),
        )
        .then_translate(Vec2::new(
            f64::from(draft.frame.bounds.x),
            f64::from(draft.frame.bounds.y),
        )),
        NodeDescriptor::Text(_) => {
            let frame = draft.frame.bounds;
            Affine::translate((
                f64::from(frame.x + frame.width * 0.5),
                f64::from(frame.y + frame.height * 0.5),
            )) * Affine::rotate(f64::from(draft.frame.angle_degrees).to_radians())
                * Affine::translate((
                    -f64::from(frame.width * 0.5),
                    -f64::from(frame.height * 0.5),
                ))
        }
    }
}

fn transform_bounds(bounds: RenderBounds, transform: Affine) -> RenderBounds {
    let rect = transform.transform_rect_bbox(Rect::new(
        f64::from(bounds.x),
        f64::from(bounds.y),
        f64::from(bounds.x + bounds.width),
        f64::from(bounds.y + bounds.height),
    ));
    RenderBounds {
        x: rect.x0 as f32,
        y: rect.y0 as f32,
        width: rect.width() as f32,
        height: rect.height() as f32,
    }
}

fn transform_diagnostic(diagnostic: RenderDiagnostic, transform: Affine) -> RenderDiagnostic {
    match diagnostic {
        RenderDiagnostic::TextOverflow {
            entity,
            available,
            actual_width,
            actual_height,
            font_size,
        } => RenderDiagnostic::TextOverflow {
            entity,
            available: transform_bounds(available, transform),
            actual_width,
            actual_height,
            font_size,
        },
        diagnostic => diagnostic,
    }
}

fn font_requests(pending: &[(usize, NodeDescriptor)]) -> Vec<FontRequest> {
    let mut seen = BTreeSet::new();
    let mut requests = Vec::new();
    for (_, descriptor) in pending {
        let NodeDescriptor::Text(text) = descriptor else {
            continue;
        };
        for family in text
            .preferred_font
            .iter()
            .chain(text.font_families.first())
            .take(1)
        {
            let style = match text.font_style.unwrap_or(FontStyle::Normal) {
                FontStyle::Normal => 0,
                FontStyle::Italic => 1,
                FontStyle::Oblique => 2,
            };
            if seen.insert((family.to_ascii_lowercase(), text.font_weight, style)) {
                requests.push(FontRequest {
                    family: family.clone(),
                    weight: text.font_weight,
                    style: text.font_style,
                });
            }
        }
    }
    requests
}

fn component_dependency<T: Component>(entity: EntityId) -> RenderDependency {
    RenderDependency::Component {
        entity,
        kind: T::KIND.to_owned(),
    }
}

fn belongs_to_page(snapshot: &Snapshot, mut entity: EntityId, page: EntityId) -> Result<bool> {
    loop {
        if entity == page {
            return Ok(true);
        }
        let Some(parent) = snapshot.parent(entity)? else {
            return Ok(false);
        };
        entity = parent;
    }
}

fn surface_size(page: &Page) -> Result<(u32, u32)> {
    if !page.width.is_finite()
        || !page.height.is_finite()
        || page.width <= 0.0
        || page.height <= 0.0
    {
        return Err(Error::invalid(
            "page dimensions must be finite and positive",
        ));
    }
    let width = page.width.ceil();
    let height = page.height.ceil();
    if width > f64::from(u32::MAX) || height > f64::from(u32::MAX) {
        return Err(Error::invalid("page dimensions exceed u32"));
    }
    let (width, height) = (width as u32, height as u32);
    if width > MAX_SURFACE_DIMENSION
        || height > MAX_SURFACE_DIMENSION
        || u64::from(width) * u64::from(height) > MAX_SURFACE_PIXELS
    {
        return Err(Error::invalid(format!(
            "page surface {width}x{height} exceeds renderer limits"
        )));
    }
    Ok((width, height))
}

fn resolve_writing_mode(
    text: &str,
    bounds: LayoutBox,
    typography: Option<&Typography>,
    analysis: Option<&OcrAnalysis>,
) -> WritingMode {
    let typography_mode = typography
        .and_then(|value| value.writing_mode)
        .map(|mode| match mode {
            koharu_scene::WritingMode::Horizontal => WritingMode::Horizontal,
            koharu_scene::WritingMode::Vertical => WritingMode::VerticalRl,
        });
    if typography.is_some_and(|value| matches!(&value.origin, Origin::User))
        && let Some(mode) = typography_mode
    {
        return mode;
    }
    if !is_chinese_or_japanese_text(text) {
        return WritingMode::Horizontal;
    }
    if let Some(mode) = typography_mode {
        return mode;
    }
    match analysis.map(|value| value.direction) {
        Some(TextDirection::Vertical) => WritingMode::VerticalRl,
        Some(TextDirection::Horizontal) => WritingMode::Horizontal,
        Some(TextDirection::Auto) | None if bounds.height > bounds.width => WritingMode::VerticalRl,
        Some(TextDirection::Auto) | None => WritingMode::Horizontal,
    }
}

fn resolve_alignment(
    alignment: Option<TextAlignment>,
    writing_mode: WritingMode,
    rtl: bool,
) -> TextAlign {
    let alignment = alignment.unwrap_or(if writing_mode.is_vertical() {
        TextAlignment::Start
    } else {
        TextAlignment::Center
    });
    match alignment {
        TextAlignment::Start if writing_mode.is_vertical() => TextAlign::Left,
        TextAlignment::End if writing_mode.is_vertical() => TextAlign::Right,
        TextAlignment::Start if rtl => TextAlign::Right,
        TextAlignment::Start => TextAlign::Left,
        TextAlignment::Center => TextAlign::Center,
        TextAlignment::End if rtl => TextAlign::Left,
        TextAlignment::End => TextAlign::Right,
        TextAlignment::Justify => TextAlign::Justify,
    }
}

fn resolve_stroke(typography: Option<&Typography>) -> Option<StrokeOptions> {
    let typography = typography?;
    let width_px = typography.stroke_width.filter(|width| *width > 0.0)?;
    Some(StrokeOptions {
        color: typography.stroke_color.unwrap_or([u8::MAX; 4]),
        width_px,
    })
}

struct CachedNode {
    node: Arc<RetainedNode>,
    last_used: u64,
}

struct NodeCache {
    entries: HashMap<(EntityId, EntityId), CachedNode>,
    capacity: usize,
    clock: u64,
}

impl NodeCache {
    fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            capacity,
            clock: 0,
        }
    }

    fn next_clock(&mut self) -> u64 {
        self.clock = self.clock.wrapping_add(1);
        self.clock
    }

    fn get(
        &mut self,
        key: (EntityId, EntityId),
        descriptor: &NodeDescriptor,
        clock: u64,
    ) -> Option<Arc<RetainedNode>> {
        let entry = self.entries.get_mut(&key)?;
        if entry.node.descriptor != *descriptor {
            return None;
        }
        entry.last_used = clock;
        Some(entry.node.clone())
    }

    fn insert(&mut self, key: (EntityId, EntityId), node: Arc<RetainedNode>, clock: u64) {
        if self.capacity == 0 {
            return;
        }
        self.entries.insert(
            key,
            CachedNode {
                node,
                last_used: clock,
            },
        );
        while self.entries.len() > self.capacity {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| *key)
            else {
                break;
            };
            self.entries.remove(&oldest);
        }
    }
}

struct AffectedDependencies {
    entities: HashSet<EntityId>,
    hierarchy: HashSet<EntityId>,
    components: HashMap<EntityId, HashSet<String>>,
    relations: HashSet<RelationId>,
    relation_queries: HashMap<EntityId, HashSet<String>>,
    relation_target_queries: HashMap<EntityId, HashSet<String>>,
    project_component_changed: bool,
}

impl AffectedDependencies {
    fn new(snapshot: &Snapshot, change: &Change) -> Result<Self> {
        let entities = change
            .entities
            .iter()
            .map(|change| match *change {
                EntityChange::Inserted(entity) | EntityChange::Removed(entity) => entity,
            })
            .collect();
        let hierarchy = change.hierarchy.iter().copied().collect();
        let mut components = HashMap::<EntityId, HashSet<String>>::new();
        let mut relations = HashSet::new();
        let mut project_component_changed = false;
        for change in &change.components {
            match change.owner {
                ComponentOwner::Entity(entity) => {
                    components
                        .entry(entity)
                        .or_default()
                        .insert(change.kind.clone());
                }
                ComponentOwner::Project => project_component_changed = true,
                ComponentOwner::Relation(relation) => {
                    relations.insert(relation);
                }
            }
        }
        let mut relation_queries = HashMap::<EntityId, HashSet<String>>::new();
        let mut relation_target_queries = HashMap::<EntityId, HashSet<String>>::new();
        for change in &change.relations {
            let id = match *change {
                RelationChange::Inserted(id)
                | RelationChange::Removed(id)
                | RelationChange::Changed(id) => id,
            };
            relations.insert(id);
            if !matches!(change, RelationChange::Removed(_)) {
                let relation = snapshot.relation(id)?.value();
                relation_queries
                    .entry(relation.source)
                    .or_default()
                    .insert(relation.kind.as_str().to_owned());
                relation_target_queries
                    .entry(relation.target)
                    .or_default()
                    .insert(relation.kind.as_str().to_owned());
            }
        }
        Ok(Self {
            entities,
            hierarchy,
            components,
            relations,
            relation_queries,
            relation_target_queries,
            project_component_changed,
        })
    }

    fn intersects(&self, dependencies: &[RenderDependency]) -> bool {
        self.project_component_changed
            || dependencies.iter().any(|dependency| match dependency {
                RenderDependency::Entity(entity) => self.entities.contains(entity),
                RenderDependency::Hierarchy(entity) => self.hierarchy.contains(entity),
                RenderDependency::Component { entity, kind } => self
                    .components
                    .get(entity)
                    .is_some_and(|kinds| kinds.contains(kind)),
                RenderDependency::Relation(relation) => self.relations.contains(relation),
                RenderDependency::RelationQuery { source, kind } => self
                    .relation_queries
                    .get(source)
                    .is_some_and(|kinds| kinds.contains(kind)),
                RenderDependency::RelationTargetQuery { target, kind } => self
                    .relation_target_queries
                    .get(target)
                    .is_some_and(|kinds| kinds.contains(kind)),
                RenderDependency::Blob(_) | RenderDependency::Font(_) => false,
            })
    }
}

fn draw_font_preview(scene: &mut Scene, layout: &crate::LayoutRun<'_>) -> anyhow::Result<()> {
    let brush = vello::peniko::Color::from_rgba8(0, 0, 0, 255);
    for line in &layout.lines {
        let (baseline_x, baseline_y) = line.baseline;
        let mut pen_x = 0.0;
        let mut pen_y = 0.0;
        for glyph in &line.glyphs {
            let font = glyph.font.skrifa_ref()?;
            if let Some(outline) = font.outline_glyphs().get(GlyphId::new(glyph.glyph_id)) {
                let mut path = BezPath::new();
                outline.draw(
                    DrawSettings::unhinted(Size::new(layout.font_size), glyph.font.location()),
                    &mut PreviewOutline(&mut path),
                )?;
                let transform = Affine::translate((
                    f64::from(baseline_x + pen_x + glyph.x_offset),
                    f64::from(baseline_y + pen_y - glyph.y_offset),
                )) * Affine::scale_non_uniform(1.0, -1.0);
                scene.fill(Fill::NonZero, transform, brush, None, &path);
            }
            pen_x += glyph.x_advance;
            pen_y -= glyph.y_advance;
        }
    }
    Ok(())
}

struct PreviewOutline<'a>(&'a mut BezPath);

impl OutlinePen for PreviewOutline<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to((f64::from(x), f64::from(y)));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to((f64::from(x), f64::from(y)));
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.quad_to(
            (f64::from(cx0), f64::from(cy0)),
            (f64::from(x), f64::from(y)),
        );
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.curve_to(
            (f64::from(cx0), f64::from(cy0)),
            (f64::from(cx1), f64::from(cy1)),
            (f64::from(x), f64::from(y)),
        );
    }

    fn close(&mut self) {
        self.0.close_path();
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, io::Cursor};

    use koharu_scene::{
        AssetInput, AssetMetadata, At, Authored, BubbleRegion, Generation, PageDraft, ProducerId,
        Session, SourceText, TextLayout as SceneTextLayout, TextLayoutKind,
    };

    use super::*;

    #[test]
    fn vertical_alignment_defaults_to_start_and_uses_the_inline_axis() {
        assert_eq!(
            resolve_alignment(None, WritingMode::VerticalRl, false),
            TextAlign::Left
        );
        assert_eq!(
            resolve_alignment(Some(TextAlignment::Start), WritingMode::VerticalRl, true,),
            TextAlign::Left
        );
        assert_eq!(
            resolve_alignment(Some(TextAlignment::End), WritingMode::VerticalRl, false,),
            TextAlign::Right
        );
        assert_eq!(
            resolve_alignment(None, WritingMode::Horizontal, false),
            TextAlign::Center
        );
    }

    #[test]
    fn user_writing_mode_overrides_language_default() {
        let mut typography = Typography {
            origin: koharu_scene::Origin::User,
            preferred_font: None,
            font_weight: None,
            font_style: None,
            size: None,
            auto_fit: true,
            color: None,
            stroke_color: None,
            stroke_width: None,
            alignment: None,
            writing_mode: Some(koharu_scene::WritingMode::Vertical),
            extensions: BTreeMap::new(),
        };
        let bounds = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 200.0,
        };

        assert_eq!(
            resolve_writing_mode("rokuna", bounds, Some(&typography), None),
            WritingMode::VerticalRl
        );
        typography.writing_mode = Some(koharu_scene::WritingMode::Horizontal);
        assert_eq!(
            resolve_writing_mode("日本語", bounds, Some(&typography), None),
            WritingMode::Horizontal
        );
    }

    #[test]
    fn generated_vertical_mode_is_limited_to_chinese_and_japanese() {
        let typography = Typography {
            origin: koharu_scene::Origin::Generated(Generation::new(
                ProducerId::new("dev.koharu.pipeline.detection").unwrap(),
            )),
            preferred_font: None,
            font_weight: None,
            font_style: None,
            size: None,
            auto_fit: true,
            color: None,
            stroke_color: None,
            stroke_width: None,
            alignment: None,
            writing_mode: Some(koharu_scene::WritingMode::Vertical),
            extensions: BTreeMap::new(),
        };
        let bounds = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 200.0,
        };

        assert_eq!(
            resolve_writing_mode("rokuna", bounds, Some(&typography), None),
            WritingMode::Horizontal
        );
        assert_eq!(
            resolve_writing_mode("繁體中文", bounds, Some(&typography), None),
            WritingMode::VerticalRl
        );
    }

    fn png(width: u32, height: u32, color: [u8; 4]) -> Arc<[u8]> {
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba(color));
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        bytes.into_inner().into()
    }

    fn asset(bytes: Arc<[u8]>, width: u32, height: u32) -> AssetInput {
        AssetInput::new(
            bytes,
            "image/png",
            AssetMetadata {
                width: Some(width),
                height: Some(height),
                attributes: BTreeMap::new(),
            },
        )
    }

    #[test]
    fn construction_and_clone_do_not_initialize_expensive_resources() {
        let renderer = Renderer::default();
        let cloned = renderer.clone();
        assert!(Arc::ptr_eq(&renderer.inner, &cloned.inner));
        assert!(!renderer.inner.fonts.is_system_initialized());
        assert!(renderer.inner.workers.get().is_none());
    }

    #[tokio::test]
    async fn source_text_is_not_visual_input() {
        let mut session = Session::memory().await.unwrap();
        let mut ids = None;
        let create = session
            .snapshot()
            .patch(|edit| {
                let page = edit.add_page(PageDraft::new("page", 200.0, 120.0), At::End)?;
                let content = edit.add_text_content(page, At::End)?;
                edit.set(
                    content,
                    &SourceText {
                        text: Authored::user("semantic source".to_owned()),
                        language: None,
                    },
                )?;
                let text = edit.add_text_layer(
                    page,
                    At::End,
                    content,
                    &SceneTextLayout {
                        origin: Origin::User,
                        kind: TextLayoutKind::Paragraph,
                    },
                )?;
                edit.set(text, &Geometry::rectangle(10.0, 10.0, 80.0, 40.0))?;
                ids = Some((page, content, text));
                Ok(())
            })
            .unwrap();
        let snapshot = session.commit(create).await.unwrap().snapshot;
        let (page, content, text) = ids.unwrap();
        let renderer = Renderer::default();
        let compiled = renderer.compile(&snapshot, page).unwrap();
        assert!(compiled.layers.iter().all(|layer| layer.entity != text));

        let translation = snapshot
            .patch(|edit| {
                edit.set(
                    content,
                    &Translation {
                        text: Authored::user("visible translation".to_owned()),
                        language: None,
                    },
                )
            })
            .unwrap();
        let commit = session.commit(translation).await.unwrap();
        let compiled = renderer.compile(&commit.snapshot, page).unwrap();
        assert!(compiled.layers.iter().any(|layer| layer.entity == text));
    }

    #[tokio::test]
    async fn analysis_regions_are_not_promoted_to_image_layers() {
        let mut session = Session::memory().await.unwrap();
        let source = AssetRole::new("source").unwrap();
        let mut ids = None;
        let create = session
            .snapshot()
            .patch(|edit| {
                let page = edit.add_page(PageDraft::new("page", 4.0, 4.0), At::End)?;
                let region = edit.add_analysis_region::<koharu_scene::TextRegion>(
                    page,
                    At::End,
                    &Geometry::rectangle(0.0, 0.0, 2.0, 2.0),
                    None,
                )?;
                edit.set_asset(region, &source, asset(png(2, 2, [1, 2, 3, 255]), 2, 2))?;
                ids = Some((page, region));
                Ok(())
            })
            .unwrap();
        let snapshot = session.commit(create).await.unwrap().snapshot;
        let (page, region) = ids.unwrap();

        let compiled = Renderer::default().compile(&snapshot, page).unwrap();
        assert!(compiled.layers.iter().all(|layer| layer.entity != region));
    }

    #[tokio::test]
    async fn fit_relation_changes_target_only_the_source_text_layer() {
        let mut session = Session::memory().await.unwrap();
        let mut ids = None;
        let create = session
            .snapshot()
            .patch(|edit| {
                let page = edit.add_page(PageDraft::new("page", 200.0, 120.0), At::End)?;
                let target = edit.add_analysis_region::<koharu_scene::TextRegion>(
                    page,
                    At::End,
                    &Geometry::rectangle(20.0, 20.0, 100.0, 60.0),
                    None,
                )?;
                let first_content = edit.add_text_content(page, At::End)?;
                edit.set(
                    first_content,
                    &SourceText {
                        text: Authored::user("source one".to_owned()),
                        language: None,
                    },
                )?;
                edit.set(
                    first_content,
                    &Translation {
                        text: Authored::user("first".to_owned()),
                        language: None,
                    },
                )?;
                let first = edit.add_text_layer(
                    page,
                    At::End,
                    first_content,
                    &SceneTextLayout {
                        origin: Origin::User,
                        kind: TextLayoutKind::Paragraph,
                    },
                )?;
                edit.set(first, &Geometry::rectangle(10.0, 10.0, 80.0, 40.0))?;
                let second_content = edit.add_text_content(page, At::End)?;
                edit.set(
                    second_content,
                    &SourceText {
                        text: Authored::user("source two".to_owned()),
                        language: None,
                    },
                )?;
                edit.set(
                    second_content,
                    &Translation {
                        text: Authored::user("second".to_owned()),
                        language: None,
                    },
                )?;
                let second = edit.add_text_layer(
                    page,
                    At::End,
                    second_content,
                    &SceneTextLayout {
                        origin: Origin::User,
                        kind: TextLayoutKind::Paragraph,
                    },
                )?;
                edit.set(second, &Geometry::rectangle(100.0, 10.0, 80.0, 40.0))?;
                ids = Some((page, target, first, second));
                Ok(())
            })
            .unwrap();
        let base = session.commit(create).await.unwrap().snapshot;
        let (page, target, first, second) = ids.unwrap();
        let renderer = Renderer::default();
        let base_compiled = renderer.compile(&base, page).unwrap();
        let first_dependencies = &base_compiled
            .layers
            .iter()
            .find(|layer| layer.entity == first)
            .unwrap()
            .dependencies;
        let second_dependencies = &base_compiled
            .layers
            .iter()
            .find(|layer| layer.entity == second)
            .unwrap()
            .dependencies;

        let add_fit = base
            .patch(|edit| edit.relate::<FitsTo>(first, target).map(|_| ()))
            .unwrap();
        let fitted = session.commit(add_fit).await.unwrap();
        let affected = AffectedDependencies::new(&fitted.snapshot, &fitted.changes).unwrap();
        assert!(affected.intersects(first_dependencies));
        assert!(!affected.intersects(second_dependencies));

        let fitted_compiled = renderer.compile(&fitted.snapshot, page).unwrap();
        let first_dependencies = &fitted_compiled
            .layers
            .iter()
            .find(|layer| layer.entity == first)
            .unwrap()
            .dependencies;
        let relation = fitted
            .snapshot
            .relation_from::<FitsTo>(first)
            .unwrap()
            .unwrap()
            .id();
        let remove_fit = fitted
            .snapshot
            .patch(|edit| edit.remove_relation(relation))
            .unwrap();
        let removed = session.commit(remove_fit).await.unwrap();
        let affected = AffectedDependencies::new(&removed.snapshot, &removed.changes).unwrap();
        assert!(affected.intersects(first_dependencies));
        assert!(!affected.intersects(second_dependencies));
    }

    #[tokio::test]
    async fn joined_balloon_flows_receive_disjoint_layout_cells() {
        let mut session = Session::memory().await.unwrap();
        let mut ids = None;
        let create = session
            .snapshot()
            .patch(|edit| {
                let page = edit.add_page(PageDraft::new("page", 160.0, 100.0), At::End)?;
                let bubble = edit.add_analysis_region::<BubbleRegion>(
                    page,
                    At::End,
                    &Geometry::rectangle(20.0, 20.0, 120.0, 60.0),
                    None,
                )?;
                let mut layers = Vec::new();
                for (index, (x, source, translation)) in [
                    (35.0, "source one", "The first translated flow"),
                    (95.0, "source two", "The second translated flow"),
                ]
                .into_iter()
                .enumerate()
                {
                    let region = edit.add_analysis_region::<koharu_scene::TextRegion>(
                        page,
                        At::End,
                        &Geometry::rectangle(x, 35.0, 20.0, 30.0),
                        None,
                    )?;
                    let content = edit.add_text_content(page, At::End)?;
                    edit.set(
                        content,
                        &SourceText {
                            text: Authored::user(source.to_owned()),
                            language: None,
                        },
                    )?;
                    edit.set(
                        content,
                        &Translation {
                            text: Authored::user(translation.to_owned()),
                            language: None,
                        },
                    )?;
                    let layer = edit.add_text_layer(
                        page,
                        At::End,
                        content,
                        &SceneTextLayout {
                            origin: Origin::User,
                            kind: TextLayoutKind::Paragraph,
                        },
                    )?;
                    edit.relate::<RecognizedFrom>(content, region)?;
                    if index == 0 {
                        edit.relate::<FlowsIn>(layer, bubble)?;
                    }
                    layers.push(layer);
                }
                ids = Some((page, bubble, layers));
                Ok(())
            })
            .unwrap();
        let base = session.commit(create).await.unwrap().snapshot;
        let (page, bubble, layers) = ids.unwrap();
        let renderer = Renderer::default();
        let base_compiled = renderer.compile(&base, page).unwrap();
        let first = base_compiled
            .layers
            .iter()
            .find(|layer| layer.entity == layers[0])
            .unwrap();
        let NodeDescriptor::Text(descriptor) = &first.descriptor else {
            panic!("expected a text descriptor");
        };
        assert!(descriptor.flow_contour.is_none());

        let add_sibling = base
            .patch(|edit| edit.relate::<FlowsIn>(layers[1], bubble).map(|_| ()))
            .unwrap();
        let joined = session.commit(add_sibling).await.unwrap();
        let affected = AffectedDependencies::new(&joined.snapshot, &joined.changes).unwrap();
        assert!(affected.intersects(&first.dependencies));

        let compiled = renderer.compile(&joined.snapshot, page).unwrap();
        let contours = layers
            .iter()
            .map(|entity| {
                let layer = compiled
                    .layers
                    .iter()
                    .find(|layer| layer.entity == *entity)
                    .unwrap();
                let NodeDescriptor::Text(descriptor) = &layer.descriptor else {
                    panic!("expected a text descriptor");
                };
                descriptor.flow_contour.clone().unwrap()
            })
            .collect::<Vec<_>>();
        let first_right = contours[0]
            .iter()
            .map(|(x, _)| *x)
            .fold(f32::NEG_INFINITY, f32::max);
        let second_left = contours[1]
            .iter()
            .map(|(x, _)| *x)
            .fold(f32::INFINITY, f32::min);
        assert!((first_right - second_left).abs() < 1e-4);
        assert!(first_right > 25.0 && first_right < 85.0);
    }

    #[tokio::test]
    async fn placement_update_reuses_image_nodes_and_reports_candidates() {
        let mut session = Session::memory().await.unwrap();
        let source = AssetRole::new("source").unwrap();
        let mut ids = None;
        let create = session
            .snapshot()
            .patch(|edit| {
                let page = edit.add_page(PageDraft::new("page", 4.0, 4.0), At::End)?;
                edit.set_asset(page, &source, asset(png(4, 4, [1, 2, 3, 255]), 4, 4))?;
                let group = edit.add_entity(page, At::End)?;
                edit.set(
                    group,
                    &Group {
                        origin: Origin::User,
                        name: "nested".to_owned(),
                    },
                )?;
                let image = edit.add_entity(group, At::End)?;
                edit.set(image, &Geometry::rectangle(0.0, 0.0, 2.0, 2.0))?;
                edit.set(
                    image,
                    &Visibility {
                        origin: Origin::User,
                        visible: false,
                        opacity: 0.3,
                    },
                )?;
                edit.set_asset(image, &source, asset(png(2, 2, [9, 8, 7, 255]), 2, 2))?;
                ids = Some((page, group, image));
                Ok(())
            })
            .unwrap();
        let snapshot = session.commit(create).await.unwrap().snapshot;
        let (page, group, image) = ids.unwrap();
        let renderer = Renderer::new().unwrap();
        let first = renderer.render(&snapshot, page).await.unwrap();
        assert_eq!(first.layers().len(), 2);
        assert_eq!(first.stats().rebuilt_layers, 2);
        assert_eq!(first.layer(image).unwrap().ancestry(), &[group]);
        assert_eq!(
            first.layer(image).unwrap().presentation(),
            Presentation {
                visible: false,
                opacity: 0.3,
            }
        );

        let move_image = snapshot
            .patch(|edit| edit.set(image, &Geometry::rectangle(1.0, 1.0, 2.0, 2.0)))
            .unwrap();
        let commit = session.commit(move_image).await.unwrap();
        let updated = renderer
            .update(&first, &commit.snapshot, &commit.changes)
            .await
            .unwrap();
        assert_eq!(updated.stats().candidate_layers, 1);
        assert_eq!(updated.stats().reused_layers, 2);
        assert_eq!(updated.stats().rebuilt_layers, 0);
        assert!(Arc::ptr_eq(
            &first.layer(image).unwrap().0.node,
            &updated.layer(image).unwrap().0.node
        ));
        assert_eq!(updated.layer(image).unwrap().bounds().x, 1.0);
        let cropped = updated.cropped(image).unwrap().unwrap();
        assert_eq!(cropped.origin(), (1, 1));
        assert_eq!(cropped.size(), (2, 2));
        assert_eq!(
            cropped.layer(image).unwrap().presentation(),
            Presentation {
                visible: true,
                opacity: 1.0,
            }
        );
    }

    #[tokio::test]
    async fn update_rejects_a_non_contiguous_revision_chain() {
        let mut session = Session::memory().await.unwrap();
        let mut page = None;
        let create = session
            .snapshot()
            .patch(|edit| {
                page = Some(edit.add_page(PageDraft::new("page", 10.0, 10.0), At::End)?);
                Ok(())
            })
            .unwrap();
        let first_commit = session.commit(create).await.unwrap();
        let page = page.unwrap();
        let renderer = Renderer::new().unwrap();
        let frame = renderer.render(&first_commit.snapshot, page).await.unwrap();
        let change = first_commit
            .snapshot
            .patch(|edit| {
                edit.set(
                    page,
                    &Visibility {
                        origin: Origin::User,
                        visible: true,
                        opacity: 0.5,
                    },
                )
            })
            .unwrap();
        let second = session.commit(change).await.unwrap();
        let error = renderer
            .update(&frame, &first_commit.snapshot, &second.changes)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("not contiguous"));
    }
}
