//! Shared schema-loading helper used by every CLI command.
//!
//! Resolves the schema path (from --schema arg or default), then delegates to
//! `prax_schema::load` which auto-detects file vs. directory.

use std::path::{Path, PathBuf};

use prax_schema::{LoadedSchema, load};

use crate::config::SCHEMA_FILE_PATH;
use crate::error::{CliError, CliResult};

/// Load the schema, preferring `args_path` if Some, falling back to the
/// configured / default schema path.
pub fn load_schema(args_path: Option<&Path>) -> CliResult<LoadedSchema> {
    let path: PathBuf = args_path
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(SCHEMA_FILE_PATH));
    if !path.exists() {
        return Err(CliError::Config(format!(
            "Schema not found: {}",
            path.display()
        )));
    }
    Ok(load(&path)?)
}
