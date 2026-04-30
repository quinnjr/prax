//! Emit per-relation codegen accessors.
//!
//! Each `#[prax(relation(...))]`-annotated field materializes as a
//! nested `pub mod <field_name>` inside the per-model module. The
//! module contains:
//!
//! - `fn fetch()` — returns a [`prax_query::relations::IncludeSpec`]
//!   keyed by the field's Rust name, so callers can write
//!   `c.user().find_many().include(user::posts::fetch())`.
//! - `struct Relation` — a zero-sized type implementing
//!   [`prax_query::relations::RelationMeta`] with the owner, target,
//!   kind, local key, and foreign key known at const time. The runtime
//!   relation executor consumes this impl when dispatching a
//!   [`ModelRelationLoader`] call.
//!
//! Paths: the emitted `Relation` impl refers to `super::super::Owner`
//! and `super::super::Target`. The first `super` hops from the
//! nested field module to the model module; the second hops from the
//! model module to the crate root where `#[derive(Model)]` places the
//! struct. Keep this aligned with `derive_client::emit` — that emitter
//! uses `super::#name` for the same reason.

use proc_macro2::TokenStream;
use quote::quote;
use syn::Ident;

/// Classification of a relation as it appears in a derive's field
/// attributes. Mirrors [`prax_query::relations::RelationKind`] but
/// avoids pulling the runtime enum into the codegen crate.
pub enum RelationKindTokens {
    /// Owner holds a FK to the target's PK.
    BelongsTo,
    /// Target holds a FK to the owner's PK (1-to-N).
    HasMany,
    /// Target holds a unique FK to the owner's PK (1-to-1).
    HasOne,
}

/// Inputs for [`emit`]. References are borrowed from the derive's
/// `FieldInfo` so the codegen never allocates a transient struct for
/// relation metadata.
pub struct RelationSpec<'a> {
    /// The field's Rust name — becomes the nested module name.
    pub field_name: &'a Ident,
    /// The model type declaring the relation (`Owner`).
    pub owner: &'a Ident,
    /// The related model type (`Target`).
    pub target: &'a Ident,
    /// Classification of the relation.
    pub kind: RelationKindTokens,
    /// Column on `Owner` that references `Target` (used for
    /// `BelongsTo`). Defaulted to `"id"` by the parser for the other
    /// kinds.
    pub local_key: &'a str,
    /// Column on `Target` that references `Owner`'s PK (used for
    /// `HasMany` / `HasOne`).
    pub foreign_key: &'a str,
}

/// Emit `pub mod <field>` with `fetch()`, `Relation`, and the nested
/// write helpers `connect()` / `create()`.
///
/// `connect()` / `create()` return a [`::prax_query::nested::NestedWriteOp`]
/// suitable for feeding into `CreateOperation::with(...)`. The child
/// payload for `create()` is a `Vec<Vec<(String, FilterValue)>>` rather
/// than a strongly-typed `<target>::CreateInput` because the derive
/// emission path doesn't materialize a per-model `CreateInput` struct —
/// switching to a typed payload would require either generating one or
/// branching the codegen between derive/schema paths. The untyped
/// payload keeps both paths compatible at the cost of losing compile-
/// time column-name checking on the nested-create arm.
pub fn emit(spec: RelationSpec<'_>) -> TokenStream {
    let field_mod = spec.field_name;
    let field_name_str = spec.field_name.to_string();
    let owner = spec.owner;
    let target = spec.target;
    let local = spec.local_key;
    let foreign = spec.foreign_key;
    let kind = match spec.kind {
        RelationKindTokens::BelongsTo => {
            quote! { ::prax_query::relations::RelationKind::BelongsTo }
        }
        RelationKindTokens::HasMany => {
            quote! { ::prax_query::relations::RelationKind::HasMany }
        }
        RelationKindTokens::HasOne => {
            quote! { ::prax_query::relations::RelationKind::HasOne }
        }
    };
    quote! {
        pub mod #field_mod {
            /// Build an [`::prax_query::relations::IncludeSpec`] for this
            /// relation. Used as `include(user::posts::fetch())`.
            pub fn fetch() -> ::prax_query::relations::IncludeSpec {
                ::prax_query::relations::IncludeSpec::new(#field_name_str)
            }

            /// Queue a nested `Connect` for feeding into
            /// `CreateOperation::with(...)`. Currently this only
            /// compiles — `NestedWriteOp::execute` returns an internal
            /// error for `Connect` until the child-PK column name is
            /// plumbed into the nested-write metadata.
            pub fn connect(
                pk: impl Into<::prax_query::filter::FilterValue>,
            ) -> ::prax_query::nested::NestedWriteOp {
                ::prax_query::nested::NestedWriteOp::Connect {
                    relation: #field_name_str.into(),
                    pk: pk.into(),
                }
            }

            /// Queue one or more nested `Create`s for feeding into
            /// `CreateOperation::with(...)`. Each child is a
            /// `Vec<(column, value)>`. The FK column + parent PK are
            /// appended automatically at execute time — callers must
            /// not include the FK themselves.
            pub fn create(
                children: ::std::vec::Vec<
                    ::std::vec::Vec<(
                        ::std::string::String,
                        ::prax_query::filter::FilterValue,
                    )>,
                >,
            ) -> ::prax_query::nested::NestedWriteOp {
                ::prax_query::nested::NestedWriteOp::Create {
                    relation: #field_name_str.into(),
                    target_table: <super::super::#target
                        as ::prax_query::traits::Model>::TABLE_NAME
                        .into(),
                    foreign_key: #foreign.into(),
                    payload: children,
                }
            }

            /// Zero-sized relation marker carrying meta via
            /// [`::prax_query::relations::RelationMeta`].
            pub struct Relation;

            impl ::prax_query::relations::RelationMeta for Relation {
                type Owner = super::super::#owner;
                type Target = super::super::#target;
                const NAME: &'static str = #field_name_str;
                const KIND: ::prax_query::relations::RelationKind = #kind;
                const LOCAL_KEY: &'static str = #local;
                const FOREIGN_KEY: &'static str = #foreign;
            }
        }
    }
}
