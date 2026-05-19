//! This file must NOT compile. Cassandra's engine does not impl
//! `SupportsRelationFilter`, so the `assert_relation_filter` call
//! triggers a trait-bound error at the call site.

use prax_query::capabilities::SupportsRelationFilter;
use prax_cassandra::CassandraEngine;

fn assert_relation_filter<E: SupportsRelationFilter>() {}

fn main() {
    assert_relation_filter::<CassandraEngine>();
}
