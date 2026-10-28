use std::{env, path::PathBuf};

use anyhow::Result;

fn main() -> Result<()> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=upstream");
    koharu_bindgen::rewrite(
        "upstream/lib.rs",
        PathBuf::from(env::var("OUT_DIR")?).join("torch_sys.rs"),
    )
}
