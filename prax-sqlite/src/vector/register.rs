//! Auto-registration of the sqlite-vector-rs extension on rusqlite connections.

use crate::vector::error::{VectorError, VectorResult};

/// Register the sqlite-vector-rs extension on a rusqlite Connection.
///
/// Called automatically by `SqlitePool::open_connection` when the `vector`
/// feature is enabled. Users only need to call this manually if they
/// construct raw rusqlite connections outside the pool.
pub fn register_vector_extension(conn: &rusqlite::Connection) -> VectorResult<()> {
    sqlite_vector_rs::register(conn).map_err(|e| VectorError::Driver(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // These tests require the sqlite-vector-rs cdylib (libsqlite_vector_rs.so/
    // .dylib/.dll) to be available on disk — the extension is loaded via
    // dlopen at runtime, not compiled in. When prax-sqlite depends on
    // sqlite-vector-rs as a library, only the rlib is built; the cdylib must
    // be produced separately (e.g. `cargo build -p sqlite-vector-rs`) and
    // made discoverable via SQLITE_VECTOR_RS_LIB or placed next to the test
    // binary. These tests are marked `#[ignore]` so they run only when the
    // environment is set up, via `cargo test -- --ignored`.

    #[test]
    #[ignore = "requires libsqlite_vector_rs.so on disk or SQLITE_VECTOR_RS_LIB set"]
    fn test_register_on_fresh_connection() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let result = register_vector_extension(&conn);
        assert!(result.is_ok(), "register failed: {:?}", result.err());
    }

    #[test]
    #[ignore = "requires libsqlite_vector_rs.so on disk or SQLITE_VECTOR_RS_LIB set"]
    fn test_vector_from_json_available_after_register() {
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
}
