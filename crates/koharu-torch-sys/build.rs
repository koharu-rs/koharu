use std::{cell::RefCell, env, ffi::OsStr, fs, path::Path, rc::Rc};

use bindgen::callbacks::{DiscoveredItem, DiscoveredItemId, ParseCallbacks, SourceLocation};

/// The shim's functions and libtorch's own C API.
const FUNCTIONS: &str = "^(aten|aoti_torch|torch)_.*";
/// libtorch's per-backend C functions, which its headers declare whatever
/// the build, but only some builds export: `aoti_torch_mps_*` only a Metal
/// one, and `aoti_torch_cpu_*` only one with MKL and oneDNN.
const BACKENDS: &str = "^aoti_torch_(mps|cpu)_.*";
/// The header the shim generates its operator functions into. They share
/// the `aten_` prefix with its hand-written functions, so only the header
/// tells koharu-torch which to make tensor methods of.
const OPERATORS: &str = "koharu_torch_aten.h";

/// The functions bindgen binds from `OPERATORS`.
#[derive(Debug)]
struct Operators(Rc<RefCell<Vec<String>>>);

impl ParseCallbacks for Operators {
    fn new_item_found(
        &self,
        _id: DiscoveredItemId,
        item: DiscoveredItem,
        location: Option<&SourceLocation>,
    ) {
        let file = location.and_then(|location| location.file_name.as_deref());
        if let DiscoveredItem::Function { final_name } = item
            && file.and_then(|file| Path::new(file).file_name()) == Some(OsStr::new(OPERATORS))
        {
            self.0.borrow_mut().push(final_name);
        }
    }
}

fn main() {
    let operators = Rc::new(RefCell::new(Vec::new()));
    koharu_bindgen::generate(|bindings| {
        bindings
            // libtorch's C headers are C++ underneath: `using` aliases and
            // its header-only types.
            .clang_args(["-x", "c++", "-std=c++17"])
            .allowlist_function(FUNCTIONS)
            .blocklist_function(BACKENDS)
            .allowlist_var("AOTI_TORCH_SUCCESS")
            .prepend_enum_name(false)
            .parse_callbacks(Box::new(Operators(operators.clone())))
    });
    // koharu-torch generates its tensor methods from these bindings.
    let out = env::var("OUT_DIR").expect("Cargo sets it");
    println!("cargo::metadata=bindings={out}/bindings.rs");
    let list = format!("{out}/operators.txt");
    fs::write(&list, operators.borrow().join("\n"))
        .unwrap_or_else(|error| panic!("couldn't write {list}: {error}"));
    println!("cargo::metadata=operators={list}");
}
