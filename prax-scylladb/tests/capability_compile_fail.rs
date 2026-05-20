//! Pins the intentional capability gap: ScyllaDB must not implement
//! the SQL capability marker traits. trybuild compiles each `.rs`
//! under `tests/ui/` and asserts the captured rustc output matches
//! the committed `.stderr` snapshot.
//!
//! The `.stderr` snapshots are toolchain-version-sensitive. If a Rust
//! upgrade changes diagnostic formatting (line numbers, caret spans,
//! error wording), regenerate via:
//!
//! ```bash
//! TRYBUILD=overwrite cargo test -p prax-scylladb --test capability_compile_fail
//! ```
//!
//! Inspect the new snapshots before committing — only the `error[E…]`
//! line and the `#[diagnostic::on_unimplemented]` note are
//! semantically meaningful.

#[test]
fn cql_engines_decline_sql_capabilities() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/cql_no_relation_filter.rs");
    // Add more compile-fail cases here if needed.
}
