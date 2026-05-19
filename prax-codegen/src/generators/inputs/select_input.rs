//! Generate `<Model>Select` — one Option<bool> per column and per relation.
//!
//! The `Select` IR enum uses `Select::Fields(Vec<String>)` for a non-empty
//! selection and `Select::All` when no fields are explicitly chosen.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Ident;

pub struct SelectField {
    /// Field name in the source code (snake_case ident).
    pub name: Ident,
    /// SQL column name (string literal). For relation fields this is
    /// the relation name and `is_relation` is true.
    pub column: String,
    /// Whether this field is a relation (skip from SELECT column list).
    pub is_relation: bool,
}

pub fn generate(
    model_ident: &Ident,
    module_name: &Ident,
    fields: &[SelectField],
) -> (TokenStream, TokenStream) {
    let select_ident = format_ident!("{}Select", model_ident);

    let decls = fields.iter().map(|f| {
        let n = &f.name;
        quote! {
            pub #n: ::core::option::Option<bool>
        }
    });

    let lowerings = fields.iter().filter(|f| !f.is_relation).map(|f| {
        let n = &f.name;
        let col = &f.column;
        quote! {
            if self.#n == ::core::option::Option::Some(true) {
                cols.push(#col.to_string());
            }
        }
    });

    let struct_tokens = quote! {
        #[derive(Debug, Clone, Default, ::serde::Serialize, ::serde::Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub struct #select_ident {
            #(#decls,)*
        }
    };

    let impl_tokens = quote! {
        impl ::prax_query::inputs::SelectInput for #module_name::#select_ident {
            type Model = #model_ident;
            fn into_ir(self) -> ::prax_query::types::Select {
                let mut cols: ::std::vec::Vec<::std::string::String> = ::std::vec::Vec::new();
                #(#lowerings)*
                if cols.is_empty() {
                    ::prax_query::types::Select::All
                } else {
                    ::prax_query::types::Select::Fields(cols)
                }
            }
        }
    };

    (struct_tokens, impl_tokens)
}
