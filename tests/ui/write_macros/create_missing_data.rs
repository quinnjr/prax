//! `create!` without a `data:` block should fail with a clear
//! required-key diagnostic.

prax_orm::prax_schema!("prax/schema.prax");

fn main() {
    let _op = prax_orm::create!(unimplemented!(), for User, {
        select: { id: true },
    });
}
