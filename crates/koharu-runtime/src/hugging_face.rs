use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use reqwest::header::LOCATION;

use crate::{
    download::{self, Digest},
    network,
    store::Store,
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Repository {
    Model,
    Dataset,
}

/// A file on the Hugging Face Hub at a pinned commit.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct HuggingFaceFile<'a> {
    kind: Repository,
    repository: &'a str,
    revision: &'a str,
    filename: &'a str,
}

impl<'a> HuggingFaceFile<'a> {
    #[must_use]
    pub const fn pinned(repository: &'a str, revision: &'a str, filename: &'a str) -> Self {
        Self {
            kind: Repository::Model,
            repository,
            revision,
            filename,
        }
    }

    #[must_use]
    pub const fn pinned_dataset(repository: &'a str, revision: &'a str, filename: &'a str) -> Self {
        Self {
            kind: Repository::Dataset,
            ..Self::pinned(repository, revision, filename)
        }
    }

    #[must_use]
    pub fn exists(self) -> bool {
        self.path().is_file()
    }

    /// The file in the store, downloaded first if it isn't there yet.
    #[tracing::instrument(skip_all, fields(repository = self.repository, filename = self.filename))]
    pub async fn resolve(self) -> Result<PathBuf> {
        Store::file(self.path(), move |stage| async move {
            let (url, digest) = self.locate().await?;
            download::fetch(&url, &stage, &digest).await
        })
        .await
    }

    /// Where the Hub serves the file from, and the hash it records for it:
    /// `X-Linked-ETag` is the SHA-256 of a file in LFS and the git blob hash
    /// of one kept in git.
    async fn locate(self) -> Result<(String, Digest)> {
        let prefix = match self.kind {
            Repository::Model => "",
            Repository::Dataset => "datasets/",
        };
        let url = reqwest::Url::parse(&format!(
            "https://huggingface.co/{prefix}{}/resolve/{}/{}",
            self.repository, self.revision, self.filename
        ))?;
        let response = network::http_without_redirects()?
            .head(url.clone())
            .send()
            .await?
            .error_for_status()?;
        let etag = response
            .headers()
            .get("x-linked-etag")
            .with_context(|| format!("the Hub sent no hash for {url}"))?
            .to_str()?
            .trim_start_matches("W/")
            .trim_matches('"')
            .to_ascii_lowercase();
        let digest = match etag.len() {
            64 => Digest::Sha256(etag),
            40 => Digest::GitSha1(etag),
            _ => bail!("the Hub's hash for {url} is `{etag}`, which isn't one Koharu checks"),
        };
        let location = match response.headers().get(LOCATION) {
            Some(location) => url.join(location.to_str()?)?.to_string(),
            None => url.to_string(),
        };
        Ok((location, digest))
    }

    fn path(self) -> PathBuf {
        Store::root()
            .join("hugging-face")
            .join(match self.kind {
                Repository::Model => "models",
                Repository::Dataset => "datasets",
            })
            .join(self.repository.replace(['/', '\\'], "--"))
            .join("snapshots")
            .join(self.revision)
            .join(self.filename)
    }
}
