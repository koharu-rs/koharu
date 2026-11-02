//! The koharu-rs/torch shim's C API to libtorch, with libtorch's own C API
//! apart from its per-backend functions, resolved from the runtime's build
//! when `load` is called. libtorch's C headers are copied verbatim from
//! `RELEASE`. `koharu_torch.h` and `koharu_torch_aten.h` come from
//! koharu-rs/torch 7de8ef4, which generates the functions `RELEASE` exports
//! with the same C types, named so a binding tells a dtype from a flag.

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    unpredictable_function_pointer_comparisons,
    clippy::all
)]

/// The koharu-rs/torch release the runtime loads.
pub const RELEASE: &str = "v2.15.0";

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
