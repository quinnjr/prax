//! `prax format` command - Format Prax schema file.

use crate::cli::FormatArgs;
use crate::config::SCHEMA_FILE_PATH;
use crate::error::{CliError, CliResult};
use crate::output::{self, success};

/// Run the format command
pub async fn run(args: FormatArgs) -> CliResult<()> {
    output::header("Format Schema");

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

    // Read schema
    output::step(1, 3, "Reading schema...");
    let schema_content = std::fs::read_to_string(&schema_path)?;

    // Parse schema to validate it first
    let schema = parse_schema(&schema_content)?;

    // Format schema
    output::step(2, 3, "Formatting...");
    let formatted = format_schema(&schema);

    // Check if formatting changed anything
    let changed = formatted != schema_content;

    if args.check {
        // Check mode - just report if formatting is needed
        if changed {
            output::newline();
            output::error("Schema is not formatted correctly!");
            output::info("Run `prax format` to fix formatting.");
            return Err(CliError::Format("Schema needs formatting".to_string()));
        } else {
            output::newline();
            success("Schema is already formatted!");
            return Ok(());
        }
    }

    // Write formatted schema
    output::step(3, 3, "Writing formatted schema...");

    if changed {
        std::fs::write(&schema_path, &formatted)?;
        output::newline();
        success("Schema formatted successfully!");
    } else {
        output::newline();
        success("Schema is already formatted!");
    }

    Ok(())
}

fn parse_schema(content: &str) -> CliResult<prax_schema::Schema> {
    // Use validate_schema to ensure field types are properly resolved
    // (e.g., FieldType::Model -> FieldType::Enum for enum references)
    prax_schema::validate_schema(content)
        .map_err(|e| CliError::Schema(format!("Syntax error: {}", e)))
}

/// Format a schema AST into a formatted string
fn format_schema(schema: &prax_schema::ast::Schema) -> String {
    let mut output = String::new();

    // Format datasource (if present in schema)
    // For now, just add a standard datasource section
    output.push_str("datasource db {\n");
    output.push_str("    provider = \"postgresql\"\n");
    output.push_str("    url      = env(\"DATABASE_URL\")\n");
    output.push_str("}\n");
    let mut first_section = false;

    // Format generator
    if !first_section {
        output.push('\n');
    }
    output.push_str("generator client {\n");
    output.push_str("    provider = \"prax-client-rust\"\n");
    output.push_str("    output   = \"./src/generated\"\n");
    output.push_str("}\n");
    first_section = false;

    // Format enums first (since they're used by models)
    for enum_def in schema.enums.values() {
        if !first_section {
            output.push('\n');
        }
        format_enum(&mut output, enum_def);
        first_section = false;
    }

    // Format models
    for model in schema.models.values() {
        if !first_section {
            output.push('\n');
        }
        format_model(&mut output, model);
        first_section = false;
    }

    // Format views
    for view in schema.views.values() {
        if !first_section {
            output.push('\n');
        }
        format_view(&mut output, view);
        first_section = false;
    }

    // Format composite types
    for composite in schema.types.values() {
        if !first_section {
            output.push('\n');
        }
        format_composite(&mut output, composite);
        first_section = false;
    }

    output
}

fn format_enum(output: &mut String, enum_def: &prax_schema::ast::Enum) {
    // Documentation
    if let Some(doc) = &enum_def.documentation {
        for line in doc.text.lines() {
            output.push_str(&format!("/// {}\n", line));
        }
    }

    output.push_str(&format!("enum {} {{\n", enum_def.name()));

    for variant in &enum_def.variants {
        // Documentation
        if let Some(doc) = &variant.documentation {
            for line in doc.text.lines() {
                output.push_str(&format!("    /// {}\n", line));
            }
        }

        output.push_str(&format!("    {}", variant.name()));

        // Format attributes
        for attr in &variant.attributes {
            output.push_str(&format!(" {}", format_attribute(attr)));
        }

        output.push('\n');
    }

    // Enum-level attributes
    for attr in &enum_def.attributes {
        output.push_str(&format!("\n    {}", format_attribute(attr)));
    }

    output.push_str("}\n");
}

fn format_model(output: &mut String, model: &prax_schema::ast::Model) {
    // Documentation
    if let Some(doc) = &model.documentation {
        for line in doc.text.lines() {
            output.push_str(&format!("/// {}\n", line));
        }
    }

    output.push_str(&format!("model {} {{\n", model.name()));

    // Calculate alignment for fields
    let max_name_len = model
        .fields
        .values()
        .map(|f| f.name().len())
        .max()
        .unwrap_or(0);

    let max_type_len = model
        .fields
        .values()
        .map(|f| format_field_type(&f.field_type, f.modifier).len())
        .max()
        .unwrap_or(0);

    for field in model.fields.values() {
        // Documentation
        if let Some(doc) = &field.documentation {
            for line in doc.text.lines() {
                output.push_str(&format!("    /// {}\n", line));
            }
        }

        let type_str = format_field_type(&field.field_type, field.modifier);

        // Pad name and type for alignment
        let padded_name = format!("{:width$}", field.name(), width = max_name_len);
        let padded_type = format!("{:width$}", type_str, width = max_type_len);

        output.push_str(&format!("    {} {}", padded_name, padded_type));

        // Format attributes
        for attr in &field.attributes {
            output.push_str(&format!(" {}", format_attribute(attr)));
        }

        output.push('\n');
    }

    // Model-level attributes
    let model_attrs: Vec<_> = model.attributes.iter().collect();
    if !model_attrs.is_empty() {
        output.push('\n');
        for attr in model_attrs {
            output.push_str(&format!("    {}\n", format_attribute(attr)));
        }
    }

    output.push_str("}\n");
}

fn format_view(output: &mut String, view: &prax_schema::ast::View) {
    // Documentation
    if let Some(doc) = &view.documentation {
        for line in doc.text.lines() {
            output.push_str(&format!("/// {}\n", line));
        }
    }

    output.push_str(&format!("view {} {{\n", view.name()));

    // Calculate alignment for fields
    let max_name_len = view
        .fields
        .values()
        .map(|f| f.name().len())
        .max()
        .unwrap_or(0);

    let max_type_len = view
        .fields
        .values()
        .map(|f| format_field_type(&f.field_type, f.modifier).len())
        .max()
        .unwrap_or(0);

    for field in view.fields.values() {
        let type_str = format_field_type(&field.field_type, field.modifier);
        let padded_name = format!("{:width$}", field.name(), width = max_name_len);
        let padded_type = format!("{:width$}", type_str, width = max_type_len);

        output.push_str(&format!("    {} {}", padded_name, padded_type));

        for attr in &field.attributes {
            output.push_str(&format!(" {}", format_attribute(attr)));
        }

        output.push('\n');
    }

    // View-level attributes
    let view_attrs: Vec<_> = view.attributes.iter().collect();
    if !view_attrs.is_empty() {
        output.push('\n');
        for attr in view_attrs {
            output.push_str(&format!("    {}\n", format_attribute(attr)));
        }
    }

    output.push_str("}\n");
}

fn format_composite(output: &mut String, composite: &prax_schema::ast::CompositeType) {
    // Documentation
    if let Some(doc) = &composite.documentation {
        for line in doc.text.lines() {
            output.push_str(&format!("/// {}\n", line));
        }
    }

    output.push_str(&format!("type {} {{\n", composite.name()));

    // Calculate alignment for fields
    let max_name_len = composite
        .fields
        .values()
        .map(|f| f.name().len())
        .max()
        .unwrap_or(0);

    let max_type_len = composite
        .fields
        .values()
        .map(|f| format_field_type(&f.field_type, f.modifier).len())
        .max()
        .unwrap_or(0);

    for field in composite.fields.values() {
        let type_str = format_field_type(&field.field_type, field.modifier);
        let padded_name = format!("{:width$}", field.name(), width = max_name_len);
        let padded_type = format!("{:width$}", type_str, width = max_type_len);

        output.push_str(&format!("    {} {}", padded_name, padded_type));

        for attr in &field.attributes {
            output.push_str(&format!(" {}", format_attribute(attr)));
        }

        output.push('\n');
    }

    output.push_str("}\n");
}

fn format_field_type(
    field_type: &prax_schema::ast::FieldType,
    modifier: prax_schema::ast::TypeModifier,
) -> String {
    use prax_schema::ast::{FieldType, ScalarType, TypeModifier};

    let base = match field_type {
        FieldType::Scalar(scalar) => match scalar {
            ScalarType::Int => "Int",
            ScalarType::BigInt => "BigInt",
            ScalarType::Float => "Float",
            ScalarType::String => "String",
            ScalarType::Boolean => "Boolean",
            ScalarType::DateTime => "DateTime",
            ScalarType::Date => "Date",
            ScalarType::Time => "Time",
            ScalarType::Json => "Json",
            ScalarType::Bytes => "Bytes",
            ScalarType::Decimal => "Decimal",
            ScalarType::Uuid => "Uuid",
            ScalarType::Cuid => "Cuid",
            ScalarType::Cuid2 => "Cuid2",
            ScalarType::NanoId => "NanoId",
            ScalarType::Ulid => "Ulid",
            ScalarType::Vector(_) => "Vector",
            ScalarType::HalfVector(_) => "HalfVector",
            ScalarType::SparseVector(_) => "SparseVector",
            ScalarType::Bit(_) => "Bit",
        }
        .to_string(),
        FieldType::Model(name) => name.to_string(),
        FieldType::Enum(name) => name.to_string(),
        FieldType::Composite(name) => name.to_string(),
        FieldType::Unsupported(name) => format!("Unsupported(\"{}\")", name),
    };

    match modifier {
        TypeModifier::Optional => format!("{}?", base),
        TypeModifier::List => format!("{}[]", base),
        TypeModifier::OptionalList => format!("{}[]?", base),
        TypeModifier::Required => base,
    }
}

fn format_attribute(attr: &prax_schema::ast::Attribute) -> String {
    // For model-level attributes we check if it's a known model attribute
    let prefix = if attr.is_model_attribute() { "@@" } else { "@" };

    if attr.args.is_empty() {
        format!("{}{}", prefix, attr.name())
    } else {
        let args: Vec<String> = attr
            .args
            .iter()
            .map(|arg| {
                if let Some(name) = &arg.name {
                    format!("{}: {}", name.as_str(), format_attribute_value(&arg.value))
                } else {
                    format_attribute_value(&arg.value)
                }
            })
            .collect();

        format!("{}{}({})", prefix, attr.name(), args.join(", "))
    }
}

fn format_attribute_value(value: &prax_schema::ast::AttributeValue) -> String {
    use prax_schema::ast::AttributeValue;

    match value {
        AttributeValue::String(s) => format!("\"{}\"", s),
        AttributeValue::Int(i) => i.to_string(),
        AttributeValue::Float(f) => f.to_string(),
        AttributeValue::Boolean(b) => b.to_string(),
        AttributeValue::Ident(id) => id.to_string(),
        AttributeValue::Function(name, args) => {
            if args.is_empty() {
                format!("{}()", name)
            } else {
                let arg_strs: Vec<String> = args.iter().map(format_attribute_value).collect();
                format!("{}({})", name, arg_strs.join(", "))
            }
        }
        AttributeValue::Array(items) => {
            let item_strs: Vec<String> = items.iter().map(format_attribute_value).collect();
            format!("[{}]", item_strs.join(", "))
        }
        AttributeValue::FieldRef(field) => field.to_string(),
        AttributeValue::FieldRefList(fields) => {
            format!(
                "[{}]",
                fields
                    .iter()
                    .map(|f| f.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    }
}
