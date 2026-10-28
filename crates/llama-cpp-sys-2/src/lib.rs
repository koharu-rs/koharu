//! llama.cpp's C API for `llama-cpp-2`, resolved from the runtime's build of
//! llama.cpp when `load` is called. The README is the design.

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    unpredictable_function_pointer_comparisons,
    clippy::all
)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
