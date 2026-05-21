//! `create!` using a where-style scalar operator (`equals:`) inside a
//! relation block in `data:`. Should produce an "unknown nested
//! operator" diagnostic suggesting `create` or `connect`.

prax_orm::prax_schema!("prax/schema.prax");

fn main() {
    let _op = prax_orm::create!(unimplemented!(), for User, {
        data: {
            email: "a@x.com",
            posts: {
                equals: 1,
            },
        },
    });
}
