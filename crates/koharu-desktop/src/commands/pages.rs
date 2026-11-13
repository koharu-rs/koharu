//! The pages rail.

use std::sync::Arc;

use koharu_scene::{Op, PageId};
use tauri::State;

use super::{Bytes, Result, blocking, edit};
use crate::{app::App, thumbnail};

#[tauri::command]
#[specta::specta]
pub async fn delete_pages(app: State<'_, Arc<App>>, pages: Vec<PageId>) -> Result<()> {
    let ops = pages
        .into_iter()
        .map(|page| Op::RemovePage { page: page.into() })
        .collect();
    edit(&app, "delete_pages", ops).await
}

/// Moves a page to `index`, counted after it's taken out.
#[tauri::command]
#[specta::specta]
pub async fn move_page(app: State<'_, Arc<App>>, page: PageId, index: usize) -> Result<()> {
    let op = Op::MovePage {
        page: page.into(),
        index,
    };
    edit(&app, "move_page", vec![op]).await
}

/// The page as it renders, 256 pixels tall, as WebP.
#[tauri::command]
#[specta::specta]
pub async fn get_thumbnail(app: State<'_, Arc<App>>, page: PageId) -> Result<Bytes> {
    let app = Arc::clone(&app);
    blocking(move || {
        let session = app.session()?;
        let frame = app.renderer.render(&session.lock(), page)?;
        Ok(Bytes(thumbnail::webp(&app.renderer, &frame)?))
    })
    .await
}
