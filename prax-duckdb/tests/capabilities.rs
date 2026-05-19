//! Compile-only test: every advertised capability trait is implemented
//! on `DuckDbEngine`. If a future commit accidentally removes an impl,
//! this fails to compile.

use prax_duckdb::DuckDbEngine;
use prax_query::capabilities::{
    SupportsCorrelatedSubquery, SupportsGeneratedColumns, SupportsNestedWrites,
    SupportsRelationFilter, SupportsScalarSubqueryInSelect,
};

fn assert_all<E>()
where
    E: SupportsRelationFilter
        + SupportsCorrelatedSubquery
        + SupportsGeneratedColumns
        + SupportsScalarSubqueryInSelect
        + SupportsNestedWrites,
{
}

#[test]
fn duckdb_engine_impls_all_capabilities() {
    assert_all::<DuckDbEngine>();
}
