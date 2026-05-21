//! `create!` using `connect_or_create:` inside a relation block in
//! `data:`. `connect_or_create` requires engine-specific lowerings
//! (Postgres `INSERT ... ON CONFLICT`, MySQL `INSERT ... ON DUPLICATE
//! KEY UPDATE`, etc.) and lands in phase 5d.

prax_orm::prax_schema!("prax/schema.prax");

fn main() {
    let _op = prax_orm::create!(unimplemented!(), for User, {
        data: {
            email: "a@x.com",
            posts: {
                connect_or_create: [{
                    where: { id: 1 },
                    create: { title: "x" },
                }],
            },
        },
    });
}
