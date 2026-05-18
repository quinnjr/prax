//! Scalar field update wrappers.
//!
//! Each `*FieldUpdate` struct carries the atomic operators expressible
//! against one scalar type. Phase 5 (write macros) consumes these;
//! phase 1 only defines them so the codegen scaffolding in phase 2
//! can refer to them.

use serde::{Deserialize, Serialize};

/// Update operators for a non-nullable `Int` (`i32`) column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IntFieldUpdate {
    /// `SET column = value`
    pub set: Option<i64>,
    /// `SET column = column + value`
    pub increment: Option<i64>,
    /// `SET column = column - value`
    pub decrement: Option<i64>,
    /// `SET column = column * value`
    pub multiply: Option<i64>,
    /// `SET column = column / value`
    pub divide: Option<i64>,
}

impl From<i32> for IntFieldUpdate {
    fn from(v: i32) -> Self {
        Self {
            set: Some(v as i64),
            ..Default::default()
        }
    }
}
impl From<i64> for IntFieldUpdate {
    fn from(v: i64) -> Self {
        Self {
            set: Some(v),
            ..Default::default()
        }
    }
}

/// Update operators for a nullable `Int` column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IntNullableFieldUpdate {
    /// `SET column = value`
    pub set: Option<i64>,
    /// `SET column = column + value`
    pub increment: Option<i64>,
    /// `SET column = column - value`
    pub decrement: Option<i64>,
    /// `SET column = column * value`
    pub multiply: Option<i64>,
    /// `SET column = column / value`
    pub divide: Option<i64>,
    /// `SET column = NULL`
    pub unset: Option<bool>,
}

/// Update operators for a non-nullable `BigInt` (`i64`) column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BigIntFieldUpdate {
    /// `SET column = value`
    pub set: Option<i64>,
    /// `SET column = column + value`
    pub increment: Option<i64>,
    /// `SET column = column - value`
    pub decrement: Option<i64>,
    /// `SET column = column * value`
    pub multiply: Option<i64>,
    /// `SET column = column / value`
    pub divide: Option<i64>,
}

impl From<i64> for BigIntFieldUpdate {
    fn from(v: i64) -> Self {
        Self {
            set: Some(v),
            ..Default::default()
        }
    }
}

/// Update operators for a nullable `BigInt` column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BigIntNullableFieldUpdate {
    /// `SET column = value`
    pub set: Option<i64>,
    /// `SET column = column + value`
    pub increment: Option<i64>,
    /// `SET column = column - value`
    pub decrement: Option<i64>,
    /// `SET column = column * value`
    pub multiply: Option<i64>,
    /// `SET column = column / value`
    pub divide: Option<i64>,
    /// `SET column = NULL`
    pub unset: Option<bool>,
}

/// Update operators for a non-nullable `Float` column.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FloatFieldUpdate {
    /// `SET column = value`
    pub set: Option<f64>,
    /// `SET column = column + value`
    pub increment: Option<f64>,
    /// `SET column = column - value`
    pub decrement: Option<f64>,
    /// `SET column = column * value`
    pub multiply: Option<f64>,
    /// `SET column = column / value`
    pub divide: Option<f64>,
}

impl From<f64> for FloatFieldUpdate {
    fn from(v: f64) -> Self {
        Self {
            set: Some(v),
            ..Default::default()
        }
    }
}

/// Update operators for a nullable `Float` column.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FloatNullableFieldUpdate {
    /// `SET column = value`
    pub set: Option<f64>,
    /// `SET column = column + value`
    pub increment: Option<f64>,
    /// `SET column = column - value`
    pub decrement: Option<f64>,
    /// `SET column = column * value`
    pub multiply: Option<f64>,
    /// `SET column = column / value`
    pub divide: Option<f64>,
    /// `SET column = NULL`
    pub unset: Option<bool>,
}

/// Update operators for a non-nullable `Decimal` column (transmitted as string).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DecimalFieldUpdate {
    /// `SET column = value`
    pub set: Option<String>,
    /// `SET column = column + value`
    pub increment: Option<String>,
    /// `SET column = column - value`
    pub decrement: Option<String>,
    /// `SET column = column * value`
    pub multiply: Option<String>,
    /// `SET column = column / value`
    pub divide: Option<String>,
}

/// Update operators for a nullable `Decimal` column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DecimalNullableFieldUpdate {
    /// `SET column = value`
    pub set: Option<String>,
    /// `SET column = column + value`
    pub increment: Option<String>,
    /// `SET column = column - value`
    pub decrement: Option<String>,
    /// `SET column = column * value`
    pub multiply: Option<String>,
    /// `SET column = column / value`
    pub divide: Option<String>,
    /// `SET column = NULL`
    pub unset: Option<bool>,
}

/// Update operators for a non-nullable `String` column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StringFieldUpdate {
    /// `SET column = value`
    pub set: Option<String>,
}

impl From<&str> for StringFieldUpdate {
    fn from(v: &str) -> Self {
        Self {
            set: Some(v.into()),
        }
    }
}
impl From<String> for StringFieldUpdate {
    fn from(v: String) -> Self {
        Self { set: Some(v) }
    }
}

/// Update operators for a nullable `String` column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StringNullableFieldUpdate {
    /// `SET column = value`
    pub set: Option<String>,
    /// `SET column = NULL`
    pub unset: Option<bool>,
}

impl From<&str> for StringNullableFieldUpdate {
    fn from(v: &str) -> Self {
        Self {
            set: Some(v.into()),
            unset: None,
        }
    }
}
impl From<String> for StringNullableFieldUpdate {
    fn from(v: String) -> Self {
        Self {
            set: Some(v),
            unset: None,
        }
    }
}

/// Update operators for a non-nullable `Boolean` column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BoolFieldUpdate {
    /// `SET column = value`
    pub set: Option<bool>,
}

impl From<bool> for BoolFieldUpdate {
    fn from(v: bool) -> Self {
        Self { set: Some(v) }
    }
}

/// Update operators for a nullable `Boolean` column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BoolNullableFieldUpdate {
    /// `SET column = value`
    pub set: Option<bool>,
    /// `SET column = NULL`
    pub unset: Option<bool>,
}

/// Update operators for an enum-typed column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "snake_case",
    bound = "E: Serialize + for<'de2> Deserialize<'de2>"
)]
pub struct EnumFieldUpdate<E> {
    /// `SET column = value`
    pub set: Option<E>,
}

/// Update operators for a nullable enum-typed column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "snake_case",
    bound = "E: Serialize + for<'de2> Deserialize<'de2>"
)]
pub struct EnumNullableFieldUpdate<E> {
    /// `SET column = value`
    pub set: Option<E>,
    /// `SET column = NULL`
    pub unset: Option<bool>,
}

/// Update operators for a non-nullable `DateTime` column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DateTimeFieldUpdate {
    /// `SET column = value` (RFC3339-encoded).
    pub set: Option<String>,
}

/// Update operators for a nullable `DateTime` column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DateTimeNullableFieldUpdate {
    /// `SET column = value` (RFC3339-encoded).
    pub set: Option<String>,
    /// `SET column = NULL`
    pub unset: Option<bool>,
}

/// Update operators for a non-nullable `Bytes` column (base64-encoded).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BytesFieldUpdate {
    /// `SET column = value`
    pub set: Option<String>,
}

/// Update operators for a nullable `Bytes` column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BytesNullableFieldUpdate {
    /// `SET column = value`
    pub set: Option<String>,
    /// `SET column = NULL`
    pub unset: Option<bool>,
}

/// Update operators for a non-nullable `Uuid` column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct UuidFieldUpdate {
    /// `SET column = value`
    pub set: Option<String>,
}

/// Update operators for a nullable `Uuid` column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct UuidNullableFieldUpdate {
    /// `SET column = value`
    pub set: Option<String>,
    /// `SET column = NULL`
    pub unset: Option<bool>,
}

/// Update operators for a non-nullable `Json` column.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct JsonFieldUpdate {
    /// `SET column = value`
    pub set: Option<serde_json::Value>,
}

/// Update operators for a nullable `Json` column.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct JsonNullableFieldUpdate {
    /// `SET column = value`
    pub set: Option<serde_json::Value>,
    /// `SET column = NULL`
    pub unset: Option<bool>,
}
