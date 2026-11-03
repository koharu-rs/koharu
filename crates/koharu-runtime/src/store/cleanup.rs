use std::path::{Path, PathBuf};

/// Removes every other version beside each of `versions`, the version
/// folders a start installed. A version another running Koharu still holds
/// open stays, and a later start removes it.
pub(crate) async fn remove_others(versions: Vec<PathBuf>) {
    let removed = tokio::task::spawn_blocking(move || {
        for version in versions {
            remove_beside(&version);
        }
    })
    .await;
    if let Err(error) = removed {
        tracing::warn!("removing older versions failed: {error}");
    }
}

/// Removes everything in `version`'s package folder but `version` itself and
/// the store's own hidden entries, such as locks and stages.
fn remove_beside(version: &Path) {
    let (Some(package), Some(keep)) = (version.parent(), version.file_name()) else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(package) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name != keep && !name.to_string_lossy().starts_with('.') {
            remove(&entry.path());
        }
    }
}

fn remove(path: &Path) {
    let result = if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    match result {
        Ok(()) => tracing::info!("removed {}", path.display()),
        Err(error) => tracing::warn!("couldn't remove {}: {error}", path.display()),
    }
}
