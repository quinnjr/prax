//! Prisma schema import functionality.
//!
//! This module provides utilities to parse Prisma schema files and convert
//! them to Prax schema format.

mod parser;
pub mod types;

pub use parser::{import_prisma_schema, import_prisma_schema_file, parse_prisma_file, parse_prisma_schema};
pub use types::*;
