//! What a run tells the app as it goes.

use koharu_scene::PageId;
use serde::{Deserialize, Serialize};

use crate::Stage;

/// A run, numbered in the order runs were asked for.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RunId(pub u64);

/// A stage started or finished on a page, or a run ended. A stage starts
/// when the GPU takes it, or for a hosted translation when it's sent.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    Started {
        run: RunId,
        page: PageId,
        stage: Stage,
    },
    Finished {
        run: RunId,
        page: PageId,
        stage: Stage,
    },
    Ended {
        run: RunId,
        outcome: Outcome,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    Done,
    /// The first failure ends the run; what it finished stays.
    Failed {
        page: PageId,
        stage: Stage,
        reason: String,
    },
    Stopped,
}
