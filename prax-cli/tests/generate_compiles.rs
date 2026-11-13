//! `prax generate` output must compile against `prax-query`, and be
//! clippy-clean under `-D warnings`.
//!
//! The string assertions in `cli_tests.rs` cannot catch a generated field
//! whose type lacks a runtime trait impl. That is how `String[]` fields
//! shipped broken: the generator emitted `Vec<String>` fields, but nothing
//! implemented `FromColumn` / `ToFilterValue` for `Vec<String>`, so every
//! consumer crate failed with E0277.
//!
//! This test generates a client for a schema with scalar-list fields into a
//! throwaway crate that depends on the workspace `prax-query` by path, and
//! runs `cargo check` on it. It reuses the workspace `Cargo.lock` and runs
//! offline, so it only needs the registry sources the workspace build has
//! already fetched.
//!
//! It then runs `cargo clippy -- -D warnings` on the same crate. Consumers
//! gate CI on clippy over the generated tree, so a lint in generated code
//! (for example `result_large_err` on the client's `query_raw` /
//! `execute_raw` / `transaction`, flagged from clippy 1.99) fails their
//! build. When clippy is not installed the clippy step is skipped, except
//! on CI (`CI` set), where it is required.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use assert_cmd::cargo::CommandCargoExt;
use tempfile::TempDir;

const SCHEMA: &str = r#"
datasource db {
  provider = "postgresql"
  url = "postgres://localhost/test"
}

enum GrantKind {
  Firm
  User
}

/// Same shape as the lexmata `access_grants` / `firm_access_policies` /
/// `firm_roles` models that first hit the missing impls.
model AccessGrant {
  id             String    @id @default(uuid())
  kind           GrantKind
  scopes         String[]
  allowed_scopes String[]  @default(dbgenerated("ARRAY[]::TEXT[]"))
  labels         String[]?
  note           String?

  @@map("access_grants")
}
"#;

const LIB_RS: &str = r#"
#![deny(warnings)]

pub mod generated;

use generated::AccessGrant;
use prax_query::filter::FilterValue;
use prax_query::traits::ModelWithPk;

/// Exercises the generated client surface for a model with list fields.
pub fn find_grants<E: prax_query::QueryEngine>(client: &generated::PraxClient<E>) {
    let _ = client.access_grant().find_many();
}

/// Reads list-field values back out of a model instance.
pub fn scope_values(grant: &AccessGrant) -> Option<FilterValue> {
    grant.get_column_value("scopes")
}
"#;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("prax-cli lives inside the workspace")
        .to_path_buf()
}

#[test]
fn generated_client_with_string_list_fields_compiles() {
    let root = workspace_root();
    let temp = TempDir::new().unwrap();
    let krate = temp.path().join("generated_check");
    let src = krate.join("src");
    fs::create_dir_all(&src).unwrap();

    let schema_path = temp.path().join("schema.prax");
    fs::write(&schema_path, SCHEMA).unwrap();

    #[allow(deprecated)]
    let status = Command::cargo_bin("prax")
        .unwrap()
        .args(["generate", "--schema"])
        .arg(&schema_path)
        .arg("--output")
        .arg(src.join("generated"))
        .status()
        .expect("run prax generate");
    assert!(status.success(), "prax generate failed");

    let generated = fs::read_to_string(src.join("generated").join("access_grant.rs")).unwrap();
    assert!(generated.contains("pub scopes: Vec<String>"), "{generated}");
    assert!(
        generated.contains("pub labels: Option<Vec<String>>"),
        "{generated}"
    );

    let prax_query = root.join("prax-query");
    fs::write(
        krate.join("Cargo.toml"),
        format!(
            r#"[package]
name = "prax-generated-compile-check"
version = "0.0.0"
edition = "2024"
publish = false

[dependencies]
prax-query = {{ path = {prax_query:?} }}
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
chrono = {{ version = "0.4", features = ["serde"] }}
uuid = {{ version = "1", features = ["serde"] }}
rust_decimal = {{ version = "1", features = ["serde"] }}

[workspace]
"#
        ),
    )
    .unwrap();
    fs::write(src.join("lib.rs"), LIB_RS).unwrap();
    fs::copy(root.join("Cargo.lock"), krate.join("Cargo.lock")).unwrap();

    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let target_dir = root.join("target").join("generated-compile-check");
    let output = Command::new(&cargo)
        .args(["check", "--offline", "--quiet", "--manifest-path"])
        .arg(krate.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", &target_dir)
        .output()
        .expect("run cargo check");
    assert!(
        output.status.success(),
        "generated client failed to compile:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let clippy_available = Command::new(&cargo)
        .args(["clippy", "--version"])
        .output()
        .is_ok_and(|o| o.status.success());
    if !clippy_available {
        assert!(
            std::env::var_os("CI").is_none(),
            "clippy is required on CI to lint the generated client"
        );
        eprintln!("skipping clippy on the generated client: cargo clippy is not installed");
        return;
    }
    let output = Command::new(&cargo)
        .args(["clippy", "--offline", "--quiet", "--manifest-path"])
        .arg(krate.join("Cargo.toml"))
        .args(["--", "-D", "warnings"])
        .env("CARGO_TARGET_DIR", &target_dir)
        .output()
        .expect("run cargo clippy");
    assert!(
        output.status.success(),
        "generated client is not clippy-clean (-D warnings):\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
