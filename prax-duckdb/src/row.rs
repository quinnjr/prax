//! Row deserialization for DuckDB.

use serde::de::DeserializeOwned;
use serde_json::Value as JsonValue;

use crate::error::{DuckDbError, DuckDbResult};

/// Trait for deserializing DuckDB rows.
pub trait FromDuckDbRow: Sized {
    /// Deserialize from a JSON value.
    fn from_json(json: JsonValue) -> DuckDbResult<Self>;
}

impl<T: DeserializeOwned> FromDuckDbRow for T {
    fn from_json(json: JsonValue) -> DuckDbResult<Self> {
        serde_json::from_value(json)
            .map_err(|e| DuckDbError::deserialization(format!("Failed to deserialize row: {}", e)))
    }
}

/// Extension trait for converting JSON values to typed rows.
pub trait JsonRowExt {
    /// Convert to a typed row.
    fn to_row<T: FromDuckDbRow>(&self) -> DuckDbResult<T>;
}

impl JsonRowExt for JsonValue {
    fn to_row<T: FromDuckDbRow>(&self) -> DuckDbResult<T> {
        T::from_json(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq)]
    struct User {
        id: i64,
        name: String,
    }

    #[test]
    fn test_from_json() {
        let json = serde_json::json!({
            "id": 1,
            "name": "Alice"
        });

        let user: User = User::from_json(json).unwrap();
        assert_eq!(user.id, 1);
        assert_eq!(user.name, "Alice");
    }

    #[test]
    fn test_json_row_ext() {
        let json = serde_json::json!({
            "id": 2,
            "name": "Bob"
        });

        let user: User = json.to_row().unwrap();
        assert_eq!(user.id, 2);
        assert_eq!(user.name, "Bob");
    }

    #[test]
    fn test_from_json_error() {
        let json = serde_json::json!({
            "id": "not a number",
            "name": "Alice"
        });

        let result: DuckDbResult<User> = User::from_json(json);
        assert!(result.is_err());
    }
}
