//! Generators for the typed input shapes.
//!
//! Each submodule is a pure function from a parsed schema model
//! (`prax_schema::ast::Model` or a derive-parsed `FieldInfo` list) to a
//! `TokenStream` containing one input type per model. The `derive.rs`
//! and `model.rs` entry points call all of these in turn and concat
//! the streams into the per-model module.

pub mod create_input;
pub mod include_input;
pub mod order_by_input;
pub mod relation_meta;
pub mod select_input;
pub mod update_input;
pub mod where_input;
pub mod where_unique_input;

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Ident;

/// Tag for a field's filter category — drives which scalar filter
/// wrapper is referenced in the generated struct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterCategory {
    String,
    Int,
    BigInt,
    Float,
    Decimal,
    Bool,
    Bytes,
    Uuid,
    Json,
    DateTime,
    Date,
    Time,
    Enum,
}

/// Map a schema-level type string to the right `FilterCategory`.
/// Returns `None` for unknown / relation fields.
pub fn filter_category_for(type_name: &str) -> Option<FilterCategory> {
    match type_name {
        "String" => Some(FilterCategory::String),
        "Int" | "i32" => Some(FilterCategory::Int),
        "BigInt" | "i64" => Some(FilterCategory::BigInt),
        "Float" | "f64" => Some(FilterCategory::Float),
        "Decimal" | "rust_decimal::Decimal" => Some(FilterCategory::Decimal),
        "Boolean" | "bool" => Some(FilterCategory::Bool),
        "Bytes" | "Vec<u8>" => Some(FilterCategory::Bytes),
        "Uuid" | "uuid::Uuid" => Some(FilterCategory::Uuid),
        "Json" | "serde_json::Value" => Some(FilterCategory::Json),
        "DateTime" | "chrono::DateTime<chrono::Utc>" => Some(FilterCategory::DateTime),
        "Date" | "chrono::NaiveDate" => Some(FilterCategory::Date),
        "Time" | "chrono::NaiveTime" => Some(FilterCategory::Time),
        _ => None,
    }
}

/// Resolve the `prax_query::inputs` filter wrapper type ident for a
/// given category + nullability.
pub fn filter_wrapper_ident(cat: FilterCategory, nullable: bool) -> Ident {
    let name = match (cat, nullable) {
        (FilterCategory::String, false) => "StringFilter",
        (FilterCategory::String, true) => "StringNullableFilter",
        (FilterCategory::Int, false) => "IntFilter",
        (FilterCategory::Int, true) => "IntNullableFilter",
        (FilterCategory::BigInt, false) => "BigIntFilter",
        (FilterCategory::BigInt, true) => "BigIntNullableFilter",
        (FilterCategory::Float, false) => "FloatFilter",
        (FilterCategory::Float, true) => "FloatNullableFilter",
        (FilterCategory::Decimal, false) => "DecimalFilter",
        (FilterCategory::Decimal, true) => "DecimalNullableFilter",
        (FilterCategory::Bool, false) => "BoolFilter",
        (FilterCategory::Bool, true) => "BoolNullableFilter",
        (FilterCategory::Bytes, false) => "BytesFilter",
        (FilterCategory::Bytes, true) => "BytesNullableFilter",
        (FilterCategory::Uuid, false) => "UuidFilter",
        (FilterCategory::Uuid, true) => "UuidNullableFilter",
        (FilterCategory::Json, false) => "InputJsonFilter",
        (FilterCategory::Json, true) => "InputJsonNullableFilter",
        (FilterCategory::DateTime, false) => "DateTimeFilter",
        (FilterCategory::DateTime, true) => "DateTimeNullableFilter",
        (FilterCategory::Date, false) => "DateFilter",
        (FilterCategory::Date, true) => "DateNullableFilter",
        (FilterCategory::Time, false) => "TimeFilter",
        (FilterCategory::Time, true) => "TimeNullableFilter",
        (FilterCategory::Enum, false) => "EnumFilter",
        (FilterCategory::Enum, true) => "EnumNullableFilter",
    };
    format_ident!("{}", name)
}

/// Resolve the field-update wrapper ident.
pub fn update_wrapper_ident(cat: FilterCategory, nullable: bool) -> Ident {
    let name = match (cat, nullable) {
        (FilterCategory::String, false) => "StringFieldUpdate",
        (FilterCategory::String, true) => "StringNullableFieldUpdate",
        (FilterCategory::Int, false) => "IntFieldUpdate",
        (FilterCategory::Int, true) => "IntNullableFieldUpdate",
        (FilterCategory::BigInt, false) => "BigIntFieldUpdate",
        (FilterCategory::BigInt, true) => "BigIntNullableFieldUpdate",
        (FilterCategory::Float, false) => "FloatFieldUpdate",
        (FilterCategory::Float, true) => "FloatNullableFieldUpdate",
        (FilterCategory::Decimal, false) => "DecimalFieldUpdate",
        (FilterCategory::Decimal, true) => "DecimalNullableFieldUpdate",
        (FilterCategory::Bool, false) => "BoolFieldUpdate",
        (FilterCategory::Bool, true) => "BoolNullableFieldUpdate",
        (FilterCategory::Bytes, false) => "BytesFieldUpdate",
        (FilterCategory::Bytes, true) => "BytesNullableFieldUpdate",
        (FilterCategory::Uuid, false) => "UuidFieldUpdate",
        (FilterCategory::Uuid, true) => "UuidNullableFieldUpdate",
        (FilterCategory::Json, false) => "JsonFieldUpdate",
        (FilterCategory::Json, true) => "JsonNullableFieldUpdate",
        (FilterCategory::DateTime, false) => "DateTimeFieldUpdate",
        (FilterCategory::DateTime, true) => "DateTimeNullableFieldUpdate",
        (FilterCategory::Date, false) => "DateTimeFieldUpdate",
        (FilterCategory::Date, true) => "DateTimeNullableFieldUpdate",
        (FilterCategory::Time, false) => "DateTimeFieldUpdate",
        (FilterCategory::Time, true) => "DateTimeNullableFieldUpdate",
        (FilterCategory::Enum, false) => "EnumFieldUpdate",
        (FilterCategory::Enum, true) => "EnumNullableFieldUpdate",
    };
    format_ident!("{}", name)
}

/// Resolve the Rust scalar payload type that the filter / update wrapper
/// expects.
pub fn scalar_payload_type(cat: FilterCategory) -> TokenStream {
    match cat {
        FilterCategory::String => quote! { ::std::string::String },
        FilterCategory::Int => quote! { i32 },
        FilterCategory::BigInt => quote! { i64 },
        FilterCategory::Float => quote! { f64 },
        FilterCategory::Decimal => quote! { ::rust_decimal::Decimal },
        FilterCategory::Bool => quote! { bool },
        FilterCategory::Bytes => quote! { ::std::vec::Vec<u8> },
        FilterCategory::Uuid => quote! { ::uuid::Uuid },
        FilterCategory::Json => quote! { ::serde_json::Value },
        FilterCategory::DateTime => quote! { ::chrono::DateTime<::chrono::Utc> },
        FilterCategory::Date => quote! { ::chrono::NaiveDate },
        FilterCategory::Time => quote! { ::chrono::NaiveTime },
        FilterCategory::Enum => {
            panic!("enum payload requires the enum ident — caller must construct")
        }
    }
}
