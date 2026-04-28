//! Code generation for filter operations.

use proc_macro2::TokenStream;
use quote::quote;

use prax_schema::ast::{Field, FieldType, ScalarType, TypeModifier};

use super::{pascal_ident, snake_ident};
use crate::types::{field_type_to_rust, supports_comparison, supports_in_op, supports_string_ops};

/// Generate filter operations for a field.
pub fn generate_field_filters(field: &Field, _model_name: &str) -> TokenStream {
    let field_name = snake_ident(field.name());
    let field_type = field_type_to_rust(&field.field_type, &TypeModifier::Required);
    let where_variant = pascal_ident(field.name());

    let col_name = field
        .attributes
        .iter()
        .find(|a| a.name() == "map")
        .and_then(|a| a.first_arg())
        .and_then(|v| v.as_string())
        .map(|s| s.to_string())
        .unwrap_or_else(|| field.name().to_string());

    let is_optional = field.modifier.is_optional();

    // Base equality operations
    let mut ops = vec![
        quote! {
            /// Filter by exact value equality.
            pub fn equals(value: #field_type) -> super::WhereParam {
                super::WhereParam::#where_variant(WhereOp::Equals(value))
            }
        },
        quote! {
            /// Filter by not equal.
            pub fn not(value: #field_type) -> super::WhereParam {
                super::WhereParam::#where_variant(WhereOp::Not(value))
            }
        },
    ];

    // Optional-specific operations
    if is_optional {
        ops.push(quote! {
            /// Filter by null value.
            pub fn is_null() -> super::WhereParam {
                super::WhereParam::#where_variant(WhereOp::IsNull)
            }
        });
        ops.push(quote! {
            /// Filter by not null.
            pub fn is_not_null() -> super::WhereParam {
                super::WhereParam::#where_variant(WhereOp::IsNotNull)
            }
        });
    }

    // In operation
    if supports_in_op(&field.field_type) {
        ops.push(quote! {
            /// Filter by value in list.
            pub fn in_(values: Vec<#field_type>) -> super::WhereParam {
                super::WhereParam::#where_variant(WhereOp::In(values))
            }
        });
        ops.push(quote! {
            /// Filter by value not in list.
            pub fn not_in(values: Vec<#field_type>) -> super::WhereParam {
                super::WhereParam::#where_variant(WhereOp::NotIn(values))
            }
        });
    }

    // Comparison operations for numeric/date types
    if let FieldType::Scalar(scalar) = &field.field_type {
        if supports_comparison(scalar) {
            ops.push(quote! {
                /// Filter by greater than.
                pub fn gt(value: #field_type) -> super::WhereParam {
                    super::WhereParam::#where_variant(WhereOp::Gt(value))
                }
            });
            ops.push(quote! {
                /// Filter by greater than or equal.
                pub fn gte(value: #field_type) -> super::WhereParam {
                    super::WhereParam::#where_variant(WhereOp::Gte(value))
                }
            });
            ops.push(quote! {
                /// Filter by less than.
                pub fn lt(value: #field_type) -> super::WhereParam {
                    super::WhereParam::#where_variant(WhereOp::Lt(value))
                }
            });
            ops.push(quote! {
                /// Filter by less than or equal.
                pub fn lte(value: #field_type) -> super::WhereParam {
                    super::WhereParam::#where_variant(WhereOp::Lte(value))
                }
            });
        }

        // String operations
        if supports_string_ops(scalar) {
            ops.push(quote! {
                /// Filter by substring match.
                pub fn contains(value: impl Into<String>) -> super::WhereParam {
                    super::WhereParam::#where_variant(WhereOp::Contains(value.into()))
                }
            });
            ops.push(quote! {
                /// Filter by prefix match.
                pub fn starts_with(value: impl Into<String>) -> super::WhereParam {
                    super::WhereParam::#where_variant(WhereOp::StartsWith(value.into()))
                }
            });
            ops.push(quote! {
                /// Filter by suffix match.
                pub fn ends_with(value: impl Into<String>) -> super::WhereParam {
                    super::WhereParam::#where_variant(WhereOp::EndsWith(value.into()))
                }
            });
        }
    }

    // Generate the where op enum for this field
    let where_op_variants = generate_where_op_variants(&field.field_type, is_optional);

    // Build to_filter match arms conditionally
    let mut to_filter_arms = vec![
        quote! { Self::Equals(v) => Filter::Equals(col, v.into()), },
        quote! { Self::Not(v) => Filter::NotEquals(col, v.into()), },
    ];

    if is_optional {
        to_filter_arms.push(quote! { Self::IsNull => Filter::IsNull(col), });
        to_filter_arms.push(quote! { Self::IsNotNull => Filter::IsNotNull(col), });
    }

    if supports_in_op(&field.field_type) {
        to_filter_arms.push(quote! {
            Self::In(vs) => Filter::In(col, vs.into_iter().map(Into::into).collect()),
        });
        to_filter_arms.push(quote! {
            Self::NotIn(vs) => Filter::NotIn(col, vs.into_iter().map(Into::into).collect()),
        });
    }

    if let FieldType::Scalar(scalar) = &field.field_type {
        if supports_comparison(scalar) {
            to_filter_arms.push(quote! { Self::Gt(v) => Filter::Gt(col, v.into()), });
            to_filter_arms.push(quote! { Self::Gte(v) => Filter::Gte(col, v.into()), });
            to_filter_arms.push(quote! { Self::Lt(v) => Filter::Lt(col, v.into()), });
            to_filter_arms.push(quote! { Self::Lte(v) => Filter::Lte(col, v.into()), });
        }

        if supports_string_ops(scalar) {
            to_filter_arms.push(quote! {
                Self::Contains(v) => Filter::Contains(col, FilterValue::String(v)),
            });
            to_filter_arms.push(quote! {
                Self::StartsWith(v) => Filter::StartsWith(col, FilterValue::String(v)),
            });
            to_filter_arms.push(quote! {
                Self::EndsWith(v) => Filter::EndsWith(col, FilterValue::String(v)),
            });
        }
    }

    quote! {
        /// Filter operations for the `#col_name` field.
        pub mod #field_name {
            use super::*;

            /// Column name in the database.
            pub const COLUMN: &str = #col_name;

            /// Where operation enum for this field.
            #[derive(Debug, Clone)]
            pub enum WhereOp {
                #where_op_variants
            }

            impl WhereOp {
                /// Convert to SQL condition string with parameter placeholder.
                pub fn to_sql(&self, param_idx: usize) -> String {
                    match self {
                        Self::Equals(_) => format!("{} = ${}", COLUMN, param_idx),
                        Self::Not(_) => format!("{} != ${}", COLUMN, param_idx),
                        Self::IsNull => format!("{} IS NULL", COLUMN),
                        Self::IsNotNull => format!("{} IS NOT NULL", COLUMN),
                        Self::In(v) => {
                            let placeholders: Vec<_> = (0..v.len())
                                .map(|i| format!("${}", param_idx + i))
                                .collect();
                            format!("{} IN ({})", COLUMN, placeholders.join(", "))
                        }
                        Self::NotIn(v) => {
                            let placeholders: Vec<_> = (0..v.len())
                                .map(|i| format!("${}", param_idx + i))
                                .collect();
                            format!("{} NOT IN ({})", COLUMN, placeholders.join(", "))
                        }
                        Self::Gt(_) => format!("{} > ${}", COLUMN, param_idx),
                        Self::Gte(_) => format!("{} >= ${}", COLUMN, param_idx),
                        Self::Lt(_) => format!("{} < ${}", COLUMN, param_idx),
                        Self::Lte(_) => format!("{} <= ${}", COLUMN, param_idx),
                        Self::Contains(_) => format!("{} LIKE '%' || ${} || '%'", COLUMN, param_idx),
                        Self::StartsWith(_) => format!("{} LIKE ${} || '%'", COLUMN, param_idx),
                        Self::EndsWith(_) => format!("{} LIKE '%' || ${}", COLUMN, param_idx),
                    }
                }

                /// Convert to prax_query::filter::Filter.
                pub fn to_filter(self) -> prax_query::filter::Filter {
                    use prax_query::filter::{Filter, FilterValue};
                    use std::borrow::Cow;
                    let col: Cow<'static, str> = Cow::Borrowed(COLUMN);
                    match self {
                        #(#to_filter_arms)*
                    }
                }
            }

            #(#ops)*
        }
    }
}

/// Generate the where op enum variants based on field type.
fn generate_where_op_variants(field_type: &FieldType, is_optional: bool) -> TokenStream {
    let base_type = match field_type {
        FieldType::Scalar(s) => match s {
            ScalarType::Int => quote! { i32 },
            ScalarType::BigInt => quote! { i64 },
            ScalarType::Float => quote! { f64 },
            ScalarType::Decimal => quote! { rust_decimal::Decimal },
            ScalarType::Boolean => quote! { bool },
            ScalarType::String => quote! { String },
            ScalarType::DateTime => quote! { chrono::DateTime<chrono::Utc> },
            ScalarType::Date => quote! { chrono::NaiveDate },
            ScalarType::Time => quote! { chrono::NaiveTime },
            ScalarType::Json => quote! { serde_json::Value },
            ScalarType::Bytes => quote! { Vec<u8> },
            ScalarType::Uuid => quote! { uuid::Uuid },
            // String-based ID types
            ScalarType::Cuid | ScalarType::Cuid2 | ScalarType::NanoId | ScalarType::Ulid => {
                quote! { String }
            }
            // PostgreSQL vector types
            ScalarType::Vector(_) | ScalarType::HalfVector(_) => quote! { Vec<f32> },
            ScalarType::SparseVector(_) => quote! { Vec<(u32, f32)> },
            ScalarType::Bit(_) => quote! { Vec<u8> },
        },
        FieldType::Enum(name) | FieldType::Model(name) | FieldType::Composite(name) => {
            let ident = quote::format_ident!("{}", name.to_string());
            quote! { super::super::#ident }
        }
        FieldType::Unsupported(_) => quote! { String },
    };

    let mut variants = vec![quote! { Equals(#base_type) }, quote! { Not(#base_type) }];

    if is_optional {
        variants.push(quote! { IsNull });
        variants.push(quote! { IsNotNull });
    }

    if supports_in_op(field_type) {
        variants.push(quote! { In(Vec<#base_type>) });
        variants.push(quote! { NotIn(Vec<#base_type>) });
    }

    if let FieldType::Scalar(scalar) = field_type {
        if supports_comparison(scalar) {
            variants.push(quote! { Gt(#base_type) });
            variants.push(quote! { Gte(#base_type) });
            variants.push(quote! { Lt(#base_type) });
            variants.push(quote! { Lte(#base_type) });
        }

        if supports_string_ops(scalar) {
            variants.push(quote! { Contains(String) });
            variants.push(quote! { StartsWith(String) });
            variants.push(quote! { EndsWith(String) });
        }
    }

    quote! {
        #(#variants,)*
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prax_schema::ast::{Ident, Span};

    fn make_span() -> Span {
        Span::new(0, 0)
    }

    fn make_ident(name: &str) -> Ident {
        Ident::new(name, make_span())
    }

    #[test]
    fn test_generate_string_field_filters() {
        let field = Field::new(
            make_ident("name"),
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
            vec![],
            make_span(),
        );

        let filters = generate_field_filters(&field, "User");
        let code = filters.to_string();

        assert!(code.contains("pub fn equals"));
        assert!(code.contains("pub fn not"));
        assert!(code.contains("pub fn contains"));
        assert!(code.contains("pub fn starts_with"));
        assert!(code.contains("pub fn ends_with"));
    }

    #[test]
    fn test_generate_int_field_filters() {
        let field = Field::new(
            make_ident("age"),
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
            vec![],
            make_span(),
        );

        let filters = generate_field_filters(&field, "User");
        let code = filters.to_string();

        assert!(code.contains("pub fn gt"));
        assert!(code.contains("pub fn gte"));
        assert!(code.contains("pub fn lt"));
        assert!(code.contains("pub fn lte"));
    }

    #[test]
    fn test_generate_optional_field_filters() {
        let field = Field::new(
            make_ident("bio"),
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Optional,
            vec![],
            make_span(),
        );

        let filters = generate_field_filters(&field, "User");
        let code = filters.to_string();

        assert!(code.contains("pub fn is_null"));
        assert!(code.contains("pub fn is_not_null"));
    }
}
