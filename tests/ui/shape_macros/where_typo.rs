//! `where!` shape macro with a near-miss on `email`. Expect a "did
//! you mean `email`?" suggestion.

prax_orm::prax_schema!("prax/schema.prax");

fn main() {
    let _w = prax_orm::r#where!(User, {
        emial: "x",
    });
}
