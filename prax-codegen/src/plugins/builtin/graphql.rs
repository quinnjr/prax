//! GraphQL plugin - generates async-graphql compatible type definitions.
//!
//! This plugin generates both GraphQL SDL strings and async-graphql derive macros
//! for seamless integration with the async-graphql ecosystem.
//!
//! # Generated Types
//!
//! For each model, generates:
//! - Output type with `#[derive(SimpleObject)]` or custom `#[Object]` impl
//! - Input type with `#[derive(InputObject)]`
//! - Filter input type for queries
//! - Order input type for sorting
//!
//! For each enum, generates:
//! - `#[derive(Enum)]` compatible enum
//!
//! # Usage
//!
//! Enable with: `PRAX_PLUGIN_GRAPHQL=1`
//!
//! For async-graphql derive macros: `PRAX_PLUGIN_GRAPHQL_ASYNC=1`
//!
//! ```rust,ignore
//! use async_graphql::{Object, InputObject, Enum};
//!
//! // Generated for model User
//! #[derive(SimpleObject)]
//! pub struct User {
//!     pub id: i32,
//!     pub email: String,
//!     #[graphql(skip)] // for @hidden fields
//!     pub internal_id: String,
//!     #[graphql(deprecation = "Use newEmail")] // for @deprecated fields
//!     pub old_email: Option<String>,
//! }
//!
//! // Input type for mutations
//! #[derive(InputObject)]
//! pub struct UserInput {
//!     pub email: String,
//!     // @readonly and @hidden fields excluded
//! }
//!
//! // Generated for enum Role
//! #[derive(Enum, Copy, Clone, Eq, PartialEq)]
//! pub enum Role {
//!     User,
//!     Admin,
//! }
//! ```

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use prax_schema::ast::{Enum, FieldType, Model, ScalarType, TypeModifier, View};

use crate::plugins::{Plugin, PluginContext, PluginOutput};

/// GraphQL plugin that generates async-graphql compatible type definitions.
///
/// When enabled, this plugin generates:
/// - GraphQL SDL strings for schema introspection
/// - async-graphql derive macros for runtime use
/// - Input/Output type variants
/// - Filter and ordering types
///
/// Enable with: `PRAX_PLUGIN_GRAPHQL=1`
/// Enable async-graphql derives: `PRAX_PLUGIN_GRAPHQL_ASYNC=1`
pub struct GraphQLPlugin;

impl Plugin for GraphQLPlugin {
    fn name(&self) -> &'static str {
        "graphql"
    }

    fn env_var(&self) -> &'static str {
        "PRAX_PLUGIN_GRAPHQL"
    }

    fn description(&self) -> &'static str {
        "Generates async-graphql compatible type definitions for models and enums"
    }

    fn on_model(&self, ctx: &PluginContext, model: &Model) -> PluginOutput {
        let model_name = model.name();
        let model_ident = format_ident!("{}", model_name);
        let input_ident = format_ident!("{}Input", model_name);
        let create_input_ident = format_ident!("{}CreateInput", model_name);
        let update_input_ident = format_ident!("{}UpdateInput", model_name);
        let filter_ident = format_ident!("{}Filter", model_name);
        let order_ident = format_ident!("{}OrderBy", model_name);

        // Generate field definitions for SDL
        let mut sdl_fields: Vec<String> = Vec::new();
        let mut output_fields: Vec<TokenStream> = Vec::new();
        let mut input_fields: Vec<TokenStream> = Vec::new();
        let mut create_fields: Vec<TokenStream> = Vec::new();
        let mut update_fields: Vec<TokenStream> = Vec::new();
        let mut filter_fields: Vec<TokenStream> = Vec::new();
        let mut field_names: Vec<String> = Vec::new();

        for field in model.fields.values() {
            let field_name = field.name();
            let field_ident = format_ident!("{}", field_name);
            field_names.push(field_name.to_string());

            // Get GraphQL type
            let gql_type = field_type_to_graphql(&field.field_type, &field.modifier);
            let rust_type = field_type_to_rust(&field.field_type, &field.modifier);

            // Extract metadata from field
            let meta = if let Some(doc) = &field.documentation {
                let enhanced = prax_schema::ast::EnhancedDocumentation::parse(&doc.text, doc.span);
                enhanced.extract_metadata()
            } else {
                prax_schema::ast::FieldMetadata::new()
            };

            // SDL field (skip hidden)
            if !meta.hidden {
                let mut sdl_field = format!("  {}: {}", field_name, gql_type);
                if meta.is_deprecated() {
                    if let Some(msg) = meta.deprecation_message() {
                        sdl_field.push_str(&format!(" @deprecated(reason: \"{}\")", msg));
                    } else {
                        sdl_field.push_str(" @deprecated");
                    }
                }
                sdl_fields.push(sdl_field);
            }

            // Build graphql attributes
            let mut gql_attrs: Vec<TokenStream> = Vec::new();

            // Hidden/skip
            if meta.hidden || meta.internal {
                gql_attrs.push(quote! { skip });
            }

            // Deprecation
            if let Some(deprecation) = &meta.deprecated {
                let msg = &deprecation.message;
                if msg.is_empty() {
                    gql_attrs.push(quote! { deprecation });
                } else {
                    gql_attrs.push(quote! { deprecation = #msg });
                }
            }

            // Custom name
            if let Some(alias) = &meta.alias {
                gql_attrs.push(quote! { name = #alias });
            }

            // Description
            let description = meta
                .description
                .as_ref()
                .or(field.documentation.as_ref().map(|d| &d.text));
            if let Some(desc) = description {
                gql_attrs.push(quote! { desc = #desc });
            }

            let gql_attr = if gql_attrs.is_empty() {
                quote! {}
            } else {
                quote! { #[graphql(#(#gql_attrs),*)] }
            };

            // Output field (for queries)
            if !meta.writeonly && !meta.hidden {
                output_fields.push(quote! {
                    #gql_attr
                    pub #field_ident: #rust_type,
                });
            }

            // Input field (for mutations) - skip readonly, output_only, and hidden
            if !meta.readonly && !meta.output_only && !meta.hidden && !meta.omit_from_input {
                let input_type = make_optional_type(&rust_type);
                input_fields.push(quote! {
                    pub #field_ident: #input_type,
                });
            }

            // Create input field - required fields stay required
            if !meta.readonly && !meta.output_only && !meta.hidden && !meta.omit_from_input {
                if field.is_id() {
                    // Skip auto-generated IDs in create
                    if !field.has_attribute("auto") {
                        create_fields.push(quote! {
                            pub #field_ident: #rust_type,
                        });
                    }
                } else {
                    create_fields.push(quote! {
                        pub #field_ident: #rust_type,
                    });
                }
            }

            // Update input field - all optional
            if !meta.readonly
                && !meta.output_only
                && !meta.hidden
                && !meta.omit_from_input
                && !field.is_id()
            {
                let optional_type = make_optional_type(&rust_type);
                update_fields.push(quote! {
                    pub #field_ident: #optional_type,
                });
            }

            // Filter fields for scalar types
            if !meta.sensitive
                && !meta.hidden
                && let FieldType::Scalar(scalar) = &field.field_type
            {
                let filter_type = scalar_to_filter_type(scalar);
                if let Some(ft) = filter_type {
                    filter_fields.push(quote! {
                        pub #field_ident: Option<#ft>,
                    });
                }
            }
        }

        let fields_str = sdl_fields.join("\n");
        let sdl = format!("type {} {{\n{}\n}}", model_name, fields_str);
        let input_sdl = generate_input_sdl(model);

        // Check if async-graphql mode is enabled
        let async_graphql_enabled = ctx.config.is_enabled("graphql_async");

        let async_graphql_derives = if async_graphql_enabled {
            quote! {
                /// Output type for GraphQL queries.
                #[derive(async_graphql::SimpleObject, Clone, Debug)]
                #[graphql(name = #model_name)]
                pub struct #model_ident {
                    #(#output_fields)*
                }

                /// Input type for GraphQL mutations.
                #[derive(async_graphql::InputObject, Clone, Debug, Default)]
                #[graphql(name = #input_ident)]
                pub struct #input_ident {
                    #(#input_fields)*
                }

                /// Create input type for GraphQL mutations.
                #[derive(async_graphql::InputObject, Clone, Debug)]
                #[graphql(name = #create_input_ident)]
                pub struct #create_input_ident {
                    #(#create_fields)*
                }

                /// Update input type for GraphQL mutations.
                #[derive(async_graphql::InputObject, Clone, Debug, Default)]
                #[graphql(name = #update_input_ident)]
                pub struct #update_input_ident {
                    #(#update_fields)*
                }

                /// Filter input type for GraphQL queries.
                #[derive(async_graphql::InputObject, Clone, Debug, Default)]
                pub struct #filter_ident {
                    #(#filter_fields)*
                    /// Logical AND of filters.
                    pub and: Option<Vec<#filter_ident>>,
                    /// Logical OR of filters.
                    pub or: Option<Vec<#filter_ident>>,
                    /// Logical NOT of filter.
                    pub not: Option<Box<#filter_ident>>,
                }

                /// Order by input type for GraphQL queries.
                #[derive(async_graphql::InputObject, Clone, Debug, Default)]
                pub struct #order_ident {
                    #(pub #field_names: Option<SortOrder>,)*
                }
            }
        } else {
            quote! {}
        };

        let field_name_strs: Vec<&str> = field_names.iter().map(|s| s.as_str()).collect();

        PluginOutput::with_tokens(quote! {
            /// GraphQL type definitions for this model.
            pub mod _graphql {
                use super::*;

                /// Get the GraphQL SDL for this type.
                pub const SDL: &str = #sdl;

                /// Get the GraphQL input type SDL.
                pub const INPUT_SDL: &str = #input_sdl;

                /// Get the GraphQL type name.
                pub const TYPE_NAME: &str = #model_name;

                /// Get the GraphQL input type name.
                pub const INPUT_TYPE_NAME: &str = concat!(#model_name, "Input");

                /// Get field names for the GraphQL type.
                pub fn field_names() -> Vec<&'static str> {
                    vec![#(#field_name_strs),*]
                }

                #async_graphql_derives
            }
        })
    }

    fn on_enum(&self, ctx: &PluginContext, enum_def: &Enum) -> PluginOutput {
        let enum_name = enum_def.name();
        let enum_ident = format_ident!("{}", enum_name);

        let variants: Vec<String> = enum_def
            .variants
            .iter()
            .map(|v| format!("  {}", v.name()))
            .collect();

        let variants_str = variants.join("\n");
        let sdl = format!("enum {} {{\n{}\n}}", enum_name, variants_str);

        // Generate variant idents for async-graphql
        let variant_idents: Vec<_> = enum_def
            .variants
            .iter()
            .map(|v| format_ident!("{}", v.name()))
            .collect();

        let variant_names: Vec<_> = enum_def.variants.iter().map(|v| v.name()).collect();

        let async_graphql_enabled = ctx.config.is_enabled("graphql_async");

        let async_graphql_enum = if async_graphql_enabled {
            quote! {
                /// GraphQL enum type.
                #[derive(async_graphql::Enum, Copy, Clone, Eq, PartialEq, Debug)]
                #[graphql(name = #enum_name)]
                pub enum #enum_ident {
                    #(
                        #[graphql(name = #variant_names)]
                        #variant_idents,
                    )*
                }
            }
        } else {
            quote! {}
        };

        PluginOutput::with_tokens(quote! {
            /// GraphQL enum definition.
            pub mod _graphql {
                use super::*;

                /// Get the GraphQL SDL for this enum.
                pub const SDL: &str = #sdl;

                /// Get the GraphQL type name.
                pub const TYPE_NAME: &str = #enum_name;

                /// Get variant names.
                pub fn variant_names() -> Vec<&'static str> {
                    vec![#(#variant_names),*]
                }

                #async_graphql_enum
            }
        })
    }

    fn on_view(&self, ctx: &PluginContext, view: &View) -> PluginOutput {
        let view_name = view.name();
        let view_ident = format_ident!("{}", view_name);

        // Views are read-only, so only output type
        let mut sdl_fields: Vec<String> = Vec::new();
        let mut output_fields: Vec<TokenStream> = Vec::new();
        let mut field_names: Vec<String> = Vec::new();

        for field in view.fields.values() {
            let field_name = field.name();
            let field_ident = format_ident!("{}", field_name);
            field_names.push(field_name.to_string());

            let gql_type = field_type_to_graphql(&field.field_type, &field.modifier);
            let rust_type = field_type_to_rust(&field.field_type, &field.modifier);

            sdl_fields.push(format!("  {}: {}", field_name, gql_type));
            output_fields.push(quote! {
                pub #field_ident: #rust_type,
            });
        }

        let fields_str = sdl_fields.join("\n");
        let sdl = format!("type {} {{\n{}\n}}", view_name, fields_str);

        let async_graphql_enabled = ctx.config.is_enabled("graphql_async");

        let async_graphql_view = if async_graphql_enabled {
            quote! {
                /// GraphQL view type (read-only).
                #[derive(async_graphql::SimpleObject, Clone, Debug)]
                #[graphql(name = #view_name)]
                pub struct #view_ident {
                    #(#output_fields)*
                }
            }
        } else {
            quote! {}
        };

        let field_name_strs: Vec<&str> = field_names.iter().map(|s| s.as_str()).collect();

        PluginOutput::with_tokens(quote! {
            /// GraphQL view definition (read-only).
            pub mod _graphql {
                use super::*;

                /// Get the GraphQL SDL for this view.
                pub const SDL: &str = #sdl;

                /// Get the GraphQL type name.
                pub const TYPE_NAME: &str = #view_name;

                /// Get field names.
                pub fn field_names() -> Vec<&'static str> {
                    vec![#(#field_name_strs),*]
                }

                #async_graphql_view
            }
        })
    }

    fn on_finish(&self, ctx: &PluginContext) -> PluginOutput {
        let mut all_sdl_parts = Vec::new();

        // Collect all model SDLs
        for model in ctx.schema.models.values() {
            let model_name = model.name();
            let fields: Vec<String> = model
                .fields
                .values()
                .filter_map(|field| {
                    // Extract metadata and skip hidden fields
                    let meta = if let Some(doc) = &field.documentation {
                        let enhanced =
                            prax_schema::ast::EnhancedDocumentation::parse(&doc.text, doc.span);
                        enhanced.extract_metadata()
                    } else {
                        prax_schema::ast::FieldMetadata::new()
                    };

                    if meta.hidden {
                        return None;
                    }

                    let field_name = field.name();
                    let gql_type = field_type_to_graphql(&field.field_type, &field.modifier);
                    let mut sdl_field = format!("  {}: {}", field_name, gql_type);

                    if meta.is_deprecated() {
                        if let Some(msg) = meta.deprecation_message() {
                            sdl_field.push_str(&format!(" @deprecated(reason: \"{}\")", msg));
                        } else {
                            sdl_field.push_str(" @deprecated");
                        }
                    }

                    Some(sdl_field)
                })
                .collect();
            let fields_str = fields.join("\n");
            all_sdl_parts.push(format!("type {} {{\n{}\n}}", model_name, fields_str));
        }

        // Collect all enum SDLs
        for enum_def in ctx.schema.enums.values() {
            let enum_name = enum_def.name();
            let variants: Vec<String> = enum_def
                .variants
                .iter()
                .map(|v| format!("  {}", v.name()))
                .collect();
            let variants_str = variants.join("\n");
            all_sdl_parts.push(format!("enum {} {{\n{}\n}}", enum_name, variants_str));
        }

        // Collect view SDLs
        for view in ctx.schema.views.values() {
            let view_name = view.name();
            let fields: Vec<String> = view
                .fields
                .values()
                .map(|field| {
                    let field_name = field.name();
                    let gql_type = field_type_to_graphql(&field.field_type, &field.modifier);
                    format!("  {}: {}", field_name, gql_type)
                })
                .collect();
            let fields_str = fields.join("\n");
            all_sdl_parts.push(format!("type {} {{\n{}\n}}", view_name, fields_str));
        }

        let full_sdl = all_sdl_parts.join("\n\n");
        let model_count = ctx.schema.models.len();
        let enum_count = ctx.schema.enums.len();
        let view_count = ctx.schema.views.len();

        let async_graphql_enabled = ctx.config.is_enabled("graphql_async");

        let common_types = if async_graphql_enabled {
            quote! {
                /// Sort order for ordering results.
                #[derive(async_graphql::Enum, Copy, Clone, Eq, PartialEq, Debug, Default)]
                pub enum SortOrder {
                    /// Ascending order.
                    #[default]
                    Asc,
                    /// Descending order.
                    Desc,
                }

                /// Null handling for sorting.
                #[derive(async_graphql::Enum, Copy, Clone, Eq, PartialEq, Debug)]
                pub enum NullsOrder {
                    /// Nulls first.
                    First,
                    /// Nulls last.
                    Last,
                }

                /// String filter operations.
                #[derive(async_graphql::InputObject, Clone, Debug, Default)]
                pub struct StringFilter {
                    pub equals: Option<String>,
                    pub not_equals: Option<String>,
                    pub contains: Option<String>,
                    pub starts_with: Option<String>,
                    pub ends_with: Option<String>,
                    pub r#in: Option<Vec<String>>,
                    pub not_in: Option<Vec<String>>,
                    pub is_null: Option<bool>,
                }

                /// Int filter operations.
                #[derive(async_graphql::InputObject, Clone, Debug, Default)]
                pub struct IntFilter {
                    pub equals: Option<i32>,
                    pub not_equals: Option<i32>,
                    pub lt: Option<i32>,
                    pub lte: Option<i32>,
                    pub gt: Option<i32>,
                    pub gte: Option<i32>,
                    pub r#in: Option<Vec<i32>>,
                    pub not_in: Option<Vec<i32>>,
                    pub is_null: Option<bool>,
                }

                /// Float filter operations.
                #[derive(async_graphql::InputObject, Clone, Debug, Default)]
                pub struct FloatFilter {
                    pub equals: Option<f64>,
                    pub not_equals: Option<f64>,
                    pub lt: Option<f64>,
                    pub lte: Option<f64>,
                    pub gt: Option<f64>,
                    pub gte: Option<f64>,
                    pub is_null: Option<bool>,
                }

                /// Boolean filter operations.
                #[derive(async_graphql::InputObject, Clone, Debug, Default)]
                pub struct BooleanFilter {
                    pub equals: Option<bool>,
                    pub not_equals: Option<bool>,
                    pub is_null: Option<bool>,
                }

                /// DateTime filter operations.
                #[derive(async_graphql::InputObject, Clone, Debug, Default)]
                pub struct DateTimeFilter {
                    pub equals: Option<String>,
                    pub not_equals: Option<String>,
                    pub lt: Option<String>,
                    pub lte: Option<String>,
                    pub gt: Option<String>,
                    pub gte: Option<String>,
                    pub is_null: Option<bool>,
                }

                /// ID filter operations.
                #[derive(async_graphql::InputObject, Clone, Debug, Default)]
                pub struct IdFilter {
                    pub equals: Option<async_graphql::ID>,
                    pub not_equals: Option<async_graphql::ID>,
                    pub r#in: Option<Vec<async_graphql::ID>>,
                    pub not_in: Option<Vec<async_graphql::ID>>,
                    pub is_null: Option<bool>,
                }
            }
        } else {
            quote! {
                /// Sort order for ordering results.
                #[derive(Copy, Clone, Eq, PartialEq, Debug, Default)]
                pub enum SortOrder {
                    /// Ascending order.
                    #[default]
                    Asc,
                    /// Descending order.
                    Desc,
                }

                /// Null handling for sorting.
                #[derive(Copy, Clone, Eq, PartialEq, Debug)]
                pub enum NullsOrder {
                    /// Nulls first.
                    First,
                    /// Nulls last.
                    Last,
                }
            }
        };

        PluginOutput::with_tokens(quote! {
            /// Combined GraphQL schema for all types.
            pub mod _graphql_schema {
                /// The complete GraphQL SDL for the schema.
                pub const FULL_SDL: &str = #full_sdl;

                /// Number of models in the schema.
                pub const MODEL_COUNT: usize = #model_count;

                /// Number of enums in the schema.
                pub const ENUM_COUNT: usize = #enum_count;

                /// Number of views in the schema.
                pub const VIEW_COUNT: usize = #view_count;

                /// Print the full GraphQL schema.
                pub fn print_schema() {
                    println!("{}", FULL_SDL);
                }

                /// Get the SDL string.
                pub fn sdl() -> &'static str {
                    FULL_SDL
                }

                #common_types
            }
        })
    }
}

/// Convert a Prax field type to GraphQL type string.
fn field_type_to_graphql(field_type: &FieldType, modifier: &TypeModifier) -> String {
    let base_type = match field_type {
        FieldType::Scalar(scalar) => match scalar {
            ScalarType::Int => "Int",
            ScalarType::BigInt => "BigInt",
            ScalarType::Float | ScalarType::Decimal => "Float",
            ScalarType::Boolean => "Boolean",
            ScalarType::String => "String",
            ScalarType::DateTime | ScalarType::Date | ScalarType::Time => "DateTime",
            ScalarType::Json => "JSON",
            ScalarType::Bytes => "String",
            ScalarType::Uuid => "ID",
            // String-based ID types are represented as ID in GraphQL
            ScalarType::Cuid | ScalarType::Cuid2 | ScalarType::NanoId | ScalarType::Ulid => "ID",
            // Vector types are represented as [Float!] in GraphQL
            ScalarType::Vector(_) | ScalarType::HalfVector(_) => "[Float!]",
            ScalarType::SparseVector(_) => "[[Float!]!]", // Array of [index, value] pairs
            ScalarType::Bit(_) => "[Int!]",               // Bit array as integers
        },
        FieldType::Enum(name) | FieldType::Model(name) | FieldType::Composite(name) => {
            return format_graphql_type(name, modifier);
        }
        FieldType::Unsupported(_) => "String",
    };

    format_graphql_type(base_type, modifier)
}

/// Format a GraphQL type with modifiers.
fn format_graphql_type(base: &str, modifier: &TypeModifier) -> String {
    match modifier {
        TypeModifier::Required => format!("{}!", base),
        TypeModifier::Optional => base.to_string(),
        TypeModifier::List => format!("[{}!]!", base),
        TypeModifier::OptionalList => format!("[{}!]", base),
    }
}

/// Convert a Prax field type to Rust type for code generation.
fn field_type_to_rust(field_type: &FieldType, modifier: &TypeModifier) -> TokenStream {
    let base_type = match field_type {
        FieldType::Scalar(scalar) => match scalar {
            ScalarType::Int => quote! { i32 },
            ScalarType::BigInt => quote! { i64 },
            ScalarType::Float => quote! { f64 },
            ScalarType::Decimal => quote! { f64 },
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
            let ident = format_ident!("{}", name.as_str());
            quote! { #ident }
        }
        FieldType::Unsupported(_) => quote! { String },
    };

    match modifier {
        TypeModifier::Required => base_type,
        TypeModifier::Optional => quote! { Option<#base_type> },
        TypeModifier::List => quote! { Vec<#base_type> },
        TypeModifier::OptionalList => quote! { Option<Vec<#base_type>> },
    }
}

/// Make a type optional if it isn't already.
fn make_optional_type(rust_type: &TokenStream) -> TokenStream {
    let type_str = rust_type.to_string();
    if type_str.starts_with("Option <") || type_str.starts_with("Option<") {
        rust_type.clone()
    } else {
        quote! { Option<#rust_type> }
    }
}

/// Get the filter type for a scalar type.
fn scalar_to_filter_type(scalar: &ScalarType) -> Option<TokenStream> {
    match scalar {
        ScalarType::Int | ScalarType::BigInt => Some(quote! { IntFilter }),
        ScalarType::Float | ScalarType::Decimal => Some(quote! { FloatFilter }),
        ScalarType::Boolean => Some(quote! { BooleanFilter }),
        ScalarType::String => Some(quote! { StringFilter }),
        ScalarType::DateTime | ScalarType::Date | ScalarType::Time => {
            Some(quote! { DateTimeFilter })
        }
        // ID types use IdFilter
        ScalarType::Uuid
        | ScalarType::Cuid
        | ScalarType::Cuid2
        | ScalarType::NanoId
        | ScalarType::Ulid => Some(quote! { IdFilter }),
        // Vector types don't have standard filters (use similarity search instead)
        ScalarType::Vector(_)
        | ScalarType::HalfVector(_)
        | ScalarType::SparseVector(_)
        | ScalarType::Bit(_) => None,
        ScalarType::Json | ScalarType::Bytes => None,
    }
}

/// Generate input SDL for a model.
fn generate_input_sdl(model: &Model) -> String {
    let model_name = model.name();
    let input_fields: Vec<String> = model
        .fields
        .values()
        .filter(|f| !f.is_id() || !f.has_attribute("auto"))
        .map(|field| {
            let field_name = field.name();
            let gql_type = field_type_to_graphql(&field.field_type, &field.modifier);
            format!("  {}: {}", field_name, gql_type)
        })
        .collect();

    let fields_str = input_fields.join("\n");
    format!("input {}Input {{\n{}\n}}", model_name, fields_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use prax_schema::Schema;
    use prax_schema::ast::{EnumVariant, Field, Ident, Span};

    fn make_span() -> Span {
        Span::new(0, 0)
    }

    fn make_ident(name: &str) -> Ident {
        Ident::new(name, make_span())
    }

    #[test]
    fn test_graphql_plugin_model() {
        let schema = Schema::new();
        let config = crate::plugins::PluginConfig::new();
        let ctx = PluginContext::new(&schema, &config);

        let mut model = Model::new(make_ident("User"), make_span());
        model.add_field(Field::new(
            make_ident("id"),
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
            vec![],
            make_span(),
        ));
        model.add_field(Field::new(
            make_ident("email"),
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
            vec![],
            make_span(),
        ));

        let plugin = GraphQLPlugin;
        let output = plugin.on_model(&ctx, &model);

        let code = output.tokens.to_string();
        assert!(code.contains("_graphql"));
        assert!(code.contains("SDL"));
        assert!(code.contains("User"));
        assert!(code.contains("INPUT_SDL"));
    }

    #[test]
    fn test_graphql_plugin_enum() {
        let schema = Schema::new();
        let config = crate::plugins::PluginConfig::new();
        let ctx = PluginContext::new(&schema, &config);

        let mut enum_def = Enum::new(make_ident("Role"), make_span());
        enum_def.add_variant(EnumVariant::new(make_ident("USER"), make_span()));
        enum_def.add_variant(EnumVariant::new(make_ident("ADMIN"), make_span()));

        let plugin = GraphQLPlugin;
        let output = plugin.on_enum(&ctx, &enum_def);

        let code = output.tokens.to_string();
        assert!(code.contains("_graphql"));
        assert!(code.contains("enum Role"));
        assert!(code.contains("variant_names"));
    }

    #[test]
    fn test_field_type_to_graphql() {
        assert_eq!(
            field_type_to_graphql(&FieldType::Scalar(ScalarType::Int), &TypeModifier::Required),
            "Int!"
        );
        assert_eq!(
            field_type_to_graphql(
                &FieldType::Scalar(ScalarType::String),
                &TypeModifier::Optional
            ),
            "String"
        );
        assert_eq!(
            field_type_to_graphql(&FieldType::Scalar(ScalarType::Int), &TypeModifier::List),
            "[Int!]!"
        );
        assert_eq!(
            field_type_to_graphql(
                &FieldType::Scalar(ScalarType::Uuid),
                &TypeModifier::Required
            ),
            "ID!"
        );
    }

    #[test]
    fn test_field_type_to_rust() {
        let int_type =
            field_type_to_rust(&FieldType::Scalar(ScalarType::Int), &TypeModifier::Required);
        assert!(int_type.to_string().contains("i32"));

        let optional_string = field_type_to_rust(
            &FieldType::Scalar(ScalarType::String),
            &TypeModifier::Optional,
        );
        assert!(optional_string.to_string().contains("Option"));
        assert!(optional_string.to_string().contains("String"));

        let list_int = field_type_to_rust(&FieldType::Scalar(ScalarType::Int), &TypeModifier::List);
        assert!(list_int.to_string().contains("Vec"));
    }

    #[test]
    fn test_scalar_to_filter_type() {
        assert!(scalar_to_filter_type(&ScalarType::Int).is_some());
        assert!(scalar_to_filter_type(&ScalarType::String).is_some());
        assert!(scalar_to_filter_type(&ScalarType::Boolean).is_some());
        assert!(scalar_to_filter_type(&ScalarType::Json).is_none());
    }

    #[test]
    fn test_make_optional_type() {
        let int_type = quote! { i32 };
        let optional = make_optional_type(&int_type);
        assert!(optional.to_string().contains("Option"));

        let already_optional = quote! { Option<i32> };
        let still_optional = make_optional_type(&already_optional);
        // Should not double-wrap
        assert!(!still_optional.to_string().contains("Option < Option"));
    }

    #[test]
    fn test_generate_input_sdl() {
        let mut model = Model::new(make_ident("User"), make_span());
        model.add_field(Field::new(
            make_ident("id"),
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
            vec![],
            make_span(),
        ));
        model.add_field(Field::new(
            make_ident("email"),
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
            vec![],
            make_span(),
        ));

        let sdl = generate_input_sdl(&model);
        assert!(sdl.contains("input UserInput"));
        assert!(sdl.contains("email: String!"));
    }
}
