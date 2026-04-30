//! Emit `impl ModelRelationLoader<E>` for a derived model.
//!
//! The ORM's `.include()` pipeline routes every relation-include
//! request through this impl. Models with no declared relations still
//! get an impl that errors out on any unknown relation name — that
//! keeps the `ModelRelationLoader` bound uniform across every derived
//! model and lets the find-operation builders require it
//! unconditionally on `exec`.
//!
//! Each arm dispatches to
//! [`::prax_query::relations::executor::load_has_many`] for the
//! relation's marker type (emitted by
//! [`super::relation_accessors`]), then splices the bucketed children
//! onto the parent slice using the parent's PK as the lookup key.
//!
//! The child-clone step requires `Target: Clone`. That's documented in
//! the caller-facing commit body and tests and enforced by rustc at
//! the `p.#fname = children.clone();` line — if a model forgets to
//! derive `Clone`, compilation fails with a clear error instead of
//! silently producing moved-out rows.

use convert_case::{Case, Casing};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Ident;

/// Per-relation inputs for [`emit`].
pub struct LoaderRelation<'a> {
    /// Field name on the owning model (also the IncludeSpec key).
    pub field_name: &'a Ident,
    /// Target model type. Used to name the generic for `load_has_many`.
    pub target: &'a Ident,
    /// Classification of the relation.
    pub kind: LoaderKind,
}

/// Kind of relation to load — currently only `HasMany`/`HasOne` share
/// the same child-bucketing executor path. `BelongsTo` would need a
/// different executor entry point (fetch parents keyed by FK → PK);
/// it's punted to a follow-up until there's a caller for it.
pub enum LoaderKind {
    /// Target holds the FK back to Owner's PK.
    HasMany,
    /// Same as `HasMany` but bucket has at most one child per parent —
    /// for now the arm still produces a `Vec<Target>`, leaving the
    /// field type (`Option<Target>` vs `Vec<Target>`) to the caller
    /// to handle upstream.
    HasOne,
}

/// Emit the `ModelRelationLoader<E>` impl for `model_name`.
///
/// `rels` may be empty — the emitter produces a valid impl that always
/// errors, preserving the "every derived model implements
/// `ModelRelationLoader`" invariant the find-operation builders rely
/// on.
pub fn emit(model_name: &Ident, rels: &[LoaderRelation<'_>]) -> TokenStream {
    // The inner arms reference `super::#field_name::Relation` — the
    // relation marker emitted by `relation_accessors::emit` is nested
    // inside the per-model module. We need the module name (snake_cased
    // model name) so we can point `super::super::<module>::<field>::Relation`
    // from the trait impl at the crate root.
    let module_name = format_ident!("{}", model_name.to_string().to_case(Case::Snake));

    let arms = rels.iter().map(|r| {
        let fname = r.field_name;
        let fname_str = r.field_name.to_string();
        let target = r.target;
        match r.kind {
            LoaderKind::HasMany | LoaderKind::HasOne => quote! {
                #fname_str => {
                    let bucketed = ::prax_query::relations::executor::load_has_many::<
                        E,
                        #model_name,
                        #target,
                        #module_name::#fname::Relation,
                    >(engine, parents).await?;
                    for p in parents.iter_mut() {
                        use ::prax_query::traits::ModelWithPk as _;
                        let key = ::prax_query::relations::executor::filter_value_key_public(&p.pk_value());
                        if let Some(children) = bucketed.get(&key) {
                            p.#fname = children.clone();
                        }
                    }
                    Ok(())
                }
            },
        }
    });

    quote! {
        impl<E: ::prax_query::traits::QueryEngine>
            ::prax_query::traits::ModelRelationLoader<E> for #model_name
        {
            fn load_relation<'a>(
                engine: &'a E,
                parents: &'a mut [Self],
                spec: &'a ::prax_query::relations::IncludeSpec,
            ) -> ::prax_query::traits::BoxFuture<'a, ::prax_query::error::QueryResult<()>>
            {
                Box::pin(async move {
                    match spec.relation_name.as_str() {
                        #(#arms)*
                        _ => Err(::prax_query::error::QueryError::internal(format!(
                            "unknown relation '{}' on {}",
                            spec.relation_name,
                            stringify!(#model_name),
                        ))),
                    }
                })
            }
        }
    }
}
