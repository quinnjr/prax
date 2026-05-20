//! Typo on a real field name should suggest the closest match.

prax_orm::prax_schema!("prax/schema.prax");

fn main() {
    let _op = prax_orm::find_many!(unimplemented!(), for User, {
        where: { emial: "x" },
    });
}
