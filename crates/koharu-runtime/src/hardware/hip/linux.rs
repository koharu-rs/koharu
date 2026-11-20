//! AMD discovery through the Linux KFD topology.

use std::{
    io,
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
/// one's own target, and the target ROCm addresses it by.
fn targets() -> io::Result<Vec<(Target, Target)>> {
    let mut targets = Vec::new();
    for node in nodes()? {
        match fs_err::read_to_string(node.join("properties")) {
            Ok(properties) => targets.extend(target(&properties, override_of(&node).as_deref())),
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

/// The `HSA_OVERRIDE_GFX_VERSION` ROCm applies to `node`: the node's own,
/// else the one for every node, as libhsakmt's `topology.c` reads them.
fn override_of(node: &Path) -> Option<String> {
    let id = node.file_name()?.to_str()?;
    [
        format!("HSA_OVERRIDE_GFX_VERSION_{id}"),
        "HSA_OVERRIDE_GFX_VERSION".to_owned(),
    ]
    .into_iter()
    .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
}

/// A node's own ROCm target and the one ROCm addresses it by under
/// `override_`, when the node is a GPU ROCm supports. ROCm loads every
/// library's kernels, libtorch's and AOTriton's among them, for the target it
/// addresses, so that is the one Koharu downloads and looks HIP's GPUs up by.
fn target(properties: &str, override_: Option<&str>) -> Option<(Target, Target)> {
    let mut simd_count = 0;
    let mut version = 0;
    for line in properties.lines() {
        let mut fields = line.split_whitespace();
        let (Some(key), Some(value), None) = (fields.next(), fields.next(), fields.next()) else {
            continue;
        };
        let Ok(value) = value.parse() else {
            continue;
        };
        match key {
            "simd_count" => simd_count = value,
            "gfx_target_version" => version = value,
            _ => {}
        }
    }
    if simd_count == 0 || version == 0 {
        return None;
    }
    let hardware = by_version(version)?;
    let addressed = match override_.and_then(|value| overridden(version, value)) {
        Some(version) => by_version(version)?,
        None => hardware,
    };
    Some((hardware, addressed))
}

fn by_version(version: i64) -> Option<Target> {
    Target::iter().find(|target| target.get_int("version") == Some(version))
}

/// The KFD version `value` overrides a GPU of `version` to, as libhsakmt
/// reads it: `major.minor.stepping` in decimal, within the GPU's major
/// version. One libhsakmt rejects stops ROCm from starting at all, which
/// the start then reports.
fn overridden(version: i64, value: &str) -> Option<i64> {
    let mut parts = value.split('.').map(|part| part.parse::<u8>().ok());
    let (Some(Some(major)), Some(Some(minor)), Some(Some(stepping)), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    let major = i64::from(major);
    (major <= 63 && major == version / 10_000)
        .then(|| major * 10_000 + i64::from(minor) * 100 + i64::from(stepping))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn honours_the_override_libhsakmt_accepts() {
        let rx_9060_xt = "simd_count 64\ngfx_target_version 120000\n";
        assert_eq!(
            target(rx_9060_xt, Some("12.0.1")),
            Some((Target::Gfx1200, Target::Gfx1201))
        );
        assert_eq!(
            target(rx_9060_xt, None),
            Some((Target::Gfx1200, Target::Gfx1200))
        );
        // Across a major version, or not three decimal numbers, libhsakmt
        // refuses it and ROCm doesn't start.
        for rejected in ["11.0.0", "12.0", "12.0.1.0", "gfx1201"] {
            assert_eq!(
                target(rx_9060_xt, Some(rejected)),
                Some((Target::Gfx1200, Target::Gfx1200))
            );
        }
    }
}
