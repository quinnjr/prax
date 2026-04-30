//! GraphQL-specific AST types and configuration.
//!
//! This module provides types for configuring GraphQL behavior directly in the schema,
//! allowing fine-grained control over how models and fields are exposed in GraphQL APIs.
//!
//! # Schema Syntax
//!
//! ```prax
//! model User {
//!     /// @graphql.skip - Hide from GraphQL
//!     internal_id   String
//!
//!     /// @graphql.name("userId") - Custom GraphQL name
//!     id            Int    @id @auto
//!
//!     /// @graphql.complexity(5) - Query complexity
//!     posts         Post[]
//!
//!     /// @graphql.resolver("customEmailResolver")
//!     email         String
//! }
//!
//! /// @graphql.interface - Generate as GraphQL interface
//! model Node {
//!     id            String @id
//! }
//!
//! /// @graphql.union(SearchResult) - Part of union type
//! model Post {
//!     id            Int    @id
//! }
//! ```

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use super::Span;

/// GraphQL-specific configuration for a model or type.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GraphQLConfig {
    /// Custom GraphQL type name (if different from model name).
    pub name: Option<String>,
    /// Description override for GraphQL.
    pub description: Option<String>,
    /// Whether to skip this type entirely in GraphQL.
    pub skip: bool,
    /// Generate as a GraphQL interface.
    pub is_interface: bool,
    /// Union types this model belongs to.
    pub union_types: Vec<String>,
    /// Implements interfaces.
    pub implements: Vec<String>,
    /// Directives to apply.
    pub directives: Vec<GraphQLDirective>,
    /// Query complexity for this type.
    pub complexity: Option<u32>,
    /// Guard/authorization expression.
    pub guard: Option<String>,
    /// Whether to generate input types.
    pub generate_input: bool,
    /// Whether to generate filter types.
    pub generate_filter: bool,
    /// Whether to generate ordering types.
    pub generate_order: bool,
    /// Custom resolver module.
    pub resolver: Option<String>,
}

impl GraphQLConfig {
    /// Create a new GraphQL config with defaults.
    pub fn new() -> Self {
        Self {
            generate_input: true,
            generate_filter: true,
            generate_order: true,
            ..Default::default()
        }
    }

    /// Set the custom GraphQL name.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Mark as skipped in GraphQL.
    pub fn skip(mut self) -> Self {
        self.skip = true;
        self
    }

    /// Mark as GraphQL interface.
    pub fn as_interface(mut self) -> Self {
        self.is_interface = true;
        self
    }

    /// Add to a union type.
    pub fn in_union(mut self, union_name: impl Into<String>) -> Self {
        self.union_types.push(union_name.into());
        self
    }

    /// Implement an interface.
    pub fn implements(mut self, interface: impl Into<String>) -> Self {
        self.implements.push(interface.into());
        self
    }

    /// Set complexity.
    pub fn with_complexity(mut self, complexity: u32) -> Self {
        self.complexity = Some(complexity);
        self
    }
}

/// GraphQL-specific configuration for a field.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GraphQLFieldConfig {
    /// Custom GraphQL field name.
    pub name: Option<String>,
    /// Description override.
    pub description: Option<String>,
    /// Whether to skip this field in GraphQL.
    pub skip: bool,
    /// Deprecation reason.
    pub deprecation: Option<String>,
    /// Query complexity for this field.
    pub complexity: Option<ComplexityConfig>,
    /// Guard/authorization expression.
    pub guard: Option<String>,
    /// Custom resolver function.
    pub resolver: Option<String>,
    /// Directives to apply.
    pub directives: Vec<GraphQLDirective>,
    /// Whether this field is derived/computed.
    pub derived: bool,
    /// Shareable across subgraphs (federation).
    pub shareable: bool,
    /// External field (federation).
    pub external: bool,
    /// Provides fields (federation).
    pub provides: Option<String>,
    /// Requires fields (federation).
    pub requires: Option<String>,
    /// Field is used as entity key (federation).
    pub key: bool,
}

impl GraphQLFieldConfig {
    /// Create new field config with defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set custom name.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Mark as skipped.
    pub fn skip(mut self) -> Self {
        self.skip = true;
        self
    }

    /// Set deprecation reason.
    pub fn deprecated(mut self, reason: impl Into<String>) -> Self {
        self.deprecation = Some(reason.into());
        self
    }

    /// Set as derived field.
    pub fn derived(mut self) -> Self {
        self.derived = true;
        self
    }
}

/// Query complexity configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ComplexityConfig {
    /// Fixed complexity value.
    Fixed(u32),
    /// Complexity based on arguments (multiplier).
    Multiplier {
        /// Base complexity.
        base: u32,
        /// Argument to multiply by.
        argument: String,
    },
    /// Custom complexity function.
    Custom(String),
}

impl Default for ComplexityConfig {
    fn default() -> Self {
        Self::Fixed(1)
    }
}

/// A GraphQL directive with arguments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphQLDirective {
    /// Directive name (without @).
    pub name: SmolStr,
    /// Directive arguments.
    pub arguments: Vec<GraphQLArgument>,
    /// Source location.
    pub span: Span,
}

impl GraphQLDirective {
    /// Create a new directive.
    pub fn new(name: impl Into<SmolStr>, span: Span) -> Self {
        Self {
            name: name.into(),
            arguments: Vec::new(),
            span,
        }
    }

    /// Add an argument.
    pub fn with_arg(mut self, name: impl Into<SmolStr>, value: GraphQLValue) -> Self {
        self.arguments.push(GraphQLArgument {
            name: name.into(),
            value,
        });
        self
    }

    /// Format as SDL directive.
    pub fn to_sdl(&self) -> String {
        if self.arguments.is_empty() {
            format!("@{}", self.name)
        } else {
            let args: Vec<String> = self
                .arguments
                .iter()
                .map(|a| format!("{}: {}", a.name, a.value.to_sdl()))
                .collect();
            format!("@{}({})", self.name, args.join(", "))
        }
    }
}

/// A GraphQL directive argument.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphQLArgument {
    /// Argument name.
    pub name: SmolStr,
    /// Argument value.
    pub value: GraphQLValue,
}

/// A GraphQL value for directive arguments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GraphQLValue {
    /// String value.
    String(String),
    /// Integer value.
    Int(i64),
    /// Float value.
    Float(f64),
    /// Boolean value.
    Boolean(bool),
    /// Enum value (unquoted).
    Enum(String),
    /// List of values.
    List(Vec<GraphQLValue>),
    /// Object value.
    Object(Vec<(String, GraphQLValue)>),
    /// Null value.
    Null,
}

impl GraphQLValue {
    /// Format as SDL value.
    pub fn to_sdl(&self) -> String {
        match self {
            Self::String(s) => format!("\"{}\"", s.replace('"', "\\\"")),
            Self::Int(i) => i.to_string(),
            Self::Float(f) => f.to_string(),
            Self::Boolean(b) => b.to_string(),
            Self::Enum(e) => e.clone(),
            Self::List(items) => {
                let vals: Vec<String> = items.iter().map(|v| v.to_sdl()).collect();
                format!("[{}]", vals.join(", "))
            }
            Self::Object(fields) => {
                let field_strs: Vec<String> = fields
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k, v.to_sdl()))
                    .collect();
                format!("{{{}}}", field_strs.join(", "))
            }
            Self::Null => "null".to_string(),
        }
    }
}

/// GraphQL subscription configuration.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SubscriptionConfig {
    /// Enable subscriptions for this model.
    pub enabled: bool,
    /// Subscribe to create events.
    pub on_create: bool,
    /// Subscribe to update events.
    pub on_update: bool,
    /// Subscribe to delete events.
    pub on_delete: bool,
    /// Custom subscription filter.
    pub filter: Option<String>,
}

impl SubscriptionConfig {
    /// Enable all subscription events.
    pub fn all() -> Self {
        Self {
            enabled: true,
            on_create: true,
            on_update: true,
            on_delete: true,
            filter: None,
        }
    }

    /// Enable only specific events.
    pub fn only_create() -> Self {
        Self {
            enabled: true,
            on_create: true,
            ..Default::default()
        }
    }
}

/// Federation 2.0 configuration for a model.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FederationConfig {
    /// This type is an entity (has @key).
    pub is_entity: bool,
    /// Key fields for entity resolution.
    pub keys: Vec<FederationKey>,
    /// This type is shareable.
    pub shareable: bool,
    /// This type is external (defined in another subgraph).
    pub external: bool,
    /// This type extends another type.
    pub extends: bool,
    /// Interface object (federation 2.3+).
    pub interface_object: bool,
}

/// Federation entity key configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FederationKey {
    /// Fields that make up the key.
    pub fields: String,
    /// Whether this key is resolvable.
    pub resolvable: bool,
}

impl FederationKey {
    /// Create a new federation key.
    pub fn new(fields: impl Into<String>) -> Self {
        Self {
            fields: fields.into(),
            resolvable: true,
        }
    }

    /// Mark as non-resolvable.
    pub fn non_resolvable(mut self) -> Self {
        self.resolvable = false;
        self
    }

    /// Format as SDL directive.
    pub fn to_sdl(&self) -> String {
        if self.resolvable {
            format!("@key(fields: \"{}\")", self.fields)
        } else {
            format!("@key(fields: \"{}\", resolvable: false)", self.fields)
        }
    }
}

/// Parse GraphQL configuration from doc tags.
pub fn parse_graphql_config_from_tags(tags: &[super::validation::DocTag]) -> GraphQLConfig {
    let mut config = GraphQLConfig::new();

    for tag in tags {
        match tag.name.as_str() {
            "graphql.skip" | "graphql_skip" => config.skip = true,
            "graphql.name" | "graphql_name" => config.name = tag.value.clone(),
            "graphql.interface" | "graphql_interface" => config.is_interface = true,
            "graphql.union" | "graphql_union" => {
                if let Some(v) = &tag.value {
                    config.union_types.push(v.clone());
                }
            }
            "graphql.implements" | "graphql_implements" => {
                if let Some(v) = &tag.value {
                    config.implements.push(v.clone());
                }
            }
            "graphql.complexity" | "graphql_complexity" => {
                if let Some(v) = &tag.value {
                    config.complexity = v.parse().ok();
                }
            }
            "graphql.guard" | "graphql_guard" => config.guard = tag.value.clone(),
            "graphql.resolver" | "graphql_resolver" => config.resolver = tag.value.clone(),
            "graphql.no_input" | "graphql_no_input" => config.generate_input = false,
            "graphql.no_filter" | "graphql_no_filter" => config.generate_filter = false,
            "graphql.no_order" | "graphql_no_order" => config.generate_order = false,
            _ => {}
        }
    }

    config
}

/// Parse field GraphQL configuration from doc tags.
pub fn parse_graphql_field_config_from_tags(
    tags: &[super::validation::DocTag],
) -> GraphQLFieldConfig {
    let mut config = GraphQLFieldConfig::new();

    for tag in tags {
        match tag.name.as_str() {
            "graphql.skip" | "graphql_skip" => config.skip = true,
            "graphql.name" | "graphql_name" => config.name = tag.value.clone(),
            "graphql.deprecation" | "graphql_deprecation" | "deprecated" => {
                config.deprecation = tag.value.clone().or(Some(String::new()));
            }
            "graphql.guard" | "graphql_guard" => config.guard = tag.value.clone(),
            "graphql.resolver" | "graphql_resolver" => config.resolver = tag.value.clone(),
            "graphql.derived" | "graphql_derived" => config.derived = true,
            "graphql.shareable" | "graphql_shareable" => config.shareable = true,
            "graphql.external" | "graphql_external" => config.external = true,
            "graphql.provides" | "graphql_provides" => config.provides = tag.value.clone(),
            "graphql.requires" | "graphql_requires" => config.requires = tag.value.clone(),
            "graphql.key" | "graphql_key" => config.key = true,
            _ => {}
        }
    }

    config
}

#[cfg(test)]
// `3.14`/`2.71` literal values are sample inputs to the SDL printer, not
// approximations of mathematical constants.
#[allow(clippy::approx_constant)]
mod tests {
    use super::*;

    #[test]
    fn test_graphql_config_builder() {
        let config = GraphQLConfig::new()
            .with_name("CustomUser")
            .as_interface()
            .in_union("SearchResult")
            .implements("Node")
            .with_complexity(10);

        assert_eq!(config.name, Some("CustomUser".to_string()));
        assert!(config.is_interface);
        assert_eq!(config.union_types, vec!["SearchResult"]);
        assert_eq!(config.implements, vec!["Node"]);
        assert_eq!(config.complexity, Some(10));
    }

    #[test]
    fn test_graphql_field_config() {
        let config = GraphQLFieldConfig::new()
            .with_name("userId")
            .deprecated("Use newId instead")
            .derived();

        assert_eq!(config.name, Some("userId".to_string()));
        assert_eq!(config.deprecation, Some("Use newId instead".to_string()));
        assert!(config.derived);
    }

    #[test]
    fn test_graphql_directive_sdl() {
        let directive = GraphQLDirective::new("deprecated", Span::new(0, 0))
            .with_arg("reason", GraphQLValue::String("Use newField".to_string()));

        assert_eq!(directive.to_sdl(), "@deprecated(reason: \"Use newField\")");

        let simple_directive = GraphQLDirective::new("shareable", Span::new(0, 0));
        assert_eq!(simple_directive.to_sdl(), "@shareable");
    }

    #[test]
    fn test_graphql_value_sdl() {
        assert_eq!(
            GraphQLValue::String("hello".to_string()).to_sdl(),
            "\"hello\""
        );
        assert_eq!(GraphQLValue::Int(42).to_sdl(), "42");
        assert_eq!(GraphQLValue::Float(3.14).to_sdl(), "3.14");
        assert_eq!(GraphQLValue::Boolean(true).to_sdl(), "true");
        assert_eq!(GraphQLValue::Enum("ADMIN".to_string()).to_sdl(), "ADMIN");
        assert_eq!(GraphQLValue::Null.to_sdl(), "null");

        let list = GraphQLValue::List(vec![
            GraphQLValue::Int(1),
            GraphQLValue::Int(2),
            GraphQLValue::Int(3),
        ]);
        assert_eq!(list.to_sdl(), "[1, 2, 3]");

        let obj = GraphQLValue::Object(vec![
            ("name".to_string(), GraphQLValue::String("test".to_string())),
            ("count".to_string(), GraphQLValue::Int(5)),
        ]);
        assert_eq!(obj.to_sdl(), "{name: \"test\", count: 5}");
    }

    #[test]
    fn test_federation_key_sdl() {
        let key = FederationKey::new("id");
        assert_eq!(key.to_sdl(), "@key(fields: \"id\")");

        let composite_key = FederationKey::new("userId organizationId");
        assert_eq!(
            composite_key.to_sdl(),
            "@key(fields: \"userId organizationId\")"
        );

        let non_resolvable = FederationKey::new("id").non_resolvable();
        assert_eq!(
            non_resolvable.to_sdl(),
            "@key(fields: \"id\", resolvable: false)"
        );
    }

    #[test]
    fn test_subscription_config() {
        let all = SubscriptionConfig::all();
        assert!(all.enabled);
        assert!(all.on_create);
        assert!(all.on_update);
        assert!(all.on_delete);

        let only_create = SubscriptionConfig::only_create();
        assert!(only_create.enabled);
        assert!(only_create.on_create);
        assert!(!only_create.on_update);
        assert!(!only_create.on_delete);
    }

    #[test]
    fn test_parse_graphql_config_from_tags() {
        use super::super::validation::DocTag;

        let span = Span::new(0, 0);
        let tags = vec![
            DocTag::new("graphql.name", Some("CustomModel".to_string()), span),
            DocTag::new("graphql.interface", None, span),
            DocTag::new("graphql.complexity", Some("5".to_string()), span),
        ];

        let config = parse_graphql_config_from_tags(&tags);

        assert_eq!(config.name, Some("CustomModel".to_string()));
        assert!(config.is_interface);
        assert_eq!(config.complexity, Some(5));
    }

    #[test]
    fn test_parse_graphql_field_config_from_tags() {
        use super::super::validation::DocTag;

        let span = Span::new(0, 0);
        let tags = vec![
            DocTag::new("graphql.name", Some("userId".to_string()), span),
            DocTag::new("deprecated", Some("Use newId".to_string()), span),
            DocTag::new("graphql.shareable", None, span),
        ];

        let config = parse_graphql_field_config_from_tags(&tags);

        assert_eq!(config.name, Some("userId".to_string()));
        assert_eq!(config.deprecation, Some("Use newId".to_string()));
        assert!(config.shareable);
    }
}
