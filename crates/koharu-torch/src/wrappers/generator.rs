//! Random number generators.

use koharu_torch_sys::{
    AtenGeneratorHandle, aten_create_generator, aten_generator_set_seed, torch_delete_generator,
};

use super::utils::check;
use crate::{Device, Result};

/// A random number generator for one device, as torch.Generator makes, for
/// the operators that take one.
pub struct Generator {
    pub(crate) c_generator: AtenGeneratorHandle,
}

// libtorch locks a generator's own mutex around its use.
unsafe impl Send for Generator {}

impl Generator {
    pub fn new(device: Device) -> Result<Generator> {
        let (device_type, index) = device.c_parts();
        let mut c_generator = std::ptr::null_mut();
        check(unsafe { aten_create_generator(device_type, index, &mut c_generator) })?;
        Ok(Generator { c_generator })
    }

    pub fn set_seed(&self, seed: u64) -> Result<()> {
        check(unsafe { aten_generator_set_seed(self.c_generator, seed) })
    }
}

impl Drop for Generator {
    fn drop(&mut self) {
        unsafe { torch_delete_generator(self.c_generator) };
    }
}
