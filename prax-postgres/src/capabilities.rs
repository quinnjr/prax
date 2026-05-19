//! Postgres engine capability declarations.
//!
//! Postgres supports the broadest set of features in the workspace:
//! relation filters, correlated subqueries, JSON path, ILIKE
//! (case-insensitive mode), full-text search, native array operators,
//! generated columns, scalar subqueries in SELECT, and nested writes.

use prax_query::capabilities::{
    SupportsArrayOps, SupportsCaseInsensitiveMode, SupportsCorrelatedSubquery,
    SupportsFullTextSearch, SupportsGeneratedColumns, SupportsJsonPath, SupportsNestedWrites,
    SupportsRelationFilter, SupportsScalarSubqueryInSelect,
};

use crate::engine::PgEngine;

impl SupportsRelationFilter for PgEngine {}
impl SupportsCorrelatedSubquery for PgEngine {}
impl SupportsJsonPath for PgEngine {}
impl SupportsCaseInsensitiveMode for PgEngine {}
impl SupportsFullTextSearch for PgEngine {}
impl SupportsArrayOps for PgEngine {}
impl SupportsGeneratedColumns for PgEngine {}
impl SupportsScalarSubqueryInSelect for PgEngine {}
impl SupportsNestedWrites for PgEngine {}
