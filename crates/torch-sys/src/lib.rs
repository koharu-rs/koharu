//! torch-sys's declarations for `tch`, resolved from the koharu-rs/torch shim
//! when `load` is called. The README is the design.

#![allow(
    non_camel_case_types,
    non_snake_case,
    unpredictable_function_pointer_comparisons,
    clippy::all
)]

include!(concat!(env!("OUT_DIR"), "/torch_sys.rs"));
