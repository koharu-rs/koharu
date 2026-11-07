use std::path::{Path, PathBuf};

use anyhow::Result;

use super::{Host, LIBRARIES, extract};
use crate::{Backend, store};

/// One of Koharu's own builds of a native library, released on GitHub as
/// one archive per platform and backend.
pub(super) struct Release {
    /// The GitHub repository the release is in.
    pub(super) repository: &'static str,
    /// The store folder its versions go in. A start keeps one version per
    /// folder, so two packages released from one repository need two.
    pub(super) package: &'static str,
    pub(super) tag: &'static str,
}

impl Release {
    /// The folder of this version, with one archive per platform and backend
    /// in it.
    pub(super) fn version(&self) -> PathBuf {
        store::root().join(self.package).join(self.tag)
    }

    /// The folder of the archive for `host` and `backend`.
    pub(super) fn folder(&self, host: Host, backend: Backend) -> PathBuf {
        let (triple, variant): (&str, &str) = (host.into(), backend.into());
        self.version().join(format!("{triple}-{variant}"))
    }

    /// Downloads the archive for `host` and `backend`, named
    /// `{triple}-{variant}.tar.gz`, which is complete when `complete` says so.
    pub(super) async fn download(
        &self,
        host: Host,
        backend: Backend,
        complete: impl Fn(&Path) -> bool + Send + Sync,
    ) -> Result<()> {
        let (triple, variant): (&str, &str) = (host.into(), backend.into());
        let url = format!(
            "https://github.com/{}/releases/download/{}/{triple}-{variant}.tar.gz",
            self.repository, self.tag
        );
        store::put(
            &self.version(),
            self.folder(host, backend),
            |folder| folder.is_dir() && complete(folder),
            move |stage| async move {
                extract(&url, &stage, LIBRARIES).await?;
                Ok(stage)
            },
        )
        .await?;
        Ok(())
    }
}
