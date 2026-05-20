//! `select` and `include` are mutually exclusive at the top level.

prax_orm::prax_schema!("prax/schema.prax");

fn main() {
    let _op = prax_orm::find_many!(unimplemented!(), for User, {
        where: { active: true },
        select: { id: true },
        include: { /* no relations on the workspace schema */ },
    });
}
