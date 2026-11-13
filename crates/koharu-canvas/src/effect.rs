//! What the canvas answers a pointer event with, for the host to carry out
//! or reflect. Pointer input arrives as `ui-events`' W3C pointer events,
//! which the desktop builds from CEF's and the example from winit's.

use koharu_scene::{Edit, LayerId, PageId, Rgba};
use serde::Serialize;
use specta::Type;
use vello::kurbo::TranslateScale;

/// What a gesture did, for the host to act on.
#[derive(Clone, Debug)]
pub enum Effect {
    /// One undo step for the host to apply. A layer it makes comes selected
    /// in the `Selection` that follows it.
    Edit(Edit),
    Selection(Vec<LayerId>),
    Camera(TranslateScale),
    /// The colour picked, opaque.
    Color(Rgba),
    /// An area to run inpainting on: its pixels and their 8-bit coverage.
    Inpaint {
        page: PageId,
        rect: koharu_scene::Rect,
        mask: Vec<u8>,
    },
    /// The brush diameter Alt and the wheel ask for, in page pixels; the
    /// host owns the brush, and sets it with `Canvas::set_brush`.
    BrushDiameter(f64),
    Notice(Notice),
}

/// Why a gesture did nothing, for the app to say in the user's language.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Notice {
    EraserNeedsRaster,
}
