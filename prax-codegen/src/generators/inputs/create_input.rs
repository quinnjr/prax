//! Generate `<Model>CreateInput` — flat scalar fields + a
//! `CreateInput` trait impl that lowers the struct to the runtime
//! [`prax_query::inputs::CreatePayload`] shape (`Vec<(column, value)>`).
//!
//! Nested writes (`connect`/`create`/`disconnect`/etc.) are deferred to
//! phase 5b. Until then phase 5a's codegen rejects relation keys
//! inside `data:` with a "phase 5b" diagnostic before reaching the
//! input lowering.
//!
//! ## Visibility note
//!
//! The trait impl emits `type Model = <Model>` to the per-model
//! struct. To avoid E0446 ("private type in public interface") when
//! the parent module is private the impl is emitted outside the
//! `pub mod <model>` block — see [`CreateInputTokens`].

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Ident;

use crate::generators::inputs::{FilterCategory, scalar_payload_type};

pub struct CreateField {
    /// Field name in the source code.
    pub name: Ident,
    /// SQL column name (used in the `(column, value)` payload).
    pub column: String,
    /// Filter category for the scalar payload.
    pub category: FilterCategory,
    /// Whether the field is `Option<T>` (nullable).
    pub nullable: bool,
    /// Whether the field has a default (Option-wrap so callers can omit).
    pub has_default: bool,
    /// For enum columns: the enum's PascalCase ident.
    pub enum_ident: Option<Ident>,
}

/// Output of the create-input generator.
///
/// Same split as [`super::where_input::WhereInputTokens`]:
/// `struct_tokens` goes inside the per-model `pub mod`; `impl_tokens`
/// is emitted at crate-root scope so the trait impl can reference the
/// model struct without leaking it through `super::<Model>`.
pub struct CreateInputTokens {
    /// `pub struct <Model>CreateInput { ... }` definition.
    pub struct_tokens: TokenStream,
    /// `impl CreateInput for <Model>CreateInput { ... }` impl.
    pub impl_tokens: TokenStream,
}

pub fn generate(
    model_ident: &Ident,
    module_name: &Ident,
    fields: &[CreateField],
) -> CreateInputTokens {
    let create_ident = format_ident!("{}CreateInput", model_ident);

    let field_decls = fields.iter().filter_map(|f| {
        let n = &f.name;
        let payload = match &f.enum_ident {
            Some(e) => quote! { #e },
            None => scalar_payload_type(f.category)?,
        };
        Some(if f.nullable || f.has_default {
            quote! { pub #n: ::core::option::Option<#payload> }
        } else {
            quote! { pub #n: #payload }
        })
    });

    let create_ident_doc = format!(
        "Create-time input for a `{}`.\n\n\
         **Warning:** this type derives `Default`. Calling `{}::default()` \
         produces zero-valued required scalar fields (`String::new()`, `0`, \
         `false`). Use struct-literal syntax for safety, or call \
         `Default::default()` only when you know every required field will \
         be overridden downstream. A strict variant is planned for the \
         operation rework.",
        create_ident, create_ident
    );
    let struct_tokens = quote! {
        #[doc = #create_ident_doc]
        #[derive(Debug, Clone, Default, ::serde::Serialize, ::serde::Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub struct #create_ident {
            #(#field_decls,)*
        }
    };

    // One push per emitted field. Required fields (`nullable=false &&
    // has_default=false`) always push; optional fields only push when
    // `Some(...)`. Enum payloads convert via the user enum's
    // `Into<FilterValue>` impl (codegen emits these in the enum module).
    let lowerings: Vec<TokenStream> = fields
        .iter()
        .filter_map(|f| {
            // Skip enum payloads when there is no enum ident — the
            // struct field is omitted in that case (matches the
            // filter on `field_decls` above).
            if matches!(f.category, FilterCategory::Enum) && f.enum_ident.is_none() {
                return None;
            }
            let n = &f.name;
            let col = &f.column;
            let is_optional = f.nullable || f.has_default;
            // Per the runtime type FilterValue uses From impls for every
            // scalar payload, so a generic `Into` cast is the simplest
            // lowering that round-trips through the existing
            // `set_many` semantics in CreateOperation.
            let push_stmt = if is_optional {
                quote! {
                    if let ::core::option::Option::Some(__v) = self.#n {
                        __out.push((
                            ::std::string::String::from(#col),
                            ::core::convert::Into::<::prax_query::filter::FilterValue>::into(__v),
                        ));
                    }
                }
            } else {
                quote! {
                    __out.push((
                        ::std::string::String::from(#col),
                        ::core::convert::Into::<::prax_query::filter::FilterValue>::into(self.#n),
                    ));
                }
            };
            Some(push_stmt)
        })
        .collect();

    let impl_tokens = quote! {
        impl ::prax_query::inputs::CreateInput for #module_name::#create_ident {
            type Model = #model_ident;
            type Data = ::prax_query::inputs::CreatePayload;
            fn into_ir(self) -> Self::Data {
                let mut __out: ::prax_query::inputs::CreatePayload =
                    ::std::vec::Vec::new();
                #(#lowerings)*
                __out
            }
        }
    };

    CreateInputTokens {
        struct_tokens,
        impl_tokens,
    }
}
