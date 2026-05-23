//! Lower DSL `select:` blocks to the per-model `<Model>Select`
//! constructor emitted by phase 2 codegen.
//!
//! Phase 2's `<Model>Select` is a struct of `Option<bool>` — one per
//! scalar field plus one per relation. Relation fields under `select`
//! gate the relation off in the lowered IR (`Select::Fields`) the same
//! way scalar fields do. Nested per-relation args (`select: { posts:
//! { where: ... } }`) are accepted but flattened to `Some(true)`
//! until phase 5 introduces `<Relation>SelectArgs`.
//!
//! ## Computed / aggregate fields
//!
//! When a field in `select:` is a schema-level aggregate (`@count`,
//! `@sum`, `@avg`, `@min`, `@max`), the lowering emits a
//! `.with_scalar_projection(ScalarProjection::new(...))` call instead of
//! setting a `<Model>Select` slot.  The caller (ops/*.rs) must chain the
//! resulting `scalar_projections` onto the operation builder.
//!
//! ## `_count` accessor
//!
//! `select: { _count: { posts: true } }` is a shorthand that emits one
//! `ScalarProjection` per listed relation without requiring a named
//! aggregate field in the schema.  Only schema-defined models are
//! supported; derive-style models can use `.with_scalar_projection`
//! directly via the runtime API (follow-up task).

#![allow(dead_code)]

use convert_case::{Case, Casing};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use super::LowerCtx;
use crate::macros::dsl::ast::{DslBlock, DslField, DslValue};

/// Lowered output of a `select:` block.
///
/// Most callers only need the `select_struct` token stream.  When the
/// block references aggregate fields or the `_count` accessor, the
/// extra `scalar_projections` must be chained onto the operation
/// builder via `.with_scalar_projection(...)`.
pub struct SelectLowering {
    /// Token stream constructing `<Model>Select`.
    pub select_struct: TokenStream,
    /// Zero or more `.with_scalar_projection(...)` call token streams.
    pub scalar_projections: Vec<TokenStream>,
}

impl std::fmt::Debug for SelectLowering {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SelectLowering")
            .field("select_struct", &self.select_struct.to_string())
            .field("scalar_projections_len", &self.scalar_projections.len())
            .finish()
    }
}

/// Convenience wrapper: lower a `select:` block and return only the
/// `<Model>Select` constructor token stream, discarding any scalar
/// projections for aggregate fields.
///
/// For read-path ops (`find_many!`, `find_first!`, `find_unique!`) use
/// [`lower_select`] directly and chain the
/// [`SelectLowering::scalar_projections`] as `.with_scalar_projection(...)`
/// calls. For write-path ops and contexts where aggregate projections are
/// not applicable, this simpler form is fine.
pub fn lower_select_struct_only(block: &DslBlock, ctx: &LowerCtx<'_>) -> syn::Result<TokenStream> {
    lower_select(block, ctx).map(|l| l.select_struct)
}

pub fn lower_select(block: &DslBlock, ctx: &LowerCtx<'_>) -> syn::Result<SelectLowering> {
    let module_ident = format_ident!("{}", ctx.model.name().to_case(Case::Snake));
    let select_ident = format_ident!("{}Select", ctx.model.name());

    let mut setters: Vec<TokenStream> = Vec::new();
    let mut scalar_projections: Vec<TokenStream> = Vec::new();

    for field in &block.fields {
        let DslField::Pair { key, value, .. } = field else {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "`select` does not support spread or conditional fields yet",
            ));
        };
        let key_str = key.to_string();

        // Special `_count: { rel: true }` accessor.
        if key_str == "_count" {
            let projs = lower_count_accessor(value, key.span(), ctx)?;
            scalar_projections.extend(projs);
            continue;
        }

        let target = ctx.model.get_field(&key_str).ok_or_else(|| {
            let candidates: Vec<String> = ctx.model.fields.keys().map(|k| k.to_string()).collect();
            let suggestion = crate::macros::validate::suggest(&key_str, &candidates);
            let msg = match suggestion {
                Some(c) => format!(
                    "unknown field `{}` on model `{}` in select block. did you mean `{}`?",
                    key_str,
                    ctx.model.name(),
                    c
                ),
                None => format!(
                    "unknown field `{}` on model `{}` in select block",
                    key_str,
                    ctx.model.name()
                ),
            };
            syn::Error::new(key.span(), msg)
        })?;

        // Aggregate (virtual) fields lower to scalar projections, not Select slots.
        if let Some(agg) = target.aggregate() {
            let proj_ts = lower_aggregate_field_projection(target.name(), &agg, key.span(), ctx)?;
            scalar_projections.push(proj_ts);
            continue;
        }

        let assign_ident = format_ident!("{}", target.name().to_case(Case::Snake));
        let bool_expr = match value {
            DslValue::Bool(b) => quote! { #b },
            DslValue::Block(_) => quote! { true },
            _ => {
                return Err(syn::Error::new(
                    key.span(),
                    format!(
                        "select value for `{}` must be `true`, `false`, or a `{{ ... }}` block",
                        key_str
                    ),
                ));
            }
        };
        setters.push(quote! {
            __s.#assign_ident = ::core::option::Option::Some(#bool_expr);
        });
    }

    let select_struct = quote! {
        {
            let mut __s: #module_ident::#select_ident =
                <#module_ident::#select_ident as ::core::default::Default>::default();
            #(#setters)*
            __s
        }
    };

    Ok(SelectLowering {
        select_struct,
        scalar_projections,
    })
}

/// Lower `_count: { rel: true, ... }` to one `with_scalar_projection` call
/// per listed relation.
fn lower_count_accessor(
    value: &DslValue,
    span: proc_macro2::Span,
    ctx: &LowerCtx<'_>,
) -> syn::Result<Vec<TokenStream>> {
    let DslValue::Block(block) = value else {
        return Err(syn::Error::new(
            span,
            "`_count` expects a `{ rel: true }` block listing relations to count",
        ));
    };

    // Collect the names of outgoing relation fields on this model.
    let relation_names: Vec<String> = ctx
        .model
        .fields
        .values()
        .filter(|f| f.is_relation() && f.modifier.is_list())
        .map(|f| f.name().to_string())
        .collect();

    if relation_names.is_empty() {
        return Err(syn::Error::new(
            span,
            format!(
                "model `{}` has no outgoing to-many relations to count",
                ctx.model.name()
            ),
        ));
    }

    let mut projections: Vec<TokenStream> = Vec::new();
    for entry in &block.fields {
        let DslField::Pair { key, value, .. } = entry else {
            return Err(syn::Error::new(
                span,
                "`_count` block does not support spread or conditional fields",
            ));
        };
        let rel_name = key.to_string();

        // Only `true` is valid.
        if !matches!(value, DslValue::Bool(true)) {
            return Err(syn::Error::new(
                key.span(),
                format!(
                    "`_count.{}` must be `true` (false is not meaningful for count projections)",
                    rel_name
                ),
            ));
        }

        // Validate the relation name; emit a did-you-mean if close.
        let rel_field = ctx.model.get_field(&rel_name).filter(|f| f.is_relation());
        if rel_field.is_none() {
            let suggestion = crate::macros::validate::suggest(&rel_name, &relation_names);
            let msg = match suggestion {
                Some(c) => format!(
                    "unknown relation `{}` on model `{}` in `_count`. did you mean `{}`?",
                    rel_name,
                    ctx.model.name(),
                    c
                ),
                None => format!(
                    "unknown relation `{}` on model `{}` in `_count`. \
                     Known to-many relations: {:?}",
                    rel_name,
                    ctx.model.name(),
                    relation_names
                ),
            };
            return Err(syn::Error::new(key.span(), msg));
        }
        let rel_field = rel_field.unwrap();

        let proj = build_count_projection_for_relation(rel_field, &rel_name, key.span(), ctx)?;
        projections.push(proj);
    }

    Ok(projections)
}

/// Build the `with_scalar_projection(...)` token stream for a single
/// `_count.<rel>: true` entry.
fn build_count_projection_for_relation(
    rel_field: &prax_schema::Field,
    rel_name: &str,
    span: proc_macro2::Span,
    ctx: &LowerCtx<'_>,
) -> syn::Result<TokenStream> {
    let prax_schema::FieldType::Model(target_model_name) = &rel_field.field_type else {
        return Err(syn::Error::new(
            span,
            format!("field `{}` is not a relation field", rel_name),
        ));
    };

    let target_model = ctx.schema.get_model(target_model_name).ok_or_else(|| {
        syn::Error::new(
            span,
            format!(
                "relation `{}` references model `{}` which is not in the schema",
                rel_name, target_model_name
            ),
        )
    })?;

    let attrs = rel_field.extract_attributes();
    let rel_attr = attrs.relation.ok_or_else(|| {
        syn::Error::new(
            span,
            format!(
                "relation `{}` has no `@relation(fields: [...], references: [...])` attribute",
                rel_name
            ),
        )
    })?;

    if rel_attr.fields.is_empty() || rel_attr.references.is_empty() {
        return Err(syn::Error::new(
            span,
            format!(
                "relation `{}` must declare `fields` and `references` in `@relation(...)`",
                rel_name
            ),
        ));
    }

    let parent_table = ctx.model.table_name();
    let parent_pk = rel_attr.fields[0].as_str();
    let target_table = target_model.table_name();
    let fk_column = rel_attr.references[0].as_str();

    let sql = aggregate_sql(
        "count",
        target_table,
        fk_column,
        parent_table,
        parent_pk,
        None,
    );
    // Alias: _count_<rel>
    let alias = format!("_count_{}", rel_name);

    Ok(quote! {
        .with_scalar_projection(::prax_query::ScalarProjection::new(
            ::std::borrow::Cow::Owned(#sql.to_string()),
            ::std::vec![],
            ::std::boxed::Box::leak(#alias.into_boxed_str()),
        ))
    })
}

/// Lower a schema-defined aggregate field (`@count`, `@sum`, etc.) in a
/// `select:` block to a `with_scalar_projection(...)` token stream.
fn lower_aggregate_field_projection(
    field_name: &str,
    agg: &prax_schema::AggregateAttribute,
    span: proc_macro2::Span,
    ctx: &LowerCtx<'_>,
) -> syn::Result<TokenStream> {
    let rel_name = agg.relation.as_str();
    let rel_field = ctx.model.get_field(rel_name).ok_or_else(|| {
        syn::Error::new(
            span,
            format!(
                "aggregate field `{}` references relation `{}` which does not exist on model `{}`",
                field_name,
                rel_name,
                ctx.model.name()
            ),
        )
    })?;

    let prax_schema::FieldType::Model(target_model_name) = &rel_field.field_type else {
        return Err(syn::Error::new(
            span,
            format!(
                "aggregate field `{}` references `{}` which is not a relation field",
                field_name, rel_name
            ),
        ));
    };

    let target_model = ctx.schema.get_model(target_model_name).ok_or_else(|| {
        syn::Error::new(
            span,
            format!(
                "relation `{}` references model `{}` which is not in the schema",
                rel_name, target_model_name
            ),
        )
    })?;

    let attrs = rel_field.extract_attributes();
    let rel_attr = attrs.relation.ok_or_else(|| {
        syn::Error::new(
            span,
            format!(
                "relation `{}` has no `@relation(fields: [...], references: [...])` attribute",
                rel_name
            ),
        )
    })?;

    if rel_attr.fields.is_empty() || rel_attr.references.is_empty() {
        return Err(syn::Error::new(
            span,
            format!(
                "relation `{}` must declare `fields` and `references` in `@relation(...)`",
                rel_name
            ),
        ));
    }

    let parent_table = ctx.model.table_name();
    let parent_pk = rel_attr.fields[0].as_str();
    let target_table = target_model.table_name();
    let fk_column = rel_attr.references[0].as_str();

    let kind = match agg.kind {
        prax_schema::AggregateKind::Count => "count",
        prax_schema::AggregateKind::Sum => "sum",
        prax_schema::AggregateKind::Avg => "avg",
        prax_schema::AggregateKind::Min => "min",
        prax_schema::AggregateKind::Max => "max",
    };
    let agg_field = agg.field.as_deref();
    let sql = aggregate_sql(
        kind,
        target_table,
        fk_column,
        parent_table,
        parent_pk,
        agg_field,
    );

    Ok(quote! {
        .with_scalar_projection(::prax_query::ScalarProjection::new(
            ::std::borrow::Cow::Owned(#sql.to_string()),
            ::std::vec![],
            ::std::boxed::Box::leak(#field_name.to_string().into_boxed_str()),
        ))
    })
}

/// Build the scalar-subquery SQL string for an aggregate projection.
///
/// # Examples
///
/// ```text
/// aggregate_sql("count", "posts", "author_id", "users", "id", None)
/// // → (SELECT COUNT(*) FROM "posts" WHERE "posts"."author_id" = "users"."id")
///
/// aggregate_sql("sum", "posts", "author_id", "users", "id", Some("views"))
/// // → (SELECT SUM("posts"."views") FROM "posts" WHERE "posts"."author_id" = "users"."id")
/// ```
pub fn aggregate_sql(
    kind: &str,
    table: &str,
    foreign_key: &str,
    parent_table: &str,
    parent_pk: &str,
    agg_field: Option<&str>,
) -> String {
    let agg_expr = match (kind, agg_field) {
        ("count", _) => "COUNT(*)".to_string(),
        (k, Some(f)) => format!(r#"{}("{}"."{}""#, k.to_uppercase(), table, f) + ")",
        _ => unreachable!("non-count aggregate requires a field name; validator enforces this"),
    };
    format!(
        r#"(SELECT {} FROM "{}" WHERE "{}"."{}" = "{}"."{}")"#,
        agg_expr, table, table, foreign_key, parent_table, parent_pk
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use prax_schema::parse_schema;
    use quote::quote;

    const SCHEMA: &str = include_str!("../../../tests/fixtures/schema.prax");

    fn lower(model_name: &str, tokens: TokenStream) -> SelectLowering {
        let schema = parse_schema(SCHEMA).unwrap();
        let model = schema.get_model(model_name).unwrap().clone();
        let ctx = LowerCtx::new(&schema, &model);
        let block = syn::parse2::<DslBlock>(tokens).unwrap();
        lower_select(&block, &ctx).unwrap()
    }

    fn lower_err(model_name: &str, tokens: TokenStream) -> syn::Error {
        let schema = parse_schema(SCHEMA).unwrap();
        let model = schema.get_model(model_name).unwrap().clone();
        let ctx = LowerCtx::new(&schema, &model);
        let block = syn::parse2::<DslBlock>(tokens).unwrap();
        lower_select(&block, &ctx).unwrap_err()
    }

    #[test]
    fn lower_select_mixed_scalar_and_relation() {
        let out = lower("User", quote!({ id: true, email: true, profile: true }));
        let s = out.select_struct.to_string();
        assert!(s.contains("UserSelect"));
        assert!(s.contains("id"));
        assert!(s.contains("email"));
        assert!(s.contains("profile"));
        assert!(out.scalar_projections.is_empty());
    }

    #[test]
    fn lower_select_unknown_field_errors() {
        let err = lower_err("User", quote!({ nope: true }));
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn lower_select_unknown_field_errors_with_suggestion() {
        let err = lower_err("User", quote!({ emial: true }));
        let msg = err.to_string();
        assert!(msg.contains("unknown field"), "got: {msg}");
        assert!(msg.contains("did you mean"), "got: {msg}");
        assert!(msg.contains("email"), "got: {msg}");
    }

    #[test]
    fn lower_select_count_accessor_emits_scalar_projection() {
        let out = lower("User", quote!({ id: true, _count: { posts: true } }));
        let s = out.select_struct.to_string();
        assert!(
            s.contains("UserSelect"),
            "select_struct missing UserSelect: {s}"
        );
        assert_eq!(out.scalar_projections.len(), 1);
        let proj = out.scalar_projections[0].to_string();
        assert!(proj.contains("with_scalar_projection"), "got: {proj}");
        assert!(proj.contains("ScalarProjection"), "got: {proj}");
        assert!(proj.contains("_count_posts"), "got: {proj}");
        assert!(proj.contains("COUNT"), "got: {proj}");
    }

    #[test]
    fn lower_select_count_accessor_unknown_relation_errors() {
        let err = lower_err("User", quote!({ _count: { nonexistent: true } }));
        let msg = err.to_string();
        assert!(msg.contains("unknown relation"), "got: {msg}");
    }

    #[test]
    fn lower_select_aggregate_field_emits_scalar_projection() {
        // post_count is an @count(posts) field in the fixture schema.
        let out = lower("User", quote!({ post_count: true }));
        // The aggregate field itself does NOT become a Select slot.
        let s = out.select_struct.to_string();
        assert!(
            !s.contains("post_count"),
            "aggregate field should not be in Select struct, got: {s}"
        );
        assert_eq!(
            out.scalar_projections.len(),
            1,
            "expected one scalar projection"
        );
        let proj = out.scalar_projections[0].to_string();
        assert!(proj.contains("with_scalar_projection"), "got: {proj}");
        assert!(proj.contains("COUNT"), "got: {proj}");
        assert!(proj.contains("post_count"), "got: {proj}");
    }

    #[test]
    fn aggregate_sql_count() {
        let sql = aggregate_sql("count", "posts", "author_id", "users", "id", None);
        assert_eq!(
            sql,
            r#"(SELECT COUNT(*) FROM "posts" WHERE "posts"."author_id" = "users"."id")"#
        );
    }

    #[test]
    fn aggregate_sql_sum() {
        let sql = aggregate_sql("sum", "posts", "author_id", "users", "id", Some("views"));
        assert_eq!(
            sql,
            r#"(SELECT SUM("posts"."views") FROM "posts" WHERE "posts"."author_id" = "users"."id")"#
        );
    }

    #[test]
    fn aggregate_sql_avg() {
        let sql = aggregate_sql("avg", "posts", "author_id", "users", "id", Some("score"));
        assert_eq!(
            sql,
            r#"(SELECT AVG("posts"."score") FROM "posts" WHERE "posts"."author_id" = "users"."id")"#
        );
    }
}
