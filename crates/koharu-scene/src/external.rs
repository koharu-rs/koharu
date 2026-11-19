use std::{
    io::{self, Read as _},
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::{Error, Hash, Project, Result, Size};

#[cfg(windows)]
mod mapped;

pub const MAX_SOURCE_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Type)]
#[serde(deny_unknown_fields)]
pub struct ArchiveMember {
    pub index: usize,
    pub name: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Type)]
#[serde(deny_unknown_fields)]
pub struct ExternalImage {
    pub path: PathBuf,
    pub member: Option<ArchiveMember>,
    pub size: Size,
    pub bytes: u64,
}

impl Project {
    pub fn external_image(&self, hash: &Hash) -> Option<&ExternalImage> {
        self.external.get(hash)
    }

    pub fn external_images(&self) -> impl Iterator<Item = (Hash, &ExternalImage)> {
        let hashes: std::collections::BTreeSet<_> = self
            .pages()
            .filter_map(|page| page.background.map(|background| background.file))
            .collect();
        hashes
            .into_iter()
            .filter_map(|hash| self.external_image(&hash).map(|source| (hash, source)))
    }
}

impl ExternalImage {
    pub fn check_location(&self) -> Result<()> {
        self.source_path()
            .map(|_| ())
            .map_err(|error| self.unavailable(error))
    }

    pub fn read(&self, project: &Path, hash: &Hash) -> Result<Vec<u8>> {
        self.read_checked(project, hash)
            .map_err(|error| self.unavailable(error))
    }

    fn unavailable(&self, error: io::Error) -> Error {
        Error::ExternalImage {
            path: self.path.clone(),
            reason: error.to_string(),
        }
    }

    fn source_path(&self) -> io::Result<PathBuf> {
        use std::path::Prefix;

        match self.path.components().next() {
            Some(Component::Prefix(prefix)) => match prefix.kind() {
                Prefix::Disk(_) | Prefix::VerbatimDisk(_) if self.path.is_absolute() => {
                    Ok(self.path.clone())
                }
                #[cfg(windows)]
                Prefix::UNC(..) | Prefix::VerbatimUNC(..) => mapped::resolve(&self.path)
                    .ok_or_else(|| {
                        io::Error::other("the linked network folder needs a mapped drive")
                    }),
                _ => Err(io::Error::other("unsupported linked source path")),
            },
            _ if self.path.is_absolute()
                || self
                    .path
                    .components()
                    .all(|part| matches!(part, Component::Normal(_))) =>
            {
                Ok(self.path.clone())
            }
            _ => Err(io::Error::other("invalid relative source path")),
        }
    }

    fn read_checked(&self, project: &Path, hash: &Hash) -> io::Result<Vec<u8>> {
        let extension = self.path.extension().and_then(|value| value.to_str());
        let expected = match self.member {
            Some(_) => &["zip", "cbz"][..],
            None => &["png", "jpg", "jpeg", "webp"][..],
        };
        if !extension.is_some_and(|value| {
            expected
                .iter()
                .any(|allowed| value.eq_ignore_ascii_case(allowed))
        }) {
            return Err(io::Error::other("the linked file type is not supported"));
        }
        let source = self.source_path()?;
        let parent = project.parent().unwrap_or_else(|| Path::new("."));
        let path = if source.is_absolute() {
            source.canonicalize()?
        } else {
            let path = parent.join(source).canonicalize()?;
            if !path.starts_with(parent.canonicalize()?) {
                return Err(io::Error::other(
                    "relative source is outside the project directory",
                ));
            }
            path
        };
        if !fs_err::metadata(&path)?.is_file() || self.bytes == 0 || self.bytes > MAX_SOURCE_BYTES {
            return Err(io::Error::other("invalid source file or image size"));
        }
        let mut file = fs_err::File::open(&path)?;
        let mut bytes = Vec::new();
        match &self.member {
            Some(member) => {
                let mut archive = zip::ZipArchive::new(file).map_err(io::Error::other)?;
                let entry = archive.by_index(member.index).map_err(io::Error::other)?;
                if entry.is_dir()
                    || entry.name().map_err(io::Error::other)? != member.name
                    || entry.size() != self.bytes
                {
                    return Err(io::Error::other("the archive entry changed"));
                }
                entry.take(self.bytes + 1).read_to_end(&mut bytes)?;
            }
            None => {
                if file.metadata()?.len() != self.bytes {
                    return Err(io::Error::other("the source image size changed"));
                }
                (&mut file).take(self.bytes + 1).read_to_end(&mut bytes)?;
            }
        }
        if bytes.len() as u64 != self.bytes || Hash::of(&bytes) != *hash {
            return Err(io::Error::other("the source image contents changed"));
        }
        Ok(bytes)
    }
}
