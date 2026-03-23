//! Error types for prax-typegen.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum TypegenError {
    #[error("schema error: {0}")]
    Schema(String),

    #[error("I/O error: {0}")]
    Io(String),

    #[error("generator '{0}' not found in schema")]
    GeneratorNotFound(String),

    #[error("generator '{0}' is disabled")]
    GeneratorDisabled(String),
}
