//! SQLite engine capability declarations.
//!
//! SQLite supports relation filters, correlated subqueries, JSON path
//! (via the JSON1 extension, enabled by default in modern SQLite builds),
//! generated columns, scalar subqueries in SELECT, and nested writes.
//! It does NOT support native case-insensitive mode (`ILIKE`), full-text
//! search predicates, or native array operators.

use prax_query::capabilities::{
    SupportsCorrelatedSubquery, SupportsGeneratedColumns, SupportsJsonPath, SupportsNestedWrites,
    SupportsRelationFilter, SupportsScalarSubqueryInSelect,
};

use crate::engine::SqliteEngine;

impl SupportsRelationFilter for SqliteEngine {}
impl SupportsCorrelatedSubquery for SqliteEngine {}
impl SupportsJsonPath for SqliteEngine {}
impl SupportsGeneratedColumns for SqliteEngine {}
impl SupportsScalarSubqueryInSelect for SqliteEngine {}
impl SupportsNestedWrites for SqliteEngine {}
