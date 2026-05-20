//! Generate `<Model>UpdateInput` — flat scalar fields wrapped in
//! `*FieldUpdate` wrappers. Nested writes (`update`/`connect`/etc.)
//! and the `UpdateInput` trait impl land later alongside the
//! operation rework.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Ident;

use crate::generators::inputs::{FilterCategory, update_wrapper_ident};

pub struct UpdateField {
    /// Field name in the source code.
    pub name: Ident,
    /// Filter category for the wrapper selection.
    pub category: FilterCategory,
    /// Whether the field is nullable (selects `*NullableFieldUpdate`).
    pub nullable: bool,
    /// For enum columns: the enum's PascalCase ident. Required for
    /// `EnumFieldUpdate<E>` instantiation.
    pub enum_ident: Option<Ident>,
}

pub fn generate(model_ident: &Ident, fields: &[UpdateField]) -> TokenStream {
    let update_ident = format_ident!("{}UpdateInput", model_ident);

    let field_decls = fields.iter().map(|f| {
        let n = &f.name;
        let wrapper = update_wrapper_ident(f.category, f.nullable);
        // `Date` and `Time` columns share `DateTimeFieldUpdate` (its
        // `set: Option<String>` is encoding-agnostic). Emit a doc note
        // on the generated field so the user-facing type asymmetry
        // between filters (typed) and updates (string-encoded) is
        // discoverable from the generated code itself.
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

    quote! {
        #[derive(Debug, Clone, Default, ::serde::Serialize, ::serde::Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub struct #update_ident {
            #(#field_decls,)*
        }
    }
}
