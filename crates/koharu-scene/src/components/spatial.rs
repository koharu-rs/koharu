use revision::revisioned;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::{
    Error, Result,
    component::{Component, ValidationContext},
};

use super::Origin;

#[revisioned(revision = 1)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[revisioned(revision = 1)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct Geometry {
    pub origin: Origin,
    pub points: Vec<Point>,
}

impl Geometry {
    #[must_use]
    pub fn rectangle(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            origin: Origin::User,
            points: vec![
                Point { x, y },
                Point { x: x + width, y },
                Point {
                    x: x + width,
                    y: y + height,
                },
                Point { x, y: y + height },
            ],
        }
    }

    /// Builds the four corners of `rectangle` turned about its centre.
    ///
    /// The corner order is what recovers the angle: readers measure it from
    /// the leading edge, so a rotation expressed as a polygon survives a round
    /// trip only while the points stay in this order.
    #[must_use]
    pub fn rotated_rectangle(x: f64, y: f64, width: f64, height: f64, angle_degrees: f64) -> Self {
        let (sin, cos) = angle_degrees.to_radians().sin_cos();
        let (half_width, half_height) = (width * 0.5, height * 0.5);
        let (center_x, center_y) = (x + half_width, y + half_height);
        Self {
            origin: Origin::User,
            points: [
                (-half_width, -half_height),
                (half_width, -half_height),
                (half_width, half_height),
                (-half_width, half_height),
            ]
            .into_iter()
            .map(|(x, y)| Point {
                x: center_x + x * cos - y * sin,
                y: center_y + x * sin + y * cos,
            })
            .collect(),
        }
    }
}

impl Component for Geometry {
    const KIND: &'static str = "dev.koharu.geometry";

    fn validate(&self, _context: &ValidationContext<'_>) -> Result<()> {
        self.origin.validate()?;
        if (3..=1_000_000).contains(&self.points.len())
            && self
                .points
                .iter()
                .all(|point| point.x.is_finite() && point.y.is_finite())
        {
            Ok(())
        } else {
            Err(Error::invalid(
                "geometry must contain finite polygon points",
            ))
        }
    }

    fn origin(&self) -> Option<&Origin> {
        Some(&self.origin)
    }

    fn set_origin(&mut self, origin: Origin) -> bool {
        self.origin = origin;
        true
    }
}

#[revisioned(revision = 1)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct Visibility {
    pub origin: Origin,
    pub visible: bool,
    pub opacity: f32,
}

impl Component for Visibility {
    const KIND: &'static str = "dev.koharu.visibility";

    fn validate(&self, _context: &ValidationContext<'_>) -> Result<()> {
        self.origin.validate()?;
        if self.opacity.is_finite() && (0.0..=1.0).contains(&self.opacity) {
            Ok(())
        } else {
            Err(Error::invalid(
                "opacity must be finite and between zero and one",
            ))
        }
    }

    fn origin(&self) -> Option<&Origin> {
        Some(&self.origin)
    }

    fn set_origin(&mut self, origin: Origin) -> bool {
        self.origin = origin;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{Geometry, Point};

    #[test]
    fn rotated_rectangle_without_rotation_matches_rectangle() {
        let rotated = Geometry::rotated_rectangle(10.0, 20.0, 100.0, 40.0, 0.0);
        let plain = Geometry::rectangle(10.0, 20.0, 100.0, 40.0);

        assert_eq!(rotated.origin, plain.origin);
        for (rotated, plain) in rotated.points.iter().zip(&plain.points) {
            assert!((rotated.x - plain.x).abs() < 1e-9);
            assert!((rotated.y - plain.y).abs() < 1e-9);
        }
    }

    #[test]
    fn rotated_rectangle_turns_the_corners_about_the_centre() {
        let geometry = Geometry::rotated_rectangle(50.0, 70.0, 100.0, 20.0, 90.0);

        let expected = [
            Point { x: 110.0, y: 30.0 },
            Point { x: 110.0, y: 130.0 },
            Point { x: 90.0, y: 130.0 },
            Point { x: 90.0, y: 30.0 },
        ];
        assert_eq!(geometry.points.len(), expected.len());
        for (point, expected) in geometry.points.iter().zip(&expected) {
            assert!((point.x - expected.x).abs() < 1e-9, "{point:?}");
            assert!((point.y - expected.y).abs() < 1e-9, "{point:?}");
        }
    }
}
