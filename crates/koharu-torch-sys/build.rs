use std::env;

/// The shim's functions and libtorch's own C API.
const FUNCTIONS: &str = "^(aten|aoti_torch|torch)_.*";
/// libtorch's per-backend C functions, which its headers declare whatever
/// the build, but only some builds export: `aoti_torch_mps_*` only a Metal
/// one, and `aoti_torch_cpu_*` only one with MKL and oneDNN.
const BACKENDS: &str = "^aoti_torch_(mps|cpu)_.*";

fn main() {
    koharu_bindgen::generate(|bindings| {
        bindings
            // libtorch's C headers are C++ underneath: `using` aliases and
            // its header-only types.
            .clang_args(["-x", "c++", "-std=c++17"])
            .allowlist_function(FUNCTIONS)
            .blocklist_function(BACKENDS)
            .allowlist_var("AOTI_TORCH_SUCCESS")
            .prepend_enum_name(false)
    });
    // koharu-torch generates its tensor methods from the bindings, and from
    // the header the shim generates its operator functions into.
    let out = env::var("OUT_DIR").expect("Cargo sets it");
    println!("cargo::metadata=bindings={out}/bindings.rs");
    let crate_dir = env::var("CARGO_MANIFEST_DIR").expect("Cargo sets it");
    println!("cargo::metadata=operators={crate_dir}/include/koharu_torch_aten.h");
}
