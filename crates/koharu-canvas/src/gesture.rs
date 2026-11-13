//! A gesture in progress. Each kind is its own type, in its tool's module,
//! and this enum forwards every step to it, so the compiler checks every
//! step for every gesture. A gesture reads the canvas through a `Context`
//! and changes nothing: it answers effects, which the canvas applies.

use cursor_icon::CursorIcon;
use koharu_renderer::{Frame, Gpu};
use koharu_scene::{LayerId, Session};
use parking_lot::Mutex;
use vello::kurbo::{Point, Rect};

use crate::{
    Brush, Colors, Effect, Tool,
    draw::overlay::Overlay,
    selection::Selection,
    snapshot::Snapshot,
    tools::{brush::Stroke, pan::Pan, select::Marquee, text::Draft, transform::Transform},
    view::{Sample, View},
};

/// What a tool reads while it handles a press or a gesture.
pub(crate) struct Context<'a> {
    pub view: &'a View,
    pub snapshot: &'a Snapshot,
    pub selection: &'a Selection,
    pub tool: Tool,
    pub brush: Brush,
    pub colors: Colors,
    /// Locked only to read pixels.
    pub session: &'a Mutex<Session>,
    pub gpu: &'a Gpu,
}

/// What a press starts, and what it answers at once.
#[derive(Default)]
pub(crate) struct Press {
    pub gesture: Option<Gesture>,
    pub effects: Vec<Effect>,
}

impl Press {
    pub fn starts(gesture: Gesture) -> Self {
        Self {
            gesture: Some(gesture),
            effects: Vec::new(),
        }
    }

    pub fn answers(effects: Vec<Effect>) -> Self {
        Self {
            gesture: None,
            effects,
        }
    }
}

/// What a gesture's preview depends on: it changes whenever the preview
/// would.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum GestureLook {
    /// Nothing the camera doesn't already show, as for a pan.
    None,
    Rect(Rect),
    Pointer(Point),
    Stroke {
        points: usize,
        last: Point,
    },
}

pub(crate) enum Gesture {
    Marquee(Marquee),
    Transform(Transform),
    Draft(Draft),
    Pan(Pan),
    Stroke(Stroke),
}

impl Gesture {
    /// Follows the pointer through `samples`, oldest first.
    pub fn moved(&mut self, cx: &Context, samples: &[Sample]) -> Vec<Effect> {
        match self {
            Self::Marquee(marquee) => marquee.moved(samples),
            Self::Transform(transform) => transform.moved(samples),
            Self::Draft(draft) => draft.moved(samples),
            Self::Pan(pan) => return pan.moved(samples),
            Self::Stroke(stroke) => return stroke.moved(cx, samples),
        }
        Vec::new()
    }

    /// What the gesture leaves when its pointer lifts.
    pub fn finish(self, cx: &Context) -> anyhow::Result<Vec<Effect>> {
        match self {
            Self::Marquee(marquee) => Ok(marquee.finish(cx)),
            Self::Transform(transform) => Ok(transform.finish(cx)),
            Self::Draft(draft) => Ok(draft.finish(cx)),
            Self::Pan(_) => Ok(Vec::new()),
            Self::Stroke(stroke) => stroke.finish(cx),
        }
    }

    pub fn preview(&self, cx: &Context, overlay: &mut Overlay) {
        match self {
            Self::Marquee(marquee) => marquee.preview(overlay),
            Self::Transform(transform) => transform.preview(cx, overlay),
            Self::Draft(draft) => draft.preview(overlay),
            Self::Pan(_) => {}
            Self::Stroke(stroke) => stroke.preview(cx, overlay),
        }
    }

    /// The pointer's shape while it runs; `None` hides it.
    pub fn cursor(&self) -> Option<CursorIcon> {
        match self {
            Self::Marquee(_) => Some(CursorIcon::Default),
            Self::Transform(transform) => Some(transform.cursor()),
            Self::Draft(_) => Some(CursorIcon::Text),
            Self::Pan(_) => Some(CursorIcon::Grabbing),
            Self::Stroke(_) => None,
        }
    }

    /// The layers its preview draws instead of the page.
    pub fn lifted(&self) -> &[LayerId] {
        match self {
            Self::Transform(transform) => transform.lifted(),
            _ => &[],
        }
    }

    pub fn look(&self) -> GestureLook {
        match self {
            Self::Marquee(marquee) => GestureLook::Rect(marquee.rect()),
            Self::Transform(transform) => GestureLook::Pointer(transform.current()),
            Self::Draft(draft) => GestureLook::Rect(draft.rect()),
            Self::Pan(_) => GestureLook::None,
            Self::Stroke(stroke) => stroke.look(),
        }
    }

    /// The frame the page draws while layers are lifted out of it.
    pub fn under(&self) -> Option<&Frame> {
        match self {
            Self::Transform(transform) => transform.under(),
            _ => None,
        }
    }

    /// The controls it drags, where the drag puts them.
    pub fn dragged(&self) -> Vec<crate::control::Control> {
        match self {
            Self::Transform(transform) => transform.dragged(),
            _ => Vec::new(),
        }
    }
}
