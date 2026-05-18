//! Reusable scalar filter wrappers shared by every generated `*WhereInput`.
//!
//! Each wrapper is a struct of `Option`-fields, one per operator. Empty
//! wrappers (all fields `None`) lower to `Filter::None`. Multiple set
//! fields AND-combine. `From<scalar>` impls support the macro shorthand
//! `email: "alice@x.com"` => `StringFilter { equals: Some("..."), .. }`.
//!
//! Every wrapper implements [`ScalarFilter`], whose `into_filter`
//! method takes the column name (which the parent `WhereInput` knows)
//! and produces a runtime [`Filter`].

use crate::filter::{Filter, FilterValue};
use serde::{Deserialize, Serialize};

/// Helper trait implemented by every scalar filter wrapper.
///
/// The wrapper itself doesn't know its column name — the parent
/// `WhereInput::into_ir` impl threads the column in when lowering.
pub trait ScalarFilter {
    /// Lower this scalar filter to a runtime [`Filter`] keyed by
    /// the given column name.
    fn into_filter(self, column: &str) -> Filter;
}

/// Comparison mode for string filters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueryMode {
    /// Default (case-sensitive) comparison.
    #[default]
    Default,
    /// Case-insensitive comparison. Requires `SupportsCaseInsensitiveMode`
    /// for engines that don't fall back to `LOWER(...)`.
    Insensitive,
}

/// Filter operators for a non-nullable `String` column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StringFilter {
    /// `column = value`
    pub equals: Option<String>,
    /// Negation of the inner filter.
    pub not: Option<Box<StringFilter>>,
    /// `column IN (...)`
    pub in_list: Option<Vec<String>>,
    /// `column NOT IN (...)`
    pub not_in: Option<Vec<String>>,
    /// `column < value`
    pub lt: Option<String>,
    /// `column <= value`
    pub lte: Option<String>,
    /// `column > value`
    pub gt: Option<String>,
    /// `column >= value`
    pub gte: Option<String>,
    /// `column LIKE %value%`
    pub contains: Option<String>,
    /// `column LIKE value%`
    pub starts_with: Option<String>,
    /// `column LIKE %value`
    pub ends_with: Option<String>,
    /// Comparison mode (case sensitivity).
    pub mode: Option<QueryMode>,
}

impl StringFilter {
    /// `equals: Some(value)`.
    pub fn equals(v: impl Into<String>) -> Self {
        Self {
            equals: Some(v.into()),
            ..Default::default()
        }
    }
    /// `contains: Some(value)`.
    pub fn contains(v: impl Into<String>) -> Self {
        Self {
            contains: Some(v.into()),
            ..Default::default()
        }
    }
    /// `starts_with: Some(value)`.
    pub fn starts_with(v: impl Into<String>) -> Self {
        Self {
            starts_with: Some(v.into()),
            ..Default::default()
        }
    }
    /// `ends_with: Some(value)`.
    pub fn ends_with(v: impl Into<String>) -> Self {
        Self {
            ends_with: Some(v.into()),
            ..Default::default()
        }
    }
}

impl From<&str> for StringFilter {
    fn from(v: &str) -> Self {
        Self::equals(v)
    }
}
impl From<String> for StringFilter {
    fn from(v: String) -> Self {
        Self::equals(v)
    }
}

impl ScalarFilter for StringFilter {
    fn into_filter(self, column: &str) -> Filter {
        let mut parts: Vec<Filter> = Vec::new();
        let col = column.to_string();
        if let Some(v) = self.equals {
            parts.push(Filter::Equals(col.clone().into(), FilterValue::String(v)));
        }
        if let Some(boxed) = self.not {
            let inner = boxed.into_filter(column);
            parts.push(Filter::Not(Box::new(inner)));
        }
        if let Some(values) = self.in_list {
            let vs: Vec<FilterValue> = values.into_iter().map(FilterValue::String).collect();
            parts.push(Filter::In(col.clone().into(), vs));
        }
        if let Some(values) = self.not_in {
            let vs: Vec<FilterValue> = values.into_iter().map(FilterValue::String).collect();
            parts.push(Filter::NotIn(col.clone().into(), vs));
        }
        if let Some(v) = self.lt {
            parts.push(Filter::Lt(col.clone().into(), FilterValue::String(v)));
        }
        if let Some(v) = self.lte {
            parts.push(Filter::Lte(col.clone().into(), FilterValue::String(v)));
        }
        if let Some(v) = self.gt {
            parts.push(Filter::Gt(col.clone().into(), FilterValue::String(v)));
        }
        if let Some(v) = self.gte {
            parts.push(Filter::Gte(col.clone().into(), FilterValue::String(v)));
        }
        if let Some(v) = self.contains {
            parts.push(Filter::Contains(col.clone().into(), FilterValue::String(v)));
        }
        if let Some(v) = self.starts_with {
            parts.push(Filter::StartsWith(
                col.clone().into(),
                FilterValue::String(v),
            ));
        }
        if let Some(v) = self.ends_with {
            parts.push(Filter::EndsWith(col.clone().into(), FilterValue::String(v)));
        }
        // `mode` is honored by the dialect layer in phase 2+; phase 1 ignores
        // it here. The field is kept so downstream phases don't need a
        // breaking-shape change.
        let _ = self.mode;
        match parts.len() {
            0 => Filter::None,
            1 => parts.into_iter().next().unwrap(),
            _ => Filter::and(parts),
        }
    }
}

/// Filter operators for a nullable `String` column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StringNullableFilter {
    /// `column = value`
    pub equals: Option<String>,
    /// Negation of the inner filter.
    pub not: Option<Box<StringNullableFilter>>,
    /// `column IN (...)`
    pub in_list: Option<Vec<String>>,
    /// `column NOT IN (...)`
    pub not_in: Option<Vec<String>>,
    /// `column < value`
    pub lt: Option<String>,
    /// `column <= value`
    pub lte: Option<String>,
    /// `column > value`
    pub gt: Option<String>,
    /// `column >= value`
    pub gte: Option<String>,
    /// `column LIKE %value%`
    pub contains: Option<String>,
    /// `column LIKE value%`
    pub starts_with: Option<String>,
    /// `column LIKE %value`
    pub ends_with: Option<String>,
    /// Comparison mode.
    pub mode: Option<QueryMode>,
    /// `is_null: Some(true)` => `IS NULL`; `Some(false)` => `IS NOT NULL`.
    pub is_null: Option<bool>,
}

impl From<&str> for StringNullableFilter {
    fn from(v: &str) -> Self {
        Self {
            equals: Some(v.into()),
            ..Default::default()
        }
    }
}
impl From<String> for StringNullableFilter {
    fn from(v: String) -> Self {
        Self {
            equals: Some(v),
            ..Default::default()
        }
    }
}

impl ScalarFilter for StringNullableFilter {
    fn into_filter(self, column: &str) -> Filter {
        let mut parts: Vec<Filter> = Vec::new();
        let col = column.to_string();
        if let Some(b) = self.is_null {
            parts.push(if b {
                Filter::IsNull(col.clone().into())
            } else {
                Filter::IsNotNull(col.clone().into())
            });
        }
        // Reuse StringFilter's lowering for the remaining ops.
        let inner = StringFilter {
            equals: self.equals,
            not: self.not.map(|b| {
                Box::new(StringFilter {
                    equals: b.equals,
                    in_list: b.in_list,
                    not_in: b.not_in,
                    lt: b.lt,
                    lte: b.lte,
                    gt: b.gt,
                    gte: b.gte,
                    contains: b.contains,
                    starts_with: b.starts_with,
                    ends_with: b.ends_with,
                    mode: b.mode,
                    not: None,
                })
            }),
            in_list: self.in_list,
            not_in: self.not_in,
            lt: self.lt,
            lte: self.lte,
            gt: self.gt,
            gte: self.gte,
            contains: self.contains,
            starts_with: self.starts_with,
            ends_with: self.ends_with,
            mode: self.mode,
        };
        let inner_filter = inner.into_filter(column);
        if !matches!(inner_filter, Filter::None) {
            parts.push(inner_filter);
        }
        match parts.len() {
            0 => Filter::None,
            1 => parts.into_iter().next().unwrap(),
            _ => Filter::and(parts),
        }
    }
}

/// Macro to emit a scalar filter wrapper + nullable counterpart that
/// lowers to a `FilterValue::$variant`. Keeps the table of integer /
/// floating / temporal / blob types DRY without sacrificing rustdoc
/// per-type.
macro_rules! scalar_filter {
    (
        $(#[$nn_meta:meta])*
        $name:ident<$rust:ty> => |$conv_v:ident: $rust2:ty| $conv:block as $fv:expr,
        $(#[$null_meta:meta])*
        nullable $null:ident
    ) => {
        $(#[$nn_meta])*
        #[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub struct $name {
            /// `column = value`
            pub equals: Option<$rust>,
            /// Negation.
            pub not: Option<Box<$name>>,
            /// `column IN (...)`
            pub in_list: Option<Vec<$rust>>,
            /// `column NOT IN (...)`
            pub not_in: Option<Vec<$rust>>,
            /// `column < value`
            pub lt: Option<$rust>,
            /// `column <= value`
            pub lte: Option<$rust>,
            /// `column > value`
            pub gt: Option<$rust>,
            /// `column >= value`
            pub gte: Option<$rust>,
        }

        impl $name {
            /// `equals: Some(value)`.
            pub fn equals(v: impl Into<$rust>) -> Self {
                Self { equals: Some(v.into()), ..Default::default() }
            }
            /// `lt: Some(value)`.
            pub fn lt(v: impl Into<$rust>) -> Self {
                Self { lt: Some(v.into()), ..Default::default() }
            }
            /// `lte: Some(value)`.
            pub fn lte(v: impl Into<$rust>) -> Self {
                Self { lte: Some(v.into()), ..Default::default() }
            }
            /// `gt: Some(value)`.
            pub fn gt(v: impl Into<$rust>) -> Self {
                Self { gt: Some(v.into()), ..Default::default() }
            }
            /// `gte: Some(value)`.
            pub fn gte(v: impl Into<$rust>) -> Self {
                Self { gte: Some(v.into()), ..Default::default() }
            }
        }

        impl ScalarFilter for $name {
            fn into_filter(self, column: &str) -> Filter {
                fn to_fv($conv_v: $rust2) -> FilterValue $conv
                let col: crate::filter::FieldName = column.to_string().into();
                let mut parts: Vec<Filter> = Vec::new();
                if let Some(v) = self.equals {
                    parts.push(Filter::Equals(col.clone(), to_fv(v)));
                }
                if let Some(boxed) = self.not {
                    let inner = boxed.into_filter(column);
                    parts.push(Filter::Not(Box::new(inner)));
                }
                if let Some(values) = self.in_list {
                    let vs: Vec<FilterValue> = values.into_iter().map(to_fv).collect();
                    parts.push(Filter::In(col.clone(), vs));
                }
                if let Some(values) = self.not_in {
                    let vs: Vec<FilterValue> = values.into_iter().map(to_fv).collect();
                    parts.push(Filter::NotIn(col.clone(), vs));
                }
                if let Some(v) = self.lt { parts.push(Filter::Lt(col.clone(), to_fv(v))); }
                if let Some(v) = self.lte { parts.push(Filter::Lte(col.clone(), to_fv(v))); }
                if let Some(v) = self.gt { parts.push(Filter::Gt(col.clone(), to_fv(v))); }
                if let Some(v) = self.gte { parts.push(Filter::Gte(col, to_fv(v))); }
                let _ = $fv;
                match parts.len() {
                    0 => Filter::None,
                    1 => parts.into_iter().next().unwrap(),
                    _ => Filter::and(parts),
                }
            }
        }

        $(#[$null_meta])*
        #[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub struct $null {
            /// `column = value`
            pub equals: Option<$rust>,
            /// Negation.
            pub not: Option<Box<$null>>,
            /// `column IN (...)`
            pub in_list: Option<Vec<$rust>>,
            /// `column NOT IN (...)`
            pub not_in: Option<Vec<$rust>>,
            /// `column < value`
            pub lt: Option<$rust>,
            /// `column <= value`
            pub lte: Option<$rust>,
            /// `column > value`
            pub gt: Option<$rust>,
            /// `column >= value`
            pub gte: Option<$rust>,
            /// IS NULL / IS NOT NULL.
            pub is_null: Option<bool>,
        }

        impl ScalarFilter for $null {
            fn into_filter(self, column: &str) -> Filter {
                let mut parts: Vec<Filter> = Vec::new();
                if let Some(b) = self.is_null {
                    parts.push(if b {
                        Filter::IsNull(column.to_string().into())
                    } else {
                        Filter::IsNotNull(column.to_string().into())
                    });
                }
                let inner = $name {
                    equals: self.equals,
                    not: self.not.map(|b| Box::new($name {
                        equals: b.equals,
                        in_list: b.in_list,
                        not_in: b.not_in,
                        lt: b.lt, lte: b.lte, gt: b.gt, gte: b.gte,
                        not: None,
                    })),
                    in_list: self.in_list,
                    not_in: self.not_in,
                    lt: self.lt, lte: self.lte, gt: self.gt, gte: self.gte,
                };
                let f = inner.into_filter(column);
                if !matches!(f, Filter::None) { parts.push(f); }
                match parts.len() {
                    0 => Filter::None,
                    1 => parts.into_iter().next().unwrap(),
                    _ => Filter::and(parts),
                }
            }
        }
    };
}

scalar_filter!(
    /// Filter for non-nullable `Int` (`i32`) columns.
    IntFilter<i32> => |v: i32| { FilterValue::Int(v as i64) } as FilterValue::Int,
    /// Filter for nullable `Int` columns.
    nullable IntNullableFilter
);

scalar_filter!(
    /// Filter for non-nullable `BigInt` (`i64`) columns.
    BigIntFilter<i64> => |v: i64| { FilterValue::Int(v) } as FilterValue::Int,
    /// Filter for nullable `BigInt` columns.
    nullable BigIntNullableFilter
);

scalar_filter!(
    /// Filter for non-nullable `Float` (`f64`) columns.
    FloatFilter<f64> => |v: f64| { FilterValue::Float(v) } as FilterValue::Float,
    /// Filter for nullable `Float` columns.
    nullable FloatNullableFilter
);

scalar_filter!(
    /// Filter for non-nullable `Decimal` (`rust_decimal::Decimal`) columns.
    ///
    /// Lowered as `FilterValue::String` because the runtime IR does not
    /// have a dedicated `Decimal` variant; the driver layer parses it on
    /// the wire.
    DecimalFilter<rust_decimal::Decimal> => |v: rust_decimal::Decimal| { FilterValue::String(v.to_string()) } as FilterValue::String,
    /// Filter for nullable `Decimal` columns.
    nullable DecimalNullableFilter
);

scalar_filter!(
    /// Filter for non-nullable `Uuid` columns.
    UuidFilter<uuid::Uuid> => |v: uuid::Uuid| { FilterValue::String(v.to_string()) } as FilterValue::String,
    /// Filter for nullable `Uuid` columns.
    nullable UuidNullableFilter
);

scalar_filter!(
    /// Filter for non-nullable `Bytes` (`Vec<u8>`) columns.
    ///
    /// Encoded as a base64-of-bytes string in FilterValue::String. The
    /// driver layer decodes back to bytes on the wire.
    BytesFilter<Vec<u8>> => |v: Vec<u8>| {
        use base64::Engine as _;
        FilterValue::String(base64::engine::general_purpose::STANDARD.encode(&v))
    } as FilterValue::String,
    /// Filter for nullable `Bytes` columns.
    nullable BytesNullableFilter
);

/// Filter operators for a non-nullable `Boolean` column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BoolFilter {
    /// `column = value`
    pub equals: Option<bool>,
    /// Negation of the inner filter.
    pub not: Option<Box<BoolFilter>>,
}

impl BoolFilter {
    /// `equals: Some(value)`.
    pub fn equals(v: bool) -> Self {
        Self {
            equals: Some(v),
            ..Default::default()
        }
    }
}

impl From<bool> for BoolFilter {
    fn from(v: bool) -> Self {
        Self::equals(v)
    }
}

impl ScalarFilter for BoolFilter {
    fn into_filter(self, column: &str) -> Filter {
        let col: crate::filter::FieldName = column.to_string().into();
        let mut parts: Vec<Filter> = Vec::new();
        if let Some(v) = self.equals {
            parts.push(Filter::Equals(col.clone(), FilterValue::Bool(v)));
        }
        if let Some(boxed) = self.not {
            parts.push(Filter::Not(Box::new(boxed.into_filter(column))));
        }
        match parts.len() {
            0 => Filter::None,
            1 => parts.into_iter().next().unwrap(),
            _ => Filter::and(parts),
        }
    }
}

/// Filter operators for a nullable `Boolean` column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BoolNullableFilter {
    /// `column = value`
    pub equals: Option<bool>,
    /// Negation.
    pub not: Option<Box<BoolNullableFilter>>,
    /// IS NULL / IS NOT NULL.
    pub is_null: Option<bool>,
}

impl ScalarFilter for BoolNullableFilter {
    fn into_filter(self, column: &str) -> Filter {
        let mut parts: Vec<Filter> = Vec::new();
        if let Some(b) = self.is_null {
            parts.push(if b {
                Filter::IsNull(column.to_string().into())
            } else {
                Filter::IsNotNull(column.to_string().into())
            });
        }
        let inner = BoolFilter {
            equals: self.equals,
            not: self.not.map(|b| {
                Box::new(BoolFilter {
                    equals: b.equals,
                    not: None,
                })
            }),
        };
        let f = inner.into_filter(column);
        if !matches!(f, Filter::None) {
            parts.push(f);
        }
        match parts.len() {
            0 => Filter::None,
            1 => parts.into_iter().next().unwrap(),
            _ => Filter::and(parts),
        }
    }
}

/// Filter operators for a non-nullable `Json` column.
///
/// Phase 1 supports `equals`/`not`. JSON-path operators land behind
/// `SupportsJsonPath` in a follow-up.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct JsonFilter {
    /// `column = value`
    pub equals: Option<serde_json::Value>,
    /// Negation.
    pub not: Option<Box<JsonFilter>>,
}

impl ScalarFilter for JsonFilter {
    fn into_filter(self, column: &str) -> Filter {
        let col: crate::filter::FieldName = column.to_string().into();
        let mut parts: Vec<Filter> = Vec::new();
        if let Some(v) = self.equals {
            parts.push(Filter::Equals(col.clone(), FilterValue::Json(v)));
        }
        if let Some(boxed) = self.not {
            parts.push(Filter::Not(Box::new(boxed.into_filter(column))));
        }
        match parts.len() {
            0 => Filter::None,
            1 => parts.into_iter().next().unwrap(),
            _ => Filter::and(parts),
        }
    }
}

/// Filter operators for a nullable `Json` column.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct JsonNullableFilter {
    /// `column = value`
    pub equals: Option<serde_json::Value>,
    /// Negation.
    pub not: Option<Box<JsonNullableFilter>>,
    /// IS NULL / IS NOT NULL.
    pub is_null: Option<bool>,
}

impl ScalarFilter for JsonNullableFilter {
    fn into_filter(self, column: &str) -> Filter {
        let mut parts: Vec<Filter> = Vec::new();
        if let Some(b) = self.is_null {
            parts.push(if b {
                Filter::IsNull(column.to_string().into())
            } else {
                Filter::IsNotNull(column.to_string().into())
            });
        }
        let inner = JsonFilter {
            equals: self.equals,
            not: self.not.map(|b| {
                Box::new(JsonFilter {
                    equals: b.equals,
                    not: None,
                })
            }),
        };
        let f = inner.into_filter(column);
        if !matches!(f, Filter::None) {
            parts.push(f);
        }
        match parts.len() {
            0 => Filter::None,
            1 => parts.into_iter().next().unwrap(),
            _ => Filter::and(parts),
        }
    }
}
