// A #[derive(Model)] struct with no #[prax(id)] field should fail with a
// clear error that names the missing primary-key attribute, not with some
// generic codegen failure deep inside the emitted module.

use prax_orm::Model;

#[derive(Model)]
#[prax(table = "no_pk")]
struct NoPk {
    name: String,
    email: String,
}

fn main() {}
