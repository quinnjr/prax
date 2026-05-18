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
