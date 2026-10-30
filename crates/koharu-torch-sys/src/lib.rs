//! The koharu-rs/torch shim's C API to libtorch, resolved from the runtime's
//! build when `load` is called. The headers are copied verbatim from
//! `RELEASE`.

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    unpredictable_function_pointer_comparisons,
    clippy::all
)]

pub mod cuda;
pub mod io;
mod traits;

pub use traits::{DoubleList, IntList, IntListOption};

/// The koharu-rs/torch release the headers come from.
pub const RELEASE: &str = "v2.13.0.7";

#[repr(C)]
pub struct C_scalar {
    _private: [u8; 0],
}

#[repr(C)]
pub struct C_tensor {
    _private: [u8; 0],
}

#[repr(C)]
pub struct C_optimizer {
    _private: [u8; 0],
}

#[allow(clippy::upper_case_acronyms)]
#[repr(C)]
pub struct CIValue {
    _private: [u8; 0],
}

#[repr(C)]
pub struct CModule_ {
    _private: [u8; 0],
}

pub type tensor = *mut C_tensor;
pub type scalar = *mut C_scalar;
pub type optimizer = *mut C_optimizer;
pub type ivalue = *mut CIValue;
pub type torch_module = *mut CModule_;

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
