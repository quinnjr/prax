//! Input validation system for Prax schemas.
//!
//! This module provides a robust validation framework that integrates with
//! documentation comments and field attributes to define validation rules.
//!
//! # Validation Syntax
//!
//! Validation rules can be specified using the `@validate` attribute or
//! through structured documentation comments with `@validate:` directives.
//!
//! ## Attribute-based Validation
//!
//! ```prax
//! model User {
//!     id       Int    @id @auto
//!     email    String @validate.email @validate.maxLength(255)
//!     username String @validate.minLength(3) @validate.maxLength(30) @validate.regex("^[a-z0-9_]+$")
//!     age      Int?   @validate.range(0, 150)
//!     website  String? @validate.url
//! }
//! ```
//!
//! ## Documentation-based Validation
//!
//! ```prax
//! model User {
//!     /// The user's email address
//!     /// @validate: email, maxLength(255)
//!     email String
//!
//!     /// Username must be lowercase alphanumeric with underscores
//!     /// @validate: minLength(3), maxLength(30)
//!     /// @validate: regex("^[a-z0-9_]+$")
//!     username String
//! }
//! ```
//!
//! # Built-in Validators
//!
//! ## String Validators
//! - `email` - Valid email format (RFC 5322)
//! - `url` - Valid URL format
//! - `uuid` - Valid UUID format (v1-v5)
//! - `cuid` - Valid CUID format
//! - `regex(pattern)` - Matches regex pattern
//! - `minLength(n)` - Minimum string length
//! - `maxLength(n)` - Maximum string length
//! - `length(min, max)` - String length range
//! - `startsWith(prefix)` - String starts with prefix
//! - `endsWith(suffix)` - String ends with suffix
//! - `contains(substring)` - String contains substring
//! - `alpha` - Only alphabetic characters
//! - `alphanumeric` - Only alphanumeric characters
//! - `lowercase` - Only lowercase characters
//! - `uppercase` - Only uppercase characters
//! - `trim` - Trimmed (no leading/trailing whitespace)
//! - `noWhitespace` - No whitespace characters
//! - `ip` - Valid IP address (v4 or v6)
//! - `ipv4` - Valid IPv4 address
//! - `ipv6` - Valid IPv6 address
//! - `creditCard` - Valid credit card number (Luhn algorithm)
//! - `phone` - Valid phone number format
//! - `slug` - URL-safe slug format
//! - `hex` - Valid hexadecimal string
//! - `base64` - Valid base64 string
//! - `json` - Valid JSON string
//!
//! ## Numeric Validators
//! - `min(n)` - Minimum value
//! - `max(n)` - Maximum value
//! - `range(min, max)` - Value within range (inclusive)
//! - `positive` - Value > 0
//! - `negative` - Value < 0
//! - `nonNegative` - Value >= 0
//! - `nonPositive` - Value <= 0
//! - `integer` - Must be integer (no decimal)
//! - `multipleOf(n)` - Value is multiple of n
//! - `finite` - Must be finite (not Infinity/NaN)
//!
//! ## Array Validators
//! - `minItems(n)` - Minimum array length
//! - `maxItems(n)` - Maximum array length
//! - `items(min, max)` - Array length range
//! - `unique` - All items must be unique
//! - `nonEmpty` - Array must have at least one item
//!
//! ## Date/Time Validators
//! - `past` - Date must be in the past
//! - `future` - Date must be in the future
//! - `pastOrPresent` - Date must be past or present
//! - `futureOrPresent` - Date must be future or present
//! - `after(date)` - Date must be after specified date
//! - `before(date)` - Date must be before specified date
//!
//! ## General Validators
//! - `required` - Field must not be null (for optional fields)
//! - `notEmpty` - Field must not be empty (string, array, etc.)
//! - `oneOf(values...)` - Value must be one of the specified values
//! - `custom(name)` - Use a custom validator function
//!
//! # Custom Validators
//!
//! Custom validators can be registered and referenced by name:
//!
//! ```text
//! // Register custom validator
//! validation::register_custom("strongPassword", |value: &str| {
//!     // Must have uppercase, lowercase, number, and special char
//!     let has_upper = value.chars().any(|c| c.is_uppercase());
//!     let has_lower = value.chars().any(|c| c.is_lowercase());
//!     let has_digit = value.chars().any(|c| c.is_numeric());
//!     let has_special = value.chars().any(|c| !c.is_alphanumeric());
//!     has_upper && has_lower && has_digit && has_special
//! });
//!
//! // Use in schema
//! model User {
//!     password String @validate.custom("strongPassword")
//! }
//! ```

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use super::Span;

/// A validation rule for a field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationRule {
    /// The type of validation.
    pub rule_type: ValidationType,
    /// Optional custom error message.
    pub message: Option<String>,
    /// Source location.
    pub span: Span,
}

impl ValidationRule {
    /// Create a new validation rule.
    pub fn new(rule_type: ValidationType, span: Span) -> Self {
        Self {
            rule_type,
            message: None,
            span,
        }
    }

    /// Create a validation rule with a custom message.
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    /// Get the error message for this rule.
    pub fn error_message(&self, field_name: &str) -> String {
        if let Some(msg) = &self.message {
            msg.clone()
        } else {
            self.rule_type.default_message(field_name)
        }
    }

    /// Check if this rule applies to strings.
    pub fn is_string_rule(&self) -> bool {
        self.rule_type.is_string_rule()
    }

    /// Check if this rule applies to numbers.
    pub fn is_numeric_rule(&self) -> bool {
        self.rule_type.is_numeric_rule()
    }

    /// Check if this rule applies to arrays.
    pub fn is_array_rule(&self) -> bool {
        self.rule_type.is_array_rule()
    }

    /// Check if this rule applies to dates.
    pub fn is_date_rule(&self) -> bool {
        self.rule_type.is_date_rule()
    }
}

/// Types of validation rules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ValidationType {
    // ==================== String Validators ====================
    /// Valid email format.
    Email,
    /// Valid URL format.
    Url,
    /// Valid UUID format (v1-v5).
    Uuid,
    /// Valid CUID format.
    Cuid,
    /// Valid CUID2 format.
    Cuid2,
    /// Valid NanoId format.
    NanoId,
    /// Valid ULID format.
    Ulid,
    /// Matches regex pattern.
    Regex(String),
    /// Minimum string length.
    MinLength(usize),
    /// Maximum string length.
    MaxLength(usize),
    /// String length range.
    Length { min: usize, max: usize },
    /// String starts with prefix.
    StartsWith(String),
    /// String ends with suffix.
    EndsWith(String),
    /// String contains substring.
    Contains(String),
    /// Only alphabetic characters.
    Alpha,
    /// Only alphanumeric characters.
    Alphanumeric,
    /// Only lowercase characters.
    Lowercase,
    /// Only uppercase characters.
    Uppercase,
    /// Trimmed (no leading/trailing whitespace).
    Trim,
    /// No whitespace characters.
    NoWhitespace,
    /// Valid IP address (v4 or v6).
    Ip,
    /// Valid IPv4 address.
    Ipv4,
    /// Valid IPv6 address.
    Ipv6,
    /// Valid credit card number.
    CreditCard,
    /// Valid phone number format.
    Phone,
    /// URL-safe slug format.
    Slug,
    /// Valid hexadecimal string.
    Hex,
    /// Valid base64 string.
    Base64,
    /// Valid JSON string.
    Json,

    // ==================== Numeric Validators ====================
    /// Minimum value.
    Min(f64),
    /// Maximum value.
    Max(f64),
    /// Value within range (inclusive).
    Range { min: f64, max: f64 },
    /// Value > 0.
    Positive,
    /// Value < 0.
    Negative,
    /// Value >= 0.
    NonNegative,
    /// Value <= 0.
    NonPositive,
    /// Must be integer (no decimal).
    Integer,
    /// Value is multiple of n.
    MultipleOf(f64),
    /// Must be finite (not Infinity/NaN).
    Finite,

    // ==================== Array Validators ====================
    /// Minimum array length.
    MinItems(usize),
    /// Maximum array length.
    MaxItems(usize),
    /// Array length range.
    Items { min: usize, max: usize },
    /// All items must be unique.
    Unique,
    /// Array must have at least one item.
    NonEmpty,

    // ==================== Date/Time Validators ====================
    /// Date must be in the past.
    Past,
    /// Date must be in the future.
    Future,
    /// Date must be past or present.
    PastOrPresent,
    /// Date must be future or present.
    FutureOrPresent,
    /// Date must be after specified date.
    After(String),
    /// Date must be before specified date.
    Before(String),

    // ==================== General Validators ====================
    /// Field must not be null (for optional fields).
    Required,
    /// Field must not be empty.
    NotEmpty,
    /// Value must be one of the specified values.
    OneOf(Vec<ValidationValue>),
    /// Use a custom validator function.
    Custom(String),
}

impl ValidationType {
    /// Get the default error message for this validation type.
    pub fn default_message(&self, field_name: &str) -> String {
        match self {
            // String validators
            Self::Email => format!("{} must be a valid email address", field_name),
            Self::Url => format!("{} must be a valid URL", field_name),
            Self::Uuid => format!("{} must be a valid UUID", field_name),
            Self::Cuid => format!("{} must be a valid CUID", field_name),
            Self::Cuid2 => format!("{} must be a valid CUID2", field_name),
            Self::NanoId => format!("{} must be a valid NanoId", field_name),
            Self::Ulid => format!("{} must be a valid ULID", field_name),
            Self::Regex(pattern) => format!("{} must match pattern: {}", field_name, pattern),
            Self::MinLength(n) => format!("{} must be at least {} characters", field_name, n),
            Self::MaxLength(n) => format!("{} must be at most {} characters", field_name, n),
            Self::Length { min, max } => {
                format!(
                    "{} must be between {} and {} characters",
                    field_name, min, max
                )
            }
            Self::StartsWith(s) => format!("{} must start with '{}'", field_name, s),
            Self::EndsWith(s) => format!("{} must end with '{}'", field_name, s),
            Self::Contains(s) => format!("{} must contain '{}'", field_name, s),
            Self::Alpha => format!("{} must contain only letters", field_name),
            Self::Alphanumeric => format!("{} must contain only letters and numbers", field_name),
            Self::Lowercase => format!("{} must be lowercase", field_name),
            Self::Uppercase => format!("{} must be uppercase", field_name),
            Self::Trim => format!(
                "{} must not have leading or trailing whitespace",
                field_name
            ),
            Self::NoWhitespace => format!("{} must not contain whitespace", field_name),
            Self::Ip => format!("{} must be a valid IP address", field_name),
            Self::Ipv4 => format!("{} must be a valid IPv4 address", field_name),
            Self::Ipv6 => format!("{} must be a valid IPv6 address", field_name),
            Self::CreditCard => format!("{} must be a valid credit card number", field_name),
            Self::Phone => format!("{} must be a valid phone number", field_name),
            Self::Slug => format!("{} must be a valid URL slug", field_name),
            Self::Hex => format!("{} must be a valid hexadecimal string", field_name),
            Self::Base64 => format!("{} must be a valid base64 string", field_name),
            Self::Json => format!("{} must be valid JSON", field_name),

            // Numeric validators
            Self::Min(n) => format!("{} must be at least {}", field_name, n),
            Self::Max(n) => format!("{} must be at most {}", field_name, n),
            Self::Range { min, max } => {
                format!("{} must be between {} and {}", field_name, min, max)
            }
            Self::Positive => format!("{} must be positive", field_name),
            Self::Negative => format!("{} must be negative", field_name),
            Self::NonNegative => format!("{} must not be negative", field_name),
            Self::NonPositive => format!("{} must not be positive", field_name),
            Self::Integer => format!("{} must be an integer", field_name),
            Self::MultipleOf(n) => format!("{} must be a multiple of {}", field_name, n),
            Self::Finite => format!("{} must be a finite number", field_name),

            // Array validators
            Self::MinItems(n) => format!("{} must have at least {} items", field_name, n),
            Self::MaxItems(n) => format!("{} must have at most {} items", field_name, n),
            Self::Items { min, max } => {
                format!("{} must have between {} and {} items", field_name, min, max)
            }
            Self::Unique => format!("{} must have unique items", field_name),
            Self::NonEmpty => format!("{} must not be empty", field_name),

            // Date validators
            Self::Past => format!("{} must be in the past", field_name),
            Self::Future => format!("{} must be in the future", field_name),
            Self::PastOrPresent => format!("{} must not be in the future", field_name),
            Self::FutureOrPresent => format!("{} must not be in the past", field_name),
            Self::After(date) => format!("{} must be after {}", field_name, date),
            Self::Before(date) => format!("{} must be before {}", field_name, date),

            // General validators
            Self::Required => format!("{} is required", field_name),
            Self::NotEmpty => format!("{} must not be empty", field_name),
            Self::OneOf(values) => {
                let options: Vec<String> = values.iter().map(|v| v.to_string()).collect();
                format!("{} must be one of: {}", field_name, options.join(", "))
            }
            Self::Custom(name) => format!("{} failed custom validation: {}", field_name, name),
        }
    }

    /// Check if this rule applies to strings.
    pub fn is_string_rule(&self) -> bool {
        matches!(
            self,
            Self::Email
                | Self::Url
                | Self::Uuid
                | Self::Cuid
                | Self::Cuid2
                | Self::NanoId
                | Self::Ulid
                | Self::Regex(_)
                | Self::MinLength(_)
                | Self::MaxLength(_)
                | Self::Length { .. }
                | Self::StartsWith(_)
                | Self::EndsWith(_)
                | Self::Contains(_)
                | Self::Alpha
                | Self::Alphanumeric
                | Self::Lowercase
                | Self::Uppercase
                | Self::Trim
                | Self::NoWhitespace
                | Self::Ip
                | Self::Ipv4
                | Self::Ipv6
                | Self::CreditCard
                | Self::Phone
                | Self::Slug
                | Self::Hex
                | Self::Base64
                | Self::Json
        )
    }

    /// Check if this rule validates an identifier format (UUID, CUID, etc.).
    pub fn is_id_format_rule(&self) -> bool {
        matches!(
            self,
            Self::Uuid | Self::Cuid | Self::Cuid2 | Self::NanoId | Self::Ulid
        )
    }

    /// Check if this rule applies to numbers.
    pub fn is_numeric_rule(&self) -> bool {
        matches!(
            self,
            Self::Min(_)
                | Self::Max(_)
                | Self::Range { .. }
                | Self::Positive
                | Self::Negative
                | Self::NonNegative
                | Self::NonPositive
                | Self::Integer
                | Self::MultipleOf(_)
                | Self::Finite
        )
    }

    /// Check if this rule applies to arrays.
    pub fn is_array_rule(&self) -> bool {
        matches!(
            self,
            Self::MinItems(_)
                | Self::MaxItems(_)
                | Self::Items { .. }
                | Self::Unique
                | Self::NonEmpty
        )
    }

    /// Check if this rule applies to dates.
    pub fn is_date_rule(&self) -> bool {
        matches!(
            self,
            Self::Past
                | Self::Future
                | Self::PastOrPresent
                | Self::FutureOrPresent
                | Self::After(_)
                | Self::Before(_)
        )
    }

    /// Get the validator name (for code generation).
    pub fn validator_name(&self) -> &'static str {
        match self {
            Self::Email => "email",
            Self::Url => "url",
            Self::Uuid => "uuid",
            Self::Cuid => "cuid",
            Self::Cuid2 => "cuid2",
            Self::NanoId => "nanoid",
            Self::Ulid => "ulid",
            Self::Regex(_) => "regex",
            Self::MinLength(_) => "min_length",
            Self::MaxLength(_) => "max_length",
            Self::Length { .. } => "length",
            Self::StartsWith(_) => "starts_with",
            Self::EndsWith(_) => "ends_with",
            Self::Contains(_) => "contains",
            Self::Alpha => "alpha",
            Self::Alphanumeric => "alphanumeric",
            Self::Lowercase => "lowercase",
            Self::Uppercase => "uppercase",
            Self::Trim => "trim",
            Self::NoWhitespace => "no_whitespace",
            Self::Ip => "ip",
            Self::Ipv4 => "ipv4",
            Self::Ipv6 => "ipv6",
            Self::CreditCard => "credit_card",
            Self::Phone => "phone",
            Self::Slug => "slug",
            Self::Hex => "hex",
            Self::Base64 => "base64",
            Self::Json => "json",
            Self::Min(_) => "min",
            Self::Max(_) => "max",
            Self::Range { .. } => "range",
            Self::Positive => "positive",
            Self::Negative => "negative",
            Self::NonNegative => "non_negative",
            Self::NonPositive => "non_positive",
            Self::Integer => "integer",
            Self::MultipleOf(_) => "multiple_of",
            Self::Finite => "finite",
            Self::MinItems(_) => "min_items",
            Self::MaxItems(_) => "max_items",
            Self::Items { .. } => "items",
            Self::Unique => "unique",
            Self::NonEmpty => "non_empty",
            Self::Past => "past",
            Self::Future => "future",
            Self::PastOrPresent => "past_or_present",
            Self::FutureOrPresent => "future_or_present",
            Self::After(_) => "after",
            Self::Before(_) => "before",
            Self::Required => "required",
            Self::NotEmpty => "not_empty",
            Self::OneOf(_) => "one_of",
            Self::Custom(_) => "custom",
        }
    }
}

/// A value that can be used in validation rules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ValidationValue {
    /// String value.
    String(String),
    /// Integer value.
    Int(i64),
    /// Float value.
    Float(f64),
    /// Boolean value.
    Bool(bool),
}

impl std::fmt::Display for ValidationValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::String(s) => write!(f, "\"{}\"", s),
            Self::Int(i) => write!(f, "{}", i),
            Self::Float(n) => write!(f, "{}", n),
            Self::Bool(b) => write!(f, "{}", b),
        }
    }
}

/// A collection of validation rules for a field.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FieldValidation {
    /// The validation rules.
    pub rules: Vec<ValidationRule>,
}

impl FieldValidation {
    /// Create empty field validation.
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    /// Add a validation rule.
    pub fn add_rule(&mut self, rule: ValidationRule) {
        self.rules.push(rule);
    }

    /// Check if there are any validation rules.
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Get the number of validation rules.
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// Check if this field has any string validation rules.
    pub fn has_string_rules(&self) -> bool {
        self.rules.iter().any(|r| r.is_string_rule())
    }

    /// Check if this field has any numeric validation rules.
    pub fn has_numeric_rules(&self) -> bool {
        self.rules.iter().any(|r| r.is_numeric_rule())
    }

    /// Check if this field has any array validation rules.
    pub fn has_array_rules(&self) -> bool {
        self.rules.iter().any(|r| r.is_array_rule())
    }

    /// Check if this field is required.
    pub fn is_required(&self) -> bool {
        self.rules
            .iter()
            .any(|r| matches!(r.rule_type, ValidationType::Required))
    }
}

/// Enhanced documentation with embedded validation directives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnhancedDocumentation {
    /// The main documentation text (cleaned of validation directives).
    pub text: String,
    /// Parsed validation rules from documentation.
    pub validation: FieldValidation,
    /// Additional metadata tags.
    pub tags: Vec<DocTag>,
    /// Source location.
    pub span: Span,
}

impl EnhancedDocumentation {
    /// Create new enhanced documentation.
    pub fn new(text: impl Into<String>, span: Span) -> Self {
        Self {
            text: text.into(),
            validation: FieldValidation::new(),
            tags: Vec::new(),
            span,
        }
    }

    /// Parse documentation text and extract validation rules.
    pub fn parse(raw_text: &str, span: Span) -> Self {
        let mut text_lines = Vec::new();
        let mut validation = FieldValidation::new();
        let mut tags = Vec::new();

        for line in raw_text.lines() {
            let trimmed = line.trim();

            // Check for @validate: directive
            if let Some(validate_content) = trimmed.strip_prefix("@validate:") {
                // Parse validation rules
                for rule_str in validate_content.split(',') {
                    if let Some(rule) = parse_validation_rule(rule_str.trim(), span) {
                        validation.add_rule(rule);
                    }
                }
            }
            // Check for other tags
            else if let Some(tag) = parse_doc_tag(trimmed, span) {
                tags.push(tag);
            }
            // Regular documentation line
            else {
                text_lines.push(line);
            }
        }

        Self {
            text: text_lines.join("\n").trim().to_string(),
            validation,
            tags,
            span,
        }
    }

    /// Check if this documentation has any validation rules.
    pub fn has_validation(&self) -> bool {
        !self.validation.is_empty()
    }

    /// Get all validation rules.
    pub fn validation_rules(&self) -> &[ValidationRule] {
        &self.validation.rules
    }

    /// Get a specific tag by name.
    pub fn get_tag(&self, name: &str) -> Option<&DocTag> {
        self.tags.iter().find(|t| t.name == name)
    }

    /// Get all tags with a specific name.
    pub fn get_tags(&self, name: &str) -> Vec<&DocTag> {
        self.tags.iter().filter(|t| t.name == name).collect()
    }

    /// Check if a specific tag exists.
    pub fn has_tag(&self, name: &str) -> bool {
        self.tags.iter().any(|t| t.name == name)
    }

    /// Extract field metadata from documentation tags.
    pub fn extract_metadata(&self) -> FieldMetadata {
        FieldMetadata::from_tags(&self.tags)
    }

    /// Check if field is marked as hidden.
    pub fn is_hidden(&self) -> bool {
        self.has_tag("hidden") || self.has_tag("internal")
    }

    /// Check if field is marked as deprecated.
    pub fn is_deprecated(&self) -> bool {
        self.has_tag("deprecated")
    }

    /// Get deprecation info if deprecated.
    pub fn deprecation_info(&self) -> Option<DeprecationInfo> {
        self.get_tag("deprecated").map(|tag| {
            let mut info = DeprecationInfo::new(tag.value.clone().unwrap_or_default());
            if let Some(since_tag) = self.get_tag("since") {
                info.since = since_tag.value.clone();
            }
            info
        })
    }

    /// Check if field is marked as sensitive.
    pub fn is_sensitive(&self) -> bool {
        self.has_tag("sensitive") || self.has_tag("writeonly")
    }

    /// Check if field is marked as readonly.
    pub fn is_readonly(&self) -> bool {
        self.has_tag("readonly") || self.has_tag("readOnly")
    }

    /// Check if field is marked as writeonly.
    pub fn is_writeonly(&self) -> bool {
        self.has_tag("writeonly") || self.has_tag("writeOnly")
    }

    /// Get examples from documentation.
    pub fn examples(&self) -> Vec<&str> {
        self.tags
            .iter()
            .filter(|t| t.name == "example")
            .filter_map(|t| t.value.as_deref())
            .collect()
    }

    /// Get the display label.
    pub fn label(&self) -> Option<&str> {
        self.get_tag("label").and_then(|t| t.value.as_deref())
    }

    /// Get the placeholder text.
    pub fn placeholder(&self) -> Option<&str> {
        self.get_tag("placeholder").and_then(|t| t.value.as_deref())
    }

    /// Get the version since this field was introduced.
    pub fn since(&self) -> Option<&str> {
        self.get_tag("since").and_then(|t| t.value.as_deref())
    }

    /// Get the field group for UI organization.
    pub fn group(&self) -> Option<&str> {
        self.get_tag("group").and_then(|t| t.value.as_deref())
    }
}

/// A documentation tag (like @example, @deprecated, @see).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocTag {
    /// Tag name (without @).
    pub name: SmolStr,
    /// Tag value/content.
    pub value: Option<String>,
    /// Source location.
    pub span: Span,
}

impl DocTag {
    /// Create a new documentation tag.
    pub fn new(name: impl Into<SmolStr>, value: Option<String>, span: Span) -> Self {
        Self {
            name: name.into(),
            value,
            span,
        }
    }
}

// ============================================================================
// Field Metadata - Documentation & API Behavior
// ============================================================================

/// Comprehensive metadata for a field controlling visibility, deprecation, and API behavior.
///
/// This can be specified via documentation comments or attributes:
///
/// ```prax
/// model User {
///     /// @hidden
///     internal_id   String
///
///     /// @deprecated Use `newEmail` instead
///     /// @since 1.0.0
///     oldEmail      String?
///
///     /// @sensitive
///     /// @writeonly
///     password      String
///
///     /// @readonly
///     /// @example "2024-01-15T10:30:00Z"
///     createdAt     DateTime
///
///     /// @label "Email Address"
///     /// @placeholder "user@example.com"
///     email         String @validate.email
/// }
/// ```
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FieldMetadata {
    // ==================== Visibility Controls ====================
    /// Field is hidden from public API/documentation.
    pub hidden: bool,
    /// Field is internal (may be exposed in admin APIs).
    pub internal: bool,
    /// Field contains sensitive data (should be masked in logs).
    pub sensitive: bool,

    // ==================== API Behavior ====================
    /// Field is read-only (cannot be set via API).
    pub readonly: bool,
    /// Field is write-only (not returned in responses, e.g., passwords).
    pub writeonly: bool,
    /// Field is only valid for input (create/update).
    pub input_only: bool,
    /// Field is only valid for output (responses).
    pub output_only: bool,
    /// Omit this field from API responses entirely.
    pub omit_from_output: bool,
    /// Omit this field from input schemas.
    pub omit_from_input: bool,

    // ==================== Deprecation ====================
    /// Field is deprecated.
    pub deprecated: Option<DeprecationInfo>,

    // ==================== Documentation ====================
    /// Human-readable label for the field.
    pub label: Option<String>,
    /// Description override (if different from doc comment).
    pub description: Option<String>,
    /// Placeholder text for input fields.
    pub placeholder: Option<String>,
    /// Example values.
    pub examples: Vec<String>,
    /// Related fields or documentation references.
    pub see_also: Vec<String>,
    /// Version when the field was introduced.
    pub since: Option<String>,

    // ==================== Serialization ====================
    /// Alias name for serialization.
    pub alias: Option<String>,
    /// Override the serialized field name.
    pub serialized_name: Option<String>,
    /// Field order hint for serialization.
    pub order: Option<i32>,
    /// Default value for serialization (if missing).
    pub default_value: Option<String>,

    // ==================== UI Hints ====================
    /// Field group/section for UI organization.
    pub group: Option<String>,
    /// Display format hint (e.g., "date", "currency", "percent").
    pub format: Option<String>,
    /// Input type hint (e.g., "textarea", "select", "radio").
    pub input_type: Option<String>,
    /// Maximum display width.
    pub max_width: Option<u32>,
    /// Field should be displayed as multiline.
    pub multiline: bool,
    /// Rich text/HTML allowed.
    pub rich_text: bool,
}

impl FieldMetadata {
    /// Create empty field metadata.
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if field should be excluded from public documentation.
    pub fn is_hidden(&self) -> bool {
        self.hidden || self.internal
    }

    /// Check if field should be excluded from API responses.
    pub fn should_omit_from_output(&self) -> bool {
        self.omit_from_output || self.writeonly || self.hidden
    }

    /// Check if field should be excluded from API input.
    pub fn should_omit_from_input(&self) -> bool {
        self.omit_from_input || self.readonly || self.output_only
    }

    /// Check if field is deprecated.
    pub fn is_deprecated(&self) -> bool {
        self.deprecated.is_some()
    }

    /// Get deprecation message if deprecated.
    pub fn deprecation_message(&self) -> Option<&str> {
        self.deprecated.as_ref().map(|d| d.message.as_str())
    }

    /// Check if field contains sensitive data.
    pub fn is_sensitive(&self) -> bool {
        self.sensitive || self.writeonly
    }

    /// Get the display label (or None if not set).
    pub fn display_label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    /// Get all examples.
    pub fn get_examples(&self) -> &[String] {
        &self.examples
    }

    /// Parse metadata from a list of doc tags.
    pub fn from_tags(tags: &[DocTag]) -> Self {
        let mut meta = Self::new();

        for tag in tags {
            match tag.name.as_str() {
                // Visibility
                "hidden" => meta.hidden = true,
                "internal" => meta.internal = true,
                "sensitive" => meta.sensitive = true,

                // API behavior
                "readonly" | "readOnly" => meta.readonly = true,
                "writeonly" | "writeOnly" => meta.writeonly = true,
                "inputOnly" | "input_only" => meta.input_only = true,
                "outputOnly" | "output_only" => meta.output_only = true,
                "omitFromOutput" | "omit_from_output" => meta.omit_from_output = true,
                "omitFromInput" | "omit_from_input" => meta.omit_from_input = true,

                // Deprecation
                "deprecated" => {
                    meta.deprecated = Some(DeprecationInfo {
                        message: tag.value.clone().unwrap_or_default(),
                        since: None,
                        replacement: None,
                    });
                }

                // Documentation
                "label" => meta.label = tag.value.clone(),
                "description" | "desc" => meta.description = tag.value.clone(),
                "placeholder" => meta.placeholder = tag.value.clone(),
                "example" => {
                    if let Some(val) = &tag.value {
                        meta.examples.push(val.clone());
                    }
                }
                "see" | "seeAlso" | "see_also" => {
                    if let Some(val) = &tag.value {
                        meta.see_also.push(val.clone());
                    }
                }
                "since" => meta.since = tag.value.clone(),

                // Serialization
                "alias" => meta.alias = tag.value.clone(),
                "serializedName" | "serialized_name" | "json" => {
                    meta.serialized_name = tag.value.clone()
                }
                "order" => {
                    if let Some(val) = &tag.value {
                        meta.order = val.parse().ok();
                    }
                }
                "default" => meta.default_value = tag.value.clone(),

                // UI hints
                "group" => meta.group = tag.value.clone(),
                "format" => meta.format = tag.value.clone(),
                "inputType" | "input_type" => meta.input_type = tag.value.clone(),
                "maxWidth" | "max_width" => {
                    if let Some(val) = &tag.value {
                        meta.max_width = val.parse().ok();
                    }
                }
                "multiline" => meta.multiline = true,
                "richText" | "rich_text" | "html" => meta.rich_text = true,

                _ => {}
            }
        }

        meta
    }
}

/// Information about a deprecated field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeprecationInfo {
    /// Deprecation message/reason.
    pub message: String,
    /// Version when deprecated.
    pub since: Option<String>,
    /// Replacement field or method.
    pub replacement: Option<String>,
}

impl DeprecationInfo {
    /// Create new deprecation info.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            since: None,
            replacement: None,
        }
    }

    /// Set the version when deprecated.
    pub fn since(mut self, version: impl Into<String>) -> Self {
        self.since = Some(version.into());
        self
    }

    /// Set the replacement field.
    pub fn replacement(mut self, field: impl Into<String>) -> Self {
        self.replacement = Some(field.into());
        self
    }

    /// Format for display.
    pub fn format_message(&self) -> String {
        let mut msg = self.message.clone();
        if let Some(since) = &self.since {
            msg.push_str(&format!(" (since {})", since));
        }
        if let Some(replacement) = &self.replacement {
            msg.push_str(&format!(" Use {} instead.", replacement));
        }
        msg
    }
}

/// Visibility level for a field or model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Visibility {
    /// Publicly visible in all APIs and documentation.
    #[default]
    Public,
    /// Internal - visible in admin APIs but not public.
    Internal,
    /// Hidden - not visible in any generated API.
    Hidden,
    /// Private - only accessible within the application.
    Private,
}

impl Visibility {
    /// Check if visible in public APIs.
    pub fn is_public(&self) -> bool {
        matches!(self, Self::Public)
    }

    /// Check if visible in admin APIs.
    pub fn is_admin_visible(&self) -> bool {
        matches!(self, Self::Public | Self::Internal)
    }

    /// Parse from string.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "public" => Some(Self::Public),
            "internal" => Some(Self::Internal),
            "hidden" => Some(Self::Hidden),
            "private" => Some(Self::Private),
            _ => None,
        }
    }
}

impl std::fmt::Display for Visibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Public => write!(f, "public"),
            Self::Internal => write!(f, "internal"),
            Self::Hidden => write!(f, "hidden"),
            Self::Private => write!(f, "private"),
        }
    }
}

/// API operation permissions for a field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FieldPermissions {
    /// Field can be read.
    pub read: bool,
    /// Field can be set on create.
    pub create: bool,
    /// Field can be updated.
    pub update: bool,
    /// Field can be used in filters.
    pub filter: bool,
    /// Field can be used in sorting.
    pub sort: bool,
}

impl FieldPermissions {
    /// All permissions enabled.
    pub fn all() -> Self {
        Self {
            read: true,
            create: true,
            update: true,
            filter: true,
            sort: true,
        }
    }

    /// Read-only permissions.
    pub fn readonly() -> Self {
        Self {
            read: true,
            create: false,
            update: false,
            filter: true,
            sort: true,
        }
    }

    /// Write-only permissions (e.g., passwords).
    pub fn writeonly() -> Self {
        Self {
            read: false,
            create: true,
            update: true,
            filter: false,
            sort: false,
        }
    }

    /// No permissions (internal field).
    pub fn none() -> Self {
        Self::default()
    }

    /// Create from field metadata.
    pub fn from_metadata(meta: &FieldMetadata) -> Self {
        if meta.hidden {
            return Self::none();
        }

        Self {
            read: !meta.writeonly && !meta.omit_from_output,
            create: !meta.readonly && !meta.output_only && !meta.omit_from_input,
            update: !meta.readonly && !meta.output_only && !meta.omit_from_input,
            filter: !meta.writeonly && !meta.sensitive,
            sort: !meta.writeonly && !meta.sensitive,
        }
    }
}

/// Parse a validation rule from a string.
fn parse_validation_rule(s: &str, span: Span) -> Option<ValidationRule> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    // Check for function-style validators: name(args)
    if let Some(paren_idx) = s.find('(') {
        let name = &s[..paren_idx];
        let args_str = s[paren_idx + 1..].trim_end_matches(')');

        let rule_type = match name {
            "minLength" | "min_length" => {
                let n: usize = args_str.trim().parse().ok()?;
                ValidationType::MinLength(n)
            }
            "maxLength" | "max_length" => {
                let n: usize = args_str.trim().parse().ok()?;
                ValidationType::MaxLength(n)
            }
            "length" => {
                let parts: Vec<&str> = args_str.split(',').collect();
                if parts.len() == 2 {
                    let min: usize = parts[0].trim().parse().ok()?;
                    let max: usize = parts[1].trim().parse().ok()?;
                    ValidationType::Length { min, max }
                } else {
                    return None;
                }
            }
            "min" => {
                let n: f64 = args_str.trim().parse().ok()?;
                ValidationType::Min(n)
            }
            "max" => {
                let n: f64 = args_str.trim().parse().ok()?;
                ValidationType::Max(n)
            }
            "range" => {
                let parts: Vec<&str> = args_str.split(',').collect();
                if parts.len() == 2 {
                    let min: f64 = parts[0].trim().parse().ok()?;
                    let max: f64 = parts[1].trim().parse().ok()?;
                    ValidationType::Range { min, max }
                } else {
                    return None;
                }
            }
            "regex" => {
                let pattern = args_str.trim().trim_matches('"').trim_matches('\'');
                ValidationType::Regex(pattern.to_string())
            }
            "startsWith" | "starts_with" => {
                let prefix = args_str.trim().trim_matches('"').trim_matches('\'');
                ValidationType::StartsWith(prefix.to_string())
            }
            "endsWith" | "ends_with" => {
                let suffix = args_str.trim().trim_matches('"').trim_matches('\'');
                ValidationType::EndsWith(suffix.to_string())
            }
            "contains" => {
                let substring = args_str.trim().trim_matches('"').trim_matches('\'');
                ValidationType::Contains(substring.to_string())
            }
            "minItems" | "min_items" => {
                let n: usize = args_str.trim().parse().ok()?;
                ValidationType::MinItems(n)
            }
            "maxItems" | "max_items" => {
                let n: usize = args_str.trim().parse().ok()?;
                ValidationType::MaxItems(n)
            }
            "items" => {
                let parts: Vec<&str> = args_str.split(',').collect();
                if parts.len() == 2 {
                    let min: usize = parts[0].trim().parse().ok()?;
                    let max: usize = parts[1].trim().parse().ok()?;
                    ValidationType::Items { min, max }
                } else {
                    return None;
                }
            }
            "multipleOf" | "multiple_of" => {
                let n: f64 = args_str.trim().parse().ok()?;
                ValidationType::MultipleOf(n)
            }
            "after" => {
                let date = args_str.trim().trim_matches('"').trim_matches('\'');
                ValidationType::After(date.to_string())
            }
            "before" => {
                let date = args_str.trim().trim_matches('"').trim_matches('\'');
                ValidationType::Before(date.to_string())
            }
            "oneOf" | "one_of" => {
                let values = parse_one_of_values(args_str);
                ValidationType::OneOf(values)
            }
            "custom" => {
                let name = args_str.trim().trim_matches('"').trim_matches('\'');
                ValidationType::Custom(name.to_string())
            }
            _ => return None,
        };

        Some(ValidationRule::new(rule_type, span))
    } else {
        // Simple validators without arguments
        let rule_type = match s {
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
            "noWhitespace" | "no_whitespace" => ValidationType::NoWhitespace,
            "ip" => ValidationType::Ip,
            "ipv4" => ValidationType::Ipv4,
            "ipv6" => ValidationType::Ipv6,
            "creditCard" | "credit_card" => ValidationType::CreditCard,
            "phone" => ValidationType::Phone,
            "slug" => ValidationType::Slug,
            "hex" => ValidationType::Hex,
            "base64" => ValidationType::Base64,
            "json" => ValidationType::Json,
            "positive" => ValidationType::Positive,
            "negative" => ValidationType::Negative,
            "nonNegative" | "non_negative" => ValidationType::NonNegative,
            "nonPositive" | "non_positive" => ValidationType::NonPositive,
            "integer" => ValidationType::Integer,
            "finite" => ValidationType::Finite,
            "unique" => ValidationType::Unique,
            "nonEmpty" | "non_empty" => ValidationType::NonEmpty,
            "past" => ValidationType::Past,
            "future" => ValidationType::Future,
            "pastOrPresent" | "past_or_present" => ValidationType::PastOrPresent,
            "futureOrPresent" | "future_or_present" => ValidationType::FutureOrPresent,
            "required" => ValidationType::Required,
            "notEmpty" | "not_empty" => ValidationType::NotEmpty,
            _ => return None,
        };

        Some(ValidationRule::new(rule_type, span))
    }
}

/// Parse values for oneOf validation.
fn parse_one_of_values(s: &str) -> Vec<ValidationValue> {
    let mut values = Vec::new();

    // Simple parsing - split by comma, handling quoted strings
    let mut current = String::new();
    let mut in_quotes = false;
    let mut quote_char = '"';

    for c in s.chars() {
        match c {
            '"' | '\'' if !in_quotes => {
                in_quotes = true;
                quote_char = c;
            }
            c if c == quote_char && in_quotes => {
                in_quotes = false;
            }
            ',' if !in_quotes => {
                if let Some(val) = parse_validation_value(current.trim()) {
                    values.push(val);
                }
                current.clear();
            }
            _ => {
                current.push(c);
            }
        }
    }

    // Don't forget the last value
    if let Some(val) = parse_validation_value(current.trim()) {
        values.push(val);
    }

    values
}

/// Parse a single validation value.
fn parse_validation_value(s: &str) -> Option<ValidationValue> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    // Check for quoted string
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        let inner = &s[1..s.len() - 1];
        return Some(ValidationValue::String(inner.to_string()));
    }

    // Check for boolean
    if s == "true" {
        return Some(ValidationValue::Bool(true));
    }
    if s == "false" {
        return Some(ValidationValue::Bool(false));
    }

    // Check for integer
    if let Ok(i) = s.parse::<i64>() {
        return Some(ValidationValue::Int(i));
    }

    // Check for float
    if let Ok(f) = s.parse::<f64>() {
        return Some(ValidationValue::Float(f));
    }

    // Default to string
    Some(ValidationValue::String(s.to_string()))
}

/// Parse a documentation tag.
fn parse_doc_tag(s: &str, span: Span) -> Option<DocTag> {
    if !s.starts_with('@') || s.starts_with("@validate") {
        return None;
    }

    let content = &s[1..]; // Remove @
    let (name, value) = if let Some(space_idx) = content.find(char::is_whitespace) {
        (
            &content[..space_idx],
            Some(content[space_idx..].trim().to_string()),
        )
    } else {
        (content, None)
    };

    Some(DocTag::new(name, value, span))
}

#[cfg(test)]
// Float literals like `3.14` are sample validation values, not math constants.
#[allow(clippy::approx_constant)]
mod tests {
    use super::*;

    #[test]
    fn test_validation_rule_new() {
        let rule = ValidationRule::new(ValidationType::Email, Span::new(0, 0));
        assert!(matches!(rule.rule_type, ValidationType::Email));
        assert!(rule.message.is_none());
    }

    #[test]
    fn test_validation_rule_with_message() {
        let rule = ValidationRule::new(ValidationType::Email, Span::new(0, 0))
            .with_message("Please enter a valid email");
        assert_eq!(rule.message, Some("Please enter a valid email".to_string()));
    }

    #[test]
    fn test_validation_type_default_messages() {
        let email_msg = ValidationType::Email.default_message("email");
        assert!(email_msg.contains("email"));
        assert!(email_msg.contains("valid"));

        let min_msg = ValidationType::Min(10.0).default_message("age");
        assert!(min_msg.contains("age"));
        assert!(min_msg.contains("10"));
    }

    #[test]
    fn test_validation_type_is_string_rule() {
        assert!(ValidationType::Email.is_string_rule());
        assert!(ValidationType::Regex(".*".to_string()).is_string_rule());
        assert!(!ValidationType::Min(0.0).is_string_rule());
    }

    #[test]
    fn test_validation_type_is_numeric_rule() {
        assert!(ValidationType::Min(0.0).is_numeric_rule());
        assert!(ValidationType::Positive.is_numeric_rule());
        assert!(!ValidationType::Email.is_numeric_rule());
    }

    #[test]
    fn test_validation_type_is_array_rule() {
        assert!(ValidationType::MinItems(1).is_array_rule());
        assert!(ValidationType::Unique.is_array_rule());
        assert!(!ValidationType::Email.is_array_rule());
    }

    #[test]
    fn test_validation_type_is_date_rule() {
        assert!(ValidationType::Past.is_date_rule());
        assert!(ValidationType::After("2024-01-01".to_string()).is_date_rule());
        assert!(!ValidationType::Email.is_date_rule());
    }

    #[test]
    fn test_field_validation() {
        let mut validation = FieldValidation::new();
        assert!(validation.is_empty());

        validation.add_rule(ValidationRule::new(ValidationType::Email, Span::new(0, 0)));
        validation.add_rule(ValidationRule::new(
            ValidationType::MaxLength(255),
            Span::new(0, 0),
        ));

        assert_eq!(validation.len(), 2);
        assert!(!validation.is_empty());
        assert!(validation.has_string_rules());
    }

    #[test]
    fn test_field_validation_is_required() {
        let mut validation = FieldValidation::new();
        assert!(!validation.is_required());

        validation.add_rule(ValidationRule::new(
            ValidationType::Required,
            Span::new(0, 0),
        ));
        assert!(validation.is_required());
    }

    #[test]
    fn test_parse_validation_rule_simple() {
        let span = Span::new(0, 0);

        let email = parse_validation_rule("email", span).unwrap();
        assert!(matches!(email.rule_type, ValidationType::Email));

        let uuid = parse_validation_rule("uuid", span).unwrap();
        assert!(matches!(uuid.rule_type, ValidationType::Uuid));

        let positive = parse_validation_rule("positive", span).unwrap();
        assert!(matches!(positive.rule_type, ValidationType::Positive));
    }

    #[test]
    fn test_parse_validation_rule_with_args() {
        let span = Span::new(0, 0);

        let min_length = parse_validation_rule("minLength(5)", span).unwrap();
        assert!(matches!(min_length.rule_type, ValidationType::MinLength(5)));

        let max = parse_validation_rule("max(100)", span).unwrap();
        assert!(
            matches!(max.rule_type, ValidationType::Max(n) if (n - 100.0).abs() < f64::EPSILON)
        );

        let range = parse_validation_rule("range(0, 100)", span).unwrap();
        if let ValidationType::Range { min, max } = range.rule_type {
            assert!((min - 0.0).abs() < f64::EPSILON);
            assert!((max - 100.0).abs() < f64::EPSILON);
        } else {
            panic!("Expected Range");
        }
    }

    #[test]
    fn test_parse_validation_rule_regex() {
        let span = Span::new(0, 0);

        let regex = parse_validation_rule(r#"regex("^[a-z]+$")"#, span).unwrap();
        if let ValidationType::Regex(pattern) = regex.rule_type {
            assert_eq!(pattern, "^[a-z]+$");
        } else {
            panic!("Expected Regex");
        }
    }

    #[test]
    fn test_parse_validation_rule_one_of() {
        let span = Span::new(0, 0);

        let one_of = parse_validation_rule(r#"oneOf("a", "b", "c")"#, span).unwrap();
        if let ValidationType::OneOf(values) = one_of.rule_type {
            assert_eq!(values.len(), 3);
            assert_eq!(values[0], ValidationValue::String("a".to_string()));
        } else {
            panic!("Expected OneOf");
        }
    }

    #[test]
    fn test_parse_validation_value() {
        assert_eq!(
            parse_validation_value("\"hello\""),
            Some(ValidationValue::String("hello".to_string()))
        );
        assert_eq!(parse_validation_value("42"), Some(ValidationValue::Int(42)));
        assert_eq!(
            parse_validation_value("3.14"),
            Some(ValidationValue::Float(3.14))
        );
        assert_eq!(
            parse_validation_value("true"),
            Some(ValidationValue::Bool(true))
        );
    }

    #[test]
    fn test_enhanced_documentation_parse() {
        let raw = r#"The user's email address
@validate: email, maxLength(255)
@deprecated Use newEmail instead"#;

        let doc = EnhancedDocumentation::parse(raw, Span::new(0, 0));

        assert_eq!(doc.text, "The user's email address");
        assert!(doc.has_validation());
        assert_eq!(doc.validation.len(), 2);
        assert_eq!(doc.tags.len(), 1);
        assert_eq!(doc.tags[0].name.as_str(), "deprecated");
    }

    #[test]
    fn test_enhanced_documentation_multiple_validate_lines() {
        let raw = r#"Username must be valid
@validate: minLength(3), maxLength(30)
@validate: regex("^[a-z0-9_]+$")"#;

        let doc = EnhancedDocumentation::parse(raw, Span::new(0, 0));

        assert_eq!(doc.text, "Username must be valid");
        assert_eq!(doc.validation.len(), 3);
    }

    #[test]
    fn test_doc_tag_parsing() {
        let span = Span::new(0, 0);

        let tag = parse_doc_tag("@deprecated Use newField instead", span).unwrap();
        assert_eq!(tag.name.as_str(), "deprecated");
        assert_eq!(tag.value, Some("Use newField instead".to_string()));

        let tag_no_value = parse_doc_tag("@internal", span).unwrap();
        assert_eq!(tag_no_value.name.as_str(), "internal");
        assert!(tag_no_value.value.is_none());
    }

    #[test]
    fn test_validation_value_display() {
        assert_eq!(
            format!("{}", ValidationValue::String("test".to_string())),
            "\"test\""
        );
        assert_eq!(format!("{}", ValidationValue::Int(42)), "42");
        assert_eq!(format!("{}", ValidationValue::Float(3.14)), "3.14");
        assert_eq!(format!("{}", ValidationValue::Bool(true)), "true");
    }

    #[test]
    fn test_validator_name() {
        assert_eq!(ValidationType::Email.validator_name(), "email");
        assert_eq!(ValidationType::MinLength(5).validator_name(), "min_length");
        assert_eq!(
            ValidationType::Range {
                min: 0.0,
                max: 100.0
            }
            .validator_name(),
            "range"
        );
    }

    // ==================== Field Metadata Tests ====================

    #[test]
    fn test_field_metadata_default() {
        let meta = FieldMetadata::new();
        assert!(!meta.hidden);
        assert!(!meta.internal);
        assert!(!meta.sensitive);
        assert!(!meta.readonly);
        assert!(!meta.writeonly);
        assert!(meta.deprecated.is_none());
        assert!(meta.label.is_none());
        assert!(meta.examples.is_empty());
    }

    #[test]
    fn test_field_metadata_from_tags() {
        let span = Span::new(0, 0);
        let tags = vec![
            DocTag::new("hidden", None, span),
            DocTag::new("sensitive", None, span),
            DocTag::new("label", Some("User ID".to_string()), span),
            DocTag::new("example", Some("12345".to_string()), span),
            DocTag::new("example", Some("67890".to_string()), span),
        ];

        let meta = FieldMetadata::from_tags(&tags);

        assert!(meta.hidden);
        assert!(meta.sensitive);
        assert_eq!(meta.label, Some("User ID".to_string()));
        assert_eq!(meta.examples.len(), 2);
        assert_eq!(meta.examples[0], "12345");
        assert_eq!(meta.examples[1], "67890");
    }

    #[test]
    fn test_field_metadata_deprecated() {
        let span = Span::new(0, 0);
        let tags = vec![DocTag::new(
            "deprecated",
            Some("Use newField instead".to_string()),
            span,
        )];

        let meta = FieldMetadata::from_tags(&tags);

        assert!(meta.is_deprecated());
        assert_eq!(meta.deprecation_message(), Some("Use newField instead"));
    }

    #[test]
    fn test_field_metadata_readonly_writeonly() {
        let span = Span::new(0, 0);

        let readonly_tags = vec![DocTag::new("readonly", None, span)];
        let readonly_meta = FieldMetadata::from_tags(&readonly_tags);
        assert!(readonly_meta.readonly);
        assert!(readonly_meta.should_omit_from_input());
        assert!(!readonly_meta.should_omit_from_output());

        let writeonly_tags = vec![DocTag::new("writeonly", None, span)];
        let writeonly_meta = FieldMetadata::from_tags(&writeonly_tags);
        assert!(writeonly_meta.writeonly);
        assert!(writeonly_meta.should_omit_from_output());
        assert!(!writeonly_meta.should_omit_from_input());
    }

    #[test]
    fn test_field_metadata_serialization() {
        let span = Span::new(0, 0);
        let tags = vec![
            DocTag::new("alias", Some("userId".to_string()), span),
            DocTag::new("serializedName", Some("user_id".to_string()), span),
            DocTag::new("order", Some("1".to_string()), span),
        ];

        let meta = FieldMetadata::from_tags(&tags);

        assert_eq!(meta.alias, Some("userId".to_string()));
        assert_eq!(meta.serialized_name, Some("user_id".to_string()));
        assert_eq!(meta.order, Some(1));
    }

    #[test]
    fn test_field_metadata_ui_hints() {
        let span = Span::new(0, 0);
        let tags = vec![
            DocTag::new("group", Some("Personal Info".to_string()), span),
            DocTag::new("format", Some("date".to_string()), span),
            DocTag::new("inputType", Some("textarea".to_string()), span),
            DocTag::new("multiline", None, span),
            DocTag::new("maxWidth", Some("500".to_string()), span),
        ];

        let meta = FieldMetadata::from_tags(&tags);

        assert_eq!(meta.group, Some("Personal Info".to_string()));
        assert_eq!(meta.format, Some("date".to_string()));
        assert_eq!(meta.input_type, Some("textarea".to_string()));
        assert!(meta.multiline);
        assert_eq!(meta.max_width, Some(500));
    }

    #[test]
    fn test_deprecation_info() {
        let info = DeprecationInfo::new("Field is deprecated")
            .since("2.0.0")
            .replacement("newField");

        assert_eq!(info.message, "Field is deprecated");
        assert_eq!(info.since, Some("2.0.0".to_string()));
        assert_eq!(info.replacement, Some("newField".to_string()));

        let formatted = info.format_message();
        assert!(formatted.contains("Field is deprecated"));
        assert!(formatted.contains("since 2.0.0"));
        assert!(formatted.contains("Use newField instead"));
    }

    #[test]
    fn test_visibility_levels() {
        assert!(Visibility::Public.is_public());
        assert!(Visibility::Public.is_admin_visible());

        assert!(!Visibility::Internal.is_public());
        assert!(Visibility::Internal.is_admin_visible());

        assert!(!Visibility::Hidden.is_public());
        assert!(!Visibility::Hidden.is_admin_visible());

        assert!(!Visibility::Private.is_public());
        assert!(!Visibility::Private.is_admin_visible());
    }

    #[test]
    fn test_visibility_from_str() {
        assert_eq!(Visibility::parse("public"), Some(Visibility::Public));
        assert_eq!(Visibility::parse("INTERNAL"), Some(Visibility::Internal));
        assert_eq!(Visibility::parse("Hidden"), Some(Visibility::Hidden));
        assert_eq!(Visibility::parse("private"), Some(Visibility::Private));
        assert_eq!(Visibility::parse("unknown"), None);
    }

    #[test]
    fn test_field_permissions_all() {
        let perms = FieldPermissions::all();
        assert!(perms.read);
        assert!(perms.create);
        assert!(perms.update);
        assert!(perms.filter);
        assert!(perms.sort);
    }

    #[test]
    fn test_field_permissions_readonly() {
        let perms = FieldPermissions::readonly();
        assert!(perms.read);
        assert!(!perms.create);
        assert!(!perms.update);
        assert!(perms.filter);
        assert!(perms.sort);
    }

    #[test]
    fn test_field_permissions_writeonly() {
        let perms = FieldPermissions::writeonly();
        assert!(!perms.read);
        assert!(perms.create);
        assert!(perms.update);
        assert!(!perms.filter);
        assert!(!perms.sort);
    }

    #[test]
    fn test_field_permissions_from_metadata() {
        let mut meta = FieldMetadata::new();
        meta.readonly = true;

        let perms = FieldPermissions::from_metadata(&meta);
        assert!(perms.read);
        assert!(!perms.create);
        assert!(!perms.update);

        let mut sensitive_meta = FieldMetadata::new();
        sensitive_meta.sensitive = true;

        let sensitive_perms = FieldPermissions::from_metadata(&sensitive_meta);
        assert!(sensitive_perms.read);
        assert!(!sensitive_perms.filter);
        assert!(!sensitive_perms.sort);
    }

    #[test]
    fn test_enhanced_documentation_metadata_extraction() {
        let raw = r#"User's password hash
@hidden
@sensitive
@writeonly
@label Password
@since 1.0.0"#;

        let doc = EnhancedDocumentation::parse(raw, Span::new(0, 0));

        assert!(doc.is_hidden());
        assert!(doc.is_sensitive());
        assert!(doc.is_writeonly());
        assert_eq!(doc.label(), Some("Password"));
        assert_eq!(doc.since(), Some("1.0.0"));

        let meta = doc.extract_metadata();
        assert!(meta.hidden);
        assert!(meta.sensitive);
        assert!(meta.writeonly);
    }

    #[test]
    fn test_enhanced_documentation_examples() {
        let raw = r#"Email address
@example user@example.com
@example admin@company.org
@placeholder Enter your email"#;

        let doc = EnhancedDocumentation::parse(raw, Span::new(0, 0));

        let examples = doc.examples();
        assert_eq!(examples.len(), 2);
        assert_eq!(examples[0], "user@example.com");
        assert_eq!(examples[1], "admin@company.org");
        assert_eq!(doc.placeholder(), Some("Enter your email"));
    }

    #[test]
    fn test_enhanced_documentation_deprecation() {
        let raw = r#"Old email field
@deprecated Use newEmail instead
@since 1.0.0"#;

        let doc = EnhancedDocumentation::parse(raw, Span::new(0, 0));

        assert!(doc.is_deprecated());
        let info = doc.deprecation_info().unwrap();
        assert_eq!(info.message, "Use newEmail instead");
        assert_eq!(info.since, Some("1.0.0".to_string()));
    }

    #[test]
    fn test_enhanced_documentation_group() {
        let raw = r#"User's display name
@group Personal Information
@format text"#;

        let doc = EnhancedDocumentation::parse(raw, Span::new(0, 0));

        assert_eq!(doc.group(), Some("Personal Information"));
        let meta = doc.extract_metadata();
        assert_eq!(meta.format, Some("text".to_string()));
    }

    // ==================== Additional Validation Type Tests ====================

    #[test]
    fn test_validation_rule_error_message_custom() {
        let rule = ValidationRule::new(ValidationType::Email, Span::new(0, 0))
            .with_message("Please provide a valid email");
        assert_eq!(rule.error_message("email"), "Please provide a valid email");
    }

    #[test]
    fn test_validation_rule_error_message_default() {
        let rule = ValidationRule::new(ValidationType::Email, Span::new(0, 0));
        let msg = rule.error_message("email");
        assert!(msg.contains("email"));
    }

    #[test]
    fn test_validation_rule_type_checks() {
        let email_rule = ValidationRule::new(ValidationType::Email, Span::new(0, 0));
        assert!(email_rule.is_string_rule());
        assert!(!email_rule.is_numeric_rule());
        assert!(!email_rule.is_array_rule());
        assert!(!email_rule.is_date_rule());

        let min_rule = ValidationRule::new(ValidationType::Min(0.0), Span::new(0, 0));
        assert!(!min_rule.is_string_rule());
        assert!(min_rule.is_numeric_rule());
        assert!(!min_rule.is_array_rule());

        let items_rule = ValidationRule::new(ValidationType::MinItems(1), Span::new(0, 0));
        assert!(!items_rule.is_string_rule());
        assert!(!items_rule.is_numeric_rule());
        assert!(items_rule.is_array_rule());

        let past_rule = ValidationRule::new(ValidationType::Past, Span::new(0, 0));
        assert!(!past_rule.is_string_rule());
        assert!(!past_rule.is_numeric_rule());
        assert!(!past_rule.is_array_rule());
        assert!(past_rule.is_date_rule());
    }

    #[test]
    fn test_validation_type_is_id_format_rule() {
        assert!(ValidationType::Uuid.is_id_format_rule());
        assert!(ValidationType::Cuid.is_id_format_rule());
        assert!(ValidationType::Cuid2.is_id_format_rule());
        assert!(ValidationType::NanoId.is_id_format_rule());
        assert!(ValidationType::Ulid.is_id_format_rule());
        assert!(!ValidationType::Email.is_id_format_rule());
    }

    #[test]
    fn test_validation_type_default_messages_comprehensive() {
        // String validators
        assert!(
            ValidationType::Url
                .default_message("website")
                .contains("URL")
        );
        assert!(ValidationType::Cuid.default_message("id").contains("CUID"));
        assert!(
            ValidationType::Cuid2
                .default_message("id")
                .contains("CUID2")
        );
        assert!(
            ValidationType::NanoId
                .default_message("id")
                .contains("NanoId")
        );
        assert!(ValidationType::Ulid.default_message("id").contains("ULID"));
        assert!(
            ValidationType::Alpha
                .default_message("name")
                .contains("letters")
        );
        assert!(
            ValidationType::Alphanumeric
                .default_message("code")
                .contains("letters and numbers")
        );
        assert!(
            ValidationType::Lowercase
                .default_message("slug")
                .contains("lowercase")
        );
        assert!(
            ValidationType::Uppercase
                .default_message("code")
                .contains("uppercase")
        );
        assert!(
            ValidationType::Trim
                .default_message("text")
                .contains("whitespace")
        );
        assert!(
            ValidationType::NoWhitespace
                .default_message("username")
                .contains("whitespace")
        );
        assert!(ValidationType::Ip.default_message("address").contains("IP"));
        assert!(
            ValidationType::Ipv4
                .default_message("address")
                .contains("IPv4")
        );
        assert!(
            ValidationType::Ipv6
                .default_message("address")
                .contains("IPv6")
        );
        assert!(
            ValidationType::CreditCard
                .default_message("card")
                .contains("credit card")
        );
        assert!(
            ValidationType::Phone
                .default_message("phone")
                .contains("phone")
        );
        assert!(ValidationType::Slug.default_message("url").contains("slug"));
        assert!(
            ValidationType::Hex
                .default_message("color")
                .contains("hexadecimal")
        );
        assert!(
            ValidationType::Base64
                .default_message("data")
                .contains("base64")
        );
        assert!(
            ValidationType::Json
                .default_message("config")
                .contains("JSON")
        );
        assert!(
            ValidationType::StartsWith("test".to_string())
                .default_message("field")
                .contains("start with")
        );
        assert!(
            ValidationType::EndsWith(".json".to_string())
                .default_message("file")
                .contains("end with")
        );
        assert!(
            ValidationType::Contains("keyword".to_string())
                .default_message("text")
                .contains("contain")
        );
        assert!(
            ValidationType::Length { min: 5, max: 10 }
                .default_message("text")
                .contains("between")
        );

        // Numeric validators
        assert!(
            ValidationType::Negative
                .default_message("balance")
                .contains("negative")
        );
        assert!(
            ValidationType::NonNegative
                .default_message("count")
                .contains("not be negative")
        );
        assert!(
            ValidationType::NonPositive
                .default_message("debt")
                .contains("not be positive")
        );
        assert!(
            ValidationType::Integer
                .default_message("count")
                .contains("integer")
        );
        assert!(
            ValidationType::MultipleOf(5.0)
                .default_message("value")
                .contains("multiple")
        );
        assert!(
            ValidationType::Finite
                .default_message("value")
                .contains("finite")
        );

        // Array validators
        assert!(
            ValidationType::MaxItems(10)
                .default_message("items")
                .contains("at most")
        );
        assert!(
            ValidationType::Items { min: 1, max: 5 }
                .default_message("tags")
                .contains("between")
        );
        assert!(
            ValidationType::Unique
                .default_message("items")
                .contains("unique")
        );

        // Date validators
        assert!(
            ValidationType::Future
                .default_message("expiry")
                .contains("future")
        );
        assert!(
            ValidationType::PastOrPresent
                .default_message("login")
                .contains("not be in the future")
        );
        assert!(
            ValidationType::FutureOrPresent
                .default_message("deadline")
                .contains("not be in the past")
        );
        assert!(
            ValidationType::Before("2025-01-01".to_string())
                .default_message("date")
                .contains("before")
        );

        // General validators
        assert!(
            ValidationType::Required
                .default_message("field")
                .contains("required")
        );
        assert!(
            ValidationType::NotEmpty
                .default_message("list")
                .contains("not be empty")
        );
        assert!(
            ValidationType::Custom("strongPassword".to_string())
                .default_message("password")
                .contains("custom")
        );
    }

    #[test]
    fn test_validation_type_validator_names() {
        assert_eq!(ValidationType::Url.validator_name(), "url");
        assert_eq!(ValidationType::Cuid.validator_name(), "cuid");
        assert_eq!(ValidationType::Cuid2.validator_name(), "cuid2");
        assert_eq!(ValidationType::NanoId.validator_name(), "nanoid");
        assert_eq!(ValidationType::Ulid.validator_name(), "ulid");
        assert_eq!(ValidationType::Alpha.validator_name(), "alpha");
        assert_eq!(
            ValidationType::Alphanumeric.validator_name(),
            "alphanumeric"
        );
        assert_eq!(ValidationType::Lowercase.validator_name(), "lowercase");
        assert_eq!(ValidationType::Uppercase.validator_name(), "uppercase");
        assert_eq!(ValidationType::Trim.validator_name(), "trim");
        assert_eq!(
            ValidationType::NoWhitespace.validator_name(),
            "no_whitespace"
        );
        assert_eq!(ValidationType::Ip.validator_name(), "ip");
        assert_eq!(ValidationType::Ipv4.validator_name(), "ipv4");
        assert_eq!(ValidationType::Ipv6.validator_name(), "ipv6");
        assert_eq!(ValidationType::CreditCard.validator_name(), "credit_card");
        assert_eq!(ValidationType::Phone.validator_name(), "phone");
        assert_eq!(ValidationType::Slug.validator_name(), "slug");
        assert_eq!(ValidationType::Hex.validator_name(), "hex");
        assert_eq!(ValidationType::Base64.validator_name(), "base64");
        assert_eq!(ValidationType::Json.validator_name(), "json");
        assert_eq!(
            ValidationType::StartsWith("".to_string()).validator_name(),
            "starts_with"
        );
        assert_eq!(
            ValidationType::EndsWith("".to_string()).validator_name(),
            "ends_with"
        );
        assert_eq!(
            ValidationType::Contains("".to_string()).validator_name(),
            "contains"
        );
        assert_eq!(
            ValidationType::Length { min: 0, max: 0 }.validator_name(),
            "length"
        );
        assert_eq!(ValidationType::Max(0.0).validator_name(), "max");
        assert_eq!(ValidationType::Negative.validator_name(), "negative");
        assert_eq!(ValidationType::NonNegative.validator_name(), "non_negative");
        assert_eq!(ValidationType::NonPositive.validator_name(), "non_positive");
        assert_eq!(ValidationType::Integer.validator_name(), "integer");
        assert_eq!(
            ValidationType::MultipleOf(0.0).validator_name(),
            "multiple_of"
        );
        assert_eq!(ValidationType::Finite.validator_name(), "finite");
        assert_eq!(ValidationType::MaxItems(0).validator_name(), "max_items");
        assert_eq!(
            ValidationType::Items { min: 0, max: 0 }.validator_name(),
            "items"
        );
        assert_eq!(ValidationType::Unique.validator_name(), "unique");
        assert_eq!(ValidationType::NonEmpty.validator_name(), "non_empty");
        assert_eq!(ValidationType::Future.validator_name(), "future");
        assert_eq!(
            ValidationType::PastOrPresent.validator_name(),
            "past_or_present"
        );
        assert_eq!(
            ValidationType::FutureOrPresent.validator_name(),
            "future_or_present"
        );
        assert_eq!(
            ValidationType::After("".to_string()).validator_name(),
            "after"
        );
        assert_eq!(
            ValidationType::Before("".to_string()).validator_name(),
            "before"
        );
        assert_eq!(ValidationType::Required.validator_name(), "required");
        assert_eq!(ValidationType::NotEmpty.validator_name(), "not_empty");
        assert_eq!(ValidationType::OneOf(vec![]).validator_name(), "one_of");
        assert_eq!(
            ValidationType::Custom("".to_string()).validator_name(),
            "custom"
        );
    }

    #[test]
    fn test_field_validation_has_rules() {
        let mut validation = FieldValidation::new();
        assert!(!validation.has_numeric_rules());
        assert!(!validation.has_array_rules());

        validation.add_rule(ValidationRule::new(
            ValidationType::Min(0.0),
            Span::new(0, 0),
        ));
        assert!(validation.has_numeric_rules());

        let mut arr_validation = FieldValidation::new();
        arr_validation.add_rule(ValidationRule::new(
            ValidationType::MinItems(1),
            Span::new(0, 0),
        ));
        assert!(arr_validation.has_array_rules());

        // Date rules are checked via rule type
        let mut date_validation = FieldValidation::new();
        date_validation.add_rule(ValidationRule::new(ValidationType::Past, Span::new(0, 0)));
        assert!(date_validation.rules.iter().any(|r| r.is_date_rule()));
    }

    #[test]
    fn test_parse_validation_rule_more_validators() {
        let span = Span::new(0, 0);

        // String validators
        let url = parse_validation_rule("url", span).unwrap();
        assert!(matches!(url.rule_type, ValidationType::Url));

        let cuid = parse_validation_rule("cuid", span).unwrap();
        assert!(matches!(cuid.rule_type, ValidationType::Cuid));

        let cuid2 = parse_validation_rule("cuid2", span).unwrap();
        assert!(matches!(cuid2.rule_type, ValidationType::Cuid2));

        let nanoid = parse_validation_rule("nanoid", span).unwrap();
        assert!(matches!(nanoid.rule_type, ValidationType::NanoId));

        let ulid = parse_validation_rule("ulid", span).unwrap();
        assert!(matches!(ulid.rule_type, ValidationType::Ulid));

        let alpha = parse_validation_rule("alpha", span).unwrap();
        assert!(matches!(alpha.rule_type, ValidationType::Alpha));

        let alphanumeric = parse_validation_rule("alphanumeric", span).unwrap();
        assert!(matches!(
            alphanumeric.rule_type,
            ValidationType::Alphanumeric
        ));

        let lowercase = parse_validation_rule("lowercase", span).unwrap();
        assert!(matches!(lowercase.rule_type, ValidationType::Lowercase));

        let uppercase = parse_validation_rule("uppercase", span).unwrap();
        assert!(matches!(uppercase.rule_type, ValidationType::Uppercase));

        let trim = parse_validation_rule("trim", span).unwrap();
        assert!(matches!(trim.rule_type, ValidationType::Trim));

        let no_whitespace = parse_validation_rule("noWhitespace", span).unwrap();
        assert!(matches!(
            no_whitespace.rule_type,
            ValidationType::NoWhitespace
        ));

        let ip = parse_validation_rule("ip", span).unwrap();
        assert!(matches!(ip.rule_type, ValidationType::Ip));

        let ipv4 = parse_validation_rule("ipv4", span).unwrap();
        assert!(matches!(ipv4.rule_type, ValidationType::Ipv4));

        let ipv6 = parse_validation_rule("ipv6", span).unwrap();
        assert!(matches!(ipv6.rule_type, ValidationType::Ipv6));

        let credit_card = parse_validation_rule("creditCard", span).unwrap();
        assert!(matches!(credit_card.rule_type, ValidationType::CreditCard));

        let phone = parse_validation_rule("phone", span).unwrap();
        assert!(matches!(phone.rule_type, ValidationType::Phone));

        let slug = parse_validation_rule("slug", span).unwrap();
        assert!(matches!(slug.rule_type, ValidationType::Slug));

        let hex = parse_validation_rule("hex", span).unwrap();
        assert!(matches!(hex.rule_type, ValidationType::Hex));

        let base64 = parse_validation_rule("base64", span).unwrap();
        assert!(matches!(base64.rule_type, ValidationType::Base64));

        let json = parse_validation_rule("json", span).unwrap();
        assert!(matches!(json.rule_type, ValidationType::Json));

        // Numeric validators
        let negative = parse_validation_rule("negative", span).unwrap();
        assert!(matches!(negative.rule_type, ValidationType::Negative));

        let non_negative = parse_validation_rule("nonNegative", span).unwrap();
        assert!(matches!(
            non_negative.rule_type,
            ValidationType::NonNegative
        ));

        let non_positive = parse_validation_rule("nonPositive", span).unwrap();
        assert!(matches!(
            non_positive.rule_type,
            ValidationType::NonPositive
        ));

        let integer = parse_validation_rule("integer", span).unwrap();
        assert!(matches!(integer.rule_type, ValidationType::Integer));

        let finite = parse_validation_rule("finite", span).unwrap();
        assert!(matches!(finite.rule_type, ValidationType::Finite));

        // Array validators
        let unique = parse_validation_rule("unique", span).unwrap();
        assert!(matches!(unique.rule_type, ValidationType::Unique));

        let non_empty = parse_validation_rule("nonEmpty", span).unwrap();
        assert!(matches!(non_empty.rule_type, ValidationType::NonEmpty));

        // Date validators
        let past = parse_validation_rule("past", span).unwrap();
        assert!(matches!(past.rule_type, ValidationType::Past));

        let future = parse_validation_rule("future", span).unwrap();
        assert!(matches!(future.rule_type, ValidationType::Future));

        let past_or_present = parse_validation_rule("pastOrPresent", span).unwrap();
        assert!(matches!(
            past_or_present.rule_type,
            ValidationType::PastOrPresent
        ));

        let future_or_present = parse_validation_rule("futureOrPresent", span).unwrap();
        assert!(matches!(
            future_or_present.rule_type,
            ValidationType::FutureOrPresent
        ));

        // General validators
        let required = parse_validation_rule("required", span).unwrap();
        assert!(matches!(required.rule_type, ValidationType::Required));

        let not_empty = parse_validation_rule("notEmpty", span).unwrap();
        assert!(matches!(not_empty.rule_type, ValidationType::NotEmpty));
    }

    #[test]
    fn test_parse_validation_rule_with_string_args() {
        let span = Span::new(0, 0);

        let starts_with = parse_validation_rule(r#"startsWith("PREFIX_")"#, span).unwrap();
        if let ValidationType::StartsWith(prefix) = starts_with.rule_type {
            assert_eq!(prefix, "PREFIX_");
        } else {
            panic!("Expected StartsWith");
        }

        let ends_with = parse_validation_rule(r#"endsWith(".json")"#, span).unwrap();
        if let ValidationType::EndsWith(suffix) = ends_with.rule_type {
            assert_eq!(suffix, ".json");
        } else {
            panic!("Expected EndsWith");
        }

        let contains = parse_validation_rule(r#"contains("keyword")"#, span).unwrap();
        if let ValidationType::Contains(substring) = contains.rule_type {
            assert_eq!(substring, "keyword");
        } else {
            panic!("Expected Contains");
        }

        let custom = parse_validation_rule(r#"custom("myValidator")"#, span).unwrap();
        if let ValidationType::Custom(name) = custom.rule_type {
            assert_eq!(name, "myValidator");
        } else {
            panic!("Expected Custom");
        }

        let after = parse_validation_rule(r#"after("2024-01-01")"#, span).unwrap();
        if let ValidationType::After(date) = after.rule_type {
            assert_eq!(date, "2024-01-01");
        } else {
            panic!("Expected After");
        }

        let before = parse_validation_rule(r#"before("2025-12-31")"#, span).unwrap();
        if let ValidationType::Before(date) = before.rule_type {
            assert_eq!(date, "2025-12-31");
        } else {
            panic!("Expected Before");
        }
    }

    #[test]
    fn test_parse_validation_rule_numeric_args() {
        let span = Span::new(0, 0);

        let min = parse_validation_rule("min(10)", span).unwrap();
        if let ValidationType::Min(n) = min.rule_type {
            assert!((n - 10.0).abs() < f64::EPSILON);
        } else {
            panic!("Expected Min");
        }

        let max = parse_validation_rule("max(100)", span).unwrap();
        if let ValidationType::Max(n) = max.rule_type {
            assert!((n - 100.0).abs() < f64::EPSILON);
        } else {
            panic!("Expected Max");
        }

        let multiple_of = parse_validation_rule("multipleOf(5)", span).unwrap();
        if let ValidationType::MultipleOf(n) = multiple_of.rule_type {
            assert!((n - 5.0).abs() < f64::EPSILON);
        } else {
            panic!("Expected MultipleOf");
        }

        let min_items = parse_validation_rule("minItems(1)", span).unwrap();
        assert!(matches!(min_items.rule_type, ValidationType::MinItems(1)));

        let max_items = parse_validation_rule("maxItems(10)", span).unwrap();
        assert!(matches!(max_items.rule_type, ValidationType::MaxItems(10)));

        let length = parse_validation_rule("length(5, 100)", span).unwrap();
        if let ValidationType::Length { min, max } = length.rule_type {
            assert_eq!(min, 5);
            assert_eq!(max, 100);
        } else {
            panic!("Expected Length");
        }

        let items = parse_validation_rule("items(1, 10)", span).unwrap();
        if let ValidationType::Items { min, max } = items.rule_type {
            assert_eq!(min, 1);
            assert_eq!(max, 10);
        } else {
            panic!("Expected Items");
        }
    }

    #[test]
    fn test_parse_validation_rule_unknown() {
        let span = Span::new(0, 0);
        assert!(parse_validation_rule("unknownValidator", span).is_none());
    }

    #[test]
    fn test_field_metadata_more_tags() {
        let span = Span::new(0, 0);
        let tags = vec![
            DocTag::new("internal", None, span),
            DocTag::new(
                "description",
                Some("A detailed description".to_string()),
                span,
            ),
            DocTag::new("seeAlso", Some("otherField".to_string()), span),
            DocTag::new("omitFromInput", None, span),
            DocTag::new("omitFromOutput", None, span),
        ];

        let meta = FieldMetadata::from_tags(&tags);

        assert!(meta.internal);
        assert_eq!(meta.description, Some("A detailed description".to_string()));
        assert_eq!(meta.see_also, vec!["otherField".to_string()]);
        assert!(meta.omit_from_input);
        assert!(meta.omit_from_output);
    }

    #[test]
    fn test_field_permissions_none() {
        let perms = FieldPermissions::none();
        assert!(!perms.read);
        assert!(!perms.create);
        assert!(!perms.update);
        assert!(!perms.filter);
        assert!(!perms.sort);
    }

    #[test]
    fn test_enhanced_documentation_no_validation() {
        let raw = "Just a simple description";
        let doc = EnhancedDocumentation::parse(raw, Span::new(0, 0));

        assert_eq!(doc.text, "Just a simple description");
        assert!(!doc.has_validation());
        assert_eq!(doc.validation.len(), 0);
        assert!(doc.tags.is_empty());
    }

    #[test]
    fn test_enhanced_documentation_readonly() {
        let raw = r#"ID field
@readonly"#;

        let doc = EnhancedDocumentation::parse(raw, Span::new(0, 0));

        assert!(doc.is_readonly());
        assert!(!doc.is_hidden());
        assert!(!doc.is_sensitive());
    }
}
