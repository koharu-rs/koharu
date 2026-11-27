//! Project ComicInfo metadata commands.

use std::sync::Arc;

use anyhow::Context as _;
use koharu_io::{COMIC_INFO_KEY, ComicInfo};
use koharu_scene::{Answer, Change, Edit, Op, Query};
use tauri::State;

use super::Result;
use crate::app::App;

fn read_comic_info(session: &koharu_scene::Session) -> anyhow::Result<ComicInfo> {
    let Answer::Metadata(metadata) = session.query(Query::GetProjectMetadata)? else {
        unreachable!("Query::GetProjectMetadata answers Answer::Metadata");
    };
    match metadata.get(COMIC_INFO_KEY) {
        Some(value) => {
            serde_json::from_value(value.clone()).context("stored ComicInfo metadata is invalid")
        }
        None => Ok(ComicInfo::default()),
    }
}

/// Returns the open project's ComicInfo metadata without mutating disk.
#[tauri::command]
#[specta::specta]
pub async fn get_comic_info(app: State<'_, Arc<App>>) -> Result<ComicInfo> {
    let app = Arc::clone(&app);
    Ok(tokio::task::spawn_blocking(move || {
        let session = app.session()?;
        let locked = session.lock();
        read_comic_info(&locked)
    })
    .await??)
}

/// Reconciles stale prose translations against the stored metadata and commits
/// the updated ComicInfo as an undoable `"set_metadata"` edit.
#[tauri::command]
#[specta::specta]
pub async fn set_comic_info(app: State<'_, Arc<App>>, mut comic_info: ComicInfo) -> Result<Change> {
    let app = Arc::clone(&app);
    Ok(
        tokio::task::spawn_blocking(move || -> anyhow::Result<Change> {
            let previous = {
                let session = app.session()?;
                let locked = session.lock();
                read_comic_info(&locked)?
            };
            comic_info.reconcile_with(&previous);
            let value = if comic_info.is_empty() {
                None
            } else {
                Some(serde_json::to_value(&comic_info)?)
            };
            app.edit(Edit {
                label: "set_metadata".to_owned(),
                ops: vec![Op::SetProjectMetadata {
                    key: COMIC_INFO_KEY.to_owned(),
                    value,
                }],
            })
        })
        .await??,
    )
}
