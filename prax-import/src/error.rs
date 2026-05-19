//! Error types for schema import operations.

use miette::Diagnostic;
use thiserror::Error;

/// Result type for import operations.
pub type ImportResult<T> = Result<T, ImportError>;

/// Errors that can occur during schema import.
#[derive(Error, Debug, Diagnostic)]
pub enum ImportError {
    /// Failed to parse Prisma schema.
    #[error("Failed to parse Prisma schema: {0}")]
    #[diagnostic(code(prax_import::prisma::parse_error))]
    PrismaParseError(String),

    /// Failed to parse Diesel schema.
    #[error("Failed to parse Diesel schema: {0}")]
    #[diagnostic(code(prax_import::diesel::parse_error))]
    DieselParseError(String),

    /// Unsupported feature in source schema.
    #[error("Unsupported feature in schema: {0}")]
    #[diagnostic(code(prax_import::unsupported_feature))]
    UnsupportedFeature(String),

    /// Failed to convert type.
    #[error("Failed to convert type: {0}")]
    #[diagnostic(code(prax_import::type_conversion_error))]
    TypeConversionError(String),

    /// Invalid relation definition.
    #[error("Invalid relation: {0}")]
    #[diagnostic(code(prax_import::invalid_relation))]
    InvalidRelation(String),

    /// File I/O error.
    #[error("File I/O error: {0}")]
    #[diagnostic(code(prax_import::io_error))]
    IoError(#[from] std::io::Error),

    /// Schema error from prax-schema.
    #[error("Schema error: {0}")]
    #[diagnostic(code(prax_import::schema_error))]
    SchemaError(#[from] prax_schema::SchemaError),

    /// Invalid configuration.
    #[error("Invalid configuration: {0}")]
    #[diagnostic(code(prax_import::invalid_config))]
    InvalidConfig(String),

    /// Duplicate Prisma model across files in a multi-file directory.
    #[error("duplicate prisma model `{name}` across files (source {} vs {})", .first.0, .second.0)]
    #[diagnostic(code(prax_import::prisma::duplicate_model))]
    DuplicatePrismaModel {
        name: String,
        first: crate::prisma::types::PrismaSourceId,
        second: crate::prisma::types::PrismaSourceId,
    },

    /// Duplicate Prisma enum across files.
    #[error("duplicate prisma enum `{name}` across files (source {} vs {})", .first.0, .second.0)]
    #[diagnostic(code(prax_import::prisma::duplicate_enum))]
    DuplicatePrismaEnum {
        name: String,
        first: crate::prisma::types::PrismaSourceId,
        second: crate::prisma::types::PrismaSourceId,
    },

    /// Multiple datasource blocks across Prisma files.
    #[error("multiple datasource blocks across prisma files (source {} vs {})", .first.0, .second.0)]
    #[diagnostic(code(prax_import::prisma::multiple_datasource))]
    MultiplePrismaDatasource {
        first: crate::prisma::types::PrismaSourceId,
        second: crate::prisma::types::PrismaSourceId,
    },

    /// Multi-file Prisma input directory contained no `*.prisma` files.
    #[error("no .prisma files found under `{}`", .path.display())]
    #[diagnostic(code(prax_import::prisma::empty_directory))]
    EmptyPrismaDirectory { path: std::path::PathBuf },
}
