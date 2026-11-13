//! What CEF paints: the page and its popup widget (an open `<select>`), kept
//! as premultiplied BGRA on the CPU with the rectangles that changed since
//! the compositor last uploaded them.

use std::{os::raw::c_int, sync::Arc};

use tauri_runtime_cef::{
    OffscreenView,
    cef::{
        Browser, ImplRenderHandler, PaintElementType, Range, Rect as CefRect, RenderHandler,
        ScreenInfo, WrapRenderHandler, rc::Rc as _, wrap_render_handler,
    },
};
use winit::{
    dpi::{LogicalPosition, LogicalSize},
    window::{ImeRequest, ImeRequestData},
};

use super::Screen;

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

/// CEF's latest paints.
#[derive(Default)]
pub struct Painted {
    pub view: Option<Paint>,
    pub popup: Option<Paint>,
    /// Where the popup goes, in window pixels.
    pub popup_origin: Option<(i32, i32)>,
}

impl Painted {
    /// The page's alpha at a window pixel: zero where it paints nothing, or
    /// outside what it painted.
    pub fn alpha(&self, x: u32, y: u32) -> u8 {
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
        screen: Arc<Screen>,
        view: OffscreenView,
    }

    impl RenderHandler {
        fn view_rect(&self, browser: Option<&mut Browser>, rect: Option<&mut CefRect>) {
            self.screen.remember(browser);
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
                let mut painted = self.screen.painted.lock();
                painted.popup = None;
                painted.popup_origin = None;
                drop(painted);
                self.view.request_redraw();
            }
        }

        fn on_popup_size(&self, _browser: Option<&mut Browser>, rect: Option<&CefRect>) {
            let Some(rect) = rect else { return };
            let scale = self.view.window().map_or(1.0, |window| window.scale_factor());
            self.screen.painted.lock().popup_origin = Some((
                (f64::from(rect.x) * scale).round() as i32,
                (f64::from(rect.y) * scale).round() as i32,
            ));
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
            self.screen.remember(browser);
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
            let mut painted = self.screen.painted.lock();
            let slot = if type_ == PaintElementType::POPUP {
                &mut painted.popup
            } else {
                &mut painted.view
            };
            update(slot, buffer, width, height, dirty);
            drop(painted);
            self.view.request_redraw();
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
