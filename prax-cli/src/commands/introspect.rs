//! Database introspection implementation.
//!
//! This module provides the actual database introspection functionality
//! using the `prax-query` introspection types.

use std::collections::HashMap;

use prax_query::introspection::{
    ColumnInfo, DatabaseSchema, EnumInfo, ForeignKeyInfo, IndexColumn, IndexInfo,
    ReferentialAction, SortOrder, TableInfo, ViewInfo, generate_prax_schema, normalize_type,
    queries,
};
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

// ============================================================================
// PostgreSQL Introspector
// ============================================================================

#[cfg(feature = "postgres")]
pub mod postgres {
    use super::*;
    use tokio_postgres::{Client, NoTls};

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
            let (client, connection) = tokio_postgres::connect(&self.connection_string, NoTls)
                .await
                .map_err(|e| CliError::Database(format!("Failed to connect: {}", e)))?;

            // Spawn the connection handler
            tokio::spawn(async move {
                if let Err(e) = connection.await {
                    eprintln!("Connection error: {}", e);
                }
            });

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
                if let Some(ref pattern) = options.table_filter {
                    if !matches_pattern(&table_name, pattern) {
                        continue;
                    }
                }
                if let Some(ref exclude) = options.exclude_pattern {
                    if matches_pattern(&table_name, exclude) {
                        continue;
                    }
                }

                let comment: Option<String> = row.try_get(1).ok();

                let mut table = TableInfo {
                    name: table_name.clone(),
                    schema: Some(schema_name.to_string()),
                    comment: if options.include_comments {
                        comment
                    } else {
                        None
                    },
                    ..Default::default()
                };

                // Get columns
                let cols_sql = queries::columns_query(
                    DatabaseType::PostgreSQL,
                    &table_name,
                    Some(schema_name),
                );
                let col_rows = client
                    .query(&cols_sql, &[])
                    .await
                    .map_err(|e| CliError::Database(format!("Failed to query columns: {}", e)))?;

                for col_row in col_rows {
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

                // Get primary keys
                let pk_sql = queries::primary_keys_query(
                    DatabaseType::PostgreSQL,
                    &table_name,
                    Some(schema_name),
                );
                let pk_rows = client.query(&pk_sql, &[]).await.map_err(|e| {
                    CliError::Database(format!("Failed to query primary keys: {}", e))
                })?;

                for pk_row in pk_rows {
                    let col_name: String = pk_row.get(0);
                    table.primary_key.push(col_name.clone());

                    // Mark column as primary key
                    if let Some(col) = table.columns.iter_mut().find(|c| c.name == col_name) {
                        col.is_primary_key = true;
                    }
                }

                // Get foreign keys
                let fk_sql = queries::foreign_keys_query(
                    DatabaseType::PostgreSQL,
                    &table_name,
                    Some(schema_name),
                );
                let fk_rows = client.query(&fk_sql, &[]).await.map_err(|e| {
                    CliError::Database(format!("Failed to query foreign keys: {}", e))
                })?;

                let mut fk_map: HashMap<String, ForeignKeyInfo> = HashMap::new();
                for fk_row in fk_rows {
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

                // Get indexes
                let idx_sql = queries::indexes_query(
                    DatabaseType::PostgreSQL,
                    &table_name,
                    Some(schema_name),
                );
                let idx_rows = client
                    .query(&idx_sql, &[])
                    .await
                    .map_err(|e| CliError::Database(format!("Failed to query indexes: {}", e)))?;

                let mut idx_map: HashMap<String, IndexInfo> = HashMap::new();
                for idx_row in idx_rows {
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

                db_schema.tables.push(table);
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

    /// Simple glob-style pattern matching.
    fn matches_pattern(name: &str, pattern: &str) -> bool {
        if pattern == "*" {
            return true;
        }

        if pattern.starts_with('*') && pattern.ends_with('*') {
            let middle = &pattern[1..pattern.len() - 1];
            return name.contains(middle);
        }

        if pattern.starts_with('*') {
            let suffix = &pattern[1..];
            return name.ends_with(suffix);
        }

        if pattern.ends_with('*') {
            let prefix = &pattern[..pattern.len() - 1];
            return name.starts_with(prefix);
        }

        name == pattern
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
