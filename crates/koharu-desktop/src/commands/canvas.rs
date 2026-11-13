//! What the interface sets on the canvas.

use std::sync::Arc;

use koharu_canvas::{Brush, Colors, Rect, Tool};
use koharu_scene::LayerId;
use serde::Deserialize;
use specta::Type;
use tauri::State;

use crate::app::App;

/// The canvas element's rectangle, in CSS pixels from the window's corner.
#[derive(Clone, Copy, Debug, Deserialize, Type)]
pub struct ViewportRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[tauri::command]
#[specta::specta]
pub fn set_canvas_viewport(app: State<'_, Arc<App>>, rect: ViewportRect) {
    app.screen.set_viewport(Rect::new(
        rect.x,
        rect.y,
        rect.x + rect.width.max(0.0),
        rect.y + rect.height.max(0.0),
    ));
    app.camera();
}

#[tauri::command]
#[specta::specta]
pub fn set_tool(app: State<'_, Arc<App>>, tool: Tool) {
    tracing::info!(?tool, "tool");
    app.screen.set_tool(tool);
}

#[tauri::command]
#[specta::specta]
pub fn set_brush(app: State<'_, Arc<App>>, brush: Brush) {
    app.screen.set_brush(brush);
}

#[tauri::command]
#[specta::specta]
pub fn set_colors(app: State<'_, Arc<App>>, colors: Colors) {
    app.screen.set_colors(colors);
}

/// Fits the page in the canvas, as the fit shortcut does.
#[tauri::command]
#[specta::specta]
pub fn fit_canvas(app: State<'_, Arc<App>>) {
    app.screen.with_canvas(|canvas| canvas.fit());
    app.camera();
    app.screen.request_redraw();
}

/// Zooms about the canvas's centre, `zoom` CSS pixels to a page pixel, as
/// the status bar's slider sets it.
#[tauri::command]
#[specta::specta]
pub fn set_zoom(app: State<'_, Arc<App>>, zoom: f64) {
    app.screen.zoom(zoom);
    app.camera();
}

/// The selection the Layers panel made.
#[tauri::command]
#[specta::specta]
pub fn select_layers(app: State<'_, Arc<App>>, layers: Vec<LayerId>) {
    app.screen.with_canvas(|canvas| canvas.select(layers));
    app.screen.request_redraw();
}
