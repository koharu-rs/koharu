/// llama.cpp's, ggml's, and mtmd's public API; the wrapper's `llama_rs_` names
/// fall under `llama_`.
const FUNCTIONS: &str = "^(ggml|gguf|llama|mtmd)_.*";
const TYPES: &str = "^(ggml|gguf|llama|mtmd)_.*";
const VARIABLES: &str = "^(GGML|GGUF|LLAMA|MTMD)_.*";

fn main() {
    koharu_bindgen::generate(|bindings| {
        bindings
            .allowlist_function(FUNCTIONS)
            // ggml-cpu.h declares it, but llama.cpp defines it nowhere, so no
            // build exports it and `load` would always fail.
            .blocklist_function("ggml_threadpool_get_n_threads")
            .allowlist_type(TYPES)
            .allowlist_var(VARIABLES)
            .derive_partialeq(true)
            .prepend_enum_name(false)
    });
}
