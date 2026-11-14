//! The canvas: it owns the view, what's shown, the selection, the tool's
//! settings, and the gesture in progress. It routes each pointer event to
//! the tool or the gesture, applies what they answer about its own state,
//! and draws.

use std::sync::Arc;

use anyhow::Context as _;
use cursor_icon::CursorIcon;
use image::RgbaImage;
use koharu_renderer::{Frame, Gpu};
use koharu_scene::{LayerId, Session};
use parking_lot::Mutex;
use ui_events::pointer::{
    PointerButton, PointerButtonEvent, PointerEvent, PointerGesture, PointerGestureEvent,
    PointerInfo, PointerScrollEvent, PointerType, PointerUpdate,
};
use vello::{
    Scene,
    kurbo::{Affine, Point, Rect, Size, TranslateScale, Vec2},
    peniko::{Color, Fill},
    wgpu,
};

use crate::{
    Brush, Camera, Colors, Effect, Tool,
    control::Control,
    draw::{Target, color, overlay::Overlay, strip_origin},
    gesture::{Context, Gesture, GestureLook, Press},
    selection::Selection,
    snapshot::{ImageSizes, Snapshot},
    tools::{self, pan::Pan, text::Draft},
    view::{Sample, View},
};

/// The page viewport over one open project. It reads the project, never
/// writes it: every change leaves as an `Effect::Edit` for the host.
pub struct Canvas {
    gpu: Arc<Gpu>,
    session: Arc<Mutex<Session>>,
    view: View,
    colors: Colors,
    snapshot: Option<Snapshot>,
    sizes: ImageSizes,
    tool: Tool,
    brush: Brush,
    selection: Selection,
    gesture: Option<Active>,
    /// The pointer's last place, while it's over the canvas.
    pointer: Option<Sample>,
    /// Bumped by every `set_frame`.
    generation: u64,
    target: Option<Target>,
    /// What the last draw showed: a draw that would show the same does
    /// nothing.
    drawn: Option<Look>,
}

/// What a draw shows: everything it reads, compared in one place.
#[derive(Clone, Debug, PartialEq)]
struct Look {
    size: Size,
    /// The camera's, which has no equality of its own.
    translation: Vec2,
    zoom: f64,
    /// A count every `set_frame` bumps, not the frame's revision: a frame of
    /// the same revision draws differently once its fonts load or
    /// `[renderer]` changes.
    generation: u64,
    /// The layers a gesture lifts out of the page to draw itself.
    lifted: Vec<LayerId>,
    scale: f64,
    colors: Colors,
    tool: Tool,
    brush: Brush,
    selection: Vec<LayerId>,
    hover: Option<LayerId>,
    /// Where the brush's ring is, in the viewport.
    ring: Option<Point>,
    gesture: GestureLook,
}

/// A gesture, and the pointer that owns it.
struct Active {
    pointer: u64,
    gesture: Gesture,
}

impl Canvas {
    pub fn new(gpu: Arc<Gpu>, session: Arc<Mutex<Session>>) -> Self {
        Self {
            gpu,
            session,
            view: View::default(),
            colors: Colors::default(),
            snapshot: None,
            sizes: ImageSizes::default(),
            tool: Tool::default(),
            brush: Brush::default(),
            selection: Selection::default(),
            gesture: None,
            pointer: None,
            generation: 0,
            target: None,
            drawn: None,
        }
    }

    /// Where the canvas sits in the window, in physical pixels, as pointer
    /// events place the pointer, and how many make a device-independent one.
    pub fn set_viewport(&mut self, viewport: Rect, scale: f64) {
        self.view.set_viewport(viewport, scale);
    }

    /// The frame to show, rendered by the host after any change. A frame of
    /// another page fits the camera to it.
    pub fn set_frame(&mut self, frame: Frame) {
        // The project changed under a gesture, by another writer: it would
        // write over that change, so it's dropped, as the editor's was.
        if self
            .snapshot
            .as_ref()
            .is_some_and(|old| old.frame().revision() != frame.revision())
        {
            self.gesture = None;
        }
        let other = self
            .snapshot
            .as_ref()
            .is_none_or(|snapshot| snapshot.page().id != frame.page());
        self.snapshot = Snapshot::new(frame, &self.session.lock(), &mut self.sizes);
        self.generation += 1;
        self.view.set_page(self.page_size(), other);
    }

    pub fn set_colors(&mut self, colors: Colors) {
        self.colors = colors;
    }

    pub fn camera(&self) -> TranslateScale {
        self.view.camera()
    }

    /// Moves the camera, kept so some of the page stays in view.
    pub fn set_camera(&mut self, camera: TranslateScale) {
        self.view.set_camera(camera);
    }

    /// Where the page shows, in device-independent pixels.
    pub fn placement(&self) -> Camera {
        self.view.placement()
    }

    /// Zooms about the viewport's centre to `zoom` device-independent pixels
    /// a page pixel.
    pub fn zoom_to(&mut self, zoom: f64) {
        self.view.zoom_to(zoom);
    }

    /// Fits the page in the viewport, and keeps it fitted as the viewport
    /// changes until the camera moves.
    pub fn fit(&mut self) -> TranslateScale {
        self.view.fit()
    }

    /// The tool the host picked. A gesture in progress is cancelled.
    pub fn set_tool(&mut self, tool: Tool) {
        self.cancel();
        self.tool = tool;
    }

    pub fn tool(&self) -> Tool {
        self.tool
    }

    pub fn set_brush(&mut self, brush: Brush) {
        self.brush = brush;
    }

    /// The selection the host made, such as the Layers panel's. A gesture
    /// in progress is cancelled.
    pub fn select(&mut self, layers: Vec<LayerId>) {
        self.cancel();
        self.selection.set(layers);
    }

    pub fn selection(&self) -> &[LayerId] {
        self.selection.ids()
    }

    /// Drops a gesture in progress; it writes nothing.
    pub fn cancel(&mut self) {
        self.gesture = None;
    }

    /// The pointer's shape, in CSS's names: the running gesture's, a
    /// selected layer's handle's under it, or the tool's own. `None` hides
    /// it, as the brush tools draw their own ring.
    pub fn cursor(&self) -> Option<CursorIcon> {
        if let Some(active) = &self.gesture {
            return active.gesture.cursor();
        }
        let handle = match (self.tool, self.pointer, self.context()) {
            (Tool::Select, Some(sample), Some(cx)) => {
                tools::select::handle_at(&cx, sample.page).map(|(_, handle)| handle)
            }
            _ => None,
        };
        handle.map_or(self.tool.cursor(), |handle| Some(handle.cursor()))
    }

    /// A pointer event in the window's physical pixels.
    pub fn input(&mut self, event: PointerEvent) -> anyhow::Result<Vec<Effect>> {
        let effects = match event {
            PointerEvent::Down(PointerButtonEvent {
                button,
                pointer,
                state,
            }) => {
                // A touch or pen contact is the primary button, as W3C
                // Pointer Events gives it button 0; ui-events-winit sends
                // none.
                let contact = matches!(pointer.pointer_type, PointerType::Touch | PointerType::Pen)
                    .then_some(PointerButton::Primary);
                let sample = self.view.sample(&state);
                self.down(id(&pointer), button.or(contact), sample)?
            }
            PointerEvent::Move(PointerUpdate {
                pointer,
                current,
                coalesced,
                ..
            }) => {
                // The positions since the last move, oldest first.
                let samples: Vec<Sample> = coalesced
                    .iter()
                    .chain([&current])
                    .map(|state| self.view.sample(state))
                    .collect();
                self.moved(id(&pointer), &samples)
            }
            PointerEvent::Up(PointerButtonEvent { pointer, state, .. }) => {
                let sample = self.view.sample(&state);
                self.up(id(&pointer), sample)?
            }
            PointerEvent::Cancel(_) => {
                self.cancel();
                Vec::new()
            }
            PointerEvent::Enter(_) => Vec::new(),
            PointerEvent::Leave(_) => {
                if self.gesture.is_none() {
                    self.pointer = None;
                    self.selection.set_hover(None);
                }
                Vec::new()
            }
            PointerEvent::Scroll(PointerScrollEvent { delta, state, .. }) => {
                let sample = self.view.sample(&state);
                match self.context() {
                    Some(cx) => tools::pan::wheel(&cx, sample, delta),
                    None => Vec::new(),
                }
            }
            PointerEvent::Gesture(PointerGestureEvent { gesture, state, .. }) => {
                let sample = self.view.sample(&state);
                match (gesture, self.context()) {
                    (PointerGesture::Pinch(delta), Some(cx)) => {
                        tools::pan::pinch(&cx, sample, delta)
                    }
                    _ => Vec::new(),
                }
            }
        };
        Ok(self.apply(effects))
    }

    /// What tools read: `None` before a frame is shown.
    fn context(&self) -> Option<Context<'_>> {
        Some(Context {
            view: &self.view,
            snapshot: self.snapshot.as_ref()?,
            selection: &self.selection,
            tool: self.tool,
            brush: self.brush,
            colors: self.colors,
            session: &self.session,
            gpu: &self.gpu,
        })
    }

    fn down(
        &mut self,
        pointer: u64,
        button: Option<PointerButton>,
        sample: Sample,
    ) -> anyhow::Result<Vec<Effect>> {
        self.pointer = Some(sample);
        match &self.gesture {
            // Its release was lost, as outside the window: it ends here.
            Some(active) if active.pointer == pointer => {
                self.gesture = None;
            }
            // One gesture at a time: a second pointer waits.
            Some(_) => return Ok(Vec::new()),
            None => {}
        }
        let Some(cx) = self.context() else {
            return Ok(Vec::new());
        };
        let press = match (self.tool, button) {
            // The middle button pans whatever the tool, as the pan tool
            // does, and the host's Space held picks it.
            (_, Some(PointerButton::Auxiliary)) | (Tool::Pan, Some(PointerButton::Primary)) => {
                Press::starts(Gesture::Pan(Pan::new(sample, self.view.camera())))
            }
            (Tool::Select, Some(PointerButton::Primary)) => tools::select::press(&cx, sample),
            (Tool::Text, Some(PointerButton::Primary)) => {
                Press::starts(Gesture::Draft(Draft::new(sample)))
            }
            (Tool::ColorPicker, Some(PointerButton::Primary)) => {
                Press::answers(tools::picker::pick(&cx, sample)?)
            }
            (tool, Some(PointerButton::Primary)) if tool.is_brush() => {
                tools::brush::press(&cx, sample)
            }
            _ => Press::default(),
        };
        if let Some(gesture) = press.gesture {
            self.gesture = Some(Active { pointer, gesture });
        }
        Ok(press.effects)
    }

    /// The pointer passed through `samples`, oldest first.
    fn moved(&mut self, pointer: u64, samples: &[Sample]) -> Vec<Effect> {
        let Some(&last) = samples.last() else {
            return Vec::new();
        };
        self.pointer = Some(last);
        match self.gesture.take() {
            Some(mut active) if active.pointer == pointer => {
                let effects = match self.context() {
                    Some(cx) => active.gesture.moved(&cx, samples),
                    None => Vec::new(),
                };
                self.gesture = Some(active);
                effects
            }
            other => {
                let idle = other.is_none();
                self.gesture = other;
                // Only the select tool hovers; another tool's move drops it.
                if idle {
                    let hover = match (self.tool, self.context()) {
                        (Tool::Select, Some(cx)) => tools::select::hover(&cx, last.page),
                        _ => None,
                    };
                    self.selection.set_hover(hover);
                }
                Vec::new()
            }
        }
    }

    fn up(&mut self, pointer: u64, sample: Sample) -> anyhow::Result<Vec<Effect>> {
        if self
            .gesture
            .as_ref()
            .is_none_or(|active| active.pointer != pointer)
        {
            return Ok(Vec::new());
        }
        self.pointer = Some(sample);
        let Some(mut active) = self.gesture.take() else {
            return Ok(Vec::new());
        };
        let Some(cx) = self.context() else {
            return Ok(Vec::new());
        };
        let mut effects = active.gesture.moved(&cx, &[sample]);
        effects.extend(active.gesture.finish(&cx)?);
        Ok(effects)
    }

    /// Applies the effects on the canvas's own state, and passes on what
    /// the host should see: a selection only when it changed, and a camera
    /// as clamped.
    fn apply(&mut self, effects: Vec<Effect>) -> Vec<Effect> {
        effects
            .into_iter()
            .filter_map(|effect| match effect {
                Effect::Selection(ids) => self
                    .selection
                    .set(ids.clone())
                    .then_some(Effect::Selection(ids)),
                Effect::Camera(camera) => {
                    self.view.set_camera(camera);
                    Some(Effect::Camera(self.view.camera()))
                }
                effect => Some(effect),
            })
            .collect()
    }

    /// Redraws if what it would show changed, and returns the texture;
    /// `None` when nothing changed or the viewport is empty.
    pub fn draw(&mut self) -> anyhow::Result<Option<&wgpu::Texture>> {
        let size = self.view.size();
        let (width, height) = (size.width as u32, size.height as u32);
        if width == 0 || height == 0 {
            return Ok(None);
        }
        let look = self.look();
        let sized = self
            .target
            .as_ref()
            .is_some_and(|target| target.size() == (width, height));
        if sized && self.drawn.as_ref() == Some(&look) {
            return Ok(None);
        }
        let (base, strips, overlay) = (self.base(), self.strips(), self.overlay());
        let target = match self.target.take() {
            Some(target) if sized => target,
            _ => Target::new(&self.gpu, width, height),
        };
        let target = self.target.insert(target);
        target.draw(&self.gpu, base, strips, &overlay)?;
        self.drawn = Some(look);
        Ok(Some(&target.texture))
    }

    /// Everything a draw reads.
    fn look(&self) -> Look {
        let gesture = self.gesture.as_ref().map(|active| &active.gesture);
        Look {
            size: self.view.size(),
            translation: self.view.camera().translation,
            zoom: self.view.camera().scale,
            generation: self.generation,
            lifted: gesture.map_or_else(Vec::new, |gesture| gesture.lifted().to_vec()),
            scale: self.view.scale(),
            colors: self.colors,
            tool: self.tool,
            brush: self.brush,
            selection: self.selection.ids().to_vec(),
            hover: self.selection.hover(),
            ring: self
                .pointer
                .filter(|_| self.tool.is_brush())
                .map(|sample| sample.screen),
            gesture: gesture.map_or(GestureLook::None, Gesture::look),
        }
    }

    /// The workspace around the page, and the page's white.
    fn base(&self) -> Scene {
        let mut base = Scene::new();
        base.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            color(self.colors.workspace),
            None,
            &self.view.size().to_rect(),
        );
        base.fill(
            Fill::NonZero,
            self.view.to_screen(),
            Color::WHITE,
            None,
            &self.page_size().to_rect(),
        );
        base
    }

    /// The texture as last drawn.
    pub fn texture(&self) -> Option<&wgpu::Texture> {
        self.target.as_ref().map(|target| &target.texture)
    }

    /// The texture as last drawn, read back.
    pub fn snapshot(&self) -> anyhow::Result<RgbaImage> {
        let target = self.target.as_ref().context("the canvas hasn't drawn")?;
        self.gpu.read_texture(&target.texture)
    }

    /// The overlay: the gesture's preview, then the selection's outlines and
    /// handles, following a drag, the hover, and the brush's ring.
    fn overlay(&self) -> Scene {
        let mut overlay = Overlay::new(&self.view, self.colors);
        let Some(cx) = self.context() else {
            return overlay.finish();
        };
        let gesture = self.gesture.as_ref().map(|active| &active.gesture);
        if let Some(gesture) = gesture {
            gesture.preview(&cx, &mut overlay);
        }
        for &id in self.selection.ids() {
            if let Some(path) = cx.snapshot.outline(id) {
                overlay.outline(path);
            }
        }
        let dragged = gesture.map(Gesture::dragged).unwrap_or_default();
        let selected: Vec<&Control> = cx
            .snapshot
            .controls()
            .iter()
            .filter(|control| self.selection.contains(control.layer))
            .map(|control| {
                dragged
                    .iter()
                    .find(|moved| moved.layer == control.layer && moved.part == control.part)
                    .unwrap_or(control)
            })
            .collect();
        for control in &selected {
            overlay.control(control);
        }
        if let Some(hover) = self.selection.hover()
            && !self.selection.contains(hover)
            && let Some(control) = cx.snapshot.control(hover)
        {
            overlay.control(control);
        }
        if let [control] = selected[..] {
            overlay.handles(control);
        }
        if self.tool.is_brush()
            && let Some(sample) = self.pointer
        {
            overlay.ring(sample.screen, self.brush.diameter);
        }
        overlay.finish()
    }

    fn page_size(&self) -> Size {
        self.snapshot.as_ref().map_or(Size::ZERO, Snapshot::size)
    }

    /// The page's strips for what the viewport shows, each at its origin,
    /// without the layers a gesture lifts.
    fn strips(&self) -> Vec<(Point, Scene)> {
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };
        let frame = self
            .gesture
            .as_ref()
            .and_then(|active| active.gesture.under())
            .unwrap_or(snapshot.frame());
        let rect = self.view.visible().intersect(frame.area());
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return Vec::new();
        }
        // The view starts on a whole viewport pixel, so the renderer's output
        // pixels are the screen's and its tiles meet on them.
        let camera = self.view.camera();
        let corner = (camera * rect.origin()).floor();
        let rect = Rect::from_points(self.view.page_point(corner), (rect.x1, rect.y1));
        let view = koharu_renderer::View {
            rect,
            scale: camera.scale,
        };
        frame
            .draw(view)
            .into_iter()
            .map(|strip| (strip_origin(camera, view, strip.rect), strip.scene))
            .collect()
    }
}

/// A pointer's id, 0 for one that has none.
fn id(pointer: &PointerInfo) -> u64 {
    pointer.pointer_id.map_or(0, |id| id.get_inner().get())
}
