//! Error types for schema parsing and validation.

// These warnings are false positives - the fields are used by derive macros
#![allow(unused_assignments)]

use miette::Diagnostic;
use thiserror::Error;

/// Result type for schema operations.
pub type SchemaResult<T> = Result<T, SchemaError>;

/// Errors that can occur during schema parsing and validation.
#[derive(Error, Debug, Diagnostic)]
pub enum SchemaError {
    /// Error reading a file.
    #[error("failed to read file: {path}")]
    #[diagnostic(code(prax::schema::io_error))]
    IoError {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// Syntax error in the schema file.
    #[error("syntax error in schema")]
    #[diagnostic(code(prax::schema::syntax_error))]
    SyntaxError {
        #[source_code]
        src: String,
        #[label("error here")]
        span: miette::SourceSpan,
        message: String,
    },

    /// Invalid model definition.
    #[error("invalid model `{name}`: {message}")]
    #[diagnostic(code(prax::schema::invalid_model))]
    InvalidModel { name: String, message: String },

    /// Invalid field definition.
    #[error("invalid field `{model}.{field}`: {message}")]
    #[diagnostic(code(prax::schema::invalid_field))]
    InvalidField {
        model: String,
        field: String,
        message: String,
    },

    /// Invalid relation definition.
    #[error("invalid relation `{model}.{field}`: {message}")]
    #[diagnostic(code(prax::schema::invalid_relation))]
    InvalidRelation {
        model: String,
        field: String,
        message: String,
    },

    /// Duplicate definition.
    #[error("duplicate {kind} `{name}`")]
    #[diagnostic(code(prax::schema::duplicate))]
    Duplicate { kind: String, name: String },

    /// Unknown type reference.
    #[error("unknown type `{type_name}` in `{model}.{field}`")]
    #[diagnostic(code(prax::schema::unknown_type))]
    UnknownType {
        model: String,
        field: String,
        type_name: String,
    },

    /// Invalid attribute.
    #[error("invalid attribute `@{attribute}`: {message}")]
    #[diagnostic(code(prax::schema::invalid_attribute))]
    InvalidAttribute { attribute: String, message: String },

    /// Missing required attribute.
    #[error("model `{model}` is missing required `@id` field")]
    #[diagnostic(code(prax::schema::missing_id))]
    MissingId { model: String },

    /// Configuration error.
    #[error("configuration error: {message}")]
    #[diagnostic(code(prax::schema::config_error))]
    ConfigError { message: String },

    /// TOML parsing error.
    #[error("failed to parse TOML")]
    #[diagnostic(code(prax::schema::toml_error))]
    TomlError {
        #[source]
        source: toml::de::Error,
    },

    /// Validation error with multiple issues.
    #[error("schema validation failed with {count} error(s)")]
    #[diagnostic(code(prax::schema::validation_failed))]
    ValidationFailed {
        count: usize,
        #[related]
        errors: Vec<SchemaError>,
    },

    /// A Vector field is missing a required @dim(N) attribute.
    #[error("field '{field}' of type Vector is missing required @dim attribute")]
    #[diagnostic(code(prax::schema::missing_vector_dimension))]
    MissingVectorDimension {
        /// Field name.
        field: String,
    },

    /// A Vector field has an invalid @vectorType value.
    #[error("invalid vector element type '{value}' (expected one of: float2, float4, float8, int1, int2, int4)")]
    #[diagnostic(code(prax::schema::invalid_vector_type))]
    InvalidVectorType {
        /// Supplied type value.
        value: String,
    },

    /// A Vector field has an invalid @metric value.
    #[error("invalid vector metric '{value}' (expected one of: cosine, l2, inner)")]
    #[diagnostic(code(prax::schema::invalid_vector_metric))]
    InvalidVectorMetric {
        /// Supplied metric value.
        value: String,
    },

    /// A Vector field has an invalid @index value.
    #[error("invalid vector index '{value}' (expected: hnsw)")]
    #[diagnostic(code(prax::schema::invalid_vector_index))]
    InvalidVectorIndex {
        /// Supplied index value.
        value: String,
    },
}

impl SchemaError {
    /// Create a syntax error with source location.
    pub fn syntax(
        src: impl Into<String>,
        offset: usize,
        len: usize,
        message: impl Into<String>,
    ) -> Self {
        Self::SyntaxError {
            src: src.into(),
            span: (offset, len).into(),
            message: message.into(),
        }
    }

    /// Create an invalid model error.
    pub fn invalid_model(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self::InvalidModel {
            name: name.into(),
            message: message.into(),
        }
    }

    /// Create an invalid field error.
    pub fn invalid_field(
        model: impl Into<String>,
        field: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::InvalidField {
            model: model.into(),
            field: field.into(),
            message: message.into(),
        }
    }

    /// Create an invalid relation error.
    pub fn invalid_relation(
        model: impl Into<String>,
        field: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::InvalidRelation {
            model: model.into(),
            field: field.into(),
            message: message.into(),
        }
    }

    /// Create a duplicate definition error.
    pub fn duplicate(kind: impl Into<String>, name: impl Into<String>) -> Self {
        Self::Duplicate {
            kind: kind.into(),
            name: name.into(),
        }
    }

    /// Create an unknown type error.
    pub fn unknown_type(
        model: impl Into<String>,
        field: impl Into<String>,
        type_name: impl Into<String>,
    ) -> Self {
        Self::UnknownType {
            model: model.into(),
            field: field.into(),
            type_name: type_name.into(),
        }
    }
}

#[cfg(test)]
#[allow(unused_assignments)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_result_type() {
        let ok_result: SchemaResult<i32> = Ok(42);
        assert!(ok_result.is_ok());
        assert_eq!(ok_result.unwrap(), 42);

        let err_result: SchemaResult<i32> = Err(SchemaError::ConfigError {
            message: "test".to_string(),
        });
        assert!(err_result.is_err());
    }

    // ==================== Error Constructor Tests ====================

    #[test]
    fn test_syntax_error() {
        let err = SchemaError::syntax("model User { }", 6, 4, "unexpected token");

        match err {
            SchemaError::SyntaxError { src, span, message } => {
                assert_eq!(src, "model User { }");
                assert_eq!(span.offset(), 6);
                assert_eq!(span.len(), 4);
                assert_eq!(message, "unexpected token");
            }
            _ => panic!("Expected SyntaxError"),
        }
    }

    #[test]
    fn test_invalid_model_error() {
        let err = SchemaError::invalid_model("User", "missing id field");

        match err {
            SchemaError::InvalidModel { name, message } => {
                assert_eq!(name, "User");
                assert_eq!(message, "missing id field");
            }
            _ => panic!("Expected InvalidModel"),
        }
    }

    #[test]
    fn test_invalid_field_error() {
        let err = SchemaError::invalid_field("User", "email", "invalid type");

        match err {
            SchemaError::InvalidField {
                model,
                field,
                message,
            } => {
                assert_eq!(model, "User");
                assert_eq!(field, "email");
                assert_eq!(message, "invalid type");
            }
            _ => panic!("Expected InvalidField"),
        }
    }

    #[test]
    fn test_invalid_relation_error() {
        let err = SchemaError::invalid_relation("Post", "author", "missing foreign key");

        match err {
            SchemaError::InvalidRelation {
                model,
                field,
                message,
            } => {
                assert_eq!(model, "Post");
                assert_eq!(field, "author");
                assert_eq!(message, "missing foreign key");
            }
            _ => panic!("Expected InvalidRelation"),
        }
    }

    #[test]
    fn test_duplicate_error() {
        let err = SchemaError::duplicate("model", "User");

        match err {
            SchemaError::Duplicate { kind, name } => {
                assert_eq!(kind, "model");
                assert_eq!(name, "User");
            }
            _ => panic!("Expected Duplicate"),
        }
    }

    #[test]
    fn test_unknown_type_error() {
        let err = SchemaError::unknown_type("Post", "category", "Category");

        match err {
            SchemaError::UnknownType {
                model,
                field,
                type_name,
            } => {
                assert_eq!(model, "Post");
                assert_eq!(field, "category");
                assert_eq!(type_name, "Category");
            }
            _ => panic!("Expected UnknownType"),
        }
    }

    // ==================== Error Display Tests ====================

    #[test]
    fn test_io_error_display() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err = SchemaError::IoError {
            path: "schema.prax".to_string(),
            source: io_err,
        };

        let display = format!("{}", err);
        assert!(display.contains("schema.prax"));
    }

    #[test]
    fn test_syntax_error_display() {
        let err = SchemaError::syntax("model", 0, 5, "unexpected");
        let display = format!("{}", err);
        assert!(display.contains("syntax error"));
    }

    #[test]
    fn test_invalid_model_display() {
        let err = SchemaError::invalid_model("User", "test message");
        let display = format!("{}", err);
        assert!(display.contains("User"));
        assert!(display.contains("test message"));
    }

    #[test]
    fn test_invalid_field_display() {
        let err = SchemaError::invalid_field("User", "email", "test");
        let display = format!("{}", err);
        assert!(display.contains("User.email"));
    }

    #[test]
    fn test_invalid_relation_display() {
        let err = SchemaError::invalid_relation("Post", "author", "test");
        let display = format!("{}", err);
        assert!(display.contains("Post.author"));
    }

    #[test]
    fn test_duplicate_display() {
        let err = SchemaError::duplicate("model", "User");
        let display = format!("{}", err);
        assert!(display.contains("duplicate"));
        assert!(display.contains("model"));
        assert!(display.contains("User"));
    }

    #[test]
    fn test_unknown_type_display() {
        let err = SchemaError::unknown_type("Post", "author", "UserType");
        let display = format!("{}", err);
        assert!(display.contains("UserType"));
        assert!(display.contains("Post.author"));
    }

    #[test]
    fn test_missing_id_display() {
        let err = SchemaError::MissingId {
            model: "User".to_string(),
        };
        let display = format!("{}", err);
        assert!(display.contains("User"));
        assert!(display.contains("@id"));
    }

    #[test]
    fn test_config_error_display() {
        let err = SchemaError::ConfigError {
            message: "invalid URL".to_string(),
        };
        let display = format!("{}", err);
        assert!(display.contains("invalid URL"));
    }

    #[test]
    fn test_validation_failed_display() {
        let err = SchemaError::ValidationFailed {
            count: 3,
            errors: vec![],
        };
        let display = format!("{}", err);
        assert!(display.contains("3"));
    }

    // ==================== Error Debug Tests ====================

    #[test]
    fn test_error_debug() {
        let err = SchemaError::invalid_model("User", "test");
        let debug = format!("{:?}", err);
        assert!(debug.contains("InvalidModel"));
        assert!(debug.contains("User"));
    }

    // ==================== Error From Constructors Tests ====================

    #[test]
    fn test_syntax_from_strings() {
        let src = String::from("content");
        let msg = String::from("message");
        let err = SchemaError::syntax(src, 0, 7, msg);

        if let SchemaError::SyntaxError { src, message, .. } = err {
            assert_eq!(src, "content");
            assert_eq!(message, "message");
        } else {
            panic!("Expected SyntaxError");
        }
    }

    #[test]
    fn test_invalid_model_from_strings() {
        let name = String::from("Model");
        let msg = String::from("error");
        let err = SchemaError::invalid_model(name, msg);

        if let SchemaError::InvalidModel { name, message } = err {
            assert_eq!(name, "Model");
            assert_eq!(message, "error");
        } else {
            panic!("Expected InvalidModel");
        }
    }

    #[test]
    fn test_invalid_field_from_strings() {
        let model = String::from("User");
        let field = String::from("email");
        let msg = String::from("error");
        let err = SchemaError::invalid_field(model, field, msg);

        if let SchemaError::InvalidField {
            model,
            field,
            message,
        } = err
        {
            assert_eq!(model, "User");
            assert_eq!(field, "email");
            assert_eq!(message, "error");
        } else {
            panic!("Expected InvalidField");
        }
    }
}
