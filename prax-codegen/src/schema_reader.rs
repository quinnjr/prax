//! Schema file reading and parsing at compile time.

use std::env;
use std::path::{Path, PathBuf};

use prax_schema::{ModelStyle, PraxConfig, Schema, validate_schema};

/// Result of reading schema and config files.
pub struct SchemaWithConfig {
    /// The parsed schema.
    pub schema: Schema,
    /// The model style from prax.toml (or default).
    pub model_style: ModelStyle,
}

/// Read and parse a schema file, resolving the path relative to the crate root.
#[allow(dead_code)]
pub fn read_and_parse_schema(path: &str) -> Result<Schema, SchemaReadError> {
    let result = read_schema_with_config(path)?;
    Ok(result.schema)
}

/// Read and parse a schema file along with prax.toml configuration.
pub fn read_schema_with_config(path: &str) -> Result<SchemaWithConfig, SchemaReadError> {
    let full_path = resolve_schema_path(path)?;

    let content = std::fs::read_to_string(&full_path).map_err(|e| SchemaReadError::Io {
        path: full_path.display().to_string(),
        error: e.to_string(),
    })?;

    // validate_schema parses and validates in one step
    let schema = validate_schema(&content).map_err(|e| SchemaReadError::Validation {
        path: full_path.display().to_string(),
        error: e.to_string(),
    })?;

    // Try to load prax.toml from the same directory or parent directories
    let model_style = load_prax_config(&full_path)
        .map(|c| c.generator.client.model_style)
        .unwrap_or_default();

    Ok(SchemaWithConfig {
        schema,
        model_style,
    })
}

/// Try to load prax.toml from the schema file's directory or parent directories.
fn load_prax_config(schema_path: &Path) -> Option<PraxConfig> {
    let mut search_dir = schema_path.parent()?;

    // Search up to 5 parent directories
    for _ in 0..5 {
        let config_path = search_dir.join("prax.toml");
        if config_path.exists()
            && let Ok(content) = std::fs::read_to_string(&config_path)
            && let Ok(config) = PraxConfig::from_str(&content)
        {
            return Some(config);
        }
        search_dir = search_dir.parent()?;
    }

    None
}

/// Resolve a schema path relative to the crate root.
fn resolve_schema_path(path: &str) -> Result<PathBuf, SchemaReadError> {
    // Try CARGO_MANIFEST_DIR first (compile time)
    if let Ok(manifest_dir) = env::var("CARGO_MANIFEST_DIR") {
        let full_path = PathBuf::from(manifest_dir).join(path);
        if full_path.exists() {
            return Ok(full_path);
        }
    }

    // Try as absolute path
    let absolute = PathBuf::from(path);
    if absolute.is_absolute() && absolute.exists() {
        return Ok(absolute);
    }

    // Try relative to current directory
    let current_dir = env::current_dir().map_err(|e| SchemaReadError::PathResolution {
        path: path.to_string(),
        error: e.to_string(),
    })?;

    let relative_path = current_dir.join(path);
    if relative_path.exists() {
        return Ok(relative_path);
    }

    // Check common schema locations (prax/ directory is the default)
    let common_paths = [
        "prax/schema.prax", // Default location
        "schema.prax",      // Root level fallback
        "prisma/schema.prax",
        "db/schema.prax",
    ];

    for common in common_paths {
        let common_path = current_dir.join(common);
        if common_path.exists() {
            return Ok(common_path);
        }
    }

    Err(SchemaReadError::NotFound {
        path: path.to_string(),
        searched: vec![
            format!("CARGO_MANIFEST_DIR/{}", path),
            format!("(absolute) {}", path),
            format!("(current_dir) {}", path),
        ],
    })
}

/// Errors that can occur when reading a schema file.
#[derive(Debug)]
#[allow(dead_code)]
pub enum SchemaReadError {
    /// File not found.
    NotFound { path: String, searched: Vec<String> },
    /// IO error reading the file.
    Io { path: String, error: String },
    /// Error parsing the schema.
    Parse { path: String, error: String },
    /// Schema validation error.
    Validation { path: String, error: String },
    /// Error resolving the schema path.
    PathResolution { path: String, error: String },
}

impl std::fmt::Display for SchemaReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound { path, searched } => {
                write!(
                    f,
                    "Schema file '{}' not found. Searched in:\n{}",
                    path,
                    searched
                        .iter()
                        .map(|s| format!("  - {}", s))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            }
            Self::Io { path, error } => {
                write!(f, "Failed to read schema file '{}': {}", path, error)
            }
            Self::Parse { path, error } => {
                write!(f, "Failed to parse schema file '{}':\n{}", path, error)
            }
            Self::Validation { path, error } => {
                write!(f, "Schema validation failed for '{}':\n{}", path, error)
            }
            Self::PathResolution { path, error } => {
                write!(f, "Failed to resolve path '{}': {}", path, error)
            }
        }
    }
}

impl std::error::Error for SchemaReadError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_schema_read_error_display() {
        let err = SchemaReadError::NotFound {
            path: "schema.prax".to_string(),
            searched: vec!["./schema.prax".to_string(), "../schema.prax".to_string()],
        };
        let msg = err.to_string();
        assert!(msg.contains("schema.prax"));
        assert!(msg.contains("not found"));
    }

    #[test]
    fn test_read_valid_schema() {
        // Create a temporary schema file
        let mut temp_file = NamedTempFile::with_suffix(".prax").unwrap();
        writeln!(
            temp_file,
            r#"
            model User {{
                id    Int    @id @auto
                email String @unique
            }}
            "#
        )
        .unwrap();

        let path = temp_file.path().to_str().unwrap();
        let result = read_and_parse_schema(path);

        assert!(result.is_ok(), "Failed to parse schema: {:?}", result.err());
        let schema = result.unwrap();
        assert!(schema.get_model("User").is_some());
    }

    #[test]
    fn test_read_invalid_schema() {
        let mut temp_file = NamedTempFile::with_suffix(".prax").unwrap();
        writeln!(temp_file, "this is not valid schema syntax {{{{").unwrap();

        let path = temp_file.path().to_str().unwrap();
        let result = read_and_parse_schema(path);

        assert!(result.is_err());
        let err = result.unwrap_err();
        // validate_schema wraps both parse and validation errors as Validation
        assert!(matches!(err, SchemaReadError::Validation { .. }));
    }

    #[test]
    fn test_read_nonexistent_file() {
        let result = read_and_parse_schema("/nonexistent/path/schema.prax");
        assert!(result.is_err());
    }
}
