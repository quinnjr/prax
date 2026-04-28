//! Emit `impl prax_query::traits::Model` for a struct parsed by the derive.

use proc_macro2::TokenStream;
use quote::quote;
use syn::Ident;

pub fn emit(
    model_name: &Ident,
    model_name_str: &str,
    table_name: &str,
    pk_columns: &[String],
    all_columns: &[String],
) -> TokenStream {
    let pks = pk_columns.iter();
    let cols = all_columns.iter();
    quote! {
        impl prax_query::traits::Model for #model_name {
            const MODEL_NAME: &'static str = #model_name_str;
            const TABLE_NAME: &'static str = #table_name;
            const PRIMARY_KEY: &'static [&'static str] = &[#(#pks),*];
            const COLUMNS: &'static [&'static str] = &[#(#cols),*];
        }
    }
}
