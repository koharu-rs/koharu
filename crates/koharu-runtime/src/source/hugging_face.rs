use std::{path::PathBuf, sync::OnceLock};

use hf_hub::{HFClient, HFRepository, RepoType, repository::download::HFByteStream, split_id};

use crate::{Store, download, network};

static CLIENT: OnceLock<HFClient> = OnceLock::new();

fn client() -> anyhow::Result<HFClient> {
    if let Some(client) = CLIENT.get() {
        return Ok(client.clone());
    }
    let max_retries = network::config()?.max_retries as usize;
    let http = network::http()?;
    let client = HFClient::builder()
        .client(http)
        .cache_enabled(false)
        .retry_max_attempts(max_retries)
        .build()?;
    Ok(CLIENT.get_or_init(|| client).clone())
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum RepositoryKind {
    Model,
    Dataset,
}

/// An immutable file snapshot hosted by Hugging Face.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct HuggingFaceFile<'a> {
    kind: RepositoryKind,
    repository: &'a str,
    revision: &'a str,
    filename: &'a str,
}

impl<'a> HuggingFaceFile<'a> {
    #[must_use]
    pub const fn pinned(repository: &'a str, revision: &'a str, filename: &'a str) -> Self {
        Self {
            kind: RepositoryKind::Model,
            repository,
            revision,
            filename,
        }
    }

    #[must_use]
    pub const fn pinned_dataset(repository: &'a str, revision: &'a str, filename: &'a str) -> Self {
        Self {
            kind: RepositoryKind::Dataset,
            ..Self::pinned(repository, revision, filename)
        }
    }

    #[must_use]
    pub fn exists(self) -> bool {
        self.path().is_file()
    }

    #[tracing::instrument(skip_all)]
    pub async fn resolve(self) -> anyhow::Result<PathBuf> {
        Store::file(self.path(), move |stage| async move {
            let client = client()?;
            let (owner, name) = split_id(self.repository);
            download::receive(self.filename, &stage, async {
                match self.kind {
                    RepositoryKind::Model => self.download(client.model(owner, name)).await,
                    RepositoryKind::Dataset => self.download(client.dataset(owner, name)).await,
                }
            })
            .await
        })
        .await
    }

    async fn download(
        self,
        repository: HFRepository<impl RepoType>,
    ) -> anyhow::Result<(Option<u64>, HFByteStream)> {
        repository
            .download_file_stream()
            .filename(self.filename)
            .revision(self.revision)
            .send()
            .await
            .map_err(Into::into)
    }

    fn path(self) -> PathBuf {
        Store::root()
            .join("hugging-face")
            .join(match self.kind {
                RepositoryKind::Model => "models",
                RepositoryKind::Dataset => "datasets",
            })
            .join(self.repository.replace(['/', '\\'], "--"))
            .join("snapshots")
            .join(self.revision)
            .join(self.filename)
    }
}
