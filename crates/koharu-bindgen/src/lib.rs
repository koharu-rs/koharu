//! A build step that generates bindings to C libraries opened at runtime.
//!
//! It turns `extern` blocks into free functions with the same names and
//! signatures that call through a table `load` fills from libraries opened at
//! runtime, so a `-sys` crate built this way can stand in for one that links.
//! `load` resolves the symbols in a loop over a static list, so its stack
//! frame doesn't grow with the number of bound functions.

use std::{env, fs, path::PathBuf};

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Expr, ExprLit, FnArg, ForeignItem, ForeignItemFn, Item, Lit, Meta, MetaNameValue, Pat, Type,
};

/// Binds the crate's `wrapper.h`, with `include/` on the include path and
/// bindgen set up further by `configure`, into `bindings.rs` in `OUT_DIR`,
/// every function resolved from libraries loaded at runtime. Cargo reruns it
/// when a header bindgen read changes; a failure stops the build.
pub fn generate(configure: impl FnOnce(bindgen::Builder) -> bindgen::Builder) {
    let crate_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo sets it"));
    let builder = bindgen::Builder::default()
        .header(crate_dir.join("wrapper.h").display().to_string())
        .clang_arg(format!("-I{}", crate_dir.join("include").display()))
        .layout_tests(false)
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()));
    let bindings = configure(builder)
        // The rewrite formats its own output.
        .formatter(bindgen::Formatter::None)
        .generate()
        .expect("bindgen failed");
    let file = syn::parse_file(&bindings.to_string()).expect("bindgen writes Rust");
    let path = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets it")).join("bindings.rs");
    fs::write(path, rewrite(file)).expect("couldn't write bindings.rs");
}

/// bindgen's output with each `extern` block replaced by its wrappers.
fn rewrite(file: syn::File) -> String {
    let mut symbols = Vec::new();
    let mut items = Vec::new();
    for item in file.items {
        let Item::ForeignMod(block) = item else {
            items.push(quote! { #item });
            continue;
        };
        for item in block.items {
            // A variadic function can't be called through a pointer of a
            // fixed type; none is bound.
            if let ForeignItem::Fn(item) = item
                && item.sig.variadic.is_none()
            {
                items.push(wrapper(&item, &block.abi, symbols.len()));
                symbols.push(symbol(&item));
            }
        }
    }
    let count = symbols.len();

    let attrs = file.attrs;
    let tokens = quote! {
        #(#attrs)*

        #(#items)*

        /// Each bound function's symbol, at the index its wrapper calls.
        static __SYMBOLS: [&str; #count] = [#(#symbols),*];

        /// A resolved function's address, which its wrapper gives its type.
        struct __Function(*const ::std::ffi::c_void);

        // Set once by `load`, then only read.
        unsafe impl Send for __Function {}
        unsafe impl Sync for __Function {}

        static __LOADED: ::std::sync::OnceLock<(
            ::std::vec::Vec<::libloading::Library>,
            ::std::vec::Vec<__Function>,
        )> = ::std::sync::OnceLock::new();

        /// Opens each library at its absolute path, in order, and resolves
        /// every function these bindings name before returning. A library
        /// that doesn't open, or a function none of them exports, is an error
        /// naming it; nothing is resolved later.
        pub fn load(paths: &[&::std::path::Path]) -> ::std::io::Result<()> {
            use ::std::{io::{Error, Result}, vec::Vec};

            let libraries = paths
                .iter()
                .map(|path| {
                    unsafe { __open(path) }.map_err(|error| {
                        Error::other(::std::format!("couldn't load {}: {error}", path.display()))
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let functions = __SYMBOLS
                .iter()
                .map(|symbol| {
                    libraries
                        .iter()
                        .find_map(|library| {
                            unsafe { library.get::<*const ::std::ffi::c_void>(symbol.as_bytes()) }.ok()
                        })
                        .map(|address| __Function(*address))
                        .ok_or_else(|| Error::other(::std::format!("no library exports {symbol}")))
                })
                .collect::<Result<Vec<_>>>()?;
            __LOADED
                .set((libraries, functions))
                .map_err(|_| Error::other("these bindings are already loaded"))
        }

        fn __function(index: usize) -> *const ::std::ffi::c_void {
            let (_, functions) = __LOADED
                .get()
                .expect("a native function was called before its library was loaded");
            functions[index].0
        }

        // A library's own dependencies come from its folder, or from those
        // already loaded.
        #[cfg(windows)]
        unsafe fn __open(path: &::std::path::Path) -> ::std::result::Result<::libloading::Library, ::libloading::Error> {
            use ::libloading::os::windows::{Library, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR};
            unsafe { Library::load_with_flags(path, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS) }
                .map(::std::convert::Into::into)
        }

        // Symbols stay local, so libraries with the same symbols don't resolve
        // into each other.
        #[cfg(unix)]
        unsafe fn __open(path: &::std::path::Path) -> ::std::result::Result<::libloading::Library, ::libloading::Error> {
            use ::libloading::os::unix::{Library, RTLD_LOCAL, RTLD_NOW};
            unsafe { Library::open(Some(path), RTLD_NOW | RTLD_LOCAL) }.map(::std::convert::Into::into)
        }
    };

    let file = syn::parse2::<syn::File>(tokens).expect("the rewrite writes Rust");
    format!(
        "// Generated by koharu-bindgen. Do not edit by hand.\n\n{}",
        prettyplease::unparse(&file)
    )
}

/// The free function that calls `function`, with the ABI `abi`, through the
/// table's address at `index`.
fn wrapper(function: &ForeignItemFn, abi: &syn::Abi, index: usize) -> TokenStream {
    let name = &function.sig.ident;
    let output = &function.sig.output;
    let (arguments, types) = arguments(function);
    let attrs = function
        .attrs
        .iter()
        .filter(|attr| !attr.path().is_ident("link_name"));
    let vis = &function.vis;
    quote! {
        #(#attrs)*
        #vis unsafe fn #name(#(#arguments: #types),*) #output {
            unsafe {
                let function: unsafe #abi fn(#(#types),*) #output =
                    ::std::mem::transmute(__function(#index));
                function(#(#arguments),*)
            }
        }
    }
}

/// The wrapper's parameter names and their types.
fn arguments(function: &ForeignItemFn) -> (Vec<syn::Ident>, Vec<&Type>) {
    let mut names = Vec::new();
    let mut types = Vec::new();
    for (index, input) in function.sig.inputs.iter().enumerate() {
        let FnArg::Typed(argument) = input else {
            unreachable!("a foreign function has no receiver");
        };
        // A header may leave a parameter unnamed or repeat a name.
        let name = match argument.pat.as_ref() {
            Pat::Ident(ident) if ident.ident != "_" && !names.contains(&ident.ident) => {
                ident.ident.clone()
            }
            _ => format_ident!("arg_{index}"),
        };
        names.push(name);
        types.push(argument.ty.as_ref());
    }
    (names, types)
}

fn symbol(function: &ForeignItemFn) -> String {
    function
        .attrs
        .iter()
        .find_map(|attr| match &attr.meta {
            Meta::NameValue(MetaNameValue {
                path,
                value:
                    Expr::Lit(ExprLit {
                        lit: Lit::Str(name),
                        ..
                    }),
                ..
            }) if path.is_ident("link_name") => Some(name.value()),
            _ => None,
        })
        .unwrap_or_else(|| function.sig.ident.to_string())
        .trim_start_matches('\u{1}')
        .to_owned()
}
