//! Shape macros require a model that exists in the schema. Passing
//! a misspelled or fictional model should emit an "unknown model"
//! diagnostic with a "did you mean" suggestion.

prax_orm::prax_schema!("prax/schema.prax");

fn main() {
    let _w = prax_orm::r#where!(Useer, {
        id: { equals: 1 },
    });
}
