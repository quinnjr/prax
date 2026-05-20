//! Generate `<Model>OrderBy` — an enum over the model's sortable columns.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Ident;

pub struct OrderByInputField {
    /// Variant name (PascalCase of the column).
    pub variant: Ident,
    /// SQL column name (string literal).
    pub column: String,
    /// Whether the column is nullable (allows NULLS FIRST/LAST). Currently
    /// unused — NULLS handling lands with the dialect layer, at which point
    /// the variant signature will carry both `SortOrder` and an optional
    /// nulls position.
    #[allow(dead_code)]
    pub nullable: bool,
}

/// Emit `<Model>OrderBy` enum + `OrderByInput` trait impl. Returns
/// `(struct_tokens, impl_tokens)` matching the established split.
pub fn generate(
    model_ident: &Ident,
    module_name: &Ident,
    fields: &[OrderByInputField],
) -> (TokenStream, TokenStream) {
    let order_by_ident = format_ident!("{}OrderBy", model_ident);

    if fields.is_empty() {
        let struct_tokens = quote! {
            /// Uninhabited because no sortable columns exist.
            #[derive(Debug, Clone, ::serde::Serialize, ::serde::Deserialize)]
            pub enum #order_by_ident {}
        };
        let impl_tokens = quote! {
            impl ::prax_query::inputs::OrderByInput for #module_name::#order_by_ident {
                type Model = #model_ident;
                fn into_ir(self) -> ::prax_query::types::OrderBy {
                    match self {}
                }
            }
        };
        return (struct_tokens, impl_tokens);
    }

    let variant_decls = fields.iter().map(|f| {
        let v = &f.variant;
        quote! { #v(::prax_query::types::SortOrder) }
    });

    let match_arms = fields.iter().map(|f| {
        let v = &f.variant;
        let col = &f.column;
        quote! {
            #module_name::#order_by_ident::#v(dir) => {
                ::prax_query::types::OrderBy::from(
                    ::prax_query::types::OrderByField::new(#col, dir),
                )
            }
        }
    });

    let struct_tokens = quote! {
        #[derive(Debug, Clone, ::serde::Serialize, ::serde::Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum #order_by_ident {
            #(#variant_decls,)*
        }
    };

    let impl_tokens = quote! {
        impl ::prax_query::inputs::OrderByInput for #module_name::#order_by_ident {
            type Model = #model_ident;
            fn into_ir(self) -> ::prax_query::types::OrderBy {
                match self {
                    #(#match_arms,)*
                }
            }
        }
    };

    (struct_tokens, impl_tokens)
}
