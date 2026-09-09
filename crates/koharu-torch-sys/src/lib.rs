#![allow(
    clippy::all,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals
)]

pub mod c_generated;
pub mod cuda;
pub mod io;
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

include!(concat!(env!("OUT_DIR"), "/torch_api.rs"));

pub unsafe fn at_autocast_is_enabled() -> ::std::os::raw::c_int {
    type FnPtr = unsafe extern "C" fn() -> bool;
    static FN: ::std::sync::OnceLock<FnPtr> = ::std::sync::OnceLock::new();
    let function =
        FN.get_or_init(|| unsafe { __koharu_bindgen_load::<FnPtr>(b"at_autocast_is_enabled\0") });
    unsafe { i32::from(function()) }
}

pub unsafe fn at_autocast_set_enabled(b: ::std::os::raw::c_int) -> ::std::os::raw::c_int {
    type FnPtr = unsafe extern "C" fn(bool) -> bool;
    static FN: ::std::sync::OnceLock<FnPtr> = ::std::sync::OnceLock::new();
    let function =
        FN.get_or_init(|| unsafe { __koharu_bindgen_load::<FnPtr>(b"at_autocast_set_enabled\0") });
    unsafe { i32::from(function(b != 0)) }
}
