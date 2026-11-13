//! What the desktop tells the React interface on its own, through the
//! channels `subscribe` hands over.

use koharu_canvas::Notice;
use koharu_scene::{LayerId, Rgba};
use parking_lot::Mutex;
use serde::Serialize;
use specta::Type;
use tauri::ipc::Channel;

use koharu_runtime::Transfer;

use crate::{app::ProjectInfo, runs::Job};

/// What the canvas did that the interface shows.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CanvasEvent {
    /// Where the page shows: a page point `p` sits at `p * zoom + (x, y)`
    /// in CSS pixels from the canvas element's corner.
    Camera {
        zoom: f64,
        x: f64,
        y: f64,
    },
    Selection {
        layers: Vec<LayerId>,
    },
    Notice {
        notice: Notice,
    },
    /// The colour the picker picked, for the brush.
    Color {
        color: Rgba,
    },
    /// The brush diameter Alt and the wheel asked for, in page pixels.
    BrushDiameter {
        diameter: f64,
    },
}

/// The interface's channels, from its latest `subscribe`; a reloaded page
/// subscribes again.
#[derive(Default)]
pub struct Channels {
    project: Mutex<Option<Channel<Option<ProjectInfo>>>>,
    canvas: Mutex<Option<Channel<CanvasEvent>>>,
    job: Mutex<Option<Channel<Job>>>,
    download: Mutex<Option<Channel<Transfer>>>,
}

impl Channels {
    pub fn subscribe(
        &self,
        project: Channel<Option<ProjectInfo>>,
        canvas: Channel<CanvasEvent>,
        job: Channel<Job>,
        download: Channel<Transfer>,
    ) {
        *self.project.lock() = Some(project);
        *self.canvas.lock() = Some(canvas);
        *self.job.lock() = Some(job);
        *self.download.lock() = Some(download);
    }

    pub fn project(&self, info: Option<ProjectInfo>) {
        send(&self.project, info);
    }

    pub fn canvas(&self, event: CanvasEvent) {
        send(&self.canvas, event);
    }

    pub fn job(&self, job: Job) {
        send(&self.job, job);
    }

    /// A model or font file's download, as it goes.
    pub fn download(&self, transfer: Transfer) {
        send(&self.download, transfer);
    }
}

/// Sends on a channel, and forgets one whose page went away.
fn send<T: tauri::ipc::IpcResponse + Clone>(channel: &Mutex<Option<Channel<T>>>, message: T) {
    let mut channel = channel.lock();
    if channel
        .as_ref()
        .is_some_and(|current| current.send(message).is_err())
    {
        *channel = None;
    }
}
