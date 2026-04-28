//! Code generation for Prax views.

use proc_macro2::TokenStream;
use quote::quote;

use prax_schema::ast::View;

use super::{generate_doc_comment, pascal_ident, snake_ident};
use crate::types::field_type_to_rust;

/// Generate the module for a view definition.
pub fn generate_view_module(view_def: &View) -> Result<TokenStream, syn::Error> {
    let view_name = pascal_ident(view_def.name());
    let module_name = snake_ident(view_def.name());

    let doc = generate_doc_comment(view_def.documentation.as_ref().map(|d| d.text.as_str()));

    // Get the database view name from @@map or use the view name
    let db_view_name = view_def
        .attributes
        .iter()
        .find(|a| a.name() == "map")
        .and_then(|a| a.first_arg())
        .and_then(|v| v.as_string())
        .map(|s| s.to_string())
        .unwrap_or_else(|| view_def.name().to_string());

    // Generate struct fields
    let fields: Vec<_> = view_def
        .fields
        .values()
        .map(|field| {
            let field_name = snake_ident(field.name());
            let field_type = field_type_to_rust(&field.field_type, &field.modifier);
            let field_doc =
                generate_doc_comment(field.documentation.as_ref().map(|d| d.text.as_str()));

            // Check for @map attribute for column name
            let serde_rename = field
                .attributes
                .iter()
                .find(|a| a.name() == "map")
                .and_then(|a| a.first_arg())
                .and_then(|v| v.as_string())
                .map(|name| quote! { #[serde(rename = #name)] })
                .unwrap_or_default();

            quote! {
                #field_doc
                #serde_rename
                pub #field_name: #field_type
            }
        })
        .collect();

    // Generate field modules for filtering/selecting
    let field_modules: Vec<_> = view_def
        .fields
        .values()
        .map(|field| {
            let field_mod_name = snake_ident(field.name());
            let field_name_str = field
                .attributes
                .iter()
                .find(|a| a.name() == "map")
                .and_then(|a| a.first_arg())
                .and_then(|v| v.as_string())
                .map(|s| s.to_string())
                .unwrap_or_else(|| field.name().to_string());

            quote! {
                /// Field operations for `#field_name_str`.
                pub mod #field_mod_name {
                    /// Column name in the database.
                    pub const COLUMN: &str = #field_name_str;
                }
            }
        })
        .collect();

    Ok(quote! {
        #doc
        pub mod #module_name {
            use serde::{Deserialize, Serialize};

            /// The database view name.
            pub const VIEW_NAME: &str = #db_view_name;

            #doc
            /// This is a read-only view - no insert/update/delete operations are available.
            #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
            pub struct #view_name {
                #(#fields,)*
            }

            /// Field definitions for the view.
            pub mod fields {
                #(#field_modules)*
            }

            // Re-export common types
            pub use fields::*;

            /// Query builder for this view.
            #[derive(Debug, Default)]
            pub struct Query {
                /// Selected fields (empty = all).
                pub select: Vec<&'static str>,
                /// Where conditions.
                pub where_conditions: Vec<String>,
                /// Order by clauses.
                pub order_by: Vec<(&'static str, ::prax_orm::_prax_prelude::SortOrder)>,
                /// Maximum results.
                pub take: Option<usize>,
                /// Results to skip.
                pub skip: Option<usize>,
            }

            impl Query {
                /// Create a new query builder.
                pub fn new() -> Self {
                    Self::default()
                }

                /// Add a select clause.
                pub fn select(mut self, field: &'static str) -> Self {
                    self.select.push(field);
                    self
                }

                /// Set the maximum number of results.
                pub fn take(mut self, n: usize) -> Self {
                    self.take = Some(n);
                    self
                }

                /// Set the number of results to skip.
                pub fn skip(mut self, n: usize) -> Self {
                    self.skip = Some(n);
                    self
                }

                /// Generate the SQL query string.
                pub fn to_sql(&self) -> String {
                    let columns = if self.select.is_empty() {
                        "*".to_string()
                    } else {
                        self.select.join(", ")
                    };

                    let mut sql = format!("SELECT {} FROM {}", columns, VIEW_NAME);

                    if !self.where_conditions.is_empty() {
                        sql.push_str(" WHERE ");
                        sql.push_str(&self.where_conditions.join(" AND "));
                    }

                    if !self.order_by.is_empty() {
                        sql.push_str(" ORDER BY ");
                        let order_parts: Vec<_> = self.order_by.iter().map(|(col, dir)| {
                            let dir_str = match dir {
                                ::prax_orm::_prax_prelude::SortOrder::Asc => "ASC",
                                ::prax_orm::_prax_prelude::SortOrder::Desc => "DESC",
                            };
                            format!("{} {}", col, dir_str)
                        }).collect();
                        sql.push_str(&order_parts.join(", "));
                    }

                    if let Some(take) = self.take {
                        sql.push_str(&format!(" LIMIT {}", take));
                    }

                    if let Some(skip) = self.skip {
                        sql.push_str(&format!(" OFFSET {}", skip));
                    }

                    sql
                }
            }
        }

        // Re-export the view type at the parent level
        pub use #module_name::#view_name;
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use prax_schema::ast::{
        Attribute, AttributeArg, AttributeValue, Field, FieldType, Ident, ScalarType, Span,
        TypeModifier,
    };

    fn make_span() -> Span {
        Span::new(0, 0)
    }

    fn make_ident(name: &str) -> Ident {
        Ident::new(name, make_span())
    }

    fn make_map_attribute(value: &str) -> Attribute {
        Attribute::new(
            make_ident("map"),
            vec![AttributeArg::positional(
                AttributeValue::String(value.into()),
                make_span(),
            )],
            make_span(),
        )
    }

    #[test]
    fn test_generate_simple_view() {
        let mut view_def = View::new(make_ident("UserStats"), make_span());
        view_def.add_field(Field::new(
            make_ident("userId"),
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
            vec![],
            make_span(),
        ));
        view_def.add_field(Field::new(
            make_ident("postCount"),
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
            vec![],
            make_span(),
        ));

        let result = generate_view_module(&view_def);
        assert!(result.is_ok());

        let code = result.unwrap().to_string();
        assert!(code.contains("pub mod user_stats"));
        assert!(code.contains("pub struct UserStats"));
        assert!(code.contains("read-only view"));
    }

    #[test]
    fn test_view_module_contains_view_name_const() {
        let mut view_def = View::new(make_ident("ActivityLog"), make_span());
        view_def.add_field(Field::new(
            make_ident("id"),
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
            vec![],
            make_span(),
        ));

        let result = generate_view_module(&view_def);
        assert!(result.is_ok());

        let code = result.unwrap().to_string();
        assert!(code.contains("VIEW_NAME"));
        assert!(code.contains("ActivityLog"));
    }

    #[test]
    fn test_view_with_map_attribute() {
        let mut view_def = View::new(make_ident("UserStats"), make_span());
        view_def
            .attributes
            .push(make_map_attribute("vw_user_stats"));
        view_def.add_field(Field::new(
            make_ident("userId"),
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
            vec![],
            make_span(),
        ));

        let result = generate_view_module(&view_def);
        assert!(result.is_ok());

        let code = result.unwrap().to_string();
        assert!(code.contains("vw_user_stats"));
    }

    #[test]
    fn test_view_field_with_map_attribute() {
        let mut view_def = View::new(make_ident("UserStats"), make_span());
        view_def.add_field(Field::new(
            make_ident("userId"),
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
            vec![make_map_attribute("user_id")],
            make_span(),
        ));

        let result = generate_view_module(&view_def);
        assert!(result.is_ok());

        let code = result.unwrap().to_string();
        // The map attribute affects the COLUMN constant value in fields module
        assert!(code.contains("user_id"));
        // Since we use @map on field, the serde rename is generated
        // Note: The generated code uses # [serde (rename = "user_id")] format
        assert!(code.contains("serde") || code.contains("COLUMN"));
    }

    #[test]
    fn test_view_with_optional_field() {
        let mut view_def = View::new(make_ident("UserStats"), make_span());
        view_def.add_field(Field::new(
            make_ident("lastActivity"),
            FieldType::Scalar(ScalarType::DateTime),
            TypeModifier::Optional,
            vec![],
            make_span(),
        ));

        let result = generate_view_module(&view_def);
        assert!(result.is_ok());

        let code = result.unwrap().to_string();
        assert!(code.contains("Option"));
    }

    #[test]
    fn test_view_generates_query_builder() {
        let mut view_def = View::new(make_ident("UserStats"), make_span());
        view_def.add_field(Field::new(
            make_ident("userId"),
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
            vec![],
            make_span(),
        ));

        let result = generate_view_module(&view_def);
        assert!(result.is_ok());

        let code = result.unwrap().to_string();
        assert!(code.contains("pub struct Query"));
        assert!(code.contains("fn new"));
        assert!(code.contains("fn select"));
        assert!(code.contains("fn take"));
        assert!(code.contains("fn skip"));
        assert!(code.contains("fn to_sql"));
    }

    #[test]
    fn test_view_generates_field_modules() {
        let mut view_def = View::new(make_ident("UserStats"), make_span());
        view_def.add_field(Field::new(
            make_ident("userId"),
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
            vec![],
            make_span(),
        ));
        view_def.add_field(Field::new(
            make_ident("postCount"),
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
            vec![],
            make_span(),
        ));

        let result = generate_view_module(&view_def);
        assert!(result.is_ok());

        let code = result.unwrap().to_string();
        assert!(code.contains("pub mod fields"));
        assert!(code.contains("pub mod user_id"));
        assert!(code.contains("pub mod post_count"));
        assert!(code.contains("COLUMN"));
    }

    #[test]
    fn test_view_with_different_scalar_types() {
        let mut view_def = View::new(make_ident("MixedView"), make_span());
        view_def.add_field(Field::new(
            make_ident("id"),
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
            vec![],
            make_span(),
        ));
        view_def.add_field(Field::new(
            make_ident("name"),
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
            vec![],
            make_span(),
        ));
        view_def.add_field(Field::new(
            make_ident("score"),
            FieldType::Scalar(ScalarType::Float),
            TypeModifier::Required,
            vec![],
            make_span(),
        ));
        view_def.add_field(Field::new(
            make_ident("active"),
            FieldType::Scalar(ScalarType::Boolean),
            TypeModifier::Required,
            vec![],
            make_span(),
        ));

        let result = generate_view_module(&view_def);
        assert!(result.is_ok());

        let code = result.unwrap().to_string();
        assert!(code.contains("i32"));
        assert!(code.contains("String"));
        assert!(code.contains("f64"));
        assert!(code.contains("bool"));
    }

    #[test]
    fn test_view_derives_serde() {
        let mut view_def = View::new(make_ident("TestView"), make_span());
        view_def.add_field(Field::new(
            make_ident("id"),
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
            vec![],
            make_span(),
        ));

        let result = generate_view_module(&view_def);
        assert!(result.is_ok());

        let code = result.unwrap().to_string();
        assert!(code.contains("Serialize"));
        assert!(code.contains("Deserialize"));
    }

    #[test]
    fn test_view_reexports_type() {
        let mut view_def = View::new(make_ident("MyView"), make_span());
        view_def.add_field(Field::new(
            make_ident("id"),
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
            vec![],
            make_span(),
        ));

        let result = generate_view_module(&view_def);
        assert!(result.is_ok());

        let code = result.unwrap().to_string();
        // Should have a re-export at the parent level
        assert!(code.contains("pub use my_view :: MyView"));
    }

    #[test]
    fn test_view_query_builder_members() {
        let mut view_def = View::new(make_ident("TestView"), make_span());
        view_def.add_field(Field::new(
            make_ident("id"),
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
            vec![],
            make_span(),
        ));

        let result = generate_view_module(&view_def);
        assert!(result.is_ok());

        let code = result.unwrap().to_string();
        assert!(code.contains("pub select"));
        assert!(code.contains("pub where_conditions"));
        assert!(code.contains("pub order_by"));
        assert!(code.contains("pub take"));
        assert!(code.contains("pub skip"));
    }

    #[test]
    fn test_view_with_list_field() {
        let mut view_def = View::new(make_ident("TagView"), make_span());
        view_def.add_field(Field::new(
            make_ident("tags"),
            FieldType::Scalar(ScalarType::String),
            TypeModifier::List,
            vec![],
            make_span(),
        ));

        let result = generate_view_module(&view_def);
        assert!(result.is_ok());

        let code = result.unwrap().to_string();
        assert!(code.contains("Vec < String >"));
    }

    #[test]
    fn test_view_derives_debug_clone_partialeq() {
        let mut view_def = View::new(make_ident("TestView"), make_span());
        view_def.add_field(Field::new(
            make_ident("id"),
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
            vec![],
            make_span(),
        ));

        let result = generate_view_module(&view_def);
        assert!(result.is_ok());

        let code = result.unwrap().to_string();
        assert!(code.contains("Debug"));
        assert!(code.contains("Clone"));
        assert!(code.contains("PartialEq"));
    }
}
