#![allow(clippy::doc_markdown, clippy::module_name_repetitions, missing_docs)]
use std::str::FromStr;

use log::Level;
use proc_macro::TokenStream;
use proc_macro_error::{abort, abort_call_site, proc_macro_error};
use quote::quote;
use syn::{
    spanned::Spanned, AttributeArgs, FieldPat, FnArg, Ident, ItemFn, Lit, NestedMeta, Pat,
    PatIdent, PatReference, PatStruct, PatTuple, PatTupleStruct, PatType, Signature,
};

const SKIP_FROM_ATTR: &str = "skip_from";

#[proc_macro_error]
#[proc_macro_attribute]
pub fn log(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input: ItemFn = syn::parse_macro_input!(item as ItemFn);
    let args = syn::parse_macro_input!(attr as AttributeArgs);
    if args.len() > 1 {
        abort_call_site!(format!(
            "Unexpected number of arguments: 1 or 0 arguments expected, got {}",
            args.len()
        ))
    }
    let log_level = args.first().map_or(Level::Debug, |nested_meta| {
        if let NestedMeta::Lit(Lit::Str(lit_str)) = nested_meta {
            Level::from_str(&lit_str.value()).expect("Failed to parse log level.")
        } else {
            abort!(nested_meta, "Invalid argument. String expected.")
        }
    });
    let log_level = format!("{}", log_level);
    let ItemFn {
        attrs,
        vis,
        block,
        sig,
        ..
    } = input;
    let Signature {
        output: return_type,
        inputs: params,
        unsafety,
        asyncness,
        constness,
        abi,
        ident,
        generics:
            syn::Generics {
                params: gen_params,
                where_clause,
                ..
            },
        ..
    } = sig;
    let param_names: Vec<_> = params
        .clone()
        .into_iter()
        .flat_map(|param| match param {
            FnArg::Typed(PatType { pat, .. }) => param_names(*pat),
            FnArg::Receiver(_) => Box::new(std::iter::once(Ident::new("self", param.span()))),
        })
        .map(|item| quote!(log::log!(log_level, "{} = {:?}, ", stringify!(#item), &#item);))
        .collect();
    let arguments = quote!(#(#param_names)*);
    let ident_str = ident.to_string();
    quote!(
		#[allow(clippy::used_underscore_binding)]
        #(#attrs) *
        #vis #constness #unsafety #asyncness #abi fn #ident<#gen_params>(#params) #return_type
        #where_clause
        {
            let log_level = <log::Level as std::str::FromStr>::from_str(#log_level).expect("Failed to parse log level.");
            log::log!(log_level, "{}[start]: ",
                #ident_str,
            );
            #arguments
            let result = #block;
            log::log!(log_level, "{}[end]: {:?}",
                #ident_str,
                &result
            );
            result
        }
    )
    .into()
}

#[proc_macro_derive(Io)]
pub fn io_derive(input: TokenStream) -> TokenStream {
    let ast = syn::parse(input).expect("Failed to parse input Token Stream.");
    impl_io(&ast)
}

#[proc_macro_derive(IntoContract)]
pub fn into_contract_derive(input: TokenStream) -> TokenStream {
    let ast = syn::parse(input).expect("Failed to parse input Token Stream.");
    impl_into_contract(&ast)
}

#[proc_macro_derive(IntoQuery)]
pub fn into_query_derive(input: TokenStream) -> TokenStream {
    let ast = syn::parse(input).expect("Failed to parse input Token Stream.");
    impl_into_query(&ast)
}

/// `FromVariant` is used for implementing `From<Variant> for Enum` and `TryFrom<Enum> for Variant`.
///
/// ```rust
/// use iroha_derive::FromVariant;
///
/// #[derive(FromVariant)]
/// enum Obj {
///     Uint(u32),
///     Int(i32),
///     String(String),
///     // You can also skip implementing `From`
///     Vec(#[skip_from] Vec<Obj>),
/// }
///
/// // For example for avoid cases like this:
/// impl<T: Into<Obj>> From<Vec<T>> for Obj {
///     fn from(vec: Vec<T>) -> Self {
///         # stringify!(
///         ...
///         # );
///         # todo!()
///     }
/// }
/// ```
#[proc_macro_derive(FromVariant, attributes(skip_from))]
pub fn from_variant_derive(input: TokenStream) -> TokenStream {
    let ast = syn::parse(input).expect("Failed to parse input Token Stream.");
    impl_from_variant(&ast)
}

fn attrs_have_ident(attrs: &[syn::Attribute], ident: &str) -> bool {
    attrs.iter().any(|attr| attr.path.is_ident(ident))
}

fn param_names(pat: Pat) -> Box<dyn Iterator<Item = Ident>> {
    match pat {
        Pat::Ident(PatIdent { ident, .. }) => Box::new(std::iter::once(ident)),
        Pat::Reference(PatReference { pat, .. }) => param_names(*pat),
        Pat::Struct(PatStruct { fields, .. }) => Box::new(
            fields
                .into_iter()
                .flat_map(|FieldPat { pat, .. }| param_names(*pat)),
        ),
        Pat::Tuple(PatTuple { elems, .. })
        | Pat::TupleStruct(PatTupleStruct {
            pat: PatTuple { elems, .. },
            ..
        }) => Box::new(elems.into_iter().flat_map(param_names)),
        _ => Box::new(std::iter::empty()),
    }
}

fn impl_io(ast: &syn::DeriveInput) -> TokenStream {
    let name = &ast.ident;
    let gen = quote! {

        impl std::convert::From<#name> for Vec<u8> {
            fn from(origin: #name) -> Self {
                origin.encode()
            }
        }

        impl std::convert::From<&#name> for Vec<u8> {
            fn from(origin: &#name) -> Self {
                origin.encode()
            }
        }

        impl std::convert::TryFrom<Vec<u8>> for #name {
            type Error = iroha_macro::error::Error;

            fn try_from(vector: Vec<u8>) -> iroha_macro::error::Result<Self> {
                use iroha_macro::error::WrapErr;
                #name::decode(&mut vector.as_slice())
                    .wrap_err_with(|| format!(
                            "Failed to deserialize vector {:?} into {}.",
                            &vector,
                            stringify!(#name),
                        )
                    )
            }
        }
    };
    gen.into()
}

const CONTAINERS: &[&str] = &["Box", "RefCell", "Cell", "Rc", "Arc", "Mutex", "RwLock"];

fn get_type_argument<'a, 'b>(
    s: &'a str,
    ty: &'b syn::TypePath,
) -> Option<&'b syn::GenericArgument> {
    let segments = &ty.path.segments;
    if segments.len() != 1 || segments[0].ident != s {
        return None;
    }

    if let syn::PathArguments::AngleBracketed(ref bracketed_arguments) = segments[0].arguments {
        assert_eq!(bracketed_arguments.args.len(), 1);
        Some(&bracketed_arguments.args[0])
    } else {
        unreachable!("No other arguments for types in enum variants possible")
    }
}

fn from_container_variant_internal(
    into_ty: &syn::Ident,
    into_variant: &syn::Ident,
    from_ty: &syn::GenericArgument,
    container_ty: &syn::TypePath,
) -> proc_macro2::TokenStream {
    quote! {
        impl std::convert::From<#from_ty> for #into_ty {
            fn from(origin: #from_ty) -> Self {
                #into_ty :: #into_variant (#container_ty :: new(origin))
            }
        }
    }
}

fn from_variant_internal(
    into_ty: &syn::Ident,
    into_variant: &syn::Ident,
    from_ty: &syn::Type,
) -> proc_macro2::TokenStream {
    quote! {
        impl std::convert::From<#from_ty> for #into_ty {
            fn from(origin: #from_ty) -> Self {
                #into_ty :: #into_variant (origin)
            }
        }
    }
}

fn from_variant(
    into_ty: &syn::Ident,
    into_variant: &syn::Ident,
    from_ty: &syn::Type,
) -> proc_macro2::TokenStream {
    let from_orig = from_variant_internal(into_ty, into_variant, from_ty);

    if let syn::Type::Path(path) = from_ty {
        let mut code = from_orig;

        for container in CONTAINERS {
            if let Some(inner) = get_type_argument(container, path) {
                let segments = path
                    .path
                    .segments
                    .iter()
                    .map(|segment| {
                        let mut segment = segment.clone();
                        segment.arguments = syn::PathArguments::default();
                        segment
                    })
                    .collect::<syn::punctuated::Punctuated<_, syn::token::Colon2>>();
                let path = syn::Path {
                    segments,
                    leading_colon: None,
                };
                let path = &syn::TypePath { path, qself: None };

                let from_inner =
                    from_container_variant_internal(into_ty, into_variant, inner, path);
                code = quote! {
                    #code
                    #from_inner
                };
            }
        }

        return code;
    }

    from_orig
}

fn try_into_variant(
    enum_ty: &syn::Ident,
    variant: &syn::Ident,
    variant_ty: &syn::Type,
) -> proc_macro2::TokenStream {
    quote! {
        impl std::convert::TryFrom<#enum_ty> for #variant_ty {
            type Error = iroha_macro::error::ErrorTryFromEnum<#enum_ty, Self>;

            fn try_from(origin: #enum_ty) -> std::result::Result<Self, iroha_macro::error::ErrorTryFromEnum<#enum_ty, Self>> {
                if let #enum_ty :: #variant(variant) = origin {
                    Ok(variant)
                } else {
                    Err(iroha_macro::error::ErrorTryFromEnum::default())
                }
            }
        }
    }
}

fn impl_from_variant(ast: &syn::DeriveInput) -> TokenStream {
    let name = &ast.ident;

    let froms = if let syn::Data::Enum(ref data_enum) = ast.data {
        &data_enum.variants
    } else {
        panic!("Only enums are supported")
    }
    .iter()
    .filter_map(|variant| {
        if let syn::Fields::Unnamed(ref unnamed) = variant.fields {
            if unnamed.unnamed.len() == 1 {
                let variant_type = &unnamed
                    .unnamed
                    .first()
                    .expect("Won't fail as we have more than  one argument for variant")
                    .ty;
                let try_into = try_into_variant(name, &variant.ident, variant_type);
                let from = if attrs_have_ident(&unnamed.unnamed[0].attrs, SKIP_FROM_ATTR) {
                    quote!()
                } else {
                    from_variant(name, &variant.ident, variant_type)
                };

                return Some(quote!(
                    #try_into
                    #from
                ));
            }
        }
        None
    });

    let gen = quote! {
        #(#froms)*
    };
    gen.into()
}

fn impl_into_contract(ast: &syn::DeriveInput) -> TokenStream {
    let name = &ast.ident;
    let gen = quote! {

        impl std::convert::From<#name> for Contract {
            fn from(origin: #name) -> Self {
                Contract::#name(origin)
            }
        }
    };
    gen.into()
}

fn impl_into_query(ast: &syn::DeriveInput) -> TokenStream {
    let name = &ast.ident;
    let gen = quote! {

        impl std::convert::From<#name> for IrohaQuery {
            fn from(origin: #name) -> Self {
                IrohaQuery::#name(origin)
            }
        }
    };
    gen.into()
}