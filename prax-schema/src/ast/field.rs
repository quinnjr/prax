//! Field definitions for the Prax schema AST.

use serde::{Deserialize, Serialize};

use super::{
    Attribute, Documentation, EnhancedDocumentation, FieldAttributes, FieldType, FieldValidation,
    Ident, Span, TypeModifier, ValidationRule, ValidationType,
};

/// A field in a model or composite type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Field {
    /// Field name.
    pub name: Ident,
    /// Field type.
    pub field_type: FieldType,
    /// Type modifier (optional, list, etc.).
    pub modifier: TypeModifier,
    /// Raw attributes as parsed.
    pub attributes: Vec<Attribute>,
    /// Documentation comment.
    pub documentation: Option<Documentation>,
    /// Validation rules for this field.
    pub validation: FieldValidation,
    /// Source location.
    pub span: Span,
}

impl Field {
    /// Create a new field.
    pub fn new(
        name: Ident,
        field_type: FieldType,
        modifier: TypeModifier,
        attributes: Vec<Attribute>,
        span: Span,
    ) -> Self {
        Self {
            name,
            field_type,
            modifier,
            attributes,
            documentation: None,
            validation: FieldValidation::new(),
            span,
        }
    }

    /// Get the field name as a string.
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    /// Check if the field is optional.
    pub fn is_optional(&self) -> bool {
        self.modifier.is_optional()
    }

    /// Check if the field is a list.
    pub fn is_list(&self) -> bool {
        self.modifier.is_list()
    }

    /// Check if this field has a specific attribute.
    pub fn has_attribute(&self, name: &str) -> bool {
        self.attributes.iter().any(|a| a.is(name))
    }

    /// Get an attribute by name.
    pub fn get_attribute(&self, name: &str) -> Option<&Attribute> {
        self.attributes.iter().find(|a| a.is(name))
    }

    /// Check if this is a primary key field.
    pub fn is_id(&self) -> bool {
        self.has_attribute("id")
    }

    /// Check if this field has a unique constraint.
    pub fn is_unique(&self) -> bool {
        self.has_attribute("unique")
    }

    /// Check if this is a relation field.
    pub fn is_relation(&self) -> bool {
        self.field_type.is_relation() || self.has_attribute("relation")
    }

    /// Extract structured field attributes.
    pub fn extract_attributes(&self) -> FieldAttributes {
        let mut attrs = FieldAttributes::default();

        for attr in &self.attributes {
            match attr.name() {
                "id" => attrs.is_id = true,
                "auto" => attrs.is_auto = true,
                "unique" => attrs.is_unique = true,
                "index" => attrs.is_indexed = true,
                "updated_at" => attrs.is_updated_at = true,
                "omit" => attrs.is_omit = true,
                "default" => {
                    attrs.default = attr.first_arg().cloned();
                }
                "map" => {
                    if let Some(val) = attr.first_arg() {
                        attrs.map = val.as_string().map(String::from);
                    }
                }
                "db" => {
                    // Parse native type like @db.VarChar(255)
                    if let Some(val) = attr.first_arg() {
                        if let super::AttributeValue::Function(name, args) = val {
                            attrs.native_type =
                                Some(super::NativeType::new(name.clone(), args.clone()));
                        } else if let Some(name) = val.as_ident() {
                            attrs.native_type = Some(super::NativeType::new(name, vec![]));
                        }
                    }
                }
                "relation" => {
                    // Parse relation attributes
                    let mut rel = super::RelationAttribute {
                        name: None,
                        fields: vec![],
                        references: vec![],
                        on_delete: None,
                        on_update: None,
                        map: None,
                    };

                    // First positional arg is the relation name
                    if let Some(val) = attr.first_arg() {
                        rel.name = val.as_string().map(String::from);
                    }

                    // Named arguments
                    if let Some(super::AttributeValue::FieldRefList(fields)) =
                        attr.get_arg("fields")
                    {
                        rel.fields = fields.clone();
                    }
                    if let Some(super::AttributeValue::FieldRefList(refs)) =
                        attr.get_arg("references")
                    {
                        rel.references = refs.clone();
                    }
                    if let Some(val) = attr.get_arg("onDelete")
                        && let Some(action) = val.as_ident()
                    {
                        rel.on_delete = super::ReferentialAction::from_str(action);
                    }
                    if let Some(val) = attr.get_arg("onUpdate")
                        && let Some(action) = val.as_ident()
                    {
                        rel.on_update = super::ReferentialAction::from_str(action);
                    }
                    if let Some(val) = attr.get_arg("map") {
                        rel.map = val.as_string().map(String::from);
                    }

                    attrs.relation = Some(rel);
                }
                _ => {}
            }
        }

        attrs
    }

    /// Set documentation.
    pub fn with_documentation(mut self, doc: Documentation) -> Self {
        self.documentation = Some(doc);
        self
    }

    /// Set enhanced documentation (with validation extraction).
    pub fn with_enhanced_documentation(mut self, doc: EnhancedDocumentation) -> Self {
        self.documentation = Some(Documentation::new(&doc.text, doc.span));
        // Merge validation rules from documentation
        for rule in doc.validation.rules {
            self.validation.add_rule(rule);
        }
        self
    }

    /// Set validation rules.
    pub fn with_validation(mut self, validation: FieldValidation) -> Self {
        self.validation = validation;
        self
    }

    /// Add a validation rule.
    pub fn add_validation_rule(&mut self, rule: ValidationRule) {
        self.validation.add_rule(rule);
    }

    /// Check if this field has any validation rules.
    pub fn has_validation(&self) -> bool {
        !self.validation.is_empty()
    }

    /// Get all validation rules for this field.
    pub fn validation_rules(&self) -> &[ValidationRule] {
        &self.validation.rules
    }

    /// Check if this field is required (via validation).
    pub fn is_validated_required(&self) -> bool {
        self.validation.is_required()
    }

    /// Extract validation rules from @validate attributes.
    ///
    /// This parses attributes like:
    /// - `@validate.email`
    /// - `@validate.minLength(5)`
    /// - `@validate.range(0, 100)`
    pub fn extract_validation_from_attributes(&mut self) {
        for attr in &self.attributes {
            let attr_name = attr.name();

            // Check for @validate prefix
            if let Some(validator_name) = attr_name.strip_prefix("validate.") {
                if let Some(rule) = self.parse_validate_attribute(validator_name, attr) {
                    self.validation.add_rule(rule);
                }
            } else if attr_name == "validate" {
                // Parse @validate(email, minLength(5), ...)
                for arg in &attr.args {
                    if let Some(rule) = self.parse_validate_arg(arg) {
                        self.validation.add_rule(rule);
                    }
                }
            }
        }
    }

    /// Parse a single @validate.* attribute.
    fn parse_validate_attribute(
        &self,
        validator_name: &str,
        attr: &Attribute,
    ) -> Option<ValidationRule> {
        let span = attr.span;

        // Parse based on validator name
        let rule_type = match validator_name {
            // String validators
            "email" => ValidationType::Email,
            "url" => ValidationType::Url,
            "uuid" => ValidationType::Uuid,
            "cuid" => ValidationType::Cuid,
            "cuid2" => ValidationType::Cuid2,
            "nanoid" | "nanoId" | "NanoId" => ValidationType::NanoId,
            "ulid" => ValidationType::Ulid,
            "alpha" => ValidationType::Alpha,
            "alphanumeric" => ValidationType::Alphanumeric,
            "lowercase" => ValidationType::Lowercase,
            "uppercase" => ValidationType::Uppercase,
            "trim" => ValidationType::Trim,
            "noWhitespace" => ValidationType::NoWhitespace,
            "ip" => ValidationType::Ip,
            "ipv4" => ValidationType::Ipv4,
            "ipv6" => ValidationType::Ipv6,
            "creditCard" => ValidationType::CreditCard,
            "phone" => ValidationType::Phone,
            "slug" => ValidationType::Slug,
            "hex" => ValidationType::Hex,
            "base64" => ValidationType::Base64,
            "json" => ValidationType::Json,

            // Numeric validators
            "positive" => ValidationType::Positive,
            "negative" => ValidationType::Negative,
            "nonNegative" => ValidationType::NonNegative,
            "nonPositive" => ValidationType::NonPositive,
            "integer" => ValidationType::Integer,
            "finite" => ValidationType::Finite,

            // Array validators
            "unique" => ValidationType::Unique,
            "nonEmpty" => ValidationType::NonEmpty,

            // Date validators
            "past" => ValidationType::Past,
            "future" => ValidationType::Future,
            "pastOrPresent" => ValidationType::PastOrPresent,
            "futureOrPresent" => ValidationType::FutureOrPresent,

            // General validators
            "required" => ValidationType::Required,
            "notEmpty" => ValidationType::NotEmpty,

            // Validators with arguments
            "minLength" => {
                let n = attr.first_arg()?.as_int()? as usize;
                ValidationType::MinLength(n)
            }
            "maxLength" => {
                let n = attr.first_arg()?.as_int()? as usize;
                ValidationType::MaxLength(n)
            }
            "length" => {
                let args = &attr.args;
                if args.len() >= 2 {
                    let min = args[0].value.as_int()? as usize;
                    let max = args[1].value.as_int()? as usize;
                    ValidationType::Length { min, max }
                } else {
                    return None;
                }
            }
            "min" => {
                let n = attr
                    .first_arg()?
                    .as_float()
                    .or_else(|| attr.first_arg()?.as_int().map(|i| i as f64))?;
                ValidationType::Min(n)
            }
            "max" => {
                let n = attr
                    .first_arg()?
                    .as_float()
                    .or_else(|| attr.first_arg()?.as_int().map(|i| i as f64))?;
                ValidationType::Max(n)
            }
            "range" => {
                let args = &attr.args;
                if args.len() >= 2 {
                    let min = args[0]
                        .value
                        .as_float()
                        .or_else(|| args[0].value.as_int().map(|i| i as f64))?;
                    let max = args[1]
                        .value
                        .as_float()
                        .or_else(|| args[1].value.as_int().map(|i| i as f64))?;
                    ValidationType::Range { min, max }
                } else {
                    return None;
                }
            }
            "regex" => {
                let pattern = attr.first_arg()?.as_string()?.to_string();
                ValidationType::Regex(pattern)
            }
            "startsWith" => {
                let prefix = attr.first_arg()?.as_string()?.to_string();
                ValidationType::StartsWith(prefix)
            }
            "endsWith" => {
                let suffix = attr.first_arg()?.as_string()?.to_string();
                ValidationType::EndsWith(suffix)
            }
            "contains" => {
                let substring = attr.first_arg()?.as_string()?.to_string();
                ValidationType::Contains(substring)
            }
            "minItems" => {
                let n = attr.first_arg()?.as_int()? as usize;
                ValidationType::MinItems(n)
            }
            "maxItems" => {
                let n = attr.first_arg()?.as_int()? as usize;
                ValidationType::MaxItems(n)
            }
            "items" => {
                let args = &attr.args;
                if args.len() >= 2 {
                    let min = args[0].value.as_int()? as usize;
                    let max = args[1].value.as_int()? as usize;
                    ValidationType::Items { min, max }
                } else {
                    return None;
                }
            }
            "multipleOf" => {
                let n = attr
                    .first_arg()?
                    .as_float()
                    .or_else(|| attr.first_arg()?.as_int().map(|i| i as f64))?;
                ValidationType::MultipleOf(n)
            }
            "after" => {
                let date = attr.first_arg()?.as_string()?.to_string();
                ValidationType::After(date)
            }
            "before" => {
                let date = attr.first_arg()?.as_string()?.to_string();
                ValidationType::Before(date)
            }
            "custom" => {
                let name = attr.first_arg()?.as_string()?.to_string();
                ValidationType::Custom(name)
            }
            _ => return None,
        };

        Some(ValidationRule::new(rule_type, span))
    }

    /// Parse a @validate(...) argument.
    fn parse_validate_arg(&self, arg: &super::AttributeArg) -> Option<ValidationRule> {
        let span = arg.span;

        match &arg.value {
            super::AttributeValue::Ident(name) => {
                // Simple validators like @validate(email, uuid)
                let rule_type = match name.as_str() {
                    "email" => ValidationType::Email,
                    "url" => ValidationType::Url,
                    "uuid" => ValidationType::Uuid,
                    "cuid" => ValidationType::Cuid,
                    "cuid2" => ValidationType::Cuid2,
                    "nanoid" | "nanoId" | "NanoId" => ValidationType::NanoId,
                    "ulid" => ValidationType::Ulid,
                    "alpha" => ValidationType::Alpha,
                    "alphanumeric" => ValidationType::Alphanumeric,
                    "lowercase" => ValidationType::Lowercase,
                    "uppercase" => ValidationType::Uppercase,
                    "trim" => ValidationType::Trim,
                    "noWhitespace" => ValidationType::NoWhitespace,
                    "ip" => ValidationType::Ip,
                    "ipv4" => ValidationType::Ipv4,
                    "ipv6" => ValidationType::Ipv6,
                    "creditCard" => ValidationType::CreditCard,
                    "phone" => ValidationType::Phone,
                    "slug" => ValidationType::Slug,
                    "hex" => ValidationType::Hex,
                    "base64" => ValidationType::Base64,
                    "json" => ValidationType::Json,
                    "positive" => ValidationType::Positive,
                    "negative" => ValidationType::Negative,
                    "nonNegative" => ValidationType::NonNegative,
                    "nonPositive" => ValidationType::NonPositive,
                    "integer" => ValidationType::Integer,
                    "finite" => ValidationType::Finite,
                    "unique" => ValidationType::Unique,
                    "nonEmpty" => ValidationType::NonEmpty,
                    "past" => ValidationType::Past,
                    "future" => ValidationType::Future,
                    "pastOrPresent" => ValidationType::PastOrPresent,
                    "futureOrPresent" => ValidationType::FutureOrPresent,
                    "required" => ValidationType::Required,
                    "notEmpty" => ValidationType::NotEmpty,
                    _ => return None,
                };
                Some(ValidationRule::new(rule_type, span))
            }
            super::AttributeValue::Function(name, args) => {
                // Validators with args like @validate(minLength(5))
                let rule_type = match name.as_str() {
                    "minLength" => {
                        let n = args.first()?.as_int()? as usize;
                        ValidationType::MinLength(n)
                    }
                    "maxLength" => {
                        let n = args.first()?.as_int()? as usize;
                        ValidationType::MaxLength(n)
                    }
                    "min" => {
                        let n = args
                            .first()?
                            .as_float()
                            .or_else(|| args.first()?.as_int().map(|i| i as f64))?;
                        ValidationType::Min(n)
                    }
                    "max" => {
                        let n = args
                            .first()?
                            .as_float()
                            .or_else(|| args.first()?.as_int().map(|i| i as f64))?;
                        ValidationType::Max(n)
                    }
                    "range" => {
                        if args.len() >= 2 {
                            let min = args[0]
                                .as_float()
                                .or_else(|| args[0].as_int().map(|i| i as f64))?;
                            let max = args[1]
                                .as_float()
                                .or_else(|| args[1].as_int().map(|i| i as f64))?;
                            ValidationType::Range { min, max }
                        } else {
                            return None;
                        }
                    }
                    "regex" => {
                        let pattern = args.first()?.as_string()?.to_string();
                        ValidationType::Regex(pattern)
                    }
                    "custom" => {
                        let validator_name = args.first()?.as_string()?.to_string();
                        ValidationType::Custom(validator_name)
                    }
                    _ => return None,
                };
                Some(ValidationRule::new(rule_type, span))
            }
            _ => None,
        }
    }
}

impl std::fmt::Display for Field {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)?;

        // Type with modifier
        match self.modifier {
            TypeModifier::Required => write!(f, " {}", self.field_type)?,
            TypeModifier::Optional => write!(f, " {}?", self.field_type)?,
            TypeModifier::List => write!(f, " {}[]", self.field_type)?,
            TypeModifier::OptionalList => write!(f, " {}[]?", self.field_type)?,
        }

        // Attributes
        for attr in &self.attributes {
            write!(f, " @{}", attr.name)?;
            if !attr.args.is_empty() {
                write!(f, "(...)")?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
// Test names mirror the camelCase Prax validator attributes they cover
// (e.g. `@nonEmpty`, `@creditCard`); keep the casing for readability.
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use crate::ast::{AttributeArg, AttributeValue, ReferentialAction, ScalarType};

    fn make_span() -> Span {
        Span::new(0, 10)
    }

    fn make_field(name: &str, field_type: FieldType, modifier: TypeModifier) -> Field {
        Field::new(
            Ident::new(name, make_span()),
            field_type,
            modifier,
            vec![],
            make_span(),
        )
    }

    fn make_attribute(name: &str) -> Attribute {
        Attribute::simple(Ident::new(name, make_span()), make_span())
    }

    fn make_attribute_with_arg(name: &str, value: AttributeValue) -> Attribute {
        Attribute::new(
            Ident::new(name, make_span()),
            vec![AttributeArg::positional(value, make_span())],
            make_span(),
        )
    }

    // ==================== Field Construction Tests ====================

    #[test]
    fn test_field_new() {
        let field = Field::new(
            Ident::new("id", make_span()),
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
            vec![],
            make_span(),
        );

        assert_eq!(field.name(), "id");
        assert!(field.field_type.is_scalar());
        assert_eq!(field.modifier, TypeModifier::Required);
        assert!(field.attributes.is_empty());
        assert!(field.documentation.is_none());
    }

    #[test]
    fn test_field_with_attributes() {
        let field = Field::new(
            Ident::new("email", make_span()),
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
            vec![make_attribute("unique")],
            make_span(),
        );

        assert_eq!(field.attributes.len(), 1);
    }

    #[test]
    fn test_field_with_documentation() {
        let field = make_field(
            "name",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Optional,
        )
        .with_documentation(Documentation::new("User's display name", make_span()));

        assert!(field.documentation.is_some());
        assert_eq!(field.documentation.unwrap().text, "User's display name");
    }

    // ==================== Field Name Tests ====================

    #[test]
    fn test_field_name() {
        let field = make_field(
            "created_at",
            FieldType::Scalar(ScalarType::DateTime),
            TypeModifier::Required,
        );
        assert_eq!(field.name(), "created_at");
    }

    // ==================== Field Modifier Tests ====================

    #[test]
    fn test_field_is_optional_required() {
        let field = make_field(
            "id",
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
        );
        assert!(!field.is_optional());
    }

    #[test]
    fn test_field_is_optional_true() {
        let field = make_field(
            "bio",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Optional,
        );
        assert!(field.is_optional());
    }

    #[test]
    fn test_field_is_list_false() {
        let field = make_field(
            "name",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        assert!(!field.is_list());
    }

    #[test]
    fn test_field_is_list_true() {
        let field = make_field(
            "tags",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::List,
        );
        assert!(field.is_list());
    }

    #[test]
    fn test_field_optional_list() {
        let field = make_field(
            "metadata",
            FieldType::Scalar(ScalarType::Json),
            TypeModifier::OptionalList,
        );
        assert!(field.is_optional());
        assert!(field.is_list());
    }

    // ==================== Field Attribute Tests ====================

    #[test]
    fn test_field_has_attribute_true() {
        let mut field = make_field(
            "id",
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
        );
        field.attributes.push(make_attribute("id"));
        field.attributes.push(make_attribute("auto"));

        assert!(field.has_attribute("id"));
        assert!(field.has_attribute("auto"));
    }

    #[test]
    fn test_field_has_attribute_false() {
        let field = make_field(
            "name",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        assert!(!field.has_attribute("unique"));
    }

    #[test]
    fn test_field_get_attribute() {
        let mut field = make_field(
            "email",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(make_attribute("unique"));

        let attr = field.get_attribute("unique");
        assert!(attr.is_some());
        assert!(attr.unwrap().is("unique"));

        assert!(field.get_attribute("id").is_none());
    }

    #[test]
    fn test_field_is_id_true() {
        let mut field = make_field(
            "id",
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
        );
        field.attributes.push(make_attribute("id"));
        assert!(field.is_id());
    }

    #[test]
    fn test_field_is_id_false() {
        let field = make_field(
            "email",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        assert!(!field.is_id());
    }

    #[test]
    fn test_field_is_unique_true() {
        let mut field = make_field(
            "email",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(make_attribute("unique"));
        assert!(field.is_unique());
    }

    #[test]
    fn test_field_is_unique_false() {
        let field = make_field(
            "name",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        assert!(!field.is_unique());
    }

    // ==================== Field Relation Tests ====================

    #[test]
    fn test_field_is_relation_by_type() {
        let field = make_field(
            "author",
            FieldType::Model("User".into()),
            TypeModifier::Required,
        );
        assert!(field.is_relation());
    }

    #[test]
    fn test_field_is_relation_by_attribute() {
        let mut field = make_field(
            "author_id",
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
        );
        field.attributes.push(make_attribute("relation"));
        assert!(field.is_relation());
    }

    #[test]
    fn test_field_is_relation_list() {
        let field = make_field("posts", FieldType::Model("Post".into()), TypeModifier::List);
        assert!(field.is_relation());
        assert!(field.is_list());
    }

    // ==================== Extract Attributes Tests ====================

    #[test]
    fn test_extract_attributes_empty() {
        let field = make_field(
            "name",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        let attrs = field.extract_attributes();

        assert!(!attrs.is_id);
        assert!(!attrs.is_auto);
        assert!(!attrs.is_unique);
        assert!(!attrs.is_indexed);
        assert!(!attrs.is_updated_at);
        assert!(!attrs.is_omit);
        assert!(attrs.default.is_none());
        assert!(attrs.map.is_none());
        assert!(attrs.native_type.is_none());
        assert!(attrs.relation.is_none());
    }

    #[test]
    fn test_extract_attributes_id_and_auto() {
        let mut field = make_field(
            "id",
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
        );
        field.attributes.push(make_attribute("id"));
        field.attributes.push(make_attribute("auto"));

        let attrs = field.extract_attributes();
        assert!(attrs.is_id);
        assert!(attrs.is_auto);
    }

    #[test]
    fn test_extract_attributes_unique() {
        let mut field = make_field(
            "email",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(make_attribute("unique"));

        let attrs = field.extract_attributes();
        assert!(attrs.is_unique);
    }

    #[test]
    fn test_extract_attributes_index() {
        let mut field = make_field(
            "name",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(make_attribute("index"));

        let attrs = field.extract_attributes();
        assert!(attrs.is_indexed);
    }

    #[test]
    fn test_extract_attributes_updated_at() {
        let mut field = make_field(
            "updated_at",
            FieldType::Scalar(ScalarType::DateTime),
            TypeModifier::Required,
        );
        field.attributes.push(make_attribute("updated_at"));

        let attrs = field.extract_attributes();
        assert!(attrs.is_updated_at);
    }

    #[test]
    fn test_extract_attributes_omit() {
        let mut field = make_field(
            "password_hash",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(make_attribute("omit"));

        let attrs = field.extract_attributes();
        assert!(attrs.is_omit);
    }

    #[test]
    fn test_extract_attributes_default_int() {
        let mut field = make_field(
            "count",
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
        );
        field
            .attributes
            .push(make_attribute_with_arg("default", AttributeValue::Int(0)));

        let attrs = field.extract_attributes();
        assert!(attrs.default.is_some());
        assert_eq!(attrs.default.as_ref().unwrap().as_int(), Some(0));
    }

    #[test]
    fn test_extract_attributes_default_function() {
        let mut field = make_field(
            "created_at",
            FieldType::Scalar(ScalarType::DateTime),
            TypeModifier::Required,
        );
        field.attributes.push(make_attribute_with_arg(
            "default",
            AttributeValue::Function("now".into(), vec![]),
        ));

        let attrs = field.extract_attributes();
        assert!(attrs.default.is_some());
        if let AttributeValue::Function(name, _) = attrs.default.as_ref().unwrap() {
            assert_eq!(name.as_str(), "now");
        } else {
            panic!("Expected Function");
        }
    }

    #[test]
    fn test_extract_attributes_map() {
        let mut field = make_field(
            "email",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(make_attribute_with_arg(
            "map",
            AttributeValue::String("email_address".into()),
        ));

        let attrs = field.extract_attributes();
        assert_eq!(attrs.map, Some("email_address".to_string()));
    }

    #[test]
    fn test_extract_attributes_native_type_ident() {
        let mut field = make_field(
            "data",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(make_attribute_with_arg(
            "db",
            AttributeValue::Ident("Text".into()),
        ));

        let attrs = field.extract_attributes();
        assert!(attrs.native_type.is_some());
        let nt = attrs.native_type.unwrap();
        assert_eq!(nt.name.as_str(), "Text");
        assert!(nt.args.is_empty());
    }

    #[test]
    fn test_extract_attributes_native_type_function() {
        let mut field = make_field(
            "name",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(make_attribute_with_arg(
            "db",
            AttributeValue::Function("VarChar".into(), vec![AttributeValue::Int(255)]),
        ));

        let attrs = field.extract_attributes();
        assert!(attrs.native_type.is_some());
        let nt = attrs.native_type.unwrap();
        assert_eq!(nt.name.as_str(), "VarChar");
        assert_eq!(nt.args.len(), 1);
    }

    #[test]
    fn test_extract_attributes_relation() {
        let mut field = make_field(
            "author",
            FieldType::Model("User".into()),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::new(
            Ident::new("relation", make_span()),
            vec![
                AttributeArg::named(
                    Ident::new("fields", make_span()),
                    AttributeValue::FieldRefList(vec!["author_id".into()]),
                    make_span(),
                ),
                AttributeArg::named(
                    Ident::new("references", make_span()),
                    AttributeValue::FieldRefList(vec!["id".into()]),
                    make_span(),
                ),
                AttributeArg::named(
                    Ident::new("onDelete", make_span()),
                    AttributeValue::Ident("Cascade".into()),
                    make_span(),
                ),
                AttributeArg::named(
                    Ident::new("onUpdate", make_span()),
                    AttributeValue::Ident("Restrict".into()),
                    make_span(),
                ),
            ],
            make_span(),
        ));

        let attrs = field.extract_attributes();
        assert!(attrs.relation.is_some());

        let rel = attrs.relation.unwrap();
        assert_eq!(rel.fields, vec!["author_id".to_string()]);
        assert_eq!(rel.references, vec!["id".to_string()]);
        assert_eq!(rel.on_delete, Some(ReferentialAction::Cascade));
        assert_eq!(rel.on_update, Some(ReferentialAction::Restrict));
    }

    #[test]
    fn test_extract_attributes_relation_with_name() {
        let mut field = make_field(
            "author",
            FieldType::Model("User".into()),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::new(
            Ident::new("relation", make_span()),
            vec![AttributeArg::positional(
                AttributeValue::String("PostAuthor".into()),
                make_span(),
            )],
            make_span(),
        ));

        let attrs = field.extract_attributes();
        assert!(attrs.relation.is_some());
        assert_eq!(attrs.relation.unwrap().name, Some("PostAuthor".to_string()));
    }

    // ==================== Field Display Tests ====================

    #[test]
    fn test_field_display_required() {
        let field = make_field(
            "id",
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
        );
        assert_eq!(format!("{}", field), "id Int");
    }

    #[test]
    fn test_field_display_optional() {
        let field = make_field(
            "bio",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Optional,
        );
        assert_eq!(format!("{}", field), "bio String?");
    }

    #[test]
    fn test_field_display_list() {
        let field = make_field(
            "tags",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::List,
        );
        assert_eq!(format!("{}", field), "tags String[]");
    }

    #[test]
    fn test_field_display_optional_list() {
        let field = make_field(
            "data",
            FieldType::Scalar(ScalarType::Json),
            TypeModifier::OptionalList,
        );
        assert_eq!(format!("{}", field), "data Json[]?");
    }

    #[test]
    fn test_field_display_with_simple_attribute() {
        let mut field = make_field(
            "id",
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
        );
        field.attributes.push(make_attribute("id"));
        assert!(format!("{}", field).contains("@id"));
    }

    #[test]
    fn test_field_display_with_attribute_args() {
        let mut field = make_field(
            "count",
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
        );
        field
            .attributes
            .push(make_attribute_with_arg("default", AttributeValue::Int(0)));
        assert!(format!("{}", field).contains("@default(...)"));
    }

    #[test]
    fn test_field_display_relation() {
        let field = make_field(
            "author",
            FieldType::Model("User".into()),
            TypeModifier::Required,
        );
        assert_eq!(format!("{}", field), "author User");
    }

    #[test]
    fn test_field_display_enum() {
        let field = make_field(
            "role",
            FieldType::Enum("Role".into()),
            TypeModifier::Required,
        );
        assert_eq!(format!("{}", field), "role Role");
    }

    // ==================== Field Equality Tests ====================

    #[test]
    fn test_field_equality() {
        let field1 = make_field(
            "id",
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
        );
        let field2 = make_field(
            "id",
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
        );
        assert_eq!(field1, field2);
    }

    #[test]
    fn test_field_inequality_name() {
        let field1 = make_field(
            "id",
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
        );
        let field2 = make_field(
            "user_id",
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
        );
        assert_ne!(field1, field2);
    }

    #[test]
    fn test_field_inequality_type() {
        let field1 = make_field(
            "id",
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
        );
        let field2 = make_field(
            "id",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        assert_ne!(field1, field2);
    }

    #[test]
    fn test_field_inequality_modifier() {
        let field1 = make_field(
            "name",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        let field2 = make_field(
            "name",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Optional,
        );
        assert_ne!(field1, field2);
    }

    // ==================== Validation Tests ====================

    #[test]
    fn test_field_with_validation() {
        let validation = FieldValidation::new();
        let field = make_field(
            "email",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        )
        .with_validation(validation);

        assert!(!field.has_validation());
    }

    #[test]
    fn test_field_add_validation_rule() {
        let mut field = make_field(
            "email",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );

        field.add_validation_rule(ValidationRule::new(ValidationType::Email, make_span()));
        assert!(field.has_validation());
        assert_eq!(field.validation_rules().len(), 1);
    }

    #[test]
    fn test_field_validation_required() {
        let mut field = make_field(
            "name",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Optional,
        );

        assert!(!field.is_validated_required());
        field.add_validation_rule(ValidationRule::new(ValidationType::Required, make_span()));
        assert!(field.is_validated_required());
    }

    #[test]
    fn test_extract_validation_email() {
        let mut field = make_field(
            "email",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.email", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
        assert_eq!(field.validation_rules().len(), 1);
    }

    #[test]
    fn test_extract_validation_url() {
        let mut field = make_field(
            "website",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Optional,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.url", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_uuid() {
        let mut field = make_field(
            "id",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.uuid", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_min_length() {
        let mut field = make_field(
            "name",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::new(
            Ident::new("validate.minLength", make_span()),
            vec![AttributeArg::positional(
                AttributeValue::Int(3),
                make_span(),
            )],
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_max_length() {
        let mut field = make_field(
            "bio",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Optional,
        );
        field.attributes.push(Attribute::new(
            Ident::new("validate.maxLength", make_span()),
            vec![AttributeArg::positional(
                AttributeValue::Int(500),
                make_span(),
            )],
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_min() {
        let mut field = make_field(
            "age",
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::new(
            Ident::new("validate.min", make_span()),
            vec![AttributeArg::positional(
                AttributeValue::Int(0),
                make_span(),
            )],
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_max() {
        let mut field = make_field(
            "percentage",
            FieldType::Scalar(ScalarType::Float),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::new(
            Ident::new("validate.max", make_span()),
            vec![AttributeArg::positional(
                AttributeValue::Float(100.0),
                make_span(),
            )],
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_range() {
        let mut field = make_field(
            "rating",
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::new(
            Ident::new("validate.range", make_span()),
            vec![
                AttributeArg::positional(AttributeValue::Int(1), make_span()),
                AttributeArg::positional(AttributeValue::Int(5), make_span()),
            ],
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_regex() {
        let mut field = make_field(
            "phone",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::new(
            Ident::new("validate.regex", make_span()),
            vec![AttributeArg::positional(
                AttributeValue::String("^\\+[0-9]+$".into()),
                make_span(),
            )],
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_positive() {
        let mut field = make_field(
            "amount",
            FieldType::Scalar(ScalarType::Float),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.positive", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_negative() {
        let mut field = make_field(
            "debt",
            FieldType::Scalar(ScalarType::Float),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.negative", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_nonNegative() {
        let mut field = make_field(
            "count",
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.nonNegative", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_alpha() {
        let mut field = make_field(
            "code",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.alpha", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_alphanumeric() {
        let mut field = make_field(
            "username",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.alphanumeric", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_lowercase() {
        let mut field = make_field(
            "slug",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.lowercase", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_uppercase() {
        let mut field = make_field(
            "country_code",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.uppercase", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_trim() {
        let mut field = make_field(
            "input",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.trim", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_ip() {
        let mut field = make_field(
            "ip_address",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.ip", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_ipv4() {
        let mut field = make_field(
            "ipv4",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.ipv4", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_ipv6() {
        let mut field = make_field(
            "ipv6",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.ipv6", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_slug() {
        let mut field = make_field(
            "url_slug",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.slug", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_hex() {
        let mut field = make_field(
            "color",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.hex", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_base64() {
        let mut field = make_field(
            "encoded",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.base64", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_json() {
        let mut field = make_field(
            "json_str",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.json", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_integer() {
        let mut field = make_field(
            "whole",
            FieldType::Scalar(ScalarType::Float),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.integer", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_finite() {
        let mut field = make_field(
            "value",
            FieldType::Scalar(ScalarType::Float),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.finite", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_unique_array() {
        let mut field = make_field(
            "tags",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::List,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.unique", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_nonEmpty() {
        let mut field = make_field(
            "required_tags",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::List,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.nonEmpty", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_past() {
        let mut field = make_field(
            "birth_date",
            FieldType::Scalar(ScalarType::DateTime),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.past", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_future() {
        let mut field = make_field(
            "expiry_date",
            FieldType::Scalar(ScalarType::DateTime),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.future", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_min_items() {
        let mut field = make_field(
            "items",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::List,
        );
        field.attributes.push(Attribute::new(
            Ident::new("validate.minItems", make_span()),
            vec![AttributeArg::positional(
                AttributeValue::Int(1),
                make_span(),
            )],
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_max_items() {
        let mut field = make_field(
            "items",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::List,
        );
        field.attributes.push(Attribute::new(
            Ident::new("validate.maxItems", make_span()),
            vec![AttributeArg::positional(
                AttributeValue::Int(10),
                make_span(),
            )],
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_multiple_of() {
        let mut field = make_field(
            "amount",
            FieldType::Scalar(ScalarType::Float),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::new(
            Ident::new("validate.multipleOf", make_span()),
            vec![AttributeArg::positional(
                AttributeValue::Float(0.01),
                make_span(),
            )],
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_starts_with() {
        let mut field = make_field(
            "prefix_field",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::new(
            Ident::new("validate.startsWith", make_span()),
            vec![AttributeArg::positional(
                AttributeValue::String("PREFIX_".into()),
                make_span(),
            )],
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_ends_with() {
        let mut field = make_field(
            "suffix_field",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::new(
            Ident::new("validate.endsWith", make_span()),
            vec![AttributeArg::positional(
                AttributeValue::String(".json".into()),
                make_span(),
            )],
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_contains() {
        let mut field = make_field(
            "text",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::new(
            Ident::new("validate.contains", make_span()),
            vec![AttributeArg::positional(
                AttributeValue::String("keyword".into()),
                make_span(),
            )],
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_after() {
        let mut field = make_field(
            "start_date",
            FieldType::Scalar(ScalarType::DateTime),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::new(
            Ident::new("validate.after", make_span()),
            vec![AttributeArg::positional(
                AttributeValue::String("2024-01-01".into()),
                make_span(),
            )],
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_before() {
        let mut field = make_field(
            "end_date",
            FieldType::Scalar(ScalarType::DateTime),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::new(
            Ident::new("validate.before", make_span()),
            vec![AttributeArg::positional(
                AttributeValue::String("2025-12-31".into()),
                make_span(),
            )],
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_custom() {
        let mut field = make_field(
            "password",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::new(
            Ident::new("validate.custom", make_span()),
            vec![AttributeArg::positional(
                AttributeValue::String("strongPassword".into()),
                make_span(),
            )],
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_length() {
        let mut field = make_field(
            "bio",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::new(
            Ident::new("validate.length", make_span()),
            vec![
                AttributeArg::positional(AttributeValue::Int(10), make_span()),
                AttributeArg::positional(AttributeValue::Int(500), make_span()),
            ],
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_cuid() {
        let mut field = make_field(
            "cuid_field",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.cuid", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_cuid2() {
        let mut field = make_field(
            "cuid2_field",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.cuid2", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_nanoid() {
        let mut field = make_field(
            "nanoid_field",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.nanoid", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_ulid() {
        let mut field = make_field(
            "ulid_field",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.ulid", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_noWhitespace() {
        let mut field = make_field(
            "username",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.noWhitespace", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_creditCard() {
        let mut field = make_field(
            "card_number",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.creditCard", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_phone() {
        let mut field = make_field(
            "phone_number",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.phone", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_nonPositive() {
        let mut field = make_field(
            "debt",
            FieldType::Scalar(ScalarType::Float),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.nonPositive", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_pastOrPresent() {
        let mut field = make_field(
            "login_date",
            FieldType::Scalar(ScalarType::DateTime),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.pastOrPresent", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_futureOrPresent() {
        let mut field = make_field(
            "schedule_date",
            FieldType::Scalar(ScalarType::DateTime),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.futureOrPresent", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_required() {
        let mut field = make_field(
            "important",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Optional,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.required", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
        assert!(field.is_validated_required());
    }

    #[test]
    fn test_extract_validation_notEmpty() {
        let mut field = make_field(
            "content",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.notEmpty", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    #[test]
    fn test_extract_validation_unknown_validator() {
        let mut field = make_field(
            "field",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::simple(
            Ident::new("validate.unknownValidator", make_span()),
            make_span(),
        ));

        field.extract_validation_from_attributes();
        // Unknown validators should be ignored
        assert!(!field.has_validation());
    }

    #[test]
    fn test_extract_validation_items() {
        let mut field = make_field(
            "tags",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::List,
        );
        field.attributes.push(Attribute::new(
            Ident::new("validate.items", make_span()),
            vec![
                AttributeArg::positional(AttributeValue::Int(1), make_span()),
                AttributeArg::positional(AttributeValue::Int(10), make_span()),
            ],
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
    }

    // ==================== Comprehensive validate() attribute tests ====================

    #[test]
    fn test_extract_validate_attribute_with_ident() {
        let mut field = make_field(
            "email",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::new(
            Ident::new("validate", make_span()),
            vec![AttributeArg::positional(
                AttributeValue::Ident("email".into()),
                make_span(),
            )],
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
        assert_eq!(field.validation_rules().len(), 1);
    }

    #[test]
    fn test_extract_validate_attribute_with_function() {
        let mut field = make_field(
            "name",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::new(
            Ident::new("validate", make_span()),
            vec![AttributeArg::positional(
                AttributeValue::Function("minLength".into(), vec![AttributeValue::Int(3)]),
                make_span(),
            )],
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
        assert_eq!(field.validation_rules().len(), 1);
    }

    #[test]
    fn test_extract_validate_multiple_validators() {
        let mut field = make_field(
            "email",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );
        field.attributes.push(Attribute::new(
            Ident::new("validate", make_span()),
            vec![
                AttributeArg::positional(AttributeValue::Ident("email".into()), make_span()),
                AttributeArg::positional(
                    AttributeValue::Function("maxLength".into(), vec![AttributeValue::Int(255)]),
                    make_span(),
                ),
            ],
            make_span(),
        ));

        field.extract_validation_from_attributes();
        assert!(field.has_validation());
        assert_eq!(field.validation_rules().len(), 2);
    }
}
