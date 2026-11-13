//! What a run is asked to do.

use std::collections::BTreeSet;

use koharu_scene::{LayerId, PageId, Rect};
use serde::{Deserialize, Serialize};
use strum::Display;

use crate::PipelineConfig;

/// One run: `steps` on a target, each page's in step order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub target: Target,
    pub steps: BTreeSet<Step>,
    /// For this run only; the pipeline's own otherwise.
    #[serde(default)]
    pub config: Option<PipelineConfig>,
}

/// What a run works on. It runs every step it's given on everything it
/// names, replacing what's there, and never a step it isn't given.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    /// Every page.
    Project,
    Pages(Vec<PageId>),
    /// Recognition, inpainting, or translation on blocks.
    Blocks(Vec<LayerId>),
    /// Inpainting only: a new raster layer limited by a layer mask to the
    /// area, as Photoshop's Generative Fill and Content-Aware Fill write one.
    /// A prompt needs an inpainter that takes one.
    Area {
        page: PageId,
        area: Area,
        prompt: Option<String>,
    },
}

/// A step, in the order a page's steps run.
#[derive(
    Clone, Copy, Debug, Display, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum Step {
    Detect,
    Recognize,
    Inpaint,
    Translate,
}

/// A painted area of a page: a byte a pixel over `rect`, in page pixels,
/// painted where it isn't 0.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Area {
    pub rect: Rect,
    pub pixels: Vec<u8>,
}
