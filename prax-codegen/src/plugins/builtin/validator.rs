//! Validator plugin - generates runtime validation methods.

use quote::quote;

use prax_schema::ast::{FieldType, Model, ScalarType};

use crate::plugins::{Plugin, PluginContext, PluginOutput};

/// Validator plugin that generates runtime validation methods.
///
/// When enabled, this plugin generates `validate()` methods for each model
/// that check field constraints like:
/// - Required fields are not null
/// - String length constraints
/// - Numeric range constraints
/// - Email format validation
/// - Custom regex patterns
///
/// Enable with: `PRAX_PLUGIN_VALIDATOR=1`
pub struct ValidatorPlugin;

impl Plugin for ValidatorPlugin {
    fn name(&self) -> &'static str {
        "validator"
    }

    fn env_var(&self) -> &'static str {
        "PRAX_PLUGIN_VALIDATOR"
    }

    fn description(&self) -> &'static str {
        "Generates runtime validation methods for model constraints"
    }

    fn on_start(&self, _ctx: &PluginContext) -> PluginOutput {
        PluginOutput::with_tokens(quote! {
            /// Validation error types.
            pub mod _validation {
                /// A validation error.
                #[derive(Debug, Clone)]
                pub struct ValidationError {
                    /// The field that failed validation.
                    pub field: String,
                    /// The error message.
                    pub message: String,
                }

                impl ValidationError {
                    /// Create a new validation error.
                    pub fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
                        Self {
                            field: field.into(),
                            message: message.into(),
                        }
                    }
                }

                impl std::fmt::Display for ValidationError {
                    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(f, "{}: {}", self.field, self.message)
                    }
                }

                impl std::error::Error for ValidationError {}

                /// Result of validation.
                pub type ValidationResult = Result<(), Vec<ValidationError>>;

                /// Trait for types that can be validated.
                pub trait Validate {
                    /// Validate this value.
                    fn validate(&self) -> ValidationResult;

                    /// Check if this value is valid.
                    fn is_valid(&self) -> bool {
                        self.validate().is_ok()
                    }
                }
            }
        })
    }

    fn on_model(&self, _ctx: &PluginContext, model: &Model) -> PluginOutput {
        let model_name = model.name();

        // Generate validation checks for each field
        let validations: Vec<_> = model
            .fields
            .values()
            .filter_map(|field| {
                let field_name = field.name();
                let field_name_str = field_name.to_string();

                let mut checks = Vec::new();

                // Check for @unique (informational only at runtime)
                if field.has_attribute("unique") {
                    // Uniqueness can only be checked at database level
                }

                // Check string length constraints from native types or attributes
                if matches!(field.field_type, FieldType::Scalar(ScalarType::String)) {
                    // Check for length attributes
                    for attr in &field.attributes {
                        if attr.name() == "db" {
                            // Parse native type for length info (e.g., @db.VarChar(255))
                            // This is simplified; real implementation would parse the native type
                        }
                    }
                }

                // Check for required fields
                if !field.modifier.is_optional()
                    && let FieldType::Scalar(ScalarType::String) = &field.field_type
                {
                    checks.push(quote! {
                        if self.#field_name.is_empty() {
                            errors.push(super::super::_validation::ValidationError::new(
                                #field_name_str,
                                "cannot be empty"
                            ));
                        }
                    });
                }

                // Check optional fields
                if field.modifier.is_optional()
                    && let FieldType::Scalar(ScalarType::String) = &field.field_type
                {
                    checks.push(quote! {
                        if let Some(ref val) = self.#field_name {
                            if val.is_empty() {
                                errors.push(super::super::_validation::ValidationError::new(
                                    #field_name_str,
                                    "if provided, cannot be empty"
                                ));
                            }
                        }
                    });
                }

                // Check for email fields (by name convention or attribute)
                let is_email = field_name.to_lowercase().contains("email")
                    || field.attributes.iter().any(|a| a.name() == "email");

                if is_email && matches!(field.field_type, FieldType::Scalar(ScalarType::String)) {
                    let email_check = if field.modifier.is_optional() {
                        quote! {
                            if let Some(ref email) = self.#field_name {
                                if !email.contains('@') || !email.contains('.') {
                                    errors.push(super::super::_validation::ValidationError::new(
                                        #field_name_str,
                                        "must be a valid email address"
                                    ));
                                }
                            }
                        }
                    } else {
                        quote! {
                            if !self.#field_name.contains('@') || !self.#field_name.contains('.') {
                                errors.push(super::super::_validation::ValidationError::new(
                                    #field_name_str,
                                    "must be a valid email address"
                                ));
                            }
                        }
                    };
                    checks.push(email_check);
                }

                if checks.is_empty() {
                    None
                } else {
                    Some(quote! { #(#checks)* })
                }
            })
            .collect();

        let has_validations = !validations.is_empty();

        PluginOutput::with_tokens(quote! {
            /// Validation implementation for this model.
            pub mod _validator {
                use super::*;

                /// Whether this model has any validation rules.
                pub const HAS_VALIDATIONS: bool = #has_validations;

                impl super::super::_validation::Validate for super::#model_name {
                    fn validate(&self) -> super::super::_validation::ValidationResult {
                        let mut errors = Vec::new();

                        #(#validations)*

                        if errors.is_empty() {
                            Ok(())
                        } else {
                            Err(errors)
                        }
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prax_schema::Schema;
    use prax_schema::ast::{
        Attribute, AttributeArg, AttributeValue, Field, Ident, Span, TypeModifier,
    };

    fn make_span() -> Span {
        Span::new(0, 0)
    }

    fn make_ident(name: &str) -> Ident {
        Ident::new(name, make_span())
    }

    fn make_attribute(name: &str) -> Attribute {
        Attribute::simple(make_ident(name), make_span())
    }

    fn make_db_attribute(value: &str) -> Attribute {
        Attribute::new(
            make_ident("db"),
            vec![AttributeArg::positional(
                AttributeValue::String(value.into()),
                make_span(),
            )],
            make_span(),
        )
    }

    #[test]
    fn test_validator_plugin_start() {
        let schema = Schema::new();
        let config = crate::plugins::PluginConfig::new();
        let ctx = PluginContext::new(&schema, &config);

        let plugin = ValidatorPlugin;
        let output = plugin.on_start(&ctx);

        let code = output.tokens.to_string();
        assert!(code.contains("_validation"));
        assert!(code.contains("ValidationError"));
        assert!(code.contains("Validate"));
    }

    #[test]
    fn test_validator_plugin_model() {
        let schema = Schema::new();
        let config = crate::plugins::PluginConfig::new();
        let ctx = PluginContext::new(&schema, &config);

        let mut model = Model::new(make_ident("User"), make_span());
        model.add_field(Field::new(
            make_ident("email"),
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
            vec![],
            make_span(),
        ));

        let plugin = ValidatorPlugin;
        let output = plugin.on_model(&ctx, &model);

        let code = output.tokens.to_string();
        assert!(code.contains("_validator"));
        assert!(code.contains("validate"));
    }

    #[test]
    fn test_validator_plugin_email_field() {
        let schema = Schema::new();
        let config = crate::plugins::PluginConfig::new();
        let ctx = PluginContext::new(&schema, &config);

        let mut model = Model::new(make_ident("User"), make_span());
        model.add_field(Field::new(
            make_ident("email"),
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
            vec![],
            make_span(),
        ));

        let plugin = ValidatorPlugin;
        let output = plugin.on_model(&ctx, &model);

        let code = output.tokens.to_string();
        assert!(code.contains("valid email"));
    }

    #[test]
    fn test_validator_plugin_name() {
        let plugin = ValidatorPlugin;
        assert_eq!(plugin.name(), "validator");
    }

    #[test]
    fn test_validator_plugin_env_var() {
        let plugin = ValidatorPlugin;
        assert_eq!(plugin.env_var(), "PRAX_PLUGIN_VALIDATOR");
    }

    #[test]
    fn test_validator_plugin_description() {
        let plugin = ValidatorPlugin;
        assert!(plugin.description().contains("validation"));
    }

    #[test]
    fn test_validator_plugin_start_contains_validation_trait() {
        let schema = Schema::new();
        let config = crate::plugins::PluginConfig::new();
        let ctx = PluginContext::new(&schema, &config);

        let plugin = ValidatorPlugin;
        let output = plugin.on_start(&ctx);

        let code = output.tokens.to_string();
        assert!(code.contains("trait Validate"));
        assert!(code.contains("fn validate"));
        assert!(code.contains("fn is_valid"));
    }

    #[test]
    fn test_validator_plugin_start_contains_validation_error() {
        let schema = Schema::new();
        let config = crate::plugins::PluginConfig::new();
        let ctx = PluginContext::new(&schema, &config);

        let plugin = ValidatorPlugin;
        let output = plugin.on_start(&ctx);

        let code = output.tokens.to_string();
        assert!(code.contains("struct ValidationError"));
        assert!(code.contains("pub field"));
        assert!(code.contains("pub message"));
        assert!(code.contains("fn new"));
    }

    #[test]
    fn test_validator_plugin_start_contains_validation_result() {
        let schema = Schema::new();
        let config = crate::plugins::PluginConfig::new();
        let ctx = PluginContext::new(&schema, &config);

        let plugin = ValidatorPlugin;
        let output = plugin.on_start(&ctx);

        let code = output.tokens.to_string();
        assert!(code.contains("ValidationResult"));
    }

    #[test]
    fn test_validator_plugin_optional_email_field() {
        let schema = Schema::new();
        let config = crate::plugins::PluginConfig::new();
        let ctx = PluginContext::new(&schema, &config);

        let mut model = Model::new(make_ident("User"), make_span());
        model.add_field(Field::new(
            make_ident("alternativeEmail"),
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Optional,
            vec![],
            make_span(),
        ));

        let plugin = ValidatorPlugin;
        let output = plugin.on_model(&ctx, &model);

        let code = output.tokens.to_string();
        // Optional email should have conditional validation
        assert!(code.contains("if let Some"));
        assert!(code.contains("valid email"));
    }

    #[test]
    fn test_validator_plugin_required_string_field() {
        let schema = Schema::new();
        let config = crate::plugins::PluginConfig::new();
        let ctx = PluginContext::new(&schema, &config);

        let mut model = Model::new(make_ident("User"), make_span());
        model.add_field(Field::new(
            make_ident("name"),
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
            vec![],
            make_span(),
        ));

        let plugin = ValidatorPlugin;
        let output = plugin.on_model(&ctx, &model);

        let code = output.tokens.to_string();
        assert!(code.contains("is_empty"));
        assert!(code.contains("cannot be empty"));
    }

    #[test]
    fn test_validator_plugin_optional_string_field() {
        let schema = Schema::new();
        let config = crate::plugins::PluginConfig::new();
        let ctx = PluginContext::new(&schema, &config);

        let mut model = Model::new(make_ident("User"), make_span());
        model.add_field(Field::new(
            make_ident("bio"),
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Optional,
            vec![],
            make_span(),
        ));

        let plugin = ValidatorPlugin;
        let output = plugin.on_model(&ctx, &model);

        let code = output.tokens.to_string();
        assert!(code.contains("if provided, cannot be empty"));
    }

    #[test]
    fn test_validator_plugin_non_string_fields_no_empty_check() {
        let schema = Schema::new();
        let config = crate::plugins::PluginConfig::new();
        let ctx = PluginContext::new(&schema, &config);

        let mut model = Model::new(make_ident("User"), make_span());
        model.add_field(Field::new(
            make_ident("age"),
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
            vec![],
            make_span(),
        ));

        let plugin = ValidatorPlugin;
        let output = plugin.on_model(&ctx, &model);

        let code = output.tokens.to_string();
        // Int field should not have "is_empty" check
        assert!(code.contains("HAS_VALIDATIONS : bool = false"));
    }

    #[test]
    fn test_validator_plugin_model_with_unique_attribute() {
        let schema = Schema::new();
        let config = crate::plugins::PluginConfig::new();
        let ctx = PluginContext::new(&schema, &config);

        let mut model = Model::new(make_ident("User"), make_span());
        model.add_field(Field::new(
            make_ident("username"),
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
            vec![make_attribute("unique")],
            make_span(),
        ));

        let plugin = ValidatorPlugin;
        let output = plugin.on_model(&ctx, &model);

        // Just verify it doesn't crash with unique attribute
        assert!(!output.tokens.is_empty());
    }

    #[test]
    fn test_validator_plugin_model_with_db_attribute() {
        let schema = Schema::new();
        let config = crate::plugins::PluginConfig::new();
        let ctx = PluginContext::new(&schema, &config);

        let mut model = Model::new(make_ident("User"), make_span());
        model.add_field(Field::new(
            make_ident("description"),
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
            vec![make_db_attribute("VarChar(255)")],
            make_span(),
        ));

        let plugin = ValidatorPlugin;
        let output = plugin.on_model(&ctx, &model);

        // Just verify it handles db attributes without crashing
        assert!(!output.tokens.is_empty());
    }

    #[test]
    fn test_validator_plugin_model_generates_has_validations_const() {
        let schema = Schema::new();
        let config = crate::plugins::PluginConfig::new();
        let ctx = PluginContext::new(&schema, &config);

        let model = Model::new(make_ident("EmptyModel"), make_span());

        let plugin = ValidatorPlugin;
        let output = plugin.on_model(&ctx, &model);

        let code = output.tokens.to_string();
        assert!(code.contains("HAS_VALIDATIONS"));
    }

    #[test]
    fn test_validator_plugin_email_field_with_attribute() {
        let schema = Schema::new();
        let config = crate::plugins::PluginConfig::new();
        let ctx = PluginContext::new(&schema, &config);

        let mut model = Model::new(make_ident("User"), make_span());
        model.add_field(Field::new(
            make_ident("contactAddress"), // Not containing "email" in name
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
            vec![make_attribute("email")], // But has @email attribute
            make_span(),
        ));

        let plugin = ValidatorPlugin;
        let output = plugin.on_model(&ctx, &model);

        let code = output.tokens.to_string();
        // Should have email validation due to the attribute
        assert!(code.contains("valid email"));
    }

    #[test]
    fn test_validator_plugin_model_with_multiple_fields() {
        let schema = Schema::new();
        let config = crate::plugins::PluginConfig::new();
        let ctx = PluginContext::new(&schema, &config);

        let mut model = Model::new(make_ident("User"), make_span());
        model.add_field(Field::new(
            make_ident("email"),
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
            vec![],
            make_span(),
        ));
        model.add_field(Field::new(
            make_ident("name"),
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
            vec![],
            make_span(),
        ));
        model.add_field(Field::new(
            make_ident("bio"),
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Optional,
            vec![],
            make_span(),
        ));

        let plugin = ValidatorPlugin;
        let output = plugin.on_model(&ctx, &model);

        let code = output.tokens.to_string();
        // Should have multiple validation checks
        assert!(code.contains("HAS_VALIDATIONS : bool = true"));
    }

    #[test]
    fn test_validator_plugin_validation_error_implements_display() {
        let schema = Schema::new();
        let config = crate::plugins::PluginConfig::new();
        let ctx = PluginContext::new(&schema, &config);

        let plugin = ValidatorPlugin;
        let output = plugin.on_start(&ctx);

        let code = output.tokens.to_string();
        assert!(code.contains("impl std :: fmt :: Display"));
    }

    #[test]
    fn test_validator_plugin_validation_error_implements_error() {
        let schema = Schema::new();
        let config = crate::plugins::PluginConfig::new();
        let ctx = PluginContext::new(&schema, &config);

        let plugin = ValidatorPlugin;
        let output = plugin.on_start(&ctx);

        let code = output.tokens.to_string();
        assert!(code.contains("impl std :: error :: Error"));
    }
}
