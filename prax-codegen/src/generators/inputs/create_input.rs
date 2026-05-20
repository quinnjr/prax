//! Generate `<Model>CreateInput` — flat scalar fields.
//!
//! Returns a single `TokenStream` (no trait impl). Nested writes
//! (`connect`/`create`/`disconnect`/etc.) and the `CreateInput` trait
//! impl that wires the data into `CreateOperation` land in a later
//! phase together with the operation rework.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Ident;

use crate::generators::inputs::{FilterCategory, scalar_payload_type};

pub struct CreateField {
    /// Field name in the source code.
    pub name: Ident,
    /// Filter category for the scalar payload.
    pub category: FilterCategory,
    /// Whether the field is `Option<T>` (nullable).
    pub nullable: bool,
    /// Whether the field has a default (Option-wrap so callers can omit).
    pub has_default: bool,
    /// For enum columns: the enum's PascalCase ident.
    pub enum_ident: Option<Ident>,
}

pub fn generate(model_ident: &Ident, fields: &[CreateField]) -> TokenStream {
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

    quote! {
        #[derive(Debug, Clone, Default, ::serde::Serialize, ::serde::Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub struct #create_ident {
            #(#field_decls,)*
        }
    }
}
