use std::{
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::Result;

use super::{State, TRANSFERS, Transfer};

const PROGRESS_INTERVAL: Duration = Duration::from_millis(50);

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// A transfer in progress, and the last progress it reported.
pub(crate) struct Activity {
    id: u64,
    name: String,
    completed: AtomicU64,
    total: AtomicU64,
    reported: Mutex<Instant>,
}

impl Activity {
    /// Starts reporting a transfer of the file `name`.
    pub(crate) fn start(name: impl Into<String>) -> Self {
        let activity = Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            name: name.into(),
            completed: AtomicU64::new(0),
            total: AtomicU64::new(0),
            reported: Mutex::new(Instant::now()),
        };
        activity.send(State::Running);
        activity
    }

    /// Records progress, and reports it at most once per
    /// `PROGRESS_INTERVAL`.
    pub(crate) fn progress(&self, completed: u64, total: u64) {
        self.completed.store(completed, Ordering::Relaxed);
        self.total.store(total, Ordering::Relaxed);
        let mut reported = self
            .reported
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if reported.elapsed() >= PROGRESS_INTERVAL {
            *reported = Instant::now();
            drop(reported);
            self.send(State::Running);
        }
    }

    /// Reports how the transfer ended, and passes `result` on.
    pub(crate) fn finish<T>(&self, result: Result<T>) -> Result<T> {
        match &result {
            Ok(_) => self.send(State::Finished),
            Err(error) => self.send(State::Failed(format!("{error:#}"))),
        }
        result
    }

    fn send(&self, state: State) {
        let _ = TRANSFERS.send(Transfer {
            id: self.id,
            name: self.name.clone(),
            completed: self.completed.load(Ordering::Relaxed),
            total: self.total.load(Ordering::Relaxed),
            state,
        });
    }
}
