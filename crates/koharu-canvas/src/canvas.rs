//! The canvas: it owns the view, what's shown, the selection, the tool's
//! settings, and the gesture in progress. It routes each pointer event to
//! the tool or the gesture, applies what they answer about its own state,
//! and draws.

use std::{
    cell::RefCell,
    collections::{HashMap, hash_map::Entry},
    sync::Arc,
};

use anyhow::Context as _;
use cursor_icon::CursorIcon;
use image::RgbaImage;
use koharu_renderer::{Drawing, Gpu, Label};
use koharu_scene::{LayerId, Rgba, Session};
use kurbo::{Affine, Point, Rect, Size, TranslateScale, Vec2};
use parking_lot::Mutex;
use ui_events::keyboard::{Code, Key, KeyState, KeyboardEvent, NamedKey};
use ui_events::pointer::{
    PointerButton, PointerButtonEvent, PointerEvent, PointerGesture, PointerGestureEvent,
    PointerInfo, PointerScrollEvent, PointerType, PointerUpdate,
};
use vello_gpu::peniko::Color;

use crate::{
    Brush, Camera, Colors, Effect, Story, Tool,
    control::Control,
    draw::{Target, color, overlay::Overlay},
    gesture::{Context, Gesture, GestureLook, Press},
    selection::Selection,
    snapshot::{Part, Snapshot},
    tools::{self, pan::Pan, text::Draft},
    view::{Sample, View},
};

/// The page viewport over one open project. It reads the project, never
/// writes it: every change leaves as an `Effect::Edit` for the host.
pub struct Canvas {
    gpu: &'static Gpu,
    session: Arc<Mutex<Session>>,
    view: View,
    colors: Colors,
    snapshot: Option<Snapshot>,
    tool: Tool,
    brush: Brush,
    story: Story,
    /// Story's numbers laid out, by number and whether it's a sound
    /// effect's, until the colours change.
    numbers: RefCell<HashMap<(usize, bool), Label>>,
    selection: Selection,
    gesture: Option<Active>,
    /// The pointer's last place, while it's over the canvas.
    pointer: Option<Sample>,
    /// The key whose press the canvas took, until it's released.
    held: Option<Code>,
    /// Bumped by every `show`.
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
    /// A count every `show` bumps, not the frame's revision: a frame of
    /// the same revision draws differently once `[renderer]` changes.
    generation: u64,
    /// The layers a gesture lifts out of the page to draw itself.
    lifted: Vec<LayerId>,
    scale: f64,
    colors: Colors,
    tool: Tool,
    brush: Brush,
    story: Story,
    selection: Vec<LayerId>,
    hover: Option<LayerId>,
    anchors: Vec<usize>,
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
    pub fn new(gpu: &'static Gpu, session: Arc<Mutex<Session>>) -> Self {
        Self {
            gpu,
            session,
            view: View::default(),
            colors: Colors::default(),
            snapshot: None,
            tool: Tool::default(),
            brush: Brush::default(),
            story: Story::default(),
            numbers: RefCell::new(HashMap::new()),
            selection: Selection::default(),
            gesture: None,
            pointer: None,
            held: None,
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

    /// Shows `snapshot`, of the frame the host rendered after a change. One
    /// of another page fits the camera to it.
    pub fn show(&mut self, snapshot: Snapshot) {
        // The project changed under a gesture, by another writer: it would
        // write over that change, so it's dropped, as the editor's was.
        if self
            .snapshot
            .as_ref()
            .is_some_and(|old| old.frame().revision() != snapshot.frame().revision())
        {
            self.gesture = None;
        }
        let other = self
            .snapshot
            .as_ref()
            .is_none_or(|old| old.page().id != snapshot.page().id);
        self.snapshot = Some(snapshot);
        self.generation += 1;
        self.view.set_page(self.page_size(), other);
    }

    pub fn set_colors(&mut self, colors: Colors) {
        self.colors = colors;
        self.numbers.get_mut().clear();
    }

    /// What the Story panel shows, which the canvas numbers the texts for.
    pub fn set_story(&mut self, story: Story) {
        self.story = story;
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

    /// A key the host passes while the canvas holds the keyboard, after a
    /// press on it, as Photoshop's document window takes the keys once
    /// clicked: what it does, or `None` for a key the canvas leaves to the
    /// host. Delete and Backspace remove the direct selection tool's picked
    /// anchors. A key whose press it took is its own until released, its
    /// repeats included, so a held key never reaches the host halfway.
    pub fn key(&mut self, event: &KeyboardEvent) -> Option<Vec<Effect>> {
        if self.held == Some(event.code) {
            if event.state == KeyState::Up {
                self.held = None;
            }
            return Some(Vec::new());
        }
        if event.state != KeyState::Down {
            return None;
        }
        let effects = match event.key {
            Key::Named(NamedKey::Delete | NamedKey::Backspace) => self.delete_anchors()?,
            _ => return None,
        };
        self.held = Some(event.code);
        Some(effects)
    }

    /// Removes the direct selection tool's picked anchors from the selected
    /// text's frame; `None` with another tool or no anchor picked.
    fn delete_anchors(&mut self) -> Option<Vec<Effect>> {
        if self.tool != Tool::DirectSelect || self.selection.anchors().is_empty() {
            return None;
        }
        let effects = tools::direct::delete(&self.context()?);
        self.selection.set_anchors(Vec::new());
        Some(self.apply(effects))
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
            (Tool::DirectSelect, Some(PointerButton::Primary)) => tools::direct::press(&cx, sample),
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
        if let Some(anchors) = press.anchors {
            self.selection.set_anchors(anchors);
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
        let picked = active.gesture.picked();
        effects.extend(active.gesture.finish(&cx)?);
        if let Some(anchors) = picked {
            self.selection.set_anchors(anchors);
        }
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
        let mut drawing = self.gpu.drawing(width as u16, height as u16);
        self.base(&mut drawing);
        self.page(&mut drawing)?;
        self.overlay(&mut drawing)?;
        let target = match self.target.take() {
            Some(target) if sized => target,
            _ => Target::new(self.gpu, width, height),
        };
        let target = self.target.insert(target);
        drawing.render(&target.view)?;
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
            story: self.story,
            selection: self.selection.ids().to_vec(),
            hover: self.selection.hover(),
            anchors: self.selection.anchors().to_vec(),
            ring: self
                .pointer
                .filter(|_| self.tool.is_brush())
                .map(|sample| sample.screen),
            gesture: gesture.map_or(GestureLook::None, Gesture::look),
        }
    }

    /// The workspace around the page, and the page's white.
    fn base(&self, drawing: &mut Drawing) {
        let scene = &mut drawing.scene;
        scene.set_transform(Affine::IDENTITY);
        scene.set_paint(color(self.colors.workspace));
        scene.fill_rect(&self.view.size().to_rect());
        scene.set_transform(self.view.to_screen());
        scene.set_paint(Color::WHITE);
        scene.fill_rect(&self.page_size().to_rect());
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

    /// The overlay: Story's frames and numbers, the gesture's preview, then
    /// the selection's outlines and handles, following a drag, the hover,
    /// and the brush's ring.
    fn overlay(&self, drawing: &mut Drawing) -> anyhow::Result<()> {
        let mut overlay = Overlay::new(drawing, &self.view, self.colors);
        let Some(cx) = self.context() else {
            return Ok(());
        };
        if self.story.shown {
            self.story(&cx, &mut overlay)?;
        }
        // A selected clean-up or mask is seen on its own in place of its
        // bounds: a mask as Photoshop's rubylith, half red over what it
        // hides, and a clean-up with the rest of the page greyed out.
        let part = self
            .selection
            .single()
            .and_then(|id| cx.snapshot.part(id).map(|part| (id, part)));
        if let Some((id, part)) = part {
            let frame = cx.snapshot.frame();
            let (tint, clear) = match part {
                Part::Cleanup => (
                    Color::from_rgba8(128, 128, 128, 153),
                    frame.layer(id).map(|drawn| drawn.cropped()),
                ),
                Part::Mask { cleanup } => (
                    Color::from_rgba8(255, 0, 0, 128),
                    frame.layer(cleanup).and_then(|drawn| drawn.mask()),
                ),
            };
            overlay.isolate(cx.snapshot.size().to_rect(), tint, clear.as_ref())?;
        }
        let isolated = part.map(|(id, _)| id);
        let gesture = self.gesture.as_ref().map(|active| &active.gesture);
        if let Some(gesture) = gesture {
            gesture.preview(&cx, &mut overlay)?;
        }
        for &id in self
            .selection
            .ids()
            .iter()
            .filter(|&&id| Some(id) != isolated)
        {
            if let Some(path) = cx.snapshot.outline(id) {
                overlay.outline(path);
            }
        }
        let dragged = gesture.map(Gesture::dragged).unwrap_or_default();
        let selected: Vec<&Control> = cx
            .snapshot
            .controls()
            .iter()
            .filter(|control| {
                self.selection.contains(control.layer) && Some(control.layer) != isolated
            })
            .map(|control| {
                dragged
                    .iter()
                    .find(|moved| moved.layer == control.layer)
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
        if self.tool == Tool::DirectSelect {
            // The selected text's frame and its anchors in place of its
            // handles; a drag's preview draws them where it puts them.
            if !gesture.is_some_and(Gesture::reshapes)
                && let Some(shape) = self
                    .selection
                    .single()
                    .and_then(|layer| tools::direct::Shape::of(&cx, layer))
            {
                overlay.anchors(&shape.outline(), &shape.anchors(), self.selection.anchors());
            }
        } else if let [control] = selected[..] {
            overlay.handles(control);
        }
        if self.tool.is_brush()
            && let Some(sample) = self.pointer
        {
            overlay.ring(sample.screen, self.brush.diameter);
        }
        Ok(())
    }

    /// Each text's frame and its number in the scene's reading order, under
    /// the selection's handles, so they never take a handle's place.
    fn story(&self, cx: &Context, overlay: &mut Overlay) -> anyhow::Result<()> {
        let mut numbers = self.numbers.borrow_mut();
        for (index, &(layer, sfx)) in cx.snapshot.story().iter().enumerate() {
            let Some(control) = cx.snapshot.control(layer) else {
                continue;
            };
            overlay.story_frame(control, self.story.hovered == Some(layer));
            let label = match numbers.entry((index + 1, sfx)) {
                Entry::Occupied(laid) => laid.into_mut(),
                Entry::Vacant(place) => {
                    // White on a filled pill, the theme's colour on an
                    // outlined one.
                    let fill = match sfx {
                        true => self.colors.selection,
                        false => Rgba([1.0, 1.0, 1.0, 1.0]),
                    };
                    match Label::new(&(index + 1).to_string(), 9.0, 600, fill)? {
                        Some(label) => place.insert(label),
                        None => continue,
                    }
                }
            };
            overlay.story_number(control.corners()[0], label, sfx)?;
        }
        Ok(())
    }

    fn page_size(&self) -> Size {
        self.snapshot.as_ref().map_or(Size::ZERO, Snapshot::size)
    }

    /// The page as the viewport shows it, without the layers a gesture
    /// lifts.
    fn page(&self, drawing: &mut Drawing) -> anyhow::Result<()> {
        let Some(snapshot) = &self.snapshot else {
            return Ok(());
        };
        let frame = self
            .gesture
            .as_ref()
            .and_then(|active| active.gesture.under())
            .unwrap_or(snapshot.frame());
        frame.paint(self.view.to_screen(), self.view.size().to_rect(), drawing)
    }
}

/// A pointer's id, 0 for one that has none.
fn id(pointer: &PointerInfo) -> u64 {
    pointer.pointer_id.map_or(0, |id| id.get_inner().get())
}
