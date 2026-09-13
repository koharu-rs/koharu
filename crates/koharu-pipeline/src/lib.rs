//! In-process, scene-native model orchestration for Koharu.

mod accelerator;
mod config;
mod error;
mod execution;
mod images;
mod model_cell;
mod pipeline;
mod progress;
mod report;
mod request;
mod resources;
mod scheduler;
mod scope;
mod stage;
mod stage_runner;
mod stages;
mod terminology;
pub use terminology::analyze_terminology;

pub use config::{
    DetectionModel, InpaintingModel, OcrModel, PipelineConfig, ProcessorConfig, TranslationConfig,
};
pub use error::{ErrorKind, PipelineError};
pub use pipeline::Pipeline;
pub use progress::{Progress, ProgressSink};
pub use report::{Committer, Report, RunStatus, StageOutput};
pub use request::{InpaintingMask, Operation, Request, StopToken};
pub use resources::{DeviceResources, ResourceSnapshot};
pub use scope::{Bounds, Scope};
pub use stage::Stage;
pub use stages::{Flux2KleinConfig, KoharuLayoutRFDetrSeg2XLConfig, RoremMixedConfig};

use images::ImageCache;
use model_cell::ModelCell;

#[cfg(test)]
mod tests;
