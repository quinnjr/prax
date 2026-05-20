//! `find_unique!` requires `where:` to point at a `@unique` column.
//! Targeting `name` (which is not unique) should fail.

prax_orm::prax_schema!("prax/schema.prax");

fn main() {
    let _op = prax_orm::find_unique!(unimplemented!(), for User, {
        where: { name: "Alice" },
    });
}
