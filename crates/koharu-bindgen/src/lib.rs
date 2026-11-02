//! The build step behind Koharu's `-sys` crates.
//!
//! It turns `extern` blocks into free functions with the same names and
//! signatures that call through a table `load` fills from libraries opened at
//! run time, so a `-sys` crate built this way can stand in for one that links.
//! `load` resolves the symbols in a loop over a static list, so its stack
//! frame doesn't grow with the number of bound functions.

use std::{env, fs, path::PathBuf};

use anyhow::{Context, Result, bail};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::{Expr, ExprLit, FnArg, ForeignItem, ForeignItemFn, Item, Lit, LitStr, Meta, Pat, Type};

/// Binds the crate's `wrapper.h`, with `include/` on the include path and
/// bindgen set up further by `configure`, into `bindings.rs` in `OUT_DIR`,
/// every function resolved from libraries loaded at run time. Cargo reruns it
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
        .unwrap_or_else(|error| panic!("bindgen failed: {error}"));
    let file = syn::parse_file(&bindings.to_string()).expect("bindgen writes Rust");
    let source = rewrite(file).unwrap_or_else(|error| panic!("{error:#}"));
    let path = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets it")).join("bindings.rs");
    fs::write(&path, source)
        .unwrap_or_else(|error| panic!("couldn't write {}: {error}", path.display()));
}

/// bindgen's output with each `extern` block replaced by its wrappers.
fn rewrite(file: syn::File) -> Result<String> {
    let mut symbols = Vec::new();
    let mut items = Vec::new();
    for item in file.items {
        let Item::ForeignMod(block) = item else {
            items.push(item);
            continue;
        };
        for item in block.items {
            // A variadic function can't be called through a pointer of a
            // fixed type; none is bound.
            if let ForeignItem::Fn(item) = item
                && item.sig.variadic.is_none()
            {
                items.push(syn::parse2(wrapper(&item, &block.abi, symbols.len())?)?);
                symbols.push(LitStr::new(
                    &format!("{}\0", symbol(&item)),
                    Span::call_site(),
                ));
            }
        }
    }
    let count = symbols.len();

    let attrs = file.attrs;
    let tokens = quote! {
        #(#attrs)*

        #(#items)*

        /// A library `load` couldn't open, or a function none of them exports.
        #[derive(Debug)]
        pub struct LoadError(::std::string::String);

        impl ::std::fmt::Display for LoadError {
            fn fmt(&self, formatter: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl ::std::error::Error for LoadError {}

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
        /// naming it; nothing is resolved later, and nothing is looked up on
        /// the search path.
        pub fn load(paths: &[&::std::path::Path]) -> ::std::result::Result<(), LoadError> {
            let mut libraries = ::std::vec::Vec::new();
            for path in paths {
                if !path.is_absolute() {
                    return Err(LoadError(::std::format!("{} isn't an absolute path", path.display())));
                }
                let library = unsafe { __open(path) }.map_err(|error| {
                    LoadError(::std::format!("couldn't load {}: {error}", path.display()))
                })?;
                libraries.push(library);
            }
            // Every missing function is named at once, not just the first.
            let mut functions = ::std::vec::Vec::with_capacity(__SYMBOLS.len());
            let mut missing = ::std::vec::Vec::new();
            for symbol in &__SYMBOLS {
                let address = libraries.iter().find_map(|library| {
                    unsafe { library.get::<*const ::std::ffi::c_void>(symbol.as_bytes()) }
                        .ok()
                        .map(|address| *address)
                });
                match address {
                    Some(address) => functions.push(__Function(address)),
                    None => missing.push(symbol.trim_end_matches('\0')),
                }
            }
            if !missing.is_empty() {
                return Err(LoadError(::std::format!("no library exports {}", missing.join(", "))));
            }
            __LOADED
                .set((libraries, functions))
                .map_err(|_| LoadError("these bindings are already loaded".into()))
        }

        fn __function(index: usize) -> *const ::std::ffi::c_void {
            match __LOADED.get() {
                Some((_, functions)) => functions[index].0,
                None => panic!("a native function was called before its library was loaded"),
            }
        }

        // A library's own dependencies come from its folder, or from those
        // already loaded, such as the CUDA libraries the runtime loads first.
        #[cfg(windows)]
        unsafe fn __open(path: &::std::path::Path) -> ::std::result::Result<::libloading::Library, ::libloading::Error> {
            use ::libloading::os::windows::{Library, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR};
            unsafe { Library::load_with_flags(path, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS) }
                .map(::std::convert::Into::into)
        }

        // Symbols stay local, so two libraries that each carry ggml don't
        // resolve into each other.
        #[cfg(unix)]
        unsafe fn __open(path: &::std::path::Path) -> ::std::result::Result<::libloading::Library, ::libloading::Error> {
            use ::libloading::os::unix::{Library, RTLD_LOCAL, RTLD_NOW};
            unsafe { Library::open(Some(path), RTLD_NOW | RTLD_LOCAL) }.map(::std::convert::Into::into)
        }
    };

    let file = syn::parse2::<syn::File>(tokens).context("generated invalid Rust")?;
    Ok(format!(
        "// Generated by koharu-bindgen. Do not edit by hand.\n\n{}",
        prettyplease::unparse(&file)
    ))
}

/// The free function that calls `function`, with the ABI `abi`, through the
/// table's address at `index`.
fn wrapper(function: &ForeignItemFn, abi: &syn::Abi, index: usize) -> Result<TokenStream> {
    let name = &function.sig.ident;
    let output = &function.sig.output;
    let (inputs, arguments, types) = arguments(function)?;
    let attrs = function
        .attrs
        .iter()
        .filter(|attr| !attr.path().is_ident("link_name"));
    let vis = &function.vis;
    Ok(quote! {
        #(#attrs)*
        #vis unsafe fn #name(#(#inputs),*) #output {
            unsafe {
                let function: unsafe #abi fn(#(#types),*) #output =
                    ::std::mem::transmute(__function(#index));
                function(#(#arguments),*)
            }
        }
    })
}

/// The wrapper's parameters, the names it passes on, and their types.
fn arguments(function: &ForeignItemFn) -> Result<(Vec<TokenStream>, Vec<syn::Ident>, Vec<Type>)> {
    let mut inputs = Vec::new();
    let mut names = Vec::new();
    let mut types = Vec::new();
    for (index, input) in function.sig.inputs.iter().enumerate() {
        let FnArg::Typed(argument) = input else {
            bail!("`{}` takes a receiver", function.sig.ident);
        };
        // A header may leave a parameter unnamed or repeat a name.
        let name = match argument.pat.as_ref() {
            Pat::Ident(ident) if ident.ident != "_" && !names.contains(&ident.ident) => {
                ident.ident.clone()
            }
            _ => format_ident!("arg_{index}"),
        };
        let ty = argument.ty.as_ref();
        inputs.push(quote! { #name: #ty });
        names.push(name);
        types.push(ty.clone());
    }
    Ok((inputs, names, types))
}

fn symbol(function: &ForeignItemFn) -> String {
    function
        .attrs
        .iter()
        .find_map(|attr| match &attr.meta {
            Meta::NameValue(meta) if meta.path.is_ident("link_name") => match &meta.value {
                Expr::Lit(ExprLit {
                    lit: Lit::Str(value),
                    ..
                }) => Some(value.value()),
                _ => None,
            },
            _ => None,
        })
        .unwrap_or_else(|| function.sig.ident.to_string())
        .trim_start_matches('\u{1}')
        .to_owned()
}
