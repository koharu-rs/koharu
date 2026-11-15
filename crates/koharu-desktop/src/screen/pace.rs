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
/// Textures kept for the frames to come, beyond those held.
const SPARES: usize = 2;

pub struct Pacer {
    /// Frames drawn and not yet shown, by when they're due.
    pending: VecDeque<Paced>,
    spares: Vec<wgpu::Texture>,
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
    mask: Option<Mask>,
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
            spares: Vec::new(),
            deliveries: VecDeque::new(),
            epoch: None,
            last: None,
            wake,
        }
    }

    /// A texture no frame holds, to draw the next frame into, if one's spare.
    pub fn spare(&mut self) -> Option<wgpu::Texture> {
        self.spares.pop()
    }

    /// Takes a frame drawn into `texture`, captured at `timestamp`
    /// microseconds, and wakes `view`'s window when it's due.
    pub fn arrive(
        &mut self,
        texture: wgpu::Texture,
        mask: Option<Mask>,
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
            let next = self.pending.pop_front();
            if let Some(skipped) = std::mem::replace(&mut shown, next) {
                self.keep(skipped.texture);
            }
        }
        let Some(paced) = shown else { return };
        if let Some(previous) = painted.view_frame.replace(paced.texture) {
            self.keep(previous);
        }
        painted.mask = paced.mask;
    }

    fn keep(&mut self, texture: wgpu::Texture) {
        if self.spares.len() < SPARES {
            self.spares.push(texture);
        }
    }
}
