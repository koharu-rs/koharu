//! The projects the start view lists: `.khr` files in one folder.

use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use koharu_scene::{EXTENSION, Session};
use serde::Serialize;
use specta::Type;

#[derive(Clone, Debug, Serialize, Type)]
pub struct ProjectSummary {
    pub name: String,
}

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

    /// The projects in name order, numbers by value.
    pub fn list(&self) -> Result<Vec<ProjectSummary>> {
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
        Ok(names
            .into_iter()
            .map(|name| ProjectSummary { name })
            .collect())
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

    /// Deletes a project's file, and SQLite's write-ahead log beside it if
    /// one was left.
    pub fn delete(&self, name: &str) -> Result<()> {
        let path = self.path(name)?;
        std::fs::remove_file(&path)
            .with_context(|| format!("failed to delete {}", path.display()))?;
        for suffix in ["-wal", "-shm"] {
            let mut sidecar = path.clone().into_os_string();
            sidecar.push(suffix);
            match std::fs::remove_file(&sidecar) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    return Err(error).with_context(|| format!("failed to delete {sidecar:?}"));
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// A project's file, for a name that is a file name on every platform.
    fn path(&self, name: &str) -> Result<PathBuf> {
        if name.trim() != name || name.is_empty() {
            bail!("a project name can't be empty or start or end with a space");
        }
        if name.ends_with('.')
            || name
                .chars()
                .any(|character| character.is_control() || r#"<>:"/\|?*"#.contains(character))
        {
            bail!("a project name can't hold characters a file name can't");
        }
        let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
        let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || stem
                .strip_prefix("COM")
                .or_else(|| stem.strip_prefix("LPT"))
                .is_some_and(|number| {
                    matches!(number, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
                });
        if reserved {
            bail!("{name:?} is a name Windows reserves");
        }
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
        let names: Vec<String> = library
            .list()?
            .into_iter()
            .map(|project| project.name)
            .collect();
        assert_eq!(names, ["chapter 9", "chapter 10"]);

        assert!(library.create("chapter 9").is_err());
        assert!(library.create("a/b").is_err());
        assert!(library.create("CON").is_err());

        let session = library.open("chapter 9")?;
        assert_eq!(session.project().pages().count(), 0);
        drop(session);

        library.delete("chapter 9")?;
        let names: Vec<String> = library
            .list()?
            .into_iter()
            .map(|project| project.name)
            .collect();
        assert_eq!(names, ["chapter 10"]);
        assert!(library.open("chapter 9").is_err());
        Ok(())
    }
}
