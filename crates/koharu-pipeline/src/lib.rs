//! Koharu's models run over a project's pages: every step on every page in one
//! click, or one step by hand on a page, a block, or a painted area. The
//! README is the design.

mod config;
mod error;
mod event;
mod models;
mod pipeline;
mod placement;
mod request;
mod scheduler;
mod steps;

pub use config::{
    DEFAULT_PROMPT, Detector, DetectorConfig, Inpainter, InpainterConfig, PipelineConfig,
    Recognizer, RecognizerConfig, TARGET, TranslatorConfig,
};
pub use error::{Error, Result};
pub use event::{Event, Outcome, RunId};
pub use pipeline::Pipeline;
pub use placement::placement;
pub use request::{Area, Request, Step, Target};
