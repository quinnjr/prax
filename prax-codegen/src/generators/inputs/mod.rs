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

/// Map a type-name string to the right `FilterCategory`.
///
/// Accepts multiple spellings because two call sites converge here:
/// - The `#[derive(Model)]` path's `extract_inner_type_name` returns
///   the last path segment after unwrapping `Option<T>` — bare idents
///   like `"i32"`, `"Vec"` (for `Vec<u8>`), `"DateTime"` (for
///   `chrono::DateTime<_>`), `"Value"` (for `serde_json::Value`),
///   `"NaiveDate"` / `"NaiveTime"`, `"Uuid"`, `"Decimal"`.
/// - The `prax_schema!` path maps `ScalarType` to schema-level names
///   (e.g. `"Int"`, `"Boolean"`, `"DateTime"`, `"Bytes"`) before
///   calling this.
///
/// Returns `None` for unknown / relation fields. **When adding a new
/// scalar type, register every spelling the derive path can produce
/// (e.g. both `"Vec"` and `"Vec<u8>"` for byte columns) — otherwise
/// the column is silently dropped from generated inputs.**
pub fn filter_category_for(type_name: &str) -> Option<FilterCategory> {
    match type_name {
        "String" => Some(FilterCategory::String),
        "Int" | "i32" => Some(FilterCategory::Int),
        "BigInt" | "i64" => Some(FilterCategory::BigInt),
        "Float" | "f64" => Some(FilterCategory::Float),
        "Decimal" | "rust_decimal::Decimal" => Some(FilterCategory::Decimal),
        "Boolean" | "bool" => Some(FilterCategory::Bool),
        // Derive path emits "Vec" (last segment of `Vec<u8>`); schema path
        // emits "Bytes". `Vec<u8>` literal kept for robustness.
        "Bytes" | "Vec<u8>" | "Vec" => Some(FilterCategory::Bytes),
        "Uuid" | "uuid::Uuid" => Some(FilterCategory::Uuid),
        // Derive path emits "Value" (last segment of `serde_json::Value`);
        // schema path emits "Json".
        "Json" | "serde_json::Value" | "Value" => Some(FilterCategory::Json),
        "DateTime" | "chrono::DateTime<chrono::Utc>" => Some(FilterCategory::DateTime),
        "Date" | "chrono::NaiveDate" | "NaiveDate" => Some(FilterCategory::Date),
        "Time" | "chrono::NaiveTime" | "NaiveTime" => Some(FilterCategory::Time),
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
        // Inside `prax_query::inputs::*` the type is `JsonFilter`; the
        // crate-root alias `InputJsonFilter` exists only to disambiguate
        // from `json::JsonFilter`. Codegen emits the inputs-module path,
        // so use the unprefixed name here.
        (FilterCategory::Json, false) => "JsonFilter",
        (FilterCategory::Json, true) => "JsonNullableFilter",
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
///
/// `Date` and `Time` columns share `DateTimeFieldUpdate` because phase 1's
/// update wrappers carry encoding-agnostic `Option<String>` payloads (the
/// driver layer parses on the wire). The filter side has typed
/// `DateFilter` / `TimeFilter` because filter values flow through
/// `FilterValue::String` after format-specific encoding at lowering time.
/// If a future phase introduces typed update wrappers, replace the shared
/// `DateTimeFieldUpdate` arms with `DateFieldUpdate` / `TimeFieldUpdate`.
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
        (FilterCategory::DateTime | FilterCategory::Date | FilterCategory::Time, false) => {
            "DateTimeFieldUpdate"
        }
        (FilterCategory::DateTime | FilterCategory::Date | FilterCategory::Time, true) => {
            "DateTimeNullableFieldUpdate"
        }
        (FilterCategory::Enum, false) => "EnumFieldUpdate",
        (FilterCategory::Enum, true) => "EnumNullableFieldUpdate",
    };
    format_ident!("{}", name)
}

/// Resolve the Rust scalar payload type that the filter / update wrapper
/// expects.
///
/// Callers handling enum-typed fields must construct the payload using
/// the enum's PascalCase ident directly; this function is unreachable
/// for `FilterCategory::Enum` and panics there to surface the contract
/// violation early.
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
        FilterCategory::Enum => unreachable!(
            "scalar_payload_type called for FilterCategory::Enum — caller must \
             construct payload from the enum's PascalCase ident instead"
        ),
    }
}
