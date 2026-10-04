use std::{str::FromStr, sync::Arc};

use revision::revisioned;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::{
    BlobId, EntityId, Error, Result,
    component::{Component, ValidationContext},
    id::validate_namespaced,
};

use super::{AssetInput, LanguageTag, Origin};

#[revisioned(revision = 1)]
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize, Type)]
pub struct Project {
    pub source_locale: Option<LanguageTag>,
    pub target_locales: Vec<LanguageTag>,
}

impl Component for Project {
    const KIND: &'static str = "dev.koharu.project";

    fn validate(&self, _context: &ValidationContext<'_>) -> Result<()> {
        if let Some(locale) = &self.source_locale {
            locale.validate()?;
        }
        for locale in &self.target_locales {
            locale.validate()?;
        }
        let mut targets = self.target_locales.clone();
        targets.sort();
        targets.dedup();
        if targets.len() == self.target_locales.len() {
            Ok(())
        } else {
            Err(Error::invalid("target locales contain duplicates"))
        }
    }
}

#[revisioned(revision = 1)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct Page {
    pub label: String,
    pub width: f64,
    pub height: f64,
}

impl Component for Page {
    const KIND: &'static str = "dev.koharu.page";

    fn validate(&self, _context: &ValidationContext<'_>) -> Result<()> {
        if self.label.len() <= 4096
            && !self.label.contains('\0')
            && self.width.is_finite()
            && self.height.is_finite()
            && self.width > 0.0
            && self.height > 0.0
        {
            Ok(())
        } else {
            Err(Error::invalid("page label or dimensions are invalid"))
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PageDraft {
    pub label: String,
    pub width: f64,
    pub height: f64,
}

impl PageDraft {
    #[must_use]
    pub fn new(label: impl Into<String>, width: f64, height: f64) -> Self {
        Self {
            label: label.into(),
            width,
            height,
        }
    }
}

impl From<PageDraft> for Page {
    fn from(value: PageDraft) -> Self {
        Self {
            label: value.label,
            width: value.width,
            height: value.height,
        }
    }
}

/// The uncut image a page was banded out of.
///
/// Provenance is anchored to the immutable source blob rather than to the page entity the
/// image was first imported as. Slicing is a pure decomposition of pixels, so a band must stay
/// traceable to the exact bytes it came from even after the user deletes that original page;
/// an `EntityId` would dangle there and force the scene kernel to police referential integrity
/// across page lifetimes. The accepted cost is that a band cannot enumerate its siblings from
/// its own component: consumers group bands by this blob, and the `slice-of` relation links a
/// band back to the source page for as long as that page is alive.
///
/// The uncut image is kept even when no page displays it, because a chapter imported as bands
/// is otherwise impossible to re-cut; the blob stays pinned by the bands that name it.
#[revisioned(revision = 1)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct SliceSource {
    pub blob: BlobId,
    pub width: f64,
    pub height: f64,
}

impl SliceSource {
    #[must_use]
    pub fn new(blob: BlobId, width: f64, height: f64) -> Self {
        Self {
            blob,
            width,
            height,
        }
    }
}

/// Marks a page as one vertical band of a taller source image.
///
/// Detection, OCR, translation, and typesetting consume pages without knowing that webtoons
/// exist, so slicing happens entirely at import: this component is the only record that the
/// page is a band, and it exists to answer "which pixels is this page cut from".
#[revisioned(revision = 1)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct PageSlice {
    pub source: SliceSource,
    /// Distance from the top of the source image to the top of this band.
    pub y_offset: f64,
    pub slice_height: f64,
}

impl Component for PageSlice {
    const KIND: &'static str = "dev.koharu.page.slice";

    /// Keeps an uncut strip alive for re-banding for as long as any band still names it,
    /// independently of whether an uncut page survives.
    fn blob_refs(&self) -> Vec<BlobId> {
        vec![self.source.blob]
    }

    fn validate(&self, context: &ValidationContext<'_>) -> Result<()> {
        let source = self.source.width.is_finite()
            && self.source.width > 0.0
            && self.source.height.is_finite()
            && self.source.height > 0.0;
        // A band that leaves the source bounds would place OCR and typesetting geometry off
        // the artwork, so the range is an invariant of the component rather than a caller duty.
        let within_source = self.y_offset.is_finite()
            && self.slice_height.is_finite()
            && self.y_offset >= 0.0
            && self.slice_height > 0.0
            && self.y_offset + self.slice_height <= self.source.height;
        if source && within_source && context.contains_blob(self.source.blob) {
            Ok(())
        } else {
            Err(Error::invalid(
                "page slice does not lie inside its stored source image",
            ))
        }
    }
}

/// The uncut image a batch of bands is cut from.
///
/// The bytes travel with the batch because a project that keeps only the bands cannot be
/// re-cut later; the blob identity is derived here so a caller cannot pair one image's
/// geometry with another image's hash.
pub struct SliceSourceInput {
    pub bytes: Arc<[u8]>,
    pub width: f64,
    pub height: f64,
}

impl SliceSourceInput {
    #[must_use]
    pub fn new(bytes: impl Into<Arc<[u8]>>, width: f64, height: f64) -> Self {
        Self {
            bytes: bytes.into(),
            width,
            height,
        }
    }
}

/// One band of a source image, ready to become a page.
#[derive(Clone, Debug)]
pub struct PageSliceDraft {
    pub label: String,
    pub y_offset: f64,
    pub slice_height: f64,
    pub image: AssetInput,
}

impl PageSliceDraft {
    #[must_use]
    pub fn new(
        label: impl Into<String>,
        y_offset: f64,
        slice_height: f64,
        image: AssetInput,
    ) -> Self {
        Self {
            label: label.into(),
            y_offset,
            slice_height,
            image,
        }
    }
}

#[revisioned(revision = 1)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct EntityOrigin {
    pub origin: Origin,
}

impl Component for EntityOrigin {
    const KIND: &'static str = "dev.koharu.entity.origin";

    fn origin(&self) -> Option<&Origin> {
        Some(&self.origin)
    }

    fn set_origin(&mut self, origin: Origin) -> bool {
        self.origin = origin;
        true
    }

    fn validate(&self, _context: &ValidationContext<'_>) -> Result<()> {
        self.origin.validate()
    }
}

#[revisioned(revision = 1)]
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize, Type)]
#[serde(transparent)]
pub struct RelationKind(String);

impl RelationKind {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        validate_namespaced(&value, "relation kind")?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn validate(&self) -> Result<()> {
        validate_namespaced(&self.0, "relation kind")
    }
}

impl FromStr for RelationKind {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::new(value)
    }
}

#[revisioned(revision = 1)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct Relation {
    pub origin: Origin,
    pub kind: RelationKind,
    pub source: EntityId,
    pub target: EntityId,
}

impl Component for Relation {
    const KIND: &'static str = "dev.koharu.relation";

    fn record_refs(&self) -> Vec<EntityId> {
        vec![self.source, self.target]
    }

    fn validate(&self, context: &ValidationContext<'_>) -> Result<()> {
        self.origin.validate()?;
        self.kind.validate()?;
        if context.contains_entity(self.source) && context.contains_entity(self.target) {
            Ok(())
        } else {
            Err(Error::invalid("relation endpoint is missing"))
        }
    }

    fn origin(&self) -> Option<&Origin> {
        Some(&self.origin)
    }

    fn set_origin(&mut self, origin: Origin) -> bool {
        self.origin = origin;
        true
    }
}
