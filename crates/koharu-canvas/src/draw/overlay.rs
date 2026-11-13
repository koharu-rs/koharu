//! The canvas's own drawing over the page, in viewport pixels: outlines,
//! handles, the marquee, and what gestures preview. Callers give page
//! geometry; lines and handles keep their size on screen at any zoom.

use koharu_scene::Rgba;
use vello::{
    Scene,
    kurbo::{Affine, BezPath, Circle, Line, Point, Rect, Stroke},
    peniko::{Color, Fill},
};

use crate::{
    Colors,
    control::{Control, Handle, Resize},
    draw::color,
    view::View,
};

/// A handle's side, in device-independent pixels.
const HANDLE: f64 = 6.0;

pub(crate) struct Overlay<'a> {
    scene: Scene,
    view: &'a View,
    colors: Colors,
}

impl<'a> Overlay<'a> {
    pub fn new(view: &'a View, colors: Colors) -> Self {
        Self {
            scene: Scene::new(),
            view,
            colors,
        }
    }

    /// A page path outlined a device pixel wide.
    pub fn outline(&mut self, path: &BezPath) {
        let path = self.view.to_screen() * path.clone();
        self.scene.stroke(
            &Stroke::new(self.view.scale()),
            Affine::IDENTITY,
            color(self.colors.selection),
            None,
            &path,
        );
    }

    pub fn control(&mut self, control: &Control) {
        self.outline(control.outline());
    }

    /// The corner squares of a control resizing by them, and the rotate
    /// handle on its stem; a control resizing by its edges catches them
    /// along its sides, which draw nothing more, as the editor's did.
    pub fn handles(&mut self, control: &Control) {
        let to_screen = self.view.to_screen();
        let selection = color(self.colors.selection);
        let side = HANDLE * self.view.scale();
        let line = Stroke::new(self.view.scale());
        for handle in control.handles() {
            let at = to_screen * control.handle_point(handle, self.view);
            let corner = !matches!(
                handle,
                Handle::North | Handle::East | Handle::South | Handle::West | Handle::Rotate
            );
            if handle == Handle::Rotate {
                let top = to_screen * control.handle_point(Handle::North, self.view);
                self.scene.stroke(
                    &line,
                    Affine::IDENTITY,
                    selection,
                    None,
                    &Line::new(top, at),
                );
            } else if !corner && control.resize != Resize::All {
                continue;
            }
            let square = Rect::from_center_size(at, (side, side));
            self.scene
                .fill(Fill::NonZero, Affine::IDENTITY, selection, None, &square);
            self.scene
                .stroke(&line, Affine::IDENTITY, Color::WHITE, None, &square);
        }
    }

    /// A selection rectangle over the page.
    pub fn marquee(&mut self, rect: Rect) {
        let rect = self.view.to_screen().transform_rect_bbox(rect);
        let selection = color(self.colors.selection);
        self.scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            selection.with_alpha(0.1),
            None,
            &rect,
        );
        self.scene.stroke(
            &Stroke::new(self.view.scale()),
            Affine::IDENTITY,
            selection,
            None,
            &rect,
        );
    }

    /// A page rectangle outlined in dashes, as a frame being dragged out.
    pub fn dashed(&mut self, rect: Rect) {
        let rect = self.view.to_screen().transform_rect_bbox(rect);
        let scale = self.view.scale();
        let dashed = Stroke::new(scale).with_dashes(0.0, [4.0 * scale, 3.0 * scale]);
        self.scene.stroke(
            &dashed,
            Affine::IDENTITY,
            color(self.colors.selection),
            None,
            &rect,
        );
    }

    /// A page path filled through the camera.
    pub fn fill(&mut self, path: &BezPath, tint: Rgba) {
        self.scene.fill(
            Fill::NonZero,
            self.view.to_screen(),
            color(tint),
            None,
            path,
        );
    }

    /// The brush's ring at `at` in the viewport, `diameter` page pixels
    /// across: white on a black ring, so it shows on any page.
    pub fn ring(&mut self, at: Point, diameter: f64) {
        let scale = self.view.scale();
        let radius = diameter * self.view.camera().scale * 0.5;
        let ring = Stroke::new(scale);
        let outer = Circle::new(at, radius + scale);
        self.scene
            .stroke(&ring, Affine::IDENTITY, Color::BLACK, None, &outer);
        self.scene.stroke(
            &ring,
            Affine::IDENTITY,
            Color::WHITE,
            None,
            &Circle::new(at, radius),
        );
    }

    /// Another scene laid in at `transform`, viewport pixels being its
    /// output.
    pub fn scene(&mut self, scene: &Scene, transform: Affine) {
        self.scene.append(scene, Some(transform));
    }

    pub fn finish(self) -> Scene {
        self.scene
    }
}
