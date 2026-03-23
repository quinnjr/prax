//! Generator definitions for the Prax schema AST.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use super::Span;

/// A generator block in the schema.
///
/// Generators define code generation targets that are invoked by `prax generate`.
///
/// ```prax
/// generator typescript {
///   provider = "prax-typegen"
///   output   = "./src/types"
///   generate = env("TYPESCRIPT_GENERATE")
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Generator {
    /// Generator name (the identifier after `generator`).
    pub name: SmolStr,
    /// The provider binary or crate name.
    pub provider: Option<SmolStr>,
    /// Output directory for generated files.
    pub output: Option<SmolStr>,
    /// Whether generation is enabled. Resolves `env()` calls at runtime.
    pub generate: GeneratorToggle,
    /// Additional key-value properties.
    pub properties: IndexMap<SmolStr, GeneratorValue>,
    /// Source location.
    pub span: Span,
}

impl Generator {
    pub fn new(name: impl Into<SmolStr>, span: Span) -> Self {
        Self {
            name: name.into(),
            provider: None,
            output: None,
            generate: GeneratorToggle::Always,
            properties: IndexMap::new(),
            span,
        }
    }

    /// Check whether this generator should run, resolving env vars.
    pub fn is_enabled(&self) -> bool {
        match &self.generate {
            GeneratorToggle::Always => true,
            GeneratorToggle::Never => false,
            GeneratorToggle::Literal(val) => *val,
            GeneratorToggle::Env(var_name) => std::env::var(var_name)
                .map(|v| {
                    let v = v.trim().to_lowercase();
                    v == "true" || v == "1" || v == "yes"
                })
                .unwrap_or(false),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Controls whether a generator runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GeneratorToggle {
    /// Always run (no `generate` property specified).
    Always,
    /// Never run.
    Never,
    /// Literal boolean value: `generate = true` or `generate = false`.
    Literal(bool),
    /// Environment variable: `generate = env("TYPESCRIPT_GENERATE")`.
    Env(SmolStr),
}

/// A value in a generator property.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GeneratorValue {
    /// A string literal.
    String(SmolStr),
    /// A boolean literal.
    Bool(bool),
    /// An environment variable reference.
    Env(SmolStr),
    /// An identifier (unquoted value).
    Ident(SmolStr),
}

impl GeneratorValue {
    /// Resolve this value to a string, reading env vars as needed.
    pub fn resolve(&self) -> Option<String> {
        match self {
            Self::String(s) => Some(s.to_string()),
            Self::Bool(b) => Some(b.to_string()),
            Self::Ident(s) => Some(s.to_string()),
            Self::Env(var) => std::env::var(var.as_str()).ok(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span() -> Span {
        Span::new(0, 10)
    }

    #[test]
    fn test_generator_new() {
        let g = Generator::new("typescript", span());
        assert_eq!(g.name(), "typescript");
        assert!(g.provider.is_none());
        assert!(g.output.is_none());
        assert!(g.is_enabled());
    }

    #[test]
    fn test_generator_toggle_always() {
        let g = Generator::new("test", span());
        assert!(g.is_enabled());
    }

    #[test]
    fn test_generator_toggle_literal_true() {
        let mut g = Generator::new("test", span());
        g.generate = GeneratorToggle::Literal(true);
        assert!(g.is_enabled());
    }

    #[test]
    fn test_generator_toggle_literal_false() {
        let mut g = Generator::new("test", span());
        g.generate = GeneratorToggle::Literal(false);
        assert!(!g.is_enabled());
    }

    #[test]
    fn test_generator_toggle_never() {
        let mut g = Generator::new("test", span());
        g.generate = GeneratorToggle::Never;
        assert!(!g.is_enabled());
    }

    #[test]
    fn test_generator_toggle_env_true() {
        unsafe { std::env::set_var("PRAX_TEST_GEN_TOGGLE", "true") };
        let mut g = Generator::new("test", span());
        g.generate = GeneratorToggle::Env("PRAX_TEST_GEN_TOGGLE".into());
        assert!(g.is_enabled());
        unsafe { std::env::remove_var("PRAX_TEST_GEN_TOGGLE") };
    }

    #[test]
    fn test_generator_toggle_env_false() {
        unsafe { std::env::set_var("PRAX_TEST_GEN_TOGGLE_F", "false") };
        let mut g = Generator::new("test", span());
        g.generate = GeneratorToggle::Env("PRAX_TEST_GEN_TOGGLE_F".into());
        assert!(!g.is_enabled());
        unsafe { std::env::remove_var("PRAX_TEST_GEN_TOGGLE_F") };
    }

    #[test]
    fn test_generator_toggle_env_missing() {
        let mut g = Generator::new("test", span());
        g.generate = GeneratorToggle::Env("PRAX_TEST_NONEXISTENT_VAR_999".into());
        assert!(!g.is_enabled());
    }

    #[test]
    fn test_generator_toggle_env_one() {
        unsafe { std::env::set_var("PRAX_TEST_GEN_ONE", "1") };
        let mut g = Generator::new("test", span());
        g.generate = GeneratorToggle::Env("PRAX_TEST_GEN_ONE".into());
        assert!(g.is_enabled());
        unsafe { std::env::remove_var("PRAX_TEST_GEN_ONE") };
    }

    #[test]
    fn test_generator_value_resolve_string() {
        let v = GeneratorValue::String("hello".into());
        assert_eq!(v.resolve(), Some("hello".to_string()));
    }

    #[test]
    fn test_generator_value_resolve_bool() {
        let v = GeneratorValue::Bool(true);
        assert_eq!(v.resolve(), Some("true".to_string()));
    }

    #[test]
    fn test_generator_value_resolve_env() {
        unsafe { std::env::set_var("PRAX_TEST_VAL", "resolved") };
        let v = GeneratorValue::Env("PRAX_TEST_VAL".into());
        assert_eq!(v.resolve(), Some("resolved".to_string()));
        unsafe { std::env::remove_var("PRAX_TEST_VAL") };
    }

    #[test]
    fn test_generator_value_resolve_env_missing() {
        let v = GeneratorValue::Env("PRAX_TEST_MISSING_VAL_999".into());
        assert_eq!(v.resolve(), None);
    }

    #[test]
    fn test_parse_generator_block() {
        use crate::parse_schema;

        let schema = parse_schema(
            r#"
            generator typescript {
                provider = "prax-typegen"
                output   = "./src/types"
            }
            "#,
        )
        .unwrap();

        assert_eq!(schema.generators.len(), 1);
        let g = schema.get_generator("typescript").unwrap();
        assert_eq!(g.provider.as_deref(), Some("prax-typegen"));
        assert_eq!(g.output.as_deref(), Some("./src/types"));
        assert!(g.is_enabled());
    }

    #[test]
    fn test_parse_generator_with_env_toggle() {
        use crate::parse_schema;

        let schema = parse_schema(
            r#"
            generator typescript {
                provider = "prax-typegen"
                output   = "./src/types"
                generate = env("PRAX_TEST_PARSE_GEN_TOGGLE")
            }
            "#,
        )
        .unwrap();

        let g = schema.get_generator("typescript").unwrap();
        assert_eq!(
            g.generate,
            GeneratorToggle::Env("PRAX_TEST_PARSE_GEN_TOGGLE".into())
        );

        assert!(!g.is_enabled());

        unsafe { std::env::set_var("PRAX_TEST_PARSE_GEN_TOGGLE", "true") };
        assert!(g.is_enabled());
        unsafe { std::env::remove_var("PRAX_TEST_PARSE_GEN_TOGGLE") };
    }

    #[test]
    fn test_parse_generator_with_bool_toggle() {
        use crate::parse_schema;

        let schema = parse_schema(
            r#"
            generator disabled {
                provider = "some-provider"
                generate = false
            }
            "#,
        )
        .unwrap();

        let g = schema.get_generator("disabled").unwrap();
        assert_eq!(g.generate, GeneratorToggle::Literal(false));
        assert!(!g.is_enabled());
    }

    #[test]
    fn test_parse_multiple_generators() {
        use crate::parse_schema;

        let schema = parse_schema(
            r#"
            generator typescript {
                provider = "prax-typegen"
                output   = "./ts"
            }

            generator python {
                provider = "prax-pygen"
                output   = "./py"
                generate = env("PYTHON_GENERATE")
            }
            "#,
        )
        .unwrap();

        assert_eq!(schema.generators.len(), 2);
        assert!(schema.get_generator("typescript").is_some());
        assert!(schema.get_generator("python").is_some());
    }

    #[test]
    fn test_enabled_generators_filters() {
        use crate::parse_schema;

        let schema = parse_schema(
            r#"
            generator enabled_one {
                provider = "a"
                generate = true
            }

            generator disabled_one {
                provider = "b"
                generate = false
            }
            "#,
        )
        .unwrap();

        let enabled = schema.enabled_generators();
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].name(), "enabled_one");
    }

    #[test]
    fn test_parse_generator_extra_properties() {
        use crate::parse_schema;

        let schema = parse_schema(
            r#"
            generator typescript {
                provider    = "prax-typegen"
                output      = "./src/types"
                emitZod     = true
                packageName = "@myapp/types"
            }
            "#,
        )
        .unwrap();

        let g = schema.get_generator("typescript").unwrap();
        assert_eq!(
            g.properties.get("emitZod"),
            Some(&GeneratorValue::Bool(true))
        );
        assert_eq!(
            g.properties.get("packageName"),
            Some(&GeneratorValue::String("@myapp/types".into()))
        );
    }
}
