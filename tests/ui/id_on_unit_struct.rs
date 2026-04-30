// A unit struct has no named fields. The derive rejects it up front
// with a message naming the "named fields" requirement so users aren't
// confused by a downstream error about a missing `Fields::Named` variant.

use prax_orm::Model;

#[derive(Model)]
struct Foo;

fn main() {}
