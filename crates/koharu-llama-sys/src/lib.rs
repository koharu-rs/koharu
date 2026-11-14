//! llama.cpp's C API and the `llama_rs_` C wrapper of its common library,
//! resolved from the runtime's build when `load` is called. The headers are
//! copied verbatim: llama.cpp's from `RELEASE`, and the wrapper's from
//! koharu-rs/llama's `cmake/wrapper`, which builds into that release.

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    unpredictable_function_pointer_comparisons,
    clippy::all
)]

/// The llama.cpp release the headers come from.
pub const RELEASE: &str = "v1.0.0";

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
