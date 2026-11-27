//! Tracked model and runtime downloads.

use koharu_runtime::{DownloadEntry, DownloadManager};

use super::Result;

pub(crate) fn list_downloads_in(manager: &DownloadManager) -> Vec<DownloadEntry> {
    manager.list_downloads()
}

pub(crate) fn cancel_download_in(manager: &DownloadManager, id: u64) -> Result<()> {
    Ok(manager.cancel_download(id)?)
}

pub(crate) fn retry_download_in(manager: &DownloadManager, id: u64) -> Result<()> {
    Ok(manager.retry_download(id)?)
}

pub(crate) fn dismiss_download_in(manager: &DownloadManager, id: u64) -> Result<()> {
    Ok(manager.dismiss_download(id)?)
}

/// Every tracked download in ascending order of `id`.
#[tauri::command]
#[specta::specta]
pub fn list_downloads() -> Vec<DownloadEntry> {
    list_downloads_in(DownloadManager::global())
}

/// Cancels the active download `id`.
#[tauri::command]
#[specta::specta]
pub fn cancel_download(id: u64) -> Result<()> {
    cancel_download_in(DownloadManager::global(), id)
}

/// Retries a failed or cancelled model download `id` in place.
#[tauri::command]
#[specta::specta]
pub async fn retry_download(id: u64) -> Result<()> {
    retry_download_in(DownloadManager::global(), id)
}

/// Removes a finished, failed, or cancelled download `id` from the list.
#[tauri::command]
#[specta::specta]
pub fn dismiss_download(id: u64) -> Result<()> {
    dismiss_download_in(DownloadManager::global(), id)
}

#[cfg(test)]
mod tests {
    use futures::StreamExt as _;
    use koharu_runtime::{DownloadState, PinnedFile, Repository};

    use super::*;

    #[tokio::test]
    async fn cancel_unknown_id_returns_error() {
        let manager = DownloadManager::new();
        assert!(cancel_download_in(&manager, 999).is_err());
    }

    #[tokio::test]
    async fn dismiss_running_fails_and_retry_then_dismiss_succeeds() {
        let manager = DownloadManager::new();
        let file = PinnedFile::new(
            Repository::model(
                "test/nonexistent",
                "1111111111111111111111111111111111111111",
            ),
            "weights.bin",
            100,
        );
        let handle = manager.start("test/nonexistent", "Test Model", Some("detect"), &[file]);
        let id = handle.id().expect("pending download id");
        assert_eq!(list_downloads_in(&manager).len(), 1);
        assert!(dismiss_download_in(&manager, id).is_err());

        cancel_download_in(&manager, id).expect("cancel running");
        let _ = handle.wait().await;

        let mut stream = manager.downloads();
        while let Some(entry) = stream.next().await {
            if entry.id == id && entry.state != DownloadState::Running {
                break;
            }
        }

        // Retry in Tokio runtime context (verifies `tokio::spawn` works cleanly), then cancel and dismiss.
        retry_download_in(&manager, id).expect("retry terminal download");
        assert_eq!(list_downloads_in(&manager)[0].state, DownloadState::Running);
        let _ = cancel_download_in(&manager, id);
        while let Some(entry) = stream.next().await {
            if entry.id == id && entry.state != DownloadState::Running {
                break;
            }
        }

        dismiss_download_in(&manager, id).expect("dismiss finished");
        assert!(list_downloads_in(&manager).is_empty());
    }
}
