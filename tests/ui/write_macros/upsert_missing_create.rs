//! `upsert!` without a `create:` block should fail with a clear
//! required-key diagnostic.

prax_orm::prax_schema!("prax/schema.prax");

fn main() {
    let _op = prax_orm::upsert!(unimplemented!(), for User, {
        where: { email: "a@x.com" },
        update: { name: { set: "Renamed" } },
    });
}
