//! Error type for vector operations.

/// Errors produced by the prax-sqlite vector API.
#[derive(Debug, thiserror::Error)]
pub enum VectorError {
    /// Embedding dimensions did not match the column definition.
    #[error("Dimension mismatch: expected {expected}, got {got}")]
    DimensionMismatch {
        /// Expected dimensions (from schema).
        expected: usize,
        /// Actual dimensions supplied at runtime.
        got: usize,
    },

    /// The distance metric is not valid for the element type.
    #[error("Unsupported metric for element type {element_type}")]
    UnsupportedMetric {
        /// Element type name (e.g. "int1").
        element_type: &'static str,
    },

    /// sqlite-vector-rs extension was not registered on the connection.
    #[error("sqlite-vector-rs extension not loaded on connection")]
    ExtensionNotLoaded,

    /// Wrapped rusqlite error.
    #[error("rusqlite error: {0}")]
    Rusqlite(#[from] rusqlite::Error),

    /// Wrapped driver-level error from sqlite-vector-rs.
    #[error("sqlite-vector-rs error: {0}")]
    Driver(String),
}

/// Convenience alias for `Result<T, VectorError>`.
pub type VectorResult<T> = Result<T, VectorError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dimension_mismatch_display() {
        let err = VectorError::DimensionMismatch {
            expected: 1536,
            got: 768,
        };
        assert_eq!(
            err.to_string(),
            "Dimension mismatch: expected 1536, got 768"
        );
    }

    #[test]
    fn test_unsupported_metric_display() {
        let err = VectorError::UnsupportedMetric {
            element_type: "int1",
        };
        assert!(err.to_string().contains("int1"));
    }

    #[test]
    fn test_extension_not_loaded_display() {
        let err = VectorError::ExtensionNotLoaded;
        assert!(err.to_string().contains("not loaded"));
    }
}
