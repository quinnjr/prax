//! Compile-only test: every advertised capability trait is implemented
//! on `MongoEngine`. If a future commit accidentally removes an impl,
//! this fails to compile.

use prax_mongodb::MongoEngine;
use prax_query::capabilities::{SupportsNestedWrites, SupportsRelationFilter};

fn assert_all<E>()
where
    E: SupportsRelationFilter + SupportsNestedWrites,
{
}

#[test]
fn mongo_engine_impls_all_capabilities() {
    assert_all::<MongoEngine>();
}
