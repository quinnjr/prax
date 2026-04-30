//! `trybuild` UI tests for the `#[derive(Model)]` macro.
//!
//! Each `.rs` file under `tests/ui/` is a compile-fail fixture: trybuild
//! invokes rustc on it and compares stderr against the sibling `.stderr`
//! file. If the derive macro's error message changes (better wording, new
//! context, etc.) regenerate the snapshots with:
//!
//! ```bash
//! TRYBUILD=overwrite cargo test --test derive_ui
//! ```
//!
//! Gated behind the `ui-tests` feature because trybuild stderr output is
//! rustc-version sensitive — a new compiler release can invalidate the
//! snapshots even with no code change. Run this locally (or in a CI job
//! pinned to a specific toolchain) with:
//!
//! ```bash
//! cargo test --test derive_ui --features ui-tests
//! ```
//!
//! Intentionally skipped cases (documented here so a future contributor
//! doesn't try to add them back):
//!
//! - **`multiple_ids`**: multiple `#[prax(id)]` fields are *accepted* by
//!   the current derive to support composite primary keys. See
//!   prax-codegen/src/generators/derive.rs — `pk_fields` is a `Vec<_>`
//!   of every id-marked field and the only failure path is the `is_empty`
//!   check. There is no error to snapshot.
//!
//! - **`unknown_prax_attr`**: `#[prax(nonsense)]` is silently ignored by
//!   `parse_nested_meta` — unknown idents fall through without
//!   `return Err(...)`. Turning this into a hard error is a separate
//!   policy change; a trybuild snapshot asserting "silent success" would
//!   be a success-build test, which isn't what this harness is for.

#[cfg(feature = "ui-tests")]
#[test]
fn derive_ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
