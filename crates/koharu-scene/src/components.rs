//! Persisted component values grouped by the responsibility that owns them.
//!
//! This module contains data and local validation only. Cross-component
//! document rules belong to `schema`; resolved views and edit intents belong
//! to `document`.

mod analysis;
mod assets;
mod groups;
mod layers;
mod provenance;
mod spatial;
mod structure;
mod text;

pub use analysis::{
    DetectionAnalysis, DetectionLabel, OcrAnalysis, Region, RegionKind, TextDirection,
};
pub(crate) use assets::Assets;
pub use assets::{Asset, AssetInput, AssetMetadata, AssetRole};
pub use groups::{Group, TextGroup};
pub use layers::{
    FontStyle, RasterLayer, RasterLayerKind, TextAlignment, TextLayout, TextLayoutKind, Typography,
    WritingMode,
};
pub use provenance::{Authored, Generation, Origin};
pub use spatial::{Geometry, Point, Visibility};
pub use structure::{
    EntityOrigin, Page, PageDraft, PageSlice, PageSliceDraft, Project, Relation, RelationKind,
    SliceSource, SliceSourceInput,
};
pub use text::{LanguageTag, SourceText, TextContent, TextRole, Translation};
