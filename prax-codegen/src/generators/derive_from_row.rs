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
///
/// `aggregate_fields` carries fields annotated with
/// `#[prax(count/sum/avg/min/max)]`. They have no underlying column in
/// the base table; the row may or may not include their projected value
/// (it's only present when the caller explicitly selects the aggregate).
/// We soft-miss the column: `ColumnNotFound` is treated as a default
/// value (`0` for Count, `None` for Sum/Avg/Min/Max). Any other error
/// (type mismatch, etc.) propagates normally.
///
/// Each `aggregate_fields` entry is `(field_ident, declared_type, kind_str,
/// col_name)` where `kind_str` is one of `"count"`, `"sum"`, `"avg"`,
/// `"min"`, `"max"`.
///
/// **Type-checking is deferred**: the user is trusted to declare `i64`
/// for `@count` fields and `Option<T>` for Sum/Avg/Min/Max. If they get
/// it wrong the generated code will fail to compile with a type-mismatch
/// error. Proper compile-time validation is a follow-up task.
pub fn emit(
    model_name: &Ident,
    scalar_fields: &[(Ident, Type, String)],
    relation_fields: &[Ident],
    aggregate_fields: &[(Ident, Type, String, String)],
) -> TokenStream {
    let rows = scalar_fields.iter().map(|(field, ty, col)| {
        quote! {
            #field: <#ty as prax_query::row::FromColumn>::from_column(row, #col)?,
        }
    });

    // For aggregate fields, soft-miss the column: ColumnNotFound → default.
    // Count fields are `i64` (never optional), default to 0.
    // Sum/Avg/Min/Max fields are `Option<T>`, default to None.
    let agg_rows = aggregate_fields.iter().map(|(field, ty, col, kind)| {
        if kind == "count" {
            // Count: i64, default to 0 when column is absent.
            quote! {
                #field: row.get_i64_opt(#col)
                    .or_else(|e| {
                        if matches!(e, prax_query::row::RowError::ColumnNotFound(_)) {
                            Ok(None)
                        } else {
                            Err(e)
                        }
                    })?
                    .unwrap_or(0),
            }
        } else {
            // Sum/Avg/Min/Max: Option<T>, default to None when absent.
            quote! {
                #field: <#ty as prax_query::row::FromColumn>::from_column(row, #col)
                    .or_else(|e| {
                        if matches!(e, prax_query::row::RowError::ColumnNotFound(_)) {
                            Ok(<#ty as ::core::default::Default>::default())
                        } else {
                            Err(e)
                        }
                    })?,
            }
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
                    #(#agg_rows)*
                    #(#relation_defaults)*
                })
            }
        }
    }
}
