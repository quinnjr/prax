//! End-to-end smoke tests for the phase-4 shape macros.
//!
//! Each test asserts:
//!   1. The macro can be invoked (it compiles).
//!   2. The result has the expected concrete type (the per-model
//!      typed input struct from phase-2 codegen).
//!
//! Composition with the read macros is exercised in Task 7's added
//! tests; this file ships the per-macro happy-path proof as each task
//! lands.

#![allow(dead_code)]
#![allow(unused_imports)]

// `prax_orm::prax_schema!` emits the per-model module that the shape
// macros refer to in their lowered output.
prax_orm::prax_schema!("prax/schema.prax");

#[test]
fn where_macro_returns_user_where_input() {
    let w: user::UserWhereInput = prax_orm::r#where!(User, {
        id: { equals: 1 },
    });
    // `UserWhereInput` is a struct with `id: Option<IntFilter>`; if
    // the macro emitted the wrong type, this binding would not compile.
    let _ = w;
}

#[test]
fn where_macro_raw_identifier_path_compiles() {
    // `where` is a Rust keyword and cannot appear as a bare path
    // segment, so callers must use the `r#where!` raw-identifier form
    // when reaching through `prax_orm::`. (At the unprefixed call site
    // `where!(...)` would also fail to parse.) Documented this way for
    // discoverability.
    let _w: user::UserWhereInput = prax_orm::r#where!(User, { active: true });
}

#[test]
fn where_macro_with_spread_composes() {
    let base = prax_orm::r#where!(User, { active: true });
    // Spread the base value into a wider filter; this exercises that
    // the macro emits a real `Default`-able struct value.
    let w = user::UserWhereInput {
        email: Some(prax_query::inputs::StringFilter::equals("alice@x.com")),
        ..base
    };
    let _: user::UserWhereInput = w;
}
