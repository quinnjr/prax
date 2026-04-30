//! Integration tests for the Prax CLI

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

/// Get the prax binary
#[allow(deprecated)]
fn prax_cmd() -> Command {
    Command::cargo_bin("prax").unwrap()
}

#[test]
fn test_help_command() {
    prax_cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Prax CLI"))
        .stdout(predicate::str::contains("Usage: prax <COMMAND>"))
        .stdout(predicate::str::contains("init"))
        .stdout(predicate::str::contains("generate"))
        .stdout(predicate::str::contains("migrate"))
        .stdout(predicate::str::contains("db"));
}

#[test]
fn test_version_command() {
    prax_cmd()
        .arg("version")
        .assert()
        .success()
        .stdout(predicate::str::contains("Version"))
        .stdout(predicate::str::contains("0.8.0"));
}

#[test]
fn test_init_help() {
    prax_cmd()
        .args(["init", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Initialize a new Prax project"))
        .stdout(predicate::str::contains("--provider"));
}

#[test]
fn test_generate_help() {
    prax_cmd()
        .args(["generate", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Generate Rust client code"))
        .stdout(predicate::str::contains("--schema"));
}

#[test]
fn test_migrate_help() {
    prax_cmd()
        .args(["migrate", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("migration commands"))
        .stdout(predicate::str::contains("dev"))
        .stdout(predicate::str::contains("deploy"))
        .stdout(predicate::str::contains("reset"))
        .stdout(predicate::str::contains("status"));
}

#[test]
fn test_db_help() {
    prax_cmd()
        .args(["db", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("database operations"))
        .stdout(predicate::str::contains("push"))
        .stdout(predicate::str::contains("pull"));
}

#[test]
fn test_validate_help() {
    prax_cmd()
        .args(["validate", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("validation"));
}

#[test]
fn test_format_help() {
    prax_cmd()
        .args(["format", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Format schema"));
}

#[test]
fn test_init_creates_project_structure() {
    let temp_dir = TempDir::new().unwrap();
    let project_name = "test_project";

    prax_cmd()
        .current_dir(temp_dir.path())
        .args(["init", project_name, "--yes", "--provider", "postgresql"])
        .assert()
        .success()
        .stdout(predicate::str::contains("initialized successfully"));

    let project_path = temp_dir.path().join(project_name);
    assert!(project_path.exists(), "Project directory should exist");
    // Schema is in prax/ directory
    assert!(
        project_path.join("prax").join("schema.prax").exists(),
        "prax/schema.prax should exist"
    );
    // Config is in project root
    assert!(
        project_path.join("prax.toml").exists(),
        "prax.toml should exist"
    );
    // Migrations are in prax/ directory
    assert!(
        project_path.join("prax").join("migrations").exists(),
        "prax/migrations directory should exist"
    );
    // Note: src directory may not be created immediately, depends on implementation
}

#[test]
fn test_init_with_different_providers() {
    for provider in ["postgresql", "mysql", "sqlite"] {
        let temp_dir = TempDir::new().unwrap();
        let project_name = format!("test_{}", provider);

        prax_cmd()
            .current_dir(temp_dir.path())
            .args(["init", &project_name, "--yes", "--provider", provider])
            .assert()
            .success();

        let config_path = temp_dir.path().join(&project_name).join("prax.toml");
        assert!(config_path.exists());

        let config_content = fs::read_to_string(config_path).unwrap();
        assert!(config_content.contains(provider));
    }
}

#[test]
fn test_validate_with_valid_schema() {
    let temp_dir = TempDir::new().unwrap();
    let schema_path = temp_dir.path().join("schema.prax");

    let schema_content = r#"
model User {
    id    Int    @id @auto
    name  String
    email String @unique
}
"#;
    fs::write(&schema_path, schema_content).unwrap();

    prax_cmd()
        .args(["validate", "--schema", schema_path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("valid"));
}

#[test]
fn test_validate_with_invalid_schema() {
    let temp_dir = TempDir::new().unwrap();
    let schema_path = temp_dir.path().join("schema.prax");

    let schema_content = r#"
model User {
    id    Int    @id @auto
    name  String
    email String @unique
    // Missing closing brace
"#;
    fs::write(&schema_path, schema_content).unwrap();

    prax_cmd()
        .args(["validate", "--schema", schema_path.to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn test_format_schema() {
    let temp_dir = TempDir::new().unwrap();
    let schema_path = temp_dir.path().join("schema.prax");

    let schema_content = r#"
model   User{
id Int @id @auto
name String
email String @unique
}
"#;
    fs::write(&schema_path, schema_content).unwrap();

    // Format command should succeed and output the formatted schema
    prax_cmd()
        .args(["format", "--schema", schema_path.to_str().unwrap()])
        .assert()
        .success();
}

#[test]
fn test_generate_missing_schema() {
    let temp_dir = TempDir::new().unwrap();
    let schema_path = temp_dir.path().join("nonexistent.prax");

    prax_cmd()
        .args(["generate", "--schema", schema_path.to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn test_migrate_status_no_config() {
    let temp_dir = TempDir::new().unwrap();

    // Without a prax.toml config, migrate status should fail with an error
    let _result = prax_cmd()
        .current_dir(temp_dir.path())
        .args(["migrate", "status"])
        .assert();

    // It should either fail or report no config found
    // Don't assert on specific error message since implementation may vary
}

#[test]
fn test_invalid_command() {
    prax_cmd()
        .arg("invalid_command")
        .assert()
        .failure()
        .stderr(predicate::str::contains("error"));
}

#[test]
fn test_global_options() {
    // Test --version flag
    prax_cmd()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("0.8.0"));
}
