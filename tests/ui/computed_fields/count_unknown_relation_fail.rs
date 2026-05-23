//! Compile-fail: `_count` on a model with no outgoing to-many relations
//! must emit "model has no outgoing to-many relations to count".
//!
//! The workspace schema (`prax/schema.prax`) defines `User` without any
//! relation fields, so any `_count: { ... }` request is rejected.
//!
//! The "did-you-mean" variant (unknown relation on a model that HAS
//! relations but the name is a typo) is covered by the unit test
//! `lower_select_count_accessor_unknown_relation_did_you_mean` in
//! `prax-codegen/src/macros/lower/select_input.rs`.

fn main() {
    let _op = prax_orm::find_many!(unimplemented!(), for User, {
        select: { id: true, _count: { pots: true } },
    });
}
