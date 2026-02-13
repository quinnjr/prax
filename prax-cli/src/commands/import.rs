//! Import schemas from Prisma, Diesel, or SeaORM.

use std::fs;
use std::path::Path;

use prax_import::prelude::*;

use crate::cli::{ImportArgs, ImportSource};
use crate::config::SCHEMA_FILE_PATH;
use crate::error::{CliError, CliResult};
use crate::output;

/// Run the import command.
pub async fn run(args: ImportArgs) -> CliResult<()> {
    output::info(&format!(
        "Importing schema from {} → {}",
        match args.from {
            ImportSource::Prisma => "Prisma",
            ImportSource::Diesel => "Diesel",
            ImportSource::SeaOrm => "SeaORM",
        },
        args.output
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "stdout".to_string())
    ));

    // Check if input file exists
    if !args.input.exists() {
        return Err(CliError::Config(format!(
            "Input file not found: {}",
            args.input.display()
        )));
    }

    // Import the schema
    let prax_schema = match args.from {
        ImportSource::Prisma => import_from_prisma(&args.input)?,
        ImportSource::Diesel => import_from_diesel(&args.input)?,
        ImportSource::SeaOrm => import_from_seaorm(&args.input)?,
    };

    output::success(&format!(
        "✓ Successfully imported {} models, {} enums",
        prax_schema.models.len(),
        prax_schema.enums.len()
    ));

    // Format the schema as text
    let schema_text = format_schema(&prax_schema);

    // Output the result
    if args.print {
        // Print to stdout
        println!("{}", schema_text);
    } else {
        // Determine output path
        let output_path = args.output.unwrap_or_else(|| {
            // Default to prax/schema.prax
            std::path::PathBuf::from(SCHEMA_FILE_PATH)
        });

        // Check if file exists and prompt if not forcing
        if output_path.exists() && !args.force {
            return Err(CliError::Config(format!(
                "Output file already exists: {}. Use --force to overwrite.",
                output_path.display()
            )));
        }

        // Write the schema to file
        fs::write(&output_path, schema_text).map_err(|e| {
            CliError::Config(format!(
                "Failed to write schema to {}: {}",
                output_path.display(),
                e
            ))
        })?;

        output::success(&format!("✓ Schema written to {}", output_path.display()));
    }

    // Print helpful next steps
    output::newline();
    output::info("Next steps:");
    output::info("  1. Review the generated schema file");
    output::info("  2. Run `prax validate` to check for any issues");
    output::info("  3. Run `prax generate` to generate Rust client code");
    output::info("  4. Run `prax migrate dev` to apply migrations");

    Ok(())
}

/// Import from a Prisma schema file.
fn import_from_prisma(input: &Path) -> CliResult<prax_schema::Schema> {
    output::info(&format!("Reading Prisma schema from {}", input.display()));

    import_prisma_schema_file(input)
        .map_err(|e| CliError::Schema(format!("Failed to import Prisma schema: {}", e)))
}

/// Import from a Diesel schema file.
fn import_from_diesel(input: &Path) -> CliResult<prax_schema::Schema> {
    output::info(&format!("Reading Diesel schema from {}", input.display()));

    import_diesel_schema_file(input)
        .map_err(|e| CliError::Schema(format!("Failed to import Diesel schema: {}", e)))
}

/// Import from a SeaORM entity file.
fn import_from_seaorm(input: &Path) -> CliResult<prax_schema::Schema> {
    output::info(&format!("Reading SeaORM entity from {}", input.display()));

    #[cfg(feature = "seaorm")]
    {
        use prax_import::seaorm::import_seaorm_entity_file;
        import_seaorm_entity_file(input)
            .map_err(|e| CliError::Schema(format!("Failed to import SeaORM entity: {}", e)))
    }

    #[cfg(not(feature = "seaorm"))]
    {
        Err(CliError::Config(
            "SeaORM import support not enabled. Rebuild with --features seaorm".to_string(),
        ))
    }
}

/// Format a Prax schema as a string.
///
/// This is a simple formatter that outputs the schema in Prax DSL format.
/// TODO: Use prax-schema's built-in formatter when available.
fn format_schema(schema: &prax_schema::Schema) -> String {
    let mut output = String::new();

    // Add datasource if present
    if let Some(datasource) = &schema.datasource {
        output.push_str(&format!(
            "datasource db {{\n  provider = \"{}\"\n",
            datasource.provider.as_str()
        ));

        if let Some(url) = &datasource.url {
            output.push_str(&format!("  url      = \"{}\"\n", url));
        }

        output.push_str("}\n\n");
    }

    // Add enums
    for (_, enum_def) in &schema.enums {
        output.push_str(&format!("enum {} {{\n", enum_def.name()));

        for variant in &enum_def.variants {
            output.push_str(&format!("  {}\n", variant.name()));
        }

        output.push_str("}\n\n");
    }

    // Add models
    for (_, model) in &schema.models {
        if let Some(doc) = &model.documentation {
            for line in doc.text.lines() {
                output.push_str(&format!("/// {}\n", line));
            }
        }

        output.push_str(&format!("model {} {{\n", model.name()));

        // Add fields
        for (_, field) in &model.fields {
            if let Some(doc) = &field.documentation {
                for line in doc.text.lines() {
                    output.push_str(&format!("  /// {}\n", line));
                }
            }

            let field_name = field.name();
            let field_type = format_field_type(&field.field_type, &field.modifier);

            output.push_str(&format!("  {} {}", field_name, field_type));

            // Add attributes
            for attr in &field.attributes {
                output.push_str(&format!(" @{}", attr.name.as_str()));

                // Add attribute arguments if present
                if !attr.args.is_empty() {
                    output.push('(');
                    for (i, arg) in attr.args.iter().enumerate() {
                        if i > 0 {
                            output.push_str(", ");
                        }
                        if let Some(name) = &arg.name {
                            output.push_str(&format!("{}: ", name.as_str()));
                        }
                        output.push_str(&format_attribute_value(&arg.value));
                    }
                    output.push(')');
                }
            }

            output.push('\n');
        }

        // Add model attributes
        for attr in &model.attributes {
            output.push_str(&format!("  @@{}", attr.name.as_str()));

            if !attr.args.is_empty() {
                output.push('(');
                for (i, arg) in attr.args.iter().enumerate() {
                    if i > 0 {
                        output.push_str(", ");
                    }
                    if let Some(name) = &arg.name {
                        output.push_str(&format!("{}: ", name.as_str()));
                    }
                    output.push_str(&format_attribute_value(&arg.value));
                }
                output.push(')');
            }

            output.push('\n');
        }

        output.push_str("}\n\n");
    }

    output
}

/// Format a field type with its modifier.
fn format_field_type(
    field_type: &prax_schema::FieldType,
    modifier: &prax_schema::TypeModifier,
) -> String {
    let base = field_type.type_name().to_string();

    match modifier {
        prax_schema::TypeModifier::Required => base,
        prax_schema::TypeModifier::Optional => format!("{}?", base),
        prax_schema::TypeModifier::List => format!("{}[]", base),
        prax_schema::TypeModifier::OptionalList => format!("{}[]?", base),
    }
}

/// Format an attribute value.
fn format_attribute_value(value: &prax_schema::AttributeValue) -> String {
    use prax_schema::AttributeValue;

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
                let args_str = args
                    .iter()
                    .map(format_attribute_value)
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{}({})", name, args_str)
            }
        }
        AttributeValue::Array(items) => {
            let items_str = items
                .iter()
                .map(format_attribute_value)
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{}]", items_str)
        }
        AttributeValue::FieldRef(name) => name.to_string(),
        AttributeValue::FieldRefList(names) => {
            let names_str = names
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{}]", names_str)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_field_type() {
        use prax_schema::{FieldType, ScalarType, TypeModifier};

        assert_eq!(
            format_field_type(
                &FieldType::Scalar(ScalarType::String),
                &TypeModifier::Required
            ),
            "String"
        );

        assert_eq!(
            format_field_type(&FieldType::Scalar(ScalarType::Int), &TypeModifier::Optional),
            "Int?"
        );

        assert_eq!(
            format_field_type(&FieldType::Scalar(ScalarType::String), &TypeModifier::List),
            "String[]"
        );
    }
}
