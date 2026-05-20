//! Lower DSL scalar-filter blocks to typed scalar filter wrapper
//! constructors.
//!
//! Given a scalar field (with its declared type and nullability) and a
//! [`DslValue`], emit a `TokenStream` that constructs the right
//! `prax_query::inputs::*Filter` literal.

#![allow(dead_code)]

use prax_schema::{FieldType, ScalarType};
use proc_macro2::TokenStream;
use quote::quote;

use crate::generators::inputs::{FilterCategory, filter_wrapper_ident, scalar_payload_type};
use crate::macros::dsl::ast::{DslBlock, DslField, DslValue};

/// Map a scalar schema-type to the codegen `FilterCategory`.
pub(crate) fn category_for_scalar(s: &ScalarType) -> Option<FilterCategory> {
    Some(match s {
        ScalarType::String => FilterCategory::String,
        ScalarType::Int => FilterCategory::Int,
        ScalarType::BigInt => FilterCategory::BigInt,
        ScalarType::Float => FilterCategory::Float,
        ScalarType::Decimal => FilterCategory::Decimal,
        ScalarType::Boolean => FilterCategory::Bool,
        ScalarType::DateTime => FilterCategory::DateTime,
        ScalarType::Date => FilterCategory::Date,
        ScalarType::Time => FilterCategory::Time,
        ScalarType::Json => FilterCategory::Json,
        ScalarType::Bytes => FilterCategory::Bytes,
        ScalarType::Uuid
        | ScalarType::Cuid
        | ScalarType::Cuid2
        | ScalarType::NanoId
        | ScalarType::Ulid => FilterCategory::Uuid,
        // Extension / unsupported types fall through to `None`. The
        // caller emits a clear diagnostic in that case.
        _ => return None,
    })
}

/// Lower a scalar filter value to a `TokenStream` that constructs the
/// right `*Filter` wrapper.
///
/// Accepts:
/// - `DslValue::Lit(lit)` → `Wrapper::equals(<lit>)` shorthand.
/// - `DslValue::Block(inner)` → struct literal of the wrapper.
/// - `DslValue::BareIdent` / `DslValue::Path` for enums.
/// - `DslValue::Expr(expr)` → `Wrapper::from(<expr>)`.
pub fn lower_scalar_filter(
    field_name: &str,
    field_type: &FieldType,
    nullable: bool,
    value: &DslValue,
) -> syn::Result<TokenStream> {
    match field_type {
        FieldType::Scalar(s) => {
            let cat = category_for_scalar(s).ok_or_else(|| {
                syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!(
                        "scalar type {:?} for field `{}` has no DSL lowering yet",
                        s, field_name
                    ),
                )
            })?;
            lower_typed_filter(field_name, cat, nullable, value)
        }
        FieldType::Enum(enum_name) => {
            lower_enum_filter(field_name, enum_name.as_str(), nullable, value)
        }
        FieldType::Composite(name) | FieldType::Unsupported(name) => Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!(
                "field `{}` has unsupported type `{}` in the read DSL",
                field_name, name
            ),
        )),
        FieldType::Model(_) => Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!(
                "field `{}` is a relation; use relation operators",
                field_name
            ),
        )),
    }
}

fn lower_typed_filter(
    field_name: &str,
    cat: FilterCategory,
    nullable: bool,
    value: &DslValue,
) -> syn::Result<TokenStream> {
    let wrapper_ident = filter_wrapper_ident(cat, nullable);

    match value {
        DslValue::Lit(lit) => Ok(quote! {
            ::prax_query::inputs::#wrapper_ident {
                equals: ::core::option::Option::Some(::core::convert::Into::into(#lit)),
                ..::core::default::Default::default()
            }
        }),
        DslValue::Bool(b) => {
            // Only Bool fields accept a bare `true`/`false` directly.
            if !matches!(cat, FilterCategory::Bool) {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!(
                        "field `{}` (category {:?}) does not accept a bare bool",
                        field_name, cat
                    ),
                ));
            }
            Ok(quote! {
                ::prax_query::inputs::#wrapper_ident {
                    equals: ::core::option::Option::Some(#b),
                    ..::core::default::Default::default()
                }
            })
        }
        DslValue::Expr(expr) => Ok(quote! {
            <::prax_query::inputs::#wrapper_ident as ::core::convert::From<_>>::from(#expr)
        }),
        DslValue::Path(p) => Ok(quote! {
            <::prax_query::inputs::#wrapper_ident as ::core::convert::From<_>>::from(#p)
        }),
        DslValue::BareIdent(id) => Err(syn::Error::new(
            id.span(),
            format!(
                "bare identifier `{}` is not allowed here; field `{}` is not an enum",
                id, field_name
            ),
        )),
        DslValue::List(_) => Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!(
                "field `{}` expects a scalar filter, not a list. Use `in_list: [...]` inside `{{ ... }}` for IN-style filters.",
                field_name
            ),
        )),
        DslValue::Block(block) => lower_filter_block(field_name, cat, &wrapper_ident, block),
    }
}

fn lower_filter_block(
    field_name: &str,
    cat: FilterCategory,
    wrapper_ident: &syn::Ident,
    block: &DslBlock,
) -> syn::Result<TokenStream> {
    let payload_ty = scalar_payload_type(cat);
    let mut field_setters: Vec<TokenStream> = Vec::new();

    for entry in &block.fields {
        let DslField::Pair { key, value, .. } = entry else {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!(
                    "scalar filter for `{}` does not support spread or conditional fields yet",
                    field_name
                ),
            ));
        };
        let op = key.to_string();
        match op.as_str() {
            "equals" | "lt" | "lte" | "gt" | "gte" | "contains" | "starts_with" | "ends_with" => {
                let v_tokens = lower_scalar_value(value, payload_ty.as_ref())?;
                let op_ident = quote::format_ident!("{op}");
                field_setters.push(quote! {
                    #op_ident: ::core::option::Option::Some(#v_tokens)
                });
            }
            "in_list" | "not_in" => {
                let DslValue::List(items) = value else {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("`{}` expects a list value (e.g. [1, 2, 3])", op),
                    ));
                };
                let item_tokens: Vec<TokenStream> = items
                    .iter()
                    .map(|i| lower_scalar_value(i, payload_ty.as_ref()))
                    .collect::<syn::Result<_>>()?;
                let op_ident = quote::format_ident!("{op}");
                field_setters.push(quote! {
                    #op_ident: ::core::option::Option::Some(::std::vec![ #(#item_tokens),* ])
                });
            }
            "not" => {
                // Recurse: `not: { ... }` lowers to Some(Box::new(inner)).
                let DslValue::Block(inner_block) = value else {
                    return Err(syn::Error::new(
                        key.span(),
                        "`not` expects a `{ ... }` block",
                    ));
                };
                let inner = lower_filter_block(field_name, cat, wrapper_ident, inner_block)?;
                field_setters.push(quote! {
                    not: ::core::option::Option::Some(::std::boxed::Box::new(#inner))
                });
            }
            "mode" => {
                // mode: insensitive | default
                let mode_ident = match value {
                    DslValue::BareIdent(id) => id.clone(),
                    DslValue::Path(p) => p
                        .segments
                        .last()
                        .map(|s| s.ident.clone())
                        .ok_or_else(|| syn::Error::new(key.span(), "empty mode path"))?,
                    _ => {
                        return Err(syn::Error::new(
                            key.span(),
                            "`mode` expects `insensitive` or `default`",
                        ));
                    }
                };
                let mode_pascal = match mode_ident.to_string().as_str() {
                    "insensitive" | "Insensitive" => quote::format_ident!("Insensitive"),
                    "default" | "Default" => quote::format_ident!("Default"),
                    other => {
                        return Err(syn::Error::new(
                            mode_ident.span(),
                            format!("unknown mode `{other}` — expected `insensitive` or `default`"),
                        ));
                    }
                };
                field_setters.push(quote! {
                    mode: ::core::option::Option::Some(::prax_query::inputs::QueryMode::#mode_pascal)
                });
            }
            "is_null" => {
                let DslValue::Bool(b) = value else {
                    return Err(syn::Error::new(
                        key.span(),
                        "`is_null` expects `true` or `false`",
                    ));
                };
                field_setters.push(quote! {
                    is_null: ::core::option::Option::Some(#b)
                });
            }
            other => {
                return Err(syn::Error::new(
                    key.span(),
                    format!("unknown scalar-filter operator `{other}` on `{field_name}`"),
                ));
            }
        }
    }

    Ok(quote! {
        ::prax_query::inputs::#wrapper_ident {
            #(#field_setters,)*
            ..::core::default::Default::default()
        }
    })
}

fn lower_scalar_value(
    value: &DslValue,
    _payload_ty: Option<&TokenStream>,
) -> syn::Result<TokenStream> {
    match value {
        DslValue::Lit(lit) => Ok(quote! { ::core::convert::Into::into(#lit) }),
        DslValue::Bool(b) => Ok(quote! { #b }),
        DslValue::Expr(expr) => Ok(quote! { ::core::convert::Into::into(#expr) }),
        DslValue::Path(p) => Ok(quote! { ::core::convert::Into::into(#p) }),
        DslValue::BareIdent(id) => Ok(quote! { ::core::convert::Into::into(#id) }),
        DslValue::Block(_) => Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "nested block is not a valid scalar operator value",
        )),
        DslValue::List(_) => Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "list is not a valid scalar operator value",
        )),
    }
}

fn lower_enum_filter(
    field_name: &str,
    enum_name: &str,
    nullable: bool,
    value: &DslValue,
) -> syn::Result<TokenStream> {
    let wrapper_ident = if nullable {
        quote::format_ident!("EnumNullableFilter")
    } else {
        quote::format_ident!("EnumFilter")
    };
    let enum_ident = quote::format_ident!("{enum_name}");

    match value {
        DslValue::BareIdent(variant) => Ok(quote! {
            ::prax_query::inputs::#wrapper_ident::<#enum_ident> {
                equals: ::core::option::Option::Some(#enum_ident::#variant),
                ..::core::default::Default::default()
            }
        }),
        DslValue::Path(p) => Ok(quote! {
            ::prax_query::inputs::#wrapper_ident::<#enum_ident> {
                equals: ::core::option::Option::Some(#p),
                ..::core::default::Default::default()
            }
        }),
        DslValue::Expr(expr) => Ok(quote! {
            ::prax_query::inputs::#wrapper_ident::<#enum_ident> {
                equals: ::core::option::Option::Some(#expr),
                ..::core::default::Default::default()
            }
        }),
        DslValue::Block(inner) => {
            // Block form: { equals: Admin, not_in: [Banned, Locked], ... }
            let mut setters: Vec<TokenStream> = Vec::new();
            for entry in &inner.fields {
                let DslField::Pair { key, value, .. } = entry else {
                    return Err(syn::Error::new(
                        proc_macro2::Span::call_site(),
                        "enum filter does not support spread or conditional fields",
                    ));
                };
                let op = key.to_string();
                match op.as_str() {
                    "equals" => {
                        let v = lower_enum_variant_value(value, &enum_ident)?;
                        setters.push(quote! { equals: ::core::option::Option::Some(#v) });
                    }
                    "in_list" | "not_in" => {
                        let DslValue::List(items) = value else {
                            return Err(syn::Error::new(
                                key.span(),
                                format!("`{op}` expects a list value"),
                            ));
                        };
                        let item_tokens: Vec<TokenStream> = items
                            .iter()
                            .map(|i| lower_enum_variant_value(i, &enum_ident))
                            .collect::<syn::Result<_>>()?;
                        let op_ident = quote::format_ident!("{op}");
                        setters.push(quote! {
                            #op_ident: ::core::option::Option::Some(::std::vec![ #(#item_tokens),* ])
                        });
                    }
                    "is_null" => {
                        if !nullable {
                            return Err(syn::Error::new(
                                key.span(),
                                format!("field `{field_name}` is not nullable; `is_null` invalid"),
                            ));
                        }
                        let DslValue::Bool(b) = value else {
                            return Err(syn::Error::new(
                                key.span(),
                                "`is_null` expects `true` or `false`",
                            ));
                        };
                        setters.push(quote! {
                            is_null: ::core::option::Option::Some(#b)
                        });
                    }
                    other => {
                        return Err(syn::Error::new(
                            key.span(),
                            format!("unknown enum-filter operator `{other}`"),
                        ));
                    }
                }
            }
            Ok(quote! {
                ::prax_query::inputs::#wrapper_ident::<#enum_ident> {
                    #(#setters,)*
                    ..::core::default::Default::default()
                }
            })
        }
        DslValue::Lit(_) | DslValue::List(_) | DslValue::Bool(_) => Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!(
                "field `{field_name}` is enum-typed; expected a variant ident or block, got a literal"
            ),
        )),
    }
}

fn lower_enum_variant_value(value: &DslValue, enum_ident: &syn::Ident) -> syn::Result<TokenStream> {
    match value {
        DslValue::BareIdent(v) => Ok(quote! { #enum_ident::#v }),
        DslValue::Path(p) => Ok(quote! { #p }),
        DslValue::Expr(e) => Ok(quote! { #e }),
        _ => Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "expected an enum variant ident here",
        )),
    }
}
