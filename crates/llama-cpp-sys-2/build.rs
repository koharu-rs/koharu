use std::{env, path::PathBuf};

use anyhow::Result;

// Upstream's bindgen settings, so every name and type `llama-cpp-2` uses is
// the same: llama-cpp-sys-2 0.1.160 build.rs.
fn main() -> Result<()> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rerun-if-changed=wrapper_mtmd.h");
    println!("cargo:rerun-if-changed=llama.cpp");

    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?).join("llama.cpp");
    let mtmd = env::var_os("CARGO_FEATURE_MTMD").is_some();
    koharu_bindgen::Bindings::new("wrapper.h")
        .configure(|builder| {
            let builder = builder
                .clang_arg(format!("-I{}", root.join("include").display()))
                .clang_arg(format!("-I{}", root.join("ggml/include").display()))
                .derive_partialeq(true)
                .allowlist_item("ggml_.*")
                .allowlist_item("gguf_.*")
                .allowlist_item("llama_.*")
                .allowlist_recursively(false)
                // Declared in ggml-cpu.h but not exported by llama.cpp's
                // shared build, and unused by `llama-cpp-2`.
                .blocklist_function("ggml_threadpool_get_n_threads")
                .raw_line("type FILE = ::std::os::raw::c_void;")
                .prepend_enum_name(false);
            if mtmd {
                builder.header("wrapper_mtmd.h").allowlist_item("mtmd_.*")
            } else {
                builder
            }
        })
        .write(PathBuf::from(env::var("OUT_DIR")?).join("bindings.rs"))
}
