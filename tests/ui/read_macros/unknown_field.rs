//! `find_many!` with a field name that doesn't exist on the model.

prax_orm::prax_schema!("prax/schema.prax");

fn main() {
    let _op = prax_orm::find_many!(unimplemented!(), for User, {
        where: { nonexistent_field: "x" },
    });
}
