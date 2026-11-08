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
        Frame::Box { fit, .. } => *fit == Fit::Shrink,
        Frame::Path { fit, .. } => *fit == PathFit::Shrink,
    }
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
    pub bounds: Rect,
}

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
        Self { edges, bounds }
    }

    /// The inside's spans along the inline axis at block position `v`, by the
    /// even-odd rule. A balloon has a span or two, so they stay off the heap.
    fn spans(&self, v: f64) -> SmallVec<[(f64, f64); 4]> {
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

    /// The widest span inside the outline all along the band `v0..v1`. It is
    /// sampled at the middles of five equal parts of the band, never at its
    /// ends, so a band reaching a flat edge of the outline isn't cut to
    /// nothing.
    pub(super) fn widest(&self, v0: f64, v1: f64) -> Option<(f64, f64)> {
        let at = |sample: usize| v0 + (v1 - v0) * (sample as f64 + 0.5) / 5.0;
        let mut spans = self.spans(at(0));
        for sample in 1..5 {
            let next = self.spans(at(sample));
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
            .into_iter()
            .max_by(|a, b| (a.1 - a.0).total_cmp(&(b.1 - b.0)))
    }
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

    #[test]
    fn a_band_across_a_diamond_is_its_narrowest_width() {
        let diamond = BezPath::from_svg("M0 50 L50 0 L100 50 L50 100 Z").expect("a path");
        let outline = Outline::new(&diamond, false);
        let (start, end) = outline.widest(40.0, 50.0).expect("a span");
        // Sampled at 41 at the narrowest: 82 wide around the middle.
        assert!((end - start - 82.0).abs() < 0.5, "{start}..{end}");
    }
}
