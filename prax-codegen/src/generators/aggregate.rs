//! Aggregate-macro support: per-model select-shape input structs,
//! result structs, args structs, GroupByColumn enum, and the
//! `aggregate()` / `group_by()` accessor + `with_aggregate_args` /
//! `with_group_by_args` extension methods on AggregateOperation /
//! GroupByOperation. Used by `count!` (extended in phase 6),
//! `aggregate!`, and `group_by!`.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

/// Information about one scalar (non-relation, non-aggregate) field
/// on a model. Built by the caller from the existing FieldInfo loop.
#[allow(dead_code)]
pub struct ScalarFieldMeta<'a> {
    pub ident: &'a syn::Ident,
    pub ty: &'a syn::Type,
    pub column_name: &'a str,
    pub is_numeric: bool,
    pub is_sortable: bool,
}

pub fn rust_type_is_numeric(ty: &syn::Type) -> bool {
    let name = type_leaf_ident(ty);
    matches!(
        name.as_deref(),
        Some(
            "i8" | "i16"
                | "i32"
                | "i64"
                | "i128"
                | "u8"
                | "u16"
                | "u32"
                | "u64"
                | "u128"
                | "f32"
                | "f64"
                | "isize"
                | "usize"
                | "Decimal"
                | "BigDecimal"
        )
    )
}

pub fn rust_type_is_sortable(ty: &syn::Type) -> bool {
    if rust_type_is_numeric(ty) {
        return true;
    }
    let name = type_leaf_ident(ty);
    matches!(
        name.as_deref(),
        Some(
            "String"
                | "str"
                | "DateTime"
                | "NaiveDateTime"
                | "NaiveDate"
                | "NaiveTime"
                | "Date"
                | "Time"
                | "Uuid"
        )
    )
}

fn type_leaf_ident(ty: &syn::Type) -> Option<String> {
    if let syn::Type::Path(tp) = ty
        && let Some(seg) = tp.path.segments.last()
    {
        if seg.ident == "Option"
            && let syn::PathArguments::AngleBracketed(args) = &seg.arguments
            && let Some(syn::GenericArgument::Type(inner)) = args.args.first()
        {
            return type_leaf_ident(inner);
        }
        return Some(seg.ident.to_string());
    }
    None
}

/// Emit the five per-model select-shape input structs.
#[allow(dead_code)]
pub fn emit_select_inputs(
    model_ident: &syn::Ident,
    scalars: &[ScalarFieldMeta<'_>],
) -> TokenStream {
    let count_name = format_ident!("{}CountSelect", model_ident);
    let sum_name = format_ident!("{}SumSelect", model_ident);
    let avg_name = format_ident!("{}AvgSelect", model_ident);
    let min_name = format_ident!("{}MinSelect", model_ident);
    let max_name = format_ident!("{}MaxSelect", model_ident);

    let count_fields = scalars.iter().map(|f| {
        let ident = f.ident;
        quote! { pub #ident: ::core::option::Option<bool> }
    });
    let numeric_fields: Vec<_> = scalars
        .iter()
        .filter(|f| f.is_numeric)
        .map(|f| {
            let ident = f.ident;
            quote! { pub #ident: ::core::option::Option<bool> }
        })
        .collect();
    let sortable_for_min_max: Vec<_> = scalars
        .iter()
        .filter(|f| f.is_sortable)
        .map(|f| {
            let ident = f.ident;
            quote! { pub #ident: ::core::option::Option<bool> }
        })
        .collect();

    let numeric_fields_2 = numeric_fields.to_vec();
    let sortable_2 = sortable_for_min_max.to_vec();

    quote! {
        #[derive(Debug, Default, Clone)]
        pub struct #count_name {
            pub _all: ::core::option::Option<bool>,
            #(#count_fields,)*
        }
        #[derive(Debug, Default, Clone)]
        pub struct #sum_name {
            #(#numeric_fields,)*
        }
        #[derive(Debug, Default, Clone)]
        pub struct #avg_name {
            #(#numeric_fields_2,)*
        }
        #[derive(Debug, Default, Clone)]
        pub struct #min_name {
            #(#sortable_for_min_max,)*
        }
        #[derive(Debug, Default, Clone)]
        pub struct #max_name {
            #(#sortable_2,)*
        }
    }
}

fn is_option_type(ty: &syn::Type) -> bool {
    if let syn::Type::Path(tp) = ty
        && let Some(seg) = tp.path.segments.last()
    {
        return seg.ident == "Option";
    }
    false
}

fn ensure_optional(ty: &syn::Type) -> TokenStream {
    if is_option_type(ty) {
        quote! { #ty }
    } else {
        quote! { ::core::option::Option<#ty> }
    }
}

/// Emit the seven per-model result-shape output structs:
///
/// - `<Model>CountSelectResult` — `i64` per scalar + `_all`.
/// - `<Model>SumResult` / `<Model>AvgResult` — `Option<f64>` per
///   numeric column (Sum widens to f64 for cross-dialect consistency;
///   AggregateResult downcasts via the runtime AggregateResult helpers).
/// - `<Model>MinResult` / `<Model>MaxResult` — `Option<T>` per sortable
///   column.
/// - `<Model>AggregateResult` — five aggregate substructs as
///   `Option<...>`.
/// - `<Model>GroupByResult` — same five substructs plus every scalar
///   column as `Option<T>` (for `by:` columns).
pub fn emit_result_structs(
    model_ident: &syn::Ident,
    scalars: &[ScalarFieldMeta<'_>],
) -> TokenStream {
    let count_result = format_ident!("{}CountSelectResult", model_ident);
    let sum_result = format_ident!("{}SumResult", model_ident);
    let avg_result = format_ident!("{}AvgResult", model_ident);
    let min_result = format_ident!("{}MinResult", model_ident);
    let max_result = format_ident!("{}MaxResult", model_ident);
    let agg_result = format_ident!("{}AggregateResult", model_ident);
    let gb_result = format_ident!("{}GroupByResult", model_ident);

    let count_fields = scalars.iter().map(|f| {
        let ident = f.ident;
        quote! { pub #ident: i64 }
    });

    let sum_fields: Vec<TokenStream> = scalars
        .iter()
        .filter(|f| f.is_numeric)
        .map(|f| {
            let ident = f.ident;
            quote! { pub #ident: ::core::option::Option<f64> }
        })
        .collect();
    let avg_fields = sum_fields.clone();

    let min_fields: Vec<TokenStream> = scalars
        .iter()
        .filter(|f| f.is_sortable)
        .map(|f| {
            let ident = f.ident;
            let outer = ensure_optional(f.ty);
            quote! { pub #ident: #outer }
        })
        .collect();
    let max_fields = min_fields.clone();

    let gb_scalar_fields = scalars.iter().map(|f| {
        let ident = f.ident;
        let outer = ensure_optional(f.ty);
        quote! { pub #ident: #outer }
    });

    quote! {
        #[derive(Debug, Default, Clone)]
        pub struct #count_result {
            pub _all: i64,
            #(#count_fields,)*
        }

        #[derive(Debug, Default, Clone)]
        pub struct #sum_result {
            #(#sum_fields,)*
        }

        #[derive(Debug, Default, Clone)]
        pub struct #avg_result {
            #(#avg_fields,)*
        }

        #[derive(Debug, Default, Clone)]
        pub struct #min_result {
            #(#min_fields,)*
        }

        #[derive(Debug, Default, Clone)]
        pub struct #max_result {
            #(#max_fields,)*
        }

        #[derive(Debug, Default, Clone)]
        pub struct #agg_result {
            pub _sum:   ::core::option::Option<#sum_result>,
            pub _avg:   ::core::option::Option<#avg_result>,
            pub _min:   ::core::option::Option<#min_result>,
            pub _max:   ::core::option::Option<#max_result>,
            pub _count: ::core::option::Option<#count_result>,
        }

        #[derive(Debug, Default, Clone)]
        pub struct #gb_result {
            #(#gb_scalar_fields,)*
            pub _sum:   ::core::option::Option<#sum_result>,
            pub _avg:   ::core::option::Option<#avg_result>,
            pub _min:   ::core::option::Option<#min_result>,
            pub _max:   ::core::option::Option<#max_result>,
            pub _count: ::core::option::Option<#count_result>,
        }
    }
}

pub fn emit_args_and_columns_enum(
    model_ident: &syn::Ident,
    scalars: &[ScalarFieldMeta<'_>],
) -> TokenStream {
    let columns_enum = format_ident!("{}GroupByColumn", model_ident);
    let args_agg = format_ident!("{}AggregateArgs", model_ident);
    let args_gb = format_ident!("{}GroupByArgs", model_ident);
    let where_input = format_ident!("{}WhereInput", model_ident);
    let count_select = format_ident!("{}CountSelect", model_ident);
    let sum_select = format_ident!("{}SumSelect", model_ident);
    let avg_select = format_ident!("{}AvgSelect", model_ident);
    let min_select = format_ident!("{}MinSelect", model_ident);
    let max_select = format_ident!("{}MaxSelect", model_ident);
    let having_ty = format_ident!("{}GroupByHaving", model_ident);
    let order_by_ty = format_ident!("{}GroupByOrderBy", model_ident);

    let variants = scalars.iter().map(|f| {
        let v = format_ident!("{}", to_pascal_case(&f.ident.to_string()));
        quote! { #v }
    });
    let column_arms = scalars.iter().map(|f| {
        let v = format_ident!("{}", to_pascal_case(&f.ident.to_string()));
        let col = f.column_name;
        quote! { Self::#v => #col }
    });

    quote! {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum #columns_enum {
            #(#variants,)*
        }

        impl #columns_enum {
            pub fn column_name(&self) -> &'static str {
                match self {
                    #(#column_arms,)*
                }
            }
        }

        #[derive(Debug, Default, Clone)]
        pub struct #args_agg {
            pub where_input: ::core::option::Option<#where_input>,
            pub _sum:   ::core::option::Option<#sum_select>,
            pub _avg:   ::core::option::Option<#avg_select>,
            pub _min:   ::core::option::Option<#min_select>,
            pub _max:   ::core::option::Option<#max_select>,
            pub _count: ::core::option::Option<#count_select>,
        }

        #[derive(Debug, Default, Clone)]
        pub struct #args_gb {
            pub by:           ::std::vec::Vec<#columns_enum>,
            pub where_input:  ::core::option::Option<#where_input>,
            pub _sum:         ::core::option::Option<#sum_select>,
            pub _avg:         ::core::option::Option<#avg_select>,
            pub _min:         ::core::option::Option<#min_select>,
            pub _max:         ::core::option::Option<#max_select>,
            pub _count:       ::core::option::Option<#count_select>,
            pub having:       ::core::option::Option<#having_ty>,
            pub order_by:     ::core::option::Option<#order_by_ty>,
        }

        #[derive(Debug, Default, Clone)]
        pub struct #having_ty {
            pub conditions: ::std::vec::Vec<::prax_query::operations::HavingCondition>,
        }

        #[derive(Debug, Default, Clone)]
        #[allow(dead_code)]
        pub struct #order_by_ty {
            pub items: ::std::vec::Vec<::std::string::String>,
        }
    }
}

fn to_pascal_case(snake: &str) -> String {
    let mut out = String::with_capacity(snake.len());
    let mut upper = true;
    for c in snake.chars() {
        if c == '_' {
            upper = true;
        } else if upper {
            out.push(c.to_ascii_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn numeric_detects_basic_ints_and_floats() {
        let ty: syn::Type = parse_quote!(i32);
        assert!(rust_type_is_numeric(&ty));
        let ty: syn::Type = parse_quote!(Option<f64>);
        assert!(rust_type_is_numeric(&ty));
        let ty: syn::Type = parse_quote!(u128);
        assert!(rust_type_is_numeric(&ty));
        let ty: syn::Type = parse_quote!(String);
        assert!(!rust_type_is_numeric(&ty));
    }

    #[test]
    fn sortable_includes_string_and_datetime() {
        let ty: syn::Type = parse_quote!(String);
        assert!(rust_type_is_sortable(&ty));
        let ty: syn::Type = parse_quote!(DateTime<Utc>);
        assert!(rust_type_is_sortable(&ty));
        let ty: syn::Type = parse_quote!(Option<NaiveDateTime>);
        assert!(rust_type_is_sortable(&ty));
        let ty: syn::Type = parse_quote!(serde_json::Value);
        assert!(!rust_type_is_sortable(&ty));
    }

    #[test]
    fn emit_result_structs_count_has_i64_for_all_and_each_scalar() {
        let model_ident: syn::Ident = parse_quote!(User);
        let id_ident: syn::Ident = parse_quote!(id);
        let email_ident: syn::Ident = parse_quote!(email);
        let id_ty: syn::Type = parse_quote!(i32);
        let email_ty: syn::Type = parse_quote!(String);
        let scalars = vec![
            ScalarFieldMeta {
                ident: &id_ident,
                ty: &id_ty,
                column_name: "id",
                is_numeric: true,
                is_sortable: true,
            },
            ScalarFieldMeta {
                ident: &email_ident,
                ty: &email_ty,
                column_name: "email",
                is_numeric: false,
                is_sortable: true,
            },
        ];
        let s = emit_result_structs(&model_ident, &scalars).to_string();
        assert!(s.contains("struct UserCountSelectResult"));
        assert!(s.contains("_all : i64"));
        assert!(s.contains("id : i64"));
        assert!(s.contains("email : i64"));
        let sum_idx = s.find("struct UserSumResult").unwrap();
        let close = s[sum_idx..].find('}').unwrap();
        let sum_body = &s[sum_idx..sum_idx + close];
        assert!(sum_body.contains("id :"));
        assert!(
            !sum_body.contains("email :"),
            "Sum body should not include non-numeric `email`: {sum_body}"
        );
    }

    #[test]
    fn emit_result_structs_group_by_includes_every_scalar_as_optional() {
        let model_ident: syn::Ident = parse_quote!(User);
        let team_id_ident: syn::Ident = parse_quote!(team_id);
        let i32_ty: syn::Type = parse_quote!(i32);
        let scalars = vec![ScalarFieldMeta {
            ident: &team_id_ident,
            ty: &i32_ty,
            column_name: "team_id",
            is_numeric: true,
            is_sortable: true,
        }];
        let s = emit_result_structs(&model_ident, &scalars).to_string();
        assert!(s.contains("struct UserGroupByResult"));
        let gb_idx = s.find("struct UserGroupByResult").unwrap();
        let close = s[gb_idx..].find('}').unwrap();
        let gb_body = &s[gb_idx..gb_idx + close];
        assert!(gb_body.contains("team_id :"));
        assert!(
            gb_body.contains("Option < i32 >")
                || gb_body.contains(":: core :: option :: Option < i32 >"),
            "expected Option<i32> in GroupByResult.team_id: {gb_body}"
        );
    }

    #[test]
    fn emit_result_structs_min_max_preserves_existing_option() {
        let model_ident: syn::Ident = parse_quote!(Post);
        let deleted_at_ident: syn::Ident = parse_quote!(deleted_at);
        let opt_ty: syn::Type = parse_quote!(Option<DateTime<Utc>>);
        let scalars = vec![ScalarFieldMeta {
            ident: &deleted_at_ident,
            ty: &opt_ty,
            column_name: "deleted_at",
            is_numeric: false,
            is_sortable: true,
        }];
        let s = emit_result_structs(&model_ident, &scalars).to_string();
        let min_idx = s.find("struct PostMinResult").unwrap();
        let close = s[min_idx..].find('}').unwrap();
        let min_body = &s[min_idx..min_idx + close];
        assert!(
            !min_body.contains("Option < Option <"),
            "must not double-wrap: {min_body}"
        );
        assert!(min_body.contains("Option < DateTime < Utc > >"));
    }

    #[test]
    fn emit_args_and_columns_enum_has_variant_per_scalar_and_column_name_impl() {
        let model_ident: syn::Ident = parse_quote!(User);
        let team_id_ident: syn::Ident = parse_quote!(team_id);
        let region_ident: syn::Ident = parse_quote!(region);
        let i32_ty: syn::Type = parse_quote!(i32);
        let str_ty: syn::Type = parse_quote!(String);
        let scalars = vec![
            ScalarFieldMeta {
                ident: &team_id_ident,
                ty: &i32_ty,
                column_name: "team_id",
                is_numeric: true,
                is_sortable: true,
            },
            ScalarFieldMeta {
                ident: &region_ident,
                ty: &str_ty,
                column_name: "region",
                is_numeric: false,
                is_sortable: true,
            },
        ];
        let s = emit_args_and_columns_enum(&model_ident, &scalars).to_string();
        assert!(s.contains("enum UserGroupByColumn"));
        assert!(s.contains("TeamId"));
        assert!(s.contains("Region"));
        assert!(s.contains("Self :: TeamId => \"team_id\""));
        assert!(s.contains("Self :: Region => \"region\""));
        assert!(s.contains("struct UserAggregateArgs"));
        assert!(s.contains("struct UserGroupByArgs"));
        assert!(s.contains("struct UserGroupByHaving"));
        assert!(s.contains("struct UserGroupByOrderBy"));
        assert!(s.contains("pub by :"));
        assert!(s.contains("Vec < UserGroupByColumn >"));
    }

    #[test]
    fn to_pascal_case_handles_snake_input() {
        assert_eq!(to_pascal_case("team_id"), "TeamId");
        assert_eq!(to_pascal_case("region"), "Region");
        assert_eq!(to_pascal_case("user_account_id"), "UserAccountId");
        assert_eq!(to_pascal_case("a"), "A");
    }

    #[test]
    fn emit_select_inputs_count_includes_all_scalars() {
        let model_ident: syn::Ident = parse_quote!(User);
        let id_ident: syn::Ident = parse_quote!(id);
        let email_ident: syn::Ident = parse_quote!(email);
        let id_ty: syn::Type = parse_quote!(i32);
        let email_ty: syn::Type = parse_quote!(String);
        let scalars = vec![
            ScalarFieldMeta {
                ident: &id_ident,
                ty: &id_ty,
                column_name: "id",
                is_numeric: true,
                is_sortable: true,
            },
            ScalarFieldMeta {
                ident: &email_ident,
                ty: &email_ty,
                column_name: "email",
                is_numeric: false,
                is_sortable: true,
            },
        ];
        let s = emit_select_inputs(&model_ident, &scalars).to_string();
        assert!(s.contains("struct UserCountSelect"));
        assert!(s.contains("pub _all"));
        let sum_idx = s.find("struct UserSumSelect").unwrap();
        let close = s[sum_idx..].find('}').unwrap();
        let sum_body = &s[sum_idx..sum_idx + close];
        assert!(sum_body.contains("pub id :"));
        assert!(!sum_body.contains("pub email :"));
    }
}
