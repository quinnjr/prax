// AggKind methods and lower_agg_select are consumed by Tasks 8-10; until
// then the compiler sees them as unused.
#![allow(dead_code)]

use convert_case::{Case, Casing};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};

use super::LowerCtx;
use crate::macros::dsl::ast::{DslBlock, DslField, DslValue};

/// Which aggregate operation the caller is lowering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggKind {
    Count,
    Sum,
    Avg,
    Min,
    Max,
}

impl AggKind {
    pub fn select_struct_suffix(self) -> &'static str {
        match self {
            Self::Count => "CountSelect",
            Self::Sum => "SumSelect",
            Self::Avg => "AvgSelect",
            Self::Min => "MinSelect",
            Self::Max => "MaxSelect",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Count => "_count",
            Self::Sum => "_sum",
            Self::Avg => "_avg",
            Self::Min => "_min",
            Self::Max => "_max",
        }
    }
}

/// Lower one `_<agg>: { col: true, ... }` block to an expression that
/// constructs `<Model><Agg>Select { <col>: Some(true), ... }`.
///
/// Validation enforced:
/// - all values must be `true` literals (opt-in only)
/// - `_all` is valid only inside `_count`
/// - every column name must exist on the model (did-you-mean diagnostic)
/// - relation and aggregate fields are rejected
/// - `Sum`/`Avg` require numeric scalar columns (Int / BigInt / Float / Decimal)
/// - empty blocks are rejected
pub fn lower_agg_select(
    kind: AggKind,
    block: &DslBlock,
    ctx: &LowerCtx<'_>,
) -> syn::Result<TokenStream> {
    let module_ident = format_ident!("{}", ctx.model.name().to_case(Case::Snake));
    let struct_ident = format_ident!("{}{}", ctx.model.name(), kind.select_struct_suffix());

    let mut setters: Vec<TokenStream> = Vec::new();

    for entry in &block.fields {
        let DslField::Pair { key, value, .. } = entry else {
            return Err(syn::Error::new(
                Span::call_site(),
                format!(
                    "`{}` block does not support spread or conditional fields",
                    kind.key()
                ),
            ));
        };
        let key_str = key.to_string();

        if !matches!(value, DslValue::Bool(true)) {
            return Err(syn::Error::new(
                key.span(),
                format!(
                    "value for `{}.{}` must be `true` (only opt-in is supported)",
                    kind.key(),
                    key_str
                ),
            ));
        }

        if key_str == "_all" {
            if kind != AggKind::Count {
                return Err(syn::Error::new(
                    key.span(),
                    format!("`_all` is only valid inside `_count`, not `{}`", kind.key()),
                ));
            }
            setters.push(quote! { __s._all = ::core::option::Option::Some(true); });
            continue;
        }

        let field = ctx.model.get_field(&key_str).ok_or_else(|| {
            let candidates: Vec<String> = ctx.model.fields.keys().map(|k| k.to_string()).collect();
            let suggestion = crate::macros::validate::suggest(&key_str, &candidates);
            let msg = match suggestion {
                Some(s) => format!(
                    "unknown column `{}` on model `{}`; did you mean `{}`?",
                    key_str,
                    ctx.model.name(),
                    s
                ),
                None => format!(
                    "unknown column `{}` on model `{}`",
                    key_str,
                    ctx.model.name()
                ),
            };
            syn::Error::new(key.span(), msg)
        })?;

        if field.is_relation() {
            return Err(syn::Error::new(
                key.span(),
                format!(
                    "field `{}` is a relation; aggregates require a scalar column",
                    key_str
                ),
            ));
        }
        if field.aggregate().is_some() {
            return Err(syn::Error::new(
                key.span(),
                format!(
                    "field `{}` is itself an aggregate; cannot aggregate an aggregate",
                    key_str
                ),
            ));
        }

        if matches!(kind, AggKind::Sum | AggKind::Avg) && !field_is_numeric(field) {
            return Err(syn::Error::new(
                key.span(),
                format!(
                    "field `{}` is not numeric; `{}` requires a numeric column (Int, BigInt, Float, or Decimal)",
                    key_str,
                    kind.key()
                ),
            ));
        }

        let col_ident = format_ident!("{}", key_str);
        setters.push(quote! {
            __s.#col_ident = ::core::option::Option::Some(true);
        });
    }

    if setters.is_empty() {
        return Err(syn::Error::new(
            Span::call_site(),
            format!(
                "`{}` block is empty; specify at least one column or remove the block",
                kind.key()
            ),
        ));
    }

    Ok(quote! {
        {
            let mut __s: #module_ident::#struct_ident =
                <#module_ident::#struct_ident as ::core::default::Default>::default();
            #(#setters)*
            __s
        }
    })
}

#[allow(dead_code)]
fn field_is_numeric(field: &prax_schema::Field) -> bool {
    use prax_schema::ast::{FieldType, ScalarType};
    matches!(
        field.field_type,
        FieldType::Scalar(ScalarType::Int)
            | FieldType::Scalar(ScalarType::BigInt)
            | FieldType::Scalar(ScalarType::Float)
            | FieldType::Scalar(ScalarType::Decimal)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use prax_schema::parse_schema;
    use quote::quote;

    const SCHEMA: &str = include_str!("../../../tests/fixtures/schema.prax");

    fn make_ctx(schema: &prax_schema::Schema, model_name: &str) -> prax_schema::Model {
        schema.get_model(model_name).unwrap().clone()
    }

    fn lower_ok(model_name: &str, kind: AggKind, tokens: TokenStream) -> TokenStream {
        let schema = parse_schema(SCHEMA).unwrap();
        let model = make_ctx(&schema, model_name);
        let ctx = LowerCtx::new(&schema, &model);
        let block = syn::parse2::<DslBlock>(tokens).unwrap();
        lower_agg_select(kind, &block, &ctx).unwrap()
    }

    fn lower_err(model_name: &str, kind: AggKind, tokens: TokenStream) -> String {
        let schema = parse_schema(SCHEMA).unwrap();
        let model = make_ctx(&schema, model_name);
        let ctx = LowerCtx::new(&schema, &model);
        let block = syn::parse2::<DslBlock>(tokens).unwrap();
        lower_agg_select(kind, &block, &ctx)
            .unwrap_err()
            .to_string()
    }

    #[test]
    fn agg_kind_select_struct_suffix() {
        assert_eq!(AggKind::Count.select_struct_suffix(), "CountSelect");
        assert_eq!(AggKind::Sum.select_struct_suffix(), "SumSelect");
        assert_eq!(AggKind::Avg.select_struct_suffix(), "AvgSelect");
        assert_eq!(AggKind::Min.select_struct_suffix(), "MinSelect");
        assert_eq!(AggKind::Max.select_struct_suffix(), "MaxSelect");
    }

    #[test]
    fn agg_kind_key() {
        assert_eq!(AggKind::Count.key(), "_count");
        assert_eq!(AggKind::Sum.key(), "_sum");
        assert_eq!(AggKind::Avg.key(), "_avg");
        assert_eq!(AggKind::Min.key(), "_min");
        assert_eq!(AggKind::Max.key(), "_max");
    }

    #[test]
    fn lower_count_scalar_field() {
        let ts = lower_ok("User", AggKind::Count, quote!({ id: true }));
        let s = ts.to_string();
        assert!(s.contains("UserCountSelect"), "got: {s}");
        assert!(s.contains("id"), "got: {s}");
    }

    #[test]
    fn lower_count_all() {
        let ts = lower_ok("User", AggKind::Count, quote!({ _all: true }));
        let s = ts.to_string();
        assert!(s.contains("_all"), "got: {s}");
    }

    #[test]
    fn lower_sum_numeric_field() {
        let ts = lower_ok("User", AggKind::Sum, quote!({ age: true }));
        let s = ts.to_string();
        assert!(s.contains("UserSumSelect"), "got: {s}");
        assert!(s.contains("age"), "got: {s}");
    }

    #[test]
    fn lower_all_in_non_count_errors() {
        let msg = lower_err("User", AggKind::Sum, quote!({ _all: true }));
        assert!(msg.contains("_all"), "got: {msg}");
        assert!(msg.contains("_count"), "got: {msg}");
    }

    #[test]
    fn lower_unknown_column_errors() {
        let msg = lower_err("User", AggKind::Min, quote!({ nonexistent: true }));
        assert!(msg.contains("unknown column"), "got: {msg}");
    }

    #[test]
    fn lower_unknown_column_did_you_mean() {
        let msg = lower_err("User", AggKind::Min, quote!({ emial: true }));
        assert!(msg.contains("did you mean"), "got: {msg}");
    }

    #[test]
    fn lower_non_true_value_errors() {
        let msg = lower_err("User", AggKind::Count, quote!({ id: false }));
        assert!(msg.contains("must be `true`"), "got: {msg}");
    }

    #[test]
    fn lower_relation_field_errors() {
        let msg = lower_err("User", AggKind::Count, quote!({ posts: true }));
        assert!(msg.contains("relation"), "got: {msg}");
    }

    #[test]
    fn lower_aggregate_field_errors() {
        let msg = lower_err("User", AggKind::Count, quote!({ post_count: true }));
        assert!(msg.contains("aggregate"), "got: {msg}");
    }

    #[test]
    fn lower_sum_non_numeric_field_errors() {
        let msg = lower_err("User", AggKind::Sum, quote!({ email: true }));
        assert!(msg.contains("not numeric"), "got: {msg}");
    }

    #[test]
    fn lower_avg_non_numeric_field_errors() {
        let msg = lower_err("User", AggKind::Avg, quote!({ email: true }));
        assert!(msg.contains("not numeric"), "got: {msg}");
    }

    #[test]
    fn lower_empty_block_errors() {
        let msg = lower_err("User", AggKind::Count, quote!({}));
        assert!(msg.contains("empty"), "got: {msg}");
    }
}
