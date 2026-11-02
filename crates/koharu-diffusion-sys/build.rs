/// stable-diffusion.cpp's API: everything its header declares, but not the C
/// library functions of the system headers it includes, which the library
/// doesn't export.
const HEADER: &str = r".*[\\/]stable-diffusion\.h";

fn main() {
    koharu_bindgen::generate(|bindings| bindings.allowlist_file(HEADER).prepend_enum_name(false));
}
