//! Compile-fail: referencing a field that doesn't exist on the schema
//! model in a `data:` block produces an "unknown field" compile error.
//!
//! `full_name` is not a field on `User` in the workspace fixture
//! schema (`prax/schema.prax`).  The more specific "computed virtual
//! and cannot be assigned" message for `@generated` fields is locked
//! by the unit tests in
//! `prax-codegen/src/macros/lower/data_input.rs`.

fn main() {
    let _op = prax_orm::create!(unimplemented!(), for User, {
        data: { email: "a@b.com", full_name: "Alice Smith" },
    });
}
