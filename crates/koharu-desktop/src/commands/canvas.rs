//! What the interface sets on the canvas. The screen tells the camera when
//! one of them moves it.

use std::sync::Arc;

use koharu_canvas::{Brush, Colors, Rect, RubyEdit, Story, Tool, Vec2};
use koharu_scene::{CharStyle, LayerId};
use tauri::State;

use crate::app::App;

/// The canvas element's rectangle, in CSS pixels from the window's corner.
#[tauri::command]
#[specta::specta]
pub fn set_viewport(app: State<'_, Arc<App>>, x: f64, y: f64, width: f64, height: f64) {
    app.screen
        .set_viewport(Rect::new(x, y, x + width.max(0.0), y + height.max(0.0)));
}

#[tauri::command]
#[specta::specta]
pub fn set_tool(app: State<'_, Arc<App>>, tool: Tool) {
    tracing::info!(?tool, "tool");
    app.screen.set_tool(Arc::clone(&app), tool);
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

/// Whether the Story panel shows, and the text whose entry the pointer is
/// on, which the canvas numbers and outlines.
#[tauri::command]
#[specta::specta]
pub fn set_story(app: State<'_, Arc<App>>, story: Story) {
    app.screen.set_story(story);
}

/// Whether the page shows its scan alone, as Lightroom's Before view shows a
/// photo without its edits.
#[tauri::command]
#[specta::specta]
pub fn set_original(app: State<'_, Arc<App>>, original: bool) {
    app.screen.set_original(original);
}

/// Fits the page in the canvas, as the fit shortcut does.
#[tauri::command]
#[specta::specta]
pub fn fit(app: State<'_, Arc<App>>) {
    app.screen.fit();
}

/// Zooms about the canvas's centre, `zoom` CSS pixels to a page pixel, as
/// the status bar's slider sets it.
#[tauri::command]
#[specta::specta]
pub fn set_zoom(app: State<'_, Arc<App>>, zoom: f64) {
    app.screen.zoom_to(zoom);
}

/// The selection the Layers panel made.
#[tauri::command]
#[specta::specta]
pub fn select(app: State<'_, Arc<App>>, layers: Vec<LayerId>) {
    app.screen.select(Arc::clone(&app), layers);
}

/// Moves the selection `dx` and `dy` page pixels with the Move tool, as
/// Photoshop's arrow keys nudge it.
#[tauri::command]
#[specta::specta]
pub fn nudge(app: State<'_, Arc<App>>, dx: f64, dy: f64) {
    app.screen.nudge(Arc::clone(&app), Vec2::new(dx, dy));
}

/// Drops the area the marquee and the lasso marked, as Photoshop's Deselect
/// does.
#[tauri::command]
#[specta::specta]
pub fn deselect(app: State<'_, Arc<App>>) {
    app.screen.deselect();
}

/// The type bar's character settings for the text being typed: the
/// selected characters take them, or at a caret the next ones typed.
#[tauri::command]
#[specta::specta]
pub fn style_typing(app: State<'_, Arc<App>>, patch: CharStyle) {
    app.screen.style_typing(patch);
}

/// What the type bar's ruby field does to the text being typed: it opens on the
/// selected characters, or on the base the caret is in, sets their ruby, and
/// closes.
#[tauri::command]
#[specta::specta]
pub fn annotate_typing(app: State<'_, Arc<App>>, edit: RubyEdit) {
    app.screen.annotate_typing(edit);
}
