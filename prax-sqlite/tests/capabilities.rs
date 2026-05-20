//! Compile-only test: every advertised capability trait is implemented
//! on `SqliteEngine`. If a future commit accidentally removes an impl,
//! this fails to compile.

use prax_query::capabilities::{
    SupportsCorrelatedSubquery, SupportsGeneratedColumns, SupportsJsonPath, SupportsNestedWrites,
    SupportsRelationFilter, SupportsScalarSubqueryInSelect,
};
use prax_sqlite::SqliteEngine;

fn assert_all<E>()
where
    E: SupportsRelationFilter
        + SupportsCorrelatedSubquery
        + SupportsJsonPath
        + SupportsGeneratedColumns
        + SupportsScalarSubqueryInSelect
        + SupportsNestedWrites,
{
}

#[test]
fn sqlite_engine_impls_all_capabilities() {
    assert_all::<SqliteEngine>();
}
