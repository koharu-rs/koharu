//! The projects the start view lists: `.khr` files in one folder.

use std::path::PathBuf;

use anyhow::{Context as _, Result, bail, ensure};
use koharu_scene::{EXTENSION, Session};
use sanitize_filename::OptionsForCheck;

/// The projects folder, `Documents/Koharu`.
pub struct Library {
    root: PathBuf,
}

impl Library {
    pub fn new(root: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&root)
            .with_context(|| format!("failed to create {}", root.display()))?;
        Ok(Self { root })
    }

    /// The projects' names in name order, numbers by value.
    pub fn list(&self) -> Result<Vec<String>> {
        let mut names: Vec<String> = std::fs::read_dir(&self.root)
            .with_context(|| format!("failed to read {}", self.root.display()))?
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .filter_map(|entry| {
                let path = entry.path();
                path.extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case(EXTENSION))
                    .then(|| Some(path.file_stem()?.to_str()?.to_owned()))
                    .flatten()
            })
            .collect();
        names.sort_by(|a, b| alphanumeric_sort::compare_str(a, b));
        Ok(names)
    }

    pub fn create(&self, name: &str) -> Result<Session> {
        let path = self.path(name)?;
        if path.exists() {
            bail!("a project named {name:?} already exists");
        }
        Session::create(&path).with_context(|| format!("failed to create {}", path.display()))
    }

    pub fn open(&self, name: &str) -> Result<Session> {
        let path = self.path(name)?;
        Session::open(&path).with_context(|| format!("failed to open {}", path.display()))
    }

    pub fn delete(&self, name: &str) -> Result<()> {
        let path = self.path(name)?;
        Session::remove(&path).with_context(|| format!("failed to delete {}", path.display()))
    }

    /// A project's file, for a name that is a file name on every platform.
    fn path(&self, name: &str) -> Result<PathBuf> {
        ensure!(!name.trim().is_empty(), "a project name can't be empty");
        let options = OptionsForCheck {
            windows: true,
            truncate: true,
        };
        ensure!(
            sanitize_filename::is_sanitized_with_options(name, options),
            "{name:?} can't be a file name"
        );
        Ok(self.root.join(format!("{name}.{EXTENSION}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_library_lists_creates_opens_and_deletes_projects() -> Result<()> {
        let root = tempfile::tempdir()?;
        let library = Library::new(root.path().join("Koharu"))?;
        assert!(library.list()?.is_empty());

        drop(library.create("chapter 10")?);
        drop(library.create("chapter 9")?);
        assert_eq!(library.list()?, ["chapter 9", "chapter 10"]);

        assert!(library.create("chapter 9").is_err());
        assert!(library.create("a/b").is_err());
        assert!(library.create("CON").is_err());
        assert!(library.create("trailing.").is_err());

        let session = library.open("chapter 9")?;
        assert_eq!(session.project().pages().count(), 0);
        drop(session);

        library.delete("chapter 9")?;
        assert_eq!(library.list()?, ["chapter 10"]);
        assert!(library.open("chapter 9").is_err());
        Ok(())
    }
}
