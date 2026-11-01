//! The models that turn a scanned page into a translated one, run through a
//! host on one device lane.

mod config;
mod plan;
mod residency;
mod run;
mod steps;

use enumflags2::{BitFlags, bitflags};
use koharu_scene::{Change, Edit, LayerId, PageId, Rect, Session};
use tokio_util::sync::CancellationToken;

pub use config::{
    Detection, DetectionModel, Inpainting, InpaintingModel, OcrModel, PipelineConfig,
    ProcessorConfig, TranslationConfig,
};
pub use run::Pipeline;
pub use steps::{Flux2KleinConfig, KoharuLayoutRFDetrSeg2XLConfig, RoremMixedConfig};

/// The project a run reads and writes, through the app's session or a test's.
pub trait Host: Send + Sync {
    /// Reads the project; the pipeline never holds a read across model work.
    fn read<T>(&self, read: impl FnOnce(&Session) -> T) -> T;
    /// Checks what the edit read and applies it, under one lock.
    fn apply(&self, edit: Edit) -> koharu_scene::Result<Change>;
    fn report(&self, progress: Progress);
}

/// One of the five steps, in the order a page goes through them.
#[bitflags]
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, strum::Display)]
#[strum(serialize_all = "lowercase")]
pub enum Step {
    Detection,
    #[strum(serialize = "OCR")]
    Ocr,
    Translation,
    Typesetting,
    Inpainting,
}

impl Step {
    /// The step whose result this one reads.
    fn needs(self) -> Option<Self> {
        match self {
            Self::Detection => None,
            Self::Ocr | Self::Inpainting => Some(Self::Detection),
            Self::Translation => Some(Self::Ocr),
            Self::Typesetting => Some(Self::Translation),
        }
    }
}

/// Any set of the five steps; a run adds none it wasn't asked for.
pub type Steps = BitFlags<Step>;

pub struct Run {
    pub target: Target,
    pub steps: Steps,
    /// Reruns every step, not only what's missing or out of date.
    pub redo: bool,
    pub cancel: CancellationToken,
}

pub enum Target {
    Pages(Vec<PageId>),
    /// OCR, translation, or typesetting of these text blocks only.
    Layers(Vec<LayerId>),
    /// Detection and inpainting inside a rectangle of one page.
    Area {
        page: PageId,
        rect: Rect,
    },
}

/// A page's step as it starts and ends, with how many of the run's page
/// steps have ended.
#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    pub page: PageId,
    pub step: Step,
    pub state: State,
    pub done: usize,
    pub total: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Started,
    Finished,
    Skipped(String),
    Failed(String),
}

/// How a run ended: every failure and skip, by page and step.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub failures: Vec<Outcome>,
    pub skips: Vec<Outcome>,
    pub done: usize,
    pub total: usize,
    pub cancelled: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub page: PageId,
    pub step: Step,
    pub reason: String,
}
