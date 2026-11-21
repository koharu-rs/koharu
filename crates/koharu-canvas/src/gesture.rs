//! A gesture in progress. Each kind is its own type, in its tool's module,
//! and this enum forwards every step to it, so the compiler checks every
//! step for every gesture. A gesture reads the canvas through a `Context`
//! and changes nothing: it answers effects, which the canvas applies.

use cursor_icon::CursorIcon;
use koharu_renderer::Frame;
use koharu_scene::{LayerId, Session};
use kurbo::{Point, Rect};
use parking_lot::Mutex;

use crate::{
    Brush, Colors, Effect, Tool,
    draw::overlay::Overlay,
    selection::Selection,
    snapshot::Snapshot,
    tools::{
        brush::Stroke,
        direct::{Pick, Reshape},
        mark::Mark,
        pan::Pan,
        select::Marquee,
        text::Draft,
        transform::Transform,
    },
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
}

/// What a press starts, what it answers at once, and the anchors it picks
/// on the selected text's frame, which the canvas keeps.
#[derive(Default)]
pub(crate) struct Press {
    pub gesture: Option<Gesture>,
    pub effects: Vec<Effect>,
    pub anchors: Option<Vec<usize>>,
}

impl Press {
    pub fn starts(gesture: Gesture) -> Self {
        Self {
            gesture: Some(gesture),
            ..Self::default()
        }
    }

    pub fn answers(effects: Vec<Effect>) -> Self {
        Self {
            effects,
            ..Self::default()
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
    Reshape(Reshape),
    Pick(Pick),
    Mark(Mark),
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
            Self::Reshape(reshape) => reshape.moved(samples),
            Self::Pick(pick) => pick.moved(samples),
            Self::Mark(mark) => mark.moved(samples),
        }
        Vec::new()
    }

    /// What the gesture leaves when its pointer lifts.
    pub fn finish(self, cx: &Context) -> anyhow::Result<Vec<Effect>> {
        match self {
            Self::Marquee(marquee) => Ok(marquee.finish(cx)),
            Self::Transform(transform) => Ok(transform.finish(cx)),
            // The canvas types into a draft itself.
            Self::Draft(_) => Ok(Vec::new()),
            Self::Pan(_) => Ok(Vec::new()),
            Self::Stroke(stroke) => stroke.finish(cx),
            Self::Reshape(reshape) => Ok(reshape.finish()),
            Self::Pick(pick) => Ok(pick.finish(cx.view)),
            // The canvas joins a mark to its area itself.
            Self::Mark(_) => Ok(Vec::new()),
        }
    }

    pub fn preview(&self, cx: &Context, overlay: &mut Overlay) -> anyhow::Result<()> {
        match self {
            Self::Marquee(marquee) => marquee.preview(overlay),
            Self::Transform(transform) => return transform.preview(overlay),
            Self::Draft(draft) => draft.preview(overlay),
            Self::Pan(_) => {}
            Self::Stroke(stroke) => stroke.preview(cx, overlay),
            Self::Reshape(reshape) => reshape.preview(overlay),
            Self::Pick(pick) => pick.preview(overlay),
            Self::Mark(mark) => mark.preview(overlay),
        }
        Ok(())
    }

    /// The pointer's shape while it runs; `None` hides it.
    pub fn cursor(&self) -> Option<CursorIcon> {
        match self {
            Self::Marquee(_) => Some(CursorIcon::Default),
            Self::Transform(transform) => Some(transform.cursor()),
            Self::Draft(_) => Some(CursorIcon::Text),
            Self::Pan(_) => Some(CursorIcon::Grabbing),
            Self::Stroke(_) => None,
            Self::Reshape(reshape) => Some(reshape.cursor()),
            Self::Pick(_) => Some(CursorIcon::Default),
            Self::Mark(_) => Some(CursorIcon::Crosshair),
        }
    }

    /// The anchors it picks on the selected text's frame, as its pointer
    /// lifts.
    pub fn picked(&self) -> Option<Vec<usize>> {
        match self {
            Self::Pick(pick) => Some(pick.picked()),
            _ => None,
        }
    }

    /// Whether it drags a frame's anchors, whose preview draws the frame.
    pub fn reshapes(&self) -> bool {
        matches!(self, Self::Reshape(_))
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
            Self::Reshape(reshape) => GestureLook::Pointer(reshape.current()),
            Self::Pick(pick) => GestureLook::Rect(pick.rect()),
            Self::Mark(mark) => mark.look(),
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
