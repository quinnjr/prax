//! Emit `impl prax_query::row::FromRow` for a struct parsed by the derive.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Ident, Type};

/// Emit `FromRow` for `model_name`.
///
/// `scalar_fields` carries the column-backed fields — each one
/// deserializes from its matching row column via `FromColumn`.
///
/// `relation_fields` carries the `Vec<Related>` fields produced by
/// `#[prax(relation(...))]`. They have no column on the parent side,
/// so `from_row` initializes them to `Default::default()` (an empty
/// `Vec` for `HasMany`/`HasOne`, `None` for optional `BelongsTo`).
/// The relation executor fills them later on the `.include()` path.
pub fn emit(
    model_name: &Ident,
    scalar_fields: &[(Ident, Type, String)],
    relation_fields: &[Ident],
) -> TokenStream {
    let rows = scalar_fields.iter().map(|(field, ty, col)| {
        quote! {
            #field: <#ty as prax_query::row::FromColumn>::from_column(row, #col)?,
        }
    });
    let relation_defaults = relation_fields.iter().map(|field| {
        quote! { #field: ::core::default::Default::default(), }
    });
    quote! {
        impl prax_query::row::FromRow for #model_name {
            fn from_row(row: &impl prax_query::row::RowRef)
                -> Result<Self, prax_query::row::RowError>
            {
                Ok(Self {
                    #(#rows)*
                    #(#relation_defaults)*
                })
            }
        }
    }
}
