//! Lower DSL `data:` blocks to the per-model typed input structs.
//!
//! Two entry points:
//!
//! - [`lower_create_data`] produces a `<Model>CreateInput` literal for
//!   the create-path macros (`create!`, `upsert!`'s `create:` key,
//!   `create_many!`'s list entries).
//! - [`lower_update_data`] produces a `<Model>UpdateInput` literal for
//!   the update-path macros (`update!`, `upsert!`'s `update:` key,
//!   `update_many!`).
//!
//! Phase 5a is scalar-only. Relation keys inside `data:` are rejected
//! with a clear "phase 5b" diagnostic; nested-write relation
//! operators (`create`/`connect`/etc.) land in phase 5b together with
//! `NestedWritePlan` IR and the executor.

#![allow(dead_code)]

use convert_case::{Case, Casing};
use prax_schema::{Field, FieldType, ScalarType};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};

use super::LowerCtx;
use crate::generators::inputs::{FilterCategory, update_wrapper_ident};
use crate::macros::dsl::ast::{DslBlock, DslField, DslValue};
use crate::macros::lower::scalar_filter::category_for_scalar;

/// Phase-5b deferral diagnostic for callers that don't yet support
/// nested writes (`create_many!`, `upsert!`, `update!`).
///
/// Wording matters: don't tell users they typed an "unknown field" —
/// the field is real, just not wired through this entry point yet.
fn relation_phase_5b_error(rel: &str, model: &str) -> syn::Error {
    let msg = format!(
        "nested write on relation `{rel}` (model `{model}`) is not supported in this macro. \
         Phase 5b lands nested `create` / `connect` on `create!` only; the other write macros \
         (update!, upsert!, create_many!) gain nested-write support in phase 5c. \
         For now, write the related rows in a separate operation and link via the FK column."
    );
    syn::Error::new(Span::call_site(), msg)
}

/// Phase-5a does not support logical combinators (`and`/`or`/`not`)
/// inside `data:`. These are valid in `where:` but make no sense for
/// a row-shaped input.
fn logical_in_data_error(op: &str, span: Span) -> syn::Error {
    syn::Error::new(
        span,
        format!("`{op}` is a `where:` operator and is not valid inside `data:`"),
    )
}

/// True for fields whose codegen-emitted `<Model>CreateInput` slot is
/// `Option<T>` rather than bare `T`.
///
/// Codegen wraps both nullable fields and fields with a default; the
/// macro must match the same gate so `Some(...)` wrapping is correct.
fn create_field_is_optional(field: &Field) -> bool {
    if field.is_optional() {
        return true;
    }
    let attrs = field.extract_attributes();
    attrs.default.is_some()
}

/// Map a scalar `FieldType` to the codegen `FilterCategory`, returning
/// `None` for relation / composite / unsupported types.
fn category_for_field(field: &Field) -> Option<FilterCategory> {
    match &field.field_type {
        FieldType::Scalar(s) => category_for_scalar(s),
        FieldType::Enum(_) => Some(FilterCategory::Enum),
        FieldType::Model(_) | FieldType::Composite(_) | FieldType::Unsupported(_) => None,
    }
}

/// Lowering result for a `data:` block on the create path with
/// nested-write support.
///
/// `scalar_input` is the `<Model>CreateInput` literal — fed to
/// `with_create_input` like before. `nested_ops` are the
/// [`NestedWriteOp`](::prax_query::nested::NestedWriteOp) token
/// streams extracted from relation keys; the macro emits a chained
/// `.with(<expr>)` call per entry.
pub struct CreateDataLowering {
    /// Token stream evaluating to a `<Model>CreateInput`.
    pub scalar_input: TokenStream,
    /// Per-relation `NestedWriteOp` expression token streams.
    pub nested_ops: Vec<TokenStream>,
}

/// Lower a `data: { ... }` block on the **create** path, recognising
/// both scalar fields and relation keys.
///
/// Relation keys with nested-write operators (`create:` / `connect:`)
/// are extracted into [`CreateDataLowering::nested_ops`]; the macro
/// chains a `.with(<nw>)` call per op onto the `CreateOperation`.
/// Scalar fields lower to a `<Model>CreateInput` literal as in phase 5a.
pub fn lower_create_data_with_nested(
    block: &DslBlock,
    ctx: &LowerCtx<'_>,
) -> syn::Result<CreateDataLowering> {
    let model_ident = format_ident!("{}", ctx.model.name());
    let module_ident = format_ident!("{}", ctx.model.name().to_case(Case::Snake));
    let input_ident = format_ident!("{}CreateInput", ctx.model.name());

    let mut scalar_stmts: Vec<TokenStream> = Vec::new();
    let mut nested_ops: Vec<TokenStream> = Vec::new();
    let mut field_iter = block.fields.iter().peekable();

    let init = if let Some(DslField::Spread { expr, by_move, .. }) = field_iter.peek() {
        let init_expr = if *by_move {
            quote!(#expr)
        } else {
            quote!(::core::clone::Clone::clone(&(#expr)))
        };
        let _ = field_iter.next();
        init_expr
    } else {
        quote!(<#module_ident::#input_ident as ::core::default::Default>::default())
    };

    for field in field_iter {
        match field {
            DslField::Pair { key, value, .. } => {
                let key_str = key.to_string();
                // Reject `where:` logical operators with a clearer error.
                if matches!(key_str.as_str(), "and" | "or" | "not") {
                    return Err(logical_in_data_error(&key_str, key.span()));
                }
                let model_field = lookup_field_with_suggestion(ctx, &key_str, key.span())?;
                if model_field.is_relation() {
                    // Lower relation key to nested-write op exprs.
                    let ops = super::data_relation::lower_create_relation(
                        model_field,
                        value,
                        key.span(),
                        ctx,
                    )?;
                    for op in ops {
                        nested_ops.push(op.op_expr);
                    }
                } else {
                    scalar_stmts.push(lower_create_pair(key, value, ctx)?);
                }
            }
            DslField::Spread { expr, by_move, .. } => {
                let assign = if *by_move {
                    quote!(__d = #expr;)
                } else {
                    quote!(__d = ::core::clone::Clone::clone(&(#expr));)
                };
                scalar_stmts.push(assign);
            }
            DslField::Conditional { .. } => {
                // Conditional handling for relation keys is deferred —
                // phase 5b's conditional-key lowering reuses the scalar
                // path, which already rejects relation keys.
                scalar_stmts.push(lower_create_conditional(field, ctx)?);
            }
        }
    }

    let scalar_input = quote! {
        {
            let mut __d: #module_ident::#input_ident = #init;
            #(#scalar_stmts)*
            let _ = stringify!(#model_ident);
            __d
        }
    };

    Ok(CreateDataLowering {
        scalar_input,
        nested_ops,
    })
}

/// Lower a `data: { ... }` block on the **create** path to a
/// `<Model>CreateInput` constructor.
///
/// Phase-5b note: This entry point is used by `create_many!` and
/// `upsert!`, which do not support nested writes yet. It still
/// rejects relation keys with the phase-5b diagnostic.
pub fn lower_create_data(block: &DslBlock, ctx: &LowerCtx<'_>) -> syn::Result<TokenStream> {
    let model_ident = format_ident!("{}", ctx.model.name());
    let module_ident = format_ident!("{}", ctx.model.name().to_case(Case::Snake));
    let input_ident = format_ident!("{}CreateInput", ctx.model.name());

    let mut stmts: Vec<TokenStream> = Vec::new();
    let mut field_iter = block.fields.iter().peekable();

    // Leading spread acts as the seed: `..base` → `Clone::clone(&base)`,
    // `..move base` → `base` (consume). Subsequent assignments
    // overwrite.
    let init = if let Some(DslField::Spread { expr, by_move, .. }) = field_iter.peek() {
        let init_expr = if *by_move {
            quote!(#expr)
        } else {
            quote!(::core::clone::Clone::clone(&(#expr)))
        };
        let _ = field_iter.next();
        init_expr
    } else {
        quote!(<#module_ident::#input_ident as ::core::default::Default>::default())
    };

    for field in field_iter {
        match field {
            DslField::Pair { key, value, .. } => {
                stmts.push(lower_create_pair(key, value, ctx)?);
            }
            DslField::Spread { expr, by_move, .. } => {
                let assign = if *by_move {
                    quote!(__d = #expr;)
                } else {
                    quote!(__d = ::core::clone::Clone::clone(&(#expr));)
                };
                stmts.push(assign);
            }
            DslField::Conditional { .. } => {
                stmts.push(lower_create_conditional(field, ctx)?);
            }
        }
    }

    Ok(quote! {
        {
            let mut __d: #module_ident::#input_ident = #init;
            #(#stmts)*
            let _ = stringify!(#model_ident);
            __d
        }
    })
}

/// Lowering result for a `data:` block on the update path with
/// nested-write support.
///
/// `scalar_input` is the `<Model>UpdateInput` literal — fed to
/// `with_update_input` like before. `nested_ops` are the
/// [`NestedWriteOp`](::prax_query::nested::NestedWriteOp) token
/// streams extracted from relation keys; the macro emits a chained
/// `.with(...)` call per entry (with the appropriate branch variant
/// for `upsert!`).
pub struct UpdateDataLowering {
    /// Token stream evaluating to a `<Model>UpdateInput`.
    pub scalar_input: TokenStream,
    /// Per-relation `NestedWriteOp` expression token streams.
    pub nested_ops: Vec<TokenStream>,
}

/// Lower a `data: { ... }` block on the **update** path, recognising
/// both scalar fields and relation keys.
///
/// Mirrors [`lower_create_data_with_nested`]: relation keys with
/// nested-write operators are extracted into
/// [`UpdateDataLowering::nested_ops`]; scalar fields lower to a
/// `<Model>UpdateInput` literal as in `lower_update_data`.
///
/// The nested-write ops produced by
/// [`super::data_relation::lower_create_relation`] are agnostic to
/// whether the parent op is create / update / upsert — they emit
/// `NestedWriteOp::*` constructors keyed off the relation metadata,
/// not the parent op type.
pub fn lower_update_data_with_nested(
    block: &DslBlock,
    ctx: &LowerCtx<'_>,
) -> syn::Result<UpdateDataLowering> {
    let model_ident = format_ident!("{}", ctx.model.name());
    let module_ident = format_ident!("{}", ctx.model.name().to_case(Case::Snake));
    let input_ident = format_ident!("{}UpdateInput", ctx.model.name());

    let mut scalar_stmts: Vec<TokenStream> = Vec::new();
    let mut nested_ops: Vec<TokenStream> = Vec::new();
    let mut field_iter = block.fields.iter().peekable();

    let init = if let Some(DslField::Spread { expr, by_move, .. }) = field_iter.peek() {
        let init_expr = if *by_move {
            quote!(#expr)
        } else {
            quote!(::core::clone::Clone::clone(&(#expr)))
        };
        let _ = field_iter.next();
        init_expr
    } else {
        quote!(<#module_ident::#input_ident as ::core::default::Default>::default())
    };

    for field in field_iter {
        match field {
            DslField::Pair { key, value, .. } => {
                let key_str = key.to_string();
                if matches!(key_str.as_str(), "and" | "or" | "not") {
                    return Err(logical_in_data_error(&key_str, key.span()));
                }
                let model_field = lookup_field_with_suggestion(ctx, &key_str, key.span())?;
                if model_field.is_relation() {
                    let ops = super::data_relation::lower_create_relation(
                        model_field,
                        value,
                        key.span(),
                        ctx,
                    )?;
                    for op in ops {
                        nested_ops.push(op.op_expr);
                    }
                } else {
                    scalar_stmts.push(lower_update_pair(key, value, ctx)?);
                }
            }
            DslField::Spread { expr, by_move, .. } => {
                let assign = if *by_move {
                    quote!(__d = #expr;)
                } else {
                    quote!(__d = ::core::clone::Clone::clone(&(#expr));)
                };
                scalar_stmts.push(assign);
            }
            DslField::Conditional { .. } => {
                scalar_stmts.push(lower_update_conditional(field, ctx)?);
            }
        }
    }

    let scalar_input = quote! {
        {
            let mut __d: #module_ident::#input_ident = #init;
            #(#scalar_stmts)*
            let _ = stringify!(#model_ident);
            __d
        }
    };

    Ok(UpdateDataLowering {
        scalar_input,
        nested_ops,
    })
}

/// Lower a `data: { ... }` block on the **update** path to a
/// `<Model>UpdateInput` constructor.
pub fn lower_update_data(block: &DslBlock, ctx: &LowerCtx<'_>) -> syn::Result<TokenStream> {
    let model_ident = format_ident!("{}", ctx.model.name());
    let module_ident = format_ident!("{}", ctx.model.name().to_case(Case::Snake));
    let input_ident = format_ident!("{}UpdateInput", ctx.model.name());

    let mut stmts: Vec<TokenStream> = Vec::new();
    let mut field_iter = block.fields.iter().peekable();

    let init = if let Some(DslField::Spread { expr, by_move, .. }) = field_iter.peek() {
        let init_expr = if *by_move {
            quote!(#expr)
        } else {
            quote!(::core::clone::Clone::clone(&(#expr)))
        };
        let _ = field_iter.next();
        init_expr
    } else {
        quote!(<#module_ident::#input_ident as ::core::default::Default>::default())
    };

    for field in field_iter {
        match field {
            DslField::Pair { key, value, .. } => {
                stmts.push(lower_update_pair(key, value, ctx)?);
            }
            DslField::Spread { expr, by_move, .. } => {
                let assign = if *by_move {
                    quote!(__d = #expr;)
                } else {
                    quote!(__d = ::core::clone::Clone::clone(&(#expr));)
                };
                stmts.push(assign);
            }
            DslField::Conditional { .. } => {
                stmts.push(lower_update_conditional(field, ctx)?);
            }
        }
    }

    Ok(quote! {
        {
            let mut __d: #module_ident::#input_ident = #init;
            #(#stmts)*
            let _ = stringify!(#model_ident);
            __d
        }
    })
}

fn lower_create_pair(
    key: &syn::Ident,
    value: &DslValue,
    ctx: &LowerCtx<'_>,
) -> syn::Result<TokenStream> {
    let key_str = key.to_string();

    // Reject `where:` logical operators with a clearer error than the
    // generic "unknown field".
    if matches!(key_str.as_str(), "and" | "or" | "not") {
        return Err(logical_in_data_error(&key_str, key.span()));
    }

    let field = lookup_field_with_suggestion(ctx, &key_str, key.span())?;

    // Relation fields → phase-5b deferral.
    if field.is_relation() {
        return Err(relation_phase_5b_error(field.name(), ctx.model.name()));
    }

    let assign_ident = format_ident!("{}", field.name().to_case(Case::Snake));
    let is_optional = create_field_is_optional(field);

    // Scalar value lowering. The codegen-emitted CreateInput field
    // type is the bare payload (e.g. `String`, `i32`) or the user
    // enum type, so we lean on `Into` to coerce literals where
    // possible.
    let value_expr = lower_create_value(field, value, key.span())?;

    let stmt = if is_optional {
        quote! { __d.#assign_ident = ::core::option::Option::Some(#value_expr); }
    } else {
        quote! { __d.#assign_ident = #value_expr; }
    };
    Ok(stmt)
}

fn lower_update_pair(
    key: &syn::Ident,
    value: &DslValue,
    ctx: &LowerCtx<'_>,
) -> syn::Result<TokenStream> {
    let key_str = key.to_string();

    if matches!(key_str.as_str(), "and" | "or" | "not") {
        return Err(logical_in_data_error(&key_str, key.span()));
    }

    let field = lookup_field_with_suggestion(ctx, &key_str, key.span())?;

    if field.is_relation() {
        return Err(relation_phase_5b_error(field.name(), ctx.model.name()));
    }

    let assign_ident = format_ident!("{}", field.name().to_case(Case::Snake));
    let wrapper_expr = lower_update_value(field, value, key.span())?;
    Ok(quote! {
        __d.#assign_ident = ::core::option::Option::Some(#wrapper_expr);
    })
}

/// Lower a scalar value for the **create** path.
///
/// Accepts:
/// - literal (`"a@x.com"`, `30`, `true`) → `Into::into(<lit>)`
/// - bare ident (`Admin`) → resolves against the field's enum type
/// - path (`Role::Admin`) → emitted verbatim
/// - `@(expr)` Rust escape → emitted verbatim
fn lower_create_value(field: &Field, value: &DslValue, span: Span) -> syn::Result<TokenStream> {
    match (&field.field_type, value) {
        // Enum field + bare ident: `role: Admin` → `Role::Admin`.
        (FieldType::Enum(enum_name), DslValue::BareIdent(variant)) => {
            let enum_ident = format_ident!("{}", enum_name.as_str());
            Ok(quote! { #enum_ident::#variant })
        }
        // Enum field + path: `role: crate::Role::Admin`.
        (FieldType::Enum(_), DslValue::Path(p)) => Ok(quote! { #p }),
        // Enum field + escape: `role: @(role_var)`.
        (FieldType::Enum(_), DslValue::Expr(e)) => Ok(quote! { #e }),
        // Non-enum + bare ident: not allowed.
        (_, DslValue::BareIdent(id)) => Err(syn::Error::new(
            id.span(),
            format!(
                "bare identifier `{}` is only allowed for enum-typed fields. \
                 Field `{}` is not an enum.",
                id,
                field.name()
            ),
        )),
        (_, DslValue::Lit(lit)) => Ok(quote! { ::core::convert::Into::into(#lit) }),
        (_, DslValue::Bool(b)) => Ok(quote! { #b }),
        (_, DslValue::Expr(e)) => Ok(quote! { (#e) }),
        (_, DslValue::Path(p)) => Ok(quote! { #p }),
        (_, DslValue::Block(_)) => Err(syn::Error::new(
            span,
            format!(
                "scalar field `{}` on the create path expects a literal, identifier, or `@(expr)`. \
                 `{{ set: ... }}` blocks are an update-path concept; on `create:` the value is the \
                 column's initial value.",
                field.name()
            ),
        )),
        (_, DslValue::List(_)) => Err(syn::Error::new(
            span,
            format!(
                "scalar field `{}` does not accept a list value on the create path",
                field.name()
            ),
        )),
    }
}

/// Lower a scalar value for the **update** path to a `*FieldUpdate`
/// wrapper literal.
fn lower_update_value(field: &Field, value: &DslValue, span: Span) -> syn::Result<TokenStream> {
    // Determine the wrapper type for the field — used by both the
    // literal-shortcut path and the block path.
    let cat = category_for_field(field).ok_or_else(|| {
        syn::Error::new(
            span,
            format!(
                "scalar field `{}` has no DSL lowering for `data:` updates",
                field.name()
            ),
        )
    })?;
    let nullable = field.is_optional();
    let wrapper_ident = update_wrapper_ident(cat, nullable);
    let wrapper_path = wrapper_path_for(cat, &wrapper_ident, field)?;

    match value {
        // Bare literal: `name: "Bob"` → wrapper { set: Some("Bob".into()), .. }
        DslValue::Lit(lit) => Ok(quote! {
            <#wrapper_path as ::core::convert::From<_>>::from(#lit)
        }),
        DslValue::Bool(b) => {
            if !matches!(cat, FilterCategory::Bool) {
                return Err(syn::Error::new(
                    span,
                    format!(
                        "field `{}` (category {cat:?}) does not accept a bare bool value",
                        field.name()
                    ),
                ));
            }
            Ok(quote! {
                <#wrapper_path as ::core::convert::From<_>>::from(#b)
            })
        }
        DslValue::Expr(e) => Ok(quote! {
            <#wrapper_path as ::core::convert::From<_>>::from(#e)
        }),
        DslValue::Path(p) => {
            // Treat as a value-producing path; rely on From<...>.
            if matches!(cat, FilterCategory::Enum) {
                Ok(quote! {
                    #wrapper_path { set: ::core::option::Option::Some(#p), ..::core::default::Default::default() }
                })
            } else {
                Ok(quote! {
                    <#wrapper_path as ::core::convert::From<_>>::from(#p)
                })
            }
        }
        DslValue::BareIdent(id) => {
            // Bare ident only valid for enum-typed fields.
            let FieldType::Enum(enum_name) = &field.field_type else {
                return Err(syn::Error::new(
                    id.span(),
                    format!(
                        "bare identifier `{id}` is only allowed for enum-typed fields. \
                         Field `{}` is not an enum.",
                        field.name()
                    ),
                ));
            };
            let enum_ident = format_ident!("{}", enum_name.as_str());
            Ok(quote! {
                #wrapper_path { set: ::core::option::Option::Some(#enum_ident::#id), ..::core::default::Default::default() }
            })
        }
        DslValue::Block(block) => {
            lower_update_block(field, cat, nullable, &wrapper_path, block, span)
        }
        DslValue::List(_) => Err(syn::Error::new(
            span,
            format!(
                "scalar field `{}` does not accept a list value on the update path",
                field.name()
            ),
        )),
    }
}

/// Build the full path to a `*FieldUpdate` wrapper — handles the enum
/// generic parameter when the field is enum-typed.
fn wrapper_path_for(
    cat: FilterCategory,
    wrapper_ident: &syn::Ident,
    field: &Field,
) -> syn::Result<TokenStream> {
    if matches!(cat, FilterCategory::Enum) {
        let FieldType::Enum(enum_name) = &field.field_type else {
            return Err(syn::Error::new(
                Span::call_site(),
                "enum category requires an enum field type",
            ));
        };
        let enum_ident = format_ident!("{}", enum_name.as_str());
        Ok(quote! { ::prax_query::inputs::#wrapper_ident::<#enum_ident> })
    } else {
        Ok(quote! { ::prax_query::inputs::#wrapper_ident })
    }
}

fn lower_update_block(
    field: &Field,
    cat: FilterCategory,
    nullable: bool,
    wrapper_path: &TokenStream,
    block: &DslBlock,
    _span: Span,
) -> syn::Result<TokenStream> {
    let mut setters: Vec<TokenStream> = Vec::new();
    for entry in &block.fields {
        let DslField::Pair { key, value, .. } = entry else {
            return Err(syn::Error::new(
                Span::call_site(),
                format!(
                    "update block for `{}` does not support spread or conditional fields yet",
                    field.name()
                ),
            ));
        };
        let op = key.to_string();
        match op.as_str() {
            "set" => {
                let v = lower_update_op_value(field, value, key.span())?;
                setters.push(quote! { set: ::core::option::Option::Some(#v) });
            }
            "increment" | "decrement" | "multiply" | "divide" => {
                if !category_has_arithmetic(cat) {
                    return Err(syn::Error::new(
                        key.span(),
                        format!(
                            "operator `{op}` is not valid on non-numeric field `{}` ({cat:?})",
                            field.name()
                        ),
                    ));
                }
                let v = lower_update_op_value(field, value, key.span())?;
                let op_ident = format_ident!("{op}");
                setters.push(quote! { #op_ident: ::core::option::Option::Some(#v) });
            }
            "unset" => {
                if !nullable {
                    return Err(syn::Error::new(
                        key.span(),
                        format!(
                            "`unset` is only valid for nullable fields. Field `{}` is not nullable.",
                            field.name()
                        ),
                    ));
                }
                let DslValue::Bool(b) = value else {
                    return Err(syn::Error::new(
                        key.span(),
                        "`unset` expects `true` or `false`",
                    ));
                };
                setters.push(quote! { unset: ::core::option::Option::Some(#b) });
            }
            other => {
                return Err(syn::Error::new(
                    key.span(),
                    format!(
                        "unknown update operator `{other}` on `{}`. \
                         Valid: set, increment, decrement, multiply, divide, unset.",
                        field.name()
                    ),
                ));
            }
        }
    }

    Ok(quote! {
        #wrapper_path {
            #(#setters,)*
            ..::core::default::Default::default()
        }
    })
}

/// Lower the RHS of an inner update operator (`set: <X>`,
/// `increment: <X>`, etc.).
///
/// The wrapper's `set` field is the typed scalar payload (e.g. `String`,
/// `i64`) and we coerce with `Into`. For enums the `set` field is the
/// user enum, so we resolve bare idents against the field's enum type.
fn lower_update_op_value(field: &Field, value: &DslValue, span: Span) -> syn::Result<TokenStream> {
    match (&field.field_type, value) {
        (FieldType::Enum(enum_name), DslValue::BareIdent(variant)) => {
            let enum_ident = format_ident!("{}", enum_name.as_str());
            Ok(quote! { #enum_ident::#variant })
        }
        (FieldType::Enum(_), DslValue::Path(p)) => Ok(quote! { #p }),
        (FieldType::Enum(_), DslValue::Expr(e)) => Ok(quote! { #e }),
        (_, DslValue::BareIdent(id)) => Err(syn::Error::new(
            id.span(),
            format!(
                "bare identifier `{id}` is only allowed for enum-typed fields. \
                 Field `{}` is not an enum.",
                field.name()
            ),
        )),
        (_, DslValue::Lit(lit)) => Ok(quote! { ::core::convert::Into::into(#lit) }),
        (_, DslValue::Bool(b)) => Ok(quote! { #b }),
        (_, DslValue::Expr(e)) => Ok(quote! { ::core::convert::Into::into(#e) }),
        (_, DslValue::Path(p)) => Ok(quote! { ::core::convert::Into::into(#p) }),
        (_, DslValue::Block(_)) => Err(syn::Error::new(
            span,
            format!(
                "nested block is not a valid update-operator value for `{}`",
                field.name()
            ),
        )),
        (_, DslValue::List(_)) => Err(syn::Error::new(
            span,
            format!(
                "list is not a valid update-operator value for `{}`",
                field.name()
            ),
        )),
    }
}

fn category_has_arithmetic(cat: FilterCategory) -> bool {
    matches!(
        cat,
        FilterCategory::Int
            | FilterCategory::BigInt
            | FilterCategory::Float
            | FilterCategory::Decimal
    )
}

fn lookup_field_with_suggestion<'a>(
    ctx: &LowerCtx<'a>,
    key: &str,
    span: Span,
) -> syn::Result<&'a Field> {
    if let Some(f) = ctx.model.get_field(key) {
        return Ok(f);
    }
    let candidates: Vec<String> = ctx.model.fields.keys().map(|k| k.to_string()).collect();
    let suggestion = crate::macros::validate::suggest(key, &candidates);
    let msg = match suggestion {
        Some(c) => format!(
            "unknown field `{key}` on model `{}` in `data:` block. did you mean `{c}`?",
            ctx.model.name()
        ),
        None => format!(
            "unknown field `{key}` on model `{}` in `data:` block",
            ctx.model.name()
        ),
    };
    Err(syn::Error::new(span, msg))
}

fn lower_create_conditional(field: &DslField, ctx: &LowerCtx<'_>) -> syn::Result<TokenStream> {
    let DslField::Conditional {
        cond,
        kind,
        key,
        value,
        ..
    } = field
    else {
        unreachable!("called with non-conditional");
    };
    let pair = lower_create_pair(key, value, ctx)?;
    use crate::macros::dsl::ast::CondKind;
    Ok(match kind {
        CondKind::If => quote! { if #cond { #pair } },
        CondKind::ElseIf => quote! { else if #cond { #pair } },
        CondKind::Else => quote! { else { #pair } },
    })
}

fn lower_update_conditional(field: &DslField, ctx: &LowerCtx<'_>) -> syn::Result<TokenStream> {
    let DslField::Conditional {
        cond,
        kind,
        key,
        value,
        ..
    } = field
    else {
        unreachable!("called with non-conditional");
    };
    let pair = lower_update_pair(key, value, ctx)?;
    use crate::macros::dsl::ast::CondKind;
    Ok(match kind {
        CondKind::If => quote! { if #cond { #pair } },
        CondKind::ElseIf => quote! { else if #cond { #pair } },
        CondKind::Else => quote! { else { #pair } },
    })
}

// `category_for_scalar` is `pub(crate)`, and `ScalarType` is needed to
// keep the cross-module use list correct under `--deny warnings`.
#[allow(dead_code)]
const _SCALAR_TYPE_USED: Option<ScalarType> = None;

#[cfg(test)]
mod tests {
    use super::*;
    use prax_schema::parse_schema;
    use quote::quote;

    const SCHEMA: &str = include_str!("../../../tests/fixtures/schema.prax");

    fn parse_block(tokens: TokenStream) -> DslBlock {
        syn::parse2::<DslBlock>(tokens).unwrap()
    }

    fn pretty(ts: TokenStream) -> String {
        ts.to_string()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Parse + validate so `FieldType::Model(\"Role\")` is remapped to
    /// `FieldType::Enum(\"Role\")` before lowering. The production
    /// proc-macro path skips validation today (phase-3 limitation),
    /// so enum-shape lowering on the macro pipeline isn't fully wired
    /// — but the unit-tested helpers exercise the lowering directly.
    fn parsed_schema() -> prax_schema::Schema {
        let mut v = prax_schema::Validator::new();
        v.validate(parse_schema(SCHEMA).unwrap()).unwrap()
    }

    fn create(model_name: &str, tokens: TokenStream) -> TokenStream {
        let schema = parsed_schema();
        let model = schema.get_model(model_name).unwrap().clone();
        let ctx = LowerCtx::new(&schema, &model);
        lower_create_data(&parse_block(tokens), &ctx).unwrap()
    }

    fn update(model_name: &str, tokens: TokenStream) -> TokenStream {
        let schema = parsed_schema();
        let model = schema.get_model(model_name).unwrap().clone();
        let ctx = LowerCtx::new(&schema, &model);
        lower_update_data(&parse_block(tokens), &ctx).unwrap()
    }

    #[test]
    fn create_data_scalar_required_and_optional() {
        let out = create("User", quote!({ email: "a@x.com", name: "Alice", age: 30 }));
        let s = pretty(out);
        assert!(s.contains("UserCreateInput"), "got: {s}");
        // Required `email` assigned bare; optional `name`/`age` wrapped
        // with Some(...).
        assert!(s.contains("__d . email ="), "got: {s}");
        assert!(s.contains("Some"), "got: {s}");
    }

    #[test]
    fn create_data_bare_enum_ident() {
        let out = create("User", quote!({ email: "a@x.com", role: Admin }));
        let s = pretty(out);
        assert!(s.contains("Role :: Admin"), "got: {s}");
    }

    #[test]
    fn create_data_expression_escape() {
        let out = create(
            "User",
            quote!({ email: @(format!("{}@x.com", name)), age: @(my_age) }),
        );
        let s = pretty(out);
        assert!(s.contains("format !"), "got: {s}");
    }

    #[test]
    fn create_data_with_spread() {
        let out = create("User", quote!({ ..base, email: "a@x.com" }));
        let s = pretty(out);
        assert!(s.contains("Clone :: clone"), "got: {s}");
        assert!(s.contains("base"), "got: {s}");
    }

    #[test]
    fn create_data_relation_key_is_phase_5c_on_legacy_callers() {
        // `lower_create_data` is now used only by `create_many!` /
        // `upsert!` / `update!`; those still reject relation keys.
        // The `create!` macro uses `lower_create_data_with_nested`
        // which routes relation keys through `data_relation`.
        let schema = parsed_schema();
        let model = schema.get_model("User").unwrap().clone();
        let ctx = LowerCtx::new(&schema, &model);
        let block = parse_block(quote!({ email: "a@x.com", posts: { create: [] } }));
        let err = lower_create_data(&block, &ctx).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("phase 5c"), "got: {msg}");
        assert!(msg.contains("posts"), "got: {msg}");
    }

    #[test]
    fn create_data_with_nested_lowers_relation_block_to_nested_ops() {
        let schema = parsed_schema();
        let model = schema.get_model("User").unwrap().clone();
        let ctx = LowerCtx::new(&schema, &model);
        let block = parse_block(quote!({
            email: "a@x.com",
            posts: { create: [{ title: "p1", published: true }] }
        }));
        let lowered = lower_create_data_with_nested(&block, &ctx).unwrap();
        assert_eq!(lowered.nested_ops.len(), 1);
        let scalar = pretty(lowered.scalar_input);
        assert!(scalar.contains("UserCreateInput"), "got: {scalar}");
        let nw = pretty(lowered.nested_ops[0].clone());
        assert!(nw.contains("NestedWriteOp"), "got: {nw}");
    }

    #[test]
    fn create_data_unknown_field_errors_with_suggestion() {
        let schema = parsed_schema();
        let model = schema.get_model("User").unwrap().clone();
        let ctx = LowerCtx::new(&schema, &model);
        let block = parse_block(quote!({ emial: "x" }));
        let err = lower_create_data(&block, &ctx).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unknown field"), "got: {msg}");
        assert!(msg.contains("did you mean"), "got: {msg}");
        assert!(msg.contains("email"), "got: {msg}");
    }

    #[test]
    fn create_data_logical_op_rejected() {
        let schema = parsed_schema();
        let model = schema.get_model("User").unwrap().clone();
        let ctx = LowerCtx::new(&schema, &model);
        let block = parse_block(quote!({ or: [{}, {}] }));
        let err = lower_create_data(&block, &ctx).unwrap_err();
        assert!(err.to_string().contains("`where:` operator"), "got: {err}");
    }

    // ========== update_data tests ==========

    #[test]
    fn update_data_plain_set_via_literal() {
        let out = update("User", quote!({ email: "bob@x.com" }));
        let s = pretty(out);
        // Plain literal lowers to `<Wrapper as From<_>>::from(lit)`.
        assert!(s.contains("UserUpdateInput"), "got: {s}");
        assert!(s.contains("StringFieldUpdate"), "got: {s}");
        assert!(s.contains("From"), "got: {s}");
    }

    #[test]
    fn update_data_explicit_set_block() {
        let out = update("User", quote!({ email: { set: "bob@x.com" } }));
        let s = pretty(out);
        // The wrapper's `set` field is initialised to Some(...).
        // The token stream's whitespace pretty-print uses fully
        // qualified `:: core :: option :: Option :: Some`, so the
        // assertion just looks for `set :` followed by an `Option`
        // path constructor.
        assert!(s.contains("set :"), "got: {s}");
        assert!(s.contains("Option :: Some"), "got: {s}");
    }

    #[test]
    fn update_data_increment_on_numeric() {
        let out = update("User", quote!({ age: { increment: 1 } }));
        let s = pretty(out);
        assert!(s.contains("IntNullableFieldUpdate"), "got: {s}");
        assert!(s.contains("increment"), "got: {s}");
    }

    #[test]
    fn update_data_decrement_on_numeric() {
        let out = update("User", quote!({ age: { decrement: 2 } }));
        let s = pretty(out);
        assert!(s.contains("decrement"), "got: {s}");
    }

    #[test]
    fn update_data_unset_on_nullable() {
        let out = update("User", quote!({ name: { unset: true } }));
        let s = pretty(out);
        assert!(s.contains("StringNullableFieldUpdate"), "got: {s}");
        assert!(s.contains("unset :"), "got: {s}");
        assert!(s.contains("Option :: Some (true)"), "got: {s}");
    }

    #[test]
    fn update_data_increment_on_string_errors() {
        let schema = parsed_schema();
        let model = schema.get_model("User").unwrap().clone();
        let ctx = LowerCtx::new(&schema, &model);
        // `email: String` (non-numeric) — `increment` is invalid.
        let block = parse_block(quote!({ email: { increment: 1 } }));
        let err = lower_update_data(&block, &ctx).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("increment"), "got: {msg}");
        assert!(msg.contains("non-numeric"), "got: {msg}");
    }

    #[test]
    fn update_data_unset_on_non_nullable_errors() {
        let schema = parsed_schema();
        let model = schema.get_model("User").unwrap().clone();
        let ctx = LowerCtx::new(&schema, &model);
        // `email: String` is required — `unset` is invalid.
        let block = parse_block(quote!({ email: { unset: true } }));
        let err = lower_update_data(&block, &ctx).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unset"), "got: {msg}");
        assert!(msg.contains("nullable"), "got: {msg}");
    }

    #[test]
    fn update_data_relation_key_is_phase_5c() {
        let schema = parsed_schema();
        let model = schema.get_model("User").unwrap().clone();
        let ctx = LowerCtx::new(&schema, &model);
        let block = parse_block(quote!({ posts: { update: [] } }));
        let err = lower_update_data(&block, &ctx).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("phase 5c"), "got: {msg}");
    }

    #[test]
    fn update_data_with_nested_lowers_relation_block_to_nested_ops() {
        let schema = parsed_schema();
        let model = schema.get_model("User").unwrap().clone();
        let ctx = LowerCtx::new(&schema, &model);
        let block = parse_block(quote!({
            name: "Renamed",
            posts: { create: [{ title: "p1", published: true }] }
        }));
        let lowered = lower_update_data_with_nested(&block, &ctx).unwrap();
        assert_eq!(lowered.nested_ops.len(), 1);
        let scalar = pretty(lowered.scalar_input);
        // The scalar half lowers to a `<Model>UpdateInput` literal.
        assert!(scalar.contains("UserUpdateInput"), "got: {scalar}");
        let nw = pretty(lowered.nested_ops[0].clone());
        assert!(nw.contains("NestedWriteOp"), "got: {nw}");
    }

    #[test]
    fn update_data_with_nested_mixes_scalar_and_disconnect() {
        let schema = parsed_schema();
        let model = schema.get_model("User").unwrap().clone();
        let ctx = LowerCtx::new(&schema, &model);
        let block = parse_block(quote!({
            name: "Renamed",
            posts: { disconnect: [{ id: 5 }] }
        }));
        let lowered = lower_update_data_with_nested(&block, &ctx).unwrap();
        assert_eq!(
            lowered.nested_ops.len(),
            1,
            "exactly one disconnect op expected"
        );
        let nw = pretty(lowered.nested_ops[0].clone());
        assert!(nw.contains("Disconnect"), "got: {nw}");
    }
}
