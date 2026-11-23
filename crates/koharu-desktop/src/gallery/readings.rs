//! How each chapter read into a project reads, under the project's name, in a
//! JSON file, so a chapter opened again turns and pairs its pages as its site
//! said.

use std::{collections::BTreeMap, path::PathBuf};

use anyhow::{Context as _, Result};
use koharu_extensions::{PageSide, ReadingDirection};
use serde::{Deserialize, Serialize};
use specta::Type;

/// How a chapter's pages read, as its extension said.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Reading {
    /// How its pages turn.
    pub direction: Option<ReadingDirection>,
    /// Which side of a two-page spread its first page takes.
    pub first_page: Option<PageSide>,
}

pub struct Readings {
    path: PathBuf,
}

impl Readings {
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// How the chapter in `project` reads, when one was read into it.
    pub fn get(&self, project: &str) -> Result<Option<Reading>> {
        Ok(self.list()?.remove(project))
    }

    pub fn put(&self, project: &str, reading: Reading) -> Result<()> {
        let mut readings = self.list()?;
        readings.insert(project.to_owned(), reading);
        Ok(fs_err::write(
            &self.path,
            serde_json::to_vec_pretty(&readings)?,
        )?)
    }

    fn list(&self) -> Result<BTreeMap<String, Reading>> {
        if !self.path.exists() {
            return Ok(BTreeMap::new());
        }
        serde_json::from_slice(&fs_err::read(&self.path)?)
            .with_context(|| format!("{} isn't valid", self.path.display()))
    }
}
