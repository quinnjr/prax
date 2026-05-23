//! Per-dialect DDL emission for @generated columns.
//!
//! Tests that each SQL generator emits the correct GENERATED AS syntax for
//! stored and virtual computed columns, including Postgres's fallback to STORED
//! (with a warning) when @virtual is requested.
//!
//! Also tests that the CQL generator rejects @generated columns (via warning)
//! and silently skips aggregate fields.

use prax_migrate::{
    CqlFieldDiff, CqlMigrationGenerator, CqlSchemaDiff, CqlTableDiff, DuckDbSqlGenerator,
    FieldDiff, ModelDiff, MssqlGenerator, MySqlGenerator, PostgresSqlGenerator, SchemaDiff,
    SqliteGenerator,
};
use prax_schema::ast::GeneratedAttribute;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_id_field() -> FieldDiff {
    FieldDiff {
        name: "id".to_string(),
        column_name: "id".to_string(),
        sql_type: "BIGINT".to_string(),
        nullable: false,
        default: None,
        is_primary_key: true,
        is_auto_increment: true,
        is_unique: false,
        vector: None,
        enum_name: None,
        generated: None,
    }
}

fn make_text_field(name: &str) -> FieldDiff {
    FieldDiff {
        name: name.to_string(),
        column_name: name.to_string(),
        sql_type: "TEXT".to_string(),
        nullable: false,
        default: None,
        is_primary_key: false,
        is_auto_increment: false,
        is_unique: false,
        vector: None,
        enum_name: None,
        generated: None,
    }
}

fn make_generated_field(name: &str, expr: &str, stored: bool) -> FieldDiff {
    FieldDiff {
        name: name.to_string(),
        column_name: name.to_string(),
        sql_type: "TEXT".to_string(),
        nullable: false,
        default: None,
        is_primary_key: false,
        is_auto_increment: false,
        is_unique: false,
        vector: None,
        enum_name: None,
        generated: Some(GeneratedAttribute {
            expression: expr.to_string(),
            stored,
        }),
    }
}

fn make_model(table_name: &str, fields: Vec<FieldDiff>) -> ModelDiff {
    ModelDiff {
        name: "User".to_string(),
        table_name: table_name.to_string(),
        fields,
        primary_key: vec!["id".to_string()],
        indexes: Vec::new(),
        unique_constraints: Vec::new(),
        foreign_keys: Vec::new(),
    }
}

const EXPR: &str = "first_name || ' ' || last_name";

fn fixture_diff_stored() -> SchemaDiff {
    let mut diff = SchemaDiff::default();
    diff.create_models.push(make_model(
        "users",
        vec![
            make_id_field(),
            make_text_field("first_name"),
            make_text_field("last_name"),
            make_generated_field("full_name", EXPR, true),
        ],
    ));
    diff
}

fn fixture_diff_virtual() -> SchemaDiff {
    let mut diff = SchemaDiff::default();
    diff.create_models.push(make_model(
        "users",
        vec![
            make_id_field(),
            make_text_field("first_name"),
            make_text_field("last_name"),
            make_generated_field("full_name", EXPR, false),
        ],
    ));
    diff
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn postgres_emits_stored_generated_column() {
    let migration = PostgresSqlGenerator.generate(&fixture_diff_stored());
    let sql = &migration.up;
    assert!(
        sql.contains(&format!("GENERATED ALWAYS AS ({EXPR}) STORED")),
        "missing PG GENERATED ... STORED in:\n{sql}"
    );
    assert!(
        migration.warnings.is_empty(),
        "unexpected warnings for @stored: {:?}",
        migration.warnings
    );
}

#[test]
fn postgres_warns_on_virtual_and_emits_stored() {
    let migration = PostgresSqlGenerator.generate(&fixture_diff_virtual());
    let sql = &migration.up;
    // Postgres has no native @virtual support; we fall back to STORED.
    assert!(
        sql.contains("STORED"),
        "expected STORED fallback for @virtual in:\n{sql}"
    );
    // A warning must be present.
    assert!(
        !migration.warnings.is_empty(),
        "expected a warning for @virtual on Postgres"
    );
    assert!(
        migration.warnings.iter().any(|w| w.contains("virtual")),
        "warning should mention 'virtual': {:?}",
        migration.warnings
    );
}

#[test]
fn mysql_emits_stored_generated_column() {
    let migration = MySqlGenerator.generate(&fixture_diff_stored());
    let sql = &migration.up;
    assert!(
        sql.contains(&format!("AS ({EXPR}) STORED")),
        "missing MySQL AS ... STORED in:\n{sql}"
    );
}

#[test]
fn mysql_emits_virtual_generated_column() {
    let migration = MySqlGenerator.generate(&fixture_diff_virtual());
    let sql = &migration.up;
    assert!(
        sql.contains(&format!("AS ({EXPR}) VIRTUAL")),
        "missing MySQL AS ... VIRTUAL in:\n{sql}"
    );
}

#[test]
fn sqlite_emits_stored_generated_column() {
    let migration = SqliteGenerator.generate(&fixture_diff_stored());
    let sql = &migration.up;
    assert!(
        sql.contains(&format!("GENERATED ALWAYS AS ({EXPR}) STORED")),
        "missing SQLite GENERATED ... STORED in:\n{sql}"
    );
}

#[test]
fn sqlite_emits_virtual_generated_column() {
    let migration = SqliteGenerator.generate(&fixture_diff_virtual());
    let sql = &migration.up;
    assert!(
        sql.contains(&format!("GENERATED ALWAYS AS ({EXPR}) VIRTUAL")),
        "missing SQLite GENERATED ... VIRTUAL in:\n{sql}"
    );
}

#[test]
fn mssql_emits_persisted_generated_column() {
    let migration = MssqlGenerator.generate(&fixture_diff_stored());
    let sql = &migration.up;
    assert!(
        sql.contains(&format!("AS ({EXPR}) PERSISTED")),
        "missing MSSQL AS ... PERSISTED in:\n{sql}"
    );
}

#[test]
fn mssql_emits_virtual_generated_column() {
    let migration = MssqlGenerator.generate(&fixture_diff_virtual());
    let sql = &migration.up;
    // MSSQL virtual: AS (<expr>) with no PERSISTED keyword.
    assert!(
        sql.contains(&format!("AS ({EXPR})")),
        "missing MSSQL AS (...) in:\n{sql}"
    );
    // Must NOT contain PERSISTED.
    assert!(
        !sql.contains("PERSISTED"),
        "MSSQL @virtual should not contain PERSISTED in:\n{sql}"
    );
}

#[test]
fn duckdb_emits_stored_generated_column() {
    let migration = DuckDbSqlGenerator.generate(&fixture_diff_stored());
    let sql = &migration.up;
    assert!(
        sql.contains(&format!("GENERATED ALWAYS AS ({EXPR}) STORED")),
        "missing DuckDB GENERATED ... STORED in:\n{sql}"
    );
}

#[test]
fn duckdb_emits_virtual_generated_column() {
    let migration = DuckDbSqlGenerator.generate(&fixture_diff_virtual());
    let sql = &migration.up;
    assert!(
        sql.contains(&format!("GENERATED ALWAYS AS ({EXPR}) VIRTUAL")),
        "missing DuckDB GENERATED ... VIRTUAL in:\n{sql}"
    );
}

// ---------------------------------------------------------------------------
// CQL helpers and tests
// ---------------------------------------------------------------------------

fn cql_simple_field(name: &str, cql_type: &str) -> CqlFieldDiff {
    CqlFieldDiff {
        name: name.into(),
        cql_type: cql_type.into(),
        is_static: false,
        generated: None,
        is_aggregate: false,
    }
}

fn cql_generated_field(name: &str) -> CqlFieldDiff {
    CqlFieldDiff {
        name: name.into(),
        cql_type: "text".into(),
        is_static: false,
        generated: Some(GeneratedAttribute {
            expression: EXPR.to_string(),
            stored: true,
        }),
        is_aggregate: false,
    }
}

fn cql_aggregate_field(name: &str) -> CqlFieldDiff {
    CqlFieldDiff {
        name: name.into(),
        cql_type: "counter".into(),
        is_static: false,
        generated: None,
        is_aggregate: true,
    }
}

fn cql_fixture_diff_with_generated() -> CqlSchemaDiff {
    CqlSchemaDiff {
        create_tables: vec![CqlTableDiff {
            name: "users".into(),
            fields: vec![
                cql_simple_field("id", "uuid"),
                cql_simple_field("first_name", "text"),
                cql_simple_field("last_name", "text"),
                cql_generated_field("full_name"),
            ],
            partition_keys: vec!["id".into()],
            clustering_keys: vec![],
            compaction: None,
            default_ttl: None,
        }],
        ..CqlSchemaDiff::default()
    }
}

fn cql_fixture_diff_aggregate_only() -> CqlSchemaDiff {
    CqlSchemaDiff {
        create_tables: vec![CqlTableDiff {
            name: "posts".into(),
            fields: vec![
                cql_simple_field("id", "uuid"),
                cql_simple_field("title", "text"),
                cql_aggregate_field("post_count"),
            ],
            partition_keys: vec!["id".into()],
            clustering_keys: vec![],
            compaction: None,
            default_ttl: None,
        }],
        ..CqlSchemaDiff::default()
    }
}

/// CQL does not support @generated columns. The generator must emit a warning
/// and omit the column from DDL rather than producing broken CQL.
#[test]
fn cql_warns_on_generated_column_and_omits_it() {
    let migration = CqlMigrationGenerator::new().generate(&cql_fixture_diff_with_generated());

    // A warning must be present mentioning the @generated field.
    assert!(
        !migration.warnings.is_empty(),
        "expected a warning for @generated on CQL engine"
    );
    assert!(
        migration
            .warnings
            .iter()
            .any(|w| w.contains("full_name") && w.contains("@generated")),
        "warning should mention 'full_name' and '@generated': {:?}",
        migration.warnings
    );

    // The column must NOT appear in the generated DDL.
    assert!(
        !migration.up.contains("full_name"),
        "generated column 'full_name' should be omitted from CQL DDL:\n{}",
        migration.up
    );

    // The other columns (and the table itself) should still be present.
    assert!(
        migration.up.contains("CREATE TABLE"),
        "table DDL should still be emitted:\n{}",
        migration.up
    );
    assert!(
        migration.up.contains("first_name"),
        "non-generated columns should still appear in DDL:\n{}",
        migration.up
    );
}

/// Aggregate fields (`@count`, `@sum`, etc.) have no DDL representation in CQL.
/// The generator must silently skip them without any warning.
#[test]
fn cql_silently_skips_aggregate_fields() {
    let migration = CqlMigrationGenerator::new().generate(&cql_fixture_diff_aggregate_only());

    // No warnings should be emitted for aggregate fields.
    assert!(
        migration.warnings.is_empty(),
        "no warnings expected for aggregate fields: {:?}",
        migration.warnings
    );

    // The aggregate column must NOT appear in the generated DDL.
    assert!(
        !migration.up.contains("post_count"),
        "aggregate field 'post_count' should be silently omitted from CQL DDL:\n{}",
        migration.up
    );

    // The table and non-aggregate columns should still be present.
    assert!(
        migration.up.contains("CREATE TABLE"),
        "table DDL should still be emitted:\n{}",
        migration.up
    );
    assert!(
        migration.up.contains("title"),
        "non-aggregate columns should still appear in DDL:\n{}",
        migration.up
    );
}
