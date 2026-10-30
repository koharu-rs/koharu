//! llama.cpp's C API, resolved from the runtime's build when `load` is called.
//! The headers are copied verbatim from `RELEASE`.

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    unpredictable_function_pointer_comparisons,
    clippy::all
)]

/// The llama.cpp release the headers come from.
pub const RELEASE: &str = "b12001";

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
