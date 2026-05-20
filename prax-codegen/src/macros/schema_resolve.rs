//! Schema discovery + caching for the read-operation proc-macros.
//!
//! Resolution order (per spec §5):
//! 1. `PRAX_SCHEMA` env var (absolute or relative to `CARGO_MANIFEST_DIR`).
//! 2. Walk up from `CARGO_MANIFEST_DIR` looking for `prax.toml`. Read
//!    `[generator.client].schema` (default `"prax/schema.prax"`),
//!    resolved relative to the `prax.toml` location.
//! 3. Hard error otherwise.
//!
//! All errors are emitted as `syn::Error` pinned at
//! `proc_macro2::Span::call_site()` so callers can `to_compile_error()`.

use std::path::{Path, PathBuf};

/// Resolve the schema path to load for the current proc-macro
/// invocation.
///
/// Used by the cached `resolve_schema` entry point (task 4) and by
/// tests directly.
#[allow(dead_code)]
pub fn resolve_schema_path() -> Result<PathBuf, syn::Error> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").map_err(|_| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            "CARGO_MANIFEST_DIR is not set; proc-macros must be invoked by Cargo.",
        )
    })?;
    let manifest_dir = PathBuf::from(manifest_dir);

    // 1. `PRAX_SCHEMA` env var wins.
    if let Ok(env_path) = std::env::var("PRAX_SCHEMA") {
        let p = PathBuf::from(&env_path);
        let abs = if p.is_absolute() {
            p
        } else {
            manifest_dir.join(p)
        };
        if !abs.exists() {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!(
                    "PRAX_SCHEMA points at '{}' but that file does not exist.",
                    abs.display()
                ),
            ));
        }
        return Ok(abs);
    }

    // 2. Walk up looking for `prax.toml`.
    let mut current: Option<&Path> = Some(&manifest_dir);
    while let Some(dir) = current {
        let candidate = dir.join("prax.toml");
        if candidate.is_file() {
            let raw = std::fs::read_to_string(&candidate).map_err(|e| {
                syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!("failed to read {}: {e}", candidate.display()),
                )
            })?;
            let toml_val: toml::Value = toml::from_str(&raw).map_err(|e| {
                syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!("failed to parse {}: {e}", candidate.display()),
                )
            })?;
            let schema_relative = toml_val
                .get("generator")
                .and_then(|g| g.get("client"))
                .and_then(|c| c.get("schema"))
                .and_then(|s| s.as_str())
                .unwrap_or("prax/schema.prax");
            let resolved = dir.join(schema_relative);
            if !resolved.exists() {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!(
                        "prax.toml at {} declares schema = '{}', but '{}' does not exist.",
                        candidate.display(),
                        schema_relative,
                        resolved.display()
                    ),
                ));
            }
            return Ok(resolved);
        }
        current = dir.parent();
    }

    Err(syn::Error::new(
        proc_macro2::Span::call_site(),
        format!(
            "Could not find a 'prax.toml' in any ancestor of {}. \
             Set PRAX_SCHEMA=path/to/schema.prax or run 'prax init'.",
            manifest_dir.display()
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::Mutex;

    // The schema_resolve_path tests mutate process-global env vars
    // (`CARGO_MANIFEST_DIR`, `PRAX_SCHEMA`). Hold this lock across an
    // entire test body so concurrent tests in the same suite don't
    // race on env state.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Convenience guard that snapshots and restores the env vars the
    /// resolver touches so tests don't leak state.
    struct EnvGuard {
        manifest: Option<String>,
        schema: Option<String>,
    }

    impl EnvGuard {
        fn new() -> Self {
            let g = Self {
                manifest: std::env::var("CARGO_MANIFEST_DIR").ok(),
                schema: std::env::var("PRAX_SCHEMA").ok(),
            };
            // SAFETY: tests holding `ENV_LOCK` are the only writers.
            unsafe {
                std::env::remove_var("PRAX_SCHEMA");
            }
            g
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: tests holding `ENV_LOCK` are the only writers.
            unsafe {
                match &self.manifest {
                    Some(v) => std::env::set_var("CARGO_MANIFEST_DIR", v),
                    None => std::env::remove_var("CARGO_MANIFEST_DIR"),
                }
                match &self.schema {
                    Some(v) => std::env::set_var("PRAX_SCHEMA", v),
                    None => std::env::remove_var("PRAX_SCHEMA"),
                }
            }
        }
    }

    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
    }

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        // Poison-tolerant: a failed test in this suite shouldn't poison
        // the lock and cascade-fail the rest.
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn schema_resolve_prax_schema_absolute_happy_path() {
        let _lock = lock();
        let _g = EnvGuard::new();
        let tmp = tempfile::tempdir().unwrap();
        let abs = tmp.path().join("custom.prax");
        write_file(&abs, "model X { id Int @id @auto }\n");
        // SAFETY: tests hold ENV_LOCK.
        unsafe {
            std::env::set_var("CARGO_MANIFEST_DIR", tmp.path());
            std::env::set_var("PRAX_SCHEMA", &abs);
        }
        let resolved = resolve_schema_path().unwrap();
        assert_eq!(resolved, abs);
    }

    #[test]
    fn schema_resolve_prax_schema_missing_errors() {
        let _lock = lock();
        let _g = EnvGuard::new();
        let tmp = tempfile::tempdir().unwrap();
        // SAFETY: tests hold ENV_LOCK.
        unsafe {
            std::env::set_var("CARGO_MANIFEST_DIR", tmp.path());
            std::env::set_var("PRAX_SCHEMA", "/does/not/exist/schema.prax");
        }
        let err = resolve_schema_path().unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("PRAX_SCHEMA"));
        assert!(msg.contains("does not exist"));
    }

    #[test]
    fn schema_resolve_walks_up_two_levels() {
        let _lock = lock();
        let _g = EnvGuard::new();
        let tmp = tempfile::tempdir().unwrap();
        // Place prax.toml at root; manifest is two levels deep.
        let manifest = tmp.path().join("apps").join("inner");
        std::fs::create_dir_all(&manifest).unwrap();
        write_file(&tmp.path().join("prax.toml"), "");
        write_file(
            &tmp.path().join("prax/schema.prax"),
            "model X { id Int @id @auto }\n",
        );
        // SAFETY: tests hold ENV_LOCK.
        unsafe {
            std::env::set_var("CARGO_MANIFEST_DIR", &manifest);
        }
        let resolved = resolve_schema_path().unwrap();
        assert_eq!(
            resolved.canonicalize().unwrap(),
            tmp.path().join("prax/schema.prax").canonicalize().unwrap()
        );
    }

    #[test]
    fn schema_resolve_explicit_generator_client_schema_override() {
        let _lock = lock();
        let _g = EnvGuard::new();
        let tmp = tempfile::tempdir().unwrap();
        write_file(
            &tmp.path().join("prax.toml"),
            "[generator.client]\nschema = \"alt.prax\"\n",
        );
        write_file(
            &tmp.path().join("alt.prax"),
            "model X { id Int @id @auto }\n",
        );
        // SAFETY: tests hold ENV_LOCK.
        unsafe {
            std::env::set_var("CARGO_MANIFEST_DIR", tmp.path());
        }
        let resolved = resolve_schema_path().unwrap();
        assert_eq!(
            resolved.canonicalize().unwrap(),
            tmp.path().join("alt.prax").canonicalize().unwrap()
        );
    }

    #[test]
    fn schema_resolve_no_prax_toml_errors() {
        let _lock = lock();
        let _g = EnvGuard::new();
        let tmp = tempfile::tempdir().unwrap();
        // SAFETY: tests hold ENV_LOCK.
        unsafe {
            std::env::set_var("CARGO_MANIFEST_DIR", tmp.path());
        }
        let err = resolve_schema_path().unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("prax.toml"));
        assert!(msg.contains("PRAX_SCHEMA"));
    }
}
