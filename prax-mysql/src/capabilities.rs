//! MySQL engine capability declarations.
//!
//! MySQL supports relation filters, correlated subqueries, JSON path
//! (MySQL >= 5.7), full-text search, generated columns, scalar subqueries
//! in SELECT, and nested writes. It does NOT support native
//! case-insensitive mode (`ILIKE`) or native array operators.

use prax_query::capabilities::{
    SupportsCorrelatedSubquery, SupportsFullTextSearch, SupportsGeneratedColumns, SupportsJsonPath,
    SupportsNestedWrites, SupportsRelationFilter, SupportsScalarSubqueryInSelect,
};

use crate::engine::MysqlEngine;

impl SupportsRelationFilter for MysqlEngine {}
impl SupportsCorrelatedSubquery for MysqlEngine {}
impl SupportsJsonPath for MysqlEngine {}
impl SupportsFullTextSearch for MysqlEngine {}
impl SupportsGeneratedColumns for MysqlEngine {}
impl SupportsScalarSubqueryInSelect for MysqlEngine {}
impl SupportsNestedWrites for MysqlEngine {}
