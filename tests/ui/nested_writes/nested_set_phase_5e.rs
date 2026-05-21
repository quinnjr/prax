//! `create!` with `set:` inside a relation block. Full-relation
//! diff-based replacement is deferred to phase 5e; the user should
//! get a clear deferral diagnostic pointing at the still-supported
//! disconnect / delete alternatives.

prax_orm::prax_schema!("prax/schema.prax");

fn main() {
    let _op = prax_orm::create!(unimplemented!(), for User, {
        data: {
            email: "a@x.com",
            posts: {
                set: [{ id: 1 }],
            },
        },
    });
}
