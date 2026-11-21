//! The canvas: it owns the view, what's shown, the selection, the tool's
//! settings, and the gesture in progress. It routes each pointer event to
//! the tool or the gesture, applies what they answer about its own state,
//! and draws.

use std::{
    cell::RefCell,
    collections::{HashMap, hash_map::Entry},
    sync::Arc,
    time::Instant,
};

use anyhow::Context as _;
use cursor_icon::CursorIcon;
use image::RgbaImage;
use koharu_renderer::{Drawing, Gpu, Label, Renderer};
use koharu_scene::{CharStyle, Content, Layer, LayerId, PageId, Rgba, Session, Text};
use kurbo::{Affine, Point, Rect, Shape as _, Size, TranslateScale, Vec2};
use parking_lot::Mutex;
use ui_events::keyboard::{Code, Key, KeyState, KeyboardEvent, NamedKey};
use ui_events::pointer::{
    PointerButton, PointerButtonEvent, PointerEvent, PointerGesture, PointerGestureEvent,
    PointerInfo, PointerScrollEvent, PointerType, PointerUpdate,
};
use vello_gpu::peniko::Color;

use crate::{
    Brush, Camera, Colors, Effect, Story, Tool,
    area::Area,
    control::{Control, REACH},
    draw::{Target, color, overlay::Overlay},
    gesture::{Context, Gesture, GestureLook, Press},
    selection::Selection,
    snapshot::{Part, Snapshot},
    tools::{self, pan::Pan, text::Draft},
    typing::{Keyed, Typed, Typing},
    view::{Sample, View},
};

/// How near a press, in device-independent pixels, still lands on the text
/// being typed.
const TYPING_SLACK: f64 = 4.0;

/// The page viewport over one open project. It reads the project, never
/// writes it: every change leaves as an `Effect::Edit` for the host.
pub struct Canvas {
    session: Arc<Mutex<Session>>,
    /// Lays out the text being typed, as the host's renders lay it out.
    renderer: Renderer,
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
    /// What the marquee and the lasso marked of the page's pixels, as
    /// Photoshop's selection; another page drops it.
    area: Option<Area>,
    /// Bumped whenever the area changes.
    marked: u64,
    gesture: Option<Active>,
    /// The text being typed into, as Photoshop's Type tool types.
    typing: Option<Typing>,
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
    /// A count every change of the area bumps.
    area: u64,
    /// Where the brush's ring is, in the viewport.
    ring: Option<Point>,
    gesture: GestureLook,
    /// The typing session's changes, and whether its caret shows.
    typing: Option<(u64, bool)>,
}

/// A gesture, and the pointer that owns it.
struct Active {
    pointer: u64,
    gesture: Gesture,
}

impl Canvas {
    pub fn new(session: Arc<Mutex<Session>>, renderer: Renderer) -> Self {
        Self {
            session,
            renderer,
            view: View::default(),
            colors: Colors::default(),
            snapshot: None,
            tool: Tool::default(),
            brush: Brush::default(),
            story: Story::default(),
            numbers: RefCell::new(HashMap::new()),
            selection: Selection::default(),
            area: None,
            marked: 0,
            gesture: None,
            typing: None,
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
        // Typing keeps what's typed across another change, such as the type
        // bar's paragraph settings, with the text's frame, place, and style
        // as that change left them; it stops once its text is gone.
        let typed = self.typing.as_ref().map(Typing::target);
        match typed {
            Some(_) if other => self.typing = None,
            Some(Some(layer)) => match typable(&snapshot.page().layers, layer) {
                Some((text, _)) => {
                    if let Some(typing) = &mut self.typing
                        && typing.rebase(&text).is_err()
                    {
                        self.typing = None;
                    }
                }
                None => self.typing = None,
            },
            _ => {}
        }
        if other {
            self.deselect();
        }
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

    /// The tool the host picked. A gesture in progress is cancelled, and
    /// typing is committed, as Photoshop commits type on another tool;
    /// returns what that commit does.
    pub fn set_tool(&mut self, tool: Tool) -> Vec<Effect> {
        self.cancel();
        self.tool = tool;
        self.finish_typing()
    }

    pub fn tool(&self) -> Tool {
        self.tool
    }

    pub fn set_brush(&mut self, brush: Brush) {
        self.brush = brush;
    }

    /// The selection the host made, such as the Layers panel's. A gesture
    /// in progress is cancelled, and typing into another layer is committed;
    /// returns what that commit does.
    pub fn select(&mut self, layers: Vec<LayerId>) -> Vec<Effect> {
        self.cancel();
        let typed = self.typing.as_ref().map(Typing::target);
        let effects = match typed {
            Some(Some(layer)) if layers == [layer] => Vec::new(),
            Some(_) => self.finish_typing(),
            None => Vec::new(),
        };
        self.selection.set(layers);
        effects
    }

    pub fn selection(&self) -> &[LayerId] {
        self.selection.ids()
    }

    /// Where the selection lies on the page, in page pixels, which the host
    /// places its own controls beside: its layers as they're laid out, a
    /// point text's lines included, and the handles drawn on them as far as
    /// a pointer reaches them, so the host's controls keep off those; `None`
    /// while nothing selected shows. While typing, the text as typed.
    pub fn selection_bounds(&self) -> Option<Rect> {
        if let Some(typing) = &self.typing {
            return Some(
                typing
                    .transform()
                    .transform_rect_bbox(typing.lettering().bounds()),
            );
        }
        let reach = self.view.page_length(REACH);
        self.snapshot
            .as_ref()?
            .controls()
            .iter()
            .filter(|control| self.selection.contains(control.layer))
            .map(|control| {
                control
                    .handles()
                    .into_iter()
                    .map(|handle| control.handle_point(handle, &self.view))
                    .fold(control.outline().bounding_box(), |bounds, point| {
                        bounds.union(Rect::from_center_size(point, (2.0 * reach, 2.0 * reach)))
                    })
            })
            .reduce(|all, bounds| all.union(bounds))
    }

    /// Where the area the marquee and the lasso marked lies on the page, in
    /// page pixels, which the host places its fill's controls beside; `None`
    /// while there's none.
    pub fn area_bounds(&self) -> Option<Rect> {
        self.area.as_ref().map(Area::bounds)
    }

    /// The area for the host to fill, as Photoshop's Generative Fill fills a
    /// selection: its page, its bounds, and its coverage over them, a byte a
    /// pixel. The area stays, to fill again.
    pub fn area(&self) -> Option<(PageId, koharu_scene::Rect, Vec<u8>)> {
        let (rect, mask) = self.area.as_ref()?.cropped();
        Some((self.snapshot.as_ref()?.page().id, rect, mask))
    }

    /// Drops the area, as Photoshop's Deselect does.
    pub fn deselect(&mut self) {
        if self.area.take().is_some() {
            self.marked += 1;
        }
    }

    pub fn is_typing(&self) -> bool {
        self.typing.is_some()
    }

    /// The text being typed into, for the host's type bar; `None` when
    /// nothing is.
    pub fn typing(&self) -> Option<Typed> {
        self.typing.as_ref().map(Typing::typed)
    }

    /// Styles what's selected while typing with `patch`'s values, or holds
    /// them for what's typed next at a caret.
    pub fn style_typing(&mut self, patch: &CharStyle) -> anyhow::Result<()> {
        match &mut self.typing {
            Some(typing) => typing.style(patch),
            None => Ok(()),
        }
    }

    /// The input method's composition while typing, `selected` being the
    /// part it has selected; empty when it stops composing.
    pub fn compose(&mut self, text: &str, selected: Option<(usize, usize)>) -> anyhow::Result<()> {
        let selected = selected.map(|(start, end)| start..end);
        match &mut self.typing {
            Some(typing) => typing.compose(text, selected),
            None => Ok(()),
        }
    }

    /// Types `text` while typing, as the input method commits it or a paste
    /// does.
    pub fn insert(&mut self, text: &str) -> anyhow::Result<()> {
        match &mut self.typing {
            Some(typing) => typing.insert(text),
            None => Ok(()),
        }
    }

    /// Where the caret is while typing, in the window's physical pixels,
    /// for the input method's candidate window.
    pub fn caret_area(&self) -> Option<Rect> {
        let typing = self.typing.as_ref()?;
        let to_window = Affine::translate(self.view.viewport().origin().to_vec2())
            * self.view.to_screen()
            * typing.transform();
        let caret = typing.caret();
        Some(Rect::from_points(
            to_window * caret.p0,
            to_window * caret.p1,
        ))
    }

    /// Commits typing, as leaving the text does: the step that keeps it,
    /// none when nothing changed.
    pub fn finish_typing(&mut self) -> Vec<Effect> {
        let effects = self.commit_typing();
        self.apply(effects)
    }

    /// Typing's commit, unapplied, for a press to answer with what else it
    /// does.
    fn commit_typing(&mut self) -> Vec<Effect> {
        let (Some(typing), Some(snapshot)) = (self.typing.take(), &self.snapshot) else {
            return Vec::new();
        };
        typing.commit(snapshot.page())
    }

    /// A key the host passes while the canvas holds the keyboard, after a
    /// press on it, as Photoshop's document window takes the keys once
    /// clicked: what it does, or `None` for a key the canvas leaves to the
    /// host. While typing, the text takes its keys, repeats included.
    /// Otherwise Delete and Backspace remove the direct selection tool's
    /// picked anchors, and a key whose press it took is its own until
    /// released, its repeats included, so a held key never reaches the host
    /// halfway.
    pub fn key(&mut self, event: &KeyboardEvent) -> anyhow::Result<Option<Vec<Effect>>> {
        if let Some(typing) = &mut self.typing {
            return Ok(match typing.key(event)? {
                Keyed::Taken(effects) => Some(effects),
                Keyed::Commit => Some(self.finish_typing()),
                Keyed::Cancel => {
                    self.typing = None;
                    Some(Vec::new())
                }
                Keyed::Pass => None,
            });
        }
        if self.held == Some(event.code) {
            if event.state == KeyState::Up {
                self.held = None;
            }
            return Ok(Some(Vec::new()));
        }
        if event.state != KeyState::Down {
            return Ok(None);
        }
        let effects = match event.key {
            Key::Named(NamedKey::Delete | NamedKey::Backspace) => self.delete_anchors(),
            _ => None,
        };
        if effects.is_some() {
            self.held = Some(event.code);
        }
        Ok(effects)
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
        if let (Some(typing), Some(sample)) = (&self.typing, self.pointer)
            && (typing.dragging.is_some()
                || typing.covers(sample.page, self.view.page_length(TYPING_SLACK)))
        {
            return Some(CursorIcon::Text);
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
        let primary = button == Some(PointerButton::Primary);
        let mut effects = Vec::new();
        if primary && let Some(typing) = &mut self.typing {
            // A press on the text puts its caret; one away commits it and
            // goes on as a press, as Photoshop's Type tool does.
            if typing.covers(sample.page, self.view.page_length(TYPING_SLACK)) {
                typing.press(sample.page, sample.modifiers.shift(), sample.clicks);
                typing.dragging = Some(pointer);
                return Ok(Vec::new());
            }
            effects = self.commit_typing();
        }
        let Some(cx) = self.context() else {
            return Ok(effects);
        };
        // The Type tool types into the text it's pressed on, as a double
        // click with the select tool does.
        let types = self.tool == Tool::Text || (self.tool == Tool::Select && sample.clicks >= 2);
        if let Some((layer, text, untranslated)) = (primary && types)
            .then(|| text_at(&cx, sample.page))
            .flatten()
        {
            let mut typing = Typing::layer(&self.renderer, layer, &text, untranslated)?;
            typing.press(sample.page, false, 1);
            typing.dragging = Some(pointer);
            self.typing = Some(typing);
            effects.push(Effect::Selection(vec![layer]));
            return Ok(effects);
        }
        let press = match (self.tool, button) {
            // The middle button pans whatever the tool, as the pan tool
            // does, and the host's Space held picks it.
            (_, Some(PointerButton::Auxiliary)) | (Tool::Pan, Some(PointerButton::Primary)) => {
                Press::starts(Gesture::Pan(Pan::new(sample, self.view.camera())))
            }
            (Tool::Select, Some(PointerButton::Primary)) => tools::select::press(&cx, sample),
            (Tool::DirectSelect, Some(PointerButton::Primary)) => tools::direct::press(&cx, sample),
            (Tool::Marquee | Tool::Lasso, Some(PointerButton::Primary)) => {
                tools::mark::press(&cx, sample)
            }
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
        effects.extend(press.effects);
        Ok(effects)
    }

    /// The pointer passed through `samples`, oldest first.
    fn moved(&mut self, pointer: u64, samples: &[Sample]) -> Vec<Effect> {
        let Some(&last) = samples.last() else {
            return Vec::new();
        };
        self.pointer = Some(last);
        if let Some(typing) = &mut self.typing
            && typing.dragging == Some(pointer)
        {
            typing.drag(last.page);
            return Vec::new();
        }
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
        if let Some(typing) = &mut self.typing
            && typing.dragging == Some(pointer)
        {
            typing.dragging = None;
            return Ok(Vec::new());
        }
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
        // A text drafted with the Type tool is typed into at once, with
        // nothing selected; committing adds it, selected.
        if let Gesture::Draft(draft) = &active.gesture {
            let text = draft.text(&cx);
            self.typing = Some(Typing::new_text(&self.renderer, text)?);
            effects.push(Effect::Selection(Vec::new()));
            return Ok(effects);
        }
        if let Gesture::Mark(mark) = &active.gesture {
            let (shape, size) = (mark.shape(), cx.snapshot.size());
            self.area = Area::marked(self.area.take(), &shape, mark.mode, size);
            self.marked += 1;
            return Ok(effects);
        }
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

    /// Redraws if what it would show changed, or the GPU it drew on was
    /// lost, and returns the texture; `None` when nothing changed or the
    /// viewport is empty.
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
            .is_some_and(|target| target.size() == (width, height) && !target.gpu.lost());
        if sized && self.drawn.as_ref() == Some(&look) {
            return Ok(None);
        }
        let gpu = Gpu::get()?;
        let mut drawing = gpu.drawing(width as u16, height as u16);
        self.base(&mut drawing);
        self.page(&mut drawing)?;
        self.overlay(&mut drawing)?;
        let target = match self.target.take() {
            Some(target) if sized => target,
            _ => Target::new(&gpu, width, height)?,
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
            area: self.marked,
            ring: self
                .pointer
                .filter(|_| self.tool.is_brush())
                .map(|sample| sample.screen),
            gesture: gesture.map_or(GestureLook::None, Gesture::look),
            typing: self
                .typing
                .as_ref()
                .map(|typing| (typing.version(), typing.caret_shows(Instant::now()))),
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
        target.gpu.read_texture(&target.texture)
    }

    /// The overlay: Story's frames and numbers, the gesture's preview, then
    /// the selection's outlines and handles, following a drag, the hover,
    /// and the brush's ring.
    fn overlay(&self, drawing: &mut Drawing) -> anyhow::Result<()> {
        let mut overlay = Overlay::new(drawing, &self.view, self.colors);
        let Some(cx) = self.context() else {
            return Ok(());
        };
        let gesture = self.gesture.as_ref().map(|active| &active.gesture);
        let dragged = gesture.map(Gesture::dragged).unwrap_or_default();
        if self.story.shown {
            for (layer, control) in self.story_controls(&cx, &dragged) {
                overlay.story_frame(control, self.story.hovered == Some(layer));
            }
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
        // The text typed into shows as typed, not as the page last drew it.
        let typed = self.typing.as_ref().and_then(Typing::target);
        if let Some(area) = &self.area {
            overlay.ants(area.edge());
        }
        if let Some(gesture) = gesture {
            gesture.preview(&cx, &mut overlay)?;
        }
        for &id in self
            .selection
            .ids()
            .iter()
            .filter(|&&id| Some(id) != isolated && Some(id) != typed)
        {
            if let Some(path) = cx.snapshot.outline(id) {
                overlay.outline(path);
            }
        }
        let selected: Vec<&Control> = cx
            .snapshot
            .controls()
            .iter()
            .filter(|control| {
                self.selection.contains(control.layer)
                    && Some(control.layer) != isolated
                    && Some(control.layer) != typed
            })
            .map(|control| {
                dragged
                    .iter()
                    .find(|moved| moved.layer == control.layer)
                    .unwrap_or(control)
            })
            .collect();
        let hovered = self
            .selection
            .hover()
            .filter(|&hover| !self.selection.contains(hover))
            .and_then(|hover| cx.snapshot.control(hover));
        for control in selected.iter().copied().chain(hovered) {
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
        // Story's numbers over the selection's handles, so none hides behind
        // one; each carries the overflow mark its corner would, which shows
        // on the outlined frames' corners when they don't.
        if self.story.shown {
            self.story_numbers(&cx, &dragged, &mut overlay)?;
        } else {
            for control in selected.iter().copied().chain(hovered) {
                overlay.overflow(control);
            }
        }
        if let Some(typing) = &self.typing {
            let transform = typing.transform();
            let bounds = transform * typing.lettering().bounds().to_path(0.1);
            overlay.outline(&bounds);
            let to_screen = self.view.to_screen() * transform;
            overlay.composing(&typing.composing(), to_screen, typing.vertical());
            if typing.caret_shows(Instant::now()) {
                overlay.caret(typing.caret(), to_screen);
            }
        }
        if self.tool.is_brush()
            && let Some(sample) = self.pointer
        {
            overlay.ring(sample.screen, self.brush.diameter);
        }
        Ok(())
    }

    /// Each text in the scene's reading order, with its control: where a
    /// drag puts it while it's dragged, as its handles go.
    fn story_controls<'a>(
        &self,
        cx: &Context<'a>,
        dragged: &'a [Control],
    ) -> Vec<(LayerId, &'a Control)> {
        let snapshot = cx.snapshot;
        snapshot
            .story()
            .iter()
            .filter_map(|&(layer, _)| {
                let control = dragged.iter().find(|moved| moved.layer == layer);
                Some((layer, control.or_else(|| snapshot.control(layer))?))
            })
            .collect()
    }

    /// Each text's number in the scene's reading order, on its frame's
    /// top-left corner.
    fn story_numbers(
        &self,
        cx: &Context,
        dragged: &[Control],
        overlay: &mut Overlay,
    ) -> anyhow::Result<()> {
        let mut numbers = self.numbers.borrow_mut();
        for (index, &(layer, sfx)) in cx.snapshot.story().iter().enumerate() {
            let control = dragged.iter().find(|moved| moved.layer == layer);
            let Some(control) = control.or_else(|| cx.snapshot.control(layer)) else {
                continue;
            };
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
            overlay.story_number(control, label, sfx)?;
        }
        Ok(())
    }

    fn page_size(&self) -> Size {
        self.snapshot.as_ref().map_or(Size::ZERO, Snapshot::size)
    }

    /// The page as the viewport shows it, without the layers a gesture
    /// lifts; while typing, without the text typed into, which draws as
    /// typed over its selection's highlight.
    fn page(&self, drawing: &mut Drawing) -> anyhow::Result<()> {
        let Some(snapshot) = &self.snapshot else {
            return Ok(());
        };
        let typed = self
            .typing
            .as_ref()
            .and_then(Typing::target)
            .map(|layer| snapshot.frame().without(&[layer]));
        let frame = self
            .gesture
            .as_ref()
            .and_then(|active| active.gesture.under())
            .or(typed.as_ref())
            .unwrap_or(snapshot.frame());
        frame.paint(self.view.to_screen(), self.view.size().to_rect(), drawing)?;
        if let Some(typing) = &self.typing {
            let transform = self.view.to_screen() * typing.transform();
            Overlay::new(drawing, &self.view, self.colors)
                .highlight(&typing.highlight(), transform);
            typing.lettering().draw(drawing, transform)?;
        }
        Ok(())
    }
}

/// A pointer's id, 0 for one that has none.
fn id(pointer: &PointerInfo) -> u64 {
    pointer.pointer_id.map_or(0, |id| id.get_inner().get())
}

/// The text a press at `page` types into: the topmost text or block it
/// picks, with what `typable` gives for it.
fn text_at(cx: &Context, page: Point) -> Option<(LayerId, Text, bool)> {
    let layer = cx.snapshot.hit(page)?.layer;
    let (text, untranslated) = typable(&cx.snapshot.page().layers, layer)?;
    Some((layer, text, untranslated))
}

/// `layer`'s text as typing takes it: a text layer's, or a block's
/// translation, and its source, standing in, when it has none, which the
/// flag says.
fn typable(layers: &[Layer], layer: LayerId) -> Option<(Text, bool)> {
    layers.iter().find_map(|found| match &found.content {
        _ if found.id != layer => typable(&found.children, layer),
        Content::Text(text) => Some(((**text).clone(), false)),
        Content::Block(block) => Some(match &block.translation {
            Some(translation) => (translation.clone(), false),
            None => (block.source.clone(), true),
        }),
        _ => None,
    })
}
