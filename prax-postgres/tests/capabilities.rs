//! Compile-only test: every advertised capability trait is implemented
//! on `PgEngine`. If a future commit accidentally removes an impl,
//! this fails to compile.

use prax_postgres::PgEngine;
use prax_query::capabilities::{
    SupportsArrayOps, SupportsCaseInsensitiveMode, SupportsCorrelatedSubquery,
    SupportsFullTextSearch, SupportsGeneratedColumns, SupportsJsonPath, SupportsNestedWrites,
    SupportsRelationFilter, SupportsScalarSubqueryInSelect,
};

fn assert_all<E>()
where
    E: SupportsRelationFilter
        + SupportsCorrelatedSubquery
        + SupportsJsonPath
        + SupportsCaseInsensitiveMode
        + SupportsFullTextSearch
        + SupportsArrayOps
        + SupportsGeneratedColumns
        + SupportsScalarSubqueryInSelect
        + SupportsNestedWrites,
{
}

#[test]
fn postgres_engine_impls_all_capabilities() {
    assert_all::<PgEngine>();
}
