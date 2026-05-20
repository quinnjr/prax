//! Unknown top-level key on `find_many!` should be rejected with a
//! "did you mean" suggestion against the allowed key set.

prax_orm::prax_schema!("prax/schema.prax");

fn main() {
    let _op = prax_orm::find_many!(unimplemented!(), for User, {
        wher: { active: true },
    });
}
