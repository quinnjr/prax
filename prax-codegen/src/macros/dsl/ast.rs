//! AST for the read-operation DSL brace block.
//!
//! See `docs/superpowers/specs/2026-05-18-typed-query-traits-design.md`
//! §4 for the grammar this AST implements.

// Several fields are consumed by the lowering pass landing in tasks
// 7-11; until then dead_code warnings would block CI.
#![allow(dead_code)]

use proc_macro2::Span;
use syn::{Expr, Ident, Lit, Path};

/// A `{ ... }` brace block: the body of an operation input or a nested
/// shape (`where: { ... }`, `posts: { where: ... }`, etc.).
#[derive(Debug, Clone)]
pub struct DslBlock {
    /// Span of the opening `{` for diagnostics.
    pub span: Span,
    /// Fields appearing inside the braces, in source order.
    pub fields: Vec<DslField>,
}

/// One entry inside a [`DslBlock`].
#[derive(Debug, Clone)]
pub enum DslField {
    /// `ident: value` — the common case.
    Pair {
        /// The field name on the left of `:`.
        key: Ident,
        /// The value on the right of `:`.
        value: DslValue,
        /// Span of the key for diagnostics.
        span: Span,
    },
    /// `..expr` or `..move expr` — Rust struct-update style.
    Spread {
        /// Right-hand expression.
        expr: Expr,
        /// True for `..move expr` (no clone); false for plain `..expr`.
        by_move: bool,
        /// Span of the leading `..`.
        span: Span,
    },
    /// `#[if(cond)] ident: value`, `#[else_if(cond)] ident: value`,
    /// `#[else] ident: value`.
    Conditional {
        /// The condition expression. For `#[else]`, this is unused;
        /// the parser stores a `syn::parse_quote!(true)`.
        cond: Expr,
        /// Which conditional kind this is.
        kind: CondKind,
        /// The key (right of the conditional attribute).
        key: Ident,
        /// The value.
        value: DslValue,
        /// Span of the conditional attribute.
        span: Span,
    },
}

/// Discriminator for the three conditional attribute forms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CondKind {
    /// `#[if(cond)]`
    If,
    /// `#[else_if(cond)]`
    ElseIf,
    /// `#[else]`
    Else,
}

/// The RHS of a `key: value` pair (or one element of a list `[...]`).
#[derive(Debug, Clone)]
pub enum DslValue {
    /// A literal — string, int, float, bool literal, etc.
    Lit(Lit),
    /// A path with `::` separators — `Role::Admin`, `crate::foo`.
    Path(Path),
    /// `@(...)` Rust escape, or a fallback expression.
    Expr(Expr),
    /// `{ ... }` nested block — another shape.
    Block(DslBlock),
    /// `[ ... ]` list — values for `in_list`, `and`/`or`, `order_by`.
    List(Vec<DslValue>),
    /// `true` / `false` keyword shorthand (for `include`/`select`).
    Bool(bool),
    /// A bare identifier at end-of-stream-or-comma — enum shorthand
    /// like `role: Admin`.
    BareIdent(Ident),
}
