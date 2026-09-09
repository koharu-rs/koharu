//! Versioned, backend-neutral display data.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

const MANIFEST_MAGIC: [u8; 8] = *b"KHRMANF\0";
const RESOURCE_MAGIC: [u8; 8] = *b"KHRRSRC\0";
pub const PREPARED_FRAME_MANIFEST_VERSION: u16 = 4;
pub const PREPARED_RESOURCE_FORMAT_VERSION: u16 = 3;
/// Logical raster tile edge used by portable frames. The one-pixel sampling
/// gutter is stored outside this logical extent where adjacent pixels exist.
pub const PREPARED_RASTER_TILE_DIMENSION: u32 = 1_024;
const MAX_SURFACE_DIMENSION: u32 = 32_768;
const MAX_SURFACE_PIXELS: u64 = 268_435_456;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LayerId([u8; 16]);

impl LayerId {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl std::fmt::Display for LayerId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Revision(u64);

impl Revision {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ResourceId([u8; 32]);

impl ResourceId {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    #[must_use]
    pub fn for_font(bytes: &[u8]) -> Self {
        Self::hash(b"koharu-prepared-font-v1\0", &[], bytes)
    }

    #[must_use]
    pub fn for_encoded_raster(width: u32, height: u32, media_type: &str, bytes: &[u8]) -> Self {
        let mut metadata = Vec::with_capacity(9 + media_type.len());
        metadata.extend_from_slice(&width.to_le_bytes());
        metadata.extend_from_slice(&height.to_le_bytes());
        metadata.extend_from_slice(media_type.as_bytes());
        metadata.push(0);
        Self::hash(b"koharu-prepared-encoded-raster-v1\0", &metadata, bytes)
    }

    #[must_use]
    pub fn for_raster_tile(source: Self, tile: PreparedRasterTile) -> Self {
        let mut metadata = [0; 56];
        metadata[..32].copy_from_slice(source.as_bytes());
        metadata[32..36].copy_from_slice(&tile.x.to_le_bytes());
        metadata[36..40].copy_from_slice(&tile.y.to_le_bytes());
        metadata[40..44].copy_from_slice(&tile.width.to_le_bytes());
        metadata[44..48].copy_from_slice(&tile.height.to_le_bytes());
        for (index, gutter) in tile.gutter.into_iter().enumerate() {
            metadata[48 + index * 2..50 + index * 2]
                .copy_from_slice(&(gutter as u16).to_le_bytes());
        }
        Self::hash(b"koharu-prepared-raster-tile-v1\0", &metadata, &[])
    }

    fn hash(domain: &[u8], metadata: &[u8], bytes: &[u8]) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(domain);
        hasher.update(metadata);
        hasher.update(bytes);
        Self(*hasher.finalize().as_bytes())
    }
}

impl std::fmt::Display for ResourceId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl std::str::FromStr for ResourceId {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        if value.len() != 64 {
            return Err(Error::invalid(
                "resource id must contain exactly 64 hexadecimal characters",
            ));
        }
        let mut bytes = [0; 32];
        for (index, pair) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
            let high = hex_nibble(pair[0])?;
            let low = hex_nibble(pair[1])?;
            bytes[index] = high << 4 | low;
        }
        Ok(Self(bytes))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreparedFrameBundle {
    pub frame: PreparedFrame,
    pub resources: Vec<PreparedResource>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreparedFrameManifest {
    pub frame: PreparedFrame,
    pub resources: Vec<PreparedResourceRef>,
}

#[derive(Serialize)]
struct ManifestPacketRef<'a> {
    magic: [u8; 8],
    version: u16,
    manifest: &'a PreparedFrameManifest,
}

#[derive(Deserialize, Serialize)]
struct ManifestPacket {
    magic: [u8; 8],
    version: u16,
    manifest: PreparedFrameManifest,
}

#[derive(Serialize)]
struct ResourcePacketRef<'a> {
    magic: [u8; 8],
    version: u16,
    resource: &'a PreparedResource,
}

#[derive(Deserialize, Serialize)]
struct ResourcePacket {
    magic: [u8; 8],
    version: u16,
    resource: PreparedResource,
}

/// One independently validated, content-addressed resource packet.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedResourcePacket {
    resource: PreparedResource,
}

#[derive(Debug, Default)]
pub struct PreparedResourceStore {
    resources: HashMap<ResourceId, PreparedResourcePacket>,
    total_bytes: u64,
}

impl PreparedFrameBundle {
    pub fn manifest(&self) -> Result<PreparedFrameManifest> {
        let manifest = PreparedFrameManifest {
            frame: self.frame.clone(),
            resources: self
                .resources
                .iter()
                .map(PreparedResource::reference)
                .collect(),
        };
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn resource_packet(&self, id: ResourceId) -> Result<PreparedResourcePacket> {
        let resource = self
            .resources
            .iter()
            .find(|resource| resource.id() == id)
            .cloned()
            .ok_or_else(|| Error::invalid(format!("prepared frame has no resource {id}")))?;
        PreparedResourcePacket::from_resource(resource)
    }

    pub fn validate(&self) -> Result<()> {
        for resource in &self.resources {
            resource.validate()?;
        }
        self.manifest().map(|_| ())
    }
}

impl PreparedFrameManifest {
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        postcard::to_stdvec(&ManifestPacketRef {
            magic: MANIFEST_MAGIC,
            version: PREPARED_FRAME_MANIFEST_VERSION,
            manifest: self,
        })
        .map_err(Error::Encode)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let packet: ManifestPacket = postcard::from_bytes(bytes).map_err(Error::Decode)?;
        if packet.magic != MANIFEST_MAGIC {
            return Err(Error::invalid(
                "prepared frame manifest magic does not match",
            ));
        }
        if packet.version != PREPARED_FRAME_MANIFEST_VERSION {
            return Err(Error::UnsupportedManifestVersion(packet.version));
        }
        packet.manifest.validate()?;
        Ok(packet.manifest)
    }

    #[must_use]
    pub fn required_resources(&self) -> &[PreparedResourceRef] {
        &self.resources
    }

    #[must_use]
    pub fn missing_resources(&self, store: &PreparedResourceStore) -> Vec<ResourceId> {
        store.missing_resources(self)
    }

    pub fn compile(&self, store: &PreparedResourceStore) -> Result<crate::Frame> {
        store.compile(self)
    }

    pub fn validate(&self) -> Result<()> {
        validate_frame(&self.frame, &self.resources)
    }
}

impl PreparedResourcePacket {
    pub fn from_resource(resource: PreparedResource) -> Result<Self> {
        resource.validate()?;
        Ok(Self { resource })
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        postcard::to_stdvec(&ResourcePacketRef {
            magic: RESOURCE_MAGIC,
            version: PREPARED_RESOURCE_FORMAT_VERSION,
            resource: &self.resource,
        })
        .map_err(Error::Encode)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let packet: ResourcePacket = postcard::from_bytes(bytes).map_err(Error::Decode)?;
        if packet.magic != RESOURCE_MAGIC {
            return Err(Error::invalid("prepared resource magic does not match"));
        }
        if packet.version != PREPARED_RESOURCE_FORMAT_VERSION {
            return Err(Error::UnsupportedResourceVersion(packet.version));
        }
        Self::from_resource(packet.resource)
    }

    #[must_use]
    pub const fn id(&self) -> ResourceId {
        self.resource.id()
    }

    #[must_use]
    pub fn kind(&self) -> PreparedResourceKind {
        self.resource.kind()
    }

    #[must_use]
    pub fn byte_len(&self) -> u64 {
        self.resource.byte_len()
    }

    #[must_use]
    pub fn resource(&self) -> &PreparedResource {
        &self.resource
    }
}

impl PreparedResourceStore {
    pub fn insert(&mut self, packet: PreparedResourcePacket) -> Option<PreparedResourcePacket> {
        let id = packet.id();
        let previous = self.resources.insert(id, packet);
        self.total_bytes = self
            .total_bytes
            .saturating_add(self.resources[&id].byte_len());
        if let Some(previous) = &previous {
            self.total_bytes = self.total_bytes.saturating_sub(previous.byte_len());
        }
        previous
    }

    pub fn insert_packet(&mut self, bytes: &[u8]) -> Result<ResourceId> {
        let packet = PreparedResourcePacket::decode(bytes)?;
        let id = packet.id();
        self.insert(packet);
        Ok(id)
    }

    #[must_use]
    pub fn contains(&self, id: ResourceId) -> bool {
        self.resources.contains_key(&id)
    }

    #[must_use]
    pub fn get(&self, id: ResourceId) -> Option<&PreparedResourcePacket> {
        self.resources.get(&id)
    }

    pub fn remove(&mut self, id: ResourceId) -> Option<PreparedResourcePacket> {
        let removed = self.resources.remove(&id)?;
        self.total_bytes = self.total_bytes.saturating_sub(removed.byte_len());
        Some(removed)
    }

    pub fn clear(&mut self) {
        self.resources.clear();
        self.total_bytes = 0;
    }

    #[must_use]
    pub const fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    #[must_use]
    pub fn missing_resources(&self, manifest: &PreparedFrameManifest) -> Vec<ResourceId> {
        manifest
            .required_resources()
            .iter()
            .filter_map(|resource| (!self.contains(resource.id)).then_some(resource.id))
            .collect()
    }

    pub fn compile(&self, manifest: &PreparedFrameManifest) -> Result<crate::Frame> {
        manifest.validate()?;
        crate::frame::compile_manifest(manifest, self)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreparedFrame {
    pub revision: Revision,
    pub page: LayerId,
    pub width: u32,
    pub height: u32,
    pub origin: (i32, i32),
    /// Kurbo affine coefficients `[a, b, c, d, e, f]`.
    pub normalization: [f64; 6],
    pub layers: Vec<PreparedLayer>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreparedLayer {
    pub id: LayerId,
    pub geometry: Vec<Point>,
    pub bounds: Bounds,
    pub local_bounds: Bounds,
    pub presentation: Presentation,
    pub kind: LayerKind,
    /// Kurbo affine coefficients `[a, b, c, d, e, f]`.
    pub placement: [f64; 6],
    pub content: PreparedContent,
    pub element_frame: Option<PreparedElementFrame>,
}

impl PreparedLayer {
    #[must_use]
    pub const fn element_frame(&self) -> Option<PreparedElementFrame> {
        self.element_frame
    }

    fn validate(
        &self,
        resources: &HashMap<ResourceId, &PreparedResourceRef>,
        referenced: &mut HashSet<ResourceId>,
    ) -> Result<()> {
        if self.geometry.len() < 3
            || self
                .geometry
                .iter()
                .any(|point| !point.x.is_finite() || !point.y.is_finite())
        {
            return Err(Error::invalid("layer geometry is invalid"));
        }
        finite_bounds(self.bounds, "layer bounds")?;
        finite_bounds(self.local_bounds, "layer local bounds")?;
        finite_affine(self.placement, "layer placement")?;
        if !self.presentation.opacity.is_finite()
            || !(0.0..=1.0).contains(&self.presentation.opacity)
        {
            return Err(Error::invalid("layer opacity is outside zero through one"));
        }
        if self.element_frame.is_some_and(|frame| !frame.is_valid()) {
            return Err(Error::invalid("layer element frame is invalid"));
        }
        match (&self.kind, &self.content) {
            (LayerKind::Raster, PreparedContent::Raster(raster)) => {
                raster.validate(resources, referenced)?;
            }
            (LayerKind::Text, PreparedContent::Vector(scene)) => {
                scene.validate(resources, referenced)?;
            }
            _ => {
                return Err(Error::invalid(
                    "prepared layer kind and content do not match",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum LayerKind {
    Raster,
    Text,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Presentation {
    pub visible: bool,
    pub opacity: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreparedElementFrame {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub angle_degrees: f32,
}

impl PreparedElementFrame {
    #[must_use]
    pub fn is_valid(self) -> bool {
        [self.x, self.y, self.width, self.height, self.angle_degrees]
            .into_iter()
            .all(f32::is_finite)
            && self.width > 0.0
            && self.height > 0.0
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PreparedContent {
    Raster(PreparedRaster),
    Vector(PreparedScene),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreparedRaster {
    pub source: ResourceId,
    pub width: u32,
    pub height: u32,
    pub tiles: Vec<PreparedRasterTile>,
}

impl PreparedRaster {
    fn validate(
        &self,
        resources: &HashMap<ResourceId, &PreparedResourceRef>,
        referenced: &mut HashSet<ResourceId>,
    ) -> Result<()> {
        validate_surface(self.width, self.height)?;
        let Some(PreparedResourceRef {
            kind:
                PreparedResourceKind::EncodedRaster {
                    width: source_width,
                    height: source_height,
                },
            ..
        }) = resources.get(&self.source).copied()
        else {
            return Err(Error::invalid(
                "raster references a missing encoded source resource",
            ));
        };
        if (*source_width, *source_height) != (self.width, self.height) {
            return Err(Error::invalid(
                "encoded raster source dimensions do not match the raster",
            ));
        }
        referenced.insert(self.source);
        let columns = self.width.div_ceil(PREPARED_RASTER_TILE_DIMENSION);
        let rows = self.height.div_ceil(PREPARED_RASTER_TILE_DIMENSION);
        if usize::try_from(u64::from(columns) * u64::from(rows)).ok() != Some(self.tiles.len()) {
            return Err(Error::invalid("raster tile count does not cover the image"));
        }
        for (index, tile) in self.tiles.iter().enumerate() {
            let index = u32::try_from(index)
                .map_err(|_| Error::invalid("raster tile index exceeds u32"))?;
            let column = index % columns;
            let row = index / columns;
            let x = column * PREPARED_RASTER_TILE_DIMENSION;
            let y = row * PREPARED_RASTER_TILE_DIMENSION;
            let width = (self.width - x).min(PREPARED_RASTER_TILE_DIMENSION);
            let height = (self.height - y).min(PREPARED_RASTER_TILE_DIMENSION);
            let gutter = [
                u32::from(x > 0),
                u32::from(y > 0),
                u32::from(x + width < self.width),
                u32::from(y + height < self.height),
            ];
            if (tile.x, tile.y, tile.width, tile.height, tile.gutter)
                != (x, y, width, height, gutter)
            {
                return Err(Error::invalid("raster tiles are not in canonical order"));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct PreparedRasterTile {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// Left, top, right, and bottom pixels retained for filtered sampling.
    pub gutter: [u32; 4],
}

impl PreparedRasterTile {
    #[must_use]
    pub fn id(self, source: ResourceId) -> ResourceId {
        ResourceId::for_raster_tile(source, self)
    }

    #[must_use]
    pub const fn source_origin(self) -> (u32, u32) {
        (self.x - self.gutter[0], self.y - self.gutter[1])
    }

    #[must_use]
    pub const fn resource_size(self) -> (u32, u32) {
        (
            self.width + self.gutter[0] + self.gutter[2],
            self.height + self.gutter[1] + self.gutter[3],
        )
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PreparedScene {
    pub commands: Vec<PreparedSceneCommand>,
}

impl PreparedScene {
    fn validate(
        &self,
        resources: &HashMap<ResourceId, &PreparedResourceRef>,
        referenced: &mut HashSet<ResourceId>,
    ) -> Result<()> {
        for command in &self.commands {
            match command {
                PreparedSceneCommand::GlyphRun(run) => {
                    let Some(PreparedResourceRef {
                        kind: PreparedResourceKind::Font,
                        ..
                    }) = resources.get(&run.font).copied()
                    else {
                        return Err(Error::invalid(
                            "glyph run references a missing font resource",
                        ));
                    };
                    referenced.insert(run.font);
                    run.validate()?;
                }
                PreparedSceneCommand::FillPath(path) => path.validate()?,
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PreparedSceneCommand {
    GlyphRun(PreparedGlyphRun),
    FillPath(PreparedPath),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreparedGlyphRun {
    pub font: ResourceId,
    pub font_index: u32,
    pub font_size: f32,
    pub normalized_coords: Vec<i16>,
    pub transform: [f64; 6],
    pub glyph_transform: Option<[f64; 6]>,
    pub hint: bool,
    pub embolden: [f32; 2],
    pub color: [u8; 4],
    pub glyphs: Vec<PreparedGlyph>,
}

impl PreparedGlyphRun {
    fn validate(&self) -> Result<()> {
        if !self.font_size.is_finite() || self.font_size <= 0.0 {
            return Err(Error::invalid("glyph run font size is invalid"));
        }
        finite_affine(self.transform, "glyph run transform")?;
        if let Some(transform) = self.glyph_transform {
            finite_affine(transform, "glyph transform")?;
        }
        if self
            .embolden
            .iter()
            .any(|strength| !strength.is_finite() || *strength < 0.0)
        {
            return Err(Error::invalid("glyph embolden strength is invalid"));
        }
        if self
            .glyphs
            .iter()
            .any(|glyph| !glyph.x.is_finite() || !glyph.y.is_finite())
        {
            return Err(Error::invalid("glyph run contains a non-finite position"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreparedGlyph {
    pub id: u32,
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreparedPath {
    pub fill: FillRule,
    pub transform: [f64; 6],
    pub color: [u8; 4],
    pub elements: Vec<PathElement>,
}

impl PreparedPath {
    fn validate(&self) -> Result<()> {
        finite_affine(self.transform, "path transform")?;
        if self.elements.is_empty() {
            return Err(Error::invalid("prepared path is empty"));
        }
        for element in &self.elements {
            let finite = match element {
                PathElement::MoveTo(values) | PathElement::LineTo(values) => {
                    values.iter().all(|value| value.is_finite())
                }
                PathElement::QuadTo(values) => values.iter().all(|value| value.is_finite()),
                PathElement::CurveTo(values) => values.iter().all(|value| value.is_finite()),
                PathElement::Close => true,
            };
            if !finite {
                return Err(Error::invalid("prepared path contains a non-finite point"));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PathElement {
    MoveTo([f64; 2]),
    LineTo([f64; 2]),
    QuadTo([f64; 4]),
    CurveTo([f64; 6]),
    Close,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum PreparedResourceKind {
    Font,
    EncodedRaster { width: u32, height: u32 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PreparedResourceRef {
    pub id: ResourceId,
    pub kind: PreparedResourceKind,
    pub byte_len: u64,
}

impl PreparedResourceRef {
    fn validate(self) -> Result<()> {
        match self.kind {
            PreparedResourceKind::Font if self.byte_len == 0 => {
                Err(Error::invalid("font resource reference is empty"))
            }
            PreparedResourceKind::Font => Ok(()),
            PreparedResourceKind::EncodedRaster { width, height } => {
                validate_surface(width, height)?;
                if self.byte_len == 0 {
                    Err(Error::invalid("encoded raster resource reference is empty"))
                } else {
                    Ok(())
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PreparedResource {
    Font {
        id: ResourceId,
        bytes: Arc<[u8]>,
    },
    EncodedRaster {
        id: ResourceId,
        width: u32,
        height: u32,
        media_type: String,
        bytes: Arc<[u8]>,
    },
}

impl PreparedResource {
    #[must_use]
    pub const fn id(&self) -> ResourceId {
        match self {
            Self::Font { id, .. } | Self::EncodedRaster { id, .. } => *id,
        }
    }

    #[must_use]
    pub const fn kind(&self) -> PreparedResourceKind {
        match self {
            Self::Font { .. } => PreparedResourceKind::Font,
            Self::EncodedRaster { width, height, .. } => PreparedResourceKind::EncodedRaster {
                width: *width,
                height: *height,
            },
        }
    }

    #[must_use]
    pub fn byte_len(&self) -> u64 {
        let length = match self {
            Self::Font { bytes, .. } => bytes.len(),
            Self::EncodedRaster { bytes, .. } => bytes.len(),
        };
        u64::try_from(length).unwrap_or(u64::MAX)
    }

    #[must_use]
    pub fn reference(&self) -> PreparedResourceRef {
        PreparedResourceRef {
            id: self.id(),
            kind: self.kind(),
            byte_len: self.byte_len(),
        }
    }

    #[must_use]
    pub fn font(bytes: &[u8]) -> Self {
        Self::font_shared(Arc::from(bytes))
    }

    #[must_use]
    pub fn font_shared(bytes: Arc<[u8]>) -> Self {
        Self::Font {
            id: ResourceId::for_font(&bytes),
            bytes,
        }
    }

    pub fn encoded_raster(
        width: u32,
        height: u32,
        media_type: impl Into<String>,
        bytes: Arc<[u8]>,
    ) -> Result<Self> {
        let media_type = media_type.into();
        let resource = Self::EncodedRaster {
            id: ResourceId::for_encoded_raster(width, height, &media_type, &bytes),
            width,
            height,
            media_type,
            bytes,
        };
        resource.validate()?;
        Ok(resource)
    }

    fn validate(&self) -> Result<()> {
        match self {
            Self::Font { id, bytes } => {
                if bytes.is_empty() {
                    return Err(Error::invalid("font resource is empty"));
                }
                if *id != ResourceId::for_font(bytes) {
                    return Err(Error::invalid("font resource content id does not match"));
                }
            }
            Self::EncodedRaster {
                id,
                width,
                height,
                media_type,
                bytes,
            } => {
                validate_surface(*width, *height)?;
                if bytes.is_empty() {
                    return Err(Error::invalid("encoded raster resource is empty"));
                }
                if media_type.is_empty()
                    || media_type.len() > 255
                    || !media_type.contains('/')
                    || media_type.contains('\0')
                {
                    return Err(Error::invalid("encoded raster media type is invalid"));
                }
                if *id != ResourceId::for_encoded_raster(*width, *height, media_type, bytes) {
                    return Err(Error::invalid(
                        "encoded raster resource content id does not match",
                    ));
                }
            }
        }
        Ok(())
    }
}

fn validate_frame(frame: &PreparedFrame, resource_refs: &[PreparedResourceRef]) -> Result<()> {
    validate_surface(frame.width, frame.height)?;
    finite_affine(frame.normalization, "frame normalization")?;
    let mut resources = HashMap::with_capacity(resource_refs.len());
    for resource in resource_refs {
        resource.validate()?;
        if resources.insert(resource.id, resource).is_some() {
            return Err(Error::invalid(
                "prepared frame manifest repeats a resource id",
            ));
        }
    }
    let mut layers = HashSet::with_capacity(frame.layers.len());
    let mut referenced = HashSet::new();
    for layer in &frame.layers {
        if !layers.insert(layer.id) {
            return Err(Error::invalid("prepared frame repeats a layer id"));
        }
        layer.validate(&resources, &mut referenced)?;
    }
    if referenced.len() != resources.len() {
        return Err(Error::invalid(
            "prepared frame manifest contains unreferenced resources",
        ));
    }
    Ok(())
}

fn hex_nibble(byte: u8) -> Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(Error::invalid(
            "resource id contains a non-hexadecimal character",
        )),
    }
}

fn validate_surface(width: u32, height: u32) -> Result<()> {
    if width == 0
        || height == 0
        || width > MAX_SURFACE_DIMENSION
        || height > MAX_SURFACE_DIMENSION
        || u64::from(width) * u64::from(height) > MAX_SURFACE_PIXELS
    {
        return Err(Error::invalid(format!(
            "invalid prepared surface {width}x{height}"
        )));
    }
    Ok(())
}

fn finite_bounds(bounds: Bounds, label: &str) -> Result<()> {
    if [bounds.x, bounds.y, bounds.width, bounds.height]
        .into_iter()
        .all(f32::is_finite)
        && bounds.width >= 0.0
        && bounds.height >= 0.0
    {
        Ok(())
    } else {
        Err(Error::invalid(format!("{label} is invalid")))
    }
}

fn finite_affine(affine: [f64; 6], label: &str) -> Result<()> {
    if affine.into_iter().all(f64::is_finite) {
        Ok(())
    } else {
        Err(Error::invalid(format!(
            "{label} contains a non-finite value"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle() -> PreparedFrameBundle {
        let resource =
            PreparedResource::encoded_raster(1, 1, "image/png", Arc::from(&b"png"[..])).unwrap();
        let source = resource.id();
        PreparedFrameBundle {
            frame: PreparedFrame {
                revision: Revision::new(9),
                page: LayerId::from_bytes([1; 16]),
                width: 100,
                height: 80,
                origin: (0, 0),
                normalization: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                layers: vec![PreparedLayer {
                    id: LayerId::from_bytes([2; 16]),
                    geometry: vec![
                        Point { x: 0.0, y: 0.0 },
                        Point { x: 1.0, y: 0.0 },
                        Point { x: 1.0, y: 1.0 },
                        Point { x: 0.0, y: 1.0 },
                    ],
                    bounds: Bounds {
                        x: 0.0,
                        y: 0.0,
                        width: 1.0,
                        height: 1.0,
                    },
                    local_bounds: Bounds {
                        x: 0.0,
                        y: 0.0,
                        width: 1.0,
                        height: 1.0,
                    },
                    presentation: Presentation {
                        visible: true,
                        opacity: 1.0,
                    },
                    kind: LayerKind::Raster,
                    placement: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                    content: PreparedContent::Raster(PreparedRaster {
                        source,
                        width: 1,
                        height: 1,
                        tiles: vec![PreparedRasterTile {
                            x: 0,
                            y: 0,
                            width: 1,
                            height: 1,
                            gutter: [0; 4],
                        }],
                    }),
                    element_frame: None,
                }],
            },
            resources: vec![resource],
        }
    }

    #[test]
    fn manifest_and_resource_packets_round_trip_independently() {
        let bundle = bundle();
        let expected_manifest = bundle.manifest().unwrap();
        let manifest = PreparedFrameManifest::decode(&expected_manifest.encode().unwrap()).unwrap();
        assert_eq!(manifest, expected_manifest);

        let resource_id = bundle.resources[0].id();
        let packet = bundle.resource_packet(resource_id).unwrap();
        let packet = PreparedResourcePacket::decode(&packet.encode().unwrap()).unwrap();
        assert_eq!(packet.id(), resource_id);
        assert_eq!(
            manifest.missing_resources(&PreparedResourceStore::default()),
            vec![resource_id]
        );

        let mut store = PreparedResourceStore::default();
        store.insert(packet);
        assert!(manifest.missing_resources(&store).is_empty());
        assert_eq!(store.total_bytes(), 3);
        let frame = manifest.compile(&store).unwrap();
        assert_eq!(
            frame.layers()[0].raster_image().unwrap().source(),
            resource_id
        );
        assert_eq!(
            frame.layers()[0].raster_image().unwrap().tiles()[0].id(),
            PreparedRasterTile {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
                gutter: [0; 4],
            }
            .id(resource_id)
        );
    }

    #[test]
    fn resource_packet_checks_content_id_once_at_decode_boundary() {
        let mut resource = bundle().resources.remove(0);
        let PreparedResource::EncodedRaster { bytes, .. } = &mut resource else {
            unreachable!()
        };
        Arc::make_mut(bytes)[0] = 0;
        let bytes = postcard::to_stdvec(&ResourcePacket {
            magic: RESOURCE_MAGIC,
            version: PREPARED_RESOURCE_FORMAT_VERSION,
            resource,
        })
        .unwrap();
        assert!(matches!(
            PreparedResourcePacket::decode(&bytes),
            Err(Error::Invalid(_))
        ));
    }

    #[test]
    fn manifest_tracks_one_encoded_source_for_all_raster_tiles() {
        let source = PreparedResource::encoded_raster(
            PREPARED_RASTER_TILE_DIMENSION + 1,
            1,
            "image/png",
            Arc::from(&b"wide-png"[..]),
        )
        .unwrap();
        let mut bundle = bundle();
        bundle.resources = vec![source.clone()];
        bundle.frame.width = PREPARED_RASTER_TILE_DIMENSION + 1;
        bundle.frame.layers[0].content = PreparedContent::Raster(PreparedRaster {
            source: source.id(),
            width: PREPARED_RASTER_TILE_DIMENSION + 1,
            height: 1,
            tiles: vec![
                PreparedRasterTile {
                    x: 0,
                    y: 0,
                    width: PREPARED_RASTER_TILE_DIMENSION,
                    height: 1,
                    gutter: [0, 0, 1, 0],
                },
                PreparedRasterTile {
                    x: PREPARED_RASTER_TILE_DIMENSION,
                    y: 0,
                    width: 1,
                    height: 1,
                    gutter: [1, 0, 0, 0],
                },
            ],
        });

        let manifest = bundle.manifest().unwrap();
        assert_eq!(manifest.required_resources().len(), 1);
        let mut store = PreparedResourceStore::default();
        assert_eq!(manifest.missing_resources(&store), vec![source.id()]);
        store.insert(bundle.resource_packet(source.id()).unwrap());
        let frame = manifest.compile(&store).unwrap();
        let tiles = frame.layers()[0].raster_image().unwrap().tiles();
        assert_eq!(tiles.len(), 2);
        assert_eq!(tiles[0].resource_size(), (1_025, 1));
        assert_eq!(tiles[1].resource_size(), (2, 1));

        let PreparedContent::Raster(raster) = &mut bundle.frame.layers[0].content else {
            unreachable!();
        };
        raster.tiles[1].gutter = [0; 4];
        assert!(matches!(bundle.manifest(), Err(Error::Invalid(_))));
    }

    #[test]
    fn packets_reject_unknown_versions() {
        let manifest = bundle().manifest().unwrap();
        let bytes = postcard::to_stdvec(&ManifestPacket {
            magic: MANIFEST_MAGIC,
            version: PREPARED_FRAME_MANIFEST_VERSION + 1,
            manifest,
        })
        .unwrap();
        assert!(matches!(
            PreparedFrameManifest::decode(&bytes),
            Err(Error::UnsupportedManifestVersion(version))
                if version == PREPARED_FRAME_MANIFEST_VERSION + 1
        ));

        let resource = bundle().resources.remove(0);
        let bytes = postcard::to_stdvec(&ResourcePacket {
            magic: RESOURCE_MAGIC,
            version: PREPARED_RESOURCE_FORMAT_VERSION + 1,
            resource,
        })
        .unwrap();
        assert!(matches!(
            PreparedResourcePacket::decode(&bytes),
            Err(Error::UnsupportedResourceVersion(version))
                if version == PREPARED_RESOURCE_FORMAT_VERSION + 1
        ));
    }

    #[test]
    fn resource_id_hex_is_canonical_and_strict() {
        let id = ResourceId::from_bytes([0xab; 32]);
        let text = id.to_string();
        assert_eq!(text, "ab".repeat(32));
        assert_eq!(text.parse::<ResourceId>().unwrap(), id);
        assert_eq!(text.to_ascii_uppercase().parse::<ResourceId>().unwrap(), id);
        assert!("ab".parse::<ResourceId>().is_err());
    }
}
