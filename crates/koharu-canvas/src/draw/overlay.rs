//! The canvas's own drawing over the page, in viewport pixels: outlines,
//! handles, the marquee, and what gestures preview. Callers give page
//! geometry; lines and handles keep their size on screen at any zoom.

use koharu_renderer::{Drawing, Frame};
use koharu_scene::Rgba;
use kurbo::{Affine, BezPath, Circle, Line, Point, Rect, Shape, Stroke};
use vello_gpu::peniko::{Color, Fill};

use crate::{
    Colors,
    control::{Control, Handle, Resize},
    draw::color,
    view::View,
};

/// A handle's side, in device-independent pixels.
const HANDLE: f64 = 6.0;

/// How finely a curve is flattened, in viewport pixels.
const TOLERANCE: f64 = 0.1;

pub(crate) struct Overlay<'a, 'g> {
    drawing: &'a mut Drawing<'g>,
    view: &'a View,
    colors: Colors,
}

impl<'a, 'g> Overlay<'a, 'g> {
    pub fn new(drawing: &'a mut Drawing<'g>, view: &'a View, colors: Colors) -> Self {
        Self {
            drawing,
            view,
            colors,
        }
    }

    /// `shape` filled in `color` through `transform`.
    fn filled(&mut self, transform: Affine, color: Color, shape: &impl Shape) {
        let scene = &mut self.drawing.scene;
        scene.set_transform(transform);
        scene.set_fill_rule(Fill::NonZero);
        scene.set_paint(color);
        scene.fill_path(&shape.to_path(TOLERANCE));
    }

    /// `shape`, in viewport pixels, stroked by `stroke` in `color`.
    fn stroked(&mut self, stroke: Stroke, color: Color, shape: &impl Shape) {
        let scene = &mut self.drawing.scene;
        scene.set_transform(Affine::IDENTITY);
        scene.set_stroke(stroke);
        scene.set_paint(color);
        scene.stroke_path(&shape.to_path(TOLERANCE));
    }

    /// A page path outlined a device pixel wide.
    pub fn outline(&mut self, path: &BezPath) {
        let path = self.view.to_screen() * path.clone();
        let selection = color(self.colors.selection);
        self.stroked(Stroke::new(self.view.scale()), selection, &path);
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
                self.stroked(line.clone(), selection, &Line::new(top, at));
            } else if !corner && control.resize != Resize::All {
                continue;
            }
            let square = Rect::from_center_size(at, (side, side));
            self.filled(Affine::IDENTITY, selection, &square);
            self.stroked(line.clone(), Color::WHITE, &square);
        }
    }

    /// A selection rectangle over the page.
    pub fn marquee(&mut self, rect: Rect) {
        let rect = self.view.to_screen().transform_rect_bbox(rect);
        let selection = color(self.colors.selection);
        self.filled(Affine::IDENTITY, selection.with_alpha(0.1), &rect);
        self.stroked(Stroke::new(self.view.scale()), selection, &rect);
    }

    /// A page rectangle outlined in dashes, as a frame being dragged out.
    pub fn dashed(&mut self, rect: Rect) {
        let rect = self.view.to_screen().transform_rect_bbox(rect);
        let scale = self.view.scale();
        let dashed = Stroke::new(scale).with_dashes(0.0, [4.0 * scale, 3.0 * scale]);
        let selection = color(self.colors.selection);
        self.stroked(dashed, selection, &rect);
    }

    /// A page path filled through the camera.
    pub fn fill(&mut self, path: &BezPath, tint: Rgba) {
        self.filled(self.view.to_screen(), color(tint), path);
    }

    /// The brush's ring at `at` in the viewport, `diameter` page pixels
    /// across: white on a black ring, so it shows on any page.
    pub fn ring(&mut self, at: Point, diameter: f64) {
        let scale = self.view.scale();
        let radius = diameter * self.view.camera().scale * 0.5;
        let ring = Stroke::new(scale);
        self.stroked(ring.clone(), Color::BLACK, &Circle::new(at, radius + scale));
        self.stroked(ring, Color::WHITE, &Circle::new(at, radius));
    }

    /// `frame` painted through `transform` in page space, then the camera,
    /// over what the viewport shows.
    pub fn frame(&mut self, frame: &Frame, transform: Affine) -> anyhow::Result<()> {
        frame.paint(
            self.view.to_screen() * transform,
            self.view.size().to_rect(),
            self.drawing,
        )
    }
}
