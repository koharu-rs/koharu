//! AMD discovery through the Linux KFD topology.

use std::{fs, io, path::Path};

use anyhow::Result;
use strum::{EnumProperty, IntoEnumIterator};

use super::Target;
use crate::{Backend, Device};

const KFD_TOPOLOGY: &str = "/sys/class/kfd/kfd/topology/nodes";

pub(super) fn probe() -> Result<Vec<Device>> {
    let topology = Path::new(KFD_TOPOLOGY);
    if !topology.is_dir() {
        return Ok(Vec::new());
    }

    let entries = fs::read_dir(topology)?;
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

    let mut devices = Vec::new();
    for node in nodes {
        let path = node.join("properties");
        let properties = match fs::read_to_string(&path) {
            Ok(properties) => properties,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
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

        let Some(target) = Target::iter().find(|target| target.get_int("version") == Some(version))
        else {
            continue;
        };
        let name = target.to_string();
        let index = devices.len();
        devices.push(Device {
            index,
            name: format!("ROCm{index}"),
            description: name.clone(),
            backend: Backend::Rocm,
            device_type: target.device_type(),
            memory_total: 0,
            memory_free: 0,
            compute_capability: 0,
            target: Some(name),
        });
    }
    Ok(devices)
}
