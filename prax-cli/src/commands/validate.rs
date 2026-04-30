//! `prax validate` command - Validate Prax schema file.

use crate::cli::ValidateArgs;
use crate::config::SCHEMA_FILE_PATH;
use crate::error::{CliError, CliResult};
use crate::output::{self, success, warn};

/// Run the validate command
pub async fn run(args: ValidateArgs) -> CliResult<()> {
    output::header("Validate Schema");

    let cwd = std::env::current_dir()?;
    let schema_path = args.schema.unwrap_or_else(|| cwd.join(SCHEMA_FILE_PATH));

    if !schema_path.exists() {
        return Err(CliError::Config(format!(
            "Schema file not found: {}",
            schema_path.display()
        )));
    }

    output::kv("Schema", &schema_path.display().to_string());
    output::newline();

    // Parse schema
    output::step(1, 3, "Parsing schema...");
    let schema_content = std::fs::read_to_string(&schema_path)?;
    let schema = parse_schema(&schema_content)?;

    // Validate schema
    output::step(2, 3, "Running validation checks...");
    let validation_result = validate_schema(&schema);

    // Check config
    output::step(3, 3, "Checking configuration...");
    let config_warnings = check_config(&schema);

    output::newline();

    // Report results
    match validation_result {
        Ok(()) => {
            if config_warnings.is_empty() {
                success("Schema is valid!");
            } else {
                success("Schema is valid with warnings:");
                output::newline();
                for warning in &config_warnings {
                    warn(warning);
                }
            }
        }
        Err(errors) => {
            output::error("Schema validation failed!");
            output::newline();
            output::section("Errors");
            for error in &errors {
                output::list_item(&format!("❌ {}", error));
            }
            if !config_warnings.is_empty() {
                output::newline();
                output::section("Warnings");
                for warning in &config_warnings {
                    warn(warning);
                }
            }
            return Err(CliError::Validation(format!(
                "Found {} validation errors",
                errors.len()
            )));
        }
    }

    output::newline();

    // Print schema summary
    output::section("Schema Summary");
    output::kv("Models", &schema.models.len().to_string());
    output::kv("Enums", &schema.enums.len().to_string());
    output::kv("Views", &schema.views.len().to_string());
    output::kv("Composites", &schema.types.len().to_string());

    // Count fields and relations
    let total_fields: usize = schema.models.values().map(|m| m.fields.len()).sum();

    // Count actual relations (exclude enum and composite type references)
    let relations: usize = schema
        .models
        .values()
        .flat_map(|m| m.fields.values())
        .filter(|f| {
            if let prax_schema::ast::FieldType::Model(ref name) = f.field_type {
                // Only count as relation if it's an actual model reference
                schema.models.contains_key(name.as_str())
                    && !schema.enums.contains_key(name.as_str())
                    && !schema.types.contains_key(name.as_str())
            } else {
                false
            }
        })
        .count();

    output::kv("Total Fields", &total_fields.to_string());
    output::kv("Relations", &relations.to_string());

    Ok(())
}

fn parse_schema(content: &str) -> CliResult<prax_schema::Schema> {
    // Use validate_schema to ensure field types are properly resolved
    // (e.g., FieldType::Model -> FieldType::Enum for enum references)
    prax_schema::validate_schema(content)
        .map_err(|e| CliError::Schema(format!("Syntax error: {}", e)))
}

fn validate_schema(schema: &prax_schema::ast::Schema) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();

    // Check for models
    if schema.models.is_empty() {
        errors.push("Schema must define at least one model".to_string());
    }

    // Validate each model
    for model in schema.models.values() {
        // Check for @id field
        let has_id = model.fields.values().any(|f| f.is_id());
        if !has_id {
            errors.push(format!(
                "Model '{}' must have a field with @id attribute",
                model.name()
            ));
        }

        // Check for duplicate field names (handled by IndexMap, but good to verify)
        let mut field_names = std::collections::HashSet::new();
        for field in model.fields.values() {
            if !field_names.insert(field.name()) {
                errors.push(format!(
                    "Duplicate field '{}' in model '{}'",
                    field.name(),
                    model.name()
                ));
            }
        }

        // Validate relations
        for field in model.fields.values() {
            if field.is_relation() {
                validate_relation(field, model, schema, &mut errors);
            }
        }
    }

    // Validate enums
    for enum_def in schema.enums.values() {
        if enum_def.variants.is_empty() {
            errors.push(format!(
                "Enum '{}' must have at least one variant",
                enum_def.name()
            ));
        }

        // Check for duplicate variants
        let mut variant_names = std::collections::HashSet::new();
        for variant in &enum_def.variants {
            if !variant_names.insert(variant.name()) {
                errors.push(format!(
                    "Duplicate variant '{}' in enum '{}'",
                    variant.name(),
                    enum_def.name()
                ));
            }
        }
    }

    // Check for duplicate model/enum names
    let mut type_names = std::collections::HashSet::new();
    for model in schema.models.values() {
        if !type_names.insert(model.name()) {
            errors.push(format!("Duplicate type name '{}'", model.name()));
        }
    }
    for enum_def in schema.enums.values() {
        if !type_names.insert(enum_def.name()) {
            errors.push(format!("Duplicate type name '{}'", enum_def.name()));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn validate_relation(
    field: &prax_schema::ast::Field,
    model: &prax_schema::ast::Model,
    schema: &prax_schema::ast::Schema,
    errors: &mut Vec<String>,
) {
    use prax_schema::ast::FieldType;

    // Get the relation target type
    let target_type = match &field.field_type {
        FieldType::Model(name) => name.as_str(),
        _ => return,
    };

    // Skip if this is actually an enum reference (parser treats non-scalar as Model initially)
    if schema.enums.contains_key(target_type) {
        return;
    }

    // Skip if this is a composite type reference
    if schema.types.contains_key(target_type) {
        return;
    }

    // Check if target model exists
    let target_model = schema.models.get(target_type);
    if target_model.is_none() {
        errors.push(format!(
            "Relation '{}' in model '{}' references unknown model '{}'",
            field.name(),
            model.name(),
            target_type
        ));
        return;
    }

    // Validate @relation attribute if present
    if let Some(relation_attr) = field.get_attribute("relation") {
        // Check fields argument
        if let Some(fields_arg) = relation_attr
            .args
            .iter()
            .find(|a| a.name.as_ref().map(|n| n.as_str()) == Some("fields"))
            && let Some(fields_str) = fields_arg.value.as_string()
        {
            let field_names: Vec<&str> = fields_str.split(',').map(|s| s.trim()).collect();
            for field_name in &field_names {
                if !model.fields.contains_key(*field_name) {
                    errors.push(format!(
                        "Relation '{}' in model '{}' references unknown field '{}'",
                        field.name(),
                        model.name(),
                        field_name
                    ));
                }
            }
        }

        // Check references argument
        if let Some(refs_arg) = relation_attr
            .args
            .iter()
            .find(|a| a.name.as_ref().map(|n| n.as_str()) == Some("references"))
            && let Some(refs_str) = refs_arg.value.as_string()
        {
            let ref_names: Vec<&str> = refs_str.split(',').map(|s| s.trim()).collect();
            let target = target_model.unwrap();
            for ref_name in &ref_names {
                if !target.fields.contains_key(*ref_name) {
                    errors.push(format!(
                        "Relation '{}' in model '{}' references unknown field '{}' in model '{}'",
                        field.name(),
                        model.name(),
                        ref_name,
                        target_type
                    ));
                }
            }
        }
    }
}

fn check_config(schema: &prax_schema::ast::Schema) -> Vec<String> {
    let mut warnings = Vec::new();

    // Check for common issues
    for model in schema.models.values() {
        // Warn about missing timestamps
        let has_created_at = model.fields.values().any(|f| {
            let name_lower = f.name().to_lowercase();
            name_lower == "createdat" || name_lower == "created_at"
        });
        let has_updated_at = model.fields.values().any(|f| {
            let name_lower = f.name().to_lowercase();
            name_lower == "updatedat" || name_lower == "updated_at"
        });

        if !has_created_at && !has_updated_at {
            warnings.push(format!(
                "Model '{}' has no timestamp fields (createdAt/updatedAt)",
                model.name()
            ));
        }
    }

    warnings
}
