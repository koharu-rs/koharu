//! Turns bindgen's `extern` blocks into free functions with the same names and
//! signatures that call through a table `load` fills from libraries opened at
//! run time, so a `-sys` crate built this way can stand in for one that links.

use anyhow::{Context, Result, bail};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::{Expr, ExprLit, FnArg, ForeignItem, ForeignItemFn, Item, Lit, LitStr, Meta, Pat, Type};

pub(crate) fn rewrite(source: &str) -> Result<String> {
    let file = syn::parse_file(source).context("couldn't parse bindgen's output")?;
    let mut items = Vec::new();
    let mut functions = Vec::new();
    for item in file.items {
        match item {
            Item::ForeignMod(block) => {
                for item in block.items {
                    // A variadic function can't be called through a pointer
                    // of a fixed type; none is bound.
                    if let ForeignItem::Fn(function) = item
                        && function.sig.variadic.is_none()
                    {
                        functions.push((block.abi.clone(), function));
                    }
                }
            }
            other => items.push(other),
        }
    }

    let mut fields = Vec::new();
    let mut symbols = Vec::new();
    let mut resolves = Vec::new();
    let mut wrappers = Vec::new();
    for (abi, function) in &functions {
        let name = &function.sig.ident;
        let symbol = LitStr::new(&format!("{}\0", symbol(function)), Span::call_site());
        symbols.push(symbol.clone());
        let output = &function.sig.output;
        let (inputs, arguments, types) = arguments(function)?;
        let attrs = function
            .attrs
            .iter()
            .filter(|attr| !attr.path().is_ident("link_name"));
        let vis = &function.vis;
        fields.push(quote! { #name: unsafe #abi fn(#(#types),*) #output });
        resolves.push(quote! { #name: unsafe { __resolve(&libraries, #symbol)? } });
        wrappers.push(quote! {
            #(#attrs)*
            #vis unsafe fn #name(#(#inputs),*) #output {
                unsafe { (__functions().#name)(#(#arguments),*) }
            }
        });
    }

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

        struct __Functions {
            #(#fields,)*
        }

        static __LOADED: ::std::sync::OnceLock<(::std::vec::Vec<::libloading::Library>, __Functions)> =
            ::std::sync::OnceLock::new();

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
            let missing = [#(#symbols),*]
                .into_iter()
                .filter(|symbol| unsafe { __resolve::<*const ::std::ffi::c_void>(&libraries, symbol) }.is_err())
                .map(|symbol| symbol.trim_end_matches('\0'))
                .collect::<::std::vec::Vec<_>>();
            if !missing.is_empty() {
                return Err(LoadError(::std::format!("no library exports {}", missing.join(", "))));
            }
            let functions = __Functions {
                #(#resolves,)*
            };
            __LOADED
                .set((libraries, functions))
                .map_err(|_| LoadError("these bindings are already loaded".into()))
        }

        unsafe fn __resolve<F: Copy>(
            libraries: &[::libloading::Library],
            symbol: &str,
        ) -> ::std::result::Result<F, LoadError> {
            libraries
                .iter()
                .find_map(|library| unsafe { library.get::<F>(symbol.as_bytes()) }.ok().map(|function| *function))
                .ok_or_else(|| {
                    LoadError(::std::format!("no library exports `{}`", symbol.trim_end_matches('\0')))
                })
        }

        fn __functions() -> &'static __Functions {
            match __LOADED.get() {
                Some((_, functions)) => functions,
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

        #(#wrappers)*
    };

    let file = syn::parse2::<syn::File>(tokens).context("generated invalid Rust")?;
    Ok(format!(
        "// Generated by koharu-bindgen. Do not edit by hand.\n\n{}",
        prettyplease::unparse(&file)
    ))
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
