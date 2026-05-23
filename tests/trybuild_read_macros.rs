//! `trybuild` UI tests for the phase-3 read-operation macros.
//!
//! Each `.rs` file under `tests/ui/read_macros/` is a compile-fail
//! fixture: trybuild invokes rustc on it and compares stderr against
//! the sibling `.stderr` file. If a diagnostic's wording changes,
//! regenerate the snapshots with:
//!
//! ```bash
//! TRYBUILD=overwrite cargo test --test trybuild_read_macros --features ui-tests
//! ```
//!
//! Gated behind the `ui-tests` feature for the same reason as
//! `derive_ui`: trybuild stderr output is rustc-version sensitive, so
//! every snapshot is implicitly pinned to a specific toolchain.
//!
//! ## Initial baselines
//!
//! Phase-3 ships the `.rs` fixtures but not the `.stderr` snapshots.
//! The first time you run this test on a given toolchain, trybuild
//! will write the observed stderr into `wip/` and report each fixture
//! as a failure. Copy `wip/*.stderr` next to the corresponding `.rs`
//! under `tests/ui/read_macros/` once you've inspected the output
//! and confirmed the diagnostics match what an end user would want.
//!
//! Reason for not committing baselines now: trybuild's child cargo
//! check pulls in the full `prax-orm` dev-dependency closure
//! (including `prax-duckdb` and its bundled `libduckdb-sys` C++
//! build), which takes ~10 min the first time and would extend this
//! crate's test surface area beyond what phase 3 needs to deliver.
//! Phase 7 (docs + cookbook) is the right time to lock in baseline
//! stderrs alongside a CI job that pins the toolchain.

#[cfg(feature = "ui-tests")]
#[test]
fn read_macros_ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/read_macros/*.rs");
}

/// `trybuild` UI tests for the phase-4 shape macros (`where!`,
/// `include!`, `select!`, `order_by!`, `cursor!`).
///
/// Each `.rs` file under `tests/ui/shape_macros/` is a compile-fail
/// fixture asserting one diagnostic class — unknown field, near-miss
/// suggestion, non-unique cursor target, unknown model. Stderrs are
/// rustc-version sensitive; regenerate with:
///
/// ```bash
/// TRYBUILD=overwrite cargo test --test trybuild_read_macros --features ui-tests
/// ```
#[cfg(feature = "ui-tests")]
#[test]
fn shape_macros_ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/shape_macros/*.rs");
}

/// `trybuild` UI tests for the phase-5a write macros (`create!`,
/// `create_many!`, `update!`, `update_many!`, `upsert!`).
///
/// Each `.rs` file under `tests/ui/write_macros/` is a compile-fail
/// fixture asserting one diagnostic class. Most important is the
/// "phase 5b" deferral diagnostic for nested-write relation keys —
/// the wording should point users at the right phase rather than
/// reading like a generic "unknown field" error.
#[cfg(feature = "ui-tests")]
#[test]
fn write_macros_ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/write_macros/*.rs");
}

/// `trybuild` UI tests for the phase-5b nested-write fixtures.
///
/// Each `.rs` file under `tests/ui/nested_writes/` exercises one
/// failure mode of `prax::create!`'s relation-key lowering inside
/// `data:` — unknown operator, phase-5c deferral, phase-5d deferral
/// for `connect_or_create`. Stderrs are rustc-version sensitive;
/// regenerate with:
///
/// ```bash
/// TRYBUILD=overwrite cargo test --test trybuild_read_macros --features ui-tests
/// ```
#[cfg(feature = "ui-tests")]
#[test]
fn nested_writes_ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/nested_writes/*.rs");
}

/// `trybuild` UI tests for phase-5.5 computed/virtual field diagnostics.
///
/// These fixtures assert the four key compile-time error paths introduced
/// in Tasks 12–14:
///
/// - assigning a `@count`/`@sum`/… aggregate field in `data:`
/// - assigning a `@generated` field in `data:`
/// - `_count: { unknown_rel: true }` (with did-you-mean)
/// - `_count` on a model that has zero outgoing to-many relations
///
/// The fixtures live under `tests/ui/computed_fields/` and the schema
/// resolver is pointed at `prax-codegen/tests/fixtures/schema.prax`
/// (which declares the `posts` relation, `post_count @count`, and
/// `full_name @generated` on `User`).
///
/// Regenerate baselines after intentional wording changes:
///
/// ```bash
/// TRYBUILD=overwrite cargo test --test trybuild_read_macros \
///     --features ui-tests computed_fields_ui
/// ```
#[cfg(feature = "ui-tests")]
#[test]
fn computed_fields_ui() {
    // Override the schema resolver to use the richer prax-codegen fixture
    // schema (which has relations + computed fields that the workspace-level
    // prax/schema.prax omits to keep read_macros_e2e lean).
    let schema_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("prax-codegen/tests/fixtures/schema.prax")
        .canonicalize()
        .expect("prax-codegen fixture schema not found — is the prax-codegen crate present?");

    // SAFETY: trybuild spawns child cargo check processes that inherit
    // this env var. This test function is the sole writer.
    unsafe {
        std::env::set_var("PRAX_SCHEMA", &schema_path);
    }

    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/computed_fields/*.rs");

    // Restore the default schema so later tests in the same process use
    // the workspace prax.toml → prax/schema.prax path.
    unsafe {
        std::env::remove_var("PRAX_SCHEMA");
    }
}
