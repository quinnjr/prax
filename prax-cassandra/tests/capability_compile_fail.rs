//! Pins the intentional capability gap: Cassandra must not implement
//! the SQL capability marker traits. trybuild compiles each `.rs`
//! under `tests/ui/` and asserts the captured rustc output matches
//! the committed `.stderr` snapshot.
//!
//! The `.stderr` snapshots are toolchain-version-sensitive. If a Rust
//! upgrade changes diagnostic formatting, regenerate via:
//!
//! ```bash
//! TRYBUILD=overwrite cargo test -p prax-cassandra --test capability_compile_fail
//! ```
//!
//! Only the `error[E…]` line and the `#[diagnostic::on_unimplemented]`
//! note are semantically meaningful; line numbers / caret spans / source
//! echoes are noise that may legitimately shift across toolchains.

#[test]
fn cql_engines_decline_sql_capabilities() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/cql_no_relation_filter.rs");
    // Add more compile-fail cases here if needed.
}
