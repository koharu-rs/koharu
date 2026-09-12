//! Typed, evolvable Koharu project semantics over [`koharu_storage`].
//!
//! The storage crate remains ignorant of pages, hierarchy, relations, text,
//! assets, and model provenance. This crate supplies those meanings through
//! independently versioned components and validates them at every project
//! snapshot boundary.

mod change;
mod component;
mod components;
mod document;
mod edit;
mod error;
mod glossary;
mod id;
mod patch;
mod schema;
mod semantics;
mod session;
mod snapshot;
mod state;

pub use change::{
    Change, ComponentChange, ComponentOwner, EntityChange, RelationChange, ValueChangeKind,
};
pub use component::{Component, ValidationContext};
pub use components::{
    Asset, AssetInput, AssetMetadata, AssetRole, Authored, DetectionAnalysis, DetectionLabel,
    EntityOrigin, FontStyle, Generation, Geometry, Group, LanguageTag, OcrAnalysis, Origin, Page,
    PageDraft, Point, Project, RasterLayer, RasterLayerKind, Region, RegionKind, Relation,
    RelationKind, SourceText, TextAlignment, TextContent, TextDirection, TextGroup, TextLayout,
    TextLayoutKind, TextRole, Translation, Typography, Visibility, WritingMode,
};
pub use document::{AnalysisRegionRef, GroupRef, TextContentRef, TextLayerRef};
pub use edit::{At, Edit, RemovePolicy};
pub use error::{Error, Result};
pub use glossary::{
    GlossaryCandidate, GlossaryCategory, GlossaryEntry, GlossaryMatch, ProjectGlossary,
    glossary_matches, normalize_term, normalize_text, relevant_glossary,
};
pub use id::{EntityId, ProducerId, ProjectId, RelationId};
pub use patch::Patch;
pub use semantics::{
    BubbleRegion, FitsTo, FlowsIn, FunctionalRelation, Inside, PanelRegion, Presents,
    RecognizedFrom, RegionSpec, RelationSpec, TextRegion,
};
pub use session::{Commit, Session};
pub use snapshot::{EntityRef, PageRef, RelationRef, Snapshot};

pub use koharu_storage::{BlobId, PatchId, Revision};

#[cfg(test)]
mod tests;
