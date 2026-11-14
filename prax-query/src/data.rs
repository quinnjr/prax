//! Ergonomic data creation utilities.
//!
//! This module provides multiple ways to create data for insert/update operations,
//! from simple struct-based approaches to flexible builders.
//!
//! # Approaches
//!
//! ## 1. Struct-Based (Recommended for simple creates)
//!
//! ```rust,ignore
//! // Generated struct with required/optional fields
//! let user = client.user().create(UserCreate {
//!     email: "bob@example.com".into(),
//!     name: Some("Bob".into()),
//!     ..Default::default()
//! }).exec().await?;
//! ```
//!
//! ## 2. Builder Pattern (For complex creates with relations)
//!
//! ```rust,ignore
//! let user = client.user()
//!     .create_with(|b| b
//!         .email("bob@example.com")
//!         .name("Bob")
//!         .posts(vec![
//!             PostCreate { title: "Hello".into(), ..Default::default() }
//!         ])
//!     )
//!     .exec().await?;
//! ```
//!
//! ## 3. Macro (Ultra-concise)
//!
//! ```rust,ignore
//! let user = client.user().create(data! {
//!     email: "bob@example.com",
//!     name: "Bob",
//! }).exec().await?;
//! ```
//!
//! ## 4. From tuples (Quick and dirty)
//!
//! ```rust,ignore
//! // For models with few required fields
//! let user = client.user().create(("bob@example.com", "Bob")).exec().await?;
//! ```

use crate::filter::FilterValue;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Trait for types that can be used as create data.
pub trait CreateData: Send + Sync {
    /// Get the field values as a map.
    fn into_fields(self) -> HashMap<String, FieldValue>;

    /// Get the model name.
    fn model_name() -> &'static str;
}

/// Trait for types that can be used as update data.
pub trait UpdateData: Send + Sync {
    /// Get the field values as a map (only set fields).
    fn into_fields(self) -> HashMap<String, FieldValue>;

    /// Get the model name.
    fn model_name() -> &'static str;
}

/// A field value that can be set in create/update operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FieldValue {
    /// Null value.
    Null,
    /// Boolean value.
    Bool(bool),
    /// Integer value.
    Int(i64),
    /// Float value.
    Float(f64),
    /// String value.
    String(String),
    /// JSON value.
    Json(serde_json::Value),
    /// Bytes value.
    Bytes(Vec<u8>),
    /// DateTime as ISO string.
    DateTime(String),
    /// UUID as string.
    Uuid(String),
    /// Array of values.
    Array(Vec<FieldValue>),
    /// Nested create data.
    Nested(Box<DataBuilder>),
    /// Connect to existing record.
    Connect(ConnectData),
    /// Disconnect from related record.
    Disconnect,
    /// Set to default value.
    Default,
    /// Increment by value.
    Increment(i64),
    /// Decrement by value.
    Decrement(i64),
    /// Multiply by value.
    Multiply(f64),
    /// Divide by value.
    Divide(f64),
    /// Append to array.
    Push(Box<FieldValue>),
    /// Unset the field.
    Unset,
}

impl FieldValue {
    /// Convert to FilterValue for query operations.
    ///
    /// `Bytes` becomes [`FilterValue::Bytes`] (bound as raw bytes) and
    /// `Array` becomes [`FilterValue::Array`] (one array-valued parameter),
    /// converting its elements recursively. Write operations (`Default`,
    /// `Increment`, `Connect`, ...) have no filter value and yield `None`,
    /// as does an `Array` containing one.
    pub fn to_filter_value(&self) -> Option<FilterValue> {
        match self {
            Self::Null => Some(FilterValue::Null),
            Self::Bool(b) => Some(FilterValue::Bool(*b)),
            Self::Int(i) => Some(FilterValue::Int(*i)),
            Self::Float(f) => Some(FilterValue::Float(*f)),
            Self::String(s) => Some(FilterValue::String(s.clone())),
            Self::Json(j) => Some(FilterValue::Json(j.clone())),
            Self::Bytes(b) => Some(FilterValue::Bytes(b.clone())),
            Self::DateTime(s) => Some(FilterValue::String(s.clone())),
            Self::Uuid(s) => Some(FilterValue::String(s.clone())),
            Self::Array(items) => items
                .iter()
                .map(Self::to_filter_value)
                .collect::<Option<Vec<_>>>()
                .map(FilterValue::Array),
            _ => None,
        }
    }
}

// Convenient From implementations
impl From<bool> for FieldValue {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}

impl From<i32> for FieldValue {
    fn from(v: i32) -> Self {
        Self::Int(v as i64)
    }
}

impl From<i64> for FieldValue {
    fn from(v: i64) -> Self {
        Self::Int(v)
    }
}

impl From<f32> for FieldValue {
    fn from(v: f32) -> Self {
        Self::Float(v as f64)
    }
}

impl From<f64> for FieldValue {
    fn from(v: f64) -> Self {
        Self::Float(v)
    }
}

impl From<String> for FieldValue {
    fn from(v: String) -> Self {
        Self::String(v)
    }
}

impl From<&str> for FieldValue {
    fn from(v: &str) -> Self {
        Self::String(v.to_string())
    }
}

impl From<serde_json::Value> for FieldValue {
    fn from(v: serde_json::Value) -> Self {
        Self::Json(v)
    }
}

impl<T: Into<FieldValue>> From<Option<T>> for FieldValue {
    fn from(v: Option<T>) -> Self {
        match v {
            Some(val) => val.into(),
            None => Self::Null,
        }
    }
}

impl<T: Into<FieldValue>> From<Vec<T>> for FieldValue {
    fn from(v: Vec<T>) -> Self {
        Self::Array(v.into_iter().map(Into::into).collect())
    }
}

/// Data for connecting to an existing record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectData {
    /// Field name to match on.
    pub field: String,
    /// Value to match.
    pub value: Box<FieldValue>,
}

impl ConnectData {
    /// Create a connect by ID.
    pub fn id(id: impl Into<FieldValue>) -> Self {
        Self {
            field: "id".to_string(),
            value: Box::new(id.into()),
        }
    }

    /// Create a connect by a specific field.
    pub fn by(field: impl Into<String>, value: impl Into<FieldValue>) -> Self {
        Self {
            field: field.into(),
            value: Box::new(value.into()),
        }
    }
}

/// A flexible data builder for create/update operations.
///
/// This builder allows setting fields dynamically and supports
/// nested creates, connects, and all update operations.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DataBuilder {
    fields: HashMap<String, FieldValue>,
}

impl DataBuilder {
    /// Create a new empty data builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a field value.
    pub fn set(mut self, field: impl Into<String>, value: impl Into<FieldValue>) -> Self {
        self.fields.insert(field.into(), value.into());
        self
    }

    /// Set a field to null.
    pub fn set_null(mut self, field: impl Into<String>) -> Self {
        self.fields.insert(field.into(), FieldValue::Null);
        self
    }

    /// Set a field to its default value.
    pub fn set_default(mut self, field: impl Into<String>) -> Self {
        self.fields.insert(field.into(), FieldValue::Default);
        self
    }

    /// Unset a field (for updates).
    pub fn unset(mut self, field: impl Into<String>) -> Self {
        self.fields.insert(field.into(), FieldValue::Unset);
        self
    }

    /// Increment a numeric field.
    pub fn increment(mut self, field: impl Into<String>, by: i64) -> Self {
        self.fields.insert(field.into(), FieldValue::Increment(by));
        self
    }

    /// Decrement a numeric field.
    pub fn decrement(mut self, field: impl Into<String>, by: i64) -> Self {
        self.fields.insert(field.into(), FieldValue::Decrement(by));
        self
    }

    /// Multiply a numeric field.
    pub fn multiply(mut self, field: impl Into<String>, by: f64) -> Self {
        self.fields.insert(field.into(), FieldValue::Multiply(by));
        self
    }

    /// Divide a numeric field.
    pub fn divide(mut self, field: impl Into<String>, by: f64) -> Self {
        self.fields.insert(field.into(), FieldValue::Divide(by));
        self
    }

    /// Push a value to an array field.
    pub fn push(mut self, field: impl Into<String>, value: impl Into<FieldValue>) -> Self {
        self.fields
            .insert(field.into(), FieldValue::Push(Box::new(value.into())));
        self
    }

    /// Connect to an existing related record by ID.
    pub fn connect(mut self, relation: impl Into<String>, id: impl Into<FieldValue>) -> Self {
        self.fields
            .insert(relation.into(), FieldValue::Connect(ConnectData::id(id)));
        self
    }

    /// Connect to an existing related record by a specific field.
    pub fn connect_by(
        mut self,
        relation: impl Into<String>,
        field: impl Into<String>,
        value: impl Into<FieldValue>,
    ) -> Self {
        self.fields.insert(
            relation.into(),
            FieldValue::Connect(ConnectData::by(field, value)),
        );
        self
    }

    /// Disconnect from a related record.
    pub fn disconnect(mut self, relation: impl Into<String>) -> Self {
        self.fields.insert(relation.into(), FieldValue::Disconnect);
        self
    }

    /// Create a nested record.
    pub fn create_nested(mut self, relation: impl Into<String>, data: DataBuilder) -> Self {
        self.fields
            .insert(relation.into(), FieldValue::Nested(Box::new(data)));
        self
    }

    /// Get the fields map.
    pub fn into_fields(self) -> HashMap<String, FieldValue> {
        self.fields
    }

    /// Check if a field is set.
    pub fn has(&self, field: &str) -> bool {
        self.fields.contains_key(field)
    }

    /// Get a field value.
    pub fn get(&self, field: &str) -> Option<&FieldValue> {
        self.fields.get(field)
    }

    /// Get the number of fields set.
    pub fn len(&self) -> usize {
        self.fields.len()
    }

    /// Check if no fields are set.
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }
}

/// Helper trait for converting to DataBuilder.
pub trait IntoData {
    /// Convert into a DataBuilder.
    fn into_data(self) -> DataBuilder;
}

impl IntoData for DataBuilder {
    fn into_data(self) -> DataBuilder {
        self
    }
}

impl IntoData for HashMap<String, FieldValue> {
    fn into_data(self) -> DataBuilder {
        DataBuilder { fields: self }
    }
}

impl IntoData for serde_json::Value {
    fn into_data(self) -> DataBuilder {
        match self {
            serde_json::Value::Object(map) => {
                let fields = map
                    .into_iter()
                    .map(|(k, v)| (k, json_to_field_value(v)))
                    .collect();
                DataBuilder { fields }
            }
            _ => DataBuilder::new(),
        }
    }
}

fn json_to_field_value(value: serde_json::Value) -> FieldValue {
    match value {
        serde_json::Value::Null => FieldValue::Null,
        serde_json::Value::Bool(b) => FieldValue::Bool(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                FieldValue::Int(i)
            } else if let Some(f) = n.as_f64() {
                FieldValue::Float(f)
            } else {
                FieldValue::Json(serde_json::Value::Number(n))
            }
        }
        serde_json::Value::String(s) => FieldValue::String(s),
        serde_json::Value::Array(arr) => {
            FieldValue::Array(arr.into_iter().map(json_to_field_value).collect())
        }
        serde_json::Value::Object(_) => FieldValue::Json(value),
    }
}

/// Macro for concise data creation.
///
/// # Examples
///
/// ```rust,ignore
/// use prax_query::data;
///
/// // Simple create
/// let user_data = data! {
///     email: "bob@example.com",
///     name: "Bob",
///     age: 30,
/// };
///
/// // With nested data
/// let post_data = data! {
///     title: "Hello World",
///     author: connect!(id: 1),
///     tags: ["rust", "orm"],
/// };
///
/// // With optional fields
/// let update_data = data! {
///     name: "Robert",
///     bio: null,
///     views: increment!(1),
/// };
/// ```
#[macro_export]
macro_rules! data {
    // Empty data
    () => {
        $crate::data::DataBuilder::new()
    };

    // Data with fields
    ($($field:ident : $value:expr),* $(,)?) => {{
        let mut builder = $crate::data::DataBuilder::new();
        $(
            builder = builder.set(stringify!($field), $value);
        )*
        builder
    }};
}

/// Macro for creating connection data.
#[macro_export]
macro_rules! connect {
    (id: $id:expr) => {
        $crate::data::FieldValue::Connect($crate::data::ConnectData::id($id))
    };
    ($field:ident : $value:expr) => {
        $crate::data::FieldValue::Connect($crate::data::ConnectData::by(stringify!($field), $value))
    };
}

/// Macro for increment operations.
#[macro_export]
macro_rules! increment {
    ($value:expr) => {
        $crate::data::FieldValue::Increment($value)
    };
}

/// Macro for decrement operations.
#[macro_export]
macro_rules! decrement {
    ($value:expr) => {
        $crate::data::FieldValue::Decrement($value)
    };
}

/// Batch create helper for creating multiple records.
#[derive(Debug, Clone, Default)]
pub struct BatchCreate<T> {
    items: Vec<T>,
    skip_duplicates: bool,
}

impl<T> BatchCreate<T> {
    /// Create a new batch create.
    pub fn new(items: Vec<T>) -> Self {
        Self {
            items,
            skip_duplicates: false,
        }
    }

    /// Skip duplicate records instead of failing.
    pub fn skip_duplicates(mut self) -> Self {
        self.skip_duplicates = true;
        self
    }

    /// Get the items.
    pub fn into_items(self) -> Vec<T> {
        self.items
    }

    /// Check if duplicates should be skipped.
    pub fn should_skip_duplicates(&self) -> bool {
        self.skip_duplicates
    }

    /// Get the number of items.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl<T> From<Vec<T>> for BatchCreate<T> {
    fn from(items: Vec<T>) -> Self {
        Self::new(items)
    }
}

/// Builder for creating records with a fluent API.
///
/// This is generated per-model and provides type-safe field setters.
///
/// # Example (Generated Code)
///
/// ```rust,ignore
/// // This is what gets generated for a User model:
/// pub struct UserCreateBuilder {
///     data: DataBuilder,
/// }
///
/// impl UserCreateBuilder {
///     pub fn email(self, email: impl Into<String>) -> Self { ... }
///     pub fn name(self, name: impl Into<String>) -> Self { ... }
///     pub fn age(self, age: i32) -> Self { ... }
///     pub fn posts(self, posts: Vec<PostCreate>) -> Self { ... }
/// }
/// ```
pub trait TypedCreateBuilder: Sized {
    /// The output type.
    type Output: CreateData;

    /// Build the create data.
    fn build(self) -> Self::Output;
}

/// Builder for updating records with a fluent API.
pub trait TypedUpdateBuilder: Sized {
    /// The output type.
    type Output: UpdateData;

    /// Build the update data.
    fn build(self) -> Self::Output;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_filter_value_maps_bytes_and_arrays() {
        assert_eq!(
            FieldValue::Bytes(vec![0, 159, 255]).to_filter_value(),
            Some(FilterValue::Bytes(vec![0, 159, 255]))
        );
        assert_eq!(
            FieldValue::Array(vec![
                FieldValue::Int(1),
                FieldValue::Null,
                FieldValue::Array(vec![FieldValue::Bytes(vec![7])]),
            ])
            .to_filter_value(),
            Some(FilterValue::Array(vec![
                FilterValue::Int(1),
                FilterValue::Null,
                FilterValue::Array(vec![FilterValue::Bytes(vec![7])]),
            ]))
        );
        assert_eq!(
            FieldValue::Array(Vec::new()).to_filter_value(),
            Some(FilterValue::Array(Vec::new()))
        );
        // One element without a filter value makes the whole array unmappable.
        assert_eq!(
            FieldValue::Array(vec![FieldValue::Int(1), FieldValue::Default]).to_filter_value(),
            None
        );
    }

    #[test]
    fn test_data_builder_basic() {
        let data = DataBuilder::new()
            .set("name", "Bob")
            .set("age", 30)
            .set("active", true);

        assert_eq!(data.len(), 3);
        assert!(data.has("name"));
        assert!(data.has("age"));
        assert!(data.has("active"));
    }

    #[test]
    fn test_data_builder_null_and_default() {
        let data = DataBuilder::new()
            .set_null("deleted_at")
            .set_default("created_at");

        assert!(matches!(data.get("deleted_at"), Some(FieldValue::Null)));
        assert!(matches!(data.get("created_at"), Some(FieldValue::Default)));
    }

    #[test]
    fn test_data_builder_numeric_operations() {
        let data = DataBuilder::new()
            .increment("views", 1)
            .decrement("stock", 5)
            .multiply("price", 1.1)
            .divide("score", 2.0);

        assert!(matches!(data.get("views"), Some(FieldValue::Increment(1))));
        assert!(matches!(data.get("stock"), Some(FieldValue::Decrement(5))));
    }

    #[test]
    fn test_data_builder_connect() {
        let data = DataBuilder::new()
            .connect("author", 1)
            .connect_by("category", "slug", "tech");

        assert!(matches!(data.get("author"), Some(FieldValue::Connect(_))));
        assert!(matches!(data.get("category"), Some(FieldValue::Connect(_))));
    }

    #[test]
    fn test_data_macro() {
        let data = data! {
            name: "Bob",
            email: "bob@example.com",
            age: 30,
        };

        assert_eq!(data.len(), 3);
        assert!(matches!(data.get("name"), Some(FieldValue::String(s)) if s == "Bob"));
    }

    #[test]
    #[allow(clippy::approx_constant)]
    fn test_field_value_conversions() {
        let _: FieldValue = true.into();
        let _: FieldValue = 42_i32.into();
        let _: FieldValue = 42_i64.into();
        let _: FieldValue = 3.14_f64.into();
        let _: FieldValue = "hello".into();
        let _: FieldValue = String::from("hello").into();
        let _: FieldValue = Some("optional").into();
        let _: FieldValue = None::<String>.into();
        let _: FieldValue = vec!["a", "b"].into();
    }

    #[test]
    fn test_batch_create() {
        let batch: BatchCreate<DataBuilder> =
            vec![data! { name: "Alice" }, data! { name: "Bob" }].into();

        assert_eq!(batch.len(), 2);
        assert!(!batch.should_skip_duplicates());

        let batch = batch.skip_duplicates();
        assert!(batch.should_skip_duplicates());
    }

    #[test]
    fn test_json_to_data() {
        let json = serde_json::json!({
            "name": "Bob",
            "age": 30,
            "active": true,
            "tags": ["a", "b"]
        });

        let data: DataBuilder = json.into_data();
        assert_eq!(data.len(), 4);
    }

    #[test]
    fn test_connect_data() {
        let by_id = ConnectData::id(1);
        assert_eq!(by_id.field, "id");

        let by_email = ConnectData::by("email", "bob@example.com");
        assert_eq!(by_email.field, "email");
    }
}
