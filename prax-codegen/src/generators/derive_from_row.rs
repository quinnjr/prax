//! Emit `impl prax_query::row::FromRow` for a struct parsed by the derive.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Ident, Type};

/// Each tuple is (rust field ident, field type, column name).
pub fn emit(model_name: &Ident, fields: &[(Ident, Type, String)]) -> TokenStream {
    let rows = fields.iter().map(|(field, ty, col)| {
        quote! {
            #field: <#ty as prax_query::row::FromColumn>::from_column(row, #col)?,
        }
    });
    quote! {
        impl prax_query::row::FromRow for #model_name {
            fn from_row(row: &impl prax_query::row::RowRef)
                -> Result<Self, prax_query::row::RowError>
            {
                Ok(Self { #(#rows)* })
            }
        }
    }
}
