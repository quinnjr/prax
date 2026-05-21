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
