use std::{env, path::PathBuf};

use anyhow::Result;

const HEADERS: [&str; 2] = ["libtch/torch_api.h", "libtch/torch_api_generated.h"];

fn main() -> Result<()> {
    println!("cargo:rerun-if-changed=build.rs");
    for header in HEADERS {
        println!("cargo:rerun-if-changed={header}");
    }

    koharu_bindgen::Bindings::new(HEADERS[0])
        .configure(|builder| {
            builder
                .header(HEADERS[1])
                .clang_arg("-Ilibtch")
                .layout_tests(false)
                // The shim's handles are opaque pointers, declared in lib.rs.
                .blocklist_type("^(tensor|scalar)$")
                .allowlist_function("^((at|ats|atc|atg)_.*|get_and_reset_last_err)$")
        })
        .write(PathBuf::from(env::var("OUT_DIR")?).join("bindings.rs"))
}
