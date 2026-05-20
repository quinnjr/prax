//! Lower DSL `where:` blocks to constructors for the per-model
//! `<Model>WhereInput` type emitted by phase 2 codegen.
//!
//! These helpers are wired into the `ops/*` entry points starting in
//! task 13; until then dead_code warnings would block clippy --deny.

#![allow(dead_code)]
//!
//! Strategy (per spec §4 expansion sketch):
//! ```text
//! {
//!     let mut __w = <ModelWhereInput>::default();
//!     __w.email   = Some(StringFilter { equals: Some("..."), .. });
//!     __w.age     = Some(IntNullableFilter { gte: Some(18), .. });
//!     __w.posts   = Some(ListRelationFilter { some: Some(...), .. });
//!     __w.or      = Some(vec![ ... ]);
//!     __w
//! }
//! ```

use convert_case::{Case, Casing};
use prax_schema::{Field, Model};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};

use super::LowerCtx;
use super::scalar_filter::lower_scalar_filter;
use crate::macros::dsl::ast::{DslBlock, DslField, DslValue};

/// Lower a `where: { ... }` block to a `TokenStream` that constructs
/// the per-model `<Model>WhereInput` value.
pub fn lower_where(block: &DslBlock, ctx: &LowerCtx<'_>) -> syn::Result<TokenStream> {
    let model_ident = format_ident!("{}", ctx.model.name());
    let module_ident = format_ident!("{}", ctx.model.name().to_case(Case::Snake));
    let where_input_ident = format_ident!("{}WhereInput", ctx.model.name());

    let mut stmts: Vec<TokenStream> = Vec::new();
    // Per the spec, leading spread becomes the seed. If the first field
    // is `..expr`, use it as the initializer; otherwise default.
    let mut field_iter = block.fields.iter().peekable();
    let init = if let Some(DslField::Spread { expr, by_move, .. }) = field_iter.peek() {
        let init_expr = if *by_move {
            quote!(#expr)
        } else {
            quote!(::core::clone::Clone::clone(&(#expr)))
        };
        // Consume the leading spread.
        let _ = field_iter.next();
        init_expr
    } else {
        quote!(<#module_ident::#where_input_ident as ::core::default::Default>::default())
    };

    for field in field_iter {
        match field {
            DslField::Pair { key, value, span } => {
                stmts.push(lower_where_pair(key, value, *span, ctx)?);
            }
            DslField::Spread { expr, by_move, .. } => {
                // Mid-block spread: overwrite __w.
                let assign = if *by_move {
                    quote!(__w = #expr;)
                } else {
                    quote!(__w = ::core::clone::Clone::clone(&(#expr));)
                };
                stmts.push(assign);
            }
            DslField::Conditional { .. } => {
                stmts.push(lower_where_conditional(field, ctx)?);
            }
        }
    }

    Ok(quote! {
        {
            let mut __w: #module_ident::#where_input_ident = #init;
            #(#stmts)*
            let __unused: &#module_ident::#where_input_ident = &__w;
            let _ = __unused;
            let _ = stringify!(#model_ident);
            __w
        }
    })
}

fn lower_where_pair(
    key: &syn::Ident,
    value: &DslValue,
    _span: Span,
    ctx: &LowerCtx<'_>,
) -> syn::Result<TokenStream> {
    let key_str = key.to_string();
    // Logical combinators first.
    match key_str.as_str() {
        "and" | "or" => {
            let DslValue::List(items) = value else {
                return Err(syn::Error::new(
                    key.span(),
                    format!("`{key_str}` expects a list of where blocks: `[{{...}}, {{...}}]`"),
                ));
            };
            let inner: Vec<TokenStream> = items
                .iter()
                .map(|v| match v {
                    DslValue::Block(b) => lower_where(b, ctx),
                    _ => Err(syn::Error::new(
                        key.span(),
                        format!("each entry under `{key_str}` must be a `{{ ... }}` block"),
                    )),
                })
                .collect::<syn::Result<_>>()?;
            let key_ident = format_ident!("{key_str}");
            return Ok(quote! {
                __w.#key_ident = ::core::option::Option::Some(::std::vec![ #(#inner),* ]);
            });
        }
        "not" => {
            let DslValue::Block(b) = value else {
                return Err(syn::Error::new(
                    key.span(),
                    "`not` expects a `{ ... }` block",
                ));
            };
            let inner = lower_where(b, ctx)?;
            return Ok(quote! {
                __w.not = ::core::option::Option::Some(::std::boxed::Box::new(#inner));
            });
        }
        _ => {}
    }

    let field = ctx.model.get_field(&key_str).ok_or_else(|| {
        let candidates = collect_field_names(ctx.model);
        let suggestion = crate::macros::validate::suggest(&key_str, &candidates);
        let msg = match suggestion {
            Some(c) => format!(
                "unknown field `{}` on model `{}`. did you mean `{}`?",
                key_str,
                ctx.model.name(),
                c
            ),
            None => format!(
                "unknown field `{}` on model `{}`",
                key_str,
                ctx.model.name()
            ),
        };
        syn::Error::new(key.span(), msg)
    })?;

    if field.is_relation() {
        return lower_relation_filter(field, value, ctx);
    }

    let field_name = field.name().to_string();
    let nullable = field.is_optional();
    let filter = lower_scalar_filter(&field_name, &field.field_type, nullable, value)?;
    let assign_ident = format_ident!("{}", field.name().to_case(Case::Snake));
    Ok(quote! {
        __w.#assign_ident = ::core::option::Option::Some(#filter);
    })
}

fn lower_relation_filter(
    field: &Field,
    value: &DslValue,
    ctx: &LowerCtx<'_>,
) -> syn::Result<TokenStream> {
    let prax_schema::FieldType::Model(target_name) = &field.field_type else {
        return Err(syn::Error::new(
            field.name.span.into_proc_macro_span_fallback(),
            format!("field `{}` is not a relation", field.name()),
        ));
    };
    let target_model = ctx.schema.get_model(target_name).ok_or_else(|| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            format!(
                "field `{}` references model `{}` which is not in the schema",
                field.name(),
                target_name
            ),
        )
    })?;
    let target_module = format_ident!("{}", target_model.name().to_case(Case::Snake));
    let target_where = format_ident!("{}WhereInput", target_model.name());
    let assign_ident = format_ident!("{}", field.name().to_case(Case::Snake));
    let target_ctx = ctx.for_model(target_model);
    let is_to_many = field.modifier.is_list();

    let DslValue::Block(block) = value else {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!(
                "relation field `{}` expects a `{{ ... }}` block with relation operators",
                field.name()
            ),
        ));
    };

    let mut setters: Vec<TokenStream> = Vec::new();
    for entry in &block.fields {
        let DslField::Pair { key, value, .. } = entry else {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "relation filter does not support spread or conditional fields yet",
            ));
        };
        let op = key.to_string();
        let allowed_ops: &[&str] = if is_to_many {
            &["some", "every", "none"]
        } else {
            &["is", "is_not", "is_null"]
        };
        if !allowed_ops.contains(&op.as_str()) {
            let kind = if is_to_many { "to-many" } else { "to-one" };
            return Err(syn::Error::new(
                key.span(),
                format!(
                    "operator `{op}` is not valid on {kind} relation `{}`. \
                     Allowed: {:?}",
                    field.name(),
                    allowed_ops
                ),
            ));
        }
        let op_ident = format_ident!("{op}");
        if op == "is_null" {
            let DslValue::Bool(b) = value else {
                return Err(syn::Error::new(
                    key.span(),
                    "`is_null` expects `true` or `false`",
                ));
            };
            setters.push(quote! { #op_ident: ::core::option::Option::Some(#b) });
        } else {
            let DslValue::Block(inner) = value else {
                return Err(syn::Error::new(
                    key.span(),
                    format!("`{op}` expects a `{{ ... }}` block describing the related row(s)"),
                ));
            };
            let inner_tokens = lower_where(inner, &target_ctx)?;
            setters.push(quote! { #op_ident: ::core::option::Option::Some(#inner_tokens) });
        }
    }

    if is_to_many {
        Ok(quote! {
            __w.#assign_ident = ::core::option::Option::Some(
                ::prax_query::inputs::ListRelationFilter::<#target_module::#target_where> {
                    #(#setters,)*
                    ..::core::default::Default::default()
                }
            );
        })
    } else {
        Ok(quote! {
            __w.#assign_ident = ::core::option::Option::Some(
                ::prax_query::inputs::SingleRelationFilter::<#target_module::#target_where> {
                    #(#setters,)*
                    ..::core::default::Default::default()
                }
            );
        })
    }
}

fn lower_where_conditional(field: &DslField, ctx: &LowerCtx<'_>) -> syn::Result<TokenStream> {
    let DslField::Conditional {
        cond,
        kind,
        key,
        value,
        ..
    } = field
    else {
        unreachable!("called with non-conditional field");
    };
    let pair_stmt = lower_where_pair(key, value, key.span(), ctx)?;
    use crate::macros::dsl::ast::CondKind;
    Ok(match kind {
        CondKind::If => quote! { if #cond { #pair_stmt } },
        CondKind::ElseIf => quote! { else if #cond { #pair_stmt } },
        CondKind::Else => quote! { else { #pair_stmt } },
    })
}

fn collect_field_names(model: &Model) -> Vec<String> {
    let mut names: Vec<String> = model.fields.keys().map(|k| k.to_string()).collect();
    names.push("and".into());
    names.push("or".into());
    names.push("not".into());
    names
}

/// Sealed trait extension so the `field.name.span` lookup compiles even
/// when the schema's `Span` doesn't expose a `proc_macro2::Span`. Spec
/// `Span` uses byte offsets; we fall back to call-site for diagnostics.
trait IntoProcMacroSpanFallback {
    fn into_proc_macro_span_fallback(self) -> proc_macro2::Span;
}

impl IntoProcMacroSpanFallback for prax_schema::Span {
    fn into_proc_macro_span_fallback(self) -> proc_macro2::Span {
        proc_macro2::Span::call_site()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prax_schema::parse_schema;
    use quote::quote;

    /// Compact view of a TokenStream that strips most whitespace so
    /// snapshots compare on structure, not formatting.
    fn pretty(ts: TokenStream) -> String {
        let raw = ts.to_string();
        // Drop trailing whitespace and collapse adjacent spaces. This
        // is good-enough normalization for snapshot tests.
        raw.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    const SCHEMA: &str = include_str!("../../../tests/fixtures/schema.prax");

    fn parse_block(tokens: TokenStream) -> DslBlock {
        syn::parse2::<DslBlock>(tokens).unwrap()
    }

    fn lower(model_name: &str, tokens: TokenStream) -> TokenStream {
        let schema = parse_schema(SCHEMA).unwrap();
        let model = schema.get_model(model_name).unwrap().clone();
        let ctx = LowerCtx::new(&schema, &model);
        let block = parse_block(tokens);
        lower_where(&block, &ctx).unwrap()
    }

    #[test]
    fn lower_where_simple_scalar_equals() {
        let out = lower("User", quote!({ email: { equals: "alice@x.com" } }));
        let s = pretty(out);
        assert!(s.contains("UserWhereInput"));
        assert!(s.contains("StringFilter"));
        assert!(s.contains("equals"));
    }

    #[test]
    fn lower_where_int_range() {
        let out = lower("User", quote!({ age: { gte: 18, lt: 65 } }));
        let s = pretty(out);
        assert!(s.contains("IntNullableFilter"));
        assert!(s.contains("gte"));
        assert!(s.contains("lt"));
    }

    #[test]
    fn lower_where_logical_or() {
        let out = lower(
            "User",
            quote!({ or: [{ active: true }, { age: { gte: 100 } }] }),
        );
        let s = pretty(out);
        assert!(s.contains(". or ="));
        assert!(s.contains("vec ! ["));
    }

    #[test]
    fn lower_where_relation_to_many_some() {
        let out = lower("User", quote!({ posts: { some: { published: true } } }));
        let s = pretty(out);
        assert!(s.contains("ListRelationFilter"));
        assert!(s.contains("PostWhereInput"));
        assert!(s.contains("some"));
    }

    #[test]
    fn lower_where_relation_to_one_is() {
        let out = lower("User", quote!({ profile: { is_null: true } }));
        let s = pretty(out);
        assert!(s.contains("SingleRelationFilter"));
        assert!(s.contains("ProfileWhereInput"));
        assert!(s.contains("is_null"));
    }

    #[test]
    fn lower_where_unknown_field_errors_with_suggestion() {
        let schema = parse_schema(SCHEMA).unwrap();
        let model = schema.get_model("User").unwrap().clone();
        let ctx = LowerCtx::new(&schema, &model);
        let block = parse_block(quote!({ emial: "x" }));
        let err = lower_where(&block, &ctx).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unknown field"), "got: {msg}");
        assert!(msg.contains("did you mean"), "got: {msg}");
        assert!(msg.contains("email"), "got: {msg}");
    }

    #[test]
    fn lower_where_with_leading_spread() {
        let out = lower("User", quote!({ ..base, email: { equals: "x" } }));
        let s = pretty(out);
        // Initializer uses Clone::clone(&(base)) per the spec.
        assert!(s.contains("Clone :: clone"), "got: {s}");
        assert!(s.contains("base"), "got: {s}");
        assert!(s.contains("email"), "got: {s}");
    }

    #[test]
    fn lower_where_with_move_spread() {
        let out = lower("User", quote!({ ..move base }));
        let s = pretty(out);
        // Move spread elides the clone.
        assert!(!s.contains("Clone :: clone"), "got: {s}");
        assert!(s.contains("base"), "got: {s}");
    }

    #[test]
    fn lower_where_with_if_conditional() {
        // `take` is not a where field; use `active` to exercise the
        // conditional-lowering happy path.
        let out = lower("User", quote!({ #[if(flag)] active: true }));
        let s = pretty(out);
        assert!(s.contains("if flag"), "got: {s}");
        assert!(s.contains("active"), "got: {s}");
    }

    #[test]
    fn lower_where_with_if_else_chain() {
        let out = lower(
            "User",
            quote!({
                #[if(a)] active: true,
                #[else_if(b)] active: false,
                #[else] active: true,
            }),
        );
        let s = pretty(out);
        assert!(s.contains("if a"), "got: {s}");
        assert!(s.contains("else if b"), "got: {s}");
        assert!(s.contains("else"), "got: {s}");
    }

    #[test]
    fn lower_where_some_on_to_one_errors() {
        let schema = parse_schema(SCHEMA).unwrap();
        let model = schema.get_model("User").unwrap().clone();
        let ctx = LowerCtx::new(&schema, &model);
        let block = parse_block(quote!({ profile: { some: { id: 1 } } }));
        let err = lower_where(&block, &ctx).unwrap_err();
        assert!(err.to_string().contains("to-one"));
    }
}
