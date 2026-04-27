//! Row -> T: FromRow helpers for PgEngine.
//!
//! `tokio_postgres::Row` does not implement `prax_query::row::RowRef` (orphan
//! rule); we wrap each row in `PgRow` first, which does. Task 3 introduced
//! that newtype.

use prax_query::error::{QueryError, QueryResult};
use prax_query::row::FromRow;
use tokio_postgres::Row;

use crate::row_ref::PgRow;

pub fn rows_into<T: FromRow>(rows: Vec<Row>) -> QueryResult<Vec<T>> {
    rows.into_iter()
        .map(|r| {
            let r = PgRow::from(r);
            T::from_row(&r).map_err(|e| QueryError::deserialization(e.to_string()))
        })
        .collect()
}

pub fn row_into<T: FromRow>(row: Row) -> QueryResult<T> {
    let row = PgRow::from(row);
    T::from_row(&row).map_err(|e| QueryError::deserialization(e.to_string()))
}
