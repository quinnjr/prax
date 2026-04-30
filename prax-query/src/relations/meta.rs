//! Metadata describing a relation between two models.
//!
//! Every relation emitted by codegen materializes as a zero-sized type
//! implementing [`RelationMeta`]. The meta trait carries enough type-level
//! and const-level information for the runtime relation executor
//! ([`super::executor`]) to build the secondary SELECT statement for
//! an `.include()` call without reflection.
//!
//! # Kinds
//!
//! - [`RelationKind::BelongsTo`] — the owner holds a FK to the target's PK.
//!   `LOCAL_KEY` is the owner's column pointing at the target.
//! - [`RelationKind::HasMany`] — the target holds a FK to the owner's PK.
//!   `FOREIGN_KEY` is the target's column pointing back at the owner.
//! - [`RelationKind::HasOne`] — like `HasMany` but with a uniqueness
//!   constraint on the target's FK column.
//!
//! The trait itself is deliberately inert — it is consulted by the
//! executor and never implements any actual loading logic. That keeps
//! the per-model impl emitted by the derive macro trivial.

use crate::traits::Model;

/// Classification of a relation between two models.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationKind {
    /// Owner holds a foreign key to the target's primary key.
    BelongsTo,
    /// Target holds a foreign key to the owner's primary key (1-to-N).
    HasMany,
    /// Target holds a unique foreign key to the owner's primary key (1-to-1).
    HasOne,
}

/// Each relation emitted by codegen materializes as a zero-sized type
/// implementing this trait. `Owner` is the model declaring the relation;
/// `Target` is the related model. `LOCAL_KEY` is the column on `Owner`
/// that references `Target` (for `BelongsTo`); `FOREIGN_KEY` is the
/// column on `Target` that references `Owner`'s PK (for `HasMany` /
/// `HasOne`).
pub trait RelationMeta {
    /// The model declaring the relation.
    type Owner: Model;
    /// The related model.
    type Target: Model;
    /// Field name on `Owner` (also the string key of the matching
    /// [`super::IncludeSpec`]).
    const NAME: &'static str;
    /// Classification of this relation (see [`RelationKind`]).
    const KIND: RelationKind;
    /// Column on `Owner` that references `Target` (used for
    /// `BelongsTo`). For `HasMany` / `HasOne` this is conventionally
    /// `"id"`.
    const LOCAL_KEY: &'static str;
    /// Column on `Target` that references `Owner`'s PK (used for
    /// `HasMany` / `HasOne`). For `BelongsTo` this is conventionally
    /// `"id"`.
    const FOREIGN_KEY: &'static str;
}
