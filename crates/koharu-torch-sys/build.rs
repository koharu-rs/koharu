/// The shim's functions, and libtorch's own C functions that make, inspect,
/// and delete the tensors, strings, and generators the shim passes, and read
/// its last error.
const FUNCTIONS: &str = "^(koharu_torch_.*|aoti_torch_(new_tensor_handle|delete_tensor_object|create_tensor_from_blob_v2|copy_|get_(data_ptr|dim|numel|sizes|strides|dtype|device_type|device_index)|is_contiguous|grad_mode_(is_enabled|set_enabled))|torch_exception_get_what_without_backtrace|torch_string_(c_str|length)|torch_delete_(string|generator))$";

fn main() {
    koharu_bindgen::generate(|bindings| {
        bindings
            // libtorch's C headers are C++ underneath: `using` aliases and
            // its header-only types.
            .clang_args(["-x", "c++", "-std=c++17"])
            .allowlist_function(FUNCTIONS)
            .allowlist_var("AOTI_TORCH_SUCCESS")
            .prepend_enum_name(false)
    });
    // koharu-torch generates its tensor methods from these bindings.
    let out = std::env::var("OUT_DIR").expect("Cargo sets it");
    println!("cargo::metadata=bindings={out}/bindings.rs");
}
