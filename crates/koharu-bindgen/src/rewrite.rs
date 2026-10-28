//! Turns `extern` blocks into free functions with the same names and
//! signatures that call through a table `load` fills from libraries opened at
//! run time, so a `-sys` crate built this way can stand in for one that links.

use std::path::Path;

use anyhow::{Context, Result, bail};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::{
    AttrStyle, Attribute, Expr, ExprLit, FnArg, ForeignItem, ForeignItemFn, Item, Lit, LitStr,
    Meta, Pat, Type,
};

/// A bound function, and the `#[cfg]`s of the modules it's declared in, which
/// decide whether `load` resolves it.
struct Function {
    abi: syn::Abi,
    item: ForeignItemFn,
    cfgs: Vec<Attribute>,
}

/// Rewrites one file. `dir` is where a `mod name;` in it is read from, as
/// `dir/name.rs`; bindgen's output has none.
pub(crate) fn rewrite(file: syn::File, dir: Option<&Path>) -> Result<String> {
    let mut functions = Vec::new();
    let items = items(file.items, dir, 0, &[], &mut functions)?;

    let mut fields = Vec::new();
    let mut checks = Vec::new();
    let mut resolves = Vec::new();
    for Function { abi, item, cfgs } in &functions {
        let name = &item.sig.ident;
        let symbol = LitStr::new(&format!("{}\0", symbol(item)), Span::call_site());
        let output = &item.sig.output;
        let (_, _, types) = arguments(item)?;
        fields.push(quote! { #(#cfgs)* #name: unsafe #abi fn(#(#types),*) #output });
        checks.push(quote! { #(#cfgs)* __check(&libraries, #symbol, &mut missing); });
        resolves.push(quote! { #(#cfgs)* #name: unsafe { __resolve(&libraries, #symbol)? } });
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
            let mut missing = ::std::vec::Vec::new();
            #(#checks)*
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

        fn __check(libraries: &[::libloading::Library], symbol: &'static str, missing: &mut ::std::vec::Vec<&'static str>) {
            if unsafe { __resolve::<*const ::std::ffi::c_void>(libraries, symbol) }.is_err() {
                missing.push(symbol.trim_end_matches('\0'));
            }
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
    };

    let file = syn::parse2::<syn::File>(tokens).context("generated invalid Rust")?;
    Ok(format!(
        "// Generated by koharu-bindgen. Do not edit by hand.\n\n{}",
        prettyplease::unparse(&file)
    ))
}

/// A module's items with each `extern` block replaced by its wrappers, and
/// each `mod name;` read in and rewritten in place.
fn items(
    items: Vec<Item>,
    dir: Option<&Path>,
    depth: usize,
    cfgs: &[Attribute],
    functions: &mut Vec<Function>,
) -> Result<Vec<Item>> {
    let mut rewritten = Vec::new();
    for item in items {
        match item {
            Item::ForeignMod(block) => {
                for item in block.items {
                    // A variadic function can't be called through a pointer
                    // of a fixed type; none is bound.
                    if let ForeignItem::Fn(item) = item
                        && item.sig.variadic.is_none()
                    {
                        rewritten.push(syn::parse2(wrapper(&item, depth)?)?);
                        functions.push(Function {
                            abi: block.abi.clone(),
                            item,
                            cfgs: cfgs.to_vec(),
                        });
                    }
                }
            }
            Item::Mod(mut module) => {
                let name = module.ident.to_string();
                let content = match module.content.take() {
                    Some((_, content)) => content,
                    None => {
                        let Some(dir) = dir else {
                            bail!("`mod {name};` has no file to read");
                        };
                        let path = dir.join(format!("{name}.rs"));
                        let source = std::fs::read_to_string(&path)
                            .with_context(|| format!("couldn't read {}", path.display()))?;
                        let file = syn::parse_file(&source)
                            .with_context(|| format!("couldn't parse {}", path.display()))?;
                        // The file's inner attributes become the module's.
                        module.attrs.extend(file.attrs.into_iter().map(|mut attr| {
                            attr.style = AttrStyle::Inner(Default::default());
                            attr
                        }));
                        file.items
                    }
                };
                let mut cfgs = cfgs.to_vec();
                cfgs.extend(
                    module
                        .attrs
                        .iter()
                        .filter(|attr| {
                            attr.path().is_ident("cfg") && matches!(attr.style, AttrStyle::Outer)
                        })
                        .cloned(),
                );
                let dir = dir.map(|dir| dir.join(&name));
                let content = self::items(content, dir.as_deref(), depth + 1, &cfgs, functions)?;
                module.content = Some((Default::default(), content));
                module.semi = None;
                rewritten.push(Item::Mod(module));
            }
            other => rewritten.push(other),
        }
    }
    Ok(rewritten)
}

/// The free function that calls `function` through the table, from a module
/// `depth` below the table's.
fn wrapper(function: &ForeignItemFn, depth: usize) -> Result<TokenStream> {
    let name = &function.sig.ident;
    let output = &function.sig.output;
    let (inputs, arguments, _) = arguments(function)?;
    let attrs = function
        .attrs
        .iter()
        .filter(|attr| !attr.path().is_ident("link_name"));
    let vis = &function.vis;
    let parents = (0..depth).map(|_| quote! { super:: });
    Ok(quote! {
        #(#attrs)*
        #vis unsafe fn #name(#(#inputs),*) #output {
            unsafe { (#(#parents)* __functions().#name)(#(#arguments),*) }
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
