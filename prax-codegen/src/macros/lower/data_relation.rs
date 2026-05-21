//! Lower a relation key inside `data:` (on the **create** path) to a
//! [`prax_query::nested::NestedWriteOp`] expression.
//!
//! Phase 5b recognises two operator keys inside a relation block:
//!
//! - `create:` → `[{ ... }, ...]` — lowers each child block to a
//!   `Vec<(column, value)>` payload and emits
//!   [`NestedWriteOp::Create`] tokens.
//! - `connect:` → `[{ id: <pk> }, ...]` — lowers each child block to
//!   the PK value and emits [`NestedWriteOp::Connect`] tokens.
//!
//! All other operator keys (`update`, `upsert`, `delete`,
//! `delete_many`, `disconnect`, `set`, `connect_or_create`) return a
//! "not yet supported" deferral diagnostic. Unknown operators get a
//! did-you-mean against `[create, connect]`.
//!
//! Why this builds `NestedWriteOp` tokens inline rather than generating
//! intermediate `<Without>` / `<CreateNestedInput>` structs: the only
//! consumer is the `create!` macro, so an extra round of codegen would
//! pay an upfront type-explosion cost for no caller benefit. When write
//! operators land on `update!` / `upsert!` they'll share the same DSL
//! pipeline and can reuse this lowering.

use convert_case::{Case, Casing};
use prax_schema::ast::{Field, FieldType};
use prax_schema::{Model, ScalarType};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};

use super::LowerCtx;
use crate::macros::dsl::ast::{DslBlock, DslField, DslValue};

/// One nested-write op emitted from a relation block in `data:`.
///
/// The macro consumer appends these as chained `.with(<expr>)` calls
/// on the `CreateOperation`.
#[derive(Debug)]
pub struct NestedRelationOp {
    /// Token stream evaluating to a `::prax_query::nested::NestedWriteOp`.
    pub op_expr: TokenStream,
}

/// Lower `<relation>: { create: [...], connect: [...] }` on the
/// create path.
///
/// Returns one or more [`NestedRelationOp`]s; the macro emits a
/// `.with(<expr>)` chain for each.
pub fn lower_create_relation(
    relation_field: &Field,
    value: &DslValue,
    parent_span: Span,
    ctx: &LowerCtx<'_>,
) -> syn::Result<Vec<NestedRelationOp>> {
    let DslValue::Block(block) = value else {
        return Err(syn::Error::new(
            parent_span,
            format!(
                "relation `{}` on `data:` expects a `{{ ... }}` block of nested-write operators (create:, connect:)",
                relation_field.name(),
            ),
        ));
    };

    // Resolve the target model and the FK column on it.
    let target_name = match &relation_field.field_type {
        FieldType::Model(n) => n.as_str(),
        _ => {
            return Err(syn::Error::new(
                parent_span,
                format!("field `{}` is not a relation field", relation_field.name(),),
            ));
        }
    };
    let target_model = ctx.schema.get_model(target_name).ok_or_else(|| {
        syn::Error::new(
            parent_span,
            format!(
                "relation `{}` references unknown target model `{}`",
                relation_field.name(),
                target_name
            ),
        )
    })?;

    let foreign_key = resolve_foreign_key(relation_field, ctx.model, target_model)?;

    let target_module_ident = format_ident!("{}", target_name.to_case(Case::Snake));
    let target_model_ident = format_ident!("{}", target_name);
    let target_input_ident = format_ident!("{}CreateInput", target_name);
    let target_table = target_model.table_name().to_string();
    // First PK column on the target. Composite-PK targets default to
    // their first declared `@id` column here — the plan's connect
    // executor only takes a single FilterValue, so composite-PK
    // connects are deferred until a later phase.
    let target_pk_column = target_model
        .id_fields()
        .into_iter()
        .next()
        .map(column_name_of)
        .ok_or_else(|| {
            syn::Error::new(
                parent_span,
                format!(
                    "target model `{target_name}` of relation `{}` has no `@id` column — \
                     nested connect/create requires a single-column primary key in phase 5b",
                    relation_field.name(),
                ),
            )
        })?;

    let relation_name_str = relation_field.name().to_string();

    let mut ops: Vec<NestedRelationOp> = Vec::new();

    for entry in &block.fields {
        let DslField::Pair { key, value, .. } = entry else {
            return Err(syn::Error::new(
                Span::call_site(),
                format!(
                    "nested relation block on `{}` does not support spread or conditional fields in phase 5b",
                    relation_field.name(),
                ),
            ));
        };

        let op_key = key.to_string();
        match op_key.as_str() {
            "create" => {
                let children = expect_list_of_blocks(value, &op_key, key.span())?;
                let target_ctx = ctx.for_model(target_model);
                let mut child_payloads: Vec<TokenStream> = Vec::with_capacity(children.len());
                for child_block in children {
                    // Lower the child block as a CreateInput of the target.
                    let input_expr =
                        super::data_input::lower_create_data(child_block, &target_ctx)?;
                    // Convert the CreateInput → Vec<(String, FilterValue)>
                    // via its `into_ir()` (CreatePayload).
                    child_payloads.push(quote! {
                        <#target_module_ident::#target_input_ident
                            as ::prax_query::inputs::CreateInput>::into_ir(#input_expr)
                    });
                }
                let op_expr = quote! {
                    ::prax_query::nested::NestedWriteOp::Create {
                        relation: #relation_name_str,
                        target_table: #target_table,
                        foreign_key: #foreign_key,
                        payload: ::std::vec![ #( #child_payloads ),* ],
                    }
                };
                ops.push(NestedRelationOp { op_expr });
                let _ = &target_model_ident;
            }
            "connect" => {
                let children = expect_list_of_blocks(value, &op_key, key.span())?;
                for child_block in children {
                    let pk_expr = lower_connect_pk(child_block, target_model, &target_pk_column)?;
                    let op_expr = quote! {
                        ::prax_query::nested::NestedWriteOp::Connect {
                            relation: #relation_name_str,
                            target_table: #target_table,
                            foreign_key: #foreign_key,
                            target_pk: #target_pk_column,
                            pk: ::core::convert::Into::<
                                ::prax_query::filter::FilterValue
                            >::into(#pk_expr),
                        }
                    };
                    ops.push(NestedRelationOp { op_expr });
                }
            }
            "update" | "update_many" | "upsert" | "delete" | "delete_many" | "disconnect"
            | "set" => {
                return Err(phase_5c_deferral(
                    &op_key,
                    relation_field.name(),
                    key.span(),
                ));
            }
            "connect_or_create" => {
                return Err(syn::Error::new(
                    key.span(),
                    format!(
                        "nested operator `connect_or_create` inside `data:` relation block \
                         `{}` is not supported in phase 5b. `connect_or_create` (engine-specific \
                         lowerings) lands in phase 5d.",
                        relation_field.name(),
                    ),
                ));
            }
            _ => {
                let candidates = vec!["create".to_string(), "connect".to_string()];
                let suggestion = crate::macros::validate::suggest(&op_key, &candidates);
                let msg = match suggestion {
                    Some(s) => format!(
                        "unknown nested operator `{op_key}` inside `data:` relation block `{}`. \
                         Did you mean `{s}`? Valid operators in phase 5b: create, connect.",
                        relation_field.name(),
                    ),
                    None => format!(
                        "unknown nested operator `{op_key}` inside `data:` relation block `{}`. \
                         Valid operators in phase 5b: create, connect.",
                        relation_field.name(),
                    ),
                };
                return Err(syn::Error::new(key.span(), msg));
            }
        }
    }

    Ok(ops)
}

fn phase_5c_deferral(op: &str, relation: &str, span: Span) -> syn::Error {
    syn::Error::new(
        span,
        format!(
            "nested operator `{op}` inside `data:` relation block `{relation}` is not \
             supported in phase 5b. Update/upsert/delete operators on relations land in phase 5c."
        ),
    )
}

fn expect_list_of_blocks<'a>(
    value: &'a DslValue,
    op_name: &str,
    span: Span,
) -> syn::Result<Vec<&'a DslBlock>> {
    match value {
        DslValue::List(items) => {
            let mut blocks: Vec<&DslBlock> = Vec::with_capacity(items.len());
            for item in items {
                let DslValue::Block(b) = item else {
                    return Err(syn::Error::new(
                        span,
                        format!(
                            "`{op_name}:` expects a list of `{{ ... }}` blocks; \
                             got a non-block list entry"
                        ),
                    ));
                };
                blocks.push(b);
            }
            Ok(blocks)
        }
        // Allow a single block as shorthand for `[{ ... }]` — Prisma accepts
        // both forms for HasOne relations.
        DslValue::Block(b) => Ok(vec![b]),
        _ => Err(syn::Error::new(
            span,
            format!(
                "`{op_name}:` expects a list of `{{ ... }}` blocks, e.g. \
                 `{op_name}: [{{ ... }}, {{ ... }}]`"
            ),
        )),
    }
}

/// Lower a `WhereUnique`-style block on the connect path to the PK
/// value expression.
///
/// Phase 5b accepts only single-column PK targets, and only the PK
/// column's name as the lookup key. Multi-column PKs and other
/// `@unique` columns are deferred.
fn lower_connect_pk(
    block: &DslBlock,
    target_model: &Model,
    target_pk_col: &str,
) -> syn::Result<TokenStream> {
    if block.fields.len() != 1 {
        return Err(syn::Error::new(
            Span::call_site(),
            format!(
                "phase 5b `connect:` expects exactly one key (`{target_pk_col}`) on target \
                 model `{}`. Multi-key connect targets are deferred.",
                target_model.name()
            ),
        ));
    }
    let DslField::Pair { key, value, .. } = &block.fields[0] else {
        return Err(syn::Error::new(
            Span::call_site(),
            "`connect:` block does not support spread or conditional fields in phase 5b",
        ));
    };

    let key_str = key.to_string();
    // Look up the field on the target model whose column matches the
    // declared key. We accept either the Rust field name or the
    // remapped column name (matches the `WhereUniqueInput` lookup).
    let field = target_model
        .get_field(&key_str)
        .or_else(|| {
            target_model
                .fields
                .values()
                .find(|f| column_name_of(f) == key_str)
        })
        .ok_or_else(|| {
            syn::Error::new(
                key.span(),
                format!(
                    "unknown field `{key_str}` on connect target `{}`",
                    target_model.name()
                ),
            )
        })?;

    let column = column_name_of(field);
    if column != target_pk_col {
        return Err(syn::Error::new(
            key.span(),
            format!(
                "phase 5b `connect:` only accepts the primary key column `{target_pk_col}`. \
                 Got `{column}`. Other `@unique` keys are deferred."
            ),
        ));
    }

    // Lower the value to an expression coerce-able to `FilterValue`.
    match value {
        DslValue::Lit(lit) => Ok(quote! { (#lit) }),
        DslValue::Bool(b) => Ok(quote! { #b }),
        DslValue::Expr(e) => Ok(quote! { (#e) }),
        DslValue::Path(p) => Ok(quote! { #p }),
        DslValue::BareIdent(id) => Err(syn::Error::new(
            id.span(),
            format!(
                "bare identifier `{id}` is not a valid PK value for `connect:`. \
                 Use a literal, path, or `@(expr)` escape."
            ),
        )),
        DslValue::Block(_) | DslValue::List(_) => Err(syn::Error::new(
            key.span(),
            "`connect:` PK value must be a literal, path, or `@(expr)` escape — \
             not a block or list",
        )),
    }
}

/// Resolve the FK column on `target_model` that points back at `parent_model`.
///
/// Looks for the back-pointer field on the target whose `field_type`
/// is `Model(parent_model.name)` and that carries a non-empty
/// `relation.fields = [..]` attribute list — the first entry is the
/// FK column name on the target table.
fn resolve_foreign_key(
    relation_field: &Field,
    parent_model: &Model,
    target_model: &Model,
) -> syn::Result<String> {
    // First, try to read the FK from the `references:` clause on the
    // relation_field itself (the inverse side commonly carries this).
    let attrs = relation_field.extract_attributes();
    if let Some(rel) = &attrs.relation
        && let Some(first) = rel.references.first()
    {
        return Ok(first.to_string());
    }

    // Otherwise, walk the target model for the back-pointer.
    for f in target_model.fields.values() {
        let FieldType::Model(target_pointer) = &f.field_type else {
            continue;
        };
        if target_pointer.as_str() != parent_model.name() {
            continue;
        }
        let target_attrs = f.extract_attributes();
        if let Some(rel) = target_attrs.relation
            && let Some(first) = rel.fields.first()
        {
            return Ok(first.to_string());
        }
    }

    Err(syn::Error::new(
        Span::call_site(),
        format!(
            "cannot resolve FK column on target model `{}` for relation `{}` on `{}`. \
             Phase 5b requires either `@relation(references: [<col>])` on the inverse side \
             or `@relation(fields: [<col>])` on the back-pointer field of `{}`.",
            target_model.name(),
            relation_field.name(),
            parent_model.name(),
            target_model.name(),
        ),
    ))
}

/// Get the SQL column name for a field — honors `@map("col")`.
fn column_name_of(field: &Field) -> String {
    let attrs = field.extract_attributes();
    attrs.map.unwrap_or_else(|| field.name().to_string())
}

// Keep ScalarType used so we don't accidentally drop the import when
// refactoring.
#[allow(dead_code)]
const _SCALAR_USED: Option<ScalarType> = None;

#[cfg(test)]
mod tests {
    use super::*;
    use prax_schema::parse_schema;
    use quote::quote;

    const SCHEMA: &str = include_str!("../../../tests/fixtures/schema.prax");

    fn parsed_schema() -> prax_schema::Schema {
        let mut v = prax_schema::Validator::new();
        v.validate(parse_schema(SCHEMA).unwrap()).unwrap()
    }

    fn parse_block(tokens: TokenStream) -> DslBlock {
        syn::parse2::<DslBlock>(tokens).unwrap()
    }

    fn pretty(ts: TokenStream) -> String {
        ts.to_string()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn lowers_nested_create_to_nested_write_op_create() {
        let schema = parsed_schema();
        let user = schema.get_model("User").unwrap().clone();
        let ctx = LowerCtx::new(&schema, &user);
        let field = user.get_field("posts").unwrap();
        let value = DslValue::Block(parse_block(quote!({
            create: [
                { title: "First", published: true },
                { title: "Second", published: false },
            ]
        })));
        let ops = lower_create_relation(field, &value, Span::call_site(), &ctx).unwrap();
        assert_eq!(ops.len(), 1);
        let s = pretty(ops[0].op_expr.clone());
        assert!(s.contains("NestedWriteOp"), "got: {s}");
        assert!(s.contains("Create"), "got: {s}");
        assert!(s.contains("posts"), "got: {s}");
    }

    #[test]
    fn lowers_nested_connect_to_nested_write_op_connect() {
        let schema = parsed_schema();
        let user = schema.get_model("User").unwrap().clone();
        let ctx = LowerCtx::new(&schema, &user);
        let field = user.get_field("posts").unwrap();
        let value = DslValue::Block(parse_block(quote!({
            connect: [{ id: 42 }]
        })));
        let ops = lower_create_relation(field, &value, Span::call_site(), &ctx).unwrap();
        assert_eq!(ops.len(), 1);
        let s = pretty(ops[0].op_expr.clone());
        assert!(s.contains("Connect"), "got: {s}");
        assert!(s.contains("target_pk"), "got: {s}");
        assert!(s.contains("42"), "got: {s}");
    }

    #[test]
    fn update_op_inside_relation_block_is_phase_5c_deferral() {
        let schema = parsed_schema();
        let user = schema.get_model("User").unwrap().clone();
        let ctx = LowerCtx::new(&schema, &user);
        let field = user.get_field("posts").unwrap();
        let value = DslValue::Block(parse_block(quote!({
            update: [{}]
        })));
        let err = lower_create_relation(field, &value, Span::call_site(), &ctx).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("phase 5c"), "got: {msg}");
        assert!(msg.contains("update"), "got: {msg}");
    }

    #[test]
    fn unknown_op_inside_relation_block_suggests_create() {
        let schema = parsed_schema();
        let user = schema.get_model("User").unwrap().clone();
        let ctx = LowerCtx::new(&schema, &user);
        let field = user.get_field("posts").unwrap();
        let value = DslValue::Block(parse_block(quote!({
            creat: [{ title: "x" }]
        })));
        let err = lower_create_relation(field, &value, Span::call_site(), &ctx).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unknown nested operator"), "got: {msg}");
        // suggest should mention "create" as a fix
        assert!(msg.contains("create"), "got: {msg}");
    }

    #[test]
    fn connect_or_create_is_phase_5d() {
        let schema = parsed_schema();
        let user = schema.get_model("User").unwrap().clone();
        let ctx = LowerCtx::new(&schema, &user);
        let field = user.get_field("posts").unwrap();
        let value = DslValue::Block(parse_block(quote!({
            connect_or_create: [{ where: {}, create: {} }]
        })));
        let err = lower_create_relation(field, &value, Span::call_site(), &ctx).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("phase 5d"), "got: {msg}");
    }
}
