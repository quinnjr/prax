//! Diesel schema import functionality.
//!
//! This module provides utilities to parse Diesel schema files and convert
//! them to Prax schema format.

mod parser;
pub mod types;

pub use parser::{
    import_diesel_schema, import_diesel_schema_file, parse_diesel_file, parse_diesel_schema,
};
pub use types::*;
