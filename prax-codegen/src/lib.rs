//! Procedural macros for the Prax ORM.
//!
//! This crate provides compile-time code generation for Prax, transforming
//! schema definitions into type-safe Rust code.
//!
//! # Macros
//!
//! - [`prax_schema!`] - Generate models from a `.prax` schema file
//! - [`Model`] - Derive macro for manual model definition
//!
//! # Plugins
//!
//! Code generation can be extended with plugins enabled via environment variables:
//!
//! ```bash
//! # Enable debug information
//! PRAX_PLUGIN_DEBUG=1 cargo build
//!
//! # Enable JSON Schema generation
//! PRAX_PLUGIN_JSON_SCHEMA=1 cargo build
//!
//! # Enable GraphQL SDL generation
//! PRAX_PLUGIN_GRAPHQL=1 cargo build
//!
//! # Enable custom serialization helpers
//! PRAX_PLUGIN_SERDE=1 cargo build
//!
//! # Enable runtime validation
//! PRAX_PLUGIN_VALIDATOR=1 cargo build
//!
//! # Enable all plugins
//! PRAX_PLUGINS_ALL=1 cargo build
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! // Generate models from schema file
//! prax::prax_schema!("schema.prax");
//!
//! // Or manually define with derive macro
//! #[derive(prax::Model)]
//! #[prax(table = "users")]
//! struct User {
//!     #[prax(id, auto)]
//!     id: i32,
//!     #[prax(unique)]
//!     email: String,
//!     name: Option<String>,
//! }
//! ```

use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, LitStr, parse_macro_input};

mod generators;
mod plugins;
mod schema_reader;
mod types;

use generators::{
    generate_enum_module, generate_model_module_with_style, generate_type_module,
    generate_view_module,
};

/// Generate models from a Prax schema file.
///
/// This macro reads a `.prax` schema file at compile time and generates
/// type-safe Rust code for all models, enums, and types defined in the schema.
///
/// # Example
///
/// ```rust,ignore
/// prax::prax_schema!("schema.prax");
///
/// // Now you can use the generated types:
/// let user = client.user().find_unique(user::id::equals(1)).exec().await?;
/// ```
///
/// # Generated Code
///
/// For each model in the schema, this macro generates:
/// - A module with the model name (snake_case)
/// - A `Data` struct representing a row from the database
/// - A `CreateInput` struct for creating new records
/// - A `UpdateInput` struct for updating records
/// - Field modules with filter operations (`equals`, `contains`, `in_`, etc.)
/// - A `WhereParam` enum for type-safe filtering
/// - An `OrderByParam` enum for sorting
/// - Select and Include builders for partial queries
#[proc_macro]
pub fn prax_schema(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as LitStr);
    let schema_path = input.value();

    match generate_from_schema(&schema_path) {
        Ok(tokens) => tokens.into(),
        Err(err) => {
            let err_msg = err.to_string();
            quote! {
                compile_error!(#err_msg);
            }
            .into()
        }
    }
}

/// Derive macro for defining Prax models manually.
///
/// This derive macro allows you to define models in Rust code instead of
/// using a `.prax` schema file. It generates the same query builder methods
/// and type-safe operations.
///
/// # Attributes
///
/// ## Struct-level
/// - `#[prax(table = "table_name")]` - Map to a different table name
/// - `#[prax(schema = "schema_name")]` - Specify database schema
///
/// ## Field-level
/// - `#[prax(id)]` - Mark as primary key
/// - `#[prax(auto)]` - Auto-increment field
/// - `#[prax(unique)]` - Unique constraint
/// - `#[prax(default = value)]` - Default value
/// - `#[prax(column = "col_name")]` - Map to different column
/// - `#[prax(relation(...))]` - Define relation
///
/// # Example
///
/// ```rust,ignore
/// #[derive(prax::Model)]
/// #[prax(table = "users")]
/// struct User {
///     #[prax(id, auto)]
///     id: i32,
///
///     #[prax(unique)]
///     email: String,
///
///     #[prax(column = "display_name")]
///     name: Option<String>,
///
///     #[prax(default = "now()")]
///     created_at: chrono::DateTime<chrono::Utc>,
/// }
/// ```
#[proc_macro_derive(Model, attributes(prax))]
pub fn derive_model(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match generators::derive_model_impl(&input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Internal function to generate code from a schema file.
fn generate_from_schema(schema_path: &str) -> Result<proc_macro2::TokenStream, syn::Error> {
    use plugins::{PluginConfig, PluginContext, PluginRegistry};
    use schema_reader::read_schema_with_config;

    // Read and parse the schema file along with prax.toml configuration
    let schema_with_config = read_schema_with_config(schema_path).map_err(|e| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("Failed to parse schema: {}", e),
        )
    })?;

    let schema = schema_with_config.schema;
    let model_style = schema_with_config.model_style;

    // Initialize plugin system with model_style from prax.toml
    // This auto-enables graphql plugins when model_style is GraphQL
    let plugin_config = PluginConfig::with_model_style(model_style);
    let plugin_registry = PluginRegistry::with_builtins();
    let plugin_ctx = PluginContext::new(&schema, &plugin_config);

    let mut output = proc_macro2::TokenStream::new();

    // Run plugin start hooks
    let start_output = plugin_registry.run_start(&plugin_ctx);
    output.extend(start_output.tokens);
    output.extend(start_output.root_items);

    // Generate enums first (models may reference them)
    for (_, enum_def) in &schema.enums {
        output.extend(generate_enum_module(enum_def)?);

        // Run plugin enum hooks
        let plugin_output = plugin_registry.run_enum(&plugin_ctx, enum_def);
        if !plugin_output.is_empty() {
            // Add plugin output to the enum module
            output.extend(plugin_output.tokens);
        }
    }

    // Generate composite types
    for (_, type_def) in &schema.types {
        output.extend(generate_type_module(type_def)?);

        // Run plugin type hooks
        let plugin_output = plugin_registry.run_type(&plugin_ctx, type_def);
        if !plugin_output.is_empty() {
            output.extend(plugin_output.tokens);
        }
    }

    // Generate views
    for (_, view_def) in &schema.views {
        output.extend(generate_view_module(view_def)?);

        // Run plugin view hooks
        let plugin_output = plugin_registry.run_view(&plugin_ctx, view_def);
        if !plugin_output.is_empty() {
            output.extend(plugin_output.tokens);
        }
    }

    // Generate models with the configured model style
    for (_, model_def) in &schema.models {
        output.extend(generate_model_module_with_style(
            model_def,
            &schema,
            model_style,
        )?);

        // Run plugin model hooks
        let plugin_output = plugin_registry.run_model(&plugin_ctx, model_def);
        if !plugin_output.is_empty() {
            output.extend(plugin_output.tokens);
        }
    }

    // Run plugin finish hooks
    let finish_output = plugin_registry.run_finish(&plugin_ctx);
    output.extend(finish_output.tokens);
    output.extend(finish_output.root_items);

    // Generate plugin documentation
    output.extend(plugins::generate_plugin_docs(&plugin_registry));

    Ok(output)
}
