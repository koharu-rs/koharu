//! The canvas's own drawing over the page, in viewport pixels: outlines,
//! handles, the marquee, and what gestures preview. Callers give page
//! geometry; lines and handles keep their size on screen at any zoom.

use koharu_renderer::{Drawing, Frame, Label};
use koharu_scene::Rgba;
use kurbo::{Affine, BezPath, Circle, Line, Point, Rect, RoundedRect, Shape, Stroke, Vec2};
use vello_gpu::peniko::{BlendMode, Color, Compose, Fill, Mix};

use crate::{
    Colors,
    control::{Control, Handle, Resize},
    draw::color,
    view::View,
};

/// A handle's side, in device-independent pixels.
const HANDLE: f64 = 6.0;

/// A Story number's height, in device-independent pixels, as the Story
/// panel's.
const NUMBER: f64 = 16.0;

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

    /// When `control`'s text runs past its frame, the overflow mark on the
    /// frame's top-left corner, where Photoshop marks paragraph type that
    /// overflows its box on the lower-right one. Koharu still draws the text
    /// that runs past, so no line goes missing from an export.
    pub fn overflow(&mut self, control: &Control) {
        if control.overflows {
            let at = self.view.to_screen() * control.handle_point(Handle::NorthWest, self.view);
            self.overflow_mark(at);
        }
    }

    /// Photoshop's overflow mark, a plus in a hollow square, centred on `at`
    /// in the viewport.
    fn overflow_mark(&mut self, at: Point) {
        let scale = self.view.scale();
        let side = (HANDLE + 4.0) * scale;
        let square = Rect::from_center_size(at, (side, side));
        let selection = color(self.colors.selection);
        let line = Stroke::new(scale);
        self.filled(Affine::IDENTITY, Color::WHITE, &square);
        self.stroked(line.clone(), selection, &square);
        let arm = side * 0.3;
        let across = Vec2::new(arm, 0.0);
        let down = Vec2::new(0.0, arm);
        self.stroked(
            line.clone(),
            selection,
            &Line::new(at - across, at + across),
        );
        self.stroked(line, selection, &Line::new(at - down, at + down));
    }

    /// What's selected while typing: the selection's colour, faint, over
    /// each of `rects`, which `transform` takes from the text to the
    /// viewport, under the glyphs.
    pub fn highlight(&mut self, rects: &[Rect], transform: Affine) {
        let tint = color(self.colors.selection).with_alpha(0.3);
        for rect in rects {
            self.filled(transform, tint, rect);
        }
    }

    /// The typing caret: `line`, which `transform` takes to the viewport, a
    /// device-independent pixel wide.
    pub fn caret(&mut self, line: Line, transform: Affine) {
        let selection = color(self.colors.selection);
        self.stroked(
            Stroke::new(self.view.scale()),
            selection,
            &(transform * line),
        );
    }

    /// The input method's composition underlined, as an editor shows it:
    /// under horizontal text and right of a vertical column, a tenth of the
    /// line in from its box, clear of the box's own outline, and thicker
    /// where the input method has selected.
    pub fn composing(&mut self, rects: &[(Rect, bool)], transform: Affine, vertical: bool) {
        let selection = color(self.colors.selection);
        for &(rect, selected) in rects {
            let edge = match vertical {
                true => {
                    let x = rect.x1 - rect.width() * 0.1;
                    Line::new((x, rect.y0), (x, rect.y1))
                }
                false => {
                    let y = rect.y1 - rect.height() * 0.1;
                    Line::new((rect.x0, y), (rect.x1, y))
                }
            };
            let width = if selected { 2.0 } else { 1.0 } * self.view.scale();
            self.stroked(Stroke::new(width), selection, &(transform * edge));
        }
    }

    /// The page under `tint`, but where `clear` draws, which shows through:
    /// a selected mask or clean-up seen on its own, as Photoshop shows a mask
    /// in its rubylith overlay.
    pub fn isolate(
        &mut self,
        page: Rect,
        tint: Color,
        clear: Option<&Frame>,
    ) -> anyhow::Result<()> {
        let to_screen = self.view.to_screen();
        let area = to_screen * page.to_path(TOLERANCE);
        let viewport = self.view.size().to_rect();
        let scene = &mut self.drawing.scene;
        scene.set_transform(Affine::IDENTITY);
        scene.push_layer(Some(&area), None, None, None, None);
        scene.set_fill_rule(Fill::NonZero);
        scene.set_paint(tint);
        scene.fill_path(&area);
        if let Some(clear) = clear {
            let cut = BlendMode::new(Mix::Normal, Compose::DestOut);
            scene.push_layer(Some(&area), Some(cut), None, None, None);
            clear.paint(to_screen, viewport, self.drawing)?;
            self.drawing.scene.pop_layer();
        }
        self.drawing.scene.pop_layer();
        Ok(())
    }

    /// A text's frame while Story shows: faint, or full and wider when its
    /// entry is pointed at.
    pub fn story_frame(&mut self, control: &Control, pointed: bool) {
        let path = self.view.to_screen() * control.outline().clone();
        let selection = color(self.colors.selection);
        let (width, alpha) = match pointed {
            true => (2.0, 1.0),
            false => (1.0, 0.5),
        };
        let stroke = Stroke::new(width * self.view.scale());
        self.stroked(stroke, selection.with_alpha(alpha), &path);
    }

    /// `control`'s place in reading order in a pill centred on its frame's
    /// top-left corner: filled for dialogue and outlined for a sound effect,
    /// as Story's numbers are, and the overflow mark beside it, where the
    /// pill would hide it, when its text runs past its frame. `label` is the
    /// number laid out in the colour that shows on the pill.
    pub fn story_number(
        &mut self,
        control: &Control,
        label: &Label,
        sfx: bool,
    ) -> anyhow::Result<()> {
        let scale = self.view.scale();
        let corner = self.view.to_screen() * control.corners()[0];
        let text = label.bounds();
        let height = NUMBER * scale;
        let width = ((text.width() + 8.0) * scale).max(height);
        let pill = RoundedRect::from_rect(
            Rect::from_center_size(corner, (width, height)),
            height * 0.5,
        );
        let selection = color(self.colors.selection);
        match sfx {
            true => {
                self.filled(Affine::IDENTITY, Color::WHITE, &pill);
                self.stroked(Stroke::new(scale), selection, &pill);
            }
            false => self.filled(Affine::IDENTITY, selection, &pill),
        }
        let middle = pill.rect().center().to_vec2();
        let at = Affine::translate(middle)
            * Affine::scale(scale)
            * Affine::translate(-text.center().to_vec2());
        label.draw(self.drawing, at)?;
        if control.overflows {
            let side = (HANDLE + 4.0) * scale;
            let beside = Point::new(pill.rect().x1 + 2.0 * scale + side * 0.5, corner.y);
            self.overflow_mark(beside);
        }
        Ok(())
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

    /// A frame's page `outline` and its `anchors`, as Photoshop draws a
    /// path's: hollow squares, and the `picked` ones filled.
    pub fn anchors(&mut self, outline: &BezPath, anchors: &[Point], picked: &[usize]) {
        self.outline(outline);
        let to_screen = self.view.to_screen();
        let selection = color(self.colors.selection);
        let side = (HANDLE - 1.0) * self.view.scale();
        let line = Stroke::new(self.view.scale());
        for (index, &anchor) in anchors.iter().enumerate() {
            let square = Rect::from_center_size(to_screen * anchor, (side, side));
            let fill = match picked.contains(&index) {
                true => selection,
                false => Color::WHITE,
            };
            self.filled(Affine::IDENTITY, fill, &square);
            self.stroked(line.clone(), selection, &square);
        }
    }

    /// A selection rectangle over the page.
    pub fn marquee(&mut self, rect: Rect) {
        let rect = self.view.to_screen().transform_rect_bbox(rect);
        let selection = color(self.colors.selection);
        self.filled(Affine::IDENTITY, selection.with_alpha(0.1), &rect);
        self.stroked(Stroke::new(self.view.scale()), selection, &rect);
    }

    /// A page path as marching ants, as Photoshop draws a selection's edge:
    /// black dashes over white, a device pixel wide, so it shows on any page.
    pub fn ants(&mut self, path: &BezPath) {
        let path = self.view.to_screen() * path.clone();
        let scale = self.view.scale();
        let dashed = Stroke::new(scale).with_dashes(0.0, [4.0 * scale, 4.0 * scale]);
        self.stroked(Stroke::new(scale), Color::WHITE, &path);
        self.stroked(dashed, Color::BLACK, &path);
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
