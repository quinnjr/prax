//! `update!` with an `increment:` operator on a String field should
//! fail with a "non-numeric" diagnostic pinned to the operator key.

prax_orm::prax_schema!("prax/schema.prax");

fn main() {
    let _op = prax_orm::update!(unimplemented!(), for User, {
        where: { id: 1 },
        data: { email: { increment: 1 } },
    });
}
