//! Where the page shows: the canvas's place in the window, its scale factor,
//! and the camera from page points to the viewport's physical pixels, with
//! the editor's pan and zoom. Pointer events become page points here, and
//! nowhere else.

use serde::Serialize;
use specta::Type;
use ui_events::{keyboard::Modifiers, pointer::PointerState};
use vello::kurbo::{Affine, Point, Rect, Size, TranslateScale, Vec2};

/// The smallest and largest zoom, as the editor's canvas allowed.
const ZOOM: (f64, f64) = (0.02, 16.0);

/// How much of the page stays in view, in device-independent pixels.
const OVERLAP: f64 = 100.0;

/// A pointer event's place, as the canvas uses it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Sample {
    /// The page point under the pointer.
    pub page: Point,
    /// The pointer in the viewport's physical pixels, for what follows the
    /// screen rather than the page: a pan, and the brush ring.
    pub screen: Point,
    pub modifiers: Modifiers,
}

/// Where the page shows, for the interface over the canvas: a page point `p`
/// sits at `p * zoom + (x, y)` in device-independent pixels from the
/// viewport's corner.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Type)]
pub struct Camera {
    pub zoom: f64,
    pub x: f64,
    pub y: f64,
}

pub(crate) struct View {
    /// Where the canvas sits in the window, in physical pixels.
    viewport: Rect,
    /// Physical pixels to a device-independent one.
    scale: f64,
    camera: TranslateScale,
    /// Whether the camera follows the viewport's size, until the user moves
    /// it.
    fitted: bool,
    page: Size,
}

impl Default for View {
    fn default() -> Self {
        Self {
            viewport: Rect::ZERO,
            scale: 1.0,
            camera: TranslateScale::default(),
            fitted: true,
            page: Size::ZERO,
        }
    }
}

impl View {
    /// Moves or resizes the canvas in the window; whether anything changed.
    pub fn set_viewport(&mut self, viewport: Rect, scale: f64) -> bool {
        if viewport == self.viewport && scale == self.scale {
            return false;
        }
        self.viewport = viewport;
        self.scale = scale;
        if self.fitted {
            self.fit();
        }
        true
    }

    /// The shown page's size; `refit` for another page.
    pub fn set_page(&mut self, page: Size, refit: bool) {
        self.page = page;
        if refit {
            self.fit();
        }
    }

    pub fn camera(&self) -> TranslateScale {
        self.camera
    }

    /// The camera in device-independent pixels.
    pub fn placement(&self) -> Camera {
        Camera {
            zoom: self.camera.scale / self.scale,
            x: self.camera.translation.x / self.scale,
            y: self.camera.translation.y / self.scale,
        }
    }

    /// Zooms about the viewport's centre to `zoom` device-independent pixels
    /// a page pixel, as a zoom slider does.
    pub fn zoom_to(&mut self, zoom: f64) {
        let centre = (self.viewport.size().to_vec2() * 0.5).to_point();
        let factor = zoom * self.scale / self.camera.scale;
        self.set_camera(self.zoomed(centre, factor));
    }

    /// Moves the camera, kept so some of the page stays in view.
    pub fn set_camera(&mut self, camera: TranslateScale) {
        self.camera = self.clamped(camera);
        self.fitted = false;
    }

    /// The page contained in the viewport and centred, kept so as the
    /// viewport changes until the camera moves.
    pub fn fit(&mut self) -> TranslateScale {
        let viewport = self.viewport.size();
        self.camera = match viewport.is_zero_area() || self.page.is_zero_area() {
            true => TranslateScale::default(),
            false => {
                let zoom = (viewport.width / self.page.width)
                    .min(viewport.height / self.page.height)
                    .max(f64::EPSILON);
                TranslateScale::new(
                    (viewport.to_vec2() - self.page.to_vec2() * zoom) * 0.5,
                    zoom,
                )
            }
        };
        self.fitted = true;
        self.camera
    }

    /// The camera zoomed by `factor` about `at` in the viewport, which
    /// stays where it is, within the zoom's range.
    pub fn zoomed(&self, at: Point, factor: f64) -> TranslateScale {
        let zoom = (self.camera.scale * factor).clamp(ZOOM.0, ZOOM.1);
        TranslateScale::from_scale_about(zoom / self.camera.scale, at) * self.camera
    }

    /// A pointer state, in window pixels, as the canvas uses it.
    pub fn sample(&self, state: &PointerState) -> Sample {
        let screen = state.physical_point() - self.viewport.origin().to_vec2();
        Sample {
            page: self.page_point(screen),
            screen,
            modifiers: state.modifiers,
        }
    }

    /// The page point the viewport shows at `screen`.
    pub fn page_point(&self, screen: Point) -> Point {
        self.camera.inverse() * screen
    }

    /// `dip` device-independent pixels on screen, in page units.
    pub fn page_length(&self, dip: f64) -> f64 {
        dip * self.scale / self.camera.scale
    }

    /// From page points to the viewport's physical pixels.
    pub fn to_screen(&self) -> Affine {
        Affine::from(self.camera)
    }

    pub fn size(&self) -> Size {
        self.viewport.size()
    }

    pub fn scale(&self) -> f64 {
        self.scale
    }

    /// The page rectangle the viewport shows.
    pub fn visible(&self) -> Rect {
        self.camera.inverse() * self.size().to_rect()
    }

    /// `camera` kept so 100 device-independent pixels of the page stay in
    /// view; a page too small for that on an axis is centred on it.
    fn clamped(&self, camera: TranslateScale) -> TranslateScale {
        let overlap = OVERLAP * self.scale;
        let axis = |offset: f64, viewport: f64, page: f64| {
            let (min, max) = (overlap - page * camera.scale, viewport - overlap);
            match min <= max {
                true => offset.clamp(min, max),
                false => (viewport - page * camera.scale) * 0.5,
            }
        };
        let (viewport, page) = (self.size(), self.page);
        TranslateScale::new(
            Vec2::new(
                axis(camera.translation.x, viewport.width, page.width),
                axis(camera.translation.y, viewport.height, page.height),
            ),
            camera.scale,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zooming_keeps_the_point_under_the_cursor_still() {
        let view = View {
            camera: TranslateScale::new(Vec2::new(50.0, 30.0), 1.0),
            ..View::default()
        };
        let at = Point::new(400.0, 300.0);
        let zoomed = view.zoomed(at, 2.0);
        assert_eq!(zoomed.scale, 2.0);
        assert_eq!(zoomed.inverse() * at, view.camera.inverse() * at);
        assert_eq!(view.zoomed(at, 1e6).scale, 16.0);
        assert_eq!(view.zoomed(at, 1e-6).scale, 0.02);
    }

    #[test]
    fn a_clamped_camera_keeps_a_hundred_pixels_of_the_page_in_view() {
        let mut view = View::default();
        view.set_viewport(Rect::new(0.0, 0.0, 1000.0, 800.0), 1.5);
        view.set_page(Size::new(2000.0, 3000.0), false);
        view.set_camera(TranslateScale::new(Vec2::new(5000.0, -9000.0), 1.0));
        assert_eq!(
            view.camera().translation,
            Vec2::new(1000.0 - 150.0, 150.0 - 3000.0)
        );
    }
}
