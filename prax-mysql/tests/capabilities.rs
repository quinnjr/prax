//! Compile-only test: every advertised capability trait is implemented
//! on `MysqlEngine`. If a future commit accidentally removes an impl,
//! this fails to compile.

use prax_mysql::MysqlEngine;
use prax_query::capabilities::{
    SupportsCorrelatedSubquery, SupportsFullTextSearch, SupportsGeneratedColumns, SupportsJsonPath,
    SupportsNestedWrites, SupportsRelationFilter, SupportsScalarSubqueryInSelect,
};

fn assert_all<E>()
where
    E: SupportsRelationFilter
        + SupportsCorrelatedSubquery
        + SupportsJsonPath
        + SupportsFullTextSearch
        + SupportsGeneratedColumns
        + SupportsScalarSubqueryInSelect
        + SupportsNestedWrites,
{
}

#[test]
fn mysql_engine_impls_all_capabilities() {
    assert_all::<MysqlEngine>();
}
