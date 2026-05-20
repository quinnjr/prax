//! `update!` with `unset:` on a non-nullable field should fail with
//! a "not nullable" diagnostic pinned to the operator key.

prax_orm::prax_schema!("prax/schema.prax");

fn main() {
    let _op = prax_orm::update!(unimplemented!(), for User, {
        where: { id: 1 },
        data: { email: { unset: true } },
    });
}
