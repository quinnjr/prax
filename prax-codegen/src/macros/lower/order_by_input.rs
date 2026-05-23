//! Lower DSL `order_by:` (and `cursor:`) values.
//!
//! `order_by` accepts either a single block (`{ created_at: desc }`)
//! or a list of blocks (`[{ a: asc }, { b: desc }]`). Both lower to a
//! `prax_query::types::OrderBy` built up via
//! `OrderBy::from_fields(...)`. Going directly to the runtime IR
//! sidesteps the phase-2 limitation that `OrderByInput::into_ir`
//! returns a single `OrderBy` (so multiple
//! `with_order_by_input` calls would clobber each other).
//!
//! ## Aggregate fields in `order_by:`
//!
//! When a key in `order_by:` is a schema-level aggregate field, the
//! generated `OrderByField` uses the scalar-subquery SQL as the column
//! expression. The `SqlBuilder` emits it verbatim in the `ORDER BY`
//! clause — e.g. `ORDER BY (SELECT COUNT(*) ...) DESC`. This is the
//! same approach used by Prisma's generated SQL for aggregate ordering.
//!
//! `cursor` lowers to the per-model `<Model>WhereUniqueInput` enum
//! variant: the block must have exactly one key, matched against a
//! `@unique` column.

#![allow(dead_code)]

use convert_case::{Case, Casing};
use prax_schema::Model;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use super::LowerCtx;
use crate::macros::dsl::ast::{DslBlock, DslField, DslValue};

/// Lower `order_by:` value.
pub fn lower_order_by(value: &DslValue, ctx: &LowerCtx<'_>) -> syn::Result<TokenStream> {
    let blocks: Vec<&DslBlock> = match value {
        DslValue::Block(b) => vec![b],
        DslValue::List(items) => items
            .iter()
            .map(|v| match v {
                DslValue::Block(b) => Ok(b),
                _ => Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "`order_by:` list entries must be `{ field: dir }` blocks",
                )),
            })
            .collect::<syn::Result<_>>()?,
        _ => {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "`order_by:` expects a `{ ... }` block or a list of blocks",
            ));
        }
    };

    let mut fields: Vec<TokenStream> = Vec::new();
    for b in blocks {
        for entry in &b.fields {
            let DslField::Pair { key, value, .. } = entry else {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "`order_by:` does not support spread or conditional fields yet",
                ));
            };
            let key_str = key.to_string();
            let dir = match value {
                DslValue::BareIdent(id) => id.to_string(),
                DslValue::Path(p) => p
                    .segments
                    .last()
                    .map(|s| s.ident.to_string())
                    .ok_or_else(|| syn::Error::new(key.span(), "empty sort path"))?,
                _ => {
                    return Err(syn::Error::new(
                        key.span(),
                        "order_by direction must be `asc` or `desc`",
                    ));
                }
            };
            let dir_lower = dir.to_lowercase();
            if !matches!(dir_lower.as_str(), "asc" | "desc") {
                return Err(syn::Error::new(
                    key.span(),
                    format!("unknown sort direction `{dir}`; expected `asc` or `desc`"),
                ));
            }

            // Check if this is an aggregate field (order by scalar subquery).
            if let Some(model_field) = ctx.model.get_field(&key_str)
                && let Some(agg) = model_field.aggregate()
            {
                let subquery = build_aggregate_order_by_column(&key_str, &agg, key.span(), ctx)?;
                let sort_order = if dir_lower == "asc" {
                    quote! { ::prax_query::types::SortOrder::Asc }
                } else {
                    quote! { ::prax_query::types::SortOrder::Desc }
                };
                fields.push(quote! {
                    ::prax_query::types::OrderByField::new(
                        ::std::string::String::from(#subquery),
                        #sort_order,
                    )
                });
                continue;
            }

            let dir_ident = quote::format_ident!("{}", dir_lower);
            let column = lookup_column(ctx.model, &key_str).ok_or_else(|| {
                syn::Error::new(
                    key.span(),
                    format!(
                        "unknown order_by field `{}` on model `{}`",
                        key_str,
                        ctx.model.name()
                    ),
                )
            })?;
            fields.push(quote! {
                ::prax_query::types::OrderByField::#dir_ident(#column)
            });
        }
    }

    Ok(quote! {
        ::prax_query::types::OrderBy::from_fields(::std::vec![ #(#fields),* ])
    })
}

/// Lower `cursor:` value to a `<Model>WhereUniqueInput::Variant(value)`.
///
/// The phase-2 codegen emits `<Model>WhereUniqueInput` as an enum with
/// one variant per `@unique` column (PascalCase variant name, scalar
/// payload).
pub fn lower_cursor(block: &DslBlock, ctx: &LowerCtx<'_>) -> syn::Result<TokenStream> {
    if block.fields.len() != 1 {
        return Err(syn::Error::new(
            block.span,
            "cursor block must have exactly one unique-key field",
        ));
    }
    let DslField::Pair { key, value, .. } = &block.fields[0] else {
        return Err(syn::Error::new(
            block.span,
            "cursor block must be a `{ field: value }` pair",
        ));
    };
    let key_str = key.to_string();
    let field = ctx.model.get_field(&key_str).ok_or_else(|| {
        syn::Error::new(
            key.span(),
            format!("unknown cursor field `{key_str}` on `{}`", ctx.model.name()),
        )
    })?;
    if !field.is_id() && !field.is_unique() {
        return Err(syn::Error::new(
            key.span(),
            format!(
                "cursor field `{}` is not a unique column. \
                 Use a field marked `@id` or `@unique`.",
                key_str
            ),
        ));
    }
    let variant = format_ident!("{}", key_str.to_case(Case::Pascal));
    let module_ident = format_ident!("{}", ctx.model.name().to_case(Case::Snake));
    let unique_ident = format_ident!("{}WhereUniqueInput", ctx.model.name());
    let payload = match value {
        DslValue::Lit(l) => quote! { ::core::convert::Into::into(#l) },
        DslValue::Expr(e) => quote! { ::core::convert::Into::into(#e) },
        DslValue::Path(p) => quote! { #p },
        DslValue::BareIdent(b) => quote! { #b },
        _ => {
            return Err(syn::Error::new(
                key.span(),
                "cursor value must be a literal or `@(expr)`",
            ));
        }
    };
    Ok(quote! {
        #module_ident::#unique_ident::#variant(#payload)
    })
}

fn lookup_column(model: &Model, field_name: &str) -> Option<String> {
    let f = model.get_field(field_name)?;
    // Aggregate fields don't map to DB columns — they use subquery SQL.
    if f.is_aggregate() {
        return None;
    }
    let attrs = f.extract_attributes();
    Some(attrs.map.unwrap_or_else(|| f.name().to_string()))
}

/// Build the scalar-subquery SQL string for an aggregate field used in
/// `order_by:`. The SQL is emitted verbatim as the `OrderByField::column`.
fn build_aggregate_order_by_column(
    field_name: &str,
    agg: &prax_schema::AggregateAttribute,
    span: proc_macro2::Span,
    ctx: &LowerCtx<'_>,
) -> syn::Result<String> {
    use crate::macros::lower::select_input::aggregate_sql;

    let rel_name = agg.relation.as_str();
    let rel_field = ctx.model.get_field(rel_name).ok_or_else(|| {
        syn::Error::new(
            span,
            format!(
                "aggregate field `{field_name}` references relation `{rel_name}` \
                 which does not exist on model `{}`",
                ctx.model.name()
            ),
        )
    })?;

    let prax_schema::FieldType::Model(target_model_name) = &rel_field.field_type else {
        return Err(syn::Error::new(
            span,
            format!("field `{rel_name}` is not a relation"),
        ));
    };

    let target_model = ctx.schema.get_model(target_model_name).ok_or_else(|| {
        syn::Error::new(
            span,
            format!("model `{target_model_name}` not found in schema"),
        )
    })?;

    let attrs = rel_field.extract_attributes();
    let rel_attr = attrs.relation.ok_or_else(|| {
        syn::Error::new(
            span,
            format!("relation `{rel_name}` has no `@relation(fields: [...], references: [...])`"),
        )
    })?;

    if rel_attr.fields.is_empty() || rel_attr.references.is_empty() {
        return Err(syn::Error::new(
            span,
            format!("relation `{rel_name}` must declare `fields` and `references`"),
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

    Ok(aggregate_sql(
        kind,
        target_table,
        fk_column,
        parent_table,
        parent_pk,
        agg_field,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use prax_schema::parse_schema;
    use quote::quote;

    const SCHEMA: &str = include_str!("../../../tests/fixtures/schema.prax");

    fn ctx<'a>(schema: &'a prax_schema::Schema, model: &'a Model) -> LowerCtx<'a> {
        LowerCtx::new(schema, model)
    }

    #[test]
    fn lower_order_by_single_block() {
        let schema = parse_schema(SCHEMA).unwrap();
        let model = schema.get_model("User").unwrap().clone();
        let ctx = ctx(&schema, &model);
        let block = syn::parse2::<DslBlock>(quote!({ created_at: desc })).unwrap();
        let out = lower_order_by(&DslValue::Block(block), &ctx)
            .unwrap()
            .to_string();
        assert!(out.contains("OrderBy"));
        assert!(out.contains("desc"));
        assert!(out.contains("created_at"));
    }

    #[test]
    fn lower_order_by_list_of_blocks() {
        let schema = parse_schema(SCHEMA).unwrap();
        let model = schema.get_model("User").unwrap().clone();
        let ctx = ctx(&schema, &model);
        // `DslValue` doesn't impl `Parse`; construct the list manually.
        let v1 = syn::parse2::<DslBlock>(quote!({ id: asc })).unwrap();
        let v2 = syn::parse2::<DslBlock>(quote!({ email: desc })).unwrap();
        let val = DslValue::List(vec![DslValue::Block(v1), DslValue::Block(v2)]);
        let out = lower_order_by(&val, &ctx).unwrap().to_string();
        assert!(out.contains("asc"));
        assert!(out.contains("desc"));
    }

    #[test]
    fn lower_order_by_unknown_field_errors() {
        let schema = parse_schema(SCHEMA).unwrap();
        let model = schema.get_model("User").unwrap().clone();
        let ctx = ctx(&schema, &model);
        let block = syn::parse2::<DslBlock>(quote!({ nope: asc })).unwrap();
        let err = lower_order_by(&DslValue::Block(block), &ctx).unwrap_err();
        assert!(err.to_string().contains("unknown order_by field"));
    }

    #[test]
    fn lower_cursor_unique_column() {
        let schema = parse_schema(SCHEMA).unwrap();
        let model = schema.get_model("User").unwrap().clone();
        let ctx = ctx(&schema, &model);
        let block = syn::parse2::<DslBlock>(quote!({ email: "alice@x.com" })).unwrap();
        let out = lower_cursor(&block, &ctx).unwrap().to_string();
        assert!(out.contains("UserWhereUniqueInput"));
        assert!(out.contains("Email"));
    }

    #[test]
    fn lower_cursor_id_column() {
        let schema = parse_schema(SCHEMA).unwrap();
        let model = schema.get_model("User").unwrap().clone();
        let ctx = ctx(&schema, &model);
        let block = syn::parse2::<DslBlock>(quote!({ id: 42 })).unwrap();
        let out = lower_cursor(&block, &ctx).unwrap().to_string();
        assert!(out.contains("Id"));
    }

    #[test]
    fn lower_cursor_non_unique_field_errors() {
        let schema = parse_schema(SCHEMA).unwrap();
        let model = schema.get_model("User").unwrap().clone();
        let ctx = ctx(&schema, &model);
        let block = syn::parse2::<DslBlock>(quote!({ name: "x" })).unwrap();
        let err = lower_cursor(&block, &ctx).unwrap_err();
        assert!(err.to_string().contains("not a unique"));
    }

    #[test]
    fn lower_cursor_multiple_fields_errors() {
        let schema = parse_schema(SCHEMA).unwrap();
        let model = schema.get_model("User").unwrap().clone();
        let ctx = ctx(&schema, &model);
        let block = syn::parse2::<DslBlock>(quote!({ id: 1, email: "x" })).unwrap();
        let err = lower_cursor(&block, &ctx).unwrap_err();
        assert!(err.to_string().contains("exactly one"));
    }

    #[test]
    fn lower_order_by_aggregate_field_emits_subquery_column() {
        // post_count is an @count(posts) field in the fixture schema.
        let schema = parse_schema(SCHEMA).unwrap();
        let model = schema.get_model("User").unwrap().clone();
        let ctx = ctx(&schema, &model);
        let block = syn::parse2::<DslBlock>(quote!({ post_count: desc })).unwrap();
        let out = lower_order_by(&DslValue::Block(block), &ctx)
            .unwrap()
            .to_string();
        assert!(out.contains("OrderBy"), "got: {out}");
        assert!(out.contains("COUNT"), "got: {out}");
        assert!(out.contains("Desc"), "got: {out}");
        // The subquery SQL should be the column expression, not a bare column name.
        assert!(out.contains("SELECT"), "got: {out}");
    }
}
