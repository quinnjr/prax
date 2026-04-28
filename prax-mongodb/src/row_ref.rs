//! Bridge between BSON documents and `prax_query::row::RowRef`.
//!
//! MongoDB is document-oriented — rows are `bson::Document`s, not SQL
//! tuples. The Client API's codegen emits `FromRow` impls that read
//! values by column name, which maps cleanly onto BSON field lookup.
//! This bridge lets a `MongoEngine` hand each fetched document to
//! `T::from_row(&BsonRowRef::new(&doc))` without changing the derive.

use bson::{Bson, Document};
use prax_query::row::{RowError, RowRef};

pub struct BsonRowRef<'a> {
    /// Temporary owned `&str` cache: `get_str` returns `&str` borrowed
    /// from self. Bson's Text variant owns a String which is already
    /// borrowable, but UTF-8 conversions and type-mismatch fallbacks
    /// need a place to materialise. Keep the accessor simple for now —
    /// if the column isn't a String, return a TypeConversion error.
    doc: &'a Document,
}

impl<'a> BsonRowRef<'a> {
    pub fn new(doc: &'a Document) -> Self {
        Self { doc }
    }
}

fn tc(column: &str, msg: impl Into<String>) -> RowError {
    RowError::TypeConversion {
        column: column.into(),
        message: msg.into(),
    }
}

fn get(doc: &Document, c: &str) -> Result<Bson, RowError> {
    doc.get(c)
        .cloned()
        .ok_or_else(|| RowError::ColumnNotFound(c.into()))
}

impl<'a> RowRef for BsonRowRef<'a> {
    fn get_i32(&self, c: &str) -> Result<i32, RowError> {
        match get(self.doc, c)? {
            Bson::Int32(i) => Ok(i),
            Bson::Int64(i) => i32::try_from(i).map_err(|_| tc(c, "i64 overflow")),
            Bson::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(tc(c, "not an integer")),
        }
    }
    fn get_i32_opt(&self, c: &str) -> Result<Option<i32>, RowError> {
        match self.doc.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Bson::Null) => Ok(None),
            Some(Bson::Int32(i)) => Ok(Some(*i)),
            Some(Bson::Int64(i)) => i32::try_from(*i)
                .map(Some)
                .map_err(|_| tc(c, "i64 overflow")),
            Some(_) => Err(tc(c, "not an integer")),
        }
    }
    fn get_i64(&self, c: &str) -> Result<i64, RowError> {
        match get(self.doc, c)? {
            Bson::Int32(i) => Ok(i as i64),
            Bson::Int64(i) => Ok(i),
            Bson::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(tc(c, "not an integer")),
        }
    }
    fn get_i64_opt(&self, c: &str) -> Result<Option<i64>, RowError> {
        match self.doc.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Bson::Null) => Ok(None),
            Some(Bson::Int32(i)) => Ok(Some(*i as i64)),
            Some(Bson::Int64(i)) => Ok(Some(*i)),
            Some(_) => Err(tc(c, "not an integer")),
        }
    }
    fn get_f64(&self, c: &str) -> Result<f64, RowError> {
        match get(self.doc, c)? {
            Bson::Double(f) => Ok(f),
            Bson::Int32(i) => Ok(i as f64),
            Bson::Int64(i) => Ok(i as f64),
            Bson::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(tc(c, "not a number")),
        }
    }
    fn get_f64_opt(&self, c: &str) -> Result<Option<f64>, RowError> {
        match self.doc.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Bson::Null) => Ok(None),
            Some(Bson::Double(f)) => Ok(Some(*f)),
            Some(Bson::Int32(i)) => Ok(Some(*i as f64)),
            Some(Bson::Int64(i)) => Ok(Some(*i as f64)),
            Some(_) => Err(tc(c, "not a number")),
        }
    }
    fn get_bool(&self, c: &str) -> Result<bool, RowError> {
        match get(self.doc, c)? {
            Bson::Boolean(b) => Ok(b),
            Bson::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(tc(c, "not a boolean")),
        }
    }
    fn get_bool_opt(&self, c: &str) -> Result<Option<bool>, RowError> {
        match self.doc.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Bson::Null) => Ok(None),
            Some(Bson::Boolean(b)) => Ok(Some(*b)),
            Some(_) => Err(tc(c, "not a boolean")),
        }
    }
    fn get_str(&self, c: &str) -> Result<&str, RowError> {
        match self.doc.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Bson::Null) => Err(RowError::UnexpectedNull(c.into())),
            Some(Bson::String(s)) => Ok(s.as_str()),
            Some(_) => Err(tc(c, "not text")),
        }
    }
    fn get_str_opt(&self, c: &str) -> Result<Option<&str>, RowError> {
        match self.doc.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Bson::Null) => Ok(None),
            Some(Bson::String(s)) => Ok(Some(s.as_str())),
            Some(_) => Err(tc(c, "not text")),
        }
    }
    fn get_bytes(&self, c: &str) -> Result<&[u8], RowError> {
        match self.doc.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Bson::Null) => Err(RowError::UnexpectedNull(c.into())),
            Some(Bson::Binary(b)) => Ok(b.bytes.as_slice()),
            Some(_) => Err(tc(c, "not bytes")),
        }
    }
    fn get_bytes_opt(&self, c: &str) -> Result<Option<&[u8]>, RowError> {
        match self.doc.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Bson::Null) => Ok(None),
            Some(Bson::Binary(b)) => Ok(Some(b.bytes.as_slice())),
            Some(_) => Err(tc(c, "not bytes")),
        }
    }
    fn get_datetime_utc(&self, c: &str) -> Result<chrono::DateTime<chrono::Utc>, RowError> {
        match get(self.doc, c)? {
            Bson::DateTime(dt) => Ok(dt.into()),
            _ => Err(tc(c, "not a datetime")),
        }
    }
    fn get_datetime_utc_opt(
        &self,
        c: &str,
    ) -> Result<Option<chrono::DateTime<chrono::Utc>>, RowError> {
        match self.doc.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Bson::Null) => Ok(None),
            Some(Bson::DateTime(dt)) => Ok(Some((*dt).into())),
            Some(_) => Err(tc(c, "not a datetime")),
        }
    }
    fn get_json(&self, c: &str) -> Result<serde_json::Value, RowError> {
        let b = get(self.doc, c)?;
        Ok(b.into_relaxed_extjson())
    }
    fn get_json_opt(&self, c: &str) -> Result<Option<serde_json::Value>, RowError> {
        match self.doc.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Bson::Null) => Ok(None),
            Some(b) => Ok(Some(b.clone().into_relaxed_extjson())),
        }
    }
}
