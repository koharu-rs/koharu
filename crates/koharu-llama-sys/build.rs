/// llama.cpp's, ggml's, and mtmd's public API.
const FUNCTIONS: &str = "^(ggml|gguf|llama|mtmd)_.*";
const TYPES: &str = "^(ggml|gguf|llama|mtmd)_.*";
const VARIABLES: &str = "^(GGML|GGUF|LLAMA|MTMD)_.*";

fn main() {
    koharu_bindgen::generate(|bindings| {
        bindings
            .allowlist_function(FUNCTIONS)
            .allowlist_type(TYPES)
            .allowlist_var(VARIABLES)
            .derive_partialeq(true)
            .prepend_enum_name(false)
    });
}
