//! `prax generate` command - Generate Rust client code from schema.

use std::path::PathBuf;

use crate::cli::GenerateArgs;
use crate::config::{CONFIG_FILE_NAME, Config, SCHEMA_FILE_PATH};
use crate::error::{CliError, CliResult};
use crate::output::{self, success};

/// Run the generate command
pub async fn run(args: GenerateArgs) -> CliResult<()> {
    output::header("Generate Prax Client");

    let cwd = std::env::current_dir()?;

    // Load config
    let config_path = cwd.join(CONFIG_FILE_NAME);
    let config = if config_path.exists() {
        Config::load(&config_path)?
    } else {
        Config::default()
    };

    // Resolve schema path
    let schema_path = args
        .schema
        .clone()
        .unwrap_or_else(|| cwd.join(SCHEMA_FILE_PATH));
    if !schema_path.exists() {
        return Err(
            CliError::Config(format!("Schema file not found: {}", schema_path.display())).into(),
        );
    }

    // Resolve output directory
    let output_dir = args
        .output
        .clone()
        .unwrap_or_else(|| PathBuf::from(&config.generator.output));

    output::kv("Schema", &schema_path.display().to_string());
    output::kv("Output", &output_dir.display().to_string());
    output::newline();

    output::step(1, 4, "Reading schema...");

    // Parse schema
    let schema_content = std::fs::read_to_string(&schema_path)?;
    let schema = parse_schema(&schema_content)?;

    output::step(2, 4, "Validating schema...");

    // Validate schema
    validate_schema(&schema)?;

    output::step(3, 4, "Generating code...");

    // Create output directory
    std::fs::create_dir_all(&output_dir)?;

    // Generate code
    let generated_files = generate_code(&schema, &output_dir, &args, &config)?;

    output::step(4, 4, "Writing files...");

    // Print generated files
    output::newline();
    output::section("Generated files");

    for file in &generated_files {
        let relative_path = file
            .strip_prefix(&cwd)
            .unwrap_or(file)
            .display()
            .to_string();
        output::list_item(&relative_path);
    }

    output::newline();
    success(&format!(
        "Generated {} files in {:.2}s",
        generated_files.len(),
        0.0 // TODO: Add timing
    ));

    Ok(())
}

/// Parse and validate the schema file
fn parse_schema(content: &str) -> CliResult<prax_schema::Schema> {
    // Use validate_schema to ensure field types are properly resolved
    // (e.g., FieldType::Model -> FieldType::Enum for enum references)
    prax_schema::validate_schema(content)
        .map_err(|e| CliError::Schema(format!("Failed to parse/validate schema: {}", e)))
}

/// Validate the schema (now a no-op since parse_schema does validation)
fn validate_schema(_schema: &prax_schema::Schema) -> CliResult<()> {
    // Validation is now done in parse_schema via validate_schema()
    Ok(())
}

/// Generate code from the schema
fn generate_code(
    schema: &prax_schema::ast::Schema,
    output_dir: &PathBuf,
    args: &GenerateArgs,
    config: &Config,
) -> CliResult<Vec<PathBuf>> {
    let mut generated_files = Vec::new();

    // Determine which features to generate
    let features = if !args.features.is_empty() {
        args.features.clone()
    } else {
        config
            .generator
            .features
            .clone()
            .unwrap_or_else(|| vec!["client".to_string()])
    };

    // Generate main client module
    let client_path = output_dir.join("mod.rs");
    let client_code = generate_client_module(schema, &features)?;
    std::fs::write(&client_path, client_code)?;
    generated_files.push(client_path);

    // Generate model modules
    for model in schema.models.values() {
        let model_path = output_dir.join(format!("{}.rs", to_snake_case(model.name())));
        let model_code = generate_model_module(model, &features)?;
        std::fs::write(&model_path, model_code)?;
        generated_files.push(model_path);
    }

    // Generate enum modules
    for enum_def in schema.enums.values() {
        let enum_path = output_dir.join(format!("{}.rs", to_snake_case(enum_def.name())));
        let enum_code = generate_enum_module(enum_def)?;
        std::fs::write(&enum_path, enum_code)?;
        generated_files.push(enum_path);
    }

    // Generate type definitions
    let types_path = output_dir.join("types.rs");
    let types_code = generate_types_module(schema)?;
    std::fs::write(&types_path, types_code)?;
    generated_files.push(types_path);

    // Generate filters
    let filters_path = output_dir.join("filters.rs");
    let filters_code = generate_filters_module(schema)?;
    std::fs::write(&filters_path, filters_code)?;
    generated_files.push(filters_path);

    Ok(generated_files)
}

/// Generate the main client module
fn generate_client_module(
    schema: &prax_schema::ast::Schema,
    _features: &[String],
) -> CliResult<String> {
    let mut code = String::new();

    code.push_str("//! Auto-generated by Prax - DO NOT EDIT\n");
    code.push_str("//!\n");
    code.push_str("//! This module contains the generated Prax client.\n\n");

    // Module declarations
    code.push_str("pub mod types;\n");
    code.push_str("pub mod filters;\n\n");

    for model in schema.models.values() {
        code.push_str(&format!("pub mod {};\n", to_snake_case(model.name())));
    }

    for enum_def in schema.enums.values() {
        code.push_str(&format!("pub mod {};\n", to_snake_case(enum_def.name())));
    }

    code.push_str("\n");

    // Re-exports
    code.push_str("pub use types::*;\n");
    code.push_str("pub use filters::*;\n\n");

    for model in schema.models.values() {
        code.push_str(&format!(
            "pub use {}::{};\n",
            to_snake_case(model.name()),
            model.name()
        ));
    }

    for enum_def in schema.enums.values() {
        code.push_str(&format!(
            "pub use {}::{};\n",
            to_snake_case(enum_def.name()),
            enum_def.name()
        ));
    }

    code.push_str("\n");

    // Client struct
    code.push_str("/// The Prax database client\n");
    code.push_str("pub struct PraxClient<E: prax_query::QueryEngine> {\n");
    code.push_str("    engine: E,\n");
    code.push_str("}\n\n");

    code.push_str("impl<E: prax_query::QueryEngine> PraxClient<E> {\n");
    code.push_str("    /// Create a new Prax client with the given query engine\n");
    code.push_str("    pub fn new(engine: E) -> Self {\n");
    code.push_str("        Self { engine }\n");
    code.push_str("    }\n\n");

    for model in schema.models.values() {
        let snake_name = to_snake_case(model.name());
        code.push_str(&format!("    /// Access {} operations\n", model.name()));
        code.push_str(&format!(
            "    pub fn {}(&self) -> {}::{}Operations<E> {{\n",
            snake_name,
            snake_name,
            model.name()
        ));
        code.push_str(&format!(
            "        {}::{}Operations::new(&self.engine)\n",
            snake_name,
            model.name()
        ));
        code.push_str("    }\n\n");
    }

    code.push_str("}\n");

    Ok(code)
}

/// Generate a model module
fn generate_model_module(
    model: &prax_schema::ast::Model,
    features: &[String],
) -> CliResult<String> {
    let mut code = String::new();

    code.push_str(&format!(
        "//! Auto-generated module for {} model\n\n",
        model.name()
    ));

    // Derive macros based on features
    let mut derives = vec!["Debug", "Clone"];
    if features.contains(&"serde".to_string()) {
        derives.push("serde::Serialize");
        derives.push("serde::Deserialize");
    }

    // Model struct
    code.push_str(&format!("#[derive({})]\n", derives.join(", ")));
    code.push_str(&format!("pub struct {} {{\n", model.name()));

    for field in model.fields.values() {
        let rust_type = field_type_to_rust(&field.field_type, field.modifier);
        let field_name = to_snake_case(field.name());

        // Add serde rename if mapped
        if let Some(attr) = field.get_attribute("map") {
            if features.contains(&"serde".to_string()) {
                if let Some(value) = attr.first_arg().and_then(|v| v.as_string()) {
                    code.push_str(&format!("    #[serde(rename = \"{}\")]\n", value));
                }
            }
        }

        code.push_str(&format!("    pub {}: {},\n", field_name, rust_type));
    }

    code.push_str("}\n\n");

    // Operations struct
    code.push_str(&format!("/// Operations for the {} model\n", model.name()));
    code.push_str(&format!(
        "pub struct {}Operations<'a, E: prax_query::QueryEngine> {{\n",
        model.name()
    ));
    code.push_str("    engine: &'a E,\n");
    code.push_str("}\n\n");

    code.push_str(&format!(
        "impl<'a, E: prax_query::QueryEngine> {}Operations<'a, E> {{\n",
        model.name()
    ));
    code.push_str("    pub fn new(engine: &'a E) -> Self {\n");
    code.push_str("        Self { engine }\n");
    code.push_str("    }\n\n");

    let table_name = model.table_name();

    // CRUD methods
    code.push_str("    /// Find many records\n");
    code.push_str(&format!(
        "    pub fn find_many(&self) -> prax_query::FindManyOperation<'a, E, {}> {{\n",
        model.name()
    ));
    code.push_str(&format!(
        "        prax_query::FindManyOperation::new(self.engine, \"{}\")\n",
        table_name
    ));
    code.push_str("    }\n\n");

    code.push_str("    /// Find a unique record\n");
    code.push_str(&format!(
        "    pub fn find_unique(&self) -> prax_query::FindUniqueOperation<'a, E, {}> {{\n",
        model.name()
    ));
    code.push_str(&format!(
        "        prax_query::FindUniqueOperation::new(self.engine, \"{}\")\n",
        table_name
    ));
    code.push_str("    }\n\n");

    code.push_str("    /// Find the first matching record\n");
    code.push_str(&format!(
        "    pub fn find_first(&self) -> prax_query::FindFirstOperation<'a, E, {}> {{\n",
        model.name()
    ));
    code.push_str(&format!(
        "        prax_query::FindFirstOperation::new(self.engine, \"{}\")\n",
        table_name
    ));
    code.push_str("    }\n\n");

    code.push_str("    /// Create a new record\n");
    code.push_str(&format!(
        "    pub fn create(&self) -> prax_query::CreateOperation<'a, E, {}> {{\n",
        model.name()
    ));
    code.push_str(&format!(
        "        prax_query::CreateOperation::new(self.engine, \"{}\")\n",
        table_name
    ));
    code.push_str("    }\n\n");

    code.push_str("    /// Update a record\n");
    code.push_str(&format!(
        "    pub fn update(&self) -> prax_query::UpdateOperation<'a, E, {}> {{\n",
        model.name()
    ));
    code.push_str(&format!(
        "        prax_query::UpdateOperation::new(self.engine, \"{}\")\n",
        table_name
    ));
    code.push_str("    }\n\n");

    code.push_str("    /// Delete a record\n");
    code.push_str(&format!(
        "    pub fn delete(&self) -> prax_query::DeleteOperation<'a, E, {}> {{\n",
        model.name()
    ));
    code.push_str(&format!(
        "        prax_query::DeleteOperation::new(self.engine, \"{}\")\n",
        table_name
    ));
    code.push_str("    }\n\n");

    code.push_str("    /// Count records\n");
    code.push_str("    pub fn count(&self) -> prax_query::CountOperation<'a, E> {\n");
    code.push_str(&format!(
        "        prax_query::CountOperation::new(self.engine, \"{}\")\n",
        table_name
    ));
    code.push_str("    }\n");

    code.push_str("}\n");

    Ok(code)
}

/// Generate an enum module
fn generate_enum_module(enum_def: &prax_schema::ast::Enum) -> CliResult<String> {
    let mut code = String::new();

    code.push_str(&format!(
        "//! Auto-generated module for {} enum\n\n",
        enum_def.name()
    ));

    code.push_str(
        "#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]\n",
    );
    code.push_str(&format!("pub enum {} {{\n", enum_def.name()));

    for variant in &enum_def.variants {
        // Check for @map attribute
        if let Some(attr) = variant.attributes.iter().find(|a| a.is("map")) {
            if let Some(value) = attr.first_arg().and_then(|v| v.as_string()) {
                code.push_str(&format!("    #[serde(rename = \"{}\")]\n", value));
            }
        }
        code.push_str(&format!("    {},\n", variant.name()));
    }

    code.push_str("}\n\n");

    // Default implementation
    if let Some(default_variant) = enum_def.variants.first() {
        code.push_str(&format!("impl Default for {} {{\n", enum_def.name()));
        code.push_str(&format!(
            "    fn default() -> Self {{\n        Self::{}\n    }}\n",
            default_variant.name()
        ));
        code.push_str("}\n");
    }

    Ok(code)
}

/// Generate types module
fn generate_types_module(schema: &prax_schema::ast::Schema) -> CliResult<String> {
    let mut code = String::new();

    code.push_str("//! Common type definitions\n\n");
    code.push_str("pub use chrono::{DateTime, Utc};\n");
    code.push_str("pub use uuid::Uuid;\n");
    code.push_str("pub use serde_json::Value as Json;\n");
    code.push_str("\n");

    // Add any custom types from composite types
    for composite in schema.types.values() {
        code.push_str("#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]\n");
        code.push_str(&format!("pub struct {} {{\n", composite.name()));
        for field in composite.fields.values() {
            let rust_type = field_type_to_rust(&field.field_type, field.modifier);
            let field_name = to_snake_case(field.name());
            code.push_str(&format!("    pub {}: {},\n", field_name, rust_type));
        }
        code.push_str("}\n\n");
    }

    Ok(code)
}

/// Generate filters module
fn generate_filters_module(schema: &prax_schema::ast::Schema) -> CliResult<String> {
    let mut code = String::new();

    code.push_str("//! Filter types for queries\n\n");
    code.push_str("use prax_query::filter::{Filter, ScalarFilter};\n\n");

    for model in schema.models.values() {
        // Where input
        code.push_str(&format!("/// Filter input for {} queries\n", model.name()));
        code.push_str("#[derive(Debug, Default, Clone)]\n");
        code.push_str(&format!("pub struct {}WhereInput {{\n", model.name()));

        for field in model.fields.values() {
            if !field.is_relation() {
                let filter_type = field_to_filter_type(&field.field_type);
                let field_name = to_snake_case(field.name());
                code.push_str(&format!(
                    "    pub {}: Option<{}>,\n",
                    field_name, filter_type
                ));
            }
        }

        code.push_str("    pub and: Option<Vec<Self>>,\n");
        code.push_str("    pub or: Option<Vec<Self>>,\n");
        code.push_str("    pub not: Option<Box<Self>>,\n");
        code.push_str("}\n\n");

        // OrderBy input
        code.push_str(&format!(
            "/// Order by input for {} queries\n",
            model.name()
        ));
        code.push_str("#[derive(Debug, Default, Clone)]\n");
        code.push_str(&format!("pub struct {}OrderByInput {{\n", model.name()));

        for field in model.fields.values() {
            if !field.is_relation() {
                let field_name = to_snake_case(field.name());
                code.push_str(&format!(
                    "    pub {}: Option<prax_query::SortOrder>,\n",
                    field_name
                ));
            }
        }

        code.push_str("}\n\n");
    }

    Ok(code)
}

/// Convert a field type to Rust type
fn field_type_to_rust(
    field_type: &prax_schema::ast::FieldType,
    modifier: prax_schema::ast::TypeModifier,
) -> String {
    use prax_schema::ast::{FieldType, ScalarType, TypeModifier};

    let base_type = match field_type {
        FieldType::Scalar(scalar) => match scalar {
            ScalarType::Int => "i32".to_string(),
            ScalarType::BigInt => "i64".to_string(),
            ScalarType::Float => "f64".to_string(),
            ScalarType::String => "String".to_string(),
            ScalarType::Boolean => "bool".to_string(),
            ScalarType::DateTime => "chrono::DateTime<chrono::Utc>".to_string(),
            ScalarType::Date => "chrono::NaiveDate".to_string(),
            ScalarType::Time => "chrono::NaiveTime".to_string(),
            ScalarType::Json => "serde_json::Value".to_string(),
            ScalarType::Bytes => "Vec<u8>".to_string(),
            ScalarType::Decimal => "rust_decimal::Decimal".to_string(),
            ScalarType::Uuid => "uuid::Uuid".to_string(),
            ScalarType::Cuid => "String".to_string(),
            ScalarType::Cuid2 => "String".to_string(),
            ScalarType::NanoId => "String".to_string(),
            ScalarType::Ulid => "String".to_string(),
            ScalarType::Vector(_) | ScalarType::HalfVector(_) => "Vec<f32>".to_string(),
            ScalarType::SparseVector(_) => "Vec<(u32, f32)>".to_string(),
            ScalarType::Bit(_) => "Vec<u8>".to_string(),
        },
        FieldType::Model(name) => name.to_string(),
        FieldType::Enum(name) => name.to_string(),
        FieldType::Composite(name) => name.to_string(),
        FieldType::Unsupported(_) => "serde_json::Value".to_string(),
    };

    match modifier {
        TypeModifier::Optional | TypeModifier::OptionalList => format!("Option<{}>", base_type),
        TypeModifier::List => format!("Vec<{}>", base_type),
        TypeModifier::Required => base_type,
    }
}

/// Convert a field type to filter type
fn field_to_filter_type(field_type: &prax_schema::ast::FieldType) -> String {
    use prax_schema::ast::{FieldType, ScalarType};

    match field_type {
        FieldType::Scalar(scalar) => match scalar {
            ScalarType::Int | ScalarType::BigInt => "ScalarFilter<i64>".to_string(),
            ScalarType::Float | ScalarType::Decimal => "ScalarFilter<f64>".to_string(),
            ScalarType::String
            | ScalarType::Uuid
            | ScalarType::Cuid
            | ScalarType::Cuid2
            | ScalarType::NanoId
            | ScalarType::Ulid => "ScalarFilter<String>".to_string(),
            ScalarType::Boolean => "ScalarFilter<bool>".to_string(),
            ScalarType::DateTime => "ScalarFilter<chrono::DateTime<chrono::Utc>>".to_string(),
            ScalarType::Date => "ScalarFilter<chrono::NaiveDate>".to_string(),
            ScalarType::Time => "ScalarFilter<chrono::NaiveTime>".to_string(),
            ScalarType::Json => "ScalarFilter<serde_json::Value>".to_string(),
            ScalarType::Bytes => "ScalarFilter<Vec<u8>>".to_string(),
            // Vector types don't have standard scalar filters
            ScalarType::Vector(_) | ScalarType::HalfVector(_) => "VectorFilter".to_string(),
            ScalarType::SparseVector(_) => "SparseVectorFilter".to_string(),
            ScalarType::Bit(_) => "BitFilter".to_string(),
        },
        FieldType::Enum(name) => format!("ScalarFilter<{}>", name),
        _ => "Filter".to_string(),
    }
}

/// Convert PascalCase to snake_case
fn to_snake_case(name: &str) -> String {
    let mut result = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(c.to_lowercase().next().unwrap());
        } else {
            result.push(c);
        }
    }
    result
}
