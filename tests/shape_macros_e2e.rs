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

#[test]
fn select_macro_returns_user_select() {
    let s: user::UserSelect = prax_orm::select!(User, {
        id: true,
        email: true,
    });
    // The selection struct is `Default + Clone`; round-trip via clone
    // to make sure the value is fully owned.
    let _ = s.clone();
}

#[test]
fn select_macro_default_when_empty() {
    // Empty selection block is allowed — produces a UserSelect with
    // all Option::None which lowers to Select::All at runtime.
    let _s: user::UserSelect = prax_orm::select!(User, {});
}

#[test]
fn include_macro_returns_user_include() {
    // `User` in the workspace fixture schema has no relation fields,
    // so the include block is empty. The macro should still resolve
    // and emit a `UserInclude` value.
    let _i: user::UserInclude = prax_orm::include!(User, {});
}

#[test]
fn order_by_macro_single_block_returns_order_by() {
    let ob: prax_query::types::OrderBy = prax_orm::order_by!(User, { created_at: desc });
    assert!(!ob.is_empty());
}

#[test]
fn order_by_macro_list_returns_multi_field_order_by() {
    let ob: prax_query::types::OrderBy = prax_orm::order_by!(User, [
        { active: desc },
        { email: asc },
    ]);
    // Multi-field sort lowers to OrderBy::Fields with two entries.
    match ob {
        prax_query::types::OrderBy::Fields(fs) => assert_eq!(fs.len(), 2),
        other => panic!("expected OrderBy::Fields, got {:?}", other),
    }
}

#[test]
fn cursor_macro_id_column_returns_where_unique_input() {
    let _c: user::UserWhereUniqueInput = prax_orm::cursor!(User, { id: 42 });
}

#[test]
fn cursor_macro_unique_email_column_returns_where_unique_input() {
    let _c: user::UserWhereUniqueInput = prax_orm::cursor!(User, { email: "alice@x.com" });
}
