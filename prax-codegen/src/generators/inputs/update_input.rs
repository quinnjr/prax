//! Generate `<Model>UpdateInput` — flat scalar fields wrapped in
//! `*FieldUpdate` wrappers, plus an `UpdateInput` trait impl that
//! lowers each wrapper to a `(column, WriteOp)` pair on the runtime
//! [`prax_query::inputs::UpdatePayload`] shape.
//!
//! Atomic operators (`increment`/`decrement`/`multiply`/`divide`) on
//! the wrappers map to the matching [`prax_query::inputs::WriteOp`]
//! variants; nullable-only `unset` maps to `WriteOp::Unset`. The plain
//! `set` field becomes `WriteOp::Set(v.into())` using the value's
//! `Into<FilterValue>` impl.
//!
//! ## Visibility note
//!
//! Same as [`super::create_input`]: the trait impl is emitted outside
//! the per-model `pub mod` to avoid E0446. See [`UpdateInputTokens`].

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Ident;

use crate::generators::inputs::{FilterCategory, update_wrapper_ident};

pub struct UpdateField {
    /// Field name in the source code.
    pub name: Ident,
    /// SQL column name (used in the `(column, WriteOp)` payload).
    pub column: String,
    /// Filter category for the wrapper selection.
    pub category: FilterCategory,
    /// Whether the field is nullable (selects `*NullableFieldUpdate`).
    pub nullable: bool,
    /// For enum columns: the enum's PascalCase ident. Required for
    /// `EnumFieldUpdate<E>` instantiation.
    pub enum_ident: Option<Ident>,
}

/// Output of the update-input generator.
///
/// Same struct/impl split as [`super::create_input::CreateInputTokens`].
pub struct UpdateInputTokens {
    /// `pub struct <Model>UpdateInput { ... }` definition.
    pub struct_tokens: TokenStream,
    /// `impl UpdateInput for <Model>UpdateInput { ... }` impl.
    pub impl_tokens: TokenStream,
}

/// True for `FilterCategory` values whose wrapper carries arithmetic
/// operators (`increment` / `decrement` / `multiply` / `divide`).
fn category_has_arithmetic(cat: FilterCategory) -> bool {
    matches!(
        cat,
        FilterCategory::Int
            | FilterCategory::BigInt
            | FilterCategory::Float
            | FilterCategory::Decimal
    )
}

pub fn generate(
    model_ident: &Ident,
    module_name: &Ident,
    fields: &[UpdateField],
) -> UpdateInputTokens {
    let update_ident = format_ident!("{}UpdateInput", model_ident);

    let field_decls = fields.iter().map(|f| {
        let n = &f.name;
        let wrapper = update_wrapper_ident(f.category, f.nullable);
        let doc = match f.category {
            FilterCategory::Date => Some(
                "Date column. The wrapper expects an `Option<String>` \
                 formatted as `YYYY-MM-DD`; `DateTimeFieldUpdate` is \
                 shared across Date/Time/DateTime by design.",
            ),
            FilterCategory::Time => Some(
                "Time column. The wrapper expects an `Option<String>` \
                 formatted as `HH:MM:SS`; `DateTimeFieldUpdate` is \
                 shared across Date/Time/DateTime by design.",
            ),
            _ => None,
        };
        let doc_attr = doc.map(|d| quote! { #[doc = #d] });
        if matches!(f.category, FilterCategory::Enum) {
            let e = f
                .enum_ident
                .as_ref()
                .expect("enum field requires enum ident");
            quote! {
                #doc_attr
                pub #n: ::core::option::Option<::prax_query::inputs::#wrapper<#e>>
            }
        } else {
            quote! {
                #doc_attr
                pub #n: ::core::option::Option<::prax_query::inputs::#wrapper>
            }
        }
    });

    let struct_tokens = quote! {
        #[derive(Debug, Clone, Default, ::serde::Serialize, ::serde::Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub struct #update_ident {
            #(#field_decls,)*
        }
    };

    // Per-field lowering: inspect each Option<*FieldUpdate> wrapper
    // and append the matching (column, WriteOp) entry. Set/Unset are
    // available on every wrapper; the arithmetic variants are gated
    // on a numeric category to keep the generated match arms tight.
    let lowerings: Vec<TokenStream> = fields
        .iter()
        .filter_map(|f| {
            if matches!(f.category, FilterCategory::Enum) && f.enum_ident.is_none() {
                return None;
            }
            let n = &f.name;
            let col = &f.column;

            // Arithmetic operators emit only for numeric categories;
            // wrappers for non-numeric scalars don't expose those
            // fields. Emitting the arms anyway would not compile.
            let arithmetic_arms = if category_has_arithmetic(f.category) {
                quote! {
                    if let ::core::option::Option::Some(__v) = __w.increment {
                        __out.push((
                            ::std::string::String::from(#col),
                            ::prax_query::inputs::WriteOp::Increment(
                                ::core::convert::Into::<
                                    ::prax_query::filter::FilterValue
                                >::into(__v),
                            ),
                        ));
                    }
                    if let ::core::option::Option::Some(__v) = __w.decrement {
                        __out.push((
                            ::std::string::String::from(#col),
                            ::prax_query::inputs::WriteOp::Decrement(
                                ::core::convert::Into::<
                                    ::prax_query::filter::FilterValue
                                >::into(__v),
                            ),
                        ));
                    }
                    if let ::core::option::Option::Some(__v) = __w.multiply {
                        __out.push((
                            ::std::string::String::from(#col),
                            ::prax_query::inputs::WriteOp::Multiply(
                                ::core::convert::Into::<
                                    ::prax_query::filter::FilterValue
                                >::into(__v),
                            ),
                        ));
                    }
                    if let ::core::option::Option::Some(__v) = __w.divide {
                        __out.push((
                            ::std::string::String::from(#col),
                            ::prax_query::inputs::WriteOp::Divide(
                                ::core::convert::Into::<
                                    ::prax_query::filter::FilterValue
                                >::into(__v),
                            ),
                        ));
                    }
                }
            } else {
                quote! {}
            };

            let unset_arm = if f.nullable {
                quote! {
                    if let ::core::option::Option::Some(true) = __w.unset {
                        __out.push((
                            ::std::string::String::from(#col),
                            ::prax_query::inputs::WriteOp::Unset,
                        ));
                    }
                }
            } else {
                quote! {}
            };

            Some(quote! {
                if let ::core::option::Option::Some(__w) = self.#n {
                    if let ::core::option::Option::Some(__v) = __w.set {
                        __out.push((
                            ::std::string::String::from(#col),
                            ::prax_query::inputs::WriteOp::Set(
                                ::core::convert::Into::<
                                    ::prax_query::filter::FilterValue
                                >::into(__v),
                            ),
                        ));
                    }
                    #arithmetic_arms
                    #unset_arm
                }
            })
        })
        .collect();

    let impl_tokens = quote! {
        impl ::prax_query::inputs::UpdateInput for #module_name::#update_ident {
            type Model = #model_ident;
            type Data = ::prax_query::inputs::UpdatePayload;
            fn into_ir(self) -> Self::Data {
                let mut __out: ::prax_query::inputs::UpdatePayload =
                    ::std::vec::Vec::new();
                #(#lowerings)*
                __out
            }
        }
    };

    UpdateInputTokens {
        struct_tokens,
        impl_tokens,
    }
}
