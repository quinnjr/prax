//! Compile-fail: referencing a model that doesn't exist in the schema
//! via `for Post` produces an "unknown model" compile error.
//!
//! The workspace schema (`prax/schema.prax`) only defines `User`; `Post`
//! is not present.  This fixture locks the "unknown model" error path of
//! the accessor parser.
//!
//! The more targeted "model has no outgoing to-many relations to count"
//! diagnostic (for a model that IS in the schema but has no to-many
//! relations) is covered by the unit test
//! `lower_select_count_on_model_without_to_many_relations_errors` in
//! `prax-codegen/src/macros/lower/select_input.rs`.

fn main() {
    let _op = prax_orm::find_many!(unimplemented!(), for Post, {
        select: { id: true, _count: { anything: true } },
    });
}
