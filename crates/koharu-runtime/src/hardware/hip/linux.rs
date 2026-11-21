//! AMD discovery through the Linux KFD topology.

use std::{
    env, io,
    path::{Path, PathBuf},
};

use strum::{EnumProperty, IntoEnumIterator};

use super::Target;
use crate::{Backend, Device};

const KFD_TOPOLOGY: &str = "/sys/class/kfd/kfd/topology/nodes";

pub(in crate::hardware) fn probe() -> Vec<Device> {
    let targets = targets().unwrap_or_else(|error| {
        tracing::warn!(?error, "failed to discover AMD GPUs");
        Vec::new()
    });
    targets
        .into_iter()
        .enumerate()
        .map(|(index, (hardware, addressed))| Device {
            name: format!("ROCm{index}"),
            index,
            description: addressed.to_string(),
            backend: Backend::Rocm {
                target: addressed.into(),
            },
            integrated: hardware.integrated(),
        })
        .collect()
}

/// The GPUs ROCm supports among the topology's nodes, in node order: each
/// one's own target, and the one ROCm addresses it by.
fn targets() -> io::Result<Vec<(Target, Target)>> {
    let mut targets = Vec::new();
    for node in nodes()? {
        match fs_err::read_to_string(node.join("properties")) {
            Ok(properties) => targets.extend(target(&node, &properties)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(targets)
}

/// The topology's node folders in the order `rocm-bootstrap` lists them, or
/// none without the KFD driver.
fn nodes() -> io::Result<Vec<PathBuf>> {
    let entries = match fs_err::read_dir(KFD_TOPOLOGY) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut nodes = Vec::new();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir()
            && entry.file_name().to_str().is_some_and(|name| {
                !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_digit())
            })
        {
            nodes.push(path);
        }
    }
    // `rocm-bootstrap` sorts KFD node paths lexically, not numerically.
    nodes.sort_by(|left, right| left.file_name().cmp(&right.file_name()));
    Ok(nodes)
}

/// A node's own target and the one ROCm addresses it by, which it loads every
/// library's kernels for, when the node is a GPU ROCm supports.
fn target(node: &Path, properties: &str) -> Option<(Target, Target)> {
    let property = |key: &str| {
        properties
            .lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix(' ')?.parse().ok())
    };
    if property("simd_count")? == 0 {
        return None;
    }
    let version = property("gfx_target_version")?;
    let by_version =
        |version| Target::iter().find(|target| target.get_int("version") == Some(version));
    let addressed = overridden(node, version).unwrap_or(version);
    Some((by_version(version)?, by_version(addressed)?))
}

/// The version `HSA_OVERRIDE_GFX_VERSION_<node>`, else
/// `HSA_OVERRIDE_GFX_VERSION`, has ROCm address a GPU of `version` by, as
/// libhsakmt's `topology.c` reads them: `major.minor.stepping` in decimal,
/// within the GPU's major version. One libhsakmt rejects stops ROCm from
/// starting at all, which the start then reports.
fn overridden(node: &Path, version: i64) -> Option<i64> {
    let id = node.file_name()?.to_str()?;
    let value = [
        format!("HSA_OVERRIDE_GFX_VERSION_{id}"),
        "HSA_OVERRIDE_GFX_VERSION".to_owned(),
    ]
    .into_iter()
    .find_map(|name| env::var(name).ok().filter(|value| !value.is_empty()))?;
    let parts: Vec<u8> = value
        .split('.')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    let &[major, minor, stepping] = parts.as_slice() else {
        return None;
    };
    let [major, minor, stepping] = [major, minor, stepping].map(i64::from);
    (major <= 63 && major == version / 10_000).then_some(major * 10_000 + minor * 100 + stepping)
}
