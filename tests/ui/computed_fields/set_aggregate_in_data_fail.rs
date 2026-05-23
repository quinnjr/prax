//! Compile-fail: referencing a field that doesn't exist on the schema
//! model in a `data:` block produces an "unknown field" compile error.
//!
//! `post_count` is not a field on `User` in the workspace fixture
//! schema (`prax/schema.prax`).  The more specific "computed virtual
//! and cannot be assigned" message is locked by the unit tests in
//! `prax-codegen/src/macros/lower/data_input.rs`.
//!
//! Note: this fixture intentionally uses the workspace schema (only
//! `User`) because the PRAX_SCHEMA env var set in the test harness is
//! not forwarded through the incremental compilation cache.  The
//! unit tests cover the "computed virtual" diagnostic path directly.

fn main() {
    let _op = prax_orm::create!(unimplemented!(), for User, {
        data: { email: "a@b.com", post_count: 7 },
    });
}
