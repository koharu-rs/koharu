//! What the desktop tells the React interface on its own, as Tauri events,
//! each carrying its owner's type.

use std::marker::PhantomData;

use koharu_canvas::{Camera, Notice};
use koharu_pipeline::Event as PipelineEvent;
use koharu_runtime::Transfer;
use koharu_scene::{Change, LayerId, PageId, Rgba};
use specta::{Type, Types, datatype::DataType};

/// An event's name, typed by what it carries, so a send and the bindings
/// agree.
pub struct Event<T> {
    pub name: &'static str,
    payload: PhantomData<fn(T)>,
}

impl<T: Type> Event<T> {
    const fn new(name: &'static str) -> Self {
        Self {
            name,
            payload: PhantomData,
        }
    }

    fn binding(&self, types: &mut Types) -> (&'static str, DataType) {
        (self.name, T::definition(types))
    }
}

/// An edit, undo, or redo of the open project, from whichever writer.
pub const CHANGE: Event<Change> = Event::new("change");
/// The open project's name, or none once it closes.
pub const PROJECT: Event<Option<String>> = Event::new("project");
/// The page the canvas shows.
pub const PAGE: Event<Option<PageId>> = Event::new("page");
/// The selection a canvas gesture made.
pub const SELECTION: Event<Vec<LayerId>> = Event::new("selection");
pub const CAMERA: Event<Camera> = Event::new("camera");
/// The colour the picker picked, for the brush.
pub const COLOR: Event<Rgba> = Event::new("color");
/// The brush diameter Alt and the wheel asked for, in page pixels.
pub const BRUSH_DIAMETER: Event<f64> = Event::new("brush_diameter");
pub const NOTICE: Event<Notice> = Event::new("notice");
pub const PIPELINE: Event<PipelineEvent> = Event::new("pipeline");
/// A model or font file's download, as it goes.
pub const DOWNLOAD: Event<Transfer> = Event::new("download");

/// Every event's name and payload, for the bindings.
pub fn bindings(types: &mut Types) -> Vec<(&'static str, DataType)> {
    vec![
        CHANGE.binding(types),
        PROJECT.binding(types),
        PAGE.binding(types),
        SELECTION.binding(types),
        CAMERA.binding(types),
        COLOR.binding(types),
        BRUSH_DIAMETER.binding(types),
        NOTICE.binding(types),
        PIPELINE.binding(types),
        DOWNLOAD.binding(types),
    ]
}
