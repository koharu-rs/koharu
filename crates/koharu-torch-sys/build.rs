/// Every shim function starts with `at`.
const FUNCTIONS: &str = "^(at.*|get_and_reset_last_err)$";
/// The shim's handles, which `src/lib.rs` declares as opaque pointers rather
/// than bindgen's `*mut c_void`.
const HANDLES: &str = "^(tensor|scalar|optimizer|torch_module|ivalue)$";

fn main() {
    koharu_bindgen::generate(|bindings| {
        bindings
            .allowlist_function(FUNCTIONS)
            .blocklist_type(HANDLES)
    });
}
