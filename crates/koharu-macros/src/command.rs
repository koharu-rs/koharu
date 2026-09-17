use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::parse::Parse;
use syn::punctuated::Punctuated;
use syn::{
    Error, FnArg, GenericArgument, Ident, ItemFn, Pat, Path, PathArguments, PathSegment, Result,
    ReturnType, Token, Type,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum ParamClass {
    State,
    Window,
    Channel,
    Payload,
}

struct Param {
    ident: Ident,
    ty: Type,
    class: ParamClass,
    window_already_option: bool,
    mutable: bool,
}

pub fn expand(attr: TokenStream, item: TokenStream) -> Result<TokenStream> {
    if !attr.is_empty() {
        return Err(Error::new(
            proc_macro2::Span::call_site(),
            "#[command] does not take attributes",
        ));
    }
    let mut item: ItemFn = syn::parse2(item)?;
    let name = item.sig.ident.clone();
    let vis = item.vis.clone();
    let asyncness = item.sig.asyncness;
    let generics = item.sig.generics.clone();
    let where_clause = generics.where_clause.clone();
    let output = item.sig.output.clone();
    let block = *item.block.clone();

    let host_ty: Path = syn::parse_str("::koharu_app::host::Host").expect("default host path");
    let api_result_ty = path_sibling(&host_ty, "ApiResult");
    let ws_sink_fn = path_sibling(&host_ty, "ws_sink");
    let ws_channel_fn = path_sibling(&host_ty, "ws_channel");
    let is_ws = name == "subscribe";

    let mut params = Vec::new();
    for arg in &item.sig.inputs {
        params.push(parse_param(arg)?);
    }

    let docs: Vec<_> = item
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("doc"))
        .cloned()
        .collect();
    item.attrs
        .retain(|attr| !attr.path().is_ident("doc") && !is_path(attr.path(), "command"));
    let body_attrs = item.attrs;

    let payload_params: Vec<_> = params
        .iter()
        .filter(|param| param.class == ParamClass::Payload)
        .collect();
    let emit_payload = !payload_params.is_empty();

    let payload_ident = format_ident!("{}Payload", to_pascal_case(&name.to_string()));
    let private_ident = format_ident!("__{name}");
    let http_ident = format_ident!("{name}_http");
    let route_ident = route_module_ident(&name);
    let name_str = name.to_string();
    let path_str = format!("/rpc/{name_str}");

    let payload_fields: Vec<_> = payload_params
        .iter()
        .map(|param| {
            let ident = &param.ident;
            let ty = &param.ty;
            quote!(pub #ident: #ty)
        })
        .collect();
    let payload_field_idents: Vec<_> = payload_params.iter().map(|param| &param.ident).collect();

    let payload_struct = if emit_payload {
        quote! {
            #[derive(::serde::Deserialize)]
            #[serde(rename_all = "camelCase")]
            #[doc(hidden)]
            #vis struct #payload_ident {
                #(#payload_fields,)*
            }
        }
    } else {
        TokenStream::new()
    };

    let private_args: Vec<_> = params
        .iter()
        .map(|param| {
            let ident = &param.ident;
            let ty = private_type(param);
            if param.mutable {
                quote!(mut #ident: #ty)
            } else {
                quote!(#ident: #ty)
            }
        })
        .collect();

    let private_fn = quote! {
        #[allow(clippy::too_many_arguments)]
        #(#body_attrs)*
        pub(crate) #asyncness fn #private_ident #generics (#(#private_args),*) #output #where_clause #block
    };

    let specta_args: Vec<_> = params
        .iter()
        .filter(|param| matches!(param.class, ParamClass::Payload | ParamClass::Channel))
        .map(|param| {
            let ident = &param.ident;
            let ty = &param.ty;
            quote!(#ident: #ty)
        })
        .collect();

    let ok_ty = result_ok_type(&output);
    let binary = ok_ty.as_ref().is_some_and(is_binary_type);
    let optional_binary = ok_ty
        .as_ref()
        .and_then(|ty| peel_wrapper(ty, "Option"))
        .is_some_and(is_binary_type);
    let specta_output = if optional_binary {
        quote!(-> ())
    } else if let Some(ok) = result_ok_type(&output) {
        quote!(-> #ok)
    } else {
        quote!(#output)
    };
    let specta_fn = quote! {
        #(#docs)*
        #[allow(clippy::too_many_arguments, dead_code, unused_variables)]
        #[specta::specta]
        #vis #asyncness fn #name #generics (#(#specta_args),*) #specta_output #where_clause {
            ::core::unreachable!()
        }
    };

    let http_args: Vec<_> = params
        .iter()
        .map(|param| http_call_arg(param, is_ws, &name, &ws_channel_fn))
        .collect::<Result<_>>()?;
    let http_call = call_private(&private_ident, &http_args, asyncness.is_some());
    let uses_paths = payload_params.iter().any(|param| param.ident == "paths");
    let json_or_multipart_ty = path_sibling(&host_ty, "JsonOrMultipart");
    let binary_response_fn = path_sibling(&host_ty, "binary_response");
    let http_fn = if is_ws {
        let payload_extract = if emit_payload {
            quote! {
                ::axum::extract::Query(payload): ::axum::extract::Query<#payload_ident>,
            }
        } else {
            TokenStream::new()
        };
        let payload_bind = if emit_payload {
            quote! {
                let #payload_ident { #(#payload_field_idents,)* } = payload;
            }
        } else {
            TokenStream::new()
        };
        quote! {
            #[allow(unused_variables)]
            #vis #asyncness fn #http_ident(
                ::axum::extract::State(host): ::axum::extract::State<#host_ty>,
                #payload_extract
                ws: ::axum::extract::ws::WebSocketUpgrade,
            ) -> impl ::axum::response::IntoResponse {
                #payload_bind
                ws.on_upgrade(move |socket| async move {
                    let __ws = #ws_sink_fn(&host, socket);
                    let __result = #http_call;
                    __ws.complete(__result);
                })
            }
        }
    } else {
        let payload_extract = if !emit_payload {
            TokenStream::new()
        } else if uses_paths {
            quote! {
                payload: #json_or_multipart_ty<#payload_ident>,
            }
        } else {
            quote! {
                ::axum::Json(payload): ::axum::Json<#payload_ident>,
            }
        };
        let payload_bind = if !emit_payload {
            TokenStream::new()
        } else if uses_paths {
            quote! {
                let #payload_ident { #(#payload_field_idents,)* } = payload.payload;
                let _uploads = payload.files;
            }
        } else {
            quote! {
                let #payload_ident { #(#payload_field_idents,)* } = payload;
            }
        };
        if binary {
            quote! {
                #[allow(unused_variables)]
                #vis #asyncness fn #http_ident(
                    ::axum::extract::State(host): ::axum::extract::State<#host_ty>,
                    #payload_extract
                ) -> #api_result_ty<::axum::response::Response> {
                    #payload_bind
                    let value = #http_call?;
                    let bytes: ::std::vec::Vec<u8> = ::std::convert::Into::into(value);
                    Ok(::axum::response::Response::builder()
                        .header(
                            ::axum::http::header::CONTENT_TYPE,
                            "application/octet-stream",
                        )
                        .body(::axum::body::Body::from(bytes))
                        .expect("octet-stream response"))
                }
            }
        } else if optional_binary {
            quote! {
                #[allow(unused_variables)]
                #vis #asyncness fn #http_ident(
                    ::axum::extract::State(host): ::axum::extract::State<#host_ty>,
                    #payload_extract
                ) -> #api_result_ty<::axum::response::Response> {
                    #payload_bind
                    match #http_call? {
                        ::core::option::Option::Some(value) => Ok(#binary_response_fn(value)),
                        ::core::option::Option::None => Ok(::axum::response::IntoResponse::into_response(::axum::Json(()))),
                    }
                }
            }
        } else {
            let json_ty = ok_ty.unwrap_or_else(|| syn::parse_str("()").expect("unit type"));
            quote! {
                #[allow(unused_variables)]
                #vis #asyncness fn #http_ident(
                    ::axum::extract::State(host): ::axum::extract::State<#host_ty>,
                    #payload_extract
                ) -> #api_result_ty<::axum::Json<#json_ty>> {
                    #payload_bind
                    Ok(::axum::Json(#http_call?))
                }
            }
        }
    };

    let method_route = if is_ws {
        quote!(::axum::routing::get)
    } else {
        quote!(::axum::routing::post)
    };

    let meta_mod = quote! {
        #[doc(hidden)]
        #vis mod #route_ident {
            pub const PATH: &str = #path_str;

            pub fn router() -> ::axum::Router<#host_ty> {
                ::axum::Router::<#host_ty>::new()
                    .route(PATH, #method_route(super::#http_ident))
            }
        }
    };

    Ok(quote! {
        #payload_struct
        #private_fn
        #specta_fn
        #http_fn
        #meta_mod
    })
}

pub fn expand_routes(input: TokenStream) -> Result<TokenStream> {
    let names = syn::parse2::<PathList>(input)?;
    if names.names.is_empty() {
        return Ok(quote! {
            ::axum::Router::<::koharu_app::host::Host>::new()
        });
    }
    let first = names.names.first().expect("non-empty command list");
    let first_mod = route_module_path(first);
    let merges = names.names.iter().skip(1).map(|name| {
        let module = route_module_path(name);
        quote! { .merge(#module::router()) }
    });
    Ok(quote! {
        {
            #first_mod::router()
            #(#merges)*
        }
    })
}

struct PathList {
    names: Punctuated<Path, Token![,]>,
}

impl Parse for PathList {
    fn parse(input: syn::parse::ParseStream) -> Result<Self> {
        Ok(Self {
            names: Punctuated::parse_terminated(input)?,
        })
    }
}

fn route_module_path(command: &Path) -> Path {
    let mut path = command.clone();
    let last = path
        .segments
        .pop()
        .expect("command path should have a segment");
    path.segments.push(PathSegment {
        ident: route_module_ident(&last.ident),
        arguments: PathArguments::None,
    });
    path
}

fn parse_param(arg: &FnArg) -> Result<Param> {
    let FnArg::Typed(pat_type) = arg else {
        return Err(Error::new_spanned(
            arg,
            "#[command] does not support method receivers",
        ));
    };
    let Pat::Ident(pat_ident) = &*pat_type.pat else {
        return Err(Error::new_spanned(
            &pat_type.pat,
            "#[command] requires named parameters",
        ));
    };
    let ty = (*pat_type.ty).clone();
    let (class, window_already_option) = classify(&ty)?;
    Ok(Param {
        ident: pat_ident.ident.clone(),
        ty,
        class,
        window_already_option,
        mutable: pat_ident.mutability.is_some(),
    })
}

fn classify(ty: &Type) -> Result<(ParamClass, bool)> {
    if is_ident(ty, "AppHandle") {
        return Err(Error::new_spanned(
            ty,
            "Host has no AppHandle; #[command] cannot inject AppHandle",
        ));
    }
    if is_ident(ty, "WebviewWindow") {
        return Ok((ParamClass::Window, false));
    }
    if let Some(inner) = peel_wrapper(ty, "Option")
        && is_ident(inner, "WebviewWindow")
    {
        return Ok((ParamClass::Window, true));
    }
    if is_ident(ty, "Channel") {
        return Ok((ParamClass::Channel, false));
    }
    if is_state_type(ty) {
        return Ok((ParamClass::State, false));
    }
    Ok((ParamClass::Payload, false))
}

fn is_state_type(ty: &Type) -> bool {
    let Some(ident) = last_ident(ty) else {
        return false;
    };
    let name = ident.to_string();
    matches!(
        name.as_str(),
        "Pipeline"
            | "Desktop"
            | "CurrentProject"
            | "Processing"
            | "AgentState"
            | "ProjectLibrary"
            | "Initialization"
            | "Host"
    ) || (name != "Channel" && name.ends_with("Channel"))
}

fn private_type(param: &Param) -> TokenStream {
    let ty = &param.ty;
    match param.class {
        ParamClass::Window if !param.window_already_option => {
            quote!(::core::option::Option<#ty>)
        }
        _ => quote!(#ty),
    }
}

fn http_call_arg(
    param: &Param,
    is_ws: bool,
    command: &Ident,
    ws_channel_fn: &Path,
) -> Result<TokenStream> {
    let ident = &param.ident;
    Ok(match param.class {
        ParamClass::State => {
            if last_ident(&param.ty).is_some_and(|ident| ident == "Host") {
                quote!(host.clone())
            } else {
                let field = host_state_field_ident(param)?;
                quote!(host.#field.clone())
            }
        }
        ParamClass::Window => quote!(host.window()),
        ParamClass::Channel if is_ws => quote!(__ws.channel(stringify!(#ident))),
        ParamClass::Channel => {
            quote!(#ws_channel_fn(&host, stringify!(#command), stringify!(#ident))?)
        }
        ParamClass::Payload => quote!(#ident),
    })
}

fn host_state_field_ident(param: &Param) -> Result<Ident> {
    let ty = &param.ty;
    let type_name = last_ident(ty)
        .map(|ident| ident.to_string())
        .unwrap_or_default();
    let Some(field) = host_state_field_name(&type_name) else {
        return Err(Error::new_spanned(
            ty,
            format!("unknown Host state type `{type_name}`"),
        ));
    };
    Ok(Ident::new(field, param.ident.span()))
}

fn host_state_field_name(type_name: &str) -> Option<&'static str> {
    Some(match type_name {
        "CurrentProject" => "project",
        "ProjectLibrary" => "library",
        "Processing" => "processing",
        "CanvasChannel" => "canvas",
        "JobChannel" => "jobs",
        "DownloadChannel" => "downloads",
        "ResourceChannel" => "resources",
        "ProjectChannel" => "project_channel",
        "Initialization" => "initialization",
        "Desktop" => "desktop",
        "AgentState" => "state",
        "Pipeline" => "pipeline",
        _ => return None,
    })
}

fn call_private(private: &Ident, args: &[TokenStream], is_async: bool) -> TokenStream {
    if is_async {
        quote!(#private(#(#args),*).await)
    } else {
        quote!(#private(#(#args),*))
    }
}

fn result_ok_type(output: &ReturnType) -> Option<Type> {
    let ReturnType::Type(_, ty) = output else {
        return None;
    };
    let Type::Path(path) = ty.as_ref() else {
        return None;
    };
    let last = path.path.segments.last()?;
    if last.ident != "Result" {
        return Some(*ty.clone());
    }
    let PathArguments::AngleBracketed(args) = &last.arguments else {
        return Some(*ty.clone());
    };
    args.args.iter().find_map(|arg| match arg {
        GenericArgument::Type(ty) => Some(ty.clone()),
        _ => None,
    })
}

fn is_binary_type(ty: &Type) -> bool {
    last_ident(ty).is_some_and(|ident| ident.to_string().ends_with("Bytes"))
}

fn path_sibling(host: &Path, ident: &str) -> Path {
    let mut path = host.clone();
    path.segments.pop();
    path.segments.push(PathSegment {
        ident: Ident::new(ident, proc_macro2::Span::call_site()),
        arguments: PathArguments::None,
    });
    path
}

fn peel_wrapper<'a>(ty: &'a Type, wrapper: &str) -> Option<&'a Type> {
    let segment = last_segment(ty)?;
    if segment.ident != wrapper {
        return None;
    }
    inner_type(segment)
}

fn is_ident(ty: &Type, name: &str) -> bool {
    last_ident(ty).is_some_and(|ident| ident == name)
}

fn last_ident(ty: &Type) -> Option<&Ident> {
    last_segment(ty).map(|segment| &segment.ident)
}

fn last_segment(ty: &Type) -> Option<&PathSegment> {
    match ty {
        Type::Path(ty) => ty.path.segments.last(),
        Type::Group(ty) => last_segment(&ty.elem),
        Type::Paren(ty) => last_segment(&ty.elem),
        Type::Reference(ty) => last_segment(&ty.elem),
        _ => None,
    }
}

fn inner_type(segment: &PathSegment) -> Option<&Type> {
    let PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    args.args.iter().find_map(|arg| match arg {
        GenericArgument::Type(ty) => Some(ty),
        _ => None,
    })
}

fn is_path(path: &syn::Path, name: &str) -> bool {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
        == name
}

fn route_module_ident(name: &Ident) -> Ident {
    format_ident!("__{name}_route")
}

fn to_pascal_case(name: &str) -> String {
    let mut out = String::new();
    let mut capitalize = true;
    for ch in name.chars() {
        if ch == '_' {
            capitalize = true;
        } else if capitalize {
            out.extend(ch.to_uppercase());
            capitalize = false;
        } else {
            out.push(ch);
        }
    }
    out
}
