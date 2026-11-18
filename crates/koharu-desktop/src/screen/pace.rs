//! Paces the frames CEF shares by their capture time. Chromium captures them
//! evenly, a frame period apart, but they reach the UI thread up to a frame
//! late and often two together, which shown on arrival is a stutter. So each
//! is shown one frame period after it would have arrived on the fastest
//! delivery seen lately, and a frame after a pause at once, so a click or a
//! key shows without the wait. Video players pace frames by their timestamps
//! the same way.

use std::{
    collections::VecDeque,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use tauri_runtime_cef::OffscreenView;

use super::{
    FRAME_RATE,
    paint::{Mask, Painted},
};

/// A frame period, in microseconds, as capture timestamps count.
const PERIOD: i64 = 1_000_000 / FRAME_RATE as i64;
/// Deliveries remembered for the fastest, a second's worth.
const DELIVERIES: usize = FRAME_RATE as usize;
/// The quiet after which a frame is shown at once.
const PAUSE: Duration = Duration::from_micros(3 * PERIOD as u64);

pub struct Pacer {
    /// Frames taken and not yet shown, by when they're due.
    pending: VecDeque<Paced>,
    /// How long after its capture each recent frame arrived, in
    /// microseconds, on a clock that starts at the first arrival.
    deliveries: VecDeque<i64>,
    epoch: Option<Instant>,
    last: Option<Instant>,
    /// Asks the window to redraw when a frame is due.
    wake: mpsc::Sender<(Instant, OffscreenView)>,
}

struct Paced {
    texture: wgpu::Texture,
    mask: Mask,
    due: Instant,
}

impl Pacer {
    pub fn new() -> Self {
        let (wake, wakes) = mpsc::channel::<(Instant, OffscreenView)>();
        // Ends when the pacer goes, with the window.
        let spawned = thread::Builder::new()
            .name("koharu frame pacer".to_owned())
            .spawn(move || {
                for (due, view) in wakes {
                    if let Some(wait) = due.checked_duration_since(Instant::now()) {
                        thread::sleep(wait);
                    }
                    view.request_redraw();
                }
            });
        if let Err(error) = spawned {
            tracing::error!("failed to start the frame pacer: {error}");
        }
        Self {
            pending: VecDeque::new(),
            deliveries: VecDeque::new(),
            epoch: None,
            last: None,
            wake,
        }
    }

    /// Takes the frame `texture` with its `mask`, captured at `timestamp`
    /// microseconds, and wakes `view`'s window when it's due.
    pub fn arrive(
        &mut self,
        texture: wgpu::Texture,
        mask: Mask,
        timestamp: u64,
        view: &OffscreenView,
    ) {
        let now = Instant::now();
        let epoch = *self.epoch.get_or_insert(now);
        let arrived = i64::try_from(now.duration_since(epoch).as_micros()).unwrap_or(i64::MAX);
        let timestamp = i64::try_from(timestamp).unwrap_or(i64::MAX);
        self.deliveries.push_back(arrived.saturating_sub(timestamp));
        if self.deliveries.len() > DELIVERIES {
            self.deliveries.pop_front();
        }
        let paused = self.pending.is_empty()
            && self
                .last
                .is_none_or(|last| now.duration_since(last) > PAUSE);
        self.last = Some(now);
        let due = if paused {
            now
        } else {
            let fastest = self.deliveries.iter().copied().min().unwrap_or(0);
            let due = timestamp.saturating_add(fastest).saturating_add(PERIOD);
            epoch + Duration::from_micros(u64::try_from(due).unwrap_or(0))
        };
        self.pending.push_back(Paced { texture, mask, due });
        let _ = self.wake.send((due, view.clone()));
    }

    /// Shows the latest frame due by `now`, if any, in `painted`.
    pub fn show(&mut self, painted: &mut Painted, now: Instant) {
        let mut shown = None;
        while self.pending.front().is_some_and(|paced| paced.due <= now) {
            shown = self.pending.pop_front();
        }
        let Some(paced) = shown else { return };
        painted.view_frame = Some(paced.texture);
        painted.mask = Some(paced.mask);
    }
}
