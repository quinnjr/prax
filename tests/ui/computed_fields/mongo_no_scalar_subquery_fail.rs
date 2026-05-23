//! Compile-fail: `MongoEngine` does not implement
//! `SupportsScalarSubqueryInSelect`, so passing it to a function that
//! requires that capability must fail with a trait bound error.
//!
//! This locks the Task-10 capability gate: MongoDB stays unimplemented
//! until a `$lookup`-lowering pass is added.

fn requires_scalar_subquery<E: prax_query::capabilities::SupportsScalarSubqueryInSelect>() {}

fn main() {
    requires_scalar_subquery::<prax_mongodb::MongoEngine>();
}
