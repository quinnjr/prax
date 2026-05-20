//! DuckDB engine capability declarations.
//!
//! DuckDB supports relation filters, correlated subqueries, generated columns,
//! scalar subqueries in SELECT, and nested writes.  It does NOT support native
//! case-insensitive mode (`ILIKE`), full-text search predicates, JSON path
//! expressions, or native array operators.

use prax_query::capabilities::{
    SupportsCorrelatedSubquery, SupportsGeneratedColumns, SupportsNestedWrites,
    SupportsRelationFilter, SupportsScalarSubqueryInSelect,
};

use crate::engine::DuckDbEngine;

impl SupportsRelationFilter for DuckDbEngine {}
impl SupportsCorrelatedSubquery for DuckDbEngine {}
impl SupportsGeneratedColumns for DuckDbEngine {}
impl SupportsScalarSubqueryInSelect for DuckDbEngine {}
impl SupportsNestedWrites for DuckDbEngine {}
