//! The marquee and the lasso, as Photoshop's: a drag marks a rectangle or a
//! freehand shape of the page's pixels, which the canvas joins to its area
//! as the pointer lifts.

use kurbo::{BezPath, Point, Rect, Shape as _};

use crate::{
    Tool,
    area::Mode,
    draw::overlay::Overlay,
    gesture::{Context, Gesture, GestureLook, Press},
    view::Sample,
};

/// A press starts a shape, which replaces the area, adds to it with Shift
/// held, or takes from it with Alt held.
pub(crate) fn press(cx: &Context, sample: Sample) -> Press {
    let modifiers = sample.modifiers;
    let mode = if modifiers.shift() {
        Mode::Add
    } else if modifiers.alt() {
        Mode::Subtract
    } else {
        Mode::Replace
    };
    Press::starts(Gesture::Mark(Mark {
        lasso: cx.tool == Tool::Lasso,
        points: vec![sample.page],
        mode,
    }))
}

/// A shape being marked, in page points: the marquee's rectangle from its
/// first point to its last, or the lasso's path through them all, closed.
pub(crate) struct Mark {
    lasso: bool,
    /// Never empty.
    points: Vec<Point>,
    pub mode: Mode,
}

impl Mark {
    pub fn moved(&mut self, samples: &[Sample]) {
        for sample in samples {
            if !self.lasso {
                self.points.truncate(1);
            }
            if self.points.last() != Some(&sample.page) {
                self.points.push(sample.page);
            }
        }
    }

    pub fn shape(&self) -> BezPath {
        if !self.lasso {
            return self.rect().to_path(0.1);
        }
        let mut path = BezPath::new();
        let mut points = self.points.iter();
        if let Some(&first) = points.next() {
            path.move_to(first);
            for &point in points {
                path.line_to(point);
            }
            path.close_path();
        }
        path
    }

    fn rect(&self) -> Rect {
        let last = self.points.last().expect("a mark keeps its first point");
        Rect::from_points(self.points[0], *last)
    }

    pub fn preview(&self, overlay: &mut Overlay) {
        overlay.ants(&self.shape());
    }

    pub fn look(&self) -> GestureLook {
        match self.lasso {
            true => GestureLook::Stroke {
                points: self.points.len(),
                last: self.points.last().copied().unwrap_or_default(),
            },
            false => GestureLook::Rect(self.rect()),
        }
    }
}
