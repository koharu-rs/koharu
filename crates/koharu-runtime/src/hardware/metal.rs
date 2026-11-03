use crate::{Backend, Device};

/// Apple silicon's GPU, which every Mac Koharu runs on has.
pub(super) fn probe() -> Device {
    Device {
        name: "MTL0".to_owned(),
        description: "Apple silicon".to_owned(),
        backend: Backend::Metal,
        index: 0,
        integrated: true,
    }
}
