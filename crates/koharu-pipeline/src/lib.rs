//! Koharu's models run over a project's pages: every stage on every page in
//! one click, or one stage by hand on a page, a block, or a painted area. The
//! README is the design.

mod config;
mod error;
mod event;
mod gpu;
mod pipeline;
mod request;
mod scheduler;
mod stages;
pub mod vram;

pub use config::PipelineConfig;
pub use error::{Error, Result};
pub use event::{
    Event, Outcome, QueueOrderReason, QueueScope, QueuedRunSummary, QueuedStep, RunId,
};
pub use pipeline::Pipeline;
pub use request::{Area, Request, RunPriority, Stage, Target};
pub use stages::{
    detect::{Detector, DetectorConfig},
    inpaint::{Inpainter, InpainterConfig},
    recognize::{Recognizer, RecognizerConfig},
    translate::TranslatorConfig,
};
