//! Writes `tensor_generated.rs`: a `Tensor` method, or an associated function
//! where an operator takes no `self` tensor, for each operator koharu-torch-sys
//! binds. The shim's header names every bool and libtorch enum, and each
//! length, device index, and result parameter, so an operator's C signature
//! gives its Rust one. A parameter of any other type stops the build.

use std::{collections::BTreeSet, env, path::PathBuf};

use proc_macro2::{Ident, Span, TokenStream};
use quote::{format_ident, quote};
use syn::{FnArg, Item, ItemFn, Pat};

/// The parameters a factory's TensorOptions become, which a caller passes as
/// one `(Kind, Device)`.
const OPTIONS: [(&str, &str); 5] = [
    ("dtype", "*mutAtenDtype"),
    ("layout", "*mutAtenLayout"),
    ("device", "*mutAtenDeviceType"),
    ("device_index_", "i32"),
    ("pin_memory", "*mutAtenBool"),
];

fn main() {
    let bindings = env::var("DEP_KOHARU_TORCH_BINDINGS").expect("koharu-torch-sys passes it");
    println!("cargo::rerun-if-changed={bindings}");
    let bindings = fs_err::read_to_string(bindings).expect("couldn't read the bindings");
    let file = syn::parse_file(&bindings).expect("bindgen writes Rust");
    // The shim's hand-written functions share the operators' prefix, so only
    // this header tells them apart.
    let header = env::var("DEP_KOHARU_TORCH_OPERATORS").expect("koharu-torch-sys passes it");
    println!("cargo::rerun-if-changed={header}");
    let header = fs_err::read_to_string(header).expect("couldn't read the operators' header");
    let operators: BTreeSet<&str> = header
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .collect();

    let functions: Vec<TokenStream> = file
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Fn(function) if operators.contains(function.sig.ident.to_string().as_str()) => {
                operator_function(function)
            }
            _ => None,
        })
        .collect();

    let file = syn::parse2::<syn::File>(quote! { impl Tensor { #(#functions)* } })
        .expect("the generated methods are valid Rust");
    let path =
        PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets it")).join("tensor_generated.rs");
    fs_err::write(path, prettyplease::unparse(&file)).expect("couldn't write the methods");
}

/// The operator's Rust name: its C name in lower case without `aten_`, with
/// `internal_` for the leading underscores of a private operator.
fn name(c_name: &Ident) -> Ident {
    let lower = c_name
        .to_string()
        .strip_prefix("aten_")
        .expect("the shim names its operators `aten_*`")
        .to_lowercase();
    let public = lower.trim_start_matches('_');
    let name = if public.len() == lower.len() {
        lower.clone()
    } else {
        format!("internal_{public}")
    };
    syn::parse_str::<Ident>(&name).unwrap_or_else(|_| Ident::new_raw(&name, Span::call_site()))
}

/// What the method or function needs from the C signature.
#[derive(Default)]
struct Function {
    receiver: bool,
    inputs: Vec<TokenStream>,
    setup: Vec<TokenStream>,
    arguments: Vec<TokenStream>,
    results: Vec<(TokenStream, TokenStream)>,
}

impl Function {
    /// An argument the C function takes as `argument`, with nothing to set up.
    fn plain(&mut self, parameter: &Ident, ty: TokenStream, argument: TokenStream) {
        self.inputs.push(quote! { #parameter: #ty });
        self.arguments.push(argument);
    }

    /// An optional argument: `value` is an `Option`, and the C function takes
    /// a pointer to it, null for `None`.
    fn optional(&mut self, parameter: &Ident, ty: TokenStream, value: TokenStream) {
        self.inputs.push(quote! { #parameter: #ty });
        self.setup.push(quote! { let mut #parameter = #value; });
        self.arguments.push(quote! {
            #parameter.as_mut().map_or(std::ptr::null_mut(), |value| value as *mut _)
        });
    }

    /// A list: `value` makes something with `as_ptr` and `len`.
    fn list(&mut self, parameter: &Ident, ty: TokenStream, value: TokenStream) {
        self.inputs.push(quote! { #parameter: #ty });
        self.setup.push(quote! { let #parameter = #value; });
        self.arguments
            .push(quote! { #parameter.as_ptr(), #parameter.len() as i64 });
    }

    /// An optional list: `value` is an `Option` of a slice, and the C
    /// function takes a pointer to its data pointer, null for `None`.
    fn optional_list(&mut self, parameter: &Ident, ty: TokenStream, value: TokenStream) {
        let pointer = format_ident!("{parameter}__pointer");
        self.inputs.push(quote! { #parameter: #ty });
        self.setup.push(quote! {
            let #parameter = #value;
            let mut #pointer = #parameter.map(|list| list.as_ptr());
        });
        self.arguments.push(quote! {
            #pointer.as_mut().map_or(std::ptr::null_mut(), |pointer| pointer as *mut _),
            #parameter.map_or(0, |list| list.len() as i64)
        });
    }
}

/// The method or function calling the operator's C function, or `None` for one
/// that takes a fixed-length bool list: the shim reads the schema's length,
/// which the binding doesn't carry, so a shorter slice would be read past its
/// end.
fn operator_function(c_function: &ItemFn) -> Option<TokenStream> {
    let c_name = &c_function.sig.ident;
    let parameters: Vec<(String, String)> = c_function
        .sig
        .inputs
        .iter()
        .map(|input| {
            let FnArg::Typed(argument) = input else {
                unreachable!("a wrapper has no receiver");
            };
            let Pat::Ident(name) = argument.pat.as_ref() else {
                unreachable!("a wrapper names its parameters");
            };
            let ty = argument.ty.as_ref();
            (
                name.ident.to_string(),
                quote!(#ty).to_string().replace(' ', ""),
            )
        })
        .collect();

    let mut function = Function::default();
    let mut index = 0;
    while index < parameters.len() {
        let (parameter, ty) = &parameters[index];
        let ident = format_ident!("{parameter}");
        // bindgen renames a parameter named after a Rust keyword, `self` to
        // `self_`, but not the length or index parameter that follows it.
        let header_name = if parameter == "self_" {
            "self"
        } else {
            parameter
        };
        // The length or device index that goes with this parameter.
        let paired = parameters
            .get(index + 1)
            .and_then(|(next, _)| next.strip_prefix(header_name))
            .filter(|suffix| matches!(*suffix, "_len_" | "_index_"));

        if parameter
            .strip_prefix("ret")
            .is_some_and(|n| n.parse::<u32>().is_ok())
        {
            let len = format_ident!("{parameter}_len_");
            let null = quote! { std::ptr::null_mut() };
            let (initial, output, value) = match (ty.as_str(), paired) {
                ("*mutAtenTensorHandle", None) => (
                    null,
                    quote! { Tensor },
                    quote! { Tensor { c_tensor: #ident } },
                ),
                ("*mut*mutAtenTensorHandle", Some("_len_")) => {
                    function.setup.push(quote! { let mut #len = 0; });
                    (
                        null,
                        quote! { Vec<Tensor> },
                        quote! { unsafe { Tensor::from_array(#ident, #len) } },
                    )
                }
                ("*muti64", None) => (quote! { 0 }, quote! { i64 }, quote! { #ident }),
                ("*mutf64", None) => (quote! { 0.0 }, quote! { f64 }, quote! { #ident }),
                ("*mutAtenBool", None) => (quote! { 0 }, quote! { bool }, quote! { #ident != 0 }),
                ("*mutAtenDtype", None) => (
                    quote! { 0 },
                    quote! { Kind },
                    quote! { Kind::from_c_int(#ident)? },
                ),
                ("*mutAtenQScheme", None) => (
                    quote! { 0 },
                    quote! { QScheme },
                    quote! { QScheme::from_c_int(#ident)? },
                ),
                ("*mutAtenScalar", None) => (
                    quote! { Scalar::Int(0).c_scalar() },
                    quote! { Scalar },
                    quote! { Scalar::from_c_scalar(#ident)? },
                ),
                _ => panic!("`{c_name}` returns `{parameter}: {ty}`, which has no Rust type here"),
            };
            function.setup.push(quote! { let mut #ident = #initial; });
            function.arguments.push(quote! { &mut #ident });
            if paired.is_some() {
                function.arguments.push(quote! { &mut #len });
            }
            function.results.push((output, value));
            index += 1 + usize::from(paired.is_some());
            continue;
        }
        assert!(
            function.results.is_empty(),
            "`{c_name}` takes `{parameter}` after a result"
        );

        if parameters[index..]
            .iter()
            .take(OPTIONS.len())
            .map(|(name, ty)| (name.as_str(), ty.as_str()))
            .eq(OPTIONS)
        {
            // `None` keeps libtorch's defaults, which for a `_like` operator
            // are its input's.
            function
                .inputs
                .push(quote! { options: impl Into<Option<(Kind, Device)>> });
            function.setup.push(quote! {
                let options = options.into();
                let mut dtype = options.map(|(kind, _)| kind.c_int());
                let options = options.map(|(_, device)| device.c_parts());
                let mut device = options.map(|(device, _)| device);
                let device_index = options.map_or(-1, |(_, index)| index);
            });
            function.arguments.push(quote! {
                dtype.as_mut().map_or(std::ptr::null_mut(), |dtype| dtype as *mut _),
                std::ptr::null_mut(),
                device.as_mut().map_or(std::ptr::null_mut(), |device| device as *mut _),
                device_index,
                std::ptr::null_mut()
            });
            index += OPTIONS.len();
            continue;
        }

        match (ty.as_str(), paired) {
            // The schema's `self`, wherever it is, as in where.self(condition,
            // self, other).
            ("AtenTensorHandle", None) if parameter == "self_" => {
                function.receiver = true;
                function.arguments.push(quote! { self.c_tensor });
            }
            ("AtenTensorHandle", None) => {
                function.plain(&ident, quote! { &Tensor }, quote! { #ident.c_tensor })
            }
            ("*mutAtenTensorHandle", None) => function.optional(
                &ident,
                quote! { Option<&Tensor> },
                quote! { #ident.map(|tensor| tensor.c_tensor) },
            ),
            ("*constAtenTensorHandle", Some("_len_")) => function.list(
                &ident,
                quote! { &[impl Borrow<Tensor>] },
                quote! { #ident.iter().map(|tensor| tensor.borrow().c_tensor).collect::<Vec<_>>() },
            ),
            ("*mut*constAtenTensorHandle", Some("_len_")) => {
                let pointers = format_ident!("{parameter}__pointers");
                function.inputs.push(quote! { #ident: &[Option<&Tensor>] });
                function.setup.push(quote! {
                    let #ident = #ident
                        .iter()
                        .map(|tensor| tensor.map(|tensor| tensor.c_tensor))
                        .collect::<Vec<_>>();
                    let mut #pointers = #ident
                        .iter()
                        .map(|handle| handle.as_ref().map_or(std::ptr::null(), |handle| handle as *const _))
                        .collect::<Vec<_>>();
                });
                function
                    .arguments
                    .push(quote! { #pointers.as_mut_ptr(), #pointers.len() as i64 });
            }
            ("i64" | "f64" | "i32", None) => {
                let ty = format_ident!("{ty}");
                function.plain(&ident, quote! { #ty }, quote! { #ident });
            }
            ("*muti64", None) => function.optional(
                &ident,
                quote! { impl Into<Option<i64>> },
                quote! { #ident.into() },
            ),
            ("*mutf64", None) => function.optional(
                &ident,
                quote! { impl Into<Option<f64>> },
                quote! { #ident.into() },
            ),
            ("*consti64", Some("_len_")) => function.list(
                &ident,
                quote! { impl IntList },
                quote! { #ident.int_list() },
            ),
            ("*mut*consti64", Some("_len_")) => function.optional_list(
                &ident,
                quote! { impl IntListOption },
                quote! { #ident.int_list_option() },
            ),
            ("*mut*constf64", Some("_len_")) => {
                function.optional_list(&ident, quote! { Option<&[f64]> }, quote! { #ident })
            }
            ("AtenBool", None) => {
                function.plain(&ident, quote! { bool }, quote! { i32::from(#ident) })
            }
            ("*mutAtenBool", None) => function.optional(
                &ident,
                quote! { impl Into<Option<bool>> },
                quote! { #ident.into().map(i32::from) },
            ),
            ("*constAtenBool", Some("_len_")) => return None,
            ("AtenDtype", None) => {
                function.plain(&ident, quote! { Kind }, quote! { #ident.c_int() })
            }
            ("*mutAtenDtype", None) => function.optional(
                &ident,
                quote! { impl Into<Option<Kind>> },
                quote! { #ident.into().map(Kind::c_int) },
            ),
            ("AtenLayout", None) => {
                function.plain(&ident, quote! { Layout }, quote! { #ident.c_int() })
            }
            ("*mutAtenLayout", None) => function.optional(
                &ident,
                quote! { impl Into<Option<Layout>> },
                quote! { #ident.into().map(Layout::c_int) },
            ),
            ("AtenMemoryFormat", None) => {
                function.plain(&ident, quote! { MemoryFormat }, quote! { #ident.c_int() })
            }
            ("*mutAtenMemoryFormat", None) => function.optional(
                &ident,
                quote! { impl Into<Option<MemoryFormat>> },
                quote! { #ident.into().map(MemoryFormat::c_int) },
            ),
            ("AtenDeviceType", Some("_index_")) => {
                let (device, index) = (
                    format_ident!("{parameter}__type"),
                    format_ident!("{parameter}__index"),
                );
                function.inputs.push(quote! { #ident: Device });
                function
                    .setup
                    .push(quote! { let (#device, #index) = #ident.c_parts(); });
                function.arguments.push(quote! { #device, #index });
            }
            ("*mutAtenDeviceType", Some("_index_")) => {
                let (device, index) = (
                    format_ident!("{parameter}__type"),
                    format_ident!("{parameter}__index"),
                );
                function
                    .inputs
                    .push(quote! { #ident: impl Into<Option<Device>> });
                function.setup.push(quote! {
                    let #ident = #ident.into().map(Device::c_parts);
                    let mut #device = #ident.map(|(device, _)| device);
                    let #index = #ident.map_or(-1, |(_, index)| index);
                });
                function.arguments.push(quote! {
                    #device.as_mut().map_or(std::ptr::null_mut(), |device| device as *mut _), #index
                });
            }
            ("AtenScalar", None) => function.plain(
                &ident,
                quote! { impl Into<Scalar> },
                quote! { #ident.into().c_scalar() },
            ),
            ("*mutAtenScalar", None) => function.optional(
                &ident,
                quote! { impl Into<Option<Scalar>> },
                quote! { #ident.into().map(Scalar::c_scalar) },
            ),
            ("*constAtenScalar", Some("_len_")) => function.list(
                &ident,
                quote! { &[Scalar] },
                quote! { #ident.iter().map(|scalar| scalar.c_scalar()).collect::<Vec<_>>() },
            ),
            ("*const::std::os::raw::c_char", None) => {
                function.inputs.push(quote! { #ident: &str });
                function
                    .setup
                    .push(quote! { let #ident = std::ffi::CString::new(#ident)?; });
                function.arguments.push(quote! { #ident.as_ptr() });
            }
            ("*mut*const::std::os::raw::c_char", None) => {
                let pointer = format_ident!("{parameter}__pointer");
                function.inputs.push(quote! { #ident: Option<&str> });
                function.setup.push(quote! {
                    let #ident = #ident.map(std::ffi::CString::new).transpose()?;
                    let mut #pointer = #ident.as_ref().map(|string| string.as_ptr());
                });
                function.arguments.push(quote! {
                    #pointer.as_mut().map_or(std::ptr::null_mut(), |pointer| pointer as *mut _)
                });
            }
            ("*mutAtenGeneratorHandle", None) => function.optional(
                &ident,
                quote! { Option<&Generator> },
                quote! { #ident.map(|generator| generator.c_generator) },
            ),
            _ => panic!("`{c_name}` takes `{parameter}: {ty}`, which has no Rust type here"),
        }
        index += 1 + usize::from(paired.is_some());
    }

    let Function {
        receiver,
        inputs,
        setup,
        arguments,
        results,
    } = function;
    let name = name(c_name);
    let receiver = receiver.then(|| quote! { &self, });
    let (types, values): (Vec<_>, Vec<_>) = results.into_iter().unzip();
    let (output, value) = match types.len() {
        1 => (types[0].clone(), values[0].clone()),
        _ => (quote! { (#(#types),*) }, quote! { (#(#values),*) }),
    };
    Some(quote! {
        pub fn #name(#receiver #(#inputs),*) -> Result<#output> {
            #(#setup)*
            check(unsafe { #c_name(#(#arguments),*) })?;
            Ok(#value)
        }
    })
}
