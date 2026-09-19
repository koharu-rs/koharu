use anyhow::{Context, Result};
use serde::Deserialize;

use crate::network;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Platform {
    WindowsX64,
    LinuxX64,
    LinuxArm64,
}

impl Platform {
    pub(crate) fn host() -> Result<Self> {
        if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
            Ok(Self::WindowsX64)
        } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            Ok(Self::LinuxX64)
        } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
            Ok(Self::LinuxArm64)
        } else {
            anyhow::bail!("PyPI runtime packages do not support this target")
        }
    }

    fn accepts(self, filename: &str) -> bool {
        match self {
            Self::WindowsX64 => filename.contains("win_amd64"),
            Self::LinuxX64 => filename.contains("manylinux") && filename.contains("x86_64"),
            Self::LinuxArm64 => filename.contains("manylinux") && filename.contains("aarch64"),
        }
    }
}

#[derive(Deserialize)]
struct Metadata {
    urls: Vec<Distribution>,
}

#[derive(Deserialize)]
struct Distribution {
    filename: String,
    url: String,
}

pub(crate) async fn wheel(project: &str, platform: Platform) -> Result<String> {
    let url = format!("https://pypi.org/pypi/{project}/json");
    let client = network::http()?;
    let metadata: Metadata = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("failed to query {project}"))?
        .error_for_status()?
        .json()
        .await
        .with_context(|| format!("invalid PyPI metadata for {project}"))?;
    metadata
        .urls
        .into_iter()
        .find(|file| file.filename.ends_with(".whl") && platform.accepts(&file.filename))
        .map(|file| file.url)
        .with_context(|| format!("{project} has no compatible wheel"))
}
