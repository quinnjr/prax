//! `cursor!` requires its key to be an `@id` or `@unique` column.
//! Targeting `name` (non-unique) should error with a span at the key.

prax_orm::prax_schema!("prax/schema.prax");

fn main() {
    let _c = prax_orm::cursor!(User, {
        name: "Alice",
    });
}
