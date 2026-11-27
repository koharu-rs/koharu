use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use backon::Retryable;
use hf_hub::{
    HFClient, HFError, RepoType, RepoTypeAny,
    progress::{DownloadEvent, ProgressEvent, ProgressHandler},
};
use tokio::sync::OnceCell;
use tokio_util::sync::CancellationToken;

use crate::{
    store,
    transfer::{self, BACKOFF, Cancelled, DownloadManager, USER_AGENT},
};

/// A Hugging Face model or dataset at a pinned commit.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Repository {
    kind: RepoTypeAny,
    id: &'static str,
    commit: &'static str,
}

/// A pinned file inside a Hugging Face repository together with its declared
/// byte size.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PinnedFile {
    pub repo: Repository,
    pub filename: &'static str,
    pub bytes: u64,
}

impl PinnedFile {
    #[must_use]
    pub const fn new(repo: Repository, filename: &'static str, bytes: u64) -> Self {
        Self {
            repo,
            filename,
            bytes,
        }
    }

    #[must_use]
    pub fn exists(&self) -> bool {
        self.repo.exists(self.filename)
    }

    #[must_use]
    pub fn path(&self) -> PathBuf {
        self.repo.path(self.filename)
    }
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

    #[must_use]
    pub const fn id(&self) -> &'static str {
        self.id
    }

    #[must_use]
    pub const fn commit(&self) -> &'static str {
        self.commit
    }

    /// The paths of the files `names` names, in the same order, each fetched
    /// first if the store doesn't have it; files transfer one after another.
    #[tracing::instrument(skip_all, fields(repository = self.id))]
    pub async fn resolve<const N: usize>(&self, names: [&str; N]) -> Result<[PathBuf; N]> {
        self.resolve_cancellable(names, &CancellationToken::new())
            .await
    }

    /// Cancellable variant of [`Repository::resolve`]. Fast-returns `Ok(paths)`
    /// without registering a download entry when all files are already present.
    #[tracing::instrument(skip_all, fields(repository = self.id))]
    pub async fn resolve_cancellable<const N: usize>(
        &self,
        names: [&str; N],
        cancel: &CancellationToken,
    ) -> Result<[PathBuf; N]> {
        if cancel.is_cancelled() {
            return Err(Cancelled.into());
        }
        let paths = names.map(|name| self.path(name));
        if names.iter().all(|name| self.exists(name)) {
            return Ok(paths);
        }
        DownloadManager::global()
            .resolve_repository(*self, &names, cancel)
            .await?;
        Ok(paths)
    }

    /// Whether the store has the file `name`, without going over the network.
    #[must_use]
    pub fn exists(&self, name: &str) -> bool {
        self.path(name).is_file()
    }

    #[must_use]
    pub fn path(&self, name: &str) -> PathBuf {
        store::root()
            .join("huggingface")
            .join(self.kind.plural())
            .join(self.id.replace(['/', '\\'], "--"))
            .join("snapshots")
            .join(self.commit)
            .join(name)
    }

    pub(crate) async fn resolve_one(
        &self,
        name: &str,
        cancel: &CancellationToken,
        on_progress: Arc<dyn Fn(u64, u64) + Send + Sync>,
    ) -> Result<PathBuf> {
        if cancel.is_cancelled() {
            return Err(Cancelled.into());
        }
        let path = self.path(name);
        let put_fut = store::put(
            &path,
            path.clone(),
            |path| path.is_file(),
            |folder| self.fetch(name, folder, cancel, Arc::clone(&on_progress)),
        );
        tokio::select! {
            biased;
            () = cancel.cancelled() => Err(Cancelled.into()),
            result = put_fut => result,
        }
    }

    /// Fetches `name` into `folder` through the Hub's client and returns where
    /// it put it. The client's local-folder download follows the Hub's
    /// redirect before it reads the Xet hash on it, so it never takes Xet's
    /// chunks: the file comes as one plain stream, which starts over when it
    /// breaks off.
    async fn fetch(
        &self,
        name: &str,
        folder: PathBuf,
        cancel: &CancellationToken,
        on_progress: Arc<dyn Fn(u64, u64) + Send + Sync>,
    ) -> Result<PathBuf> {
        if cancel.is_cancelled() {
            return Err(Cancelled.into());
        }
        let (owner, repository) = hf_hub::split_id(self.id);
        let progress = Arc::new(RepoProgress(on_progress));
        let download = (|| async {
            Ok(client()
                .await?
                .repository(self.kind, owner, repository)
                .download_file()
                .filename(name)
                .revision(self.commit)
                .local_dir(folder.clone())
                .progress(Arc::clone(&progress))
                .send()
                .await?)
        })
        .retry(BACKOFF)
        .when(|error| !cancel.is_cancelled() && broke_off(error))
        .notify(|error, delay| tracing::warn!(?delay, "retrying {name}: {error:#}"));

        tokio::select! {
            biased;
            () = cancel.cancelled() => Err(Cancelled.into()),
            result = download => result,
        }
    }
}

/// Whether a file broke off on its way: the client retries the request for a
/// file, but not what comes of it.
fn broke_off(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref(),
        Some(HFError::Request { source, .. }) if source.is_decode()
    )
}

/// The Hub's client, over the transfers' own. Files go to the store, not the
/// Hub's own cache.
async fn client() -> Result<&'static HFClient> {
    static CLIENT: OnceCell<HFClient> = OnceCell::const_new();
    CLIENT
        .get_or_try_init(|| async {
            HFClient::builder()
                .client(transfer::client().await?.clone())
                // The client sends it already; the Hub's own client for
                // redirects takes it from here.
                .user_agent(USER_AGENT)
                .cache_enabled(false)
                // It waits half a second before it asks again, then one, and so
                // on to eight, each cut short at random.
                .retry_base_delay(Duration::from_millis(250))
                .build()
                .context("couldn't build the Hugging Face client")
        })
        .await
}

struct RepoProgress(Arc<dyn Fn(u64, u64) + Send + Sync>);

impl ProgressHandler for RepoProgress {
    fn on_progress(&self, event: &ProgressEvent) {
        let ProgressEvent::Download(event) = event else {
            return;
        };
        match event {
            DownloadEvent::Start { total_bytes, .. } => (self.0)(0, *total_bytes),
            DownloadEvent::Progress { files } => {
                for file in files {
                    (self.0)(file.bytes_completed, file.total_bytes);
                }
            }
            DownloadEvent::AggregateProgress {
                bytes_completed,
                total_bytes,
                ..
            } => (self.0)(*bytes_completed, *total_bytes),
            DownloadEvent::Complete => {}
        }
    }
}
