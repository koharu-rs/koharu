//! AMD discovery through the Linux KFD topology.

use std::{fs, io, path::PathBuf};

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
        .map(|(index, target)| Device {
            name: format!("ROCm{index}"),
            index,
            description: target.to_string(),
            backend: Backend::Rocm {
                target: target.into(),
            },
            integrated: target.integrated(),
        })
        .collect()
}

/// The targets of the GPUs ROCm supports among the topology's nodes, in
/// node order.
fn targets() -> io::Result<Vec<Target>> {
    let mut targets = Vec::new();
    for node in nodes()? {
        match fs::read_to_string(node.join("properties")) {
            Ok(properties) => targets.extend(target(&properties)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(targets)
}

/// The topology's node folders in the order `rocm-bootstrap` lists them, or
/// none without the KFD driver.
fn nodes() -> io::Result<Vec<PathBuf>> {
    let entries = match fs::read_dir(KFD_TOPOLOGY) {
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

/// The ROCm target a node's properties name, when the node is a GPU ROCm
/// supports.
fn target(properties: &str) -> Option<Target> {
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
    Target::iter().find(|target| target.get_int("version") == Some(version))
}
