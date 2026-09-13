use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::{ProgressSink, Scope, Stage};
use koharu_scene::EntityId;

#[derive(Clone, Debug)]
pub struct InpaintingMask {
    pub page: EntityId,
    pub png: Arc<[u8]>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, Type)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum Operation {
    #[default]
    Full,
    Through {
        stage: Stage,
    },
    Only {
        stage: Stage,
    },
    Stages {
        stages: Vec<Stage>,
    },
    StageMajor {
        stages: Vec<Stage>,
    },
}

impl Operation {
    pub(crate) fn stages(&self) -> Result<Vec<Stage>> {
        let stages = match self {
            Self::Full => Stage::ALL.to_vec(),
            Self::Through {
                stage: Stage::Detection,
            } => vec![Stage::Detection],
            Self::Through { stage: Stage::Ocr } => vec![Stage::Detection, Stage::Ocr],
            Self::Through {
                stage: Stage::Translation,
            } => {
                vec![Stage::Detection, Stage::Ocr, Stage::Translation]
            }
            Self::Through {
                stage: Stage::Inpainting,
            } => vec![Stage::Detection, Stage::Inpainting],
            Self::Only { stage } => vec![*stage],
            Self::Stages { stages } | Self::StageMajor { stages } => Stage::ALL
                .into_iter()
                .filter(|stage| stages.contains(stage))
                .collect(),
        };
        if stages.is_empty() {
            bail!("at least one pipeline stage must be selected");
        }
        Ok(stages)
    }
}

#[derive(Clone)]
pub struct Request {
    pub operation: Operation,
    pub scope: Scope,
    pub stop: StopToken,
    pub progress: Option<ProgressSink>,
    pub inpainting_mask: Option<InpaintingMask>,
}

impl Default for Request {
    fn default() -> Self {
        Self {
            operation: Operation::Full,
            scope: Scope::Project,
            stop: StopToken::default(),
            progress: None,
            inpainting_mask: None,
        }
    }
}

#[derive(Clone, Default)]
pub struct StopToken(Arc<AtomicBool>);

impl StopToken {
    pub fn stop(&self) {
        self.0.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn stopped(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}
