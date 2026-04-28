//! Code generation for Prax models.

use proc_macro2::TokenStream;
use quote::quote;

use prax_schema::ModelStyle;
use prax_schema::ast::{FieldType, Model, Schema, TypeModifier};

use super::fields::{
    generate_field_module, generate_order_by_param, generate_select_param, generate_set_param,
};
use super::{generate_doc_comment, pascal_ident, snake_ident};
use crate::types::field_type_to_rust;

/// Generate the complete module for a model.
///
/// When `model_style` is `GraphQL`, the generated structs will include
/// async-graphql derive macros (`SimpleObject`, `InputObject`).
#[allow(dead_code)]
pub fn generate_model_module(model: &Model, schema: &Schema) -> Result<TokenStream, syn::Error> {
    generate_model_module_with_style(model, schema, ModelStyle::Standard)
}

/// Generate the complete module for a model with a specific style.
pub fn generate_model_module_with_style(
    model: &Model,
    schema: &Schema,
    model_style: ModelStyle,
) -> Result<TokenStream, syn::Error> {
    let model_name = pascal_ident(model.name());
    let module_name = snake_ident(model.name());

    let doc = generate_doc_comment(model.documentation.as_ref().map(|d| d.text.as_str()));

    // Get database table name
    let table_name = model.table_name().to_string();
    let table_name_str = table_name.as_str();

    // Get primary key field(s)
    let pk_fields = get_primary_key_fields(model);
    let pk_field_names: Vec<_> = pk_fields.iter().map(|f| f.as_str()).collect();

    // Generate Data struct fields
    let data_fields: Vec<_> = model
        .fields
        .values()
        .map(|field| {
            let field_name = snake_ident(field.name());
            let field_type = field_type_to_rust(&field.field_type, &field.modifier);
            let field_doc =
                generate_doc_comment(field.documentation.as_ref().map(|d| d.text.as_str()));

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

    // Generate CreateInput fields (excluding auto-generated fields)
    let create_fields: Vec<_> = model
        .fields
        .values()
        .filter(|f| {
            let attrs = f.extract_attributes();
            !attrs.is_auto && !attrs.is_updated_at && !matches!(f.field_type, FieldType::Model(_))
        })
        .map(|field| {
            let field_name = snake_ident(field.name());
            let is_optional =
                field.modifier.is_optional() || field.extract_attributes().default.is_some();
            let base_type = field_type_to_rust(&field.field_type, &TypeModifier::Required);
            let field_type = if is_optional {
                quote! { Option<#base_type> }
            } else {
                base_type
            };

            quote! {
                pub #field_name: #field_type
            }
        })
        .collect();

    // Generate UpdateInput fields (all optional)
    let update_fields: Vec<_> = model
        .fields
        .values()
        .filter(|f| {
            let attrs = f.extract_attributes();
            !attrs.is_auto && !attrs.is_updated_at && !matches!(f.field_type, FieldType::Model(_))
        })
        .map(|field| {
            let field_name = snake_ident(field.name());
            let base_type = field_type_to_rust(&field.field_type, &TypeModifier::Required);

            quote! {
                pub #field_name: Option<#base_type>
            }
        })
        .collect();

    // Generate field modules
    let field_modules: Vec<_> = model
        .fields
        .values()
        .map(|field| generate_field_module(field, model))
        .collect();

    // Generate where param enum
    let where_param = generate_where_param(model);

    // Generate select, order by, and set params
    let select_param = generate_select_param(model);
    let order_by_param = generate_order_by_param(model);
    let set_param = generate_set_param(model);

    // Generate query builder
    let query_builder = generate_query_builder(model, &table_name);

    // Generate pre-compiled SQL constants
    let precompiled_sql = generate_precompiled_sql(model, &table_name);

    // Generate relation helpers
    let relation_helpers = generate_relation_helpers(model, schema);

    // Generate GraphQL derives if model_style is GraphQL
    let model_name_str = model.name();
    let (model_derives, create_input_derives, update_input_derives) = if model_style.is_graphql() {
        (
            quote! {
                #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, async_graphql::SimpleObject)]
                #[graphql(name = #model_name_str)]
            },
            quote! {
                #[derive(Debug, Clone, Default, Serialize, Deserialize, async_graphql::InputObject)]
                #[graphql(name = "CreateInput")]
            },
            quote! {
                #[derive(Debug, Clone, Default, Serialize, Deserialize, async_graphql::InputObject)]
                #[graphql(name = "UpdateInput")]
            },
        )
    } else {
        (
            quote! { #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] },
            quote! { #[derive(Debug, Clone, Default, Serialize, Deserialize)] },
            quote! { #[derive(Debug, Clone, Default, Serialize, Deserialize)] },
        )
    };

    Ok(quote! {
        #doc
        pub mod #module_name {
            use serde::{Deserialize, Serialize};

            /// Database table name.
            pub const TABLE_NAME: &str = #table_name_str;

            /// Primary key column(s).
            pub const PRIMARY_KEY: &[&str] = &[#(#pk_field_names),*];

            #doc
            /// Represents a row from the `#table_name_str` table.
            #model_derives
            pub struct #model_name {
                #(#data_fields,)*
            }

            impl super::_prax_prelude::PraxModel for #model_name {
                const TABLE_NAME: &'static str = TABLE_NAME;
                const PRIMARY_KEY: &'static [&'static str] = PRIMARY_KEY;
            }

            /// Input type for creating a new record.
            #create_input_derives
            pub struct CreateInput {
                #(#create_fields,)*
            }

            /// Input type for updating a record.
            #update_input_derives
            pub struct UpdateInput {
                #(#update_fields,)*
            }

            // Field modules
            #(#field_modules)*

            // Where param enum
            #where_param

            // Select, OrderBy, and Set params
            #select_param
            #order_by_param
            #set_param

            // Query builder
            #query_builder

            // Pre-compiled SQL
            #precompiled_sql

            // Relation helpers
            #relation_helpers
        }

        // Re-export the model type at the parent level
        pub use #module_name::#model_name;
    })
}

/// Get the primary key field names for a model.
fn get_primary_key_fields(model: &Model) -> Vec<String> {
    // Check for composite @@id
    if let Some(attr) = model.attributes.iter().find(|a| a.name() == "id") {
        if let Some(prax_schema::ast::AttributeValue::FieldRefList(fields)) = attr.first_arg() {
            return fields.iter().map(|s| s.to_string()).collect();
        }
    }

    // Otherwise, find @id field
    model
        .fields
        .values()
        .filter(|f| f.is_id())
        .map(|f| f.name().to_string())
        .collect()
}

/// Generate the WhereParam enum for a model.
fn generate_where_param(model: &Model) -> TokenStream {
    let variants: Vec<_> = model
        .fields
        .values()
        .map(|field| {
            let name = pascal_ident(field.name());
            let field_mod = snake_ident(field.name());
            quote! { #name(#field_mod::WhereOp) }
        })
        .collect();

    let to_sql_matches: Vec<_> = model
        .fields
        .values()
        .map(|field| {
            let name = pascal_ident(field.name());
            let field_mod = snake_ident(field.name());
            quote! { Self::#name(op) => #field_mod::COLUMN }
        })
        .collect();

    let from_filter_arms: Vec<_> = model
        .fields
        .values()
        .map(|field| {
            let name = pascal_ident(field.name());
            quote! { WhereParam::#name(op) => op.to_filter(), }
        })
        .collect();

    quote! {
        /// Where clause parameters for filtering queries.
        #[derive(Debug, Clone)]
        pub enum WhereParam {
            #(#variants,)*
            /// Combine with AND.
            And(Vec<WhereParam>),
            /// Combine with OR.
            Or(Vec<WhereParam>),
            /// Negate the condition.
            Not(Box<WhereParam>),
        }

        impl WhereParam {
            /// Get the column name for simple conditions.
            pub fn column(&self) -> Option<&'static str> {
                match self {
                    #(#to_sql_matches,)*
                    Self::And(_) | Self::Or(_) | Self::Not(_) => None,
                }
            }

            /// Combine multiple conditions with AND.
            pub fn and(conditions: Vec<WhereParam>) -> Self {
                Self::And(conditions)
            }

            /// Combine multiple conditions with OR.
            pub fn or(conditions: Vec<WhereParam>) -> Self {
                Self::Or(conditions)
            }

            /// Negate a condition.
            pub fn not(condition: WhereParam) -> Self {
                Self::Not(Box::new(condition))
            }
        }

        impl From<WhereParam> for prax_query::filter::Filter {
            fn from(p: WhereParam) -> Self {
                match p {
                    #(#from_filter_arms)*
                    WhereParam::And(ps) => prax_query::filter::Filter::And(
                        ps.into_iter().map(Into::into).collect::<Vec<_>>().into_boxed_slice()
                    ),
                    WhereParam::Or(ps) => prax_query::filter::Filter::Or(
                        ps.into_iter().map(Into::into).collect::<Vec<_>>().into_boxed_slice()
                    ),
                    WhereParam::Not(p) => prax_query::filter::Filter::Not(Box::new((*p).into())),
                }
            }
        }
    }
}

/// Generate the query builder for a model.
fn generate_query_builder(_model: &Model, _table_name: &str) -> TokenStream {
    quote! {
        /// Query builder for the model.
        #[derive(Debug, Default)]
        pub struct Query {
            /// Select specific fields.
            pub select: Vec<SelectParam>,
            /// Where conditions.
            pub where_: Vec<WhereParam>,
            /// Order by clauses.
            pub order_by: Vec<OrderByParam>,
            /// Skip N records.
            pub skip: Option<usize>,
            /// Take N records.
            pub take: Option<usize>,
            /// Distinct fields.
            pub distinct: Vec<SelectParam>,
        }

        impl Query {
            /// Create a new query builder.
            pub fn new() -> Self {
                Self::default()
            }

            /// Add a where condition.
            pub fn r#where(mut self, param: WhereParam) -> Self {
                self.where_.push(param);
                self
            }

            /// Add multiple where conditions with AND.
            pub fn r#whereand(mut self, params: Vec<WhereParam>) -> Self {
                self.where_.push(WhereParam::And(params));
                self
            }

            /// Add multiple where conditions with OR.
            pub fn r#whereor(mut self, params: Vec<WhereParam>) -> Self {
                self.where_.push(WhereParam::Or(params));
                self
            }

            /// Order by a field.
            pub fn order_by(mut self, param: OrderByParam) -> Self {
                self.order_by.push(param);
                self
            }

            /// Skip N records.
            pub fn skip(mut self, n: usize) -> Self {
                self.skip = Some(n);
                self
            }

            /// Take N records.
            pub fn take(mut self, n: usize) -> Self {
                self.take = Some(n);
                self
            }

            /// Select specific fields.
            pub fn select(mut self, fields: Vec<SelectParam>) -> Self {
                self.select = fields;
                self
            }

            /// Get distinct values.
            pub fn distinct(mut self, fields: Vec<SelectParam>) -> Self {
                self.distinct = fields;
                self
            }

            /// Generate the SELECT SQL query.
            pub fn to_select_sql(&self) -> String {
                let columns = if self.select.is_empty() {
                    "*".to_string()
                } else {
                    self.select.iter().map(|s| s.column()).collect::<Vec<_>>().join(", ")
                };

                let distinct = if self.distinct.is_empty() {
                    String::new()
                } else {
                    format!(
                        "DISTINCT ON ({}) ",
                        self.distinct.iter().map(|d| d.column()).collect::<Vec<_>>().join(", ")
                    )
                };

                let mut sql = format!("SELECT {}{} FROM {}", distinct, columns, TABLE_NAME);

                // WHERE clause would be added here with parameter binding

                if !self.order_by.is_empty() {
                    sql.push_str(" ORDER BY ");
                    sql.push_str(
                        &self.order_by.iter().map(|o| o.to_sql()).collect::<Vec<_>>().join(", ")
                    );
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

        /// Actions available on the model.
        pub struct Actions;

        impl Actions {
            /// Find multiple records.
            pub fn find_many() -> Query {
                Query::new()
            }

            /// Find a unique record (by primary key or unique constraint).
            pub fn find_unique() -> Query {
                Query::new().take(1)
            }

            /// Find the first matching record.
            pub fn find_first() -> Query {
                Query::new().take(1)
            }

            /// Create input for a new record.
            pub fn create() -> CreateInput {
                CreateInput::default()
            }

            /// Update input for a record.
            pub fn update() -> UpdateInput {
                UpdateInput::default()
            }
        }
    }
}

/// Generate pre-compiled SQL constants for common queries.
///
/// This generates `const` SQL strings that can be used directly without
/// any runtime string construction, achieving ~5ns lookup time.
fn generate_precompiled_sql(model: &Model, table_name: &str) -> TokenStream {
    let pk_fields = get_primary_key_fields(model);

    // Generate column list for SELECT (all scalar fields)
    let columns: Vec<_> = model
        .fields
        .values()
        .filter(|f| !matches!(f.field_type, FieldType::Model(_)))
        .map(|f| f.name().to_string())
        .collect();
    let column_list = columns.join(", ");

    // Generate INSERT columns (exclude auto-generated)
    let insert_columns: Vec<_> = model
        .fields
        .values()
        .filter(|f| {
            let attrs = f.extract_attributes();
            !attrs.is_auto && !attrs.is_updated_at && !matches!(f.field_type, FieldType::Model(_))
        })
        .map(|f| f.name().to_string())
        .collect();

    let insert_column_list = insert_columns.join(", ");
    let insert_placeholders: Vec<_> = (1..=insert_columns.len())
        .map(|i| format!("${}", i))
        .collect();
    let insert_placeholder_list = insert_placeholders.join(", ");

    // Generate UPDATE SET clause
    let update_columns: Vec<_> = model
        .fields
        .values()
        .filter(|f| {
            let attrs = f.extract_attributes();
            !attrs.is_auto && !attrs.is_updated_at && !matches!(f.field_type, FieldType::Model(_))
        })
        .enumerate()
        .map(|(i, f)| format!("{} = ${}", f.name(), i + 1))
        .collect();
    let update_set_clause = update_columns.join(", ");
    let update_pk_placeholder = format!("${}", update_columns.len() + 1);

    // Primary key WHERE clause
    let pk_where = if pk_fields.len() == 1 {
        format!("{} = $1", pk_fields[0])
    } else {
        pk_fields
            .iter()
            .enumerate()
            .map(|(i, f)| format!("{} = ${}", f, i + 1))
            .collect::<Vec<_>>()
            .join(" AND ")
    };

    // Generate SQL strings
    let find_all_sql = format!("SELECT {} FROM {}", column_list, table_name);
    let find_by_id_sql = format!(
        "SELECT {} FROM {} WHERE {}",
        column_list, table_name, pk_where
    );
    let count_sql = format!("SELECT COUNT(*) FROM {}", table_name);
    let insert_sql = format!(
        "INSERT INTO {} ({}) VALUES ({}) RETURNING {}",
        table_name, insert_column_list, insert_placeholder_list, column_list
    );
    let insert_no_return_sql = format!(
        "INSERT INTO {} ({}) VALUES ({})",
        table_name, insert_column_list, insert_placeholder_list
    );
    let update_by_id_sql = format!(
        "UPDATE {} SET {} WHERE {} RETURNING {}",
        table_name,
        update_set_clause,
        pk_where.replace("$1", &update_pk_placeholder),
        column_list
    );
    let delete_by_id_sql = format!("DELETE FROM {} WHERE {}", table_name, pk_where);
    let exists_by_id_sql = format!(
        "SELECT EXISTS(SELECT 1 FROM {} WHERE {})",
        table_name, pk_where
    );

    // Generate cache key constants
    let cache_key_prefix = table_name.to_lowercase();
    let cache_key_find_all = format!("{}:find_all", cache_key_prefix);
    let cache_key_find_by_id = format!("{}:find_by_id", cache_key_prefix);
    let cache_key_count = format!("{}:count", cache_key_prefix);
    let cache_key_insert = format!("{}:insert", cache_key_prefix);
    let cache_key_update = format!("{}:update_by_id", cache_key_prefix);
    let cache_key_delete = format!("{}:delete_by_id", cache_key_prefix);

    // Parameter counts for const functions
    let insert_param_count = insert_columns.len();
    let update_param_count = update_columns.len() + 1; // +1 for the primary key

    quote! {
        /// Pre-compiled SQL constants for zero-allocation query building.
        ///
        /// These constants are generated at compile time and provide ~5ns access
        /// compared to runtime string construction.
        ///
        /// # Example
        ///
        /// ```rust,ignore
        /// // Use the const SQL directly
        /// let sql = user::sql::FIND_BY_ID;
        ///
        /// // Or use the typed query functions
        /// let (sql, param_count) = user::sql::find_by_id();
        /// ```
        pub mod sql {
            /// SELECT all columns from the table.
            pub const FIND_ALL: &str = #find_all_sql;

            /// SELECT by primary key.
            pub const FIND_BY_ID: &str = #find_by_id_sql;

            /// COUNT all records.
            pub const COUNT: &str = #count_sql;

            /// INSERT a new record (with RETURNING).
            pub const INSERT: &str = #insert_sql;

            /// INSERT a new record (without RETURNING).
            pub const INSERT_NO_RETURN: &str = #insert_no_return_sql;

            /// UPDATE by primary key (with RETURNING).
            pub const UPDATE_BY_ID: &str = #update_by_id_sql;

            /// DELETE by primary key.
            pub const DELETE_BY_ID: &str = #delete_by_id_sql;

            /// Check if record exists by primary key.
            pub const EXISTS_BY_ID: &str = #exists_by_id_sql;

            /// Cache keys for use with SqlTemplateCache.
            pub mod cache_keys {
                pub const FIND_ALL: &str = #cache_key_find_all;
                pub const FIND_BY_ID: &str = #cache_key_find_by_id;
                pub const COUNT: &str = #cache_key_count;
                pub const INSERT: &str = #cache_key_insert;
                pub const UPDATE_BY_ID: &str = #cache_key_update;
                pub const DELETE_BY_ID: &str = #cache_key_delete;
            }

            /// Get FIND_ALL SQL with parameter count.
            #[inline(always)]
            pub const fn find_all() -> (&'static str, usize) {
                (FIND_ALL, 0)
            }

            /// Get FIND_BY_ID SQL with parameter count.
            #[inline(always)]
            pub const fn find_by_id() -> (&'static str, usize) {
                (FIND_BY_ID, 1)
            }

            /// Get COUNT SQL with parameter count.
            #[inline(always)]
            pub const fn count() -> (&'static str, usize) {
                (COUNT, 0)
            }

            /// Get INSERT SQL with parameter count.
            #[inline(always)]
            pub const fn insert() -> (&'static str, usize) {
                (INSERT, #insert_param_count)
            }

            /// Get UPDATE_BY_ID SQL with parameter count.
            #[inline(always)]
            pub const fn update_by_id() -> (&'static str, usize) {
                (UPDATE_BY_ID, #update_param_count)
            }

            /// Get DELETE_BY_ID SQL with parameter count.
            #[inline(always)]
            pub const fn delete_by_id() -> (&'static str, usize) {
                (DELETE_BY_ID, 1)
            }

            /// Register all SQL templates in the global cache.
            ///
            /// Call this at application startup for fastest cache lookups.
            pub fn register_all_templates() {
                use prax_query::cache::register_global_template;
                register_global_template(cache_keys::FIND_ALL, FIND_ALL);
                register_global_template(cache_keys::FIND_BY_ID, FIND_BY_ID);
                register_global_template(cache_keys::COUNT, COUNT);
                register_global_template(cache_keys::INSERT, INSERT);
                register_global_template(cache_keys::UPDATE_BY_ID, UPDATE_BY_ID);
                register_global_template(cache_keys::DELETE_BY_ID, DELETE_BY_ID);
            }
        }
    }
}

/// Generate relation helper types.
fn generate_relation_helpers(model: &Model, _schema: &Schema) -> TokenStream {
    let relation_fields: Vec<_> = model
        .fields
        .values()
        .filter(|f| matches!(f.field_type, FieldType::Model(_)))
        .collect();

    if relation_fields.is_empty() {
        return TokenStream::new();
    }

    let include_variants: Vec<_> = relation_fields
        .iter()
        .map(|f| {
            let name = pascal_ident(f.name());
            let is_list = f.modifier.is_list();
            if is_list {
                quote! { #name(Option<Box<super::super::#name::Query>>) }
            } else {
                quote! { #name }
            }
        })
        .collect();

    quote! {
        /// Include related records in the query.
        #[derive(Debug, Clone, Default)]
        pub enum IncludeParam {
            #[default]
            None,
            #(#include_variants,)*
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prax_schema::ast::{Attribute, Field, Ident, ScalarType, Span};

    fn make_span() -> Span {
        Span::new(0, 0)
    }

    fn make_ident(name: &str) -> Ident {
        Ident::new(name, make_span())
    }

    fn make_simple_schema() -> Schema {
        let mut schema = Schema::new();
        let mut user = Model::new(make_ident("User"), make_span());
        user.add_field(Field::new(
            make_ident("id"),
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
            vec![
                Attribute::simple(make_ident("id"), make_span()),
                Attribute::simple(make_ident("auto"), make_span()),
            ],
            make_span(),
        ));
        user.add_field(Field::new(
            make_ident("email"),
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
            vec![Attribute::simple(make_ident("unique"), make_span())],
            make_span(),
        ));
        user.add_field(Field::new(
            make_ident("name"),
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Optional,
            vec![],
            make_span(),
        ));
        schema.add_model(user);
        schema
    }

    #[test]
    fn test_generate_model_module() {
        let schema = make_simple_schema();
        let model = schema.get_model("User").unwrap();

        let result = generate_model_module(model, &schema);
        assert!(result.is_ok());

        let code = result.unwrap().to_string();
        assert!(code.contains("pub mod user"));
        assert!(code.contains("pub struct User"));
        assert!(code.contains("pub struct CreateInput"));
        assert!(code.contains("pub struct UpdateInput"));
        assert!(code.contains("pub enum WhereParam"));
        assert!(code.contains("pub struct Query"));
        // Verify pre-compiled SQL module
        assert!(code.contains("pub mod sql"));
        assert!(code.contains("FIND_ALL"));
        assert!(code.contains("FIND_BY_ID"));
        assert!(code.contains("INSERT"));
    }

    #[test]
    fn test_get_primary_key_fields() {
        let schema = make_simple_schema();
        let model = schema.get_model("User").unwrap();

        let pk = get_primary_key_fields(model);
        assert_eq!(pk, vec!["id"]);
    }

    #[test]
    fn test_generate_model_module_graphql_style() {
        let schema = make_simple_schema();
        let model = schema.get_model("User").unwrap();

        let result = generate_model_module_with_style(model, &schema, ModelStyle::GraphQL);
        assert!(result.is_ok());

        let code = result.unwrap().to_string();

        // Verify GraphQL derives are present
        assert!(
            code.contains("async_graphql :: SimpleObject"),
            "Should have SimpleObject derive"
        );
        assert!(
            code.contains("async_graphql :: InputObject"),
            "Should have InputObject derive"
        );

        // Verify graphql name attribute
        assert!(code.contains("graphql"), "Should have graphql attributes");
    }

    #[test]
    fn test_generate_model_module_standard_style() {
        let schema = make_simple_schema();
        let model = schema.get_model("User").unwrap();

        let result = generate_model_module_with_style(model, &schema, ModelStyle::Standard);
        assert!(result.is_ok());

        let code = result.unwrap().to_string();

        // Verify GraphQL derives are NOT present
        assert!(
            !code.contains("async_graphql"),
            "Should NOT have async_graphql derives"
        );
        assert!(
            !code.contains("SimpleObject"),
            "Should NOT have SimpleObject derive"
        );
    }
}
