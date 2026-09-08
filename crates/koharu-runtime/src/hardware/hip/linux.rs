//! AMD discovery through the Linux KFD topology.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use strum::{EnumProperty, IntoEnumIterator};

use super::GfxTarget;
use crate::{Backend, Device, DeviceType};

const KFD_TOPOLOGY: &str = "/sys/class/kfd/kfd/topology/nodes";

#[derive(Debug, thiserror::Error)]
pub(super) enum ProbeError {
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

pub(super) fn probe() -> Result<Vec<Device>, ProbeError> {
    let topology = Path::new(KFD_TOPOLOGY);
    if !topology.is_dir() {
        return Ok(Vec::new());
    }

    let entries = fs::read_dir(topology).map_err(|source| ProbeError::Io {
        path: topology.to_owned(),
        source,
    })?;
    let mut nodes = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| ProbeError::Io {
            path: topology.to_owned(),
            source,
        })?;
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

    let mut devices = Vec::new();
    for node in nodes {
        let path = node.join("properties");
        let properties = match fs::read_to_string(&path) {
            Ok(properties) => properties,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(source) => return Err(ProbeError::Io { path, source }),
        };

        let mut simd_count = 0;
        let mut version = 0;
        for line in properties.lines() {
            let mut fields = line.split_whitespace();
            let (Some(key), Some(value), None) = (fields.next(), fields.next(), fields.next())
            else {
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
            continue;
        }

        let Some(target) = GfxTarget::iter().find(|target| *target as i64 == version) else {
            tracing::warn!(version, path = %path.display(), "skipping unknown AMD architecture");
            continue;
        };
        let index = devices.len();
        devices.push(Device {
            index,
            name: format!("ROCm{index}"),
            description: target.to_string(),
            backend: Backend::Rocm,
            device_type: if target.get_str("integrated").is_some() {
                DeviceType::IntegratedGpu
            } else {
                DeviceType::Gpu
            },
            memory_total: 0,
            memory_free: 0,
            compute_capability: 0,
            target: Some(target),
        });
    }
    Ok(devices)
}
