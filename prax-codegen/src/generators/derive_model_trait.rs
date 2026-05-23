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
    generated_fields: &[(&str, &str, bool)],
    aggregate_fields: &[(&str, &str, &str, Option<&str>)],
) -> TokenStream {
    let pks = pk_columns.iter();
    let cols = all_columns.iter();

    // Build GENERATED_FIELDS literal: &[(&str, &str, bool)]
    let gen_entries = generated_fields.iter().map(|(field, expr, stored)| {
        quote! { (#field, #expr, #stored) }
    });

    // Build AGGREGATE_FIELDS literal: &[(&str, &str, &str, Option<&str>)]
    let agg_entries = aggregate_fields
        .iter()
        .map(|(field, kind, rel, opt_field)| {
            let opt_field_tokens = match opt_field {
                Some(f) => quote! { Some(#f) },
                None => quote! { None },
            };
            quote! { (#field, #kind, #rel, #opt_field_tokens) }
        });

    quote! {
        impl prax_query::traits::Model for #model_name {
            const MODEL_NAME: &'static str = #model_name_str;
            const TABLE_NAME: &'static str = #table_name;
            const PRIMARY_KEY: &'static [&'static str] = &[#(#pks),*];
            const COLUMNS: &'static [&'static str] = &[#(#cols),*];
            const GENERATED_FIELDS: &'static [(&'static str, &'static str, bool)] =
                &[#(#gen_entries),*];
            const AGGREGATE_FIELDS: &'static [(
                &'static str,
                &'static str,
                &'static str,
                Option<&'static str>,
            )] = &[#(#agg_entries),*];
        }
    }
}
