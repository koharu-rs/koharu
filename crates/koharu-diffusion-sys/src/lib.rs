//! stable-diffusion.cpp's C API, resolved from the runtime's build when `load`
//! is called. The header is copied verbatim from `COMMIT`.

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    unpredictable_function_pointer_comparisons,
    clippy::all
)]

/// The stable-diffusion.cpp commit the header comes from; the runtime checks
/// the library's `sd_commit()` against it.
pub const COMMIT: &str = "1d0a58a";

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
