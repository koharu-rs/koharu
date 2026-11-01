use std::{
    path::PathBuf,
    sync::{Arc, OnceLock},
};

use anyhow::{Context, Result};
use hf_hub::{
    HFClient, RepoType, RepoTypeAny,
    progress::{DownloadEvent, ProgressEvent, ProgressHandler},
};

use crate::{
    store,
    transfer::{Activity, USER_AGENT},
};

/// A Hugging Face model or dataset at a pinned commit.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Repository {
    kind: RepoTypeAny,
    id: &'static str,
    commit: &'static str,
}

impl Repository {
    #[must_use]
    pub const fn model(id: &'static str, commit: &'static str) -> Self {
        Self::pinned(RepoTypeAny::Model, id, commit)
    }

    #[must_use]
    pub const fn dataset(id: &'static str, commit: &'static str) -> Self {
        Self::pinned(RepoTypeAny::Dataset, id, commit)
    }

    /// A repository is pinned only by a full commit hash, which a constant
    /// checks as it builds.
    const fn pinned(kind: RepoTypeAny, id: &'static str, commit: &'static str) -> Self {
        let hash = commit.as_bytes();
        assert!(hash.len() == 40, "a commit hash has 40 digits");
        let mut index = 0;
        while index < hash.len() {
            assert!(
                hash[index].is_ascii_hexdigit(),
                "a commit hash is hexadecimal"
            );
            index += 1;
        }
        Self { kind, id, commit }
    }

    /// The paths of the files `names` names, in the same order, each fetched
    /// first if the store doesn't have it; files transfer one after another.
    #[tracing::instrument(skip_all, fields(repository = self.id))]
    pub async fn resolve<const N: usize>(&self, names: [&str; N]) -> Result<[PathBuf; N]> {
        let paths = names.map(|name| self.path(name));
        for (name, path) in names.iter().zip(&paths) {
            store::file(path.clone(), |folder| self.fetch(name, folder)).await?;
        }
        Ok(paths)
    }

    /// Whether the store has the file `name`, without going over the network.
    #[must_use]
    pub fn exists(&self, name: &str) -> bool {
        self.path(name).is_file()
    }

    /// Fetches `name` into `folder` through the Hub's client, which takes Xet
    /// storage's chunks in parallel, and returns where it put it.
    async fn fetch(&self, name: &str, folder: PathBuf) -> Result<PathBuf> {
        let (owner, repository) = hf_hub::split_id(self.id);
        let activity = Arc::new(Activity::start(name.rsplit('/').next().unwrap_or(name)));
        let fetched = async {
            Ok(client()?
                .repository(self.kind, owner, repository)
                .download_file()
                .filename(name)
                .revision(self.commit)
                .local_dir(folder)
                .progress(Arc::clone(&activity))
                .send()
                .await?)
        };
        activity.finish(fetched.await)
    }

    fn path(&self, name: &str) -> PathBuf {
        store::root()
            .join("huggingface")
            .join(self.kind.plural())
            .join(self.id.replace(['/', '\\'], "--"))
            .join("snapshots")
            .join(self.commit)
            .join(name)
    }
}

/// The Hub's client. Files go to the store, not the Hub's own cache.
fn client() -> Result<&'static HFClient> {
    static CLIENT: OnceLock<HFClient> = OnceLock::new();
    if let Some(client) = CLIENT.get() {
        return Ok(client);
    }
    let client = HFClient::builder()
        .user_agent(USER_AGENT)
        .cache_enabled(false)
        .build()
        .context("couldn't build the Hugging Face client")?;
    Ok(CLIENT.get_or_init(|| client))
}

impl ProgressHandler for Activity {
    fn on_progress(&self, event: &ProgressEvent) {
        let ProgressEvent::Download(event) = event else {
            return;
        };
        match event {
            DownloadEvent::Start { total_bytes, .. } => self.progress(0, *total_bytes),
            DownloadEvent::Progress { files } => {
                for file in files {
                    self.progress(file.bytes_completed, file.total_bytes);
                }
            }
            DownloadEvent::AggregateProgress {
                bytes_completed,
                total_bytes,
                ..
            } => self.progress(*bytes_completed, *total_bytes),
            DownloadEvent::Complete => {}
        }
    }
}
