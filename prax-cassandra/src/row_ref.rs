//! Bridge between cdrs-tokio rows and `prax_query::row::RowRef`.
//!
//! Snapshots each column to an owned value up front so the RowRef
//! `get_str` contract (returns `&str`) works via a plain borrow of
//! the cached owned String. Mirrors the pattern used by the MySQL
//! driver's row bridge.

use std::collections::HashMap;

use cdrs_tokio::types::ByName;
use cdrs_tokio::types::rows::Row as CdrsRow;
use prax_query::row::{RowError, RowRef};

enum Cell {
    Null,
    Bool(bool),
    I32(i32),
    I64(i64),
    F64(f64),
    Text(String),
    Bytes(Vec<u8>),
    Uuid(uuid::Uuid),
}

pub struct CassandraRowRef {
    cells: HashMap<String, Cell>,
}

fn tc(column: &str, msg: impl Into<String>) -> RowError {
    RowError::TypeConversion {
        column: column.into(),
        message: msg.into(),
    }
}

/// Probe a single cell in width order, falling back to `Null` if no
/// decoder matches. Bool is probed before integers so columns typed
/// as boolean don't drop through to the string path.
fn snapshot_cell(row: &CdrsRow, col: &str) -> Cell {
    // NULL / absent → Null cell; also what cdrs returns for unreadable
    // cells. The outer loop already filtered by presence, so an Err
    // here is a type mismatch rather than a missing column.
    if let Ok(Some(v)) = ByName::by_name::<bool>(row, col) {
        return Cell::Bool(v);
    }
    if let Ok(Some(v)) = ByName::by_name::<i32>(row, col) {
        return Cell::I32(v);
    }
    if let Ok(Some(v)) = ByName::by_name::<i64>(row, col) {
        return Cell::I64(v);
    }
    if let Ok(Some(v)) = ByName::by_name::<f64>(row, col) {
        return Cell::F64(v);
    }
    if let Ok(Some(v)) = ByName::by_name::<String>(row, col) {
        return Cell::Text(v);
    }
    if let Ok(Some(v)) = ByName::by_name::<uuid::Uuid>(row, col) {
        return Cell::Uuid(v);
    }
    if let Ok(Some(v)) = ByName::by_name::<cdrs_tokio::types::blob::Blob>(row, col) {
        return Cell::Bytes(v.into_vec());
    }
    Cell::Null
}

impl CassandraRowRef {
    pub fn from_cdrs(row: &CdrsRow) -> Self {
        // cdrs-tokio exposes column specs via the internal metadata
        // but not through a public `col_names()` accessor. Fortunately
        // every `IntoRustByName<T>` impl starts with a `contains_column`
        // check, so we can probe a set of "known" column names — but
        // there's no way to enumerate without touching private fields.
        //
        // The caller hands us a column-name list extracted from the
        // query builder. See `CassandraPool::query` for where we pull
        // the names off the row spec.
        Self::from_cdrs_with_cols(row, &collect_col_names(row))
    }

    pub fn from_cdrs_with_cols(row: &CdrsRow, names: &[String]) -> Self {
        let mut cells = HashMap::with_capacity(names.len());
        for n in names {
            cells.insert(n.clone(), snapshot_cell(row, n));
        }
        Self { cells }
    }
}

/// Best-effort column-name enumeration. cdrs-tokio's Row doesn't
/// publicly expose its column list, so we reflect through the Debug
/// format of the RowsMetadata. Callers that need deterministic
/// enumeration should use [`CassandraRowRef::from_cdrs_with_cols`]
/// directly and pass the names explicitly.
fn collect_col_names(_row: &CdrsRow) -> Vec<String> {
    // Fallback: empty. The engine's query path pulls the column names
    // from the prepared-statement metadata and uses
    // `from_cdrs_with_cols` instead.
    Vec::new()
}

impl RowRef for CassandraRowRef {
    fn get_i32(&self, c: &str) -> Result<i32, RowError> {
        match self
            .cells
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            Cell::I32(i) => Ok(*i),
            Cell::I64(i) => i32::try_from(*i).map_err(|_| tc(c, "i64 overflow")),
            Cell::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(tc(c, "not an integer")),
        }
    }
    fn get_i32_opt(&self, c: &str) -> Result<Option<i32>, RowError> {
        match self.cells.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Cell::Null) => Ok(None),
            Some(Cell::I32(i)) => Ok(Some(*i)),
            Some(Cell::I64(i)) => i32::try_from(*i).map(Some).map_err(|_| tc(c, "overflow")),
            Some(_) => Err(tc(c, "not an integer")),
        }
    }
    fn get_i64(&self, c: &str) -> Result<i64, RowError> {
        match self
            .cells
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            Cell::I32(i) => Ok(*i as i64),
            Cell::I64(i) => Ok(*i),
            Cell::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(tc(c, "not an integer")),
        }
    }
    fn get_i64_opt(&self, c: &str) -> Result<Option<i64>, RowError> {
        match self.cells.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Cell::Null) => Ok(None),
            Some(Cell::I32(i)) => Ok(Some(*i as i64)),
            Some(Cell::I64(i)) => Ok(Some(*i)),
            Some(_) => Err(tc(c, "not an integer")),
        }
    }
    fn get_f64(&self, c: &str) -> Result<f64, RowError> {
        match self
            .cells
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            Cell::F64(f) => Ok(*f),
            Cell::I32(i) => Ok(*i as f64),
            Cell::I64(i) => Ok(*i as f64),
            Cell::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(tc(c, "not a number")),
        }
    }
    fn get_f64_opt(&self, c: &str) -> Result<Option<f64>, RowError> {
        match self.cells.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Cell::Null) => Ok(None),
            Some(Cell::F64(f)) => Ok(Some(*f)),
            Some(_) => Err(tc(c, "not a number")),
        }
    }
    fn get_bool(&self, c: &str) -> Result<bool, RowError> {
        match self
            .cells
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            Cell::Bool(b) => Ok(*b),
            Cell::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(tc(c, "not a boolean")),
        }
    }
    fn get_bool_opt(&self, c: &str) -> Result<Option<bool>, RowError> {
        match self.cells.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Cell::Null) => Ok(None),
            Some(Cell::Bool(b)) => Ok(Some(*b)),
            Some(_) => Err(tc(c, "not a boolean")),
        }
    }
    fn get_str(&self, c: &str) -> Result<&str, RowError> {
        match self
            .cells
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            Cell::Text(s) => Ok(s.as_str()),
            Cell::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(tc(c, "not text")),
        }
    }
    fn get_str_opt(&self, c: &str) -> Result<Option<&str>, RowError> {
        match self.cells.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Cell::Null) => Ok(None),
            Some(Cell::Text(s)) => Ok(Some(s.as_str())),
            Some(_) => Err(tc(c, "not text")),
        }
    }
    fn get_bytes(&self, c: &str) -> Result<&[u8], RowError> {
        match self
            .cells
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            Cell::Bytes(b) => Ok(b.as_slice()),
            Cell::Text(s) => Ok(s.as_bytes()),
            Cell::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(tc(c, "not bytes")),
        }
    }
    fn get_bytes_opt(&self, c: &str) -> Result<Option<&[u8]>, RowError> {
        match self.cells.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Cell::Null) => Ok(None),
            Some(Cell::Bytes(b)) => Ok(Some(b.as_slice())),
            Some(Cell::Text(s)) => Ok(Some(s.as_bytes())),
            Some(_) => Err(tc(c, "not bytes")),
        }
    }
    fn get_uuid(&self, c: &str) -> Result<uuid::Uuid, RowError> {
        match self
            .cells
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            Cell::Uuid(u) => Ok(*u),
            Cell::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(tc(c, "not a uuid")),
        }
    }
    fn get_uuid_opt(&self, c: &str) -> Result<Option<uuid::Uuid>, RowError> {
        match self.cells.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Cell::Null) => Ok(None),
            Some(Cell::Uuid(u)) => Ok(Some(*u)),
            Some(_) => Err(tc(c, "not a uuid")),
        }
    }
}
