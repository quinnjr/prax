//! Datasource and PostgreSQL extension definitions.

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use super::Span;

/// Database provider type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DatabaseProvider {
    /// PostgreSQL database.
    PostgreSQL,
    /// MySQL database.
    MySQL,
    /// SQLite database.
    SQLite,
    /// MongoDB database.
    MongoDB,
}

impl DatabaseProvider {
    /// Parse a provider from a string.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "postgresql" | "postgres" => Some(Self::PostgreSQL),
            "mysql" => Some(Self::MySQL),
            "sqlite" => Some(Self::SQLite),
            "mongodb" => Some(Self::MongoDB),
            _ => None,
        }
    }

    /// Get the provider as a string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PostgreSQL => "postgresql",
            Self::MySQL => "mysql",
            Self::SQLite => "sqlite",
            Self::MongoDB => "mongodb",
        }
    }

    /// Check if this provider supports extensions.
    pub fn supports_extensions(&self) -> bool {
        matches!(self, Self::PostgreSQL)
    }
}

impl std::fmt::Display for DatabaseProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A PostgreSQL extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostgresExtension {
    /// Extension name (e.g., "pg_trgm", "vector", "uuid-ossp").
    pub name: SmolStr,
    /// Optional schema to install the extension into.
    pub schema: Option<SmolStr>,
    /// Optional version constraint.
    pub version: Option<SmolStr>,
    /// Source span for error reporting.
    pub span: Span,
}

impl PostgresExtension {
    /// Create a new extension.
    pub fn new(name: impl Into<SmolStr>, span: Span) -> Self {
        Self {
            name: name.into(),
            schema: None,
            version: None,
            span,
        }
    }

    /// Set the schema for this extension.
    pub fn with_schema(mut self, schema: impl Into<SmolStr>) -> Self {
        self.schema = Some(schema.into());
        self
    }

    /// Set the version for this extension.
    pub fn with_version(mut self, version: impl Into<SmolStr>) -> Self {
        self.version = Some(version.into());
        self
    }

    /// Get the extension name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Generate the CREATE EXTENSION SQL.
    pub fn to_create_sql(&self) -> String {
        let mut sql = format!("CREATE EXTENSION IF NOT EXISTS \"{}\"", self.name);
        if let Some(schema) = &self.schema {
            sql.push_str(&format!(" SCHEMA \"{}\"", schema));
        }
        if let Some(version) = &self.version {
            sql.push_str(&format!(" VERSION '{}'", version));
        }
        sql.push(';');
        sql
    }

    /// Generate the DROP EXTENSION SQL.
    pub fn to_drop_sql(&self) -> String {
        format!("DROP EXTENSION IF EXISTS \"{}\" CASCADE;", self.name)
    }

    /// Check if this is a known extension that provides custom types.
    pub fn provides_custom_types(&self) -> bool {
        matches!(
            self.name.as_str(),
            "vector" | "pgvector" | "postgis" | "hstore" | "ltree" | "cube" | "citext"
        )
    }
}

impl std::fmt::Display for PostgresExtension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

/// Well-known PostgreSQL extensions with their capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WellKnownExtension {
    /// pg_trgm - Trigram text similarity search.
    PgTrgm,
    /// vector/pgvector - Vector similarity search for AI/ML embeddings.
    Vector,
    /// uuid-ossp - UUID generation functions.
    UuidOssp,
    /// pgcrypto - Cryptographic functions.
    PgCrypto,
    /// postgis - Geographic objects and spatial queries.
    PostGIS,
    /// hstore - Key-value store.
    HStore,
    /// ltree - Hierarchical tree-like data.
    LTree,
    /// citext - Case-insensitive text.
    Citext,
    /// cube - Multi-dimensional cubes.
    Cube,
    /// pg_stat_statements - Query statistics.
    PgStatStatements,
    /// aws_lambda - AWS Lambda integration.
    AwsLambda,
    /// aws_s3 - AWS S3 integration.
    AwsS3,
    /// plpgsql - PL/pgSQL procedural language.
    PlPgSQL,
}

impl WellKnownExtension {
    /// Get the extension name as used in CREATE EXTENSION.
    pub fn extension_name(&self) -> &'static str {
        match self {
            Self::PgTrgm => "pg_trgm",
            Self::Vector => "vector",
            Self::UuidOssp => "uuid-ossp",
            Self::PgCrypto => "pgcrypto",
            Self::PostGIS => "postgis",
            Self::HStore => "hstore",
            Self::LTree => "ltree",
            Self::Citext => "citext",
            Self::Cube => "cube",
            Self::PgStatStatements => "pg_stat_statements",
            Self::AwsLambda => "aws_lambda",
            Self::AwsS3 => "aws_s3",
            Self::PlPgSQL => "plpgsql",
        }
    }

    /// Parse a well-known extension from a string.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "pg_trgm" => Some(Self::PgTrgm),
            "vector" | "pgvector" => Some(Self::Vector),
            "uuid-ossp" | "uuid_ossp" => Some(Self::UuidOssp),
            "pgcrypto" => Some(Self::PgCrypto),
            "postgis" => Some(Self::PostGIS),
            "hstore" => Some(Self::HStore),
            "ltree" => Some(Self::LTree),
            "citext" => Some(Self::Citext),
            "cube" => Some(Self::Cube),
            "pg_stat_statements" => Some(Self::PgStatStatements),
            "aws_lambda" => Some(Self::AwsLambda),
            "aws_s3" => Some(Self::AwsS3),
            "plpgsql" => Some(Self::PlPgSQL),
            _ => None,
        }
    }

    /// Get a description of what this extension provides.
    pub fn description(&self) -> &'static str {
        match self {
            Self::PgTrgm => "Trigram-based text similarity search",
            Self::Vector => "Vector similarity search for AI/ML embeddings",
            Self::UuidOssp => "UUID generation functions",
            Self::PgCrypto => "Cryptographic functions",
            Self::PostGIS => "Geographic objects and spatial queries",
            Self::HStore => "Key-value store type",
            Self::LTree => "Hierarchical tree-like data",
            Self::Citext => "Case-insensitive text type",
            Self::Cube => "Multi-dimensional cube data type",
            Self::PgStatStatements => "Query execution statistics",
            Self::AwsLambda => "AWS Lambda function invocation",
            Self::AwsS3 => "AWS S3 storage integration",
            Self::PlPgSQL => "PL/pgSQL procedural language",
        }
    }
}

/// Datasource configuration block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Datasource {
    /// Datasource name (usually "db").
    pub name: SmolStr,
    /// Database provider.
    pub provider: DatabaseProvider,
    /// Connection URL (can be an env var reference).
    pub url: Option<SmolStr>,
    /// Environment variable name for the URL.
    pub url_env: Option<SmolStr>,
    /// PostgreSQL extensions to enable.
    pub extensions: Vec<PostgresExtension>,
    /// Additional provider-specific properties.
    pub properties: Vec<(SmolStr, SmolStr)>,
    /// Source span for error reporting.
    pub span: Span,
    /// Source file this datasource was parsed from (None for single-file path).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<crate::loader::SourceId>,
}

impl Datasource {
    /// Create a new datasource.
    pub fn new(name: impl Into<SmolStr>, provider: DatabaseProvider, span: Span) -> Self {
        Self {
            name: name.into(),
            provider,
            url: None,
            url_env: None,
            extensions: Vec::new(),
            properties: Vec::new(),
            span,
            source_id: None,
        }
    }

    /// Set the URL.
    pub fn with_url(mut self, url: impl Into<SmolStr>) -> Self {
        self.url = Some(url.into());
        self
    }

    /// Set the URL from an environment variable.
    pub fn with_url_env(mut self, env_var: impl Into<SmolStr>) -> Self {
        self.url_env = Some(env_var.into());
        self
    }

    /// Add an extension.
    pub fn add_extension(&mut self, ext: PostgresExtension) {
        self.extensions.push(ext);
    }

    /// Add a property.
    pub fn add_property(&mut self, key: impl Into<SmolStr>, value: impl Into<SmolStr>) {
        self.properties.push((key.into(), value.into()));
    }

    /// Check if this datasource has a specific extension.
    pub fn has_extension(&self, name: &str) -> bool {
        self.extensions.iter().any(|e| e.name == name)
    }

    /// Get extension by name.
    pub fn get_extension(&self, name: &str) -> Option<&PostgresExtension> {
        self.extensions.iter().find(|e| e.name == name)
    }

    /// Check if vector extension is enabled.
    pub fn has_vector_support(&self) -> bool {
        self.has_extension("vector") || self.has_extension("pgvector")
    }

    /// Generate SQL to create all extensions.
    pub fn extensions_create_sql(&self) -> Vec<String> {
        self.extensions.iter().map(|e| e.to_create_sql()).collect()
    }
}

impl Default for Datasource {
    fn default() -> Self {
        Self {
            name: SmolStr::new("db"),
            provider: DatabaseProvider::PostgreSQL,
            url: None,
            url_env: None,
            extensions: Vec::new(),
            properties: Vec::new(),
            span: Span::new(0, 0),
            source_id: None,
        }
    }
}

impl std::fmt::Display for Datasource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "datasource {} {{ provider = {} }}",
            self.name, self.provider
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_span() -> Span {
        Span::new(0, 10)
    }

    // ==================== DatabaseProvider Tests ====================

    #[test]
    fn test_database_provider_from_str() {
        assert_eq!(
            DatabaseProvider::from_str("postgresql"),
            Some(DatabaseProvider::PostgreSQL)
        );
        assert_eq!(
            DatabaseProvider::from_str("postgres"),
            Some(DatabaseProvider::PostgreSQL)
        );
        assert_eq!(
            DatabaseProvider::from_str("PostgreSQL"),
            Some(DatabaseProvider::PostgreSQL)
        );
        assert_eq!(
            DatabaseProvider::from_str("mysql"),
            Some(DatabaseProvider::MySQL)
        );
        assert_eq!(
            DatabaseProvider::from_str("sqlite"),
            Some(DatabaseProvider::SQLite)
        );
        assert_eq!(
            DatabaseProvider::from_str("mongodb"),
            Some(DatabaseProvider::MongoDB)
        );
        assert_eq!(DatabaseProvider::from_str("unknown"), None);
    }

    #[test]
    fn test_database_provider_as_str() {
        assert_eq!(DatabaseProvider::PostgreSQL.as_str(), "postgresql");
        assert_eq!(DatabaseProvider::MySQL.as_str(), "mysql");
        assert_eq!(DatabaseProvider::SQLite.as_str(), "sqlite");
        assert_eq!(DatabaseProvider::MongoDB.as_str(), "mongodb");
    }

    #[test]
    fn test_database_provider_supports_extensions() {
        assert!(DatabaseProvider::PostgreSQL.supports_extensions());
        assert!(!DatabaseProvider::MySQL.supports_extensions());
        assert!(!DatabaseProvider::SQLite.supports_extensions());
        assert!(!DatabaseProvider::MongoDB.supports_extensions());
    }

    // ==================== PostgresExtension Tests ====================

    #[test]
    fn test_postgres_extension_new() {
        let ext = PostgresExtension::new("vector", make_span());
        assert_eq!(ext.name(), "vector");
        assert!(ext.schema.is_none());
        assert!(ext.version.is_none());
    }

    #[test]
    fn test_postgres_extension_with_schema() {
        let ext = PostgresExtension::new("postgis", make_span()).with_schema("public");
        assert_eq!(ext.schema, Some(SmolStr::new("public")));
    }

    #[test]
    fn test_postgres_extension_with_version() {
        let ext = PostgresExtension::new("vector", make_span()).with_version("0.5.0");
        assert_eq!(ext.version, Some(SmolStr::new("0.5.0")));
    }

    #[test]
    fn test_postgres_extension_to_create_sql() {
        let ext = PostgresExtension::new("pg_trgm", make_span());
        assert_eq!(
            ext.to_create_sql(),
            "CREATE EXTENSION IF NOT EXISTS \"pg_trgm\";"
        );

        let ext_with_schema =
            PostgresExtension::new("postgis", make_span()).with_schema("extensions");
        assert_eq!(
            ext_with_schema.to_create_sql(),
            "CREATE EXTENSION IF NOT EXISTS \"postgis\" SCHEMA \"extensions\";"
        );

        let ext_with_version = PostgresExtension::new("vector", make_span()).with_version("0.5.0");
        assert_eq!(
            ext_with_version.to_create_sql(),
            "CREATE EXTENSION IF NOT EXISTS \"vector\" VERSION '0.5.0';"
        );
    }

    #[test]
    fn test_postgres_extension_to_drop_sql() {
        let ext = PostgresExtension::new("vector", make_span());
        assert_eq!(
            ext.to_drop_sql(),
            "DROP EXTENSION IF EXISTS \"vector\" CASCADE;"
        );
    }

    #[test]
    fn test_postgres_extension_provides_custom_types() {
        assert!(PostgresExtension::new("vector", make_span()).provides_custom_types());
        assert!(PostgresExtension::new("postgis", make_span()).provides_custom_types());
        assert!(PostgresExtension::new("hstore", make_span()).provides_custom_types());
        assert!(!PostgresExtension::new("pg_trgm", make_span()).provides_custom_types());
    }

    // ==================== WellKnownExtension Tests ====================

    #[test]
    fn test_well_known_extension_from_str() {
        assert_eq!(
            WellKnownExtension::from_str("vector"),
            Some(WellKnownExtension::Vector)
        );
        assert_eq!(
            WellKnownExtension::from_str("pgvector"),
            Some(WellKnownExtension::Vector)
        );
        assert_eq!(
            WellKnownExtension::from_str("pg_trgm"),
            Some(WellKnownExtension::PgTrgm)
        );
        assert_eq!(
            WellKnownExtension::from_str("uuid-ossp"),
            Some(WellKnownExtension::UuidOssp)
        );
        assert_eq!(WellKnownExtension::from_str("unknown"), None);
    }

    #[test]
    fn test_well_known_extension_name() {
        assert_eq!(WellKnownExtension::Vector.extension_name(), "vector");
        assert_eq!(WellKnownExtension::PgTrgm.extension_name(), "pg_trgm");
        assert_eq!(WellKnownExtension::UuidOssp.extension_name(), "uuid-ossp");
    }

    // ==================== Datasource Tests ====================

    #[test]
    fn test_datasource_new() {
        let ds = Datasource::new("db", DatabaseProvider::PostgreSQL, make_span());
        assert_eq!(ds.name.as_str(), "db");
        assert_eq!(ds.provider, DatabaseProvider::PostgreSQL);
        assert!(ds.extensions.is_empty());
    }

    #[test]
    fn test_datasource_with_url() {
        let ds = Datasource::new("db", DatabaseProvider::PostgreSQL, make_span())
            .with_url("postgresql://localhost/mydb");
        assert_eq!(ds.url, Some(SmolStr::new("postgresql://localhost/mydb")));
    }

    #[test]
    fn test_datasource_with_url_env() {
        let ds = Datasource::new("db", DatabaseProvider::PostgreSQL, make_span())
            .with_url_env("DATABASE_URL");
        assert_eq!(ds.url_env, Some(SmolStr::new("DATABASE_URL")));
    }

    #[test]
    fn test_datasource_add_extension() {
        let mut ds = Datasource::new("db", DatabaseProvider::PostgreSQL, make_span());
        ds.add_extension(PostgresExtension::new("vector", make_span()));
        ds.add_extension(PostgresExtension::new("pg_trgm", make_span()));

        assert_eq!(ds.extensions.len(), 2);
        assert!(ds.has_extension("vector"));
        assert!(ds.has_extension("pg_trgm"));
        assert!(!ds.has_extension("postgis"));
    }

    #[test]
    fn test_datasource_has_vector_support() {
        let mut ds = Datasource::new("db", DatabaseProvider::PostgreSQL, make_span());
        assert!(!ds.has_vector_support());

        ds.add_extension(PostgresExtension::new("vector", make_span()));
        assert!(ds.has_vector_support());
    }

    #[test]
    fn test_datasource_extensions_create_sql() {
        let mut ds = Datasource::new("db", DatabaseProvider::PostgreSQL, make_span());
        ds.add_extension(PostgresExtension::new("vector", make_span()));
        ds.add_extension(PostgresExtension::new("pg_trgm", make_span()));

        let sqls = ds.extensions_create_sql();
        assert_eq!(sqls.len(), 2);
        assert!(sqls[0].contains("vector"));
        assert!(sqls[1].contains("pg_trgm"));
    }

    #[test]
    fn test_datasource_default() {
        let ds = Datasource::default();
        assert_eq!(ds.name.as_str(), "db");
        assert_eq!(ds.provider, DatabaseProvider::PostgreSQL);
    }
}
