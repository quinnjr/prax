//! Per-dialect DDL emission for @generated columns.
//!
//! Tests that each SQL generator emits the correct GENERATED AS syntax for
//! stored and virtual computed columns, including Postgres's fallback to STORED
//! (with a warning) when @virtual is requested.

use prax_migrate::{
    DuckDbSqlGenerator, FieldDiff, ModelDiff, MssqlGenerator, MySqlGenerator, PostgresSqlGenerator,
    SchemaDiff, SqliteGenerator,
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
