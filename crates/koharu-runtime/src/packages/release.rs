use std::path::{Path, PathBuf};

use anyhow::Result;

use super::{Host, LIBRARIES, install_archive};
use crate::{Backend, store};

/// One of Koharu's own builds of a native library, released on GitHub as
/// one archive per platform and backend.
pub(super) struct Release {
    /// The GitHub repository the release is in.
    pub(super) repository: &'static str,
    /// The store folder its versions install in. The store keeps one version
    /// per folder, so two packages released from one repository need two.
    pub(super) package: &'static str,
    pub(super) tag: &'static str,
}

impl Release {
    /// Installs the archive for `host` and `backend`, named
    /// `{triple}-{variant}.tar.gz`, which is complete when `complete` says so,
    /// and returns its folder.
    pub(super) async fn install(
        &self,
        host: Host,
        backend: Backend,
        complete: impl Fn(&Path) -> bool + Send + Sync,
    ) -> Result<PathBuf> {
        let (triple, variant): (&str, &str) = (host.into(), backend.into());
        let archive = format!("{triple}-{variant}");
        let url = format!(
            "https://github.com/{}/releases/download/{}/{archive}.tar.gz",
            self.repository, self.tag
        );
        store::install(
            store::root()
                .join(self.package)
                .join(self.tag)
                .join(&archive),
            |folder| folder.is_dir() && complete(folder),
            move |stage| async move {
                install_archive(&url, &stage, LIBRARIES).await?;
                Ok(stage)
            },
        )
        .await
    }
}
