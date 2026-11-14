//! Loading the sqlite-vector-rs extension into rusqlite connections.
//!
//! sqlite-vector-rs ships as a SQLite *loadable extension* (a cdylib:
//! `libsqlite_vector_rs.so` / `.dylib` / `sqlite_vector_rs.dll`). Since
//! 0.13.1 prax-sqlite loads it itself through rusqlite's `load_extension`
//! instead of depending on the `sqlite-vector-rs` crate, whose library API
//! pins rusqlite < 0.40 (and so an unpatched bundled SQLite,
//! RUSTSEC-2026-0459). Build the cdylib from
//! <https://github.com/quinnjr/sqlite-vector-rs> (`cargo build --release`).

use std::path::{Path, PathBuf};

use crate::vector::error::{VectorError, VectorResult};

/// Environment variable naming the extension library (with or without
/// its platform suffix).
pub const VECTOR_LIB_ENV: &str = "SQLITE_VECTOR_RS_LIB";

fn lib_stem() -> &'static str {
    if cfg!(target_os = "windows") {
        "sqlite_vector_rs"
    } else {
        "libsqlite_vector_rs"
    }
}

fn lib_suffix() -> &'static str {
    if cfg!(target_os = "macos") {
        ".dylib"
    } else if cfg!(target_os = "windows") {
        ".dll"
    } else {
        ".so"
    }
}

/// `base` itself if it exists, else `base` + the platform suffix if that
/// exists. SQLite's loader adds the suffix itself, so either form loads.
fn existing(base: &Path) -> Option<PathBuf> {
    if base.is_file() {
        return Some(base.to_path_buf());
    }
    let mut with_suffix = base.as_os_str().to_owned();
    with_suffix.push(lib_suffix());
    let with_suffix = PathBuf::from(with_suffix);
    if with_suffix.is_file() {
        return Some(with_suffix);
    }
    None
}

/// Locate the sqlite-vector-rs extension library.
///
/// Search order (the same as `sqlite_vector_rs::register`):
/// 1. `$SQLITE_VECTOR_RS_LIB`;
/// 2. next to the current executable;
/// 3. `target/debug` and `target/release` under the working directory.
pub fn find_vector_extension() -> Option<PathBuf> {
    if let Some(val) = std::env::var_os(VECTOR_LIB_ENV)
        && let Some(path) = existing(Path::new(&val))
    {
        return Some(path);
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
        && let Some(path) = existing(&dir.join(lib_stem()))
    {
        return Some(path);
    }
    for profile in ["debug", "release"] {
        if let Some(path) = existing(&Path::new("target").join(profile).join(lib_stem())) {
            return Some(path);
        }
    }
    None
}

/// Load the sqlite-vector-rs extension into a rusqlite connection.
///
/// Called automatically by `SqlitePool` for every new connection when the
/// `vector` feature is enabled. Call it yourself only for connections you
/// open outside the pool. Extension loading is enabled just for the load
/// and disabled again afterwards, so SQL can't call `load_extension()`.
pub fn register_vector_extension(conn: &rusqlite::Connection) -> VectorResult<()> {
    let path = find_vector_extension().ok_or_else(|| {
        VectorError::Driver(format!(
            "could not find the sqlite-vector-rs extension library; build it \
             (https://github.com/quinnjr/sqlite-vector-rs) and set {VECTOR_LIB_ENV}"
        ))
    })?;
    load_vector_extension(conn, &path)
}

/// Load the sqlite-vector-rs extension from an explicit library path.
pub fn load_vector_extension(conn: &rusqlite::Connection, path: &Path) -> VectorResult<()> {
    // SAFETY: loading a native extension runs its code in-process; the
    // caller chooses the library (via the path or SQLITE_VECTOR_RS_LIB).
    // The guard re-disables extension loading on every exit path.
    #[allow(unsafe_code)]
    let guard = unsafe { rusqlite::LoadExtensionGuard::new(conn) }
        .map_err(|e| VectorError::Driver(e.to_string()))?;
    #[allow(unsafe_code)]
    let result = unsafe { conn.load_extension(path, None::<&str>) };
    drop(guard);
    result.map_err(|e| VectorError::Driver(format!("{}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    // These tests require the sqlite-vector-rs cdylib (libsqlite_vector_rs.so/
    // .dylib/.dll) to be discoverable on disk: the extension is loaded at
    // runtime, not compiled into the test binary. CI builds it and sets
    // `SQLITE_VECTOR_RS_LIB`.
    //
    // CI runs `cargo test --workspace --all-features -- --include-ignored`,
    // which both enables every feature and forces ignored tests to run, so
    // neither `#[ignore]` nor a Cargo feature gate suffices. Skip at runtime
    // when the env var is not set so the tests no-op in environments that
    // didn't provision the cdylib, and exercise the real registration when
    // the loader can find it.
    fn vector_lib_available() -> bool {
        std::env::var_os("SQLITE_VECTOR_RS_LIB").is_some()
    }

    #[test]
    fn test_register_on_fresh_connection() {
        if !vector_lib_available() {
            eprintln!("skipping: SQLITE_VECTOR_RS_LIB not set");
            return;
        }
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let result = register_vector_extension(&conn);
        assert!(result.is_ok(), "register failed: {:?}", result.err());
    }

    #[test]
    fn test_vector_from_json_available_after_register() {
        if !vector_lib_available() {
            eprintln!("skipping: SQLITE_VECTOR_RS_LIB not set");
            return;
        }
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        register_vector_extension(&conn).unwrap();
        // If the function is registered, this query should succeed.
        let dims: i64 = conn
            .query_row(
                "SELECT vector_dims(vector_from_json('[1.0, 2.0, 3.0]', 'float4'), 'float4')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(dims, 3);
    }

    #[test]
    fn missing_library_is_a_driver_error() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let err = load_vector_extension(&conn, Path::new("/nonexistent/libnope.so")).unwrap_err();
        assert!(matches!(err, VectorError::Driver(_)), "{err:?}");
    }
}
