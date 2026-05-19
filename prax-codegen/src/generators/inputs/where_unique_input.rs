//! Generate `<Model>WhereUniqueInput` — an enum over the model's
//! primary-key and `@unique` columns.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Ident;

use crate::generators::inputs::{FilterCategory, scalar_payload_type};

/// One unique-key column.
pub struct UniqueColumn {
    /// The variant name (PascalCase of the column).
    pub variant: Ident,
    /// SQL column name (string literal).
    pub column: String,
    /// Filter category for the scalar payload.
    pub category: FilterCategory,
    /// For enum columns: the enum's PascalCase ident. Phase 2 leaves
    /// this `None` (enum-aware codegen lands later).
    pub enum_ident: Option<Ident>,
}

/// Emit the `<Model>WhereUniqueInput` enum + `WhereUniqueInput` trait impl.
///
/// Returns `(struct_tokens, impl_tokens)` — `struct_tokens` is emitted
/// inside the per-model `pub mod` block, `impl_tokens` at crate-root
/// scope, mirroring Task 3's split pattern.
pub fn generate(
    model_ident: &Ident,
    module_name: &Ident,
    columns: &[UniqueColumn],
) -> (TokenStream, TokenStream) {
    let where_unique_ident = format_ident!("{}WhereUniqueInput", model_ident);

    if columns.is_empty() {
        let struct_tokens = quote! {
            /// Uninhabited because the model has no unique key.
            #[derive(Debug, Clone, ::serde::Serialize, ::serde::Deserialize)]
            pub enum #where_unique_ident {}
        };
        let impl_tokens = quote! {
            impl ::prax_query::inputs::WhereUniqueInput for #module_name::#where_unique_ident {
                // Use the model ident directly (not module_name::model_ident) to
                // avoid E0603 "private struct import": `use super::*` inside the
                // generated `pub mod` is not a pub re-export, so referencing the
                // model as `module::Model` from outside would be accessing a
                // private import.
                type Model = #model_ident;
                fn into_ir(self) -> ::prax_query::filter::Filter {
                    match self {}
                }
            }
        };
        return (struct_tokens, impl_tokens);
    }

    let variant_decls = columns.iter().map(|c| {
        let v = &c.variant;
        let payload = if let Some(e) = &c.enum_ident {
            quote! { #e }
        } else {
            scalar_payload_type(c.category)
        };
        quote! { #v(#payload) }
    });

    let lower_arms = columns.iter().map(|c| {
        let v = &c.variant;
        let col = &c.column;
        let body = match c.category {
            FilterCategory::Int => quote! { ::prax_query::filter::FilterValue::Int(value as i64) },
            FilterCategory::BigInt => quote! { ::prax_query::filter::FilterValue::Int(value) },
            FilterCategory::Float => quote! { ::prax_query::filter::FilterValue::Float(value) },
            FilterCategory::Bool => quote! { ::prax_query::filter::FilterValue::Bool(value) },
            FilterCategory::String => quote! { ::prax_query::filter::FilterValue::String(value) },
            FilterCategory::Decimal => {
                quote! { ::prax_query::filter::FilterValue::String(value.to_string()) }
            }
            FilterCategory::Uuid => {
                quote! { ::prax_query::filter::FilterValue::String(value.to_string()) }
            }
            FilterCategory::Bytes => quote! {
                {
                    use base64::Engine as _;
                    ::prax_query::filter::FilterValue::String(
                        base64::engine::general_purpose::STANDARD.encode(&value),
                    )
                }
            },
            FilterCategory::DateTime => {
                quote! { ::prax_query::filter::FilterValue::String(value.to_rfc3339()) }
            }
            FilterCategory::Date => {
                quote! { ::prax_query::filter::FilterValue::String(value.to_string()) }
            }
            FilterCategory::Time => quote! {
                ::prax_query::filter::FilterValue::String(value.format("%H:%M:%S").to_string())
            },
            FilterCategory::Json => quote! { ::prax_query::filter::FilterValue::Json(value) },
            FilterCategory::Enum => {
                quote! { ::prax_query::filter::FilterValue::String(value.to_string()) }
            }
        };
        // Fully-qualify the pattern with the module path to avoid introducing
        // a `use` statement at the impl scope (which triggers E0603 "private
        // struct import" when the macro is expanded in integration-test files).
        quote! {
            #module_name::#where_unique_ident::#v(value) => ::prax_query::filter::Filter::Equals(
                ::std::borrow::Cow::Borrowed(#col),
                #body,
            )
        }
    });

    let struct_tokens = quote! {
        #[derive(Debug, Clone, ::serde::Serialize, ::serde::Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum #where_unique_ident {
            #(#variant_decls,)*
        }
    };

    let impl_tokens = quote! {
        impl ::prax_query::inputs::WhereUniqueInput for #module_name::#where_unique_ident {
            // Use the model ident directly (not module_name::model_ident) to
            // avoid E0603 "private struct import": `use super::*` inside the
            // generated `pub mod` is not a pub re-export, so referencing the
            // model as `module::Model` from outside would be accessing a
            // private import.
            type Model = #model_ident;
            fn into_ir(self) -> ::prax_query::filter::Filter {
                match self {
                    #(#lower_arms,)*
                }
            }
        }
    };

    (struct_tokens, impl_tokens)
}
