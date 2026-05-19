//! MSSQL engine capability declarations.
//!
//! MSSQL supports the full SQL capability set minus array operators and
//! case-insensitive mode: relation filters, correlated subqueries, JSON path,
//! full-text search, generated columns, scalar subqueries in SELECT, and
//! nested writes.

use prax_query::capabilities::{
    SupportsCorrelatedSubquery, SupportsFullTextSearch, SupportsGeneratedColumns, SupportsJsonPath,
    SupportsNestedWrites, SupportsRelationFilter, SupportsScalarSubqueryInSelect,
};

use crate::engine::MssqlEngine;

impl SupportsRelationFilter for MssqlEngine {}
impl SupportsCorrelatedSubquery for MssqlEngine {}
impl SupportsJsonPath for MssqlEngine {}
impl SupportsFullTextSearch for MssqlEngine {}
impl SupportsGeneratedColumns for MssqlEngine {}
impl SupportsScalarSubqueryInSelect for MssqlEngine {}
impl SupportsNestedWrites for MssqlEngine {}
