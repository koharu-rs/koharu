//! The main window: one webview rendering off-screen in CEF over the native
//! canvas, composited with wgpu on the window's own surface, and the
//! window's input routed to one or the other.
//!
//! The main thread owns all of it. CEF's UI thread is the event loop's: the
//! runtime drives CEF through its external message pump, so the render
//! handler, the cursor handler, and `on_window_event` all run there, and
//! other threads reach the window through `Screen`, whose changes run there
//! in the order asked. CEF may call the render handler back from inside a
//! call into it, so nothing borrows the window across a call into CEF.

mod compose;
mod input;
mod keys;
mod pace;
mod paint;
mod refresh;
mod shared;

use std::{
    cell::RefCell,
    io::Cursor,
    sync::{Arc, OnceLock},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result};
use image::{DynamicImage, RgbaImage, codecs::jpeg::JpegEncoder, imageops::FilterType};
use koharu_canvas::{
    Brush, Camera, Canvas, Colors, Effect, Facing, Rect, Snapshot, Story, Tool, Typed, Vec2,
};
use koharu_renderer::Gpu;
use koharu_scene::{Bounds, CharStyle, LayerId, PageId};
use tauri::{
    AppHandle, LogicalPosition, Manager as _, WebviewUrl,
    webview::{PageLoadEvent, WebviewBuilder},
};
use tauri_runtime_cef::{
    AsCefWebviewAttributes as _, CefRuntime, Offscreen, OffscreenView, WebviewBuilderCefExt as _,
    cef::{
        AcceleratedPaintInfo, Browser, BrowserHost, CefString, CursorType, ImplBrowser as _,
        ImplBrowserHost as _, ImplFrame as _, PaintElementType, sys::cef_cursor_type_t,
    },
};
use tokio::sync::oneshot::{self, error::TryRecvError};
use winit::{
    cursor::CursorIcon,
    dpi::{LogicalSize, PhysicalPosition, PhysicalSize},
    event::{Ime, WindowEvent},
    monitor::MonitorHandle,
    window::{ImeCapabilities, ImeEnableRequest, ImeRequest, ImeRequestData, Window},
};

use self::{compose::Compositor, input::Input, pace::Pacer, paint::Painted, shared::Importer};
pub use self::{input::Pointer, shared::sharing};
use crate::{app::App, events};

/// The window's label in `tauri.conf.json`, and its webview's.
pub const MAIN: &str = "main";

thread_local! {
    /// The main window, on the main thread, from `open` until it's
    /// destroyed.
    static WINDOW: RefCell<Option<Main>> = const { RefCell::new(None) };
}

/// Runs `f` on the main window, if this is the main thread and it's open.
fn with<T>(f: impl FnOnce(&mut Main) -> T) -> Option<T> {
    WINDOW.with_borrow_mut(|main| main.as_mut().map(f))
}

/// Runs `change` on the main window, draws it again, and tells the
/// interface where the page and the selection show if either moved, once
/// the borrow is over.
fn change(change: impl FnOnce(&mut Main)) {
    let moved = with(|main| {
        change(main);
        main.request_redraw();
        main.follow_typing();
        (
            main.handle.clone(),
            main.moved(),
            main.selection_moved(),
            main.area_moved(),
            main.typing_moved(),
        )
    });
    let Some((handle, camera, selection, area, typing)) = moved else {
        return;
    };
    if let Some(camera) = camera {
        events::CAMERA.tell(&handle, camera);
    }
    if let Some(bounds) = selection {
        events::SELECTION_BOUNDS.tell(&handle, bounds);
    }
    if let Some(bounds) = area {
        events::AREA.tell(&handle, bounds);
    }
    if let Some(typed) = typing {
        events::TYPING.tell(&handle, typed);
    }
}

/// The main window as other threads reach it: each change runs on the main
/// thread, in the order asked, and draws again. Before the window opens, as
/// in tests, which open none, a change goes nowhere.
#[derive(Clone, Default)]
pub struct Screen(Arc<OnceLock<AppHandle<CefRuntime>>>);

impl Screen {
    fn change(&self, change: impl FnOnce(&mut Main) + Send + 'static) {
        let Some(handle) = self.0.get() else {
            return;
        };
        if let Err(error) = handle.run_on_main_thread(move || self::change(change)) {
            tracing::warn!("failed to change the window: {error}");
        }
    }

    /// The open project's canvas, set up as the interface asked, or none.
    pub fn set_canvas(&self, canvas: Option<Canvas>) {
        self.change(move |main| main.set_canvas(canvas));
    }

    /// What the canvas shows, after a change, with the page facing it in a
    /// two-page spread.
    pub fn show(&self, snapshot: Snapshot, facing: Option<Facing>) {
        self.change(move |main| {
            if let Some(canvas) = &mut main.canvas {
                canvas.show(snapshot);
                canvas.face(facing);
            }
        });
    }

    /// Moves the canvas to the element's rectangle, in CSS pixels.
    pub fn set_viewport(&self, viewport: Rect) {
        self.change(move |main| {
            main.viewport = viewport;
            main.place();
        });
    }

    /// Like `change`, for a change that answers canvas effects, which `app`
    /// carries out once the window is free again.
    fn answer(&self, app: Arc<App>, act: impl FnOnce(&mut Main) -> Vec<Effect> + Send + 'static) {
        let Some(handle) = self.0.get() else {
            return;
        };
        let run = move || {
            let mut effects = Vec::new();
            change(|main| effects = act(main));
            input::carry(&app, effects);
        };
        if let Err(error) = handle.run_on_main_thread(run) {
            tracing::warn!("failed to change the window: {error}");
        }
    }

    /// The interface's tool; another commits typing.
    pub fn set_tool(&self, app: Arc<App>, tool: Tool) {
        self.answer(app, move |main| {
            main.settings.tool = tool;
            main.canvas
                .as_mut()
                .map_or_else(Vec::new, |canvas| canvas.set_tool(tool))
        });
    }

    /// Commits typing, as leaving its page does.
    pub fn finish_typing(&self, app: Arc<App>) {
        self.answer(app, |main| {
            main.canvas
                .as_mut()
                .map_or_else(Vec::new, Canvas::finish_typing)
        });
    }

    /// Styles what's being typed, as the type bar asks: the selected
    /// characters, or the next ones typed at a caret.
    pub fn style_typing(&self, patch: CharStyle) {
        self.change(move |main| {
            if let Some(canvas) = &mut main.canvas
                && let Err(error) = canvas.style_typing(&patch)
            {
                tracing::error!("the canvas failed to style its typing: {error:#}");
            }
        });
    }

    /// Types `text` into what's being typed, as a paste does.
    pub fn insert(&self, text: String) {
        self.change(move |main| {
            if let Some(canvas) = &mut main.canvas
                && let Err(error) = canvas.insert(&text)
            {
                tracing::error!("the canvas failed to paste: {error:#}");
            }
        });
    }

    pub fn set_brush(&self, brush: Brush) {
        self.change(move |main| {
            main.settings.brush = brush;
            if let Some(canvas) = &mut main.canvas {
                canvas.set_brush(brush);
            }
        });
    }

    pub fn set_colors(&self, colors: Colors) {
        self.change(move |main| {
            main.settings.colors = colors;
            if let Some(canvas) = &mut main.canvas {
                canvas.set_colors(colors);
            }
        });
    }

    pub fn set_story(&self, story: Story) {
        self.change(move |main| {
            main.settings.story = story;
            if let Some(canvas) = &mut main.canvas {
                canvas.set_story(story);
            }
        });
    }

    /// Fits the page in the canvas.
    pub fn fit(&self) {
        self.change(|main| {
            if let Some(canvas) = &mut main.canvas {
                canvas.fit();
            }
        });
    }

    /// Zooms about the canvas's centre, `zoom` CSS pixels to a page pixel.
    pub fn zoom_to(&self, zoom: f64) {
        self.change(move |main| {
            if let Some(canvas) = &mut main.canvas {
                canvas.zoom_to(zoom);
            }
        });
    }

    /// The interface's selection; another layer commits typing.
    pub fn select(&self, app: Arc<App>, layers: Vec<LayerId>) {
        self.answer(app, move |main| {
            main.canvas
                .as_mut()
                .map_or_else(Vec::new, |canvas| canvas.select(layers))
        });
    }

    /// An arrow key's move of the selection.
    pub fn nudge(&self, app: Arc<App>, by: Vec2) {
        self.answer(app, move |main| {
            main.canvas
                .as_mut()
                .map_or_else(Vec::new, |canvas| canvas.nudge(by))
        });
    }

    /// The canvas's area, for a fill: its page, its bounds, and its coverage.
    pub fn area(&self) -> oneshot::Receiver<Option<(PageId, koharu_scene::Rect, Vec<u8>)>> {
        let (sender, receiver) = oneshot::channel();
        self.change(move |main| {
            // The command waiting for it may be gone.
            let _ = sender.send(main.canvas.as_ref().and_then(Canvas::area));
        });
        receiver
    }

    /// Drops the canvas's area.
    pub fn deselect(&self) {
        self.change(|main| {
            if let Some(canvas) = &mut main.canvas {
                canvas.deselect();
            }
        });
    }

    /// The window as its next composited frame shows it: the canvas and
    /// CEF's paint together.
    pub fn capture(&self) -> oneshot::Receiver<RgbaImage> {
        let (sender, receiver) = oneshot::channel();
        self.change(move |main| main.captures.push(sender));
        receiver
    }

    /// The window as its next frame shows it, its longer side at most 1600
    /// pixels and as JPEG at quality 80, for an error event. The main
    /// thread draws that frame, so there it's `None`, as when no frame
    /// comes within a second.
    pub fn screenshot(&self) -> Option<Vec<u8>> {
        if thread::current().name() == Some("main") {
            return None;
        }
        let mut receiver = self.capture();
        let deadline = Instant::now() + Duration::from_secs(1);
        let image = loop {
            match receiver.try_recv() {
                Ok(image) => break DynamicImage::ImageRgba8(image),
                Err(TryRecvError::Empty) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return None,
            }
        };
        let image = if image.width().max(image.height()) > 1600 {
            image.resize(1600, 1600, FilterType::Triangle)
        } else {
            image
        };
        let mut jpeg = Cursor::new(Vec::new());
        JpegEncoder::new_with_quality(&mut jpeg, 80)
            .encode_image(&image.to_rgb8())
            .ok()?;
        Some(jpeg.into_inner())
    }
}

/// The main window as the main thread holds it.
struct Main {
    /// The GPU the surface and CEF's frames are on.
    gpu: Arc<Gpu>,
    /// Tells the interface where the page shows.
    handle: AppHandle<CefRuntime>,
    /// The webview's layout, which the runtime keeps current; set when CEF
    /// creates the browser.
    view: Option<OffscreenView>,
    painted: Painted,
    /// Captures waiting for the next composited frame.
    captures: Vec<oneshot::Sender<RgbaImage>>,
    /// The open project's canvas.
    canvas: Option<Canvas>,
    /// The canvas element's rectangle, in CSS pixels.
    viewport: Rect,
    /// What the interface set, for a canvas made later.
    settings: Settings,
    /// The camera as the interface last heard it.
    camera: Option<Camera>,
    /// Where the selection lies on the page, as the interface last heard it.
    selection: Option<Bounds>,
    /// Where the canvas's area lies on the page, as the interface last heard
    /// it.
    area: Option<Bounds>,
    /// What's being typed, as the interface last heard it.
    typed: Option<Typed>,
    /// The typing caret where the input method's candidate window was last
    /// placed, in the window's physical pixels.
    caret: Option<Rect>,
    /// Redraws the canvas while it types, for its caret's blink.
    blink: Option<tauri::async_runtime::JoinHandle<()>>,
    /// The browser CEF first paints for, which input reaches through its
    /// host.
    browser: Option<Browser>,
    input: Input,
    /// CEF's cursor, put back when the pointer leaves the canvas; `None`
    /// hides it.
    web_cursor: Option<CursorIcon>,
    compositor: Option<Compositor>,
    /// Whether the input method composes for the page yet.
    ime: bool,
    /// Takes CEF's frames and paces them, where it shares them as GPU
    /// textures.
    shared: Option<Shared>,
    /// The monitor the window was last on.
    monitor: Option<MonitorHandle>,
    /// The frames a second CEF renders the page at, and the pacer paces
    /// by: that monitor's refresh rate.
    frame_rate: i32,
}

struct Shared {
    importer: Importer,
    pacer: Pacer,
}

impl Shared {
    fn new(gpu: &Gpu) -> Self {
        Self {
            importer: Importer::new(gpu),
            pacer: Pacer::new(),
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Settings {
    tool: Tool,
    brush: Brush,
    colors: Colors,
    story: Story,
}

impl Main {
    fn request_redraw(&self) {
        if let Some(view) = &self.view {
            view.request_redraw();
        }
    }

    fn window(&self) -> Option<Arc<dyn Window>> {
        self.view.as_ref().and_then(OffscreenView::window)
    }

    /// Physical pixels to a device-independent one.
    fn scale(&self) -> f64 {
        self.window().map_or(1.0, |window| window.scale_factor())
    }

    /// The canvas's place in the window, in whole physical pixels.
    fn physical_viewport(&self) -> Rect {
        let (scale, viewport) = (self.scale(), self.viewport);
        Rect::new(
            (viewport.x0 * scale).round(),
            (viewport.y0 * scale).round(),
            (viewport.x1 * scale).round(),
            (viewport.y1 * scale).round(),
        )
    }

    fn set_canvas(&mut self, canvas: Option<Canvas>) {
        let (viewport, scale, settings) = (self.physical_viewport(), self.scale(), self.settings);
        self.canvas = canvas.map(|mut canvas| {
            canvas.set_viewport(viewport, scale);
            canvas.set_tool(settings.tool);
            canvas.set_brush(settings.brush);
            canvas.set_colors(settings.colors);
            canvas.set_story(settings.story);
            canvas
        });
        if self.canvas.is_none() {
            self.camera = None;
        }
    }

    /// Places the canvas in the window again, for its viewport and scale.
    fn place(&mut self) {
        let (viewport, scale) = (self.physical_viewport(), self.scale());
        if let Some(canvas) = &mut self.canvas {
            canvas.set_viewport(viewport, scale);
        }
    }

    /// The camera, if it moved since the interface last heard it.
    fn moved(&mut self) -> Option<Camera> {
        let camera = self.canvas.as_ref()?.placement();
        (self.camera != Some(camera)).then(|| {
            self.camera = Some(camera);
            camera
        })
    }

    /// Where the selection lies on the page, if that moved since the
    /// interface last heard it.
    fn selection_moved(&mut self) -> Option<Option<Bounds>> {
        let bounds = self
            .canvas
            .as_ref()
            .and_then(Canvas::selection_bounds)
            .map(bounds);
        (self.selection != bounds).then(|| {
            self.selection = bounds;
            bounds
        })
    }

    /// Where the canvas's area lies on the page, if that moved since the
    /// interface last heard it.
    fn area_moved(&mut self) -> Option<Option<Bounds>> {
        let area = self
            .canvas
            .as_ref()
            .and_then(Canvas::area_bounds)
            .map(bounds);
        (self.area != area).then(|| {
            self.area = area;
            area
        })
    }

    /// What's being typed, if that changed since the interface last heard
    /// it.
    fn typing_moved(&mut self) -> Option<Option<Typed>> {
        let typed = self.canvas.as_ref().and_then(Canvas::typing);
        (self.typed != typed).then(|| {
            self.typed = typed.clone();
            typed
        })
    }

    /// While the canvas types, places the input method's candidate window at
    /// its caret, as an editor does, and keeps the caret blinking.
    fn follow_typing(&mut self) {
        let caret = self.canvas.as_ref().and_then(Canvas::caret_area);
        if caret != self.caret {
            self.caret = caret;
            if let (Some(caret), Some(window)) = (caret, self.window()) {
                let area = ImeRequestData::default().with_cursor_area(
                    PhysicalPosition::new(caret.x0, caret.y0).into(),
                    PhysicalSize::new(caret.width().max(1.0), caret.height().max(1.0)).into(),
                );
                let _ = window.request_ime_update(ImeRequest::Update(area));
            }
        }
        match (caret.is_some(), &self.blink) {
            (true, None) => self.blink = Some(blink(self.handle.clone())),
            (false, Some(_)) => {
                if let Some(blink) = self.blink.take() {
                    blink.abort();
                }
            }
            _ => {}
        }
    }

    fn host(&self) -> Option<BrowserHost> {
        self.browser.as_ref().and_then(|browser| browser.host())
    }

    /// Takes a frame CEF shared: the page's with its mask, or the popup's.
    fn take_shared(&mut self, type_: PaintElementType, info: &AcceleratedPaintInfo) -> Result<()> {
        // On a lost GPU, frames wait for the window to move to a new one.
        let Some(shared) = self.shared.as_mut().filter(|_| !self.gpu.lost()) else {
            return Ok(());
        };
        let frame = shared.importer.take(&self.gpu, info)?;
        if type_ == PaintElementType::POPUP {
            self.painted.popup_frame = Some(frame);
            return Ok(());
        }
        let mask = shared.importer.mask(&self.gpu, &frame)?;
        if let Some(view) = &self.view {
            shared
                .pacer
                .arrive(frame, mask, info.extra.timestamp, self.frame_rate, view);
        }
        Ok(())
    }

    /// Follows the window to the monitor it's on now, rendering at that
    /// monitor's refresh rate; the new frame rate, if it changed.
    fn follow_monitor(&mut self) -> Option<i32> {
        let monitor = self.window()?.current_monitor();
        if monitor == self.monitor {
            return None;
        }
        let frame_rate = refresh::frame_rate(monitor.as_ref());
        self.monitor = monitor;
        (frame_rate != self.frame_rate).then(|| {
            self.frame_rate = frame_rate;
            frame_rate
        })
    }

    /// CEF's cursor, over its page.
    fn set_web_cursor(&mut self, cursor: CursorType) {
        self.web_cursor = icon(cursor);
        if self.input.over_web()
            && let Some(window) = self.window()
        {
            set_cursor(window.as_ref(), self.web_cursor);
        }
    }

    /// Composites the canvas and CEF's paint on the window, and answers the
    /// waiting captures with the same frame.
    fn present(&mut self, window: &Arc<dyn Window>) {
        let size = window.surface_size();
        // A minimized window has no surface to draw on; the restore's
        // resize draws again.
        if size.width == 0 || size.height == 0 {
            return;
        }
        // What the window made on a lost GPU goes with it, to be made again
        // on the process's new one: the surface below, and CEF's frames,
        // which CEF paints again after this call returns.
        if self.gpu.lost() {
            self.gpu = match Gpu::get() {
                Ok(gpu) => gpu,
                Err(error) => {
                    tracing::error!("failed to present the window: {error:#}");
                    return;
                }
            };
            self.compositor = None;
            self.painted.view_frame = None;
            self.painted.popup_frame = None;
            if let Some(shared) = &mut self.shared {
                *shared = Shared::new(&self.gpu);
            }
            if let Some(host) = self.host() {
                host.invalidate(PaintElementType::VIEW);
                host.invalidate(PaintElementType::POPUP);
            }
        }
        if let Some(shared) = &mut self.shared {
            shared.pacer.show(&mut self.painted, Instant::now());
        }
        if self.compositor.is_none() {
            match Compositor::new(&self.gpu, Arc::clone(window)) {
                Ok(created) => self.compositor = Some(created),
                Err(error) => {
                    tracing::error!("failed to create the window's surface: {error:#}");
                    return;
                }
            }
        }
        let origin = self.physical_viewport().origin();
        let Some(compositor) = self.compositor.as_mut() else {
            return;
        };
        let canvas = self.canvas.as_mut().and_then(|canvas| {
            if let Err(error) = canvas.draw() {
                tracing::error!("failed to draw the canvas: {error:#}");
            }
            canvas.texture().map(|texture| {
                let size = (f64::from(texture.width()), f64::from(texture.height()));
                (texture, Rect::from_origin_size(origin, size))
            })
        });
        if let Err(error) = compositor.present(
            &self.gpu,
            (size.width, size.height),
            &mut self.painted,
            canvas,
            std::mem::take(&mut self.captures),
        ) {
            tracing::error!("failed to present the window: {error:#}");
            // The next frame makes the surface again, and draws what
            // `painted` still holds on it, as after a lost GPU.
            self.compositor = None;
        }
    }
}

/// The winit cursor of CEF's.
fn icon(cursor: CursorType) -> Option<CursorIcon> {
    match cursor.as_ref() {
        cef_cursor_type_t::CT_NONE => None,
        cef_cursor_type_t::CT_CROSS => Some(CursorIcon::Crosshair),
        cef_cursor_type_t::CT_HAND => Some(CursorIcon::Pointer),
        cef_cursor_type_t::CT_IBEAM => Some(CursorIcon::Text),
        cef_cursor_type_t::CT_VERTICALTEXT => Some(CursorIcon::VerticalText),
        cef_cursor_type_t::CT_WAIT => Some(CursorIcon::Wait),
        cef_cursor_type_t::CT_PROGRESS => Some(CursorIcon::Progress),
        cef_cursor_type_t::CT_HELP => Some(CursorIcon::Help),
        cef_cursor_type_t::CT_EASTRESIZE => Some(CursorIcon::EResize),
        cef_cursor_type_t::CT_NORTHRESIZE => Some(CursorIcon::NResize),
        cef_cursor_type_t::CT_NORTHEASTRESIZE => Some(CursorIcon::NeResize),
        cef_cursor_type_t::CT_NORTHWESTRESIZE => Some(CursorIcon::NwResize),
        cef_cursor_type_t::CT_SOUTHRESIZE => Some(CursorIcon::SResize),
        cef_cursor_type_t::CT_SOUTHEASTRESIZE => Some(CursorIcon::SeResize),
        cef_cursor_type_t::CT_SOUTHWESTRESIZE => Some(CursorIcon::SwResize),
        cef_cursor_type_t::CT_WESTRESIZE => Some(CursorIcon::WResize),
        cef_cursor_type_t::CT_NORTHSOUTHRESIZE => Some(CursorIcon::NsResize),
        cef_cursor_type_t::CT_EASTWESTRESIZE => Some(CursorIcon::EwResize),
        cef_cursor_type_t::CT_NORTHEASTSOUTHWESTRESIZE => Some(CursorIcon::NeswResize),
        cef_cursor_type_t::CT_NORTHWESTSOUTHEASTRESIZE => Some(CursorIcon::NwseResize),
        cef_cursor_type_t::CT_COLUMNRESIZE => Some(CursorIcon::ColResize),
        cef_cursor_type_t::CT_ROWRESIZE => Some(CursorIcon::RowResize),
        cef_cursor_type_t::CT_MIDDLEPANNING
        | cef_cursor_type_t::CT_EASTPANNING
        | cef_cursor_type_t::CT_NORTHPANNING
        | cef_cursor_type_t::CT_NORTHEASTPANNING
        | cef_cursor_type_t::CT_NORTHWESTPANNING
        | cef_cursor_type_t::CT_SOUTHPANNING
        | cef_cursor_type_t::CT_SOUTHEASTPANNING
        | cef_cursor_type_t::CT_SOUTHWESTPANNING
        | cef_cursor_type_t::CT_WESTPANNING
        | cef_cursor_type_t::CT_MOVE => Some(CursorIcon::Move),
        cef_cursor_type_t::CT_CELL => Some(CursorIcon::Cell),
        cef_cursor_type_t::CT_CONTEXTMENU => Some(CursorIcon::ContextMenu),
        cef_cursor_type_t::CT_ALIAS => Some(CursorIcon::Alias),
        cef_cursor_type_t::CT_NODROP | cef_cursor_type_t::CT_NOTALLOWED => {
            Some(CursorIcon::NotAllowed)
        }
        cef_cursor_type_t::CT_COPY => Some(CursorIcon::Copy),
        cef_cursor_type_t::CT_ZOOMIN => Some(CursorIcon::ZoomIn),
        cef_cursor_type_t::CT_ZOOMOUT => Some(CursorIcon::ZoomOut),
        cef_cursor_type_t::CT_GRAB => Some(CursorIcon::Grab),
        cef_cursor_type_t::CT_GRABBING => Some(CursorIcon::Grabbing),
        _ => Some(CursorIcon::Default),
    }
}

/// Shows `cursor`, or hides the pointer for `None`.
fn set_cursor(window: &dyn Window, cursor: Option<CursorIcon>) {
    match cursor {
        Some(icon) => {
            window.set_cursor_visible(true);
            window.set_cursor(icon.into());
        }
        None => window.set_cursor_visible(false),
    }
}

/// Keeps the browser the render handler is first called with.
fn remember(browser: Option<&mut Browser>) {
    if let Some(browser) = browser {
        with(|main| {
            main.browser.get_or_insert_with(|| browser.clone());
        });
    }
}

/// Renders the page at the refresh rate of the monitor the window moved to.
fn follow_monitor() {
    let moved = with(|main| main.follow_monitor().zip(main.host())).flatten();
    if let Some((frame_rate, host)) = moved {
        host.set_windowless_frame_rate(frame_rate);
    }
}

/// Gives the page that just started the window's focus. CEF renders each
/// document with a widget of its own that starts unfocused, so the focus the
/// window last gave left with the document before, the placeholder the
/// runtime opens first included, and an unfocused page takes keys but draws
/// no caret.
fn focus_page() {
    let host = with(|main| {
        let focused = main.window().is_some_and(|window| window.has_focus());
        focused.then(|| main.host()).flatten()
    })
    .flatten();
    if let Some(host) = host {
        host.set_focus(1);
    }
}

/// Takes the keyboard focus from the page's focused field, as a press on
/// the canvas would in a browser.
fn blur() {
    let frame = with(|main| {
        main.browser
            .as_ref()
            .and_then(|browser| browser.main_frame())
    });
    if let Some(frame) = frame.flatten() {
        frame.execute_java_script(
            Some(&CefString::from(
                "document.activeElement instanceof HTMLElement && document.activeElement.blur()",
            )),
            None,
            0,
        );
    }
}

/// Lets the input method compose text for the page, with its candidate
/// window placed from CEF's composition bounds as they arrive. winit takes
/// the cursor area only with a first one, which those bounds replace.
fn enable_ime(window: &dyn Window) {
    if with(|main| std::mem::replace(&mut main.ime, true)) != Some(false) {
        return;
    }
    let first = ImeRequestData::default().with_cursor_area(
        LogicalPosition::new(0, 0).into(),
        LogicalSize::new(1, 1).into(),
    );
    let request = ImeEnableRequest::new(ImeCapabilities::new().with_cursor_area(), first)
        .expect("the request carries the cursor area it asks for");
    if let Err(error) = window.request_ime_update(ImeRequest::Enable(request)) {
        tracing::warn!("the window has no input method: {error}");
    }
}

/// How often the canvas redraws while it types: twice a caret blink, so it
/// turns on time.
const BLINK: Duration = Duration::from_millis(265);

/// Redraws the window every `BLINK`, for the typing caret's blink.
fn blink(handle: AppHandle<CefRuntime>) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(BLINK).await;
            let redraw = || {
                with(|main| main.request_redraw());
            };
            if handle.run_on_main_thread(redraw).is_err() {
                break;
            }
        }
    })
}

/// Opens the main window from its configuration, with one webview covering
/// it that renders off-screen into it, as GPU textures when `shared`, and
/// makes `screen` reach it. Runs on the main thread, in the app's setup.
pub fn open(
    handle: &AppHandle<CefRuntime>,
    screen: &Screen,
    gpu: Arc<Gpu>,
    shared: bool,
) -> Result<()> {
    let frames = shared.then(|| Shared::new(&gpu));
    WINDOW.set(Some(Main {
        gpu,
        handle: handle.clone(),
        view: None,
        painted: Painted::default(),
        captures: Vec::new(),
        canvas: None,
        viewport: Rect::ZERO,
        settings: Settings::default(),
        camera: None,
        selection: None,
        area: None,
        typed: None,
        caret: None,
        blink: None,
        browser: None,
        input: Input::default(),
        web_cursor: Some(CursorIcon::Default),
        compositor: None,
        ime: false,
        shared: frames,
        monitor: None,
        frame_rate: refresh::FALLBACK,
    }));
    let _ = screen.0.set(handle.clone());

    let config = handle
        .config()
        .app
        .windows
        .iter()
        .find(|window| window.label == MAIN)
        .context("the main window's configuration is missing")?;
    let window = tauri::window::WindowBuilder::from_config(handle, config)?
        .build()
        .context("failed to create the main window")?;
    add_webview(&window, config.url.clone(), shared)?;
    #[cfg(target_os = "macos")]
    keys::take_edit_chords();
    window.show()?;
    window.set_focus()?;
    Ok(())
}

/// Adds the main webview to `window`, covering it and rendering off-screen
/// into it, as GPU textures when `shared`.
fn add_webview(window: &tauri::Window<CefRuntime>, url: WebviewUrl, shared: bool) -> Result<()> {
    let mut offscreen = Offscreen::new(|view| {
        with(|main| main.view = Some(view.clone()));
        paint::Painter::new(view)
    });
    // Without a native view, Chromium has nowhere to set the cursor itself.
    offscreen.cursor_handler = Some(Arc::new(|cursor, _| {
        with(|main| main.set_web_cursor(cursor));
    }));
    offscreen.shared_texture_enabled = shared;

    let size = window
        .inner_size()?
        .to_logical::<f64>(window.scale_factor()?);
    let mut webview = WebviewBuilder::<CefRuntime>::new(MAIN, url)
        .auto_resize()
        .position(LogicalPosition::new(0.0, 0.0))
        .size(size)
        // The canvas shows through where the page paints nothing.
        .transparent(true)
        // The app's window opens no DevTools window, debug builds' Ctrl+Shift+I included;
        // their DevTools protocol on port 4000 is the runtime's, and stays.
        .devtools(false)
        .on_page_load(|_, payload| {
            if payload.event() == PageLoadEvent::Started {
                focus_page();
            }
        })
        // The runtime makes the render handler, which gives the window its
        // view, before it asks for the settings.
        .with_browser_settings(|settings| {
            settings.windowless_frame_rate = with(|main| {
                main.follow_monitor();
                main.frame_rate
            })
            .unwrap_or(refresh::FALLBACK);
        });
    webview
        .runtime_specific_attributes_mut()
        .as_cef_webview_attributes_mut()
        .context("the webview isn't on the CEF runtime")?
        .offscreen = Some(offscreen);
    window
        .add_child(webview)
        .context("failed to add the main webview")?;
    Ok(())
}

/// Puts the main webview back with CEF painting into memory, after a frame
/// it shared couldn't be imported: the interface reloads once, on the path
/// every machine takes, and keeps working.
fn copy_frames() {
    let handle = with(|main| {
        main.shared.take()?;
        main.painted = Painted::default();
        main.browser = None;
        Some(main.handle.clone())
    })
    .flatten();
    let Some(handle) = handle else { return };
    // Off CEF's callback, which mustn't wait on the window's operations.
    thread::spawn(move || {
        if let Some(webview) = handle.get_webview(MAIN)
            && let Err(error) = webview.close()
        {
            tracing::error!("failed to close the main webview: {error}");
            return;
        }
        // Its label frees once the runtime has let it go.
        let deadline = Instant::now() + Duration::from_secs(5);
        while handle.get_webview(MAIN).is_some() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        let reopen = handle.clone();
        let ran = handle.run_on_main_thread(move || {
            let added = reopen
                .get_window(MAIN)
                .context("the main window is gone")
                .and_then(|window| {
                    let url = reopen
                        .config()
                        .app
                        .windows
                        .iter()
                        .find(|window| window.label == MAIN)
                        .context("the main window's configuration is missing")?
                        .url
                        .clone();
                    add_webview(&window, url, false)
                });
            if let Err(error) = added {
                tracing::error!("failed to add the main webview again: {error:#}");
            }
        });
        if let Err(error) = ran {
            tracing::error!("failed to add the main webview again: {error}");
        }
    });
}

/// Routes `pointer` as the main window would have, with Ctrl held for it when
/// `ctrl`, since the window reports modifiers apart from the pointer. The
/// debug build's `debug_pointer` calls it on the main thread.
#[cfg(debug_assertions)]
pub fn feed(app: &Arc<App>, pointer: Pointer, ctrl: bool) {
    let Some(window) = with(|main| main.window()).flatten() else {
        return;
    };
    let modifiers = if ctrl {
        winit::keyboard::ModifiersState::CONTROL
    } else {
        winit::keyboard::ModifiersState::empty()
    };
    let Some(held) = with(|main| std::mem::replace(&mut main.input.modifiers, modifiers)) else {
        return;
    };
    input::pointer(app, window.as_ref(), pointer);
    with(|main| main.input.modifiers = held);
}

/// The native events of every window, from `Cef::on_window_event`. It runs
/// before the runtime's own handling, which still resizes the webview and
/// closes the window.
pub fn on_window_event(app: &Arc<App>, label: &str, window: Arc<dyn Window>, event: &WindowEvent) {
    if label != MAIN {
        return;
    }
    match event {
        // The surface holds the window, which must go before the runtime
        // lets it go, and CEF's browser goes before CEF does.
        WindowEvent::Destroyed => drop(WINDOW.take()),
        WindowEvent::RedrawRequested => {
            with(|main| main.present(&window));
        }
        WindowEvent::SurfaceResized(_) => window.request_redraw(),
        WindowEvent::Moved(_) => follow_monitor(),
        WindowEvent::ScaleFactorChanged { .. } => change(Main::place),
        WindowEvent::Focused(focused) => {
            if *focused {
                enable_ime(window.as_ref());
            }
            if let Some(host) = with(|main| main.host()).flatten() {
                host.set_focus(i32::from(*focused));
            }
        }
        WindowEvent::ModifiersChanged(modifiers) => {
            with(|main| main.input.modifiers = modifiers.state());
        }
        WindowEvent::KeyboardInput { event, .. } => input::key(app, event),
        // While the canvas types, the input method composes into it, as
        // Photoshop's type composes inline; otherwise into the page.
        WindowEvent::Ime(ime) => {
            let mut typed = false;
            change(|main| {
                let field = main.input.field;
                let Some(canvas) = main
                    .canvas
                    .as_mut()
                    .filter(|canvas| !field && canvas.is_typing())
                else {
                    return;
                };
                typed = true;
                let composed = match ime {
                    Ime::Preedit(text, cursor) => canvas.compose(text, *cursor),
                    Ime::Commit(text) => canvas.insert(text),
                    _ => Ok(()),
                };
                if let Err(error) = composed {
                    tracing::error!("the canvas failed the input method: {error:#}");
                }
            });
            if !typed && let Some(host) = with(|main| main.host()).flatten() {
                keys::send_ime(&host, ime);
            }
        }
        event => {
            if let Some(pointer) = Pointer::from_window_event(event) {
                input::pointer(app, window.as_ref(), pointer);
            }
        }
    }
}

/// A page rectangle as the interface takes it.
fn bounds(rect: Rect) -> Bounds {
    Bounds {
        x: rect.x0,
        y: rect.y0,
        width: rect.width(),
        height: rect.height(),
    }
}
