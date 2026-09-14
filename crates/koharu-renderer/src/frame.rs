//! Immutable retained page output and synchronous vector access.

use std::{collections::HashMap, sync::Arc};

use anyhow::anyhow;
use koharu_rasterizer::{
    Bounds as PreparedBounds, LayerId, LayerKind as PreparedLayerKind,
    PREPARED_RASTER_TILE_DIMENSION, Point as PreparedPoint, PreparedContent, PreparedElementFrame,
    PreparedFrame, PreparedFrameBundle, PreparedLayer, PreparedRaster, PreparedRasterTile,
    PreparedResource, Presentation as PreparedPresentation, ResourceId,
    Revision as PreparedRevision,
};
use koharu_scene::{BlobId, EntityId, Geometry, LanguageTag, RelationId, Revision};
use vello::kurbo::Affine;

use crate::{Error, Result, TextAlign, WritingMode};

const MAX_SURFACE_DIMENSION: u32 = 32_768;
const MAX_SURFACE_PIXELS: u64 = 268_435_456;

#[derive(Clone)]
pub struct Frame(pub(crate) Arc<FrameData>);

pub(crate) struct FrameData {
    pub(crate) revision: Revision,
    pub(crate) page: EntityId,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) origin: (i32, i32),
    pub(crate) normalization: Affine,
    pub(crate) layers: Arc<[Layer]>,
    pub(crate) layer_index: Arc<HashMap<EntityId, usize>>,
    pub(crate) dependencies: Arc<[RenderDependency]>,
    pub(crate) diagnostics: Arc<[RenderDiagnostic]>,
    pub(crate) stats: RetentionStats,
    pub(crate) prepared: Arc<PreparedFrameBundle>,
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Frame")
            .field("revision", &self.revision())
            .field("page", &self.page())
            .field("size", &self.size())
            .field("origin", &self.origin())
            .field("layers", &self.layers())
            .field("diagnostics", &self.diagnostics())
            .field("stats", &self.stats())
            .finish_non_exhaustive()
    }
}

impl Frame {
    #[must_use]
    pub fn revision(&self) -> Revision {
        self.0.revision
    }

    #[must_use]
    pub fn page(&self) -> EntityId {
        self.0.page
    }

    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        (self.0.width, self.0.height)
    }

    #[must_use]
    pub fn origin(&self) -> (i32, i32) {
        self.0.origin
    }

    #[must_use]
    pub fn layers(&self) -> &[Layer] {
        &self.0.layers
    }

    #[must_use]
    pub fn layer(&self, entity: EntityId) -> Option<&Layer> {
        self.0
            .layer_index
            .get(&entity)
            .map(|index| &self.0.layers[*index])
    }

    #[must_use]
    pub fn dependencies(&self) -> &[RenderDependency] {
        &self.0.dependencies
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[RenderDiagnostic] {
        &self.0.diagnostics
    }

    #[must_use]
    pub fn stats(&self) -> RetentionStats {
        self.0.stats
    }

    #[must_use]
    pub fn prepared(&self) -> &PreparedFrameBundle {
        &self.0.prepared
    }

    pub fn raster_frame(&self) -> Result<koharu_rasterizer::Frame> {
        let raster_sources = self
            .layers()
            .iter()
            .filter_map(|layer| layer.raster_image())
            .map(|image| (image.source, Arc::clone(&image.pixels)))
            .collect::<HashMap<_, _>>();
        self.0
            .prepared
            .as_ref()
            .clone()
            .into_frame_with_raster_sources(&raster_sources)
            .map_err(|error| Error::Backend(anyhow!(error)))
    }

    /// Returns one entity normalized into a tightly cropped frame.
    ///
    /// The original layer retains authored presentation. The isolated copy is
    /// visible at full opacity so layered exports can store pixels separately
    /// from their visibility and opacity metadata.
    pub fn cropped(&self, entity: EntityId) -> Result<Option<Self>> {
        let Some(layer) = self.layer(entity).cloned() else {
            return Ok(None);
        };
        let bounds = layer.bounds();
        if ![bounds.x, bounds.y, bounds.width, bounds.height]
            .into_iter()
            .all(f32::is_finite)
            || bounds.width < 0.0
            || bounds.height < 0.0
        {
            return Err(Error::invalid(format!(
                "visual layer bounds are not finite for entity {entity}"
            )));
        }
        let edges = [
            bounds.x.floor(),
            bounds.y.floor(),
            (bounds.x + bounds.width).ceil(),
            (bounds.y + bounds.height).ceil(),
        ];
        if edges.iter().any(|edge| {
            f64::from(*edge) < f64::from(i32::MIN) || f64::from(*edge) > f64::from(i32::MAX)
        }) {
            return Err(Error::invalid("visual layer crop origin exceeds i32"));
        }
        let [left, top, right, bottom] = edges.map(|edge| edge as i32);
        let width = u32::try_from((i64::from(right) - i64::from(left)).max(1))
            .map_err(|_| Error::invalid("visual layer width exceeds u32"))?;
        let height = u32::try_from((i64::from(bottom) - i64::from(top)).max(1))
            .map_err(|_| Error::invalid("visual layer height exceeds u32"))?;
        if width > MAX_SURFACE_DIMENSION
            || height > MAX_SURFACE_DIMENSION
            || u64::from(width) * u64::from(height) > MAX_SURFACE_PIXELS
        {
            return Err(Error::invalid(format!(
                "cropped surface {width}x{height} exceeds renderer limits"
            )));
        }
        let isolated = Layer(Arc::new(LayerData {
            presentation: Presentation {
                visible: true,
                opacity: 1.0,
            },
            ..layer.0.clone_for_frame()
        }));
        let layers: Arc<[Layer]> = vec![isolated].into();
        let prepared = prepare_frame(
            self.revision(),
            self.page(),
            width,
            height,
            (left, top),
            Affine::translate((-f64::from(left), -f64::from(top))),
            &layers,
        )?;
        Ok(Some(Self(Arc::new(FrameData {
            revision: self.revision(),
            page: self.page(),
            width,
            height,
            origin: (left, top),
            normalization: Affine::translate((-f64::from(left), -f64::from(top))),
            layers,
            layer_index: Arc::new(HashMap::from([(entity, 0)])),
            dependencies: self.0.dependencies.clone(),
            diagnostics: self.0.diagnostics.clone(),
            stats: self.0.stats,
            prepared: Arc::new(prepared),
        }))))
    }

    pub(crate) fn at_revision(&self, revision: Revision, stats: RetentionStats) -> Self {
        let mut prepared = self.0.prepared.as_ref().clone();
        prepared.frame.revision = PreparedRevision::new(revision.get());
        Self(Arc::new(FrameData {
            revision,
            page: self.0.page,
            width: self.0.width,
            height: self.0.height,
            origin: self.0.origin,
            normalization: self.0.normalization,
            layers: self.0.layers.clone(),
            layer_index: self.0.layer_index.clone(),
            dependencies: self.0.dependencies.clone(),
            diagnostics: self.0.diagnostics.clone(),
            stats,
            prepared: Arc::new(prepared),
        }))
    }
}

#[derive(Clone)]
pub struct Layer(pub(crate) Arc<LayerData>);

impl std::fmt::Debug for Layer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Layer")
            .field("entity", &self.entity())
            .field("bounds", &self.bounds())
            .field("presentation", &self.presentation())
            .field("kind", self.kind())
            .finish_non_exhaustive()
    }
}

pub(crate) struct LayerData {
    pub(crate) entity: EntityId,
    pub(crate) geometry: Geometry,
    pub(crate) bounds: RenderBounds,
    pub(crate) presentation: Presentation,
    pub(crate) ancestry: Arc<[EntityId]>,
    pub(crate) kind: LayerKind,
    pub(crate) placement: Affine,
    pub(crate) node: Arc<RetainedNode>,
    pub(crate) dependencies: Arc<[RenderDependency]>,
}

impl LayerData {
    fn clone_for_frame(&self) -> Self {
        Self {
            entity: self.entity,
            geometry: self.geometry.clone(),
            bounds: self.bounds,
            presentation: self.presentation,
            ancestry: self.ancestry.clone(),
            kind: self.kind.clone(),
            placement: self.placement,
            node: self.node.clone(),
            dependencies: self.dependencies.clone(),
        }
    }
}

impl Layer {
    #[must_use]
    pub fn entity(&self) -> EntityId {
        self.0.entity
    }

    #[must_use]
    pub fn geometry(&self) -> &Geometry {
        &self.0.geometry
    }

    #[must_use]
    pub fn bounds(&self) -> RenderBounds {
        self.0.bounds
    }

    #[must_use]
    pub fn presentation(&self) -> Presentation {
        self.0.presentation
    }

    #[must_use]
    pub fn ancestry(&self) -> &[EntityId] {
        &self.0.ancestry
    }

    #[must_use]
    pub fn kind(&self) -> &LayerKind {
        &self.0.kind
    }

    #[must_use]
    pub fn dependencies(&self) -> &[RenderDependency] {
        &self.0.dependencies
    }

    #[must_use]
    pub fn raster_image(&self) -> Option<&RasterImage> {
        self.0.node.image.as_ref()
    }

    #[must_use]
    pub fn placement(&self) -> Affine {
        self.0.placement
    }

    #[must_use]
    pub fn element_frame(&self) -> Option<PreparedElementFrame> {
        element_frame(self)
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Presentation {
    pub visible: bool,
    pub opacity: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LayerKind {
    Image(ImageMetadata),
    Text(TextMetadata),
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ImageKind {
    Source,
    Cleanup,
    Paint,
    Embedded,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImageMetadata {
    pub name: Option<String>,
    pub kind: ImageKind,
}

#[derive(Clone)]
pub struct RasterImage {
    pub(crate) blob: BlobId,
    pub(crate) source: ResourceId,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) media_type: String,
    pub(crate) encoded: Arc<[u8]>,
    pub(crate) pixels: Arc<[u8]>,
}

impl RasterImage {
    #[must_use]
    pub fn blob(&self) -> BlobId {
        self.blob
    }

    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextMetadata {
    pub text: String,
    pub language: Option<LanguageTag>,
    pub rendered_bounds: RenderBounds,
    pub layout_bounds: RenderBounds,
    pub post_script_fonts: Vec<String>,
    pub font_size: f32,
    pub color: [u8; 4],
    pub alignment: TextAlign,
    pub writing_mode: WritingMode,
    pub angle_degrees: f32,
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct RetentionStats {
    pub candidate_layers: usize,
    pub reused_layers: usize,
    pub rebuilt_layers: usize,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RenderDependency {
    Entity(EntityId),
    Hierarchy(EntityId),
    Component { entity: EntityId, kind: String },
    Relation(RelationId),
    RelationQuery { source: EntityId, kind: String },
    RelationTargetQuery { target: EntityId, kind: String },
    Blob(BlobId),
    Font(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum RenderDiagnostic {
    MissingAsset {
        entity: EntityId,
        role: String,
    },
    TextOverflow {
        entity: EntityId,
        available: RenderBounds,
        actual_width: f32,
        actual_height: f32,
        font_size: f32,
    },
    TextBelowReadableSize {
        entity: EntityId,
        font_size: f32,
        minimum_font_size: f32,
    },
}

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct RenderBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl From<crate::bubble::LayoutBox> for RenderBounds {
    fn from(value: crate::bubble::LayoutBox) -> Self {
        Self {
            x: value.x,
            y: value.y,
            width: value.width,
            height: value.height,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NodeDescriptor {
    Image(ImageNodeDescriptor),
    Text(Box<crate::text_renderer::TextNodeDescriptor>),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ImageNodeDescriptor {
    pub(crate) blob: BlobId,
    pub(crate) media_type: String,
    pub(crate) expected_size: Option<(u32, u32)>,
    pub(crate) require_size: Option<(u32, u32)>,
}

pub(crate) struct RetainedNode {
    pub(crate) descriptor: NodeDescriptor,
    pub(crate) scene: Arc<koharu_rasterizer::PreparedScene>,
    pub(crate) resources: Arc<[PreparedResource]>,
    pub(crate) local_bounds: RenderBounds,
    pub(crate) image: Option<RasterImage>,
    pub(crate) text: Option<LocalTextMetadata>,
    pub(crate) diagnostics: Arc<[RenderDiagnostic]>,
}

#[derive(Clone)]
pub(crate) struct LocalTextMetadata {
    pub(crate) rendered_bounds: RenderBounds,
    pub(crate) layout_bounds: RenderBounds,
    pub(crate) post_script_fonts: Vec<String>,
    pub(crate) font_size: f32,
    pub(crate) color: [u8; 4],
}

pub(crate) fn prepare_frame(
    revision: Revision,
    page: EntityId,
    width: u32,
    height: u32,
    origin: (i32, i32),
    normalization: Affine,
    layers: &[Layer],
) -> Result<PreparedFrameBundle> {
    let mut resources = Vec::<PreparedResource>::new();
    let mut prepared_layers = Vec::with_capacity(layers.len());
    for layer in layers {
        let (kind, content) = if let Some(image) = layer.raster_image() {
            (
                PreparedLayerKind::Raster,
                PreparedContent::Raster(prepare_raster_tiles(image, &mut resources)?),
            )
        } else if matches!(layer.kind(), LayerKind::Text(_)) {
            for resource in layer.0.node.resources.iter() {
                if !resources
                    .iter()
                    .any(|candidate| candidate.id() == resource.id())
                {
                    resources.push(resource.clone());
                }
            }
            (
                PreparedLayerKind::Text,
                PreparedContent::Vector(layer.0.node.scene.as_ref().clone()),
            )
        } else {
            // Missing raster assets remain in native diagnostics and metadata but
            // contribute no visual command to the portable frame.
            continue;
        };
        prepared_layers.push(PreparedLayer {
            id: layer_id(layer.entity()),
            geometry: layer
                .geometry()
                .points
                .iter()
                .map(|point| PreparedPoint {
                    x: point.x,
                    y: point.y,
                })
                .collect(),
            bounds: prepared_bounds(layer.bounds()),
            local_bounds: prepared_bounds(layer.0.node.local_bounds),
            presentation: PreparedPresentation {
                visible: layer.presentation().visible,
                opacity: layer.presentation().opacity.clamp(0.0, 1.0),
            },
            kind,
            placement: layer.placement().as_coeffs(),
            content,
            element_frame: element_frame(layer),
        });
    }
    Ok(PreparedFrameBundle {
        frame: PreparedFrame {
            revision: PreparedRevision::new(revision.get()),
            page: layer_id(page),
            width,
            height,
            origin,
            normalization: normalization.as_coeffs(),
            layers: prepared_layers,
        },
        resources,
    })
}

fn prepare_raster_tiles(
    image: &RasterImage,
    resources: &mut Vec<PreparedResource>,
) -> Result<PreparedRaster> {
    let resource = PreparedResource::encoded_raster(
        image.width,
        image.height,
        image.media_type.clone(),
        Arc::clone(&image.encoded),
    )
    .map_err(|error| Error::Backend(anyhow!(error)))?;
    let source = resource.id();
    if !resources.iter().any(|candidate| candidate.id() == source) {
        resources.push(resource);
    }
    let mut tiles = Vec::new();
    for y in (0..image.height).step_by(PREPARED_RASTER_TILE_DIMENSION as usize) {
        let height = (image.height - y).min(PREPARED_RASTER_TILE_DIMENSION);
        for x in (0..image.width).step_by(PREPARED_RASTER_TILE_DIMENSION as usize) {
            let width = (image.width - x).min(PREPARED_RASTER_TILE_DIMENSION);
            let gutter = [
                u32::from(x > 0),
                u32::from(y > 0),
                u32::from(x + width < image.width),
                u32::from(y + height < image.height),
            ];
            tiles.push(PreparedRasterTile {
                x,
                y,
                width,
                height,
                gutter,
            });
        }
    }
    Ok(PreparedRaster {
        source,
        width: image.width,
        height: image.height,
        tiles,
    })
}

fn layer_id(entity: EntityId) -> LayerId {
    LayerId::from_bytes(*entity.as_uuid().as_bytes())
}

fn prepared_bounds(bounds: RenderBounds) -> PreparedBounds {
    PreparedBounds {
        x: bounds.x,
        y: bounds.y,
        width: bounds.width,
        height: bounds.height,
    }
}

fn element_frame(layer: &Layer) -> Option<PreparedElementFrame> {
    let LayerKind::Text(text) = layer.kind() else {
        return None;
    };
    let bounds = text.rendered_bounds;
    if bounds.width > 0.0 && bounds.height > 0.0 {
        return Some(PreparedElementFrame {
            x: bounds.x,
            y: bounds.y,
            width: bounds.width,
            height: bounds.height,
            angle_degrees: text.angle_degrees,
        });
    }
    geometry_frame(layer.geometry())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn element_frame_round_trips_a_committed_angle() {
        // The canvas control reads this frame, so losing the angle here is what
        // snaps a rotated text layer back upright.
        let geometry = Geometry::rotated_rectangle(60.0, 65.0, 80.0, 30.0, 27.0);

        let frame = geometry_frame(&geometry).unwrap();
        assert!((frame.x - 60.0).abs() < 1e-4);
        assert!((frame.y - 65.0).abs() < 1e-4);
        assert!((frame.width - 80.0).abs() < 1e-4);
        assert!((frame.height - 30.0).abs() < 1e-4);
        assert!((frame.angle_degrees - 27.0).abs() < 1e-4);
    }

    #[test]
    fn prepared_raster_tiles_retain_neighbor_gutters() {
        let width = PREPARED_RASTER_TILE_DIMENSION + 1;
        let mut pixels = Vec::with_capacity(width as usize * 4);
        for x in 0..width {
            pixels.extend_from_slice(&[(x & 0xff) as u8, (x >> 8) as u8, 0, 255]);
        }
        let encoded: Arc<[u8]> = Arc::from(&b"encoded-wide-image"[..]);
        let source = ResourceId::for_encoded_raster(width, 1, "image/png", &encoded);
        let image = RasterImage {
            blob: BlobId::for_bytes(&pixels),
            source,
            width,
            height: 1,
            media_type: "image/png".to_owned(),
            encoded,
            pixels: pixels.into(),
        };
        let mut resources = Vec::new();

        let raster = prepare_raster_tiles(&image, &mut resources).unwrap();

        assert_eq!(raster.tiles.len(), 2);
        assert_eq!(resources.len(), 1);
        assert_eq!(raster.source, source);
        assert_eq!(raster.tiles[0].gutter, [0, 0, 1, 0]);
        assert_eq!(raster.tiles[1].gutter, [1, 0, 0, 0]);
        assert_eq!(raster.tiles[0].resource_size(), (1_025, 1));
        assert_eq!(raster.tiles[1].resource_size(), (2, 1));
        let PreparedResource::EncodedRaster {
            width,
            height,
            bytes,
            ..
        } = &resources[0]
        else {
            unreachable!();
        };
        assert_eq!((*width, *height), (PREPARED_RASTER_TILE_DIMENSION + 1, 1));
        assert_eq!(bytes.as_ref(), b"encoded-wide-image");
    }
}

fn geometry_frame(geometry: &Geometry) -> Option<PreparedElementFrame> {
    let points = &geometry.points;
    if points.is_empty()
        || points
            .iter()
            .any(|point| !point.x.is_finite() || !point.y.is_finite())
    {
        return None;
    }
    if points.len() == 4 {
        let top = (points[1].x - points[0].x, points[1].y - points[0].y);
        let right = (points[2].x - points[1].x, points[2].y - points[1].y);
        let width = top.0.hypot(top.1);
        let height = right.0.hypot(right.1);
        if width > f64::EPSILON && height > f64::EPSILON {
            let center_x = points.iter().map(|point| point.x).sum::<f64>() * 0.25;
            let center_y = points.iter().map(|point| point.y).sum::<f64>() * 0.25;
            return Some(PreparedElementFrame {
                x: (center_x - width * 0.5) as f32,
                y: (center_y - height * 0.5) as f32,
                width: width as f32,
                height: height as f32,
                angle_degrees: top.1.atan2(top.0).to_degrees() as f32,
            });
        }
    }
    let (mut min_x, mut min_y) = (f64::INFINITY, f64::INFINITY);
    let (mut max_x, mut max_y) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for point in points {
        min_x = min_x.min(point.x);
        min_y = min_y.min(point.y);
        max_x = max_x.max(point.x);
        max_y = max_y.max(point.y);
    }
    let width = max_x - min_x;
    let height = max_y - min_y;
    (width > f64::EPSILON && height > f64::EPSILON).then_some(PreparedElementFrame {
        x: min_x as f32,
        y: min_y as f32,
        width: width as f32,
        height: height as f32,
        angle_degrees: 0.0,
    })
}
