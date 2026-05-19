//! Prisma schema import functionality.
//!
//! This module provides utilities to parse Prisma schema files and convert
//! them to Prax schema format.

pub mod multi_file;
mod parser;
pub mod types;

pub use multi_file::{PrismaFile, PrismaSourceMap, discover_prisma, parse_and_merge_directory};
pub use parser::{
    import_prisma_schema, import_prisma_schema_file, parse_prisma_file, parse_prisma_schema,
};
pub use types::*;
