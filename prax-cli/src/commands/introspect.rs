//! Database introspection implementation.
//!
//! This module provides the actual database introspection functionality
//! using the `prax-query` introspection types.

use prax_query::introspection::{
    ColumnInfo, DatabaseSchema, ForeignKeyInfo, IndexColumn, IndexInfo, ReferentialAction,
    SortOrder, TableInfo, generate_prax_schema, normalize_type, queries,
};
// `EnumInfo`/`ViewInfo` are only constructed by the PostgreSQL introspector
// (the other backends build enums/views differently or not at all), so import
// them only when that feature is compiled to keep single-feature builds clean.
#[cfg(feature = "postgres")]
use prax_query::introspection::{EnumInfo, ViewInfo};
use prax_query::sql::DatabaseType;

use crate::config::Config;
use crate::error::{CliError, CliResult};

/// Introspection options.
#[derive(Debug, Clone)]
pub struct IntrospectionOptions {
    /// Schema/namespace to introspect.
    pub schema: Option<String>,
    /// Include views.
    pub include_views: bool,
    /// Include materialized views.
    pub include_materialized_views: bool,
    /// Table filter pattern.
    pub table_filter: Option<String>,
    /// Tables to exclude.
    pub exclude_pattern: Option<String>,
    /// Include comments.
    pub include_comments: bool,
    /// Sample size for MongoDB.
    pub sample_size: usize,
}

impl Default for IntrospectionOptions {
    fn default() -> Self {
        Self {
            schema: None,
            include_views: false,
            include_materialized_views: false,
            table_filter: None,
            exclude_pattern: None,
            include_comments: true,
            sample_size: 100,
        }
    }
}

/// Database introspector trait.
#[allow(async_fn_in_trait)]
pub trait Introspector {
    /// Introspect the database and return schema information.
    async fn introspect(&self, options: &IntrospectionOptions) -> CliResult<DatabaseSchema>;
}

/// Get the database type from provider string.
pub fn get_database_type(provider: &str) -> CliResult<DatabaseType> {
    match provider.to_lowercase().as_str() {
        "postgresql" | "postgres" | "pg" => Ok(DatabaseType::PostgreSQL),
        "mysql" | "mariadb" => Ok(DatabaseType::MySQL),
        "sqlite" | "sqlite3" => Ok(DatabaseType::SQLite),
        "mssql" | "sqlserver" | "sql_server" => Ok(DatabaseType::MSSQL),
        _ => Err(CliError::Config(format!(
            "Unsupported database provider: {}",
            provider
        ))),
    }
}

/// Get default schema for database type.
pub fn default_schema(db_type: DatabaseType) -> &'static str {
    match db_type {
        DatabaseType::PostgreSQL => "public",
        DatabaseType::MySQL => "",
        DatabaseType::SQLite => "",
        DatabaseType::MSSQL => "dbo",
    }
}

/// Introspect a database, dispatching to the backend matching `provider`.
///
/// This is the single entry point shared by `db pull` and the migration
/// engine's `resolve_source_schema`. Each backend is behind its cargo
/// feature; a provider whose feature was not compiled in returns a clear
/// `Config` error rather than silently doing nothing.
pub async fn introspect_database(
    provider: &str,
    database_url: &str,
    options: &IntrospectionOptions,
) -> CliResult<DatabaseSchema> {
    let db_type = get_database_type(provider)?;
    match db_type {
        DatabaseType::PostgreSQL => {
            #[cfg(feature = "postgres")]
            {
                postgres::PostgresIntrospector::new(database_url.to_string())
                    .introspect(options)
                    .await
            }
            #[cfg(not(feature = "postgres"))]
            {
                let _ = (database_url, options);
                Err(CliError::FeatureUnavailable(
                    "PostgreSQL introspection requires the `postgres` feature: rebuild with \
                     --features postgres."
                        .to_string(),
                ))
            }
        }
        DatabaseType::MySQL => {
            #[cfg(feature = "mysql")]
            {
                mysql::MysqlIntrospector::new(database_url.to_string())
                    .introspect(options)
                    .await
            }
            #[cfg(not(feature = "mysql"))]
            {
                let _ = (database_url, options);
                Err(CliError::FeatureUnavailable(
                    "MySQL introspection requires the `mysql` feature: rebuild with \
                     --features mysql."
                        .to_string(),
                ))
            }
        }
        DatabaseType::SQLite => {
            #[cfg(feature = "sqlite")]
            {
                sqlite::SqliteIntrospector::new(database_url.to_string())
                    .introspect(options)
                    .await
            }
            #[cfg(not(feature = "sqlite"))]
            {
                let _ = (database_url, options);
                Err(CliError::FeatureUnavailable(
                    "SQLite introspection requires the `sqlite` feature: rebuild with \
                     --features sqlite."
                        .to_string(),
                ))
            }
        }
        DatabaseType::MSSQL => {
            #[cfg(feature = "mssql")]
            {
                mssql::MssqlIntrospector::new(database_url.to_string())
                    .introspect(options)
                    .await
            }
            #[cfg(not(feature = "mssql"))]
            {
                let _ = (database_url, options);
                Err(CliError::FeatureUnavailable(
                    "MSSQL introspection requires the `mssql` feature: rebuild with \
                     --features mssql."
                        .to_string(),
                ))
            }
        }
    }
}

// ============================================================================
// PostgreSQL Introspector
// ============================================================================

#[cfg(feature = "postgres")]
pub mod postgres {
    use std::collections::HashMap;

    use super::*;
    use tokio_postgres::{Client, NoTls, Row};

    /// PostgreSQL introspector.
    pub struct PostgresIntrospector {
        connection_string: String,
    }

    impl PostgresIntrospector {
        /// Create a new PostgreSQL introspector.
        pub fn new(connection_string: String) -> Self {
            Self { connection_string }
        }

        /// Connect to the database.
        async fn connect(&self) -> CliResult<Client> {
            // Parse the DSN the same way tokio-postgres will, so the sslmode
            // it carries is honored: anything but `disable` goes through the
            // workspace's shared rustls connector (chain + hostname verified
            // against the Mozilla root store). `prefer` still falls back to
            // plaintext when the server declines TLS.
            let config = self
                .connection_string
                .parse::<tokio_postgres::Config>()
                .map_err(|e| CliError::Config(format!("Invalid connection string: {}", e)))?;

            let tls_disabled = matches!(
                config.get_ssl_mode(),
                tokio_postgres::config::SslMode::Disable
            );

            if tls_disabled && !config.get_hosts().iter().all(is_local_host) {
                crate::output::warn(
                    "sslmode=disable with a non-local host: credentials and data will be \
                     sent in plaintext.",
                );
            }

            // The two connector types produce different `Connection`
            // generics, so drive each arm independently and unify on the
            // stream-agnostic `Client`. Each connect is bounded by
            // `INTROSPECT_CONNECT_TIMEOUT_SECS` so an unreachable-but-not-
            // refused host does not hang the CLI (parity with the pool-based
            // backends).
            let connect_timeout =
                std::time::Duration::from_secs(super::INTROSPECT_CONNECT_TIMEOUT_SECS);
            let client = if tls_disabled {
                let (client, connection) = tokio::time::timeout(
                    connect_timeout,
                    tokio_postgres::connect(&self.connection_string, NoTls),
                )
                .await
                .map_err(|_| {
                    CliError::Unreachable("Failed to connect: connection timed out".to_string())
                })?
                .map_err(|e| CliError::Unreachable(format!("Failed to connect: {}", e)))?;
                tokio::spawn(async move {
                    if let Err(e) = connection.await {
                        eprintln!("Connection error: {}", e);
                    }
                });
                client
            } else {
                let (client, connection) = tokio::time::timeout(
                    connect_timeout,
                    tokio_postgres::connect(
                        &self.connection_string,
                        prax_postgres::tls::make_tls_connector(),
                    ),
                )
                .await
                .map_err(|_| {
                    CliError::Unreachable("Failed to connect: connection timed out".to_string())
                })?
                .map_err(|e| CliError::Unreachable(format!("Failed to connect: {}", e)))?;
                tokio::spawn(async move {
                    if let Err(e) = connection.await {
                        eprintln!("Connection error: {}", e);
                    }
                });
                client
            };

            Ok(client)
        }
    }

    impl Introspector for PostgresIntrospector {
        async fn introspect(&self, options: &IntrospectionOptions) -> CliResult<DatabaseSchema> {
            let client = self.connect().await?;
            let schema_name = options.schema.as_deref().unwrap_or("public");

            let mut db_schema = DatabaseSchema {
                name: "database".to_string(),
                schema: Some(schema_name.to_string()),
                ..Default::default()
            };

            // Get tables
            let tables_sql = queries::tables_query(DatabaseType::PostgreSQL, Some(schema_name));
            let table_rows = client
                .query(&tables_sql, &[])
                .await
                .map_err(|e| CliError::Database(format!("Failed to query tables: {}", e)))?;

            for row in table_rows {
                let table_name: String = row.get(0);

                // Apply filters
                if let Some(ref pattern) = options.table_filter
                    && !matches_pattern(&table_name, pattern)
                {
                    continue;
                }
                if let Some(ref exclude) = options.exclude_pattern
                    && matches_pattern(&table_name, exclude)
                {
                    continue;
                }

                let comment: Option<String> = row.try_get(1).ok();

                db_schema.tables.push(TableInfo {
                    name: table_name,
                    schema: Some(schema_name.to_string()),
                    comment: if options.include_comments {
                        comment
                    } else {
                        None
                    },
                    ..Default::default()
                });
            }

            // Fetch columns, primary keys, foreign keys, and indexes for the
            // whole schema in one query each, then group rows by table in
            // memory: 4 round-trips total instead of 4 per table. The table
            // name is appended as the last selected column and used as the
            // first ORDER BY key so grouped rows keep the exact per-table
            // ordering of the original per-table queries.
            let cols_sql = "SELECT \
                    c.column_name, \
                    c.data_type, \
                    c.udt_name, \
                    c.is_nullable = 'YES' as nullable, \
                    c.column_default, \
                    c.character_maximum_length, \
                    c.numeric_precision, \
                    c.numeric_scale, \
                    col_description((quote_ident(c.table_schema) || '.' || quote_ident(c.table_name))::regclass, c.ordinal_position) as comment, \
                    CASE WHEN c.column_default LIKE 'nextval%' THEN true ELSE false END as auto_increment, \
                    c.table_name \
                 FROM information_schema.columns c \
                 WHERE c.table_schema = $1 \
                 ORDER BY c.table_name, c.ordinal_position";
            let col_rows = client
                .query(cols_sql, &[&schema_name])
                .await
                .map_err(|e| CliError::Database(format!("Failed to query columns: {}", e)))?;

            let mut columns_by_table: HashMap<String, Vec<Row>> = HashMap::new();
            for col_row in col_rows {
                let table_name: String = col_row.get(10);
                columns_by_table
                    .entry(table_name)
                    .or_default()
                    .push(col_row);
            }

            let pk_sql = "SELECT a.attname as column_name, c.relname as table_name \
                 FROM pg_index i \
                 JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY(i.indkey) \
                 JOIN pg_class c ON c.oid = i.indrelid \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE i.indisprimary AND n.nspname = $1 \
                 ORDER BY c.relname, array_position(i.indkey, a.attnum)";
            let pk_rows = client
                .query(pk_sql, &[&schema_name])
                .await
                .map_err(|e| CliError::Database(format!("Failed to query primary keys: {}", e)))?;

            let mut pks_by_table: HashMap<String, Vec<Row>> = HashMap::new();
            for pk_row in pk_rows {
                let table_name: String = pk_row.get(1);
                pks_by_table.entry(table_name).or_default().push(pk_row);
            }

            let fk_sql = "SELECT \
                    tc.constraint_name, \
                    kcu.column_name, \
                    ccu.table_name as referenced_table, \
                    ccu.table_schema as referenced_schema, \
                    ccu.column_name as referenced_column, \
                    rc.delete_rule, \
                    rc.update_rule, \
                    tc.table_name \
                 FROM information_schema.table_constraints tc \
                 JOIN information_schema.key_column_usage kcu ON tc.constraint_name = kcu.constraint_name \
                 JOIN information_schema.constraint_column_usage ccu ON ccu.constraint_name = tc.constraint_name \
                 JOIN information_schema.referential_constraints rc ON rc.constraint_name = tc.constraint_name \
                 WHERE tc.constraint_type = 'FOREIGN KEY' AND tc.table_schema = $1 \
                 ORDER BY tc.table_name, tc.constraint_name, kcu.ordinal_position";
            let fk_rows = client
                .query(fk_sql, &[&schema_name])
                .await
                .map_err(|e| CliError::Database(format!("Failed to query foreign keys: {}", e)))?;

            let mut fks_by_table: HashMap<String, Vec<Row>> = HashMap::new();
            for fk_row in fk_rows {
                let table_name: String = fk_row.get(7);
                fks_by_table.entry(table_name).or_default().push(fk_row);
            }

            let idx_sql = "SELECT \
                    i.relname as index_name, \
                    a.attname as column_name, \
                    ix.indisunique as is_unique, \
                    ix.indisprimary as is_primary, \
                    am.amname as index_type, \
                    pg_get_expr(ix.indpred, ix.indrelid) as filter, \
                    t.relname as table_name \
                 FROM pg_index ix \
                 JOIN pg_class t ON t.oid = ix.indrelid \
                 JOIN pg_class i ON i.oid = ix.indexrelid \
                 JOIN pg_namespace n ON n.oid = t.relnamespace \
                 JOIN pg_am am ON i.relam = am.oid \
                 JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = ANY(ix.indkey) \
                 WHERE n.nspname = $1 \
                 ORDER BY t.relname, i.relname, array_position(ix.indkey, a.attnum)";
            let idx_rows = client
                .query(idx_sql, &[&schema_name])
                .await
                .map_err(|e| CliError::Database(format!("Failed to query indexes: {}", e)))?;

            let mut indexes_by_table: HashMap<String, Vec<Row>> = HashMap::new();
            for idx_row in idx_rows {
                let table_name: String = idx_row.get(6);
                indexes_by_table
                    .entry(table_name)
                    .or_default()
                    .push(idx_row);
            }

            // Populate each table from the pre-fetched rows.
            for table in &mut db_schema.tables {
                for col_row in columns_by_table.remove(&table.name).unwrap_or_default() {
                    let col_name: String = col_row.get(0);
                    let data_type: String = col_row.get(1);
                    let udt_name: String = col_row.get(2);
                    let nullable: bool = col_row.get(3);
                    let default: Option<String> = col_row.try_get(4).ok();
                    let max_length: Option<i32> = col_row.try_get(5).ok();
                    let precision: Option<i32> = col_row.try_get(6).ok();
                    let scale: Option<i32> = col_row.try_get(7).ok();
                    let comment: Option<String> = col_row.try_get(8).ok();
                    let auto_increment: bool = col_row.try_get(9).unwrap_or(false);

                    let normalized = normalize_type(
                        DatabaseType::PostgreSQL,
                        &udt_name,
                        max_length,
                        precision,
                        scale,
                    );

                    table.columns.push(ColumnInfo {
                        name: col_name,
                        db_type: data_type,
                        normalized_type: normalized,
                        nullable,
                        default,
                        auto_increment,
                        max_length,
                        precision,
                        scale,
                        comment: if options.include_comments {
                            comment
                        } else {
                            None
                        },
                        ..Default::default()
                    });
                }

                for pk_row in pks_by_table.remove(&table.name).unwrap_or_default() {
                    let col_name: String = pk_row.get(0);
                    table.primary_key.push(col_name.clone());

                    // Mark column as primary key
                    if let Some(col) = table.columns.iter_mut().find(|c| c.name == col_name) {
                        col.is_primary_key = true;
                    }
                }

                let mut fk_map: HashMap<String, ForeignKeyInfo> = HashMap::new();
                for fk_row in fks_by_table.remove(&table.name).unwrap_or_default() {
                    let constraint_name: String = fk_row.get(0);
                    let column_name: String = fk_row.get(1);
                    let ref_table: String = fk_row.get(2);
                    let ref_schema: Option<String> = fk_row.try_get(3).ok();
                    let ref_column: String = fk_row.get(4);
                    let delete_rule: String = fk_row.get(5);
                    let update_rule: String = fk_row.get(6);

                    let fk =
                        fk_map
                            .entry(constraint_name.clone())
                            .or_insert_with(|| ForeignKeyInfo {
                                name: constraint_name,
                                columns: Vec::new(),
                                referenced_table: ref_table,
                                referenced_schema: ref_schema,
                                referenced_columns: Vec::new(),
                                on_delete: ReferentialAction::from_str(&delete_rule),
                                on_update: ReferentialAction::from_str(&update_rule),
                            });

                    fk.columns.push(column_name);
                    fk.referenced_columns.push(ref_column);
                }

                table.foreign_keys = fk_map.into_values().collect();

                let mut idx_map: HashMap<String, IndexInfo> = HashMap::new();
                for idx_row in indexes_by_table.remove(&table.name).unwrap_or_default() {
                    let idx_name: String = idx_row.get(0);
                    let col_name: String = idx_row.get(1);
                    let is_unique: bool = idx_row.get(2);
                    let is_primary: bool = idx_row.get(3);
                    let idx_type: Option<String> = idx_row.try_get(4).ok();
                    let filter: Option<String> = idx_row.try_get(5).ok();

                    let idx = idx_map
                        .entry(idx_name.clone())
                        .or_insert_with(|| IndexInfo {
                            name: idx_name,
                            columns: Vec::new(),
                            is_unique,
                            is_primary,
                            index_type: idx_type,
                            filter,
                        });

                    idx.columns.push(IndexColumn {
                        name: col_name,
                        order: SortOrder::Asc,
                        ..Default::default()
                    });
                }

                table.indexes = idx_map.into_values().collect();
            }

            // Get enums
            let enums_sql = queries::enums_query(Some(schema_name));
            let enum_rows = client
                .query(&enums_sql, &[])
                .await
                .map_err(|e| CliError::Database(format!("Failed to query enums: {}", e)))?;

            let mut enum_map: HashMap<String, EnumInfo> = HashMap::new();
            for enum_row in enum_rows {
                let enum_name: String = enum_row.get(0);
                let enum_value: String = enum_row.get(1);

                let enum_info = enum_map
                    .entry(enum_name.clone())
                    .or_insert_with(|| EnumInfo {
                        name: enum_name,
                        schema: Some(schema_name.to_string()),
                        values: Vec::new(),
                    });

                enum_info.values.push(enum_value);
            }

            db_schema.enums = enum_map.into_values().collect();

            // Get views
            if options.include_views || options.include_materialized_views {
                let views_sql = queries::views_query(DatabaseType::PostgreSQL, Some(schema_name));
                let view_rows = client
                    .query(&views_sql, &[])
                    .await
                    .map_err(|e| CliError::Database(format!("Failed to query views: {}", e)))?;

                for view_row in view_rows {
                    let view_name: String = view_row.get(0);
                    let definition: Option<String> = view_row.try_get(1).ok();
                    let is_materialized: bool = view_row.get(2);

                    if is_materialized && !options.include_materialized_views {
                        continue;
                    }
                    if !is_materialized && !options.include_views {
                        continue;
                    }

                    db_schema.views.push(ViewInfo {
                        name: view_name,
                        schema: Some(schema_name.to_string()),
                        definition,
                        is_materialized,
                        columns: Vec::new(),
                    });
                }
            }

            Ok(db_schema)
        }
    }

    /// Whether a parsed DSN host is local (loopback TCP or a Unix socket).
    fn is_local_host(host: &tokio_postgres::config::Host) -> bool {
        match host {
            tokio_postgres::config::Host::Tcp(name) => {
                name == "localhost" || name == "127.0.0.1" || name == "::1"
            }
            tokio_postgres::config::Host::Unix(_) => true,
        }
    }
}

// ============================================================================
// Shared helpers for JSON-row backends (MySQL)
// ============================================================================

/// Bounded connect timeout for introspection pools. Introspection is a
/// short-lived, interactive step; without a bound an unreachable-but-not-
/// refused host (e.g. a firewall drop) would hang the CLI. `migrate dev`
/// relies on this so it can fall back to greenfield when a DB is unreachable.
#[cfg(any(
    feature = "postgres",
    feature = "mysql",
    feature = "sqlite",
    feature = "mssql"
))]
pub(crate) const INTROSPECT_CONNECT_TIMEOUT_SECS: u64 = 5;

/// A source of introspection rows returned as JSON objects, keyed by column
/// name. Implemented for the raw engines of the JSON-capable backends so the
/// MySQL and SQLite introspectors share one row-fetch shim instead of each
/// hand-rolling `raw_sql_query(sql, &[]) -> into_json`.
#[cfg(any(feature = "mysql", feature = "sqlite"))]
trait JsonRowSource {
    /// Run `sql` (no bind params) and return each row as a JSON object.
    async fn json_rows(&self, sql: &str) -> CliResult<Vec<serde_json::Value>>;
}

#[cfg(feature = "mysql")]
impl JsonRowSource for prax_mysql::MysqlRawEngine {
    async fn json_rows(&self, sql: &str) -> CliResult<Vec<serde_json::Value>> {
        let rows = self
            .raw_sql_query(sql, &[])
            .await
            .map_err(|e| CliError::Database(format!("Introspection query failed: {}", e)))?;
        Ok(rows.into_iter().map(|r| r.into_json()).collect())
    }
}

#[cfg(feature = "sqlite")]
impl JsonRowSource for prax_sqlite::SqliteRawEngine {
    async fn json_rows(&self, sql: &str) -> CliResult<Vec<serde_json::Value>> {
        let rows = self
            .raw_sql_query(sql, &[])
            .await
            .map_err(|e| CliError::Database(format!("Introspection query failed: {}", e)))?;
        Ok(rows.into_iter().map(|r| r.into_json()).collect())
    }
}

/// Simple glob-style pattern matching shared across introspectors.
///
/// Supported subset: `*` (match all), `pre*` (prefix), `*suf` (suffix),
/// `*mid*` (substring/contains). Interior wildcards (e.g. `a*b*c` or
/// `pre*suf`) are **not** supported — such a pattern falls through to an
/// exact-string compare and will typically match nothing. Callers should
/// stick to the four supported shapes for table include/exclude filters.
#[cfg(any(
    feature = "postgres",
    feature = "mysql",
    feature = "sqlite",
    feature = "mssql"
))]
fn matches_pattern(name: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }

    if pattern.starts_with('*') && pattern.ends_with('*') {
        let middle = &pattern[1..pattern.len() - 1];
        return name.contains(middle);
    }

    if let Some(suffix) = pattern.strip_prefix('*') {
        return name.ends_with(suffix);
    }

    if let Some(prefix) = pattern.strip_suffix('*') {
        return name.starts_with(prefix);
    }

    name == pattern
}

/// Look up a column in a JSON object row case-insensitively.
///
/// MySQL's `information_schema` returns unaliased column names in uppercase
/// (e.g. `TABLE_NAME`) while aliased expressions keep the alias case, so a
/// single query row can mix cases. Match the exact key first, then fall back
/// to a case-insensitive scan.
#[cfg(any(feature = "mysql", feature = "sqlite"))]
fn json_get<'a>(row: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    if let Some(v) = row.get(key) {
        return Some(v);
    }
    row.as_object()?
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v)
}

/// Read an optional string column from a serde_json object row.
#[cfg(any(feature = "mysql", feature = "sqlite"))]
fn json_str(row: &serde_json::Value, key: &str) -> Option<String> {
    json_get(row, key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// Read a boolean column from a JSON object row, treating MySQL's 1/0 and
/// "YES"/"NO" forms as booleans. Only the MySQL introspector needs this;
/// SQLite reads its PRAGMA booleans via `json_i32(...) != 0`.
#[cfg(feature = "mysql")]
fn json_bool(row: &serde_json::Value, key: &str) -> bool {
    match json_get(row, key) {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::Number(n)) => n.as_i64().is_some_and(|i| i != 0),
        Some(serde_json::Value::String(s)) => {
            matches!(s.as_str(), "1" | "YES" | "yes" | "true" | "TRUE")
        }
        _ => false,
    }
}

/// Read an integer column from a JSON object row.
#[cfg(any(feature = "mysql", feature = "sqlite"))]
fn json_i32(row: &serde_json::Value, key: &str) -> Option<i32> {
    match json_get(row, key) {
        Some(serde_json::Value::Number(n)) => n.as_i64().and_then(|i| i32::try_from(i).ok()),
        Some(serde_json::Value::String(s)) => s.parse::<i32>().ok(),
        _ => None,
    }
}

// ============================================================================
// MySQL Introspector
// ============================================================================

#[cfg(feature = "mysql")]
pub mod mysql {
    use super::*;
    use prax_mysql::{MysqlPool, MysqlRawEngine};

    /// MySQL introspector backed by the `prax-mysql` engine's raw query API.
    pub struct MysqlIntrospector {
        connection_string: String,
    }

    impl MysqlIntrospector {
        /// Create a new MySQL introspector.
        pub fn new(connection_string: String) -> Self {
            Self { connection_string }
        }

        async fn engine(&self) -> CliResult<MysqlRawEngine> {
            let pool = MysqlPool::builder()
                .url(self.connection_string.clone())
                .connection_timeout(std::time::Duration::from_secs(
                    super::INTROSPECT_CONNECT_TIMEOUT_SECS,
                ))
                .build()
                .await
                .map_err(|e| CliError::Unreachable(format!("Failed to connect: {}", e)))?;
            Ok(MysqlRawEngine::new(pool))
        }

        /// Run introspection SQL, returning each row as a JSON object.
        /// Delegates to the shared [`super::JsonRowSource`] shim.
        async fn rows(engine: &MysqlRawEngine, sql: &str) -> CliResult<Vec<serde_json::Value>> {
            super::JsonRowSource::json_rows(engine, sql).await
        }
    }

    impl Introspector for MysqlIntrospector {
        async fn introspect(&self, options: &IntrospectionOptions) -> CliResult<DatabaseSchema> {
            let engine = self.engine().await?;
            // MySQL has no schema namespace distinct from the database; the
            // connection's default database scopes information_schema queries.
            let schema = options.schema.clone();
            let schema_ref = schema.as_deref();

            let mut db_schema = DatabaseSchema {
                name: "database".to_string(),
                schema: schema.clone(),
                ..Default::default()
            };

            let table_rows = Self::rows(
                &engine,
                &queries::tables_query(DatabaseType::MySQL, schema_ref),
            )
            .await?;
            for row in &table_rows {
                let Some(table_name) = json_str(row, "table_name") else {
                    continue;
                };
                if let Some(ref pattern) = options.table_filter
                    && !matches_pattern(&table_name, pattern)
                {
                    continue;
                }
                if let Some(ref exclude) = options.exclude_pattern
                    && matches_pattern(&table_name, exclude)
                {
                    continue;
                }

                db_schema.tables.push(TableInfo {
                    name: table_name,
                    schema: schema.clone(),
                    comment: if options.include_comments {
                        json_str(row, "comment").filter(|c| !c.is_empty())
                    } else {
                        None
                    },
                    ..Default::default()
                });
            }

            for table in &mut db_schema.tables {
                populate_table(&engine, table, schema_ref, options).await?;
            }

            Ok(db_schema)
        }
    }

    /// Fill a table's columns, primary key, foreign keys, and indexes.
    async fn populate_table(
        engine: &MysqlRawEngine,
        table: &mut TableInfo,
        schema: Option<&str>,
        options: &IntrospectionOptions,
    ) -> CliResult<()> {
        // Columns
        let col_rows = MysqlIntrospector::rows(
            engine,
            &queries::columns_query(DatabaseType::MySQL, &table.name, schema),
        )
        .await?;
        for row in &col_rows {
            let Some(name) = json_str(row, "column_name") else {
                continue;
            };
            let data_type = json_str(row, "data_type").unwrap_or_default();
            let max_length = json_i32(row, "character_maximum_length");
            let precision = json_i32(row, "numeric_precision");
            let scale = json_i32(row, "numeric_scale");
            let normalized = normalize_type(
                DatabaseType::MySQL,
                &data_type,
                max_length,
                precision,
                scale,
            );

            table.columns.push(ColumnInfo {
                name,
                db_type: data_type,
                normalized_type: normalized,
                nullable: json_bool(row, "nullable"),
                default: json_str(row, "column_default"),
                auto_increment: json_bool(row, "auto_increment"),
                max_length,
                precision,
                scale,
                comment: if options.include_comments {
                    json_str(row, "comment").filter(|c| !c.is_empty())
                } else {
                    None
                },
                ..Default::default()
            });
        }

        // Primary key
        let pk_rows = MysqlIntrospector::rows(
            engine,
            &queries::primary_keys_query(DatabaseType::MySQL, &table.name, schema),
        )
        .await?;
        for row in &pk_rows {
            if let Some(col) = json_str(row, "column_name") {
                table.primary_key.push(col.clone());
                if let Some(c) = table.columns.iter_mut().find(|c| c.name == col) {
                    c.is_primary_key = true;
                }
            }
        }

        // Foreign keys (grouped by constraint name, columns in order)
        let fk_rows = MysqlIntrospector::rows(
            engine,
            &queries::foreign_keys_query(DatabaseType::MySQL, &table.name, schema),
        )
        .await?;
        let mut fk_map: std::collections::HashMap<String, ForeignKeyInfo> =
            std::collections::HashMap::new();
        let mut fk_order: Vec<String> = Vec::new();
        for row in &fk_rows {
            let Some(cname) = json_str(row, "constraint_name") else {
                continue;
            };
            let fk = fk_map.entry(cname.clone()).or_insert_with(|| {
                fk_order.push(cname.clone());
                ForeignKeyInfo {
                    name: cname.clone(),
                    columns: Vec::new(),
                    referenced_table: json_str(row, "referenced_table").unwrap_or_default(),
                    referenced_schema: json_str(row, "referenced_schema"),
                    referenced_columns: Vec::new(),
                    on_delete: ReferentialAction::from_str(
                        &json_str(row, "delete_rule").unwrap_or_default(),
                    ),
                    on_update: ReferentialAction::from_str(
                        &json_str(row, "update_rule").unwrap_or_default(),
                    ),
                }
            });
            if let Some(col) = json_str(row, "column_name") {
                fk.columns.push(col);
            }
            if let Some(rc) = json_str(row, "referenced_column") {
                fk.referenced_columns.push(rc);
            }
        }
        table.foreign_keys = fk_order
            .into_iter()
            .filter_map(|n| fk_map.remove(&n))
            .collect();

        // Indexes (grouped by name, columns in order)
        let idx_rows = MysqlIntrospector::rows(
            engine,
            &queries::indexes_query(DatabaseType::MySQL, &table.name, schema),
        )
        .await?;
        let mut idx_map: std::collections::HashMap<String, IndexInfo> =
            std::collections::HashMap::new();
        let mut idx_order: Vec<String> = Vec::new();
        for row in &idx_rows {
            let Some(iname) = json_str(row, "index_name") else {
                continue;
            };
            let idx = idx_map.entry(iname.clone()).or_insert_with(|| {
                idx_order.push(iname.clone());
                IndexInfo {
                    name: iname.clone(),
                    columns: Vec::new(),
                    is_unique: json_bool(row, "is_unique"),
                    is_primary: json_bool(row, "is_primary"),
                    index_type: json_str(row, "index_type"),
                    filter: json_str(row, "filter"),
                }
            });
            if let Some(col) = json_str(row, "column_name") {
                idx.columns.push(IndexColumn {
                    name: col,
                    order: SortOrder::Asc,
                    ..Default::default()
                });
            }
        }
        table.indexes = idx_order
            .into_iter()
            .filter_map(|n| idx_map.remove(&n))
            .collect();

        Ok(())
    }
}

// ============================================================================
// SQLite Introspector
// ============================================================================

#[cfg(feature = "sqlite")]
pub mod sqlite {
    use super::*;
    use prax_sqlite::{SqlitePool, SqliteRawEngine};

    /// SQLite introspector backed by the `prax-sqlite` engine's raw query API.
    ///
    /// SQLite exposes structure through PRAGMAs rather than an
    /// `information_schema`, so this introspector parses PRAGMA output shapes
    /// (`table_info`, `foreign_key_list`, `index_list`/`index_info`) directly.
    pub struct SqliteIntrospector {
        connection_string: String,
    }

    impl SqliteIntrospector {
        /// Create a new SQLite introspector.
        pub fn new(connection_string: String) -> Self {
            Self { connection_string }
        }

        async fn engine(&self) -> CliResult<SqliteRawEngine> {
            let pool = SqlitePool::builder()
                .url(self.connection_string.clone())
                .connection_timeout(std::time::Duration::from_secs(
                    super::INTROSPECT_CONNECT_TIMEOUT_SECS,
                ))
                .build()
                .await
                .map_err(|e| CliError::Unreachable(format!("Failed to open database: {}", e)))?;
            Ok(SqliteRawEngine::new(pool))
        }

        async fn rows(engine: &SqliteRawEngine, sql: &str) -> CliResult<Vec<serde_json::Value>> {
            super::JsonRowSource::json_rows(engine, sql).await
        }
    }

    impl Introspector for SqliteIntrospector {
        async fn introspect(&self, options: &IntrospectionOptions) -> CliResult<DatabaseSchema> {
            let engine = self.engine().await?;

            let mut db_schema = DatabaseSchema {
                name: "database".to_string(),
                schema: None,
                ..Default::default()
            };

            // SQLite has no schema namespace; the tables_query ignores it.
            let table_rows =
                Self::rows(&engine, &queries::tables_query(DatabaseType::SQLite, None)).await?;
            for row in &table_rows {
                let Some(table_name) = json_str(row, "table_name") else {
                    continue;
                };
                if let Some(ref pattern) = options.table_filter
                    && !matches_pattern(&table_name, pattern)
                {
                    continue;
                }
                if let Some(ref exclude) = options.exclude_pattern
                    && matches_pattern(&table_name, exclude)
                {
                    continue;
                }
                db_schema.tables.push(TableInfo {
                    name: table_name,
                    ..Default::default()
                });
            }

            for table in &mut db_schema.tables {
                populate_table(&engine, table).await?;
            }

            Ok(db_schema)
        }
    }

    async fn populate_table(engine: &SqliteRawEngine, table: &mut TableInfo) -> CliResult<()> {
        // PRAGMA table_info: cid, name, type, notnull, dflt_value, pk
        // `pk` is the 1-based ordinal within the primary key (0 = not part).
        let col_rows = SqliteIntrospector::rows(
            engine,
            &queries::columns_query(DatabaseType::SQLite, &table.name, None),
        )
        .await?;
        let mut pk_positions: Vec<(i32, String)> = Vec::new();
        for row in &col_rows {
            let Some(name) = json_str(row, "name") else {
                continue;
            };
            let decl_type = json_str(row, "type").unwrap_or_default();
            // SQLite types can carry a size, e.g. VARCHAR(255); normalize on
            // the affinity keyword (leading identifier chars).
            let base_type: String = decl_type
                .split([' ', '('])
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            let normalized = normalize_type(DatabaseType::SQLite, &base_type, None, None, None);
            let not_null = json_i32(row, "notnull").unwrap_or(0) != 0;
            let pk_pos = json_i32(row, "pk").unwrap_or(0);
            if pk_pos > 0 {
                pk_positions.push((pk_pos, name.clone()));
            }

            table.columns.push(ColumnInfo {
                name,
                db_type: decl_type,
                normalized_type: normalized,
                // Nullability comes solely from the column's NOT NULL flag;
                // PK membership is represented separately via is_primary_key
                // (→ @id), so a composite PK with a nullable member is not
                // silently forced non-null.
                nullable: !not_null,
                default: json_str(row, "dflt_value"),
                // rowid INTEGER PRIMARY KEY columns auto-increment; detected
                // below once the PK is known.
                auto_increment: false,
                is_primary_key: pk_pos > 0,
                ..Default::default()
            });
        }

        // Primary key columns in PK order.
        pk_positions.sort_by_key(|(pos, _)| *pos);
        table.primary_key = pk_positions.into_iter().map(|(_, name)| name).collect();

        // A single INTEGER PRIMARY KEY is an alias for rowid (auto-increment).
        if table.primary_key.len() == 1
            && let Some(col) = table
                .columns
                .iter_mut()
                .find(|c| c.name == table.primary_key[0])
            && col.db_type.to_ascii_lowercase().contains("int")
        {
            col.auto_increment = true;
        }

        // PRAGMA foreign_key_list: id, seq, table, from, to, on_update, on_delete, match
        // Rows for one FK share `id`; `seq` orders the columns.
        let fk_rows = SqliteIntrospector::rows(
            engine,
            &queries::foreign_keys_query(DatabaseType::SQLite, &table.name, None),
        )
        .await?;
        let mut fk_map: std::collections::BTreeMap<i64, ForeignKeyInfo> =
            std::collections::BTreeMap::new();
        for row in &fk_rows {
            let id = row.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
            let fk = fk_map.entry(id).or_insert_with(|| ForeignKeyInfo {
                // SQLite FKs are unnamed; synthesize a stable name so the
                // diff engine can match them. The .prax must pin this via
                // @relation(map: "fk_<table>_<col>") to round-trip cleanly.
                name: String::new(),
                columns: Vec::new(),
                referenced_table: json_str(row, "table").unwrap_or_default(),
                referenced_schema: None,
                referenced_columns: Vec::new(),
                on_delete: ReferentialAction::from_str(
                    &json_str(row, "on_delete").unwrap_or_default(),
                ),
                on_update: ReferentialAction::from_str(
                    &json_str(row, "on_update").unwrap_or_default(),
                ),
            });
            if let Some(from) = json_str(row, "from") {
                fk.columns.push(from);
            }
            if let Some(to) = json_str(row, "to") {
                fk.referenced_columns.push(to);
            }
        }
        table.foreign_keys = fk_map
            .into_values()
            .map(|mut fk| {
                if fk.name.is_empty() {
                    fk.name = format!("fk_{}_{}", table.name, fk.columns.join("_"));
                }
                fk
            })
            .collect();

        // PRAGMA index_list: seq, name, unique, origin, partial
        // origin 'pk' is the implicit PK index; skip it (already represented).
        let idx_rows = SqliteIntrospector::rows(
            engine,
            &queries::indexes_query(DatabaseType::SQLite, &table.name, None),
        )
        .await?;
        for row in &idx_rows {
            let Some(idx_name) = json_str(row, "name") else {
                continue;
            };
            let origin = json_str(row, "origin").unwrap_or_default();
            let is_primary = origin == "pk";
            let is_unique = json_i32(row, "unique").unwrap_or(0) != 0;

            // PRAGMA index_info(name): seqno, cid, name — the indexed columns.
            let info_rows = SqliteIntrospector::rows(
                engine,
                &format!("PRAGMA index_info('{}')", idx_name.replace('\'', "''")),
            )
            .await?;
            let columns: Vec<IndexColumn> = info_rows
                .iter()
                .filter_map(|r| json_str(r, "name"))
                .map(|name| IndexColumn {
                    name,
                    order: SortOrder::Asc,
                    ..Default::default()
                })
                .collect();

            table.indexes.push(IndexInfo {
                name: idx_name,
                columns,
                is_unique,
                is_primary,
                index_type: None,
                filter: None,
            });
        }

        Ok(())
    }
}

// ============================================================================
// MSSQL Introspector
// ============================================================================

#[cfg(feature = "mssql")]
pub mod mssql {
    use super::*;
    use prax_mssql::MssqlPool;
    use prax_mssql::Row;

    /// MSSQL introspector backed by the `prax-mssql` engine's pooled
    /// connection. Reads typed `tiberius::Row` columns from the `sys.*`
    /// catalog queries.
    pub struct MssqlIntrospector {
        connection_string: String,
    }

    impl MssqlIntrospector {
        /// Create a new MSSQL introspector.
        pub fn new(connection_string: String) -> Self {
            Self { connection_string }
        }
    }

    /// Read a nullable string column by name.
    fn row_str(row: &Row, col: &str) -> Option<String> {
        row.try_get::<&str, _>(col)
            .ok()
            .flatten()
            .map(str::to_string)
    }

    /// Read a bit/int column as a bool.
    fn row_bool(row: &Row, col: &str) -> bool {
        if let Ok(Some(b)) = row.try_get::<bool, _>(col) {
            return b;
        }
        // Some bit-like columns arrive as integers.
        row_i64(row, col).is_some_and(|i| i != 0)
    }

    /// Read an integer column, tolerating i16/i32/i64/u8 widths.
    fn row_i64(row: &Row, col: &str) -> Option<i64> {
        if let Ok(Some(v)) = row.try_get::<i32, _>(col) {
            return Some(v as i64);
        }
        if let Ok(Some(v)) = row.try_get::<i64, _>(col) {
            return Some(v);
        }
        if let Ok(Some(v)) = row.try_get::<i16, _>(col) {
            return Some(v as i64);
        }
        if let Ok(Some(v)) = row.try_get::<u8, _>(col) {
            return Some(v as i64);
        }
        None
    }

    fn row_i32(row: &Row, col: &str) -> Option<i32> {
        row_i64(row, col).and_then(|v| i32::try_from(v).ok())
    }

    impl Introspector for MssqlIntrospector {
        async fn introspect(&self, options: &IntrospectionOptions) -> CliResult<DatabaseSchema> {
            let pool = MssqlPool::builder()
                .connection_string(self.connection_string.clone())
                .connection_timeout(std::time::Duration::from_secs(
                    super::INTROSPECT_CONNECT_TIMEOUT_SECS,
                ))
                .build()
                .await
                .map_err(|e| CliError::Unreachable(format!("Failed to connect: {}", e)))?;
            let mut conn = pool.get().await.map_err(|e| {
                CliError::Unreachable(format!("Failed to acquire connection: {}", e))
            })?;

            let schema_name = options.schema.clone().unwrap_or_else(|| "dbo".to_string());
            let schema_ref = Some(schema_name.as_str());

            let mut db_schema = DatabaseSchema {
                name: "database".to_string(),
                schema: Some(schema_name.clone()),
                ..Default::default()
            };

            let table_rows = conn
                .query(&queries::tables_query(DatabaseType::MSSQL, schema_ref), &[])
                .await
                .map_err(|e| CliError::Database(format!("Failed to query tables: {}", e)))?;
            for row in &table_rows {
                let Some(table_name) = row_str(row, "table_name") else {
                    continue;
                };
                if let Some(ref pattern) = options.table_filter
                    && !matches_pattern(&table_name, pattern)
                {
                    continue;
                }
                if let Some(ref exclude) = options.exclude_pattern
                    && matches_pattern(&table_name, exclude)
                {
                    continue;
                }
                db_schema.tables.push(TableInfo {
                    name: table_name,
                    schema: Some(schema_name.clone()),
                    comment: if options.include_comments {
                        row_str(row, "comment")
                    } else {
                        None
                    },
                    ..Default::default()
                });
            }

            for i in 0..db_schema.tables.len() {
                let table_name = db_schema.tables[i].name.clone();

                // Columns
                let col_rows = conn
                    .query(
                        &queries::columns_query(DatabaseType::MSSQL, &table_name, schema_ref),
                        &[],
                    )
                    .await
                    .map_err(|e| CliError::Database(format!("Failed to query columns: {}", e)))?;
                for row in &col_rows {
                    let Some(name) = row_str(row, "column_name") else {
                        continue;
                    };
                    let data_type = row_str(row, "data_type").unwrap_or_default();
                    // NOTE: sys.columns.max_length is a BYTE length, not a
                    // character count — nvarchar(255) reports 510 and MAX
                    // reports -1. Harmless today because VarChar/Char normalize
                    // to TEXT (length discarded) in the diff-source mapping; if
                    // length-sensitive types are added, halve for n-types and
                    // special-case -1 → MAX before comparing.
                    let max_length = row_i32(row, "character_maximum_length");
                    let precision = row_i32(row, "numeric_precision");
                    let scale = row_i32(row, "numeric_scale");
                    let normalized = normalize_type(
                        DatabaseType::MSSQL,
                        &data_type,
                        max_length,
                        precision,
                        scale,
                    );
                    db_schema.tables[i].columns.push(ColumnInfo {
                        name,
                        db_type: data_type,
                        normalized_type: normalized,
                        nullable: row_bool(row, "nullable"),
                        default: row_str(row, "column_default"),
                        auto_increment: row_bool(row, "auto_increment"),
                        max_length,
                        precision,
                        scale,
                        comment: if options.include_comments {
                            row_str(row, "comment")
                        } else {
                            None
                        },
                        ..Default::default()
                    });
                }

                // Primary key
                let pk_rows = conn
                    .query(
                        &queries::primary_keys_query(DatabaseType::MSSQL, &table_name, schema_ref),
                        &[],
                    )
                    .await
                    .map_err(|e| {
                        CliError::Database(format!("Failed to query primary keys: {}", e))
                    })?;
                for row in &pk_rows {
                    if let Some(col) = row_str(row, "column_name") {
                        db_schema.tables[i].primary_key.push(col.clone());
                        if let Some(c) = db_schema.tables[i]
                            .columns
                            .iter_mut()
                            .find(|c| c.name == col)
                        {
                            c.is_primary_key = true;
                        }
                    }
                }

                // Foreign keys
                let fk_rows = conn
                    .query(
                        &queries::foreign_keys_query(DatabaseType::MSSQL, &table_name, schema_ref),
                        &[],
                    )
                    .await
                    .map_err(|e| {
                        CliError::Database(format!("Failed to query foreign keys: {}", e))
                    })?;
                let mut fk_map: std::collections::HashMap<String, ForeignKeyInfo> =
                    std::collections::HashMap::new();
                let mut fk_order: Vec<String> = Vec::new();
                for row in &fk_rows {
                    let Some(cname) = row_str(row, "constraint_name") else {
                        continue;
                    };
                    let fk = fk_map.entry(cname.clone()).or_insert_with(|| {
                        fk_order.push(cname.clone());
                        ForeignKeyInfo {
                            name: cname.clone(),
                            columns: Vec::new(),
                            referenced_table: row_str(row, "referenced_table").unwrap_or_default(),
                            referenced_schema: row_str(row, "referenced_schema"),
                            referenced_columns: Vec::new(),
                            on_delete: ReferentialAction::from_str(
                                &row_str(row, "delete_rule").unwrap_or_default(),
                            ),
                            on_update: ReferentialAction::from_str(
                                &row_str(row, "update_rule").unwrap_or_default(),
                            ),
                        }
                    });
                    if let Some(col) = row_str(row, "column_name") {
                        fk.columns.push(col);
                    }
                    if let Some(rc) = row_str(row, "referenced_column") {
                        fk.referenced_columns.push(rc);
                    }
                }
                db_schema.tables[i].foreign_keys = fk_order
                    .into_iter()
                    .filter_map(|n| fk_map.remove(&n))
                    .collect();

                // Indexes
                let idx_rows = conn
                    .query(
                        &queries::indexes_query(DatabaseType::MSSQL, &table_name, schema_ref),
                        &[],
                    )
                    .await
                    .map_err(|e| CliError::Database(format!("Failed to query indexes: {}", e)))?;
                let mut idx_map: std::collections::HashMap<String, IndexInfo> =
                    std::collections::HashMap::new();
                let mut idx_order: Vec<String> = Vec::new();
                for row in &idx_rows {
                    let Some(iname) = row_str(row, "index_name") else {
                        continue;
                    };
                    let idx = idx_map.entry(iname.clone()).or_insert_with(|| {
                        idx_order.push(iname.clone());
                        IndexInfo {
                            name: iname.clone(),
                            columns: Vec::new(),
                            is_unique: row_bool(row, "is_unique"),
                            is_primary: row_bool(row, "is_primary"),
                            index_type: row_str(row, "index_type"),
                            filter: row_str(row, "filter"),
                        }
                    });
                    if let Some(col) = row_str(row, "column_name") {
                        idx.columns.push(IndexColumn {
                            name: col,
                            order: SortOrder::Asc,
                            ..Default::default()
                        });
                    }
                }
                db_schema.tables[i].indexes = idx_order
                    .into_iter()
                    .filter_map(|n| idx_map.remove(&n))
                    .collect();
            }

            Ok(db_schema)
        }
    }
}

// ============================================================================
// Output Formatters
// ============================================================================

/// Generate Prax schema output.
pub fn format_as_prax(schema: &DatabaseSchema, config: &Config) -> String {
    let mut output = String::new();

    output.push_str("// Generated by `prax db pull`\n");
    output.push_str("// Edit this file to customize your schema\n\n");

    output.push_str("datasource db {\n");
    output.push_str(&format!(
        "    provider = \"{}\"\n",
        config.database.provider
    ));
    output.push_str("    url      = env(\"DATABASE_URL\")\n");
    output.push_str("}\n\n");

    output.push_str("generator client {\n");
    output.push_str("    provider = \"prax-client-rust\"\n");
    output.push_str("    output   = \"./src/generated\"\n");
    output.push_str("}\n\n");

    // Use the generate_prax_schema function
    output.push_str(&generate_prax_schema(schema));

    output
}

/// Generate JSON output.
pub fn format_as_json(schema: &DatabaseSchema) -> CliResult<String> {
    serde_json::to_string_pretty(schema)
        .map_err(|e| CliError::Config(format!("Failed to serialize schema: {}", e)))
}

/// Generate SQL DDL output.
pub fn format_as_sql(schema: &DatabaseSchema, db_type: DatabaseType) -> String {
    let mut output = String::new();

    output.push_str("-- Generated by `prax db pull`\n");
    output.push_str(&format!("-- Database: {}\n\n", db_type_name(db_type)));

    // Generate enums (PostgreSQL only)
    if db_type == DatabaseType::PostgreSQL {
        for enum_info in &schema.enums {
            output.push_str(&format!("CREATE TYPE {} AS ENUM (\n", enum_info.name));
            let values: Vec<String> = enum_info
                .values
                .iter()
                .map(|v| format!("    '{}'", v))
                .collect();
            output.push_str(&values.join(",\n"));
            output.push_str("\n);\n\n");
        }
    }

    // Generate tables
    for table in &schema.tables {
        output.push_str(&format!(
            "CREATE TABLE {} (\n",
            quote_identifier(&table.name, db_type)
        ));

        let mut col_defs: Vec<String> = Vec::new();

        for col in &table.columns {
            let mut def = format!(
                "    {} {}",
                quote_identifier(&col.name, db_type),
                col.db_type
            );

            if !col.nullable {
                def.push_str(" NOT NULL");
            }

            if let Some(ref default) = col.default {
                def.push_str(&format!(" DEFAULT {}", default));
            }

            col_defs.push(def);
        }

        // Primary key
        if !table.primary_key.is_empty() {
            let pk_cols: Vec<String> = table
                .primary_key
                .iter()
                .map(|c| quote_identifier(c, db_type))
                .collect();
            col_defs.push(format!("    PRIMARY KEY ({})", pk_cols.join(", ")));
        }

        output.push_str(&col_defs.join(",\n"));
        output.push_str("\n);\n\n");

        // Indexes
        for idx in &table.indexes {
            if idx.is_primary {
                continue;
            }

            let unique = if idx.is_unique { "UNIQUE " } else { "" };
            let cols: Vec<String> = idx
                .columns
                .iter()
                .map(|c| quote_identifier(&c.name, db_type))
                .collect();

            output.push_str(&format!(
                "CREATE {}INDEX {} ON {} ({});\n",
                unique,
                quote_identifier(&idx.name, db_type),
                quote_identifier(&table.name, db_type),
                cols.join(", ")
            ));
        }

        output.push('\n');
    }

    output
}

fn db_type_name(db_type: DatabaseType) -> &'static str {
    match db_type {
        DatabaseType::PostgreSQL => "PostgreSQL",
        DatabaseType::MySQL => "MySQL",
        DatabaseType::SQLite => "SQLite",
        DatabaseType::MSSQL => "SQL Server",
    }
}

fn quote_identifier(name: &str, db_type: DatabaseType) -> String {
    match db_type {
        DatabaseType::PostgreSQL => format!("\"{}\"", name),
        DatabaseType::MySQL => format!("`{}`", name),
        DatabaseType::SQLite => format!("\"{}\"", name),
        DatabaseType::MSSQL => format!("[{}]", name),
    }
}
