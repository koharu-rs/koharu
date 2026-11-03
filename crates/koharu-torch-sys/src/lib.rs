//! The koharu-rs/torch shim's C API to libtorch, with libtorch's own C API
//! apart from its per-backend functions, resolved from the runtime's build
//! when `load` is called. Every header is copied verbatim from `SHIM`'s
//! `include.tar.gz`: the shim's own, whose C types are named so a binding
//! tells a dtype from a flag, and the libtorch C headers they include.

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    unpredictable_function_pointer_comparisons,
    clippy::all
)]

/// The koharu-rs/torch shim release the runtime loads.
pub const SHIM: &str = "2.15.0+koharu.2";

/// The koharu-rs/torch libtorch release `SHIM` was built against, which the
/// runtime loads beside it.
pub const LIBTORCH: &str = "2.15.0+libtorch.1";

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
