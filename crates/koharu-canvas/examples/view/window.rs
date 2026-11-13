//! The window: winit's events, through the reducer, as the W3C pointer and
//! keyboard events the desktop gets from CEF, the surface the canvas's
//! texture is shown on, and the key bindings React would own.

use std::sync::Arc;

use anyhow::{Context as _, Result};
use koharu_canvas::{Rect, Tool};
use ui_events::{
    ScrollDelta,
    keyboard::{Key, KeyState, KeyboardEvent, NamedKey},
    pointer::PointerEvent,
};
use ui_events_winit::{WindowEventReducer, WindowEventTranslation};
use vello::wgpu;
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::WindowEvent,
    event_loop::ActiveEventLoop,
    window::{Window, WindowId},
};

use crate::host::Host;

/// The window, its surface, and the input state React would keep.
pub struct App {
    host: Host,
    window: Option<Screen>,
    /// winit's events as W3C pointer and keyboard events.
    reducer: WindowEventReducer,
    /// The tool to go back to when Space is let go.
    before_space: Option<Tool>,
}

struct Screen {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    blitter: wgpu::util::TextureBlitter,
}

impl App {
    pub fn new(host: Host) -> Self {
        Self {
            host,
            window: None,
            reducer: WindowEventReducer::default(),
            before_space: None,
        }
    }

    fn input(&mut self, event: PointerEvent) {
        let handled = self
            .host
            .canvas
            .input(event)
            .and_then(|effects| self.host.handle(effects));
        if let Err(error) = handled {
            tracing::warn!("{error:#}");
        }
        self.redraw();
    }

    fn redraw(&self) {
        if let Some(screen) = &self.window {
            let cursor = self.host.canvas.cursor();
            screen.window.set_cursor_visible(cursor.is_some());
            if let Some(cursor) = cursor {
                screen.window.set_cursor(cursor);
            }
            screen.window.request_redraw();
        }
    }

    fn present(&mut self) -> Result<()> {
        let Some(screen) = &self.window else {
            return Ok(());
        };
        let gpu = self.host.renderer.gpu()?;
        let texture = match self.host.canvas.draw()? {
            Some(texture) => texture,
            None => match self.host.canvas.texture() {
                Some(texture) => texture,
                None => return Ok(()),
            },
        };
        let source = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let frame = match screen.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            _ => {
                screen.surface.configure(&gpu.device, &screen.config);
                return Ok(());
            }
        };
        let target = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        screen
            .blitter
            .copy(&gpu.device, &mut encoder, &source, &target);
        gpu.queue.submit([encoder.finish()]);
        gpu.queue.present(frame);
        Ok(())
    }

    fn key(&mut self, event: &KeyboardEvent) -> Result<()> {
        let canvas = &mut self.host.canvas;
        let pressed = event.state == KeyState::Down;
        if let Key::Character(space) = &event.key
            && space == " "
        {
            match (pressed, self.before_space) {
                (true, None) => {
                    self.before_space = Some(canvas.tool());
                    canvas.set_tool(Tool::Pan);
                }
                (false, Some(tool)) => {
                    self.before_space = None;
                    canvas.set_tool(tool);
                }
                _ => {}
            }
            return Ok(());
        }
        if !pressed {
            return Ok(());
        }
        let command = event.modifiers.ctrl() || event.modifiers.meta();
        match &event.key {
            Key::Named(NamedKey::Escape) => canvas.cancel(),
            Key::Named(NamedKey::Delete) => self.host.delete()?,
            Key::Character(text) => match (command, text.to_lowercase().as_str()) {
                (true, "z") => self.host.history(event.modifiers.shift())?,
                (false, "f") => {
                    canvas.fit();
                }
                (false, letter) => {
                    let tool = match letter {
                        "v" => Tool::Select,
                        "t" => Tool::Text,
                        "b" => Tool::Draw,
                        "e" => Tool::Eraser,
                        "i" => Tool::ColorPicker,
                        "j" => Tool::Remove,
                        "h" => Tool::Pan,
                        _ => return Ok(()),
                    };
                    canvas.set_tool(tool);
                }
                _ => {}
            },
            _ => {}
        }
        Ok(())
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let screen = (|| -> Result<Screen> {
            let window = Arc::new(
                event_loop.create_window(
                    Window::default_attributes()
                        .with_title("koharu-canvas")
                        .with_inner_size(LogicalSize::new(1280.0, 800.0)),
                )?,
            );
            let gpu = self.host.renderer.gpu()?;
            let surface = gpu.instance.create_surface(Arc::clone(&window))?;
            let capabilities = surface.get_capabilities(&gpu.adapter);
            // CEF's bytes and the canvas's are already encoded, so the surface
            // doesn't encode again.
            let format = capabilities
                .formats
                .iter()
                .copied()
                .find(|format| !format.is_srgb())
                .context("the surface has no non-sRGB format")?;
            let size = window.inner_size();
            let mut config = surface
                .get_default_config(&gpu.adapter, size.width.max(1), size.height.max(1))
                .context("the adapter can't present to the window")?;
            config.format = format;
            config.view_formats = Vec::new();
            config.present_mode = wgpu::PresentMode::Fifo;
            surface.configure(&gpu.device, &config);
            let blitter = wgpu::util::TextureBlitter::new(&gpu.device, format);
            self.host
                .canvas
                .set_viewport(window_rect(size), window.scale_factor());
            Ok(Screen {
                window,
                surface,
                config,
                blitter,
            })
        })();
        match screen {
            Ok(screen) => {
                screen.window.request_redraw();
                self.window = Some(screen);
            }
            Err(error) => {
                tracing::error!("couldn't open the window: {error:#}");
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let (Some(screen), Ok(gpu)) = (&mut self.window, self.host.renderer.gpu()) {
                    screen.config.width = size.width.max(1);
                    screen.config.height = size.height.max(1);
                    screen.surface.configure(&gpu.device, &screen.config);
                    let scale = screen.window.scale_factor();
                    self.host.canvas.set_viewport(window_rect(size), scale);
                }
                self.redraw();
            }
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.present() {
                    tracing::warn!("couldn't draw: {error:#}");
                }
            }
            event => {
                let Some(screen) = &self.window else {
                    return;
                };
                match self.reducer.reduce(screen.window.scale_factor(), &event) {
                    Some(WindowEventTranslation::Pointer(event)) => {
                        self.input(browser_wheel(event))
                    }
                    Some(WindowEventTranslation::Keyboard(event)) => {
                        if let Err(error) = self.key(&event) {
                            tracing::warn!("{error:#}");
                        }
                        self.redraw();
                    }
                    None => {}
                }
            }
        }
    }
}

/// The whole window, in physical pixels: the canvas fills it.
fn window_rect(size: winit::dpi::PhysicalSize<u32>) -> Rect {
    Rect::new(0.0, 0.0, f64::from(size.width), f64::from(size.height))
}

/// A wheel with the browser's sign. ui-events-winit 0.3.0 passes winit's
/// delta through, whose positive values move the content down, where
/// `ScrollDelta`'s move the viewport down, as a browser's `deltaY` does.
fn browser_wheel(event: PointerEvent) -> PointerEvent {
    let PointerEvent::Scroll(mut scroll) = event else {
        return event;
    };
    scroll.delta = match scroll.delta {
        ScrollDelta::LineDelta(x, y) => ScrollDelta::LineDelta(-x, -y),
        ScrollDelta::PageDelta(x, y) => ScrollDelta::PageDelta(-x, -y),
        ScrollDelta::PixelDelta(delta) => ScrollDelta::PixelDelta((-delta.x, -delta.y).into()),
    };
    PointerEvent::Scroll(scroll)
}
