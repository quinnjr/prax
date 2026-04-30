//! Emit `impl prax_query::traits::ModelWithPk` for a struct.
//!
//! Used by both the `#[derive(Model)]` path and the `prax_schema!` macro
//! path. The caller is responsible for filtering relation fields (`Vec<Model>`)
//! out of the `fields` slice — only scalar columns appear here.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Ident, Type};

/// Emit a `ModelWithPk` implementation for `model_name`.
///
/// Each tuple is `(field_ident, rust_type, sql_column_name, is_id)`.
/// The emitter routes `is_id` fields into `pk_value()` (collapsing
/// composite keys into `FilterValue::List`) and routes every field
/// into the `get_column_value()` match.
pub fn emit(model_name: &Ident, fields: &[(Ident, Type, String, bool)]) -> TokenStream {
    let id_fields: Vec<_> = fields.iter().filter(|(_, _, _, is_id)| *is_id).collect();

    let pk_expr = if id_fields.len() == 1 {
        let (field, ty, _col, _) = id_fields[0];
        quote! {
            <#ty as ::prax_query::filter::ToFilterValue>::to_filter_value(&self.#field)
        }
    } else if id_fields.is_empty() {
        // The caller (`derive.rs` / `model.rs`) rejects no-PK models
        // before we get here, but emit a safe fallback rather than
        // fabricating a panic in generated code.
        quote! { ::prax_query::filter::FilterValue::Null }
    } else {
        let items = id_fields.iter().map(|(field, ty, _, _)| {
            quote! {
                <#ty as ::prax_query::filter::ToFilterValue>::to_filter_value(&self.#field)
            }
        });
        quote! { ::prax_query::filter::FilterValue::List(vec![ #(#items),* ]) }
    };

    let col_arms = fields.iter().map(|(field, ty, col, _)| {
        quote! {
            #col => ::core::option::Option::Some(
                <#ty as ::prax_query::filter::ToFilterValue>::to_filter_value(&self.#field)
            ),
        }
    });

    quote! {
        impl ::prax_query::traits::ModelWithPk for #model_name {
            fn pk_value(&self) -> ::prax_query::filter::FilterValue {
                #pk_expr
            }

            fn get_column_value(
                &self,
                column: &str,
            ) -> ::core::option::Option<::prax_query::filter::FilterValue> {
                match column {
                    #(#col_arms)*
                    _ => ::core::option::Option::None,
                }
            }
        }
    }
}
