//! The direct selection tool: a text's frame reshaped by its anchors, as
//! Photoshop's Direct Selection tool reshapes a text's path, the text fitting
//! the new shape once the drag ends. A press on an anchor or a segment drags
//! what it picks, Shift adding to the picked anchors, and a marquee picks the
//! anchors it encloses. A frame stays closed: removing anchors joins their
//! neighbours, as Photoshop's Delete Anchor Point tool does, where its Delete
//! key would open the path.

use cursor_icon::CursorIcon;
use koharu_scene::{Edit, Fit, Frame as TextFrame, LayerId, Op, Path, PathFit, Ref};
use kurbo::{
    Affine, BezPath, CubicBez, Line, ParamCurveNearest as _, PathEl, PathSeg, Point, QuadBez, Rect,
    Shape as _, Vec2,
};
use serde_json::Value;

use super::transform::Lettering;
use crate::{
    Effect,
    control::REACH,
    draw::overlay::Overlay,
    gesture::{Context, Gesture, Press},
    view::{Sample, View},
};

/// The segment reaching an anchor from the one before it; a ring's first
/// anchor's is the one that closes the ring.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Segment {
    Line,
    Quad(Point),
    Cubic(Point, Point),
}

/// One closed subpath: its anchors, each with the segment reaching it.
type Ring = Vec<(Point, Segment)>;

/// A text's frame as the tool edits it: its rings in the text's own
/// coordinates, the transform that puts them on the page, and the frame
/// they came from, whose fit and anchor an edit keeps.
#[derive(Clone, Debug)]
pub(crate) struct Shape {
    layer: LayerId,
    lettering: Lettering,
    frame: TextFrame,
    transform: Affine,
    rings: Vec<Ring>,
}

impl Shape {
    /// `layer`'s frame, when the tool can edit it: a path, or a box that
    /// becomes one, but not a growing box, which has no path form, nor point
    /// text, nor a locked layer.
    pub fn of(cx: &Context, layer: LayerId) -> Option<Self> {
        if cx.snapshot.control(layer)?.locked {
            return None;
        }
        let (lettering, text) = Lettering::of(&cx.snapshot.layer(layer)?.content)?;
        let path = match &text.frame {
            TextFrame::Path { path, .. } => BezPath::from_svg(path.as_str()).ok()?,
            TextFrame::Box { bounds, fit, .. } if *fit != Fit::Grow => {
                Rect::from(*bounds).to_path(0.1)
            }
            _ => return None,
        };
        let rings = rings(&path);
        (!rings.is_empty()).then(|| Self {
            layer,
            lettering,
            frame: text.frame.clone(),
            transform: Affine::from(text.transform),
            rings,
        })
    }

    /// Its anchors on the page, ring by ring.
    pub fn anchors(&self) -> Vec<Point> {
        self.rings
            .iter()
            .flatten()
            .map(|(point, _)| self.transform * *point)
            .collect()
    }

    /// Its outline on the page.
    pub fn outline(&self) -> BezPath {
        self.transform * path(&self.rings)
    }

    /// The anchor nearest `at` on the page, within a handle's reach.
    fn anchor_at(&self, at: Point, view: &View) -> Option<usize> {
        let reach = view.page_length(REACH);
        self.anchors()
            .into_iter()
            .enumerate()
            .map(|(index, anchor)| (index, anchor.distance(at)))
            .filter(|(_, distance)| *distance <= reach)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(index, _)| index)
    }

    /// The anchors at both ends of the segment nearest `at` on the page,
    /// within a handle's reach.
    fn segment_at(&self, at: Point, view: &View) -> Option<[usize; 2]> {
        let reach = view.page_length(REACH);
        let mut first = 0;
        let mut nearest: Option<([usize; 2], f64)> = None;
        for ring in &self.rings {
            let count = ring.len();
            for (index, &(to, segment)) in ring.iter().enumerate() {
                let before = (index + count - 1) % count;
                let from = ring[before].0;
                let curve = match segment {
                    Segment::Line => PathSeg::Line(Line::new(from, to)),
                    Segment::Quad(control) => PathSeg::Quad(QuadBez::new(from, control, to)),
                    Segment::Cubic(a, b) => PathSeg::Cubic(CubicBez::new(from, a, b, to)),
                };
                let distance = (self.transform * curve)
                    .nearest(at, 1e-6)
                    .distance_sq
                    .sqrt();
                if distance <= reach && nearest.is_none_or(|(_, best)| distance < best) {
                    nearest = Some(([first + before, first + index], distance));
                }
            }
            first += count;
        }
        nearest.map(|(ends, _)| ends)
    }

    /// The rings with `anchors` moved by the page vector `by`, a cubic's
    /// handles going with the anchor they leave or reach.
    fn moved(&self, anchors: &[usize], by: Vec2) -> Vec<Ring> {
        // The page vector in the text's own coordinates, its tilt undone.
        let inverse = self.transform.inverse();
        let by = inverse * by.to_point() - inverse * Point::ZERO;
        let mut rings = self.rings.clone();
        let mut first = 0;
        for ring in &mut rings {
            let count = ring.len();
            let picked = |index: usize| anchors.contains(&(first + index));
            for (index, (point, segment)) in ring.iter_mut().enumerate() {
                let before = picked((index + count - 1) % count);
                if picked(index) {
                    *point += by;
                }
                if let Segment::Cubic(a, b) = segment {
                    if before {
                        *a += by;
                    }
                    if picked(index) {
                        *b += by;
                    }
                }
            }
            first += count;
        }
        rings
    }

    /// The rings without `anchors`, each removed one's neighbours joined;
    /// `None` when a ring would keep fewer than three.
    fn removed(&self, anchors: &[usize]) -> Option<Vec<Ring>> {
        let mut first = 0;
        let mut rings = Vec::with_capacity(self.rings.len());
        for ring in &self.rings {
            let count = ring.len();
            let picked: Vec<usize> = (0..count)
                .filter(|index| anchors.contains(&(first + index)))
                .collect();
            first += count;
            if count - picked.len() < 3 && !picked.is_empty() {
                return None;
            }
            let mut kept = ring.clone();
            // From the last, so the ones before keep their places.
            for &index in picked.iter().rev() {
                let count = kept.len();
                let from = kept[(index + count - 1) % count].0;
                let next = (index + 1) % count;
                kept[next].1 = joined(from, kept[index].1, kept[next]);
                kept.remove(index);
            }
            rings.push(kept);
        }
        Some(rings)
    }

    /// The edit giving the text `rings` as its frame: a path keeps its fit,
    /// anchor, and inset; a box becomes a path with its fit and anchor and
    /// no inset.
    fn edit(&self, rings: &[Ring]) -> Option<Effect> {
        let path = Path::try_from(path(rings).to_svg()).ok()?;
        let frame = match &self.frame {
            TextFrame::Path {
                fit, anchor, inset, ..
            } => TextFrame::Path {
                path,
                fit: *fit,
                anchor: *anchor,
                inset: *inset,
            },
            TextFrame::Box { fit, anchor, .. } => TextFrame::Path {
                path,
                fit: match fit {
                    Fit::Shrink => PathFit::Shrink,
                    Fit::None => PathFit::None,
                    Fit::Best => PathFit::Best,
                    Fit::Grow => return None,
                },
                anchor: *anchor,
                inset: 0.0,
            },
            TextFrame::Point => return None,
        };
        let mut frame = serde_json::to_value(frame).ok()?;
        // A merge patch switches a box to a path by clearing the box.
        frame["box"] = Value::Null;
        Some(Effect::Edit(Edit {
            label: "edit_frame".to_owned(),
            ops: vec![Op::UpdateLayer {
                layer: Ref::from(self.layer),
                patch: self.lettering.patch(frame),
            }],
        }))
    }
}

/// `path`'s closed subpaths as rings. A last anchor back on its ring's first
/// is the first itself, and its segment the one closing the ring.
fn rings(path: &BezPath) -> Vec<Ring> {
    fn closed(mut ring: Ring, rings: &mut Vec<Ring>) {
        if ring.len() > 1
            && ring.last().map(|(point, _)| *point) == Some(ring[0].0)
            && let Some((_, segment)) = ring.pop()
        {
            ring[0].1 = segment;
        }
        if !ring.is_empty() {
            rings.push(ring);
        }
    }
    let mut rings = Vec::new();
    let mut ring = Ring::new();
    for element in path.elements() {
        match *element {
            PathEl::MoveTo(point) => {
                closed(std::mem::take(&mut ring), &mut rings);
                ring.push((point, Segment::Line));
            }
            PathEl::LineTo(point) => ring.push((point, Segment::Line)),
            PathEl::QuadTo(control, point) => ring.push((point, Segment::Quad(control))),
            PathEl::CurveTo(a, b, point) => ring.push((point, Segment::Cubic(a, b))),
            PathEl::ClosePath => closed(std::mem::take(&mut ring), &mut rings),
        }
    }
    closed(ring, &mut rings);
    rings
}

/// `rings` as a path, each closed.
fn path(rings: &[Ring]) -> BezPath {
    let to = |path: &mut BezPath, segment, point| match segment {
        Segment::Line => path.line_to(point),
        Segment::Quad(control) => path.quad_to(control, point),
        Segment::Cubic(a, b) => path.curve_to(a, b, point),
    };
    let mut path = BezPath::new();
    for ring in rings {
        let Some(&(start, closing)) = ring.first() else {
            continue;
        };
        path.move_to(start);
        for &(point, segment) in &ring[1..] {
            to(&mut path, segment, point);
        }
        if closing != Segment::Line {
            to(&mut path, closing, start);
        }
        path.close_path();
    }
    path
}

/// The one segment from `from` that replaces the two meeting at a removed
/// anchor, the first reaching it and the second leaving it for `next`: a
/// line where both were lines, else a cubic through their outer handles.
fn joined(from: Point, first: Segment, (next, second): (Point, Segment)) -> Segment {
    match (first, second) {
        (Segment::Line, Segment::Line) => Segment::Line,
        _ => Segment::Cubic(
            match first {
                Segment::Line => from,
                Segment::Quad(control) | Segment::Cubic(control, _) => control,
            },
            match second {
                Segment::Line => next,
                Segment::Quad(control) | Segment::Cubic(_, control) => control,
            },
        ),
    }
}

/// A press: on the selected text's anchor or segment, a drag of what it
/// picks; on another text, that text selected; else a marquee over the
/// selected text's anchors.
pub(crate) fn press(cx: &Context, sample: Sample) -> Press {
    let at = sample.page;
    let additive = sample.modifiers.shift();
    let selected = cx.selection.single().and_then(|layer| Shape::of(cx, layer));
    if let Some(shape) = &selected {
        let current = cx.selection.anchors();
        let picked = match shape.anchor_at(at, cx.view) {
            Some(anchor) if additive || !current.contains(&anchor) => {
                Some(with(if additive { current } else { &[] }, [anchor]))
            }
            Some(_) => Some(current.to_vec()),
            None => shape
                .segment_at(at, cx.view)
                .map(|ends| with(if additive { current } else { &[] }, ends)),
        };
        if let Some(picked) = picked {
            return Press {
                gesture: Some(Gesture::Reshape(Reshape {
                    shape: shape.clone(),
                    anchors: picked.clone(),
                    start: at,
                    current: at,
                })),
                effects: Vec::new(),
                anchors: Some(picked),
            };
        }
    }
    let inside = selected
        .as_ref()
        .is_some_and(|shape| shape.outline().contains(at));
    let other = cx
        .snapshot
        .controls()
        .iter()
        .rev()
        .filter(|control| control.picks && !control.locked && control.contains(at))
        .find_map(|control| Shape::of(cx, control.layer))
        .filter(|shape| Some(shape.layer) != selected.as_ref().map(|shape| shape.layer));
    if let (Some(shape), false) = (other, inside) {
        return Press::answers(vec![Effect::Selection(vec![shape.layer])]);
    }
    match selected {
        Some(shape) => Press::starts(Gesture::Pick(Pick {
            shape,
            start: at,
            current: at,
            before: match additive {
                true => cx.selection.anchors().to_vec(),
                false => Vec::new(),
            },
            inside,
        })),
        None => Press::answers(vec![Effect::Selection(Vec::new())]),
    }
}

/// `anchors` with `more` added, each once.
fn with(anchors: &[usize], more: impl IntoIterator<Item = usize>) -> Vec<usize> {
    let mut anchors = anchors.to_vec();
    for anchor in more {
        if !anchors.contains(&anchor) {
            anchors.push(anchor);
        }
    }
    anchors
}

/// The edit removing the selected text's picked anchors; nothing when a ring
/// would keep fewer than three, so Delete never opens or loses a frame.
pub(crate) fn delete(cx: &Context) -> Vec<Effect> {
    cx.selection
        .single()
        .and_then(|layer| Shape::of(cx, layer))
        .and_then(|shape| shape.edit(&shape.removed(cx.selection.anchors())?))
        .into_iter()
        .collect()
}

/// Picked anchors dragged from `start` to `current`, in page points.
pub(crate) struct Reshape {
    shape: Shape,
    anchors: Vec<usize>,
    start: Point,
    current: Point,
}

impl Reshape {
    pub fn current(&self) -> Point {
        self.current
    }

    pub fn moved(&mut self, samples: &[Sample]) {
        if let Some(last) = samples.last() {
            self.current = last.page;
        }
    }

    /// The edit the drag leaves, if it moved.
    pub fn finish(self) -> Vec<Effect> {
        if self.current == self.start {
            return Vec::new();
        }
        let rings = self.shape.moved(&self.anchors, self.current - self.start);
        self.shape.edit(&rings).into_iter().collect()
    }

    /// The frame where the drag puts it, with its anchors.
    pub fn preview(&self, overlay: &mut Overlay) {
        let moved = Shape {
            rings: self.shape.moved(&self.anchors, self.current - self.start),
            ..self.shape.clone()
        };
        overlay.anchors(&moved.outline(), &moved.anchors(), &self.anchors);
    }

    pub fn cursor(&self) -> CursorIcon {
        CursorIcon::Default
    }
}

/// A marquee over the selected text's anchors, from where it started to
/// where it is, in page points; a click outside the text drops the text.
pub(crate) struct Pick {
    shape: Shape,
    start: Point,
    current: Point,
    /// The anchors kept with Shift held.
    before: Vec<usize>,
    inside: bool,
}

impl Pick {
    pub fn rect(&self) -> Rect {
        Rect::from_points(self.start, self.current)
    }

    pub fn moved(&mut self, samples: &[Sample]) {
        if let Some(last) = samples.last() {
            self.current = last.page;
        }
    }

    /// The anchors it encloses, with those kept with Shift.
    pub fn picked(&self) -> Vec<usize> {
        let rect = self.rect();
        let enclosed = self
            .shape
            .anchors()
            .into_iter()
            .enumerate()
            .filter(|(_, anchor)| rect.contains(*anchor))
            .map(|(index, _)| index);
        with(&self.before, enclosed)
    }

    /// A click outside the text drops it.
    pub fn finish(self, view: &View) -> Vec<Effect> {
        let rect = self.rect();
        let click = rect.width().max(rect.height()) <= view.page_length(REACH) * 0.5;
        match click && !self.inside {
            true => vec![Effect::Selection(Vec::new())],
            false => Vec::new(),
        }
    }

    pub fn preview(&self, overlay: &mut Overlay) {
        overlay.marquee(self.rect());
    }
}
