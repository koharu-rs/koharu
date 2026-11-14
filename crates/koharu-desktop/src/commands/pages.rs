//! The pages rail's thumbnails.

use std::sync::Arc;

use anyhow::ensure;
use koharu_scene::PageId;
use tauri::State;

use super::{Bytes, Result};
use crate::app::App;

/// How tall a thumbnail is, in pixels.
const HEIGHT: u32 = 256;

/// The page as it renders, `HEIGHT` pixels tall, as WebP.
#[tauri::command]
#[specta::specta]
pub async fn get_thumbnail(app: State<'_, Arc<App>>, page: PageId) -> Result<Bytes> {
    let app = Arc::clone(&app);
    Ok(tokio::task::spawn_blocking(move || {
        let session = app.session()?;
        let frame = app.renderer.render(&session.lock(), page)?;
        let raster = app.renderer.rasterize(&frame)?;
        let (width, height) = raster.image.dimensions();
        ensure!(width > 0 && height > 0, "the page is empty");
        let scaled = (u64::from(width) * u64::from(HEIGHT) / u64::from(height)).max(1);
        let image = image::imageops::thumbnail(&raster.image, u32::try_from(scaled)?, HEIGHT);
        let encoder = webp::Encoder::from_rgba(image.as_raw(), image.width(), image.height());
        Ok(Bytes(encoder.encode(80.0).to_vec()))
    })
    .await??)
}
