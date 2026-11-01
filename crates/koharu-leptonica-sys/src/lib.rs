//! Leptonica's C API, compiled from `leptonica/` and `include/` and linked
//! statically. Both are copied verbatim from release 1.87.0 (commit 13275a2).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::all
)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
