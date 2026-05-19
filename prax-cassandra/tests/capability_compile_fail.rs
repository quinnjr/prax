//! Pins the intentional capability gap: Cassandra must not implement
//! the SQL capability marker traits. trybuild compiles each `.rs`
//! under `tests/ui/` and asserts the captured rustc output matches
//! the committed `.stderr` snapshot.

#[test]
fn cql_engines_decline_sql_capabilities() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/cql_no_relation_filter.rs");
    // Add more compile-fail cases here if needed.
}
