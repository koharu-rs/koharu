//! libtorch's C API, through Koharu's shim, resolved from the runtime's build
//! when `load` is called. The headers are the shim's own, copied from the
//! `koharu-rs/torch` release the runtime installs.

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    unpredictable_function_pointer_comparisons,
    clippy::all
)]

mod traits;

pub use traits::{DoubleList, IntList, IntListOption};

#[repr(C)]
pub struct C_scalar {
    _private: [u8; 0],
}

#[repr(C)]
pub struct C_tensor {
    _private: [u8; 0],
}

pub type tensor = *mut C_tensor;
pub type scalar = *mut C_scalar;

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
