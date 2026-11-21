//! What CEF paints: the page and its popup widget (an open `<select>`), kept
//! as premultiplied BGRA on the CPU with the rectangles that changed since
//! the compositor last uploaded them, or, where CEF shares its frames, as
//! Koharu's own GPU textures with the page's transparency mask.

use std::os::raw::c_int;

use tauri_runtime_cef::{
    OffscreenView,
    cef::{
        AcceleratedPaintInfo, Browser, ImplRenderHandler, PaintElementType, Range, Rect as CefRect,
        RenderHandler, ScreenInfo, TextInputMode, WrapRenderHandler, rc::Rc as _,
        sys::cef_text_input_mode_t, wrap_render_handler,
    },
};
use winit::{
    dpi::{LogicalPosition, LogicalSize},
    window::{ImeRequest, ImeRequestData},
};

use super::{copy_frames, remember, with};

/// One painted image.
pub struct Paint {
    pub width: u32,
    pub height: u32,
    /// Premultiplied BGRA, row by row.
    pub bytes: Vec<u8>,
    /// What changed since the last upload; all of it after a size change.
    pub dirty: Vec<PixelRect>,
}

/// A rectangle of a paint's pixels.
#[derive(Clone, Copy, Debug)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// The page's transparency at a fraction of its size: each byte the largest
/// alpha of a `scale`×`scale` block of its pixels.
pub struct Mask {
    pub width: u32,
    pub height: u32,
    pub scale: u32,
    pub bytes: Vec<u8>,
}

/// CEF's latest paints.
#[derive(Default)]
pub struct Painted {
    pub view: Option<Paint>,
    pub popup: Option<Paint>,
    /// Where the popup goes, in window pixels.
    pub popup_origin: Option<(i32, i32)>,
    /// The page and the popup as Koharu's textures, where CEF shares its
    /// frames, in place of `view` and `popup`.
    pub view_frame: Option<wgpu::Texture>,
    pub popup_frame: Option<wgpu::Texture>,
    /// The page's transparency, where CEF shares its frames.
    pub mask: Option<Mask>,
}

impl Painted {
    /// The page's alpha at a window pixel: zero where it paints nothing, or
    /// outside what it painted; a block's at a time where CEF shares its
    /// frames, so the page takes up to a block's width more of the canvas.
    pub fn alpha(&self, x: u32, y: u32) -> u8 {
        if let Some(mask) = &self.mask {
            let (x, y) = (x / mask.scale, y / mask.scale);
            return if x < mask.width && y < mask.height {
                mask.bytes[(y * mask.width + x) as usize]
            } else {
                0
            };
        }
        self.view
            .as_ref()
            .filter(|paint| x < paint.width && y < paint.height)
            .map_or(0, |paint| {
                paint.bytes[(y as usize * paint.width as usize + x as usize) * 4 + 3]
            })
    }
}

/// Copies what changed of a paint CEF lends for the duration of its call.
fn update(slot: &mut Option<Paint>, buffer: &[u8], width: u32, height: u32, dirty: &[CefRect]) {
    let Some(paint) = slot
        .as_mut()
        .filter(|paint| paint.width == width && paint.height == height)
    else {
        *slot = Some(Paint {
            width,
            height,
            bytes: buffer.to_vec(),
            dirty: vec![PixelRect {
                x: 0,
                y: 0,
                width,
                height,
            }],
        });
        return;
    };
    let stride = width as usize * 4;
    for rect in dirty.iter().filter_map(|rect| clip(rect, width, height)) {
        for row in rect.y..rect.y + rect.height {
            let start = row as usize * stride + rect.x as usize * 4;
            let end = start + rect.width as usize * 4;
            paint.bytes[start..end].copy_from_slice(&buffer[start..end]);
        }
        paint.dirty.push(rect);
    }
}

/// `rect` within a `width`×`height` paint, if any of it is.
fn clip(rect: &CefRect, width: u32, height: u32) -> Option<PixelRect> {
    let left = rect.x.clamp(0, width as i32) as u32;
    let top = rect.y.clamp(0, height as i32) as u32;
    let right = rect.x.saturating_add(rect.width).clamp(0, width as i32) as u32;
    let bottom = rect.y.saturating_add(rect.height).clamp(0, height as i32) as u32;
    (left < right && top < bottom).then_some(PixelRect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}

wrap_render_handler! {
    pub struct Painter {
        view: OffscreenView,
    }

    impl RenderHandler {
        fn view_rect(&self, browser: Option<&mut Browser>, rect: Option<&mut CefRect>) {
            remember(browser);
            let Some(rect) = rect else { return };
            let size = self.view.bounds().size.to_logical::<f64>(1.0);
            // CEF asserts on an empty view, which a minimized window has.
            *rect = CefRect {
                x: 0,
                y: 0,
                width: (size.width.round() as i32).max(1),
                height: (size.height.round() as i32).max(1),
            };
        }

        /// Where a point of the view is on the screen, which CEF opens its
        /// own windows at, the context menu among them; unanswered, it takes
        /// the view's point for the screen's.
        fn screen_point(
            &self,
            _browser: Option<&mut Browser>,
            view_x: c_int,
            view_y: c_int,
            screen_x: Option<&mut c_int>,
            screen_y: Option<&mut c_int>,
        ) -> c_int {
            let (Some(window), Some(screen_x), Some(screen_y)) =
                (self.view.window(), screen_x, screen_y)
            else {
                return 0;
            };
            let Ok(outer) = window.outer_position() else {
                return 0;
            };
            let surface = window.surface_position();
            let scale = window.scale_factor();
            let view = self.view.bounds().position.to_logical::<f64>(1.0);
            let x = f64::from(outer.x + surface.x) + (view.x + f64::from(view_x)) * scale;
            let y = f64::from(outer.y + surface.y) + (view.y + f64::from(view_y)) * scale;
            // CEF takes device pixels, but DIPs on macOS.
            #[cfg(target_os = "macos")]
            let (x, y) = (x / scale, y / scale);
            *screen_x = x.round() as c_int;
            *screen_y = y.round() as c_int;
            1
        }

        fn screen_info(
            &self,
            _browser: Option<&mut Browser>,
            screen_info: Option<&mut ScreenInfo>,
        ) -> c_int {
            let (Some(info), Some(window)) = (screen_info, self.view.window()) else {
                return 0;
            };
            // The view is in DIPs, and CEF paints physical pixels.
            info.device_scale_factor = window.scale_factor() as f32;
            1
        }

        fn on_popup_show(&self, _browser: Option<&mut Browser>, show: c_int) {
            if show == 0 {
                with(|main| {
                    main.painted.popup = None;
                    main.painted.popup_frame = None;
                    main.painted.popup_origin = None;
                });
                self.view.request_redraw();
            }
        }

        fn on_popup_size(&self, _browser: Option<&mut Browser>, rect: Option<&CefRect>) {
            let Some(rect) = rect else { return };
            let scale = self.view.window().map_or(1.0, |window| window.scale_factor());
            let origin = (
                (f64::from(rect.x) * scale).round() as i32,
                (f64::from(rect.y) * scale).round() as i32,
            );
            with(|main| main.painted.popup_origin = Some(origin));
        }

        fn on_paint(
            &self,
            browser: Option<&mut Browser>,
            type_: PaintElementType,
            dirty_rects: Option<&[CefRect]>,
            buffer: *const u8,
            width: c_int,
            height: c_int,
        ) {
            remember(browser);
            let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(height)) else {
                return;
            };
            if buffer.is_null() || width == 0 || height == 0 {
                return;
            }
            // SAFETY: CEF lends a `width`×`height` BGRA buffer for the
            // duration of this call.
            let buffer =
                unsafe { std::slice::from_raw_parts(buffer, width as usize * height as usize * 4) };
            let dirty = dirty_rects.unwrap_or_default();
            with(|main| {
                let slot = if type_ == PaintElementType::POPUP {
                    &mut main.painted.popup
                } else {
                    &mut main.painted.view
                };
                update(slot, buffer, width, height, dirty);
            });
            self.view.request_redraw();
        }

        fn on_accelerated_paint(
            &self,
            browser: Option<&mut Browser>,
            type_: PaintElementType,
            _dirty_rects: Option<&[CefRect]>,
            info: Option<&AcceleratedPaintInfo>,
        ) {
            remember(browser);
            let Some(info) = info else { return };
            if let Some(Err(error)) = with(|main| main.take_shared(type_, info)) {
                tracing::error!("falling back to CEF's software frames: {error:#}");
                copy_frames();
                return;
            }
            // The pacer wakes the window when a page frame is due.
            if type_ == PaintElementType::POPUP {
                self.view.request_redraw();
            }
        }

        fn on_virtual_keyboard_requested(
            &self,
            _browser: Option<&mut Browser>,
            input_mode: TextInputMode,
        ) {
            // A field of the page's took the focus or gave it up: while one
            // has it, its keys and the input method are the page's, even
            // while the canvas types.
            let field = *input_mode.as_ref() != cef_text_input_mode_t::CEF_TEXT_INPUT_MODE_NONE;
            with(|main| main.input.field = field);
        }

        fn on_ime_composition_range_changed(
            &self,
            _browser: Option<&mut Browser>,
            _selected_range: Option<&Range>,
            character_bounds: Option<&[CefRect]>,
        ) {
            // The candidate window keeps clear of the composition's bounds,
            // which CEF gives in the view's DIPs.
            let Some(window) = self.view.window() else { return };
            let Some((first, rest)) = character_bounds.and_then(<[CefRect]>::split_first) else {
                return;
            };
            let (left, top) = (first.x, first.y);
            let (right, bottom) = rest.iter().fold(
                (first.x + first.width, first.y + first.height),
                |(right, bottom), bound| {
                    (right.max(bound.x + bound.width), bottom.max(bound.y + bound.height))
                },
            );
            let area = ImeRequestData::default().with_cursor_area(
                LogicalPosition::new(left, top).into(),
                LogicalSize::new((right - left).max(1), (bottom - top).max(1)).into(),
            );
            let _ = window.request_ime_update(ImeRequest::Update(area));
        }
    }
}
