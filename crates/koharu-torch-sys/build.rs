use std::{env, path::Path};

use anyhow::Result;
use koharu_bindgen::Generator;

const SHIM_LIBRARY_NAME: &str = "koharu-torch";
const OPAQUE_TYPES: &str = "^(tensor|scalar|optimizer|torch_module|ivalue)$";
const TORCH_API_HEADER: &str = "libtch/torch_api.h";
const TORCH_API_GENERATED_HEADER: &str = "libtch/torch_api_generated.h";
const RERUN_IF_CHANGED: &[&str] = &["build.rs", TORCH_API_HEADER, TORCH_API_GENERATED_HEADER];

fn main() -> Result<()> {
    for path in RERUN_IF_CHANGED {
        println!("cargo:rerun-if-changed={path}");
    }
    generate_bindings(Path::new(&env::var("OUT_DIR")?))
}

fn generate_bindings(out_dir: &Path) -> Result<()> {
    generator(TORCH_API_HEADER)
        .with_bindgen(|builder| {
            builder
                .allowlist_function("^(at.*|ato.*|ats.*|atc.*|atm.*|ati.*|get_and_reset_last_err)$")
                .blocklist_function("^at_autocast_(is_enabled|set_enabled)$")
        })
        .write_to_file(out_dir.join("torch_api.rs"))?;

    generator(TORCH_API_GENERATED_HEADER)
        .with_bindgen(|builder| builder.clang_arg("-Ilibtch").allowlist_function("^atg_.*"))
        .write_to_file(out_dir.join("torch_api_generated.rs"))?;

    Ok(())
}

fn generator(header: impl AsRef<Path>) -> Generator {
    Generator::from_header(header, SHIM_LIBRARY_NAME)
        .with_bindgen(|builder| builder.layout_tests(false).blocklist_type(OPAQUE_TYPES))
}
