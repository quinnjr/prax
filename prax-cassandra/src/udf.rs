//! User-defined function and aggregate management.
//!
//! Cassandra supports user-defined functions (UDFs) and user-defined
//! aggregates (UDAs) written in Java or JavaScript (the latter deprecated
//! in 4.x). This module provides typed wrappers for CREATE/DROP.

use crate::error::CassandraResult;
use crate::pool::CassandraPool;

/// Supported UDF languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UdfLanguage {
    /// Java (default, recommended).
    Java,
    /// JavaScript (deprecated in Cassandra 4.0+, removed in 5.0).
    JavaScript,
}

impl UdfLanguage {
    /// CQL language identifier.
    pub fn as_str(&self) -> &str {
        match self {
            UdfLanguage::Java => "java",
            UdfLanguage::JavaScript => "javascript",
        }
    }
}

/// Definition of a user-defined function.
#[derive(Debug, Clone)]
pub struct UdfDefinition {
    /// Keyspace the function lives in.
    pub keyspace: String,
    /// Function name.
    pub name: String,
    /// (arg_name, cql_type) pairs.
    pub arguments: Vec<(String, String)>,
    /// Return type (CQL).
    pub return_type: String,
    /// Implementation language.
    pub language: UdfLanguage,
    /// Function body (language-specific source).
    pub body: String,
    /// Whether the function is called when any argument is null.
    pub called_on_null: bool,
}

/// Definition of a user-defined aggregate.
#[derive(Debug, Clone)]
pub struct UdaDefinition {
    /// Keyspace.
    pub keyspace: String,
    /// Aggregate name.
    pub name: String,
    /// CQL argument types.
    pub arg_types: Vec<String>,
    /// State function name.
    pub state_function: String,
    /// State value type (CQL).
    pub state_type: String,
    /// Optional finalizer function name.
    pub final_function: Option<String>,
    /// Optional initial condition.
    pub initial_condition: Option<String>,
}

impl CassandraPool {
    /// Create a user-defined function.
    pub async fn create_function(&self, def: &UdfDefinition) -> CassandraResult<()> {
        let _ = def;
        Err(crate::error::CassandraError::Query(
            "create_function not yet wired to cdrs-tokio".into(),
        ))
    }

    /// Drop a user-defined function.
    pub async fn drop_function(
        &self,
        keyspace: &str,
        name: &str,
        arg_types: &[&str],
    ) -> CassandraResult<()> {
        let _ = (keyspace, name, arg_types);
        Err(crate::error::CassandraError::Query(
            "drop_function not yet wired to cdrs-tokio".into(),
        ))
    }

    /// Create a user-defined aggregate.
    pub async fn create_aggregate(&self, def: &UdaDefinition) -> CassandraResult<()> {
        let _ = def;
        Err(crate::error::CassandraError::Query(
            "create_aggregate not yet wired to cdrs-tokio".into(),
        ))
    }

    /// Drop a user-defined aggregate.
    pub async fn drop_aggregate(
        &self,
        keyspace: &str,
        name: &str,
        arg_types: &[&str],
    ) -> CassandraResult<()> {
        let _ = (keyspace, name, arg_types);
        Err(crate::error::CassandraError::Query(
            "drop_aggregate not yet wired to cdrs-tokio".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_udf_language_as_str() {
        assert_eq!(UdfLanguage::Java.as_str(), "java");
        assert_eq!(UdfLanguage::JavaScript.as_str(), "javascript");
    }

    #[test]
    fn test_udf_definition_construction() {
        let udf = UdfDefinition {
            keyspace: "myapp".into(),
            name: "plus_one".into(),
            arguments: vec![("x".into(), "int".into())],
            return_type: "int".into(),
            language: UdfLanguage::Java,
            body: "return x + 1;".into(),
            called_on_null: false,
        };
        assert_eq!(udf.arguments.len(), 1);
        assert!(!udf.called_on_null);
    }

    #[test]
    fn test_uda_definition_optional_fields() {
        let uda = UdaDefinition {
            keyspace: "myapp".into(),
            name: "my_sum".into(),
            arg_types: vec!["int".into()],
            state_function: "accumulate".into(),
            state_type: "int".into(),
            final_function: None,
            initial_condition: Some("0".into()),
        };
        assert!(uda.final_function.is_none());
        assert_eq!(uda.initial_condition.as_deref(), Some("0"));
    }
}
