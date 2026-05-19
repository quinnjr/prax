//! CLI error types and result alias.

use miette::Diagnostic;
use thiserror::Error;

/// Result type alias for CLI operations
pub type CliResult<T> = Result<T, CliError>;

/// CLI error types
#[derive(Error, Debug, Diagnostic)]
pub enum CliError {
    /// IO error
    #[error("IO error: {0}")]
    #[diagnostic(code(prax::io))]
    Io(#[from] std::io::Error),

    /// Configuration error
    #[error("Configuration error: {0}")]
    #[diagnostic(code(prax::config))]
    Config(String),

    /// Schema parsing error
    #[error("Schema error: {0}")]
    #[diagnostic(code(prax::schema))]
    Schema(String),

    /// Validation error
    #[error("Validation error: {0}")]
    #[diagnostic(code(prax::validation))]
    Validation(String),

    /// Migration error
    #[error("Migration error: {0}")]
    #[diagnostic(code(prax::migration))]
    Migration(String),

    /// Database error
    #[error("Database error: {0}")]
    #[diagnostic(code(prax::database))]
    Database(String),

    /// Command error
    #[error("Command error: {0}")]
    #[diagnostic(code(prax::command))]
    Command(String),

    /// Format error
    #[error("Format error: {0}")]
    #[diagnostic(code(prax::format))]
    Format(String),

    /// Code generation error
    #[error("Codegen error: {0}")]
    #[diagnostic(code(prax::codegen))]
    Codegen(String),
}

impl From<toml::de::Error> for CliError {
    fn from(err: toml::de::Error) -> Self {
        CliError::Config(format!("Failed to parse TOML: {}", err))
    }
}

impl From<toml::ser::Error> for CliError {
    fn from(err: toml::ser::Error) -> Self {
        CliError::Config(format!("Failed to serialize TOML: {}", err))
    }
}

impl From<prax_schema::LoadError> for CliError {
    fn from(e: prax_schema::LoadError) -> Self {
        use prax_schema::SchemaError;
        // Resolve SourceId references to file paths for human-readable output.
        let resolved = render_schema_error(&e.error, &e.sources);
        match &e.error {
            SchemaError::ValidationFailed { .. } => CliError::Validation(resolved),
            _ => CliError::Schema(resolved),
        }
    }
}

impl From<prax_schema::SchemaError> for CliError {
    fn from(e: prax_schema::SchemaError) -> Self {
        use prax_schema::SchemaError;
        match &e {
            SchemaError::ValidationFailed { .. } => CliError::Validation(e.to_string()),
            _ => CliError::Schema(e.to_string()),
        }
    }
}

/// Render a SchemaError with file paths resolved from the SourceMap.
fn render_schema_error(err: &prax_schema::SchemaError, sources: &prax_schema::SourceMap) -> String {
    use prax_schema::SchemaError;
    use std::fmt::Write;

    let mut out = err.to_string();
    match err {
        SchemaError::ParseInFile { source, inner } => {
            if let Some(p) = sources.path_of(*source) {
                let _ = write!(out, "\n  in: {}\n  detail: {}", p.display(), inner);
            }
        }
        SchemaError::DuplicateAcrossFiles { first, second, .. }
        | SchemaError::MultipleDatasource { first, second } => {
            if let (Some(a), Some(b)) = (
                sources.path_of(first.source),
                sources.path_of(second.source),
            ) {
                let _ = write!(
                    out,
                    "\n  first:  {} (bytes {}..{})\n  second: {} (bytes {}..{})",
                    a.display(),
                    first.span.start,
                    first.span.end,
                    b.display(),
                    second.span.start,
                    second.span.end,
                );
            }
        }
        SchemaError::ValidationFailed { errors, .. } => {
            for (i, e) in errors.iter().enumerate() {
                let _ = write!(out, "\n  {}. {}", i + 1, render_schema_error(e, sources));
            }
        }
        _ => {}
    }
    out
}
