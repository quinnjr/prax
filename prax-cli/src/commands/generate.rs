//! `prax generate` command - Generate Rust client code from schema.

use std::collections::{HashMap, HashSet};
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

    // Build relation graph for cycle detection
    let relation_graph = build_relation_graph(schema);

    // Generate main client module
    let client_path = output_dir.join("mod.rs");
    let client_code = generate_client_module(schema, &features)?;
    std::fs::write(&client_path, client_code)?;
    generated_files.push(client_path);

    // Generate model modules
    for model in schema.models.values() {
        let model_path = output_dir.join(format!("{}.rs", to_snake_case(model.name())));
        let model_code = generate_model_module(model, &features, &relation_graph)?;
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

/// Build a graph of model relations for cycle detection.
/// Returns a map from model name to the set of model names it references
/// (non-list relations only, since Vec<T> doesn't cause infinite size).
fn build_relation_graph(
    schema: &prax_schema::ast::Schema,
) -> HashMap<String, HashSet<String>> {
    let mut graph: HashMap<String, HashSet<String>> = HashMap::new();

    for model in schema.models.values() {
        let entry = graph.entry(model.name().to_string()).or_default();
        for field in model.fields.values() {
            if let prax_schema::ast::FieldType::Model(ref target) = field.field_type {
                if !field.is_list() {
                    entry.insert(target.to_string());
                }
            }
        }
    }

    graph
}

/// Check if a non-list relation field from `source_model` to `target_model`
/// participates in a cycle (i.e. target_model can reach source_model through
/// non-list relations). If so, the field must be wrapped in Box<T>.
fn needs_boxing(
    source_model: &str,
    target_model: &str,
    graph: &HashMap<String, HashSet<String>>,
) -> bool {
    let mut visited = HashSet::new();
    let mut stack = vec![target_model.to_string()];

    while let Some(current) = stack.pop() {
        if current == source_model {
            return true;
        }
        if !visited.insert(current.clone()) {
            continue;
        }
        if let Some(neighbors) = graph.get(&current) {
            for neighbor in neighbors {
                stack.push(neighbor.clone());
            }
        }
    }

    false
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
    code.push_str("#[allow(unused_imports)]\npub use types::*;\n");
    code.push_str("#[allow(unused_imports)]\npub use filters::*;\n\n");

    for model in schema.models.values() {
        code.push_str(&format!(
            "#[allow(unused_imports)]\npub use {}::{};\n",
            to_snake_case(model.name()),
            model.name()
        ));
    }

    for enum_def in schema.enums.values() {
        code.push_str(&format!(
            "#[allow(unused_imports)]\npub use {}::{};\n",
            to_snake_case(enum_def.name()),
            enum_def.name()
        ));
    }

    code.push_str("\n");

    // Client struct with Clone bound and derive
    code.push_str("#[allow(dead_code)]\n");
    code.push_str("/// The Prax database client\n");
    code.push_str("#[derive(Clone)]\n");
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
            "        {}::{}Operations::new(self.engine.clone())\n",
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
    relation_graph: &HashMap<String, HashSet<String>>,
) -> CliResult<String> {
    let mut code = String::new();

    code.push_str(&format!(
        "//! Auto-generated module for {} model\n\n",
        model.name()
    ));

    // Import sibling types for relation fields
    code.push_str("#[allow(unused_imports)]\n");
    code.push_str("use super::*;\n");
    code.push_str("#[allow(unused_imports)]\n");
    code.push_str("use prax_query::traits::Model;\n\n");

    // Derive macros based on features
    let mut derives = vec!["Debug", "Clone"];
    if features.contains(&"serde".to_string()) {
        derives.push("serde::Serialize");
        derives.push("serde::Deserialize");
    }

    // Model struct
    code.push_str("#[allow(dead_code)]\n");
    code.push_str(&format!("#[derive({})]\n", derives.join(", ")));
    code.push_str(&format!("pub struct {} {{\n", model.name()));

    for field in model.fields.values() {
        let field_name = to_snake_case(field.name());

        // Add serde rename if mapped
        if let Some(attr) = field.get_attribute("map") {
            if features.contains(&"serde".to_string()) {
                if let Some(value) = attr.first_arg().and_then(|v| v.as_string()) {
                    code.push_str(&format!("    #[serde(rename = \"{}\")]\n", value));
                }
            }
        }

        let rust_type = field_type_to_rust_with_boxing(
            &field.field_type,
            field.modifier,
            model.name(),
            relation_graph,
        );
        code.push_str(&format!("    pub {}: {},\n", field_name, rust_type));
    }

    code.push_str("}\n\n");

    // Model trait implementation
    let table_name = model.table_name();
    let id_fields: Vec<&str> = model.id_fields().iter().map(|f| f.name()).collect();
    let scalar_columns: Vec<String> = model
        .scalar_fields()
        .iter()
        .map(|f| {
            // Use @map name if present, otherwise snake_case the field name
            f.get_attribute("map")
                .and_then(|a| a.first_arg())
                .and_then(|v| v.as_string())
                .map(|s| s.to_string())
                .unwrap_or_else(|| to_snake_case(f.name()))
        })
        .collect();

    code.push_str(&format!("impl Model for {} {{\n", model.name()));
    code.push_str(&format!(
        "    const MODEL_NAME: &'static str = \"{}\";\n",
        model.name()
    ));
    code.push_str(&format!(
        "    const TABLE_NAME: &'static str = \"{}\";\n",
        table_name
    ));
    code.push_str(&format!(
        "    const PRIMARY_KEY: &'static [&'static str] = &[{}];\n",
        id_fields
            .iter()
            .map(|f| format!("\"{}\"", to_snake_case(f)))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    code.push_str(&format!(
        "    const COLUMNS: &'static [&'static str] = &[{}];\n",
        scalar_columns
            .iter()
            .map(|c| format!("\"{}\"", c))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    code.push_str("}\n\n");

    // Operations struct (owned engine, no lifetime)
    code.push_str("#[allow(dead_code)]\n");
    code.push_str(&format!("/// Operations for the {} model\n", model.name()));
    code.push_str(&format!(
        "pub struct {}Operations<E: prax_query::QueryEngine> {{\n",
        model.name()
    ));
    code.push_str("    engine: E,\n");
    code.push_str("}\n\n");

    code.push_str(&format!(
        "impl<E: prax_query::QueryEngine> {}Operations<E> {{\n",
        model.name()
    ));
    code.push_str("    pub fn new(engine: E) -> Self {\n");
    code.push_str("        Self { engine }\n");
    code.push_str("    }\n\n");

    // CRUD methods (1-arg constructors, no lifetime on return types)
    code.push_str("    /// Find many records\n");
    code.push_str(&format!(
        "    pub fn find_many(&self) -> prax_query::FindManyOperation<E, {}> {{\n",
        model.name()
    ));
    code.push_str("        prax_query::FindManyOperation::new(self.engine.clone())\n");
    code.push_str("    }\n\n");

    code.push_str("    /// Find a unique record\n");
    code.push_str(&format!(
        "    pub fn find_unique(&self) -> prax_query::FindUniqueOperation<E, {}> {{\n",
        model.name()
    ));
    code.push_str("        prax_query::FindUniqueOperation::new(self.engine.clone())\n");
    code.push_str("    }\n\n");

    code.push_str("    /// Find the first matching record\n");
    code.push_str(&format!(
        "    pub fn find_first(&self) -> prax_query::FindFirstOperation<E, {}> {{\n",
        model.name()
    ));
    code.push_str("        prax_query::FindFirstOperation::new(self.engine.clone())\n");
    code.push_str("    }\n\n");

    code.push_str("    /// Create a new record\n");
    code.push_str(&format!(
        "    pub fn create(&self) -> prax_query::CreateOperation<E, {}> {{\n",
        model.name()
    ));
    code.push_str("        prax_query::CreateOperation::new(self.engine.clone())\n");
    code.push_str("    }\n\n");

    code.push_str("    /// Update a record\n");
    code.push_str(&format!(
        "    pub fn update(&self) -> prax_query::UpdateOperation<E, {}> {{\n",
        model.name()
    ));
    code.push_str("        prax_query::UpdateOperation::new(self.engine.clone())\n");
    code.push_str("    }\n\n");

    code.push_str("    /// Delete a record\n");
    code.push_str(&format!(
        "    pub fn delete(&self) -> prax_query::DeleteOperation<E, {}> {{\n",
        model.name()
    ));
    code.push_str("        prax_query::DeleteOperation::new(self.engine.clone())\n");
    code.push_str("    }\n\n");

    code.push_str("    /// Count records\n");
    code.push_str(&format!(
        "    pub fn count(&self) -> prax_query::CountOperation<E, {}> {{\n",
        model.name()
    ));
    code.push_str("        prax_query::CountOperation::new(self.engine.clone())\n");
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

    code.push_str("#[allow(dead_code)]\n");
    code.push_str(
        "#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]\n",
    );
    code.push_str(&format!("pub enum {} {{\n", enum_def.name()));

    for variant in &enum_def.variants {
        let raw_name = variant.name();
        let pascal_name = to_pascal_case(raw_name);

        // Check for explicit @map attribute first
        if let Some(attr) = variant.attributes.iter().find(|a| a.is("map")) {
            if let Some(value) = attr.first_arg().and_then(|v| v.as_string()) {
                code.push_str(&format!("    #[serde(rename = \"{}\")]\n", value));
                code.push_str(&format!("    {},\n", pascal_name));
                continue;
            }
        }

        // If variant name differs from PascalCase form, add serde rename
        if raw_name != pascal_name {
            code.push_str(&format!("    #[serde(rename = \"{}\")]\n", raw_name));
        }
        code.push_str(&format!("    {},\n", pascal_name));
    }

    code.push_str("}\n\n");

    // Display implementation for SQL serialization
    code.push_str(&format!("impl std::fmt::Display for {} {{\n", enum_def.name()));
    code.push_str("    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {\n");
    code.push_str("        match self {\n");
    for variant in &enum_def.variants {
        let raw_name = variant.name();
        let pascal_name = to_pascal_case(raw_name);
        let db_value = variant.db_value();
        code.push_str(&format!(
            "            Self::{} => write!(f, \"{}\"),\n",
            pascal_name, db_value
        ));
    }
    code.push_str("        }\n");
    code.push_str("    }\n");
    code.push_str("}\n\n");

    // Default implementation
    if let Some(default_variant) = enum_def.variants.first() {
        let pascal_name = to_pascal_case(default_variant.name());
        code.push_str(&format!("impl Default for {} {{\n", enum_def.name()));
        code.push_str(&format!(
            "    fn default() -> Self {{\n        Self::{}\n    }}\n",
            pascal_name
        ));
        code.push_str("}\n");
    }

    Ok(code)
}

/// Generate types module
fn generate_types_module(schema: &prax_schema::ast::Schema) -> CliResult<String> {
    let mut code = String::new();

    code.push_str("//! Common type definitions\n\n");
    code.push_str("#[allow(unused_imports)]\npub use chrono::{DateTime, Utc};\n");
    code.push_str("#[allow(unused_imports)]\npub use uuid::Uuid;\n");
    code.push_str("#[allow(unused_imports)]\npub use serde_json::Value as Json;\n");
    code.push_str("\n");

    // Add any custom types from composite types
    for composite in schema.types.values() {
        code.push_str("#[allow(dead_code)]\n");
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
    code.push_str("#[allow(unused_imports)]\n");
    code.push_str("use prax_query::filter::{Filter, ScalarFilter};\n");

    // Collect all enum types referenced by model scalar fields
    let mut referenced_enums = HashSet::new();
    for model in schema.models.values() {
        for field in model.fields.values() {
            if !field.is_relation() {
                if let prax_schema::ast::FieldType::Enum(ref name) = field.field_type {
                    referenced_enums.insert(name.to_string());
                }
            }
        }
    }

    // Import enum types
    for enum_name in &referenced_enums {
        code.push_str(&format!(
            "#[allow(unused_imports)]\nuse super::{}::{};\n",
            to_snake_case(enum_name),
            enum_name
        ));
    }

    code.push_str("\n");

    for model in schema.models.values() {
        // Where input
        code.push_str("#[allow(dead_code)]\n");
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
        code.push_str("#[allow(dead_code)]\n");
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

/// Convert a field type to Rust type (basic, without boxing)
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

/// Convert a field type to Rust type with Box<T> wrapping for cyclic relations.
fn field_type_to_rust_with_boxing(
    field_type: &prax_schema::ast::FieldType,
    modifier: prax_schema::ast::TypeModifier,
    source_model: &str,
    relation_graph: &HashMap<String, HashSet<String>>,
) -> String {
    use prax_schema::ast::{FieldType, TypeModifier};

    // For model references (non-list), check if boxing is needed to break cycles
    if let FieldType::Model(target) = field_type {
        if !matches!(modifier, TypeModifier::List) {
            let should_box = needs_boxing(source_model, target, relation_graph);
            let base = target.to_string();
            return match modifier {
                TypeModifier::Optional | TypeModifier::OptionalList => {
                    if should_box {
                        format!("Option<Box<{}>>", base)
                    } else {
                        format!("Option<{}>", base)
                    }
                }
                TypeModifier::Required => {
                    if should_box {
                        format!("Box<{}>", base)
                    } else {
                        base
                    }
                }
                TypeModifier::List => unreachable!(),
            };
        }
    }

    // Fallback to basic conversion for non-cyclic fields
    field_type_to_rust(field_type, modifier)
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

/// Convert snake_case, SCREAMING_SNAKE_CASE, or any other casing to PascalCase.
fn to_pascal_case(name: &str) -> String {
    if name.is_empty() {
        return String::new();
    }

    // If already PascalCase (starts with uppercase, contains lowercase), return as-is
    let first = name.chars().next().unwrap();
    if first.is_uppercase() && name.chars().any(|c| c.is_lowercase()) && !name.contains('_') {
        return name.to_string();
    }

    // Split on underscores and capitalize each segment
    name.split('_')
        .filter(|s| !s.is_empty())
        .map(|segment| {
            let mut chars = segment.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => {
                    let rest: String = chars.collect();
                    format!("{}{}", first.to_uppercase(), rest.to_lowercase())
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_snake_case() {
        assert_eq!(to_snake_case("BoardMember"), "board_member");
        assert_eq!(to_snake_case("User"), "user");
        assert_eq!(to_snake_case("JiraImportConfig"), "jira_import_config");
    }

    #[test]
    fn test_to_pascal_case_from_snake() {
        assert_eq!(to_pascal_case("card_created"), "CardCreated");
        assert_eq!(to_pascal_case("branch_deleted"), "BranchDeleted");
        assert_eq!(to_pascal_case("pr_merged"), "PrMerged");
    }

    #[test]
    fn test_to_pascal_case_from_screaming() {
        assert_eq!(to_pascal_case("CARD_CREATED"), "CardCreated");
        assert_eq!(to_pascal_case("PR_MERGED"), "PrMerged");
    }

    #[test]
    fn test_to_pascal_case_already_pascal() {
        assert_eq!(to_pascal_case("Admin"), "Admin");
        assert_eq!(to_pascal_case("SuperAdmin"), "SuperAdmin");
        assert_eq!(to_pascal_case("Low"), "Low");
    }

    #[test]
    fn test_to_pascal_case_single_word() {
        assert_eq!(to_pascal_case("active"), "Active");
        assert_eq!(to_pascal_case("ACTIVE"), "Active");
    }

    #[test]
    fn test_needs_boxing_direct_cycle() {
        let mut graph = HashMap::new();
        graph.insert(
            "Board".to_string(),
            HashSet::from(["JiraConfig".to_string()]),
        );
        graph.insert(
            "JiraConfig".to_string(),
            HashSet::from(["Board".to_string()]),
        );

        assert!(needs_boxing("Board", "JiraConfig", &graph));
        assert!(needs_boxing("JiraConfig", "Board", &graph));
    }

    #[test]
    fn test_needs_boxing_no_cycle() {
        let mut graph = HashMap::new();
        graph.insert("Post".to_string(), HashSet::from(["User".to_string()]));
        graph.insert("User".to_string(), HashSet::new());

        assert!(!needs_boxing("Post", "User", &graph));
    }

    #[test]
    fn test_needs_boxing_indirect_cycle() {
        let mut graph = HashMap::new();
        graph.insert("A".to_string(), HashSet::from(["B".to_string()]));
        graph.insert("B".to_string(), HashSet::from(["C".to_string()]));
        graph.insert("C".to_string(), HashSet::from(["A".to_string()]));

        assert!(needs_boxing("A", "B", &graph));
        assert!(needs_boxing("B", "C", &graph));
        assert!(needs_boxing("C", "A", &graph));
    }
}
