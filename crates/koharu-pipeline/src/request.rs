//! What a run is asked to do.

use std::collections::BTreeSet;

use koharu_scene::{LayerId, PageId, Rect};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{Display, VariantArray};

use crate::PipelineConfig;

/// Queue scheduling priority for a pipeline request.
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, JsonSchema, specta::Type,
)]
#[serde(rename_all = "snake_case")]
pub enum RunPriority {
    #[default]
    Normal,
    /// Jumps ahead of queued batch runs and starts at the next page/stage boundary.
    RerunNow,
}

/// One run: `stages` on a target, each page's in stage order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub target: Target,
    pub stages: BTreeSet<Stage>,
    /// For this run only; the pipeline's own otherwise.
    #[serde(default)]
    pub config: Option<PipelineConfig>,
    /// Queue priority (`normal` queues behind active batch work, `rerun_now` preempts).
    #[serde(default)]
    pub priority: RunPriority,
}

/// What a run works on. It runs every stage it's given on everything it
/// names, replacing what's there, and never a stage it isn't given.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[derive(specta::Type)]
pub enum Target {
    /// Every page.
    Project,
    Pages(Vec<PageId>),
    /// Recognition, inpainting, or translation on blocks.
    Blocks(Vec<LayerId>),
    /// Inpainting only, onto `layer`, a raster or a block's clean-up, whose
    /// mask opens there, or a new raster layer limited by a layer mask to the
    /// area, as Photoshop's Generative Fill and Content-Aware Fill write one.
    /// Its prompt goes ahead of the inpainter section's; LaMa, which takes
    /// none, ignores it.
    Area {
        page: PageId,
        area: Area,
        prompt: Option<String>,
        layer: Option<LayerId>,
    },
    /// Project ComicInfo prose metadata translation only.
    Metadata,
}

/// A stage, in the order a page's stages run.
#[derive(
    Clone,
    Copy,
    Debug,
    Display,
    Eq,
    Hash,
    Ord,
    PartialEq,
    PartialOrd,
    Serialize,
    Deserialize,
    JsonSchema,
    VariantArray,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[derive(specta::Type)]
pub enum Stage {
    Detect,
    Recognize,
    Inpaint,
    Translate,
}

impl Stage {
    #[must_use]
    pub(crate) const fn index(self) -> usize {
        match self {
            Self::Detect => 0,
            Self::Recognize => 1,
            Self::Inpaint => 2,
            Self::Translate => 3,
        }
    }
}

/// A painted area of a page: a byte a pixel over `rect`, in page pixels,
/// painted where it isn't 0.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, specta::Type)]
pub struct Area {
    pub rect: Rect,
    pub pixels: Vec<u8>,
}
