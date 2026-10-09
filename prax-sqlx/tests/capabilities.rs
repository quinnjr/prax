//! Compile-only test: `SqlxEngine` advertises row-lock support (the backend
//! is only known at runtime; SQLite rejects locks at `exec`).

use prax_query::capabilities::SupportsRowLocks;
use prax_sqlx::SqlxEngine;

fn assert_row_locks<E: SupportsRowLocks>() {}

#[test]
fn sqlx_engine_impls_row_locks() {
    assert_row_locks::<SqlxEngine>();
}
