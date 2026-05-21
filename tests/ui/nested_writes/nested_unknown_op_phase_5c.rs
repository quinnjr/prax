//! `create!` with an unknown nested operator inside a relation block
//! in `data:`. Phase 5b only ships `create:` and `connect:`; everything
//! else should produce a phase-5c (or 5d for `connect_or_create`)
//! deferral diagnostic pointing at the offending operator key.

prax_orm::prax_schema!("prax/schema.prax");

fn main() {
    let _op = prax_orm::create!(unimplemented!(), for User, {
        data: {
            email: "a@x.com",
            posts: {
                update: [{ where: { id: 1 }, data: { title: "x" } }],
            },
        },
    });
}
