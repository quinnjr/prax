//! Compile-fail: `DuckDbEngine` does not implement `SupportsRowLocks`, so it
//! cannot satisfy the bound that gates `.lock(..)` / `.for_update()` on the
//! find operations.

fn requires<E: prax_query::capabilities::SupportsRowLocks>() {}

fn main() {
    requires::<prax_duckdb::DuckDbEngine>();
}
