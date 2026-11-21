//! The select tool: a click, a drag of the selection, the marquee, and what
//! the pointer hovers.

use koharu_scene::{Edit, LayerId, Op};
use kurbo::{Affine, Point, Rect, Vec2};

use super::{
    command,
    transform::{Mode, Transform, transformed},
};
use crate::{
    Effect,
    control::{Control, Handle},
    draw::overlay::Overlay,
    gesture::{Context, Gesture, Press},
    selection::Selection,
    view::Sample,
};

/// A press on a handle resizes or turns its layer, and one inside a selected
/// layer drags the selection, even one under another layer; a click selects
/// what it hits, toggled with Shift, Ctrl, or Command, and drags the
/// selection; one that hits nothing starts a marquee.
pub(crate) fn press(cx: &Context, sample: Sample) -> Press {
    let page = sample.page;
    if let Some((control, handle)) = handle_at(cx, page) {
        let mode = match handle {
            Handle::Rotate => Mode::Rotate,
            handle => Mode::Resize(handle),
        };
        let transform = Transform::new(cx, mode, page, vec![control.clone()]);
        return Press::starts(Gesture::Transform(transform));
    }
    let additive = sample.modifiers.shift() || command(sample.modifiers);
    let inside = cx
        .snapshot
        .controls()
        .iter()
        .any(|control| cx.selection.contains(control.layer) && control.contains(page));
    if inside && !additive {
        return Press::starts(drag(cx, cx.selection.ids(), page));
    }
    let Some(target) = cx.snapshot.hit(page).map(|control| control.layer) else {
        return Press::starts(Gesture::Marquee(Marquee {
            start: page,
            current: page,
            additive,
        }));
    };
    let next = match (additive, cx.selection.contains(target)) {
        (true, _) => cx.selection.toggled(target),
        (false, true) => cx.selection.ids().to_vec(),
        (false, false) => vec![target],
    };
    Press {
        gesture: next.contains(&target).then(|| drag(cx, &next, page)),
        effects: vec![Effect::Selection(next)],
        anchors: None,
    }
}

/// The single selected control's handle at the page point `at`.
pub(crate) fn handle_at<'a>(cx: &Context<'a>, at: Point) -> Option<(&'a Control, Handle)> {
    let control = cx.snapshot.control(cx.selection.single()?)?;
    control
        .handle_at(at, cx.view)
        .map(|handle| (control, handle))
}

/// The layer a click at `page` would select.
pub(crate) fn hover(cx: &Context, page: Point) -> Option<LayerId> {
    cx.snapshot.hit(page).map(|control| control.layer)
}

/// A move of `selection`'s unlocked layers from `page`.
fn drag(cx: &Context, selection: &[LayerId], page: Point) -> Gesture {
    let controls = movable(cx, selection).cloned().collect();
    Gesture::Transform(Transform::new(cx, Mode::Move, page, controls))
}

/// An arrow key's move of the selection `by` page pixels, as Photoshop's
/// Move tool nudges it: the layers a drag takes, in one edit.
pub(crate) fn nudge(cx: &Context, by: Vec2) -> Vec<Effect> {
    let ops: Vec<Op> = movable(cx, cx.selection.ids())
        .map(|control| transformed(control, Affine::translate(by)))
        .collect();
    if ops.is_empty() {
        return Vec::new();
    }
    vec![Effect::Edit(Edit {
        label: "move".to_owned(),
        ops,
    })]
}

/// The selected layers a move takes: the unlocked ones the canvas shows.
fn movable<'a>(cx: &Context<'a>, selection: &'a [LayerId]) -> impl Iterator<Item = &'a Control> {
    cx.snapshot
        .controls()
        .iter()
        .filter(move |control| selection.contains(&control.layer) && !control.locked)
}

/// A selection rectangle, from where it started to where it is, in page
/// points.
pub(crate) struct Marquee {
    start: Point,
    current: Point,
    additive: bool,
}

impl Marquee {
    pub fn rect(&self) -> Rect {
        Rect::from_points(self.start, self.current)
    }

    pub fn moved(&mut self, samples: &[Sample]) {
        if let Some(last) = samples.last() {
            self.current = last.page;
        }
    }

    /// What lies wholly inside it, added to the selection when `additive`.
    pub fn finish(self, cx: &Context) -> Vec<Effect> {
        let rect = self.rect();
        let inside: Vec<LayerId> = cx
            .snapshot
            .controls()
            .iter()
            .filter(|control| control.picks && !control.locked && control.inside(rect))
            .map(|control| control.layer)
            .collect();
        let next = match self.additive {
            true => cx.selection.with(inside),
            false => Selection::default().with(inside),
        };
        vec![Effect::Selection(next)]
    }

    pub fn preview(&self, overlay: &mut Overlay) {
        overlay.marquee(self.rect());
    }
}
