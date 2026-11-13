//! A frame's room for lines, in logical coordinates: `u` along the inline
//! axis and `v` along the block axis.

use anyhow::{Context, Result};
use koharu_scene::{Anchor, Fit, Frame, PathFit};
use smallvec::SmallVec;
use vello::kurbo::{Affine, BezPath, PathEl, Point, Rect};

/// Where a text's lines go.
pub(super) enum Room {
    /// Lines as long as they are, from the origin.
    Point,
    /// A box's inline extent, as wide as every line's room.
    Box {
        bounds: Rect,
        anchor: Anchor,
        /// The box grows along the block axis to hold the lines.
        grow: bool,
    },
    /// A closed path's inside, each line taking the widest span along its
    /// band, `inset` ems in from the outline.
    Path {
        outline: Outline,
        anchor: Anchor,
        inset: f32,
    },
}

impl Room {
    pub(super) fn new(frame: &Frame, vertical: bool) -> Result<Self> {
        Ok(match frame {
            Frame::Point => Self::Point,
            Frame::Box {
                bounds,
                fit,
                anchor,
            } => Self::Box {
                bounds: logical(vertical).transform_rect_bbox(Rect::new(
                    bounds.x,
                    bounds.y,
                    bounds.x + bounds.width,
                    bounds.y + bounds.height,
                )),
                anchor: *anchor,
                grow: *fit == Fit::Grow,
            },
            Frame::Path {
                path,
                anchor,
                inset,
                ..
            } => Self::Path {
                outline: Outline::new(
                    &BezPath::from_svg(path.as_str()).context("invalid path data")?,
                    vertical,
                ),
                anchor: *anchor,
                inset: *inset as f32,
            },
        })
    }
}

/// Whether `frame`'s text shrinks to fit it.
pub(super) fn shrinks(frame: &Frame) -> bool {
    match frame {
        Frame::Point => false,
        Frame::Box { fit, .. } => matches!(fit, Fit::Shrink | Fit::Best),
        Frame::Path { fit, .. } => matches!(fit, PathFit::Shrink | PathFit::Best),
    }
}

/// Whether `frame`'s text also grows to fill it.
pub(super) fn fills(frame: &Frame) -> bool {
    matches!(
        frame,
        Frame::Box { fit: Fit::Best, .. }
            | Frame::Path {
                fit: PathFit::Best,
                ..
            }
    )
}

/// From the text's own coordinates to logical ones. Horizontal text keeps x
/// and y; vertical text sets lines top to bottom and columns right to left,
/// so u is y and v is -x.
pub(super) fn logical(vertical: bool) -> Affine {
    match vertical {
        true => Affine::new([0.0, -1.0, 1.0, 0.0, 0.0, 0.0]),
        false => Affine::IDENTITY,
    }
}

/// The rectangle `u0..u1` by `v0..v1` in logical coordinates, in the text's
/// own.
pub(super) fn physical(vertical: bool, u0: f64, v0: f64, u1: f64, v1: f64) -> Rect {
    logical(vertical)
        .inverse()
        .transform_rect_bbox(Rect::new(u0, v0, u1, v1))
}

/// A closed outline's inside, in logical coordinates.
pub(super) struct Outline {
    edges: Vec<(Point, Point)>,
    /// Each corner's block position, in order.
    corners: Vec<f64>,
    pub bounds: Rect,
}

/// The spans inside an outline all along a band. A balloon has a span or two,
/// so they stay off the heap.
pub(super) type Spans = SmallVec<[(f64, f64); 4]>;

impl Outline {
    pub(super) fn new(path: &BezPath, vertical: bool) -> Self {
        let path = logical(vertical) * path.clone();
        let mut edges = Vec::new();
        let (mut first, mut last) = (Point::ZERO, Point::ZERO);
        vello::kurbo::flatten(path.iter(), 0.25, |element| match element {
            PathEl::MoveTo(point) => {
                if last != first {
                    edges.push((last, first));
                }
                first = point;
                last = first;
            }
            PathEl::LineTo(point) => {
                edges.push((last, point));
                last = point;
            }
            PathEl::ClosePath => {
                if last != first {
                    edges.push((last, first));
                }
                last = first;
            }
            _ => {}
        });
        if last != first {
            edges.push((last, first));
        }
        let bounds = edges.iter().flat_map(|(a, b)| [*a, *b]).fold(
            Rect::new(
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ),
            |rect, point| rect.union_pt(point),
        );
        let mut corners: Vec<f64> = edges.iter().map(|(start, _)| start.y).collect();
        corners.sort_unstable_by(f64::total_cmp);
        corners.dedup();
        Self {
            edges,
            corners,
            bounds,
        }
    }

    /// The inside's spans along the inline axis at block position `v`, by the
    /// even-odd rule.
    fn spans(&self, v: f64) -> Spans {
        let mut crossings: SmallVec<[f64; 8]> = self
            .edges
            .iter()
            .filter(|(a, b)| (a.y <= v) != (b.y <= v))
            .map(|(a, b)| a.x + (v - a.y) / (b.y - a.y) * (b.x - a.x))
            .collect();
        crossings.sort_unstable_by(f64::total_cmp);
        crossings
            .as_chunks::<2>()
            .0
            .iter()
            .map(|[a, b]| (*a, *b))
            .collect()
    }

    /// The spans inside the outline all along the band `v0..v1`. It is
    /// sampled at the middles of five equal parts of the band, never at its
    /// ends, so a band reaching a flat edge of the outline isn't cut to
    /// nothing, and at each of the outline's corners inside the band: the
    /// sides run straight between them, so a spike of a ragged balloon's
    /// outline can't slip between samples into the lines, which the balloon
    /// clips.
    pub(super) fn band(&self, v0: f64, v1: f64) -> Spans {
        let at = |sample: usize| v0 + (v1 - v0) * (sample as f64 + 0.5) / 5.0;
        let inside = self.corners.partition_point(|corner| *corner <= v0)
            ..self.corners.partition_point(|corner| *corner < v1);
        let mut spans = self.spans(at(0));
        for v in (1..5).map(at).chain(self.corners[inside].iter().copied()) {
            let next = self.spans(v);
            spans = spans
                .iter()
                .flat_map(|(a0, a1)| {
                    next.iter().filter_map(move |(b0, b1)| {
                        let (start, end) = (a0.max(*b0), a1.min(*b1));
                        (end > start).then_some((start, end))
                    })
                })
                .collect();
        }
        spans
    }
}

/// The inline position lines in `bands` are centred on: of their spans'
/// middles, the one they reach furthest on both sides of, all together, so a
/// tail or a wide corner of a single band can't pull it aside. Photoshop's
/// area type centres each line in its own span, so the lines of a lopsided
/// shape, such as a balloon's share beside another text, wander from side to
/// side; a letterer keeps a balloon's lines on one axis, and so does Koharu.
pub(super) fn axis(bands: &[Spans]) -> Option<f64> {
    let room = |axis: f64| -> f64 {
        bands
            .iter()
            .filter_map(|spans| reach(spans, axis))
            .map(|(start, end)| end - start)
            .sum()
    };
    bands
        .iter()
        .flatten()
        .map(|(start, end)| (start + end) / 2.0)
        .map(|axis| (room(axis), axis))
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, axis)| axis)
}

/// The span of `spans` holding `axis`, cut to reach as far on each side of
/// it; `None` where none holds it.
pub(super) fn reach(spans: &[(f64, f64)], axis: f64) -> Option<(f64, f64)> {
    let (start, end) = spans
        .iter()
        .find(|(start, end)| (*start..=*end).contains(&axis))?;
    let reach = (axis - start).min(end - axis);
    (reach > 0.0).then_some((axis - reach, axis + reach))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vertical_room_runs_down_the_page() -> Result<()> {
        let frame: Frame = serde_json::from_value(serde_json::json!({
            "box": {"bounds": {"x": 100.0, "y": 20.0, "width": 160.0, "height": 300.0}}
        }))?;
        let Room::Box { bounds, .. } = Room::new(&frame, true)? else {
            panic!("a box");
        };
        // Lines are as long as the box is tall, and stack from its right edge.
        assert_eq!((bounds.width(), bounds.height()), (300.0, 160.0));
        assert_eq!(bounds.y0, -260.0);
        Ok(())
    }

    /// The widest of `spans`.
    fn widest(spans: &[(f64, f64)]) -> (f64, f64) {
        spans
            .iter()
            .copied()
            .max_by(|a, b| (a.1 - a.0).total_cmp(&(b.1 - b.0)))
            .expect("a span")
    }

    #[test]
    fn a_band_across_a_diamond_is_its_narrowest_width() {
        let diamond = BezPath::from_svg("M0 50 L50 0 L100 50 L50 100 Z").expect("a path");
        let outline = Outline::new(&diamond, false);
        let (start, end) = widest(&outline.band(40.0, 50.0));
        // Sampled at 41 at the narrowest: 82 wide around the middle.
        assert!((end - start - 82.0).abs() < 0.5, "{start}..{end}");
    }

    #[test]
    fn a_band_keeps_off_a_spike_between_its_samples() {
        // A thin spike into a square from the left, at 48, where a band from
        // 40 to 60 is sampled at 42, 46, 50, 54, and 58.
        let notched =
            BezPath::from_svg("M0 0 L100 0 L100 100 L0 100 L0 49 L60 48 L0 47 Z").expect("a path");
        let outline = Outline::new(&notched, false);
        let (start, end) = widest(&outline.band(40.0, 60.0));
        assert!(start >= 59.9 && end <= 100.0, "{start}..{end}");
    }

    /// Five lines' bands down `outline`, 20 apart.
    fn bands(outline: &Outline) -> Vec<Spans> {
        (0..5)
            .map(|line| outline.band(f64::from(line) * 20.0, f64::from(line + 1) * 20.0))
            .collect()
    }

    #[test]
    fn a_bulge_beside_one_line_does_not_pull_the_others_aside() {
        // A 100-wide balloon bulging out to 160 beside its middle line, whose
        // own span's middle, 80, would leave the other four 40 each side.
        let bulging =
            BezPath::from_svg("M0 0 L100 0 L100 40 L160 40 L160 60 L100 60 L100 100 L0 100 Z")
                .expect("a path");
        let axis = axis(&bands(&Outline::new(&bulging, false))).expect("an axis");
        assert!((axis - 50.0).abs() < 1.0, "{axis}");
    }

    #[test]
    fn lines_in_a_lopsided_outline_keep_one_axis() {
        // Wide at the top, its right side slanting in to 60 at the bottom.
        let slanted = BezPath::from_svg("M0 0 L100 0 L60 100 L0 100 Z").expect("a path");
        let bands = bands(&Outline::new(&slanted, false));
        let axis = axis(&bands).expect("an axis");
        // Every line is centred on one axis, not on its own band's middle,
        // and the bottom line still reaches its band's left edge.
        for spans in &bands {
            let (start, end) = reach(spans, axis).expect("a span");
            assert!(((start + end) / 2.0 - axis).abs() < 1e-9);
        }
        let (start, _) = reach(&bands[4], axis).expect("a span");
        assert!(start > 10.0, "{start}");
    }
}
