//! `create!` with an unknown field inside `data:`. The diagnostic
//! should suggest the closest match (`email`) and pin the span at the
//! offending key.

prax_orm::prax_schema!("prax/schema.prax");

fn main() {
    let _op = prax_orm::create!(unimplemented!(), for User, {
        data: { emial: "x@y.com", active: true },
    });
}
