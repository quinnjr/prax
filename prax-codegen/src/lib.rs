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
mod macros;
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

/// `prax::find_many!` — schema-aware declarative DSL for the
/// fluent-builder's `find_many` operation. See spec §4 for the full
/// grammar.
///
/// ```rust,ignore
/// prax::find_many!(client.user, {
///     where: { email: { contains: "@example.com" } },
///     order_by: { created_at: desc },
///     take: 10,
/// });
/// ```
#[proc_macro]
pub fn find_many(input: TokenStream) -> TokenStream {
    match macros::ops::find_many::expand_find_many(input.into()) {
        Ok(t) => t.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// `prax::find_unique!` — schema-aware DSL targeting `find_unique`.
/// The `where:` block must match a single `@unique` (or `@id`) column.
#[proc_macro]
pub fn find_unique(input: TokenStream) -> TokenStream {
    match macros::ops::find_unique::expand_find_unique(input.into()) {
        Ok(t) => t.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// `prax::find_first!` — schema-aware DSL targeting `find_first`.
#[proc_macro]
pub fn find_first(input: TokenStream) -> TokenStream {
    match macros::ops::find_first::expand_find_first(input.into()) {
        Ok(t) => t.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// `prax::count!` — schema-aware DSL targeting `count`. Phase 3 only
/// supports the `where:` key; the Prisma-style `_count` aggregate
/// (`select: { _count: { posts: true } }`) is phase 6.
#[proc_macro]
pub fn count(input: TokenStream) -> TokenStream {
    match macros::ops::count::expand_count(input.into()) {
        Ok(t) => t.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// `prax::delete!` — schema-aware DSL targeting `delete`. The
/// `where:` block must match a unique column.
#[proc_macro]
pub fn delete(input: TokenStream) -> TokenStream {
    match macros::ops::delete::expand_delete(input.into()) {
        Ok(t) => t.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// `prax::delete_many!` — schema-aware DSL targeting `delete_many`.
/// The `where:` block is the non-unique form.
///
/// **Warning:** an empty / `Filter::None` filter matches every row in
/// the table. See `WhereInput`'s trait-level note.
#[proc_macro]
pub fn delete_many(input: TokenStream) -> TokenStream {
    match macros::ops::delete_many::expand_delete_many(input.into()) {
        Ok(t) => t.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// `prax::r#where!` — schema-aware shape macro returning a
/// `<Model>WhereInput` value. Composes with the read macros via
/// `..spread`:
///
/// ```rust,ignore
/// let active = prax::r#where!(User, { active: true });
/// let _ = prax::find_many!(client.user, {
///     ..active,
///     email: { contains: "@x.com" },
/// });
/// ```
///
/// Exported as `r#where` because `where` is a Rust keyword and the
/// raw-identifier prefix is required at the call site whenever the
/// macro is reached through a path (`prax::r#where!(...)`).
#[proc_macro]
pub fn r#where(input: TokenStream) -> TokenStream {
    match macros::ops::shape::expand_where_shape(input.into()) {
        Ok(t) => t.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// `prax::include!` — schema-aware shape macro returning a
/// `<Model>Include` value. Composes with the read macros via
/// `..spread` to build reusable relation-include shapes.
///
/// ```rust,ignore
/// let with_posts = prax::include!(User, { posts: true });
/// let _ = prax::find_unique!(client.user, {
///     where: { id: 1 },
///     include: { ..with_posts },
/// });
/// ```
///
/// Distinct from `std::include!` — they live in different modules and
/// there is no ambiguity at the call site as long as the path is
/// fully qualified (`prax::include!`).
#[proc_macro]
pub fn include(input: TokenStream) -> TokenStream {
    match macros::ops::shape::expand_include_shape(input.into()) {
        Ok(t) => t.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// `prax::select!` — schema-aware shape macro returning a
/// `<Model>Select` value. Composes with the read macros via `..spread`.
///
/// ```rust,ignore
/// let lite = prax::select!(User, { id: true, email: true });
/// let _ = prax::find_many!(client.user, {
///     select: { ..lite },
/// });
/// ```
#[proc_macro]
pub fn select(input: TokenStream) -> TokenStream {
    match macros::ops::shape::expand_select_shape(input.into()) {
        Ok(t) => t.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// `prax::order_by!` — schema-aware shape macro returning an
/// `OrderBy` value. Accepts either a single `{ field: dir }` block or
/// a list of such blocks for multi-key sorts.
///
/// ```rust,ignore
/// let newest_first = prax::order_by!(User, { created_at: desc });
/// let _ = prax::find_many!(client.user, {
///     order_by: { created_at: desc },
/// });
/// // or as a list:
/// let by_active_then_email = prax::order_by!(User, [
///     { active: desc },
///     { email: asc },
/// ]);
/// ```
#[proc_macro]
pub fn order_by(input: TokenStream) -> TokenStream {
    match macros::ops::shape::expand_order_by_shape(input.into()) {
        Ok(t) => t.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// `prax::create!` — schema-aware DSL targeting `create`. Top-level
/// keys: `data:` (required), `include` xor `select`. Phase 5a is
/// scalar-only — relation operators inside `data:` (nested writes)
/// land in phase 5b.
///
/// ```rust,ignore
/// prax::create!(client.user, {
///     data: { email: "a@x.com", name: "Alice", age: 30 },
///     select: { id: true, email: true },
/// });
/// ```
#[proc_macro]
pub fn create(input: TokenStream) -> TokenStream {
    match macros::ops::create::expand_create(input.into()) {
        Ok(t) => t.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// `prax::cursor!` — schema-aware shape macro returning a
/// `<Model>WhereUniqueInput` value for use as a `cursor:` argument to
/// the read macros.
///
/// The block must have exactly one entry whose key refers to an
/// `@id` or `@unique` column on the model.
///
/// ```rust,ignore
/// let from = prax::cursor!(User, { id: 42 });
/// let _ = prax::find_many!(client.user, {
///     cursor: { id: 42 },
///     take: 10,
/// });
/// ```
#[proc_macro]
pub fn cursor(input: TokenStream) -> TokenStream {
    match macros::ops::shape::expand_cursor_shape(input.into()) {
        Ok(t) => t.into(),
        Err(e) => e.to_compile_error().into(),
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
