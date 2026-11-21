//! The projects the start view lists: `.khr` files in one folder.

use std::{path::PathBuf, time::SystemTime};

use anyhow::{Context as _, Result, bail, ensure};
use koharu_scene::{EXTENSION, Session};
use sanitize_filename::OptionsForCheck;

/// The projects folder.
pub struct Library {
    root: PathBuf,
}

impl Library {
    pub fn new(root: PathBuf) -> Result<Self> {
        fs_err::create_dir_all(&root)?;
        Ok(Self { root })
    }

    /// The projects' names, the last changed first; two changed at once in
    /// name order, numbers by value.
    pub fn list(&self) -> Result<Vec<String>> {
        let mut projects: Vec<(Option<SystemTime>, String)> = fs_err::read_dir(&self.root)?
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .filter_map(|entry| {
                let path = entry.path();
                let name = path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case(EXTENSION))
                    .then(|| Some(path.file_stem()?.to_str()?.to_owned()))
                    .flatten()?;
                let changed = entry.metadata().and_then(|metadata| metadata.modified());
                Some((changed.ok(), name))
            })
            .collect();
        projects.sort_by(|(a_changed, a), (b_changed, b)| {
            b_changed
                .cmp(a_changed)
                .then_with(|| alphanumeric_sort::compare_str(a, b))
        });
        Ok(projects.into_iter().map(|(_, name)| name).collect())
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
