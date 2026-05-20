//! This file must NOT compile. ScyllaDB's engine does not impl
//! `SupportsRelationFilter`, so the `assert_relation_filter` call
//! triggers a trait-bound error at the call site.

use prax_query::capabilities::SupportsRelationFilter;
use prax_scylladb::ScyllaEngine;

fn assert_relation_filter<E: SupportsRelationFilter>() {}

fn main() {
    assert_relation_filter::<ScyllaEngine>();
}
