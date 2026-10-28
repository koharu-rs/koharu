use std::{env, path::PathBuf};

use anyhow::Result;

/// The functions Koharu calls; bindgen brings the types they take.
const FUNCTIONS: &str = "^(sd_ctx_params_init|new_sd_ctx|free_sd_ctx|sd_ctx_supports_image_generation|sd_img_gen_params_init|generate_image|free_sd_images|sd_set_log_callback|sd_commit)$";

fn main() -> Result<()> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rerun-if-changed=include");

    let include = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?).join("include");
    koharu_bindgen::Bindings::new("wrapper.h")
        .configure(|builder| {
            builder
                .clang_arg(format!("-I{}", include.display()))
                .layout_tests(false)
                .allowlist_function(FUNCTIONS)
                .prepend_enum_name(false)
        })
        .write(PathBuf::from(env::var("OUT_DIR")?).join("bindings.rs"))
}
