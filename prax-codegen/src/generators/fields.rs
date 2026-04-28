//! Code generation for field modules.

use proc_macro2::TokenStream;
use quote::quote;

use prax_schema::ast::{Field, FieldType, Model, TypeModifier};

use super::{generate_doc_comment, pascal_ident, snake_ident};
use crate::types::field_type_to_rust;

/// Generate the field module with select, order, and set operations.
pub fn generate_field_module(field: &Field, model: &Model) -> TokenStream {
    let field_name = snake_ident(field.name());
    let field_name_pascal = pascal_ident(field.name());
    let field_type = field_type_to_rust(&field.field_type, &TypeModifier::Required);
    let _full_field_type = field_type_to_rust(&field.field_type, &field.modifier);

    let doc = generate_doc_comment(field.documentation.as_ref().map(|d| d.text.as_str()));

    // Get database column name
    let col_name = field
        .attributes
        .iter()
        .find(|a| a.name() == "map")
        .and_then(|a| a.first_arg())
        .and_then(|v| v.as_string())
        .map(|s| s.to_string())
        .unwrap_or_else(|| field.name().to_string());

    let is_optional = field.modifier.is_optional();
    let is_list = field.modifier.is_list();
    let is_relation = matches!(field.field_type, FieldType::Model(_));

    // Generate order by operations
    let order_by = if !is_list && !is_relation {
        quote! {
            /// Order by this field ascending.
            pub fn asc() -> super::OrderByParam {
                super::OrderByParam::#field_name_pascal(::prax_orm::_prax_prelude::SortOrder::Asc)
            }

            /// Order by this field descending.
            pub fn desc() -> super::OrderByParam {
                super::OrderByParam::#field_name_pascal(::prax_orm::_prax_prelude::SortOrder::Desc)
            }
        }
    } else {
        TokenStream::new()
    };

    // Generate set operations for updates
    let set_ops = if !is_relation {
        let set_type = if is_optional {
            quote! { Option<#field_type> }
        } else {
            field_type.clone()
        };

        quote! {
            /// Set this field to a new value.
            pub fn set(value: #set_type) -> super::SetParam {
                super::SetParam::#field_name_pascal(value)
            }
        }
    } else {
        TokenStream::new()
    };

    // Generate increment/decrement for numeric types
    let numeric_ops = match &field.field_type {
        FieldType::Scalar(s) if crate::types::supports_comparison(s) => {
            if matches!(
                s,
                prax_schema::ast::ScalarType::Int
                    | prax_schema::ast::ScalarType::BigInt
                    | prax_schema::ast::ScalarType::Float
                    | prax_schema::ast::ScalarType::Decimal
            ) {
                quote! {
                    /// Increment this field by the given amount.
                    pub fn increment(amount: #field_type) -> super::SetParam {
                        super::SetParam::#field_name_pascal(super::#field_name::get_current_value() + amount)
                    }

                    /// Decrement this field by the given amount.
                    pub fn decrement(amount: #field_type) -> super::SetParam {
                        super::SetParam::#field_name_pascal(super::#field_name::get_current_value() - amount)
                    }
                }
            } else {
                TokenStream::new()
            }
        }
        _ => TokenStream::new(),
    };

    // Generate filter operations
    let filters = super::filters::generate_field_filters(field, model.name());

    quote! {
        #doc
        pub mod #field_name {
            /// Database column name.
            pub const COLUMN: &str = #col_name;

            /// Whether this field is optional.
            pub const IS_OPTIONAL: bool = #is_optional;

            /// Whether this field is a list.
            pub const IS_LIST: bool = #is_list;

            /// Select this field.
            pub fn select() -> super::SelectParam {
                super::SelectParam::#field_name_pascal
            }

            #order_by
            #set_ops
            #numeric_ops

            // Re-export filter operations
            #filters
        }
    }
}

/// Generate the select param enum for a model.
pub fn generate_select_param(model: &Model) -> TokenStream {
    let variants: Vec<_> = model
        .fields
        .values()
        .filter(|f| !matches!(f.field_type, FieldType::Model(_)))
        .map(|f| {
            let name = pascal_ident(f.name());
            quote! { #name }
        })
        .collect();

    let variant_names: Vec<_> = model
        .fields
        .values()
        .filter(|f| !matches!(f.field_type, FieldType::Model(_)))
        .map(|f| {
            let name = pascal_ident(f.name());
            let col = f
                .attributes
                .iter()
                .find(|a| a.name() == "map")
                .and_then(|a| a.first_arg())
                .and_then(|v| v.as_string())
                .map(|s| s.to_string())
                .unwrap_or_else(|| f.name().to_string());
            (name, col)
        })
        .collect();

    let column_matches: Vec<_> = variant_names
        .iter()
        .map(|(name, col)| {
            quote! { Self::#name => #col }
        })
        .collect();

    quote! {
        /// Fields that can be selected.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum SelectParam {
            #(#variants,)*
        }

        impl SelectParam {
            /// Get the column name for this field.
            pub fn column(&self) -> &'static str {
                match self {
                    #(#column_matches,)*
                }
            }
        }
    }
}

/// Generate the order by param enum for a model.
pub fn generate_order_by_param(model: &Model) -> TokenStream {
    let variants: Vec<_> = model
        .fields
        .values()
        .filter(|f| !f.modifier.is_list() && !matches!(f.field_type, FieldType::Model(_)))
        .map(|f| {
            let name = pascal_ident(f.name());
            quote! { #name(::prax_orm::_prax_prelude::SortOrder) }
        })
        .collect();

    let variant_names: Vec<_> = model
        .fields
        .values()
        .filter(|f| !f.modifier.is_list() && !matches!(f.field_type, FieldType::Model(_)))
        .map(|f| {
            let name = pascal_ident(f.name());
            let col = f
                .attributes
                .iter()
                .find(|a| a.name() == "map")
                .and_then(|a| a.first_arg())
                .and_then(|v| v.as_string())
                .map(|s| s.to_string())
                .unwrap_or_else(|| f.name().to_string());
            (name, col)
        })
        .collect();

    let column_matches: Vec<_> = variant_names
        .iter()
        .map(|(name, col)| {
            quote! { Self::#name(order) => (#col, order) }
        })
        .collect();

    quote! {
        /// Order by parameters.
        #[derive(Debug, Clone, Copy)]
        pub enum OrderByParam {
            #(#variants,)*
        }

        impl OrderByParam {
            /// Get the column name and sort order.
            pub fn column_and_order(&self) -> (&'static str, &::prax_orm::_prax_prelude::SortOrder) {
                match self {
                    #(#column_matches,)*
                }
            }

            /// Generate SQL ORDER BY clause part.
            pub fn to_sql(&self) -> String {
                let (col, order) = self.column_and_order();
                let dir = match order {
                    ::prax_orm::_prax_prelude::SortOrder::Asc => "ASC",
                    ::prax_orm::_prax_prelude::SortOrder::Desc => "DESC",
                };
                format!("{} {}", col, dir)
            }
        }
    }
}

/// Generate the set param enum for updates.
pub fn generate_set_param(model: &Model) -> TokenStream {
    let variants: Vec<_> = model
        .fields
        .values()
        .filter(|f| !matches!(f.field_type, FieldType::Model(_)))
        .map(|f| {
            let name = pascal_ident(f.name());
            let field_type = field_type_to_rust(&f.field_type, &f.modifier);
            quote! { #name(#field_type) }
        })
        .collect();

    let variant_names: Vec<_> = model
        .fields
        .values()
        .filter(|f| !matches!(f.field_type, FieldType::Model(_)))
        .map(|f| {
            let name = pascal_ident(f.name());
            let col = f
                .attributes
                .iter()
                .find(|a| a.name() == "map")
                .and_then(|a| a.first_arg())
                .and_then(|v| v.as_string())
                .map(|s| s.to_string())
                .unwrap_or_else(|| f.name().to_string());
            (name, col)
        })
        .collect();

    let column_matches: Vec<_> = variant_names
        .iter()
        .map(|(name, col)| {
            quote! { Self::#name(_) => #col }
        })
        .collect();

    quote! {
        /// Parameters for setting field values in updates.
        #[derive(Debug, Clone)]
        pub enum SetParam {
            #(#variants,)*
        }

        impl SetParam {
            /// Get the column name for this parameter.
            pub fn column(&self) -> &'static str {
                match self {
                    #(#column_matches,)*
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prax_schema::ast::{Ident, ScalarType, Span};

    fn make_span() -> Span {
        Span::new(0, 0)
    }

    fn make_ident(name: &str) -> Ident {
        Ident::new(name, make_span())
    }

    fn make_model() -> Model {
        let mut model = Model::new(make_ident("User"), make_span());
        model.add_field(Field::new(
            make_ident("id"),
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
            vec![],
            make_span(),
        ));
        model.add_field(Field::new(
            make_ident("name"),
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
            vec![],
            make_span(),
        ));
        model.add_field(Field::new(
            make_ident("email"),
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Optional,
            vec![],
            make_span(),
        ));
        model
    }

    #[test]
    fn test_generate_select_param() {
        let model = make_model();
        let select = generate_select_param(&model);
        let code = select.to_string();

        assert!(code.contains("pub enum SelectParam"));
        assert!(code.contains("Id"));
        assert!(code.contains("Name"));
        assert!(code.contains("Email"));
    }

    #[test]
    fn test_generate_order_by_param() {
        let model = make_model();
        let order_by = generate_order_by_param(&model);
        let code = order_by.to_string();

        assert!(code.contains("pub enum OrderByParam"));
        assert!(code.contains("SortOrder"));
    }

    #[test]
    fn test_generate_set_param() {
        let model = make_model();
        let set = generate_set_param(&model);
        let code = set.to_string();

        assert!(code.contains("pub enum SetParam"));
        assert!(code.contains("Id"));
        assert!(code.contains("Name"));
    }
}
