//! The shelf: the series the user opened, the last first, in a JSON file.

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use koharu_extensions::SeriesLink;

pub struct Shelf {
    path: PathBuf,
}

impl Shelf {
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn list(&self) -> Result<Vec<SeriesLink>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        serde_json::from_slice(&fs_err::read(&self.path)?)
            .with_context(|| format!("{} isn't valid", self.path.display()))
    }

    /// Puts `series` first.
    pub fn put(&self, series: SeriesLink) -> Result<()> {
        self.edit(|shelf| {
            shelf.retain(|kept| kept.url != series.url);
            shelf.insert(0, series);
        })
    }

    pub fn remove(&self, url: &str) -> Result<()> {
        self.edit(|shelf| shelf.retain(|kept| kept.url != url))
    }

    fn edit(&self, edit: impl FnOnce(&mut Vec<SeriesLink>)) -> Result<()> {
        let mut shelf = self.list()?;
        edit(&mut shelf);
        Ok(fs_err::write(
            &self.path,
            serde_json::to_vec_pretty(&shelf)?,
        )?)
    }
}
