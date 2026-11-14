//! The main window: one webview rendering off-screen in CEF over the native
//! canvas, composited with wgpu on the window's own surface, and the
//! window's input routed to one or the other.
//!
//! CEF's UI thread is the event loop's: the runtime drives CEF through its
//! external message pump, so the render handler, the cursor handler, and
//! `on_window_event` all run on the main thread. CEF may call the render
//! handler back from inside a call into it, so no lock here is held across a
//! call into CEF.

mod compose;
mod input;
mod keys;
mod paint;

use std::{
    io::Cursor,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result};
use image::{DynamicImage, RgbaImage, codecs::jpeg::JpegEncoder, imageops::FilterType};
use koharu_canvas::{Brush, Camera, Canvas, Colors, Rect, Tool};
use koharu_renderer::{Frame, Gpu};
use parking_lot::Mutex;
use tauri::{AppHandle, LogicalPosition, Manager as _, webview::WebviewBuilder};
use tauri_runtime_cef::{
    AsCefWebviewAttributes as _, CefRuntime, Offscreen, OffscreenView, WebviewBuilderCefExt as _,
    cef::{
        Browser, BrowserHost, CefString, CursorType, ImplBrowser as _, ImplBrowserHost as _,
        ImplFrame as _, sys::cef_cursor_type_t,
    },
};
use tokio::sync::oneshot::{self, error::TryRecvError};
use winit::{
    cursor::CursorIcon,
    event::WindowEvent,
    window::{ImeCapabilities, ImeEnableRequest, ImeRequest, ImeRequestData, Window},
};

use self::{compose::Compositor, input::Input, paint::Painted};
use crate::app::App;

pub use self::input::Pointer;

/// An error event's window image: its longer side, its JPEG quality, and
/// how long it waits for the frame.
const SNAPSHOT_SIDE: u32 = 1600;
const SNAPSHOT_QUALITY: u8 = 80;
const SNAPSHOT_WAIT: Duration = Duration::from_secs(1);

/// The window's label in `tauri.conf.json`, and its webview's.
pub const MAIN: &str = "main";

/// What the main window shows, shared between the main thread, where CEF
/// paints and winit delivers events, and the commands.
pub struct Screen {
    gpu: Arc<Gpu>,
    /// The webview's layout, which the runtime keeps current; set when CEF
    /// creates the browser.
    pub view: OnceLock<OffscreenView>,
    pub painted: Mutex<Painted>,
    /// A `capture_window` waiting for the next composited frame.
    pub capture: Mutex<Option<oneshot::Sender<RgbaImage>>>,
    /// The open project's canvas.
    canvas: Mutex<Option<Canvas>>,
    /// The canvas element's rectangle, in CSS pixels.
    viewport: Mutex<Rect>,
    /// What the interface set, for a canvas made later.
    settings: Mutex<Settings>,
    /// The camera as the interface last heard it.
    camera: Mutex<Option<Camera>>,
    /// The browser CEF first paints for, which input reaches through its
    /// host.
    browser: Mutex<Option<Browser>>,
    input: Mutex<Input>,
    /// CEF's cursor, put back when the pointer leaves the canvas; `None`
    /// hides it.
    web_cursor: Mutex<Option<CursorIcon>>,
    compositor: Mutex<Option<Compositor>>,
    ime: AtomicBool,
}

#[derive(Clone, Copy, Default)]
struct Settings {
    tool: Tool,
    brush: Brush,
    colors: Colors,
}

impl Screen {
    pub fn new(gpu: Arc<Gpu>) -> Self {
        Self {
            gpu,
            view: OnceLock::new(),
            painted: Mutex::new(Painted::default()),
            capture: Mutex::new(None),
            canvas: Mutex::new(None),
            viewport: Mutex::new(Rect::ZERO),
            settings: Mutex::new(Settings::default()),
            camera: Mutex::new(None),
            browser: Mutex::new(None),
            input: Mutex::new(Input::default()),
            web_cursor: Mutex::new(Some(CursorIcon::Default)),
            compositor: Mutex::new(None),
            ime: AtomicBool::new(false),
        }
    }

    pub fn request_redraw(&self) {
        if let Some(view) = self.view.get() {
            view.request_redraw();
        }
    }

    fn window(&self) -> Option<Arc<dyn Window>> {
        self.view.get().and_then(OffscreenView::window)
    }

    /// Physical pixels to a device-independent one.
    fn scale(&self) -> f64 {
        self.window().map_or(1.0, |window| window.scale_factor())
    }

    /// The canvas's place in the window, in whole physical pixels.
    fn physical_viewport(&self) -> Rect {
        let scale = self.scale();
        let viewport = *self.viewport.lock();
        Rect::new(
            (viewport.x0 * scale).round(),
            (viewport.y0 * scale).round(),
            (viewport.x1 * scale).round(),
            (viewport.y1 * scale).round(),
        )
    }

    /// Shows `canvas`, set up as the interface asked.
    pub fn show(&self, mut canvas: Canvas) {
        let settings = *self.settings.lock();
        canvas.set_viewport(self.physical_viewport(), self.scale());
        canvas.set_tool(settings.tool);
        canvas.set_brush(settings.brush);
        canvas.set_colors(settings.colors);
        *self.canvas.lock() = Some(canvas);
        self.request_redraw();
    }

    pub fn hide(&self) {
        *self.canvas.lock() = None;
        *self.camera.lock() = None;
    }

    pub fn set_frame(&self, frame: Frame) {
        self.with_canvas(|canvas| canvas.set_frame(frame));
    }

    /// Runs `f` on the canvas, if a project is open.
    pub fn with_canvas<T>(&self, f: impl FnOnce(&mut Canvas) -> T) -> Option<T> {
        self.canvas.lock().as_mut().map(f)
    }

    /// Moves the canvas to the element's rectangle, in CSS pixels.
    pub fn set_viewport(&self, viewport: Rect) {
        *self.viewport.lock() = viewport;
        self.rescale();
    }

    /// Places the canvas again for the window's scale.
    fn rescale(&self) {
        let (viewport, scale) = (self.physical_viewport(), self.scale());
        self.with_canvas(|canvas| canvas.set_viewport(viewport, scale));
        self.request_redraw();
    }

    pub fn set_tool(&self, tool: Tool) {
        self.settings.lock().tool = tool;
        self.with_canvas(|canvas| canvas.set_tool(tool));
        self.request_redraw();
    }

    pub fn set_brush(&self, brush: Brush) {
        self.settings.lock().brush = brush;
        self.with_canvas(|canvas| canvas.set_brush(brush));
        self.request_redraw();
    }

    pub fn set_colors(&self, colors: Colors) {
        self.settings.lock().colors = colors;
        self.with_canvas(|canvas| canvas.set_colors(colors));
        self.request_redraw();
    }

    /// The camera, if it moved since the interface last heard it.
    pub fn camera(&self) -> Option<Camera> {
        let camera = self.with_canvas(|canvas| canvas.placement())?;
        let mut last = self.camera.lock();
        (*last != Some(camera)).then(|| *last.insert(camera))
    }

    fn host(&self) -> Option<BrowserHost> {
        self.browser
            .lock()
            .as_ref()
            .and_then(|browser| browser.host())
    }

    /// Keeps the browser the render handler is first called with.
    fn remember(&self, browser: Option<&mut Browser>) {
        if let Some(browser) = browser {
            self.browser.lock().get_or_insert_with(|| browser.clone());
        }
    }

    /// Takes the keyboard focus from the page's focused field, as a press
    /// on the canvas would in a browser.
    fn blur(&self) {
        let frame = self
            .browser
            .lock()
            .as_ref()
            .and_then(|browser| browser.main_frame());
        if let Some(frame) = frame {
            frame.execute_java_script(
                Some(&CefString::from(
                    "document.activeElement instanceof HTMLElement && document.activeElement.blur()",
                )),
                None,
                0,
            );
        }
    }

    /// CEF's cursor, over its page.
    fn set_web_cursor(&self, cursor: CursorType) {
        let icon = match cursor.as_ref() {
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
        };
        *self.web_cursor.lock() = icon;
        if self.input.lock().over_web()
            && let Some(window) = self.window()
        {
            set_cursor(window.as_ref(), icon);
        }
    }

    /// The window as its next frame shows it, downscaled to fit
    /// `SNAPSHOT_SIDE` and as JPEG, for an error event. The main thread
    /// draws that frame, so there it's `None`, as when no frame comes in
    /// `SNAPSHOT_WAIT` or another capture is waiting.
    pub fn snapshot(&self) -> Option<Vec<u8>> {
        if thread::current().name() == Some("main") {
            return None;
        }
        let (sender, mut receiver) = oneshot::channel();
        {
            let mut capture = self.capture.lock();
            if capture.as_ref().is_some_and(|waiting| !waiting.is_closed()) {
                return None;
            }
            *capture = Some(sender);
        }
        self.request_redraw();
        let deadline = Instant::now() + SNAPSHOT_WAIT;
        let image = loop {
            match receiver.try_recv() {
                Ok(image) => break DynamicImage::ImageRgba8(image),
                Err(TryRecvError::Empty) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return None,
            }
        };
        let image = if image.width().max(image.height()) > SNAPSHOT_SIDE {
            image.resize(SNAPSHOT_SIDE, SNAPSHOT_SIDE, FilterType::Triangle)
        } else {
            image
        };
        let mut jpeg = Cursor::new(Vec::new());
        JpegEncoder::new_with_quality(&mut jpeg, SNAPSHOT_QUALITY)
            .encode_image(&image.to_rgb8())
            .ok()?;
        Some(jpeg.into_inner())
    }

    /// Composites the canvas and CEF's paint on the window, and answers a
    /// waiting capture with the same frame.
    fn present(&self, window: &Arc<dyn Window>) {
        let size = window.surface_size();
        // A minimized window has no surface to draw on; the restore's
        // resize draws again.
        if size.width == 0 || size.height == 0 {
            return;
        }
        let gpu = &self.gpu;
        let mut compositor = self.compositor.lock();
        if compositor.is_none() {
            match Compositor::new(gpu, Arc::clone(window)) {
                Ok(created) => *compositor = Some(created),
                Err(error) => {
                    tracing::error!("failed to create the window's surface: {error:#}");
                    return;
                }
            }
        }
        let Some(compositor) = compositor.as_mut() else {
            return;
        };
        let origin = self.physical_viewport().origin();
        let mut canvas = self.canvas.lock();
        let canvas = canvas.as_mut().and_then(|canvas| {
            if let Err(error) = canvas.draw() {
                tracing::error!("failed to draw the canvas: {error:#}");
            }
            canvas.texture().map(|texture| {
                let size = (f64::from(texture.width()), f64::from(texture.height()));
                (texture, Rect::from_origin_size(origin, size))
            })
        });
        let capture = self.capture.lock().take();
        if let Err(error) = compositor.present(
            gpu,
            (size.width, size.height),
            &mut self.painted.lock(),
            canvas,
            capture,
        ) {
            tracing::error!("failed to present the window: {error:#}");
        }
    }

    /// Lets the input method compose text for the page, with its candidate
    /// window placed from CEF's composition bounds as they arrive.
    fn enable_ime(&self, window: &dyn Window) {
        if self.ime.swap(true, Ordering::Relaxed) {
            return;
        }
        let request = ImeEnableRequest::new(
            ImeCapabilities::new().with_cursor_area(),
            ImeRequestData::default(),
        );
        if let Some(request) = request
            && let Err(error) = window.request_ime_update(ImeRequest::Enable(request))
        {
            tracing::warn!("the window has no input method: {error}");
        }
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

/// Opens the main window from its configuration, with one webview covering
/// it that renders off-screen into `Screen`.
pub fn open(app: &AppHandle<CefRuntime>) -> Result<()> {
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|window| window.label == MAIN)
        .context("the main window's configuration is missing")?;
    let window = tauri::window::WindowBuilder::from_config(app, config)?
        .build()
        .context("failed to create the main window")?;

    let screen = Arc::clone(&app.state::<Arc<App>>().screen);
    let painter = Arc::clone(&screen);
    let mut offscreen = Offscreen::new(move |view| {
        let _ = painter.view.set(view.clone());
        paint::Painter::new(Arc::clone(&painter), view)
    });
    // Without a native view, Chromium has nowhere to set the cursor itself.
    offscreen.cursor_handler = Some(Arc::new(move |cursor, _| screen.set_web_cursor(cursor)));

    let size = window
        .inner_size()?
        .to_logical::<f64>(window.scale_factor()?);
    let mut webview = WebviewBuilder::<CefRuntime>::new(MAIN, config.url.clone())
        .auto_resize()
        .position(LogicalPosition::new(0.0, 0.0))
        .size(size)
        // The canvas shows through where the page paints nothing.
        .transparent(true)
        .with_browser_settings(|settings| settings.windowless_frame_rate = 60);
    webview
        .runtime_specific_attributes_mut()
        .as_cef_webview_attributes_mut()
        .context("the webview isn't on the CEF runtime")?
        .offscreen = Some(offscreen);
    window
        .add_child(webview)
        .context("failed to add the main webview")?;
    window.show()?;
    window.set_focus()?;
    Ok(())
}

/// Routes `pointer` as the main window would have, with Ctrl held for it when
/// `ctrl`, since the window reports modifiers apart from the pointer. The
/// debug build's `debug_pointer` calls it on the main thread.
#[cfg(debug_assertions)]
pub fn feed(app: &Arc<App>, pointer: Pointer, ctrl: bool) {
    let screen = &app.screen;
    let Some(window) = screen.window() else {
        return;
    };
    let modifiers = if ctrl {
        winit::keyboard::ModifiersState::CONTROL
    } else {
        winit::keyboard::ModifiersState::empty()
    };
    let held = std::mem::replace(&mut screen.input.lock().modifiers, modifiers);
    input::pointer(app, window.as_ref(), pointer);
    screen.input.lock().modifiers = held;
}

/// The native events of every window, from `Cef::on_window_event`. It runs
/// before the runtime's own handling, which still resizes the webview and
/// closes the window.
pub fn on_window_event(app: &Arc<App>, label: &str, window: Arc<dyn Window>, event: &WindowEvent) {
    if label != MAIN {
        return;
    }
    let screen = &app.screen;
    match event {
        // The surface holds the window, which must go before the runtime
        // lets it go.
        WindowEvent::Destroyed => drop(screen.compositor.lock().take()),
        WindowEvent::RedrawRequested => screen.present(&window),
        WindowEvent::SurfaceResized(_) => window.request_redraw(),
        WindowEvent::ScaleFactorChanged { .. } => {
            screen.rescale();
            app.camera();
        }
        WindowEvent::Focused(focused) => {
            if *focused {
                screen.enable_ime(window.as_ref());
            }
            if let Some(host) = screen.host() {
                host.set_focus(i32::from(*focused));
            }
        }
        WindowEvent::ModifiersChanged(modifiers) => {
            screen.input.lock().modifiers = modifiers.state();
        }
        WindowEvent::KeyboardInput { event, .. } => {
            let modifiers = screen.input.lock().modifiers;
            if let Some(host) = screen.host() {
                keys::send_key(&host, event, modifiers);
            }
        }
        WindowEvent::Ime(ime) => {
            if let Some(host) = screen.host() {
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
