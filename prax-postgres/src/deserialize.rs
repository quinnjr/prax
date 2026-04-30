//! Row -> T: FromRow helpers for PgEngine.
//!
//! `tokio_postgres::Row` does not implement `prax_query::row::RowRef` (orphan
//! rule); we wrap each row in `PgRow` first, which does.

use prax_query::error::{QueryError, QueryResult};
use prax_query::row::FromRow;
use tokio_postgres::Row;

use crate::row_ref::PgRow;

/// Decode a batch of driver rows into typed models.
///
/// # Short-circuit on decode error
///
/// Uses `Result<Vec<T>, _>::collect`, which returns the first decode
/// error and discards every successfully-decoded row before it. A
/// row-level type mismatch therefore aborts the whole batch rather
/// than returning partial results. Callers that want per-row
/// recovery should manually iterate `rows` and handle each
/// `T::from_row` result.
pub fn rows_into<T: FromRow>(rows: Vec<Row>) -> QueryResult<Vec<T>> {
    rows.into_iter()
        .map(|r| {
            let r = PgRow::from(r);
            T::from_row(&r).map_err(|e| {
                let msg = e.to_string();
                QueryError::deserialization(msg).with_source(e)
            })
        })
        .collect()
}

/// Decode a single driver row into a typed model.
///
/// Returns a deserialization error if any column fails to decode or
/// if the FromRow impl returns an error. No partial-result fallback.
pub fn row_into<T: FromRow>(row: Row) -> QueryResult<T> {
    let row = PgRow::from(row);
    T::from_row(&row).map_err(|e| {
        let msg = e.to_string();
        QueryError::deserialization(msg).with_source(e)
    })
}
