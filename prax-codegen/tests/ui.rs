//! Trybuild UI tests for the prax macro DSL — computed/virtual field diagnostics.
//!
//! Each `*_fail.rs` fixture under `tests/ui/` exercises one compile-time
//! error path in the macro lowering pipeline.  The paired `.stderr` baselines
//! are checked in so CI catches any wording regression.
//!
//! Regenerate baselines after intentional message changes:
//!
//! ```bash
//! TRYBUILD=overwrite cargo test -p prax-codegen --test ui
//! ```
//!
//! The test is gated behind the `ui-tests` feature so that trybuild's
//! rustc-version-sensitive stderr snapshots don't cause noise on every
//! `cargo test` run on different toolchains.  Enable explicitly:
//!
//! ```bash
//! cargo test -p prax-codegen --test ui --features ui-tests
//! ```
//!
//! ## PRAX_SCHEMA
//!
//! The macro lowering helpers resolve the schema at proc-macro expansion
//! time via the `PRAX_SCHEMA` env var (or by walking up to `prax.toml`).
//! This harness sets `PRAX_SCHEMA` to the absolute path of the
//! `prax-codegen` fixture schema so that fixtures can invoke the macros
//! without needing a workspace-level schema that includes computed fields.

#[cfg(feature = "ui-tests")]
#[test]
fn ui() {
    // Point the macro schema resolver at the prax-codegen fixture schema,
    // which declares User (with posts relation, post_count @count, full_name
    // @generated), Post, and Profile.
    let schema_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/schema.prax")
        .canonicalize()
        .expect("fixture schema not found");

    // SAFETY: this is a single-threaded test harness (trybuild spawns
    // its compilation in a subprocess), so setting an env var here is safe.
    unsafe {
        std::env::set_var("PRAX_SCHEMA", &schema_path);
    }

    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*_fail.rs");
}
