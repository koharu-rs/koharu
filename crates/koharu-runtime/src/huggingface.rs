use std::path::PathBuf;

use anyhow::Result;

use crate::{store, transfer};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Kind {
    Model,
    Dataset,
}

/// A Hugging Face model or dataset at a pinned commit.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Repository {
    kind: Kind,
    id: &'static str,
    commit: &'static str,
}

impl Repository {
    #[must_use]
    pub const fn model(id: &'static str, commit: &'static str) -> Self {
        Self::pinned(Kind::Model, id, commit)
    }

    #[must_use]
    pub const fn dataset(id: &'static str, commit: &'static str) -> Self {
        Self::pinned(Kind::Dataset, id, commit)
    }

    /// A repository is pinned only by a full commit hash, which a constant
    /// checks as it builds.
    const fn pinned(kind: Kind, id: &'static str, commit: &'static str) -> Self {
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
            let prefix = match self.kind {
                Kind::Model => "",
                Kind::Dataset => "datasets/",
            };
            let url = format!(
                "https://huggingface.co/{prefix}{}/resolve/{}/{name}",
                self.id, self.commit
            );
            store::file(path.clone(), move |stage| async move {
                transfer::fetch(&url, &stage).await
            })
            .await?;
        }
        Ok(paths)
    }

    /// Whether the store has the file `name`, without going over the network.
    #[must_use]
    pub fn exists(&self, name: &str) -> bool {
        self.path(name).is_file()
    }

    fn path(&self, name: &str) -> PathBuf {
        store::root()
            .join("huggingface")
            .join(match self.kind {
                Kind::Model => "models",
                Kind::Dataset => "datasets",
            })
            .join(self.id.replace(['/', '\\'], "--"))
            .join("snapshots")
            .join(self.commit)
            .join(name)
    }
}
