//! Generate `<Model>Include` — one Option<bool> per relation.
//!
//! Returns `(struct_tokens, impl_tokens)` for the same split as
//! `where_input` (struct inside `pub mod #module_name`, impl at
//! crate-root scope).

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Ident;

pub struct IncludeField {
    /// Field name in the source code (snake_case ident).
    pub name: Ident,
    /// SQL relation name (used by IncludeSpec).
    pub relation: String,
}

pub fn generate(
    model_ident: &Ident,
    module_name: &Ident,
    relations: &[IncludeField],
) -> (TokenStream, TokenStream) {
    let include_ident = format_ident!("{}Include", model_ident);

    let decls = relations.iter().map(|r| {
        let n = &r.name;
        quote! {
            pub #n: ::core::option::Option<bool>
        }
    });

    let lowerings = relations.iter().map(|r| {
        let n = &r.name;
        let rel = &r.relation;
        quote! {
            if self.#n == ::core::option::Option::Some(true) {
                inc = inc.with(::prax_query::relations::IncludeSpec::new(#rel));
            }
        }
    });

    let struct_tokens = quote! {
        #[derive(Debug, Clone, Default, ::serde::Serialize, ::serde::Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub struct #include_ident {
            #(#decls,)*
        }
    };

    let impl_tokens = quote! {
        impl ::prax_query::inputs::IncludeInput for #module_name::#include_ident {
            type Model = #model_ident;
            fn into_ir(self) -> ::prax_query::relations::Include {
                let mut inc = ::prax_query::relations::Include::new();
                #(#lowerings)*
                inc
            }
        }
    };

    (struct_tokens, impl_tokens)
}
