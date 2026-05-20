//! `where!` shape macro with a field name that doesn't exist on the
//! model. Expect a clear "unknown field" diagnostic with a span at
//! the bad key.

prax_orm::prax_schema!("prax/schema.prax");

fn main() {
    let _w = prax_orm::r#where!(User, {
        nonexistent_field: "x",
    });
}
