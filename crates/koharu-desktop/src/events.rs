//! What the desktop tells the React interface on its own, as Tauri events,
//! each carrying its owner's type.

use std::marker::PhantomData;

use koharu_canvas::{Camera, Notice, Typed};
use koharu_pipeline::Event as PipelineEvent;
use koharu_runtime::DownloadEntry;
use koharu_scene::{Bounds, Change, LayerId, PageId, Rgba};
use serde::Serialize;
use specta::{Type, Types, datatype::DataType};
use tauri::{AppHandle, Emitter as _};
use tauri_runtime_cef::CefRuntime;

use crate::gallery::Loading;

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

impl<T: Serialize + Clone> Event<T> {
    /// Tells the interface `payload` through `handle`.
    pub fn tell(&self, handle: &AppHandle<CefRuntime>, payload: T) {
        if let Err(error) = handle.emit(self.name, payload) {
            tracing::warn!("failed to tell the interface {}: {error}", self.name);
        }
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
/// Where the selection lies on the page as it's laid out, once that
/// changes; none while nothing selected shows.
pub const SELECTION_BOUNDS: Event<Option<Bounds>> = Event::new("selection_bounds");
/// Where the area the marquee and the lasso marked lies on the page, once
/// that changes; none while there's none.
pub const AREA: Event<Option<Bounds>> = Event::new("area");
pub const CAMERA: Event<Camera> = Event::new("camera");
/// The text being typed into on the canvas and its selection's character
/// style, once either changes; none when typing stops.
pub const TYPING: Event<Option<Typed>> = Event::new("typing");
/// The colour the picker picked, for the brush.
pub const COLOR: Event<Rgba> = Event::new("color");
/// The brush diameter Alt and the wheel asked for, in page pixels.
pub const BRUSH_DIAMETER: Event<f64> = Event::new("brush_diameter");
pub const NOTICE: Event<Notice> = Event::new("notice");
pub const PIPELINE: Event<PipelineEvent> = Event::new("pipeline");
/// A model or runtime package download, as it goes.
pub const DOWNLOAD: Event<DownloadEntry> = Event::new("download");
/// An extension's chapter loading into the open project, as it goes.
pub const CHAPTER: Event<Loading> = Event::new("chapter");

/// A file or folder dragged over or dropped onto the main window, in CSS pixels.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum FileDrag {
    Enter { x: f64, y: f64 },
    Over { x: f64, y: f64 },
    Drop { x: f64, y: f64, paths: Vec<String> },
    Leave,
}

pub const FILE_DRAG: Event<FileDrag> = Event::new("file_drag");

/// Every event's name and payload, for the bindings.
pub fn bindings(types: &mut Types) -> Vec<(&'static str, DataType)> {
    vec![
        CHANGE.binding(types),
        PROJECT.binding(types),
        PAGE.binding(types),
        SELECTION.binding(types),
        SELECTION_BOUNDS.binding(types),
        AREA.binding(types),
        CAMERA.binding(types),
        TYPING.binding(types),
        COLOR.binding(types),
        BRUSH_DIAMETER.binding(types),
        NOTICE.binding(types),
        PIPELINE.binding(types),
        DOWNLOAD.binding(types),
        CHAPTER.binding(types),
        FILE_DRAG.binding(types),
    ]
}
