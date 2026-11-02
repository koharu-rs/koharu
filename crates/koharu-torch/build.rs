//! Writes `tensor_generated.rs`: a `Tensor` method, or an associated function
//! where an operator takes no `self` tensor, for each operator koharu-torch-sys
//! binds. The shim's header names every bool and libtorch enum, and each
//! length, device index, and result parameter, so an operator's C signature
//! gives its Rust one. A parameter of any other type stops the build.

use std::{collections::BTreeSet, env, fs, path::PathBuf};

use proc_macro2::{Ident, Span, TokenStream};
use quote::{format_ident, quote};
use syn::{FnArg, Item, Pat};

/// What the shim's operator functions are named after.
const PREFIX: &str = "koharu_torch_aten_";

/// The parameters a factory's TensorOptions become, which a caller passes as
/// one `(Kind, Device)`.
const OPTIONS: [(&str, &str); 5] = [
    ("dtype", "*mutKoharuTorchDtype"),
    ("layout", "*mutKoharuTorchLayout"),
    ("device", "*mutKoharuTorchDeviceType"),
    ("device_index_", "i32"),
    ("pin_memory", "*mutKoharuTorchBool"),
];

fn main() {
    let bindings = env::var("DEP_KOHARU_TORCH_BINDINGS").expect("koharu-torch-sys passes it");
    println!("cargo::rerun-if-changed={bindings}");
    let source = fs::read_to_string(&bindings)
        .unwrap_or_else(|error| panic!("couldn't read {bindings}: {error}"));
    let file = syn::parse_file(&source).expect("bindgen writes Rust");

    let mut names = BTreeSet::new();
    let mut functions = Vec::new();
    for item in file.items {
        let Item::Fn(function) = item else {
            continue;
        };
        let c_name = function.sig.ident;
        let Some(operator) = c_name.to_string().strip_prefix(PREFIX).map(str::to_owned) else {
            continue;
        };
        let parameters = function
            .sig
            .inputs
            .iter()
            .map(|input| {
                let FnArg::Typed(argument) = input else {
                    panic!("`{c_name}` takes a receiver");
                };
                let Pat::Ident(name) = argument.pat.as_ref() else {
                    panic!("`{c_name}` has an unnamed parameter");
                };
                let ty = argument.ty.as_ref();
                (
                    name.ident.to_string(),
                    quote!(#ty).to_string().replace(' ', ""),
                )
            })
            .collect::<Vec<_>>();
        let name = name(&operator);
        assert!(
            names.insert(name.to_string()),
            "two operators are named `{name}`"
        );
        if let Some(function) = operator_function(&c_name, &name, &parameters) {
            functions.push(function);
        }
    }

    let file = syn::parse2::<syn::File>(quote! { impl Tensor { #(#functions)* } })
        .expect("the generated methods are valid Rust");
    let path =
        PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets it")).join("tensor_generated.rs");
    fs::write(&path, prettyplease::unparse(&file))
        .unwrap_or_else(|error| panic!("couldn't write {}: {error}", path.display()));
}

/// The operator's Rust name: its C name in lower case, with `internal_` for
/// the leading underscores of a private operator.
fn name(operator: &str) -> Ident {
    let lower = operator.to_lowercase();
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

    /// A result the C function writes through a pointer to `initial`.
    fn result(
        &mut self,
        parameter: &Ident,
        initial: TokenStream,
        ty: TokenStream,
        value: TokenStream,
    ) {
        self.setup.push(quote! { let mut #parameter = #initial; });
        self.arguments.push(quote! { &mut #parameter });
        self.results.push((ty, value));
    }
}

/// The method or function calling `c_name`, or `None` for an operator that
/// takes a fixed-length bool list: the shim reads the schema's length, which
/// the binding doesn't carry, so a shorter slice would be read past its end.
fn operator_function(
    c_name: &Ident,
    name: &Ident,
    parameters: &[(String, String)],
) -> Option<TokenStream> {
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
        let next = |suffix: &str| {
            parameters
                .get(index + 1)
                .is_some_and(|(next, _)| *next == format!("{header_name}{suffix}"))
        };

        if parameter
            .strip_prefix("ret")
            .is_some_and(|n| n.parse::<u32>().is_ok())
        {
            match (ty.as_str(), next("_len_")) {
                ("*mutAtenTensorHandle", false) => function.result(
                    &ident,
                    quote! { std::ptr::null_mut() },
                    quote! { Tensor },
                    quote! { Tensor { c_tensor: #ident } },
                ),
                ("*mut*mutAtenTensorHandle", true) => {
                    let len = format_ident!("{parameter}_len_");
                    function.setup.push(quote! { let mut #len = 0; });
                    function.result(
                        &ident,
                        quote! { std::ptr::null_mut() },
                        quote! { Vec<Tensor> },
                        quote! { unsafe { Tensor::from_array(#ident, #len) } },
                    );
                    function.arguments.push(quote! { &mut #len });
                    index += 1;
                }
                ("*muti64", false) => {
                    function.result(&ident, quote! { 0 }, quote! { i64 }, quote! { #ident })
                }
                ("*mutf64", false) => {
                    function.result(&ident, quote! { 0.0 }, quote! { f64 }, quote! { #ident })
                }
                ("*mutKoharuTorchBool", false) => function.result(
                    &ident,
                    quote! { 0 },
                    quote! { bool },
                    quote! { #ident != 0 },
                ),
                ("*mutKoharuTorchDtype", false) => function.result(
                    &ident,
                    quote! { 0 },
                    quote! { Kind },
                    quote! { Kind::from_c_int(#ident)? },
                ),
                ("*mutKoharuTorchQScheme", false) => function.result(
                    &ident,
                    quote! { 0 },
                    quote! { QScheme },
                    quote! { QScheme::from_c_int(#ident)? },
                ),
                ("*mutKoharuTorchScalar", false) => function.result(
                    &ident,
                    quote! { Scalar::Int(0).c_scalar() },
                    quote! { Scalar },
                    quote! { Scalar::from_c_scalar(#ident)? },
                ),
                _ => panic!("`{c_name}` returns `{parameter}: {ty}`, which has no Rust type here"),
            }
            index += 1;
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

        let (len, device_index) = (next("_len_"), next("_index_"));
        match (ty.as_str(), len, device_index) {
            // The schema's `self`, wherever it is, as in where.self(condition,
            // self, other).
            ("AtenTensorHandle", false, false) if parameter == "self_" => {
                function.receiver = true;
                function.arguments.push(quote! { self.c_tensor });
            }
            ("AtenTensorHandle", false, false) => {
                function.inputs.push(quote! { #ident: &Tensor });
                function.arguments.push(quote! { #ident.c_tensor });
            }
            ("*mutAtenTensorHandle", false, false) => function.optional(
                &ident,
                quote! { Option<&Tensor> },
                quote! { #ident.map(|tensor| tensor.c_tensor) },
            ),
            ("*constAtenTensorHandle", true, false) => function.list(
                &ident,
                quote! { &[impl Borrow<Tensor>] },
                quote! { #ident.iter().map(|tensor| tensor.borrow().c_tensor).collect::<Vec<_>>() },
            ),
            ("*mut*constAtenTensorHandle", true, false) => {
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
            ("i64", false, false) | ("f64", false, false) | ("i32", false, false) => {
                let ty = format_ident!("{ty}");
                function.inputs.push(quote! { #ident: #ty });
                function.arguments.push(quote! { #ident });
            }
            ("*muti64", false, false) => function.optional(
                &ident,
                quote! { impl Into<Option<i64>> },
                quote! { #ident.into() },
            ),
            ("*mutf64", false, false) => function.optional(
                &ident,
                quote! { impl Into<Option<f64>> },
                quote! { #ident.into() },
            ),
            ("*consti64", true, false) => function.list(
                &ident,
                quote! { impl IntList },
                quote! { #ident.int_list() },
            ),
            ("*mut*consti64", true, false) => function.optional_list(
                &ident,
                quote! { impl IntListOption },
                quote! { #ident.int_list_option() },
            ),
            ("*mut*constf64", true, false) => {
                function.optional_list(&ident, quote! { Option<&[f64]> }, quote! { #ident })
            }
            ("KoharuTorchBool", false, false) => {
                function.inputs.push(quote! { #ident: bool });
                function.arguments.push(quote! { i32::from(#ident) });
            }
            ("*mutKoharuTorchBool", false, false) => function.optional(
                &ident,
                quote! { impl Into<Option<bool>> },
                quote! { #ident.into().map(i32::from) },
            ),
            ("*constKoharuTorchBool", true, false) => return None,
            ("KoharuTorchDtype", false, false) => {
                function.inputs.push(quote! { #ident: Kind });
                function.arguments.push(quote! { #ident.c_int() });
            }
            ("*mutKoharuTorchDtype", false, false) => function.optional(
                &ident,
                quote! { impl Into<Option<Kind>> },
                quote! { #ident.into().map(Kind::c_int) },
            ),
            ("KoharuTorchLayout", false, false) => {
                function.inputs.push(quote! { #ident: Layout });
                function.arguments.push(quote! { #ident.c_int() });
            }
            ("*mutKoharuTorchLayout", false, false) => function.optional(
                &ident,
                quote! { impl Into<Option<Layout>> },
                quote! { #ident.into().map(Layout::c_int) },
            ),
            ("KoharuTorchMemoryFormat", false, false) => {
                function.inputs.push(quote! { #ident: MemoryFormat });
                function.arguments.push(quote! { #ident.c_int() });
            }
            ("*mutKoharuTorchMemoryFormat", false, false) => function.optional(
                &ident,
                quote! { impl Into<Option<MemoryFormat>> },
                quote! { #ident.into().map(MemoryFormat::c_int) },
            ),
            ("KoharuTorchDeviceType", false, true) => {
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
            ("*mutKoharuTorchDeviceType", false, true) => {
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
            ("KoharuTorchScalar", false, false) => {
                function.inputs.push(quote! { #ident: impl Into<Scalar> });
                function.arguments.push(quote! { #ident.into().c_scalar() });
            }
            ("*mutKoharuTorchScalar", false, false) => function.optional(
                &ident,
                quote! { impl Into<Option<Scalar>> },
                quote! { #ident.into().map(Scalar::c_scalar) },
            ),
            ("*constKoharuTorchScalar", true, false) => function.list(
                &ident,
                quote! { &[Scalar] },
                quote! { #ident.iter().map(|scalar| scalar.c_scalar()).collect::<Vec<_>>() },
            ),
            ("*const::std::os::raw::c_char", false, false) => {
                function.inputs.push(quote! { #ident: &str });
                function
                    .setup
                    .push(quote! { let #ident = std::ffi::CString::new(#ident)?; });
                function.arguments.push(quote! { #ident.as_ptr() });
            }
            ("*mut*const::std::os::raw::c_char", false, false) => {
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
            ("*mutAtenGeneratorHandle", false, false) => function.optional(
                &ident,
                quote! { Option<&Generator> },
                quote! { #ident.map(|generator| generator.c_generator) },
            ),
            _ => panic!("`{c_name}` takes `{parameter}: {ty}`, which has no Rust type here"),
        }
        index += 1 + usize::from(len) + usize::from(device_index);
    }

    let Function {
        receiver,
        inputs,
        setup,
        arguments,
        results,
    } = function;
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
