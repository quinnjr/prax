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
            let dir_ident = match dir.to_lowercase().as_str() {
                "asc" => quote::format_ident!("asc"),
                "desc" => quote::format_ident!("desc"),
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("unknown sort direction `{other}`; expected `asc` or `desc`"),
                    ));
                }
            };
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
    let attrs = f.extract_attributes();
    Some(attrs.map.unwrap_or_else(|| f.name().to_string()))
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
}
