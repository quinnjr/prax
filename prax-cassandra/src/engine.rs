//! Query execution engine.
//!
//! This module defines the public query API (query/execute/batch/LWT/paging).
//! Routes every statement through the cdrs-tokio session held by the
//! underlying [`CassandraPool`].

use crate::error::{CassandraError, CassandraResult};
use crate::pool::CassandraPool;
use crate::row::{FromRow, Row};

/// Aggregate result of a CQL query.
#[derive(Debug, Default)]
pub struct QueryResult {
    /// Rows returned by the query. Empty for non-SELECT statements.
    pub rows: Vec<Row>,
    /// Whether a lightweight transaction applied.
    pub applied: Option<bool>,
}

impl CassandraPool {
    /// Execute a query returning rows.
    pub async fn query(&self, cql: &str) -> CassandraResult<QueryResult> {
        let envelope = self
            .connection()
            .session()
            .query(cql)
            .await
            .map_err(|e| CassandraError::Query(format!("query failed: {e}")))?;

        decode_query_envelope(envelope)
    }

    /// Execute a query with positional values bound to its `?`
    /// placeholders. Values ride on the QUERY message directly (no
    /// prepare step); an empty value set degrades to [`Self::query`].
    pub async fn query_with_values(
        &self,
        cql: &str,
        values: cdrs_tokio::query::QueryValues,
    ) -> CassandraResult<QueryResult> {
        if values.is_empty() {
            return self.query(cql).await;
        }
        let envelope = self
            .connection()
            .session()
            .query_with_values(cql, values)
            .await
            .map_err(|e| CassandraError::Query(format!("query failed: {e}")))?;

        decode_query_envelope(envelope)
    }

    /// Execute a statement not expecting rows (INSERT, UPDATE, DELETE, DDL).
    pub async fn execute(&self, cql: &str) -> CassandraResult<()> {
        self.connection()
            .session()
            .query(cql)
            .await
            .map_err(|e| CassandraError::Query(format!("execute failed: {e}")))?;
        Ok(())
    }

    /// Execute a statement with positional values bound to its `?`
    /// placeholders, not expecting rows. An empty value set degrades
    /// to [`Self::execute`].
    pub async fn execute_with_values(
        &self,
        cql: &str,
        values: cdrs_tokio::query::QueryValues,
    ) -> CassandraResult<()> {
        if values.is_empty() {
            return self.execute(cql).await;
        }
        self.connection()
            .session()
            .query_with_values(cql, values)
            .await
            .map_err(|e| CassandraError::Query(format!("execute failed: {e}")))?;
        Ok(())
    }

    /// Query a single row, deserialized into T.
    pub async fn query_one<T: FromRow>(&self, cql: &str) -> CassandraResult<T> {
        let result = self.query(cql).await?;
        let row = result
            .rows
            .into_iter()
            .next()
            .ok_or_else(|| CassandraError::Query("query_one: no rows returned".into()))?;
        T::from_row(&row)
    }

    /// Query many rows.
    pub async fn query_many<T: FromRow>(&self, cql: &str) -> CassandraResult<Vec<T>> {
        let result = self.query(cql).await?;
        result.rows.iter().map(|row| T::from_row(row)).collect()
    }

    /// Execute a lightweight transaction. Returns whether the CAS succeeded.
    ///
    /// Errors when the response carries no `[applied]` column — that
    /// means the statement was not actually an LWT (or was misrouted),
    /// and silently reporting `false` would lie to the caller.
    pub async fn execute_lwt(&self, cql: &str) -> CassandraResult<bool> {
        let result = self.query(cql).await?;
        result.applied.ok_or_else(|| {
            CassandraError::Query(
                "execute_lwt: statement was not an LWT — response carried no [applied] column"
                    .into(),
            )
        })
    }

    /// Build a batch of statements.
    pub fn batch(&self) -> BatchBuilder<'_> {
        BatchBuilder {
            pool: self,
            statements: Vec::new(),
        }
    }
}

/// Parse a query/execute response envelope into a [`QueryResult`].
///
/// SELECT responses carry a `ResponseBody::Result` with rows;
/// INSERT/UPDATE/DELETE responses typically carry an empty result.
/// LWT responses carry a single row with the `[applied]` boolean
/// column first.
fn decode_query_envelope(envelope: cdrs_tokio::frame::Envelope) -> CassandraResult<QueryResult> {
    let body = envelope
        .response_body()
        .map_err(|e| CassandraError::Query(format!("response body parse: {e}")))?;

    let (rows, applied) = if let Some(raw_rows) = body.into_rows() {
        let applied = detect_applied_flag(&raw_rows)?;
        let decoded: Vec<crate::row::Row> = raw_rows
            .into_iter()
            .map(|r| crate::row::Row::from_cdrs_row(&r))
            .collect::<CassandraResult<_>>()?;
        (decoded, applied)
    } else {
        (Vec::new(), None)
    };

    Ok(QueryResult { rows, applied })
}

/// Extract the LWT `[applied]` flag from the first raw row. `Ok(None)`
/// means the result carried no `[applied]` column — i.e. the
/// statement was not an LWT. A present-but-undecodable flag is a real
/// decode error and is propagated: cdrs-tokio's `by_name` returns `Err`
/// both when the column is absent and when decoding fails, so column
/// presence is checked first to keep the two cases distinct.
fn detect_applied_flag(raw_rows: &[cdrs_tokio::types::rows::Row]) -> CassandraResult<Option<bool>> {
    use cdrs_tokio::types::ByName;
    let Some(row) = raw_rows.first() else {
        return Ok(None);
    };
    if !row.contains_column("[applied]") {
        return Ok(None);
    }
    row.by_name::<bool>("[applied]").map_err(|e| {
        CassandraError::Query(format!(
            "execute_lwt: [applied] column present but failed to decode as bool: {e}"
        ))
    })
}

/// One batch entry: the CQL text plus the positional values to bind
/// to its `?` placeholders (empty for literal-only statements).
type BatchStatement = (String, Vec<prax_query::filter::FilterValue>);

/// Builder for a CQL batch.
pub struct BatchBuilder<'a> {
    pool: &'a CassandraPool,
    statements: Vec<BatchStatement>,
}

impl<'a> BatchBuilder<'a> {
    /// Add a statement to the batch.
    ///
    /// The statement is sent as-is with no bound values, so it must
    /// use CQL literals only — a statement containing `?` placeholders
    /// fails server-side. Use
    /// [`add_statement_with_values`](Self::add_statement_with_values)
    /// for parameterized statements.
    pub fn add_statement(mut self, cql: impl Into<String>) -> Self {
        self.statements.push((cql.into(), Vec::new()));
        self
    }

    /// Add a statement with positional values bound to its `?`
    /// placeholders. Values are converted from
    /// [`FilterValue`](prax_query::filter::FilterValue) with the same
    /// rules the `QueryEngine` impl uses (`Int` → 8-byte bigint,
    /// `Float` → double, `Json` → text, `List` → CQL collection wire
    /// encoding); a conversion failure surfaces at execute time.
    pub fn add_statement_with_values(
        mut self,
        cql: impl Into<String>,
        values: Vec<prax_query::filter::FilterValue>,
    ) -> Self {
        self.statements.push((cql.into(), values));
        self
    }

    /// Execute the batch as a LOGGED batch (default).
    pub async fn execute(self) -> CassandraResult<()> {
        self.execute_logged().await
    }

    /// Execute the batch as a LOGGED batch.
    pub async fn execute_logged(self) -> CassandraResult<()> {
        self.execute_with_type(cdrs_tokio::frame::message_batch::BatchType::Logged)
            .await
    }

    /// Execute the batch as an UNLOGGED batch.
    pub async fn execute_unlogged(self) -> CassandraResult<()> {
        self.execute_with_type(cdrs_tokio::frame::message_batch::BatchType::Unlogged)
            .await
    }

    /// Execute the batch as a COUNTER batch.
    pub async fn execute_counter(self) -> CassandraResult<()> {
        self.execute_with_type(cdrs_tokio::frame::message_batch::BatchType::Counter)
            .await
    }

    async fn execute_with_type(
        self,
        batch_type: cdrs_tokio::frame::message_batch::BatchType,
    ) -> CassandraResult<()> {
        let batch = build_batch(self.statements, batch_type)?;
        self.pool
            .connection()
            .session()
            .batch(batch)
            .await
            .map_err(|e| CassandraError::Query(format!("batch execute: {e}")))?;
        Ok(())
    }

    /// Number of statements in the batch (for test/debug).
    pub fn len(&self) -> usize {
        self.statements.len()
    }

    /// True if the batch has no statements.
    pub fn is_empty(&self) -> bool {
        self.statements.is_empty()
    }
}

/// Assemble a cdrs batch from accumulated statements, converting each
/// statement's [`FilterValue`](prax_query::filter::FilterValue)s to
/// positional query values. Pure so it stays testable without a live
/// cluster.
fn build_batch(
    statements: Vec<BatchStatement>,
    batch_type: cdrs_tokio::frame::message_batch::BatchType,
) -> CassandraResult<cdrs_tokio::query::QueryBatch> {
    if statements.is_empty() {
        return Err(CassandraError::Query("cannot execute empty batch".into()));
    }
    let mut builder = cdrs_tokio::query::BatchQueryBuilder::new().with_batch_type(batch_type);
    for (stmt, params) in statements {
        let values = params_to_query_values(&params)
            .map_err(|e| CassandraError::Query(format!("batch statement values: {e}")))?;
        builder = builder.add_query(stmt, values);
    }
    builder
        .build()
        .map_err(|e| CassandraError::Query(format!("batch build: {e}")))
}

/// Top-level query engine for the Cassandra driver.
///
/// Thin wrapper around [`CassandraPool`] that lets `#[derive(Model)]`-
/// generated `Client<E>` target Cassandra through the same codegen
/// pipeline the SQL drivers use. Routes SELECT/DELETE through the real
/// cdrs-tokio session, binding `?` placeholders via QUERY-message
/// values; `execute_update` runs the UPDATE then re-SELECTs rows
/// matching the WHERE clause; `execute_insert` currently returns
/// [`QueryError::unsupported`](prax_query::QueryError::unsupported) —
/// there is no safe way to identify which bound values key the
/// follow-up PK SELECT. Prefer [`prax_scylladb::ScyllaEngine`] for
/// typed Client inserts against any CQL-compatible cluster.
#[derive(Clone)]
pub struct CassandraEngine {
    pool: CassandraPool,
}

impl CassandraEngine {
    /// Create a new engine wrapping the given pool.
    pub fn new(pool: CassandraPool) -> Self {
        Self { pool }
    }

    /// Borrow the underlying pool. Exposed for callers that need to
    /// reach the raw query/execute/batch helpers directly.
    pub fn pool(&self) -> &CassandraPool {
        &self.pool
    }
}

impl prax_query::traits::QueryEngine for CassandraEngine {
    fn dialect(&self) -> &dyn prax_query::dialect::SqlDialect {
        &prax_query::dialect::Cql
    }

    fn query_many<T: prax_query::traits::Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<prax_query::filter::FilterValue>,
    ) -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<Vec<T>>> {
        let sql = sql.to_string();
        let pool = self.pool.clone();
        Box::pin(async move {
            let values = params_to_query_values(&params)
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
            let result = pool
                .query_with_values(&sql, values)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
            // Hoist the column list out of the per-row closure — it is
            // the same Vec for every row of this query.
            let cols: Vec<String> = T::COLUMNS.iter().map(|s| s.to_string()).collect();
            result
                .rows
                .iter()
                .map(|r| decode_row::<T>(r.as_cdrs(), &cols))
                .collect()
        })
    }

    fn query_one<T: prax_query::traits::Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<prax_query::filter::FilterValue>,
    ) -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<T>> {
        let sql = sql.to_string();
        let pool = self.pool.clone();
        Box::pin(async move {
            let values = params_to_query_values(&params)
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
            let result = pool
                .query_with_values(&sql, values)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
            let cdrs_row = result
                .rows
                .iter()
                .map(|r| r.as_cdrs())
                .next()
                .ok_or_else(|| prax_query::QueryError::not_found(T::MODEL_NAME))?;
            let cols: Vec<String> = T::COLUMNS.iter().map(|s| s.to_string()).collect();
            decode_row::<T>(cdrs_row, &cols)
        })
    }

    fn query_optional<T: prax_query::traits::Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<prax_query::filter::FilterValue>,
    ) -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<Option<T>>> {
        let sql = sql.to_string();
        let pool = self.pool.clone();
        Box::pin(async move {
            let values = params_to_query_values(&params)
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
            let result = pool
                .query_with_values(&sql, values)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
            let cols: Vec<String> = T::COLUMNS.iter().map(|s| s.to_string()).collect();
            result
                .rows
                .iter()
                .map(|r| r.as_cdrs())
                .next()
                .map(|row| decode_row::<T>(row, &cols))
                .transpose()
        })
    }

    fn execute_insert<T: prax_query::traits::Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        _params: Vec<prax_query::filter::FilterValue>,
    ) -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<T>> {
        // Bound params work (see params_to_query_values), but a safe
        // PK-keyed follow-up SELECT still isn't possible: the engine
        // can't tell which of the bound values belong to PK columns,
        // and a LIMIT 1 with no WHERE would race concurrent writers
        // and return the wrong row. Refuse rather than fabricate a
        // result. The Scylla driver is feature-complete on this path
        // and is the recommended CQL backend for typed Client inserts.
        let _ = (sql, T::MODEL_NAME);
        Box::pin(async move {
            Err(prax_query::QueryError::unsupported(
                "CassandraEngine::execute_insert cannot safely identify the \
                 PK values needed to re-fetch the inserted row; use \
                 ScyllaEngine or call pool.execute_with_values + \
                 pool.query_with_values manually",
            ))
        })
    }

    fn execute_update<T: prax_query::traits::Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<prax_query::filter::FilterValue>,
    ) -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<Vec<T>>> {
        let sql = sql.to_string();
        let pool = self.pool.clone();
        Box::pin(async move {
            let values = params_to_query_values(&params)
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
            pool.execute_with_values(&sql, values)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
            // Recover the WHERE clause from the generated UPDATE so the
            // follow-up SELECT touches the same rows. Refuse to SELECT
            // everything on a WHERE-less UPDATE — that would be a
            // worse failure mode than erroring.
            let where_clause = extract_where_clause(&sql).ok_or_else(|| {
                prax_query::QueryError::internal(
                    "CassandraEngine::execute_update: UPDATE lacked a WHERE \
                     clause; refusing to SELECT entire table",
                )
            })?;
            // The WHERE params are the tail of `params` — the UPDATE
            // SET clause consumes the head. Count the SET placeholders
            // to find the split point.
            let set_count = count_set_placeholders(&sql).ok_or_else(|| {
                prax_query::QueryError::internal(
                    "CassandraEngine::execute_update: could not count SET \
                     placeholders",
                )
            })?;
            let select_sql = format!(
                "SELECT {} FROM {} WHERE {}",
                T::COLUMNS.join(", "),
                T::TABLE_NAME,
                where_clause,
            );
            let where_values = params_to_query_values(params.get(set_count..).unwrap_or(&[]))
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
            let result = pool
                .query_with_values(&select_sql, where_values)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
            let cols: Vec<String> = T::COLUMNS.iter().map(|s| s.to_string()).collect();
            result
                .rows
                .iter()
                .map(|r| decode_row::<T>(r.as_cdrs(), &cols))
                .collect()
        })
    }

    /// CQL reports no affected-row count for DELETE — the protocol
    /// returns an empty result on success. The returned `0` therefore
    /// means "the protocol carries no count", not "no rows were
    /// deleted".
    fn execute_delete(
        &self,
        sql: &str,
        params: Vec<prax_query::filter::FilterValue>,
    ) -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<u64>> {
        let sql = sql.to_string();
        let pool = self.pool.clone();
        Box::pin(async move {
            let values = params_to_query_values(&params)
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
            pool.execute_with_values(&sql, values)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
            Ok(0)
        })
    }

    /// Like [`execute_delete`](Self::execute_delete), CQL reports no
    /// affected-row count for arbitrary statements — the returned `0`
    /// means "the protocol carries no count", not "nothing happened".
    fn execute_raw(
        &self,
        sql: &str,
        params: Vec<prax_query::filter::FilterValue>,
    ) -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<u64>> {
        self.execute_delete(sql, params)
    }

    fn count(
        &self,
        sql: &str,
        params: Vec<prax_query::filter::FilterValue>,
    ) -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<u64>> {
        let sql = sql.to_string();
        let pool = self.pool.clone();
        Box::pin(async move {
            let values = params_to_query_values(&params)
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
            let result = pool
                .query_with_values(&sql, values)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
            decode_count(&result.rows)
        })
    }
}

// WHERE-clause extraction and SET-placeholder counting live in
// prax_query::sql::parse — import here under the old name to
// minimise churn.
use prax_query::sql::parse::{count_set_placeholders, extract_where_body as extract_where_clause};

/// Decode one cdrs-tokio row into the caller's `T: Model + FromRow`.
/// The caller builds `cols` once per query (from `T::COLUMNS`) and
/// passes it in, so the per-row decode does no column-list
/// allocation; error-wrapping lives here so every QueryEngine method
/// shares it.
fn decode_row<T: prax_query::traits::Model + prax_query::row::FromRow>(
    cdrs_row: &cdrs_tokio::types::rows::Row,
    cols: &[String],
) -> prax_query::QueryResult<T> {
    let rr = crate::row_ref::CassandraRowRef::from_cdrs_with_cols(cdrs_row, cols);
    T::from_row(&rr).map_err(|e| {
        let msg = e.to_string();
        prax_query::QueryError::deserialization(msg).with_source(e)
    })
}

/// Decode a `SELECT COUNT(*)` result: one row whose first column is a
/// CQL bigint.
fn decode_count(rows: &[crate::row::Row]) -> prax_query::QueryResult<u64> {
    use cdrs_tokio::types::ByIndex;
    let first = rows
        .first()
        .ok_or_else(|| prax_query::QueryError::deserialization("count returned no row"))?;
    match first.as_cdrs().by_index::<i64>(0) {
        Ok(Some(n)) => Ok(n as u64),
        _ => Err(prax_query::QueryError::deserialization(
            "count column missing or wrong type in CQL result",
        )),
    }
}

/// Convert bound params to positional CQL query values, in order.
fn params_to_query_values(
    params: &[prax_query::filter::FilterValue],
) -> CassandraResult<cdrs_tokio::query::QueryValues> {
    let values = params
        .iter()
        .map(filter_value_to_value)
        .collect::<CassandraResult<Vec<_>>>()?;
    Ok(cdrs_tokio::query::QueryValues::SimpleValues(values))
}

/// Convert one [`FilterValue`](prax_query::filter::FilterValue) to a
/// cdrs wire value. Numbers encode at their widest CQL form
/// (`Int` → 8-byte bigint, `Float` → 8-byte double), mirroring the
/// Scylla engine; `Json` binds as text.
fn filter_value_to_value(
    value: &prax_query::filter::FilterValue,
) -> CassandraResult<cdrs_tokio::types::value::Value> {
    use cdrs_tokio::types::value::Value;
    use prax_query::filter::FilterValue;
    match value {
        FilterValue::Null => Ok(Value::Null),
        FilterValue::Bool(b) => Ok(Value::new(*b)),
        FilterValue::Int(i) => Ok(Value::new(*i)),
        FilterValue::Float(f) => Ok(Value::new(*f)),
        FilterValue::String(s) => Ok(Value::new(s.clone())),
        FilterValue::Json(j) => Ok(Value::new(j.to_string())),
        FilterValue::List(items) | FilterValue::Array(items) => {
            Ok(Value::Some(encode_cql_list(items)?))
        }
        // CQL blob: raw bytes.
        FilterValue::Bytes(b) => Ok(Value::Some(b.clone())),
        _ => Err(CassandraError::Query(
            "unsupported FilterValue variant".into(),
        )),
    }
}

/// Raw wire bytes for a single value, used for collection elements.
/// Null is rejected: CQL forbids null collection elements.
fn filter_value_wire_bytes(value: &prax_query::filter::FilterValue) -> CassandraResult<Vec<u8>> {
    use cdrs_tokio::types::value::Bytes;
    use prax_query::filter::FilterValue;
    match value {
        FilterValue::Null => Err(CassandraError::Query(
            "cannot bind a null element inside a CQL collection".into(),
        )),
        FilterValue::Bool(b) => Ok(Bytes::from(*b).into_inner()),
        FilterValue::Int(i) => Ok(Bytes::from(*i).into_inner()),
        FilterValue::Float(f) => Ok(Bytes::from(*f).into_inner()),
        FilterValue::String(s) => Ok(Bytes::from(s.clone()).into_inner()),
        FilterValue::Json(j) => Ok(Bytes::from(j.to_string()).into_inner()),
        FilterValue::List(items) | FilterValue::Array(items) => encode_cql_list(items),
        FilterValue::Bytes(b) => Ok(b.clone()),
        _ => Err(CassandraError::Query(
            "unsupported FilterValue variant".into(),
        )),
    }
}

/// Encode a CQL collection value: an `[int]` element count followed
/// by `[int len][bytes]` per element (native protocol collection
/// wire format).
fn encode_cql_list(items: &[prax_query::filter::FilterValue]) -> CassandraResult<Vec<u8>> {
    let count = i32::try_from(items.len()).map_err(|_| {
        CassandraError::Query(format!("list too large to bind: {} elements", items.len()))
    })?;
    let mut out = Vec::new();
    out.extend_from_slice(&count.to_be_bytes());
    for item in items {
        let bytes = filter_value_wire_bytes(item)?;
        let len = i32::try_from(bytes.len())
            .map_err(|_| CassandraError::Query("collection element too large to bind".into()))?;
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(&bytes);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdrs_tokio::frame::Version;
    use cdrs_tokio::frame::message_result::{
        BodyResResultRows, ColSpec, ColType, ColTypeOption, RowsMetadata, RowsMetadataFlags,
        TableSpec,
    };
    use cdrs_tokio::types::CBytes;
    use prax_query::filter::FilterValue;

    /// Build real cdrs rows over a single column, for engine decode
    /// tests that must not need a live cluster.
    fn single_column_rows(
        name: &str,
        col_type: ColType,
        cells: Vec<Option<Vec<u8>>>,
    ) -> Vec<cdrs_tokio::types::rows::Row> {
        let body = BodyResResultRows {
            metadata: RowsMetadata {
                flags: RowsMetadataFlags::GLOBAL_TABLE_SPACE,
                columns_count: 1,
                paging_state: None,
                new_metadata_id: None,
                global_table_spec: Some(TableSpec {
                    ks_name: "ks".into(),
                    table_name: "t".into(),
                }),
                col_specs: vec![ColSpec {
                    table_spec: None,
                    name: name.to_string(),
                    col_type: ColTypeOption {
                        id: col_type,
                        value: None,
                    },
                }],
            },
            rows_count: cells.len() as i32,
            rows_content: cells
                .into_iter()
                .map(|c| vec![c.map_or_else(CBytes::new_null, CBytes::new)])
                .collect(),
            protocol_version: Version::V4,
        };
        cdrs_tokio::types::rows::Row::from_body(body)
    }

    fn wrapped_rows(cdrs_rows: &[cdrs_tokio::types::rows::Row]) -> Vec<crate::row::Row> {
        cdrs_rows
            .iter()
            .map(crate::row::Row::from_cdrs_row)
            .collect::<CassandraResult<_>>()
            .unwrap()
    }

    #[test]
    fn test_query_result_default_is_empty() {
        let r = QueryResult::default();
        assert!(r.rows.is_empty());
        assert!(r.applied.is_none());
    }

    #[test]
    fn batch_empty_statements_is_an_error() {
        let err = build_batch(vec![], cdrs_tokio::frame::message_batch::BatchType::Logged)
            .expect_err("empty batch must fail");
        assert!(err.to_string().contains("empty batch"));
    }

    #[test]
    fn batch_binds_values_for_placeholders() {
        let batch = build_batch(
            vec![
                (
                    "INSERT INTO t (id, v) VALUES (?, ?)".to_string(),
                    vec![FilterValue::Int(1), FilterValue::String("a".into())],
                ),
                (
                    "UPDATE t SET v = ? WHERE id = ?".to_string(),
                    vec![FilterValue::Int(2), FilterValue::Int(1)],
                ),
            ],
            cdrs_tokio::frame::message_batch::BatchType::Unlogged,
        )
        .unwrap();
        assert_eq!(batch.request.queries.len(), 2);
        // First statement carries two positional values; the first is
        // the 8-byte bigint encoding of 1.
        match &batch.request.queries[0].values {
            cdrs_tokio::query::QueryValues::SimpleValues(values) => {
                assert_eq!(values.len(), 2);
                assert_eq!(
                    values[0],
                    cdrs_tokio::types::value::Value::Some(1i64.to_be_bytes().to_vec())
                );
                assert_eq!(
                    values[1],
                    cdrs_tokio::types::value::Value::Some(b"a".to_vec())
                );
            }
            other => panic!("expected simple values, got {other:?}"),
        }
        // Second statement binds its two values as well.
        assert_eq!(batch.request.queries[1].values.len(), 2);
    }

    #[test]
    fn where_clause_and_set_placeholder_split() {
        let sql = "UPDATE users SET name = ?, age = ? WHERE id = ?";
        assert_eq!(extract_where_clause(sql).as_deref(), Some("id = ?"));
        assert_eq!(count_set_placeholders(sql), Some(2));
        let params = [
            FilterValue::String("x".into()),
            FilterValue::Int(3),
            FilterValue::Int(7),
        ];
        // The follow-up SELECT binds only the WHERE tail.
        let tail = params.get(2..).unwrap_or(&[]);
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0], FilterValue::Int(7));
        // No WHERE -> extraction refuses (execute_update relies on this).
        assert!(extract_where_clause("UPDATE t SET a = ?").is_none());
    }

    #[test]
    fn lwt_applied_flag_detected_from_rows() {
        let rows = single_column_rows("[applied]", ColType::Boolean, vec![Some(vec![1])]);
        assert_eq!(detect_applied_flag(&rows).unwrap(), Some(true));

        let rows = single_column_rows("[applied]", ColType::Boolean, vec![Some(vec![0])]);
        assert_eq!(detect_applied_flag(&rows).unwrap(), Some(false));

        // A non-LWT result has no [applied] column -> None, which is
        // what makes execute_lwt error instead of returning false.
        let rows = single_column_rows(
            "id",
            ColType::Bigint,
            vec![Some(1i64.to_be_bytes().to_vec())],
        );
        assert_eq!(detect_applied_flag(&rows).unwrap(), None);

        assert_eq!(detect_applied_flag(&[]).unwrap(), None);
    }

    #[test]
    fn lwt_applied_flag_decode_failure_is_not_confused_with_non_lwt() {
        // A present [applied] column whose wire type does not decode as a
        // bool must surface as an error, not collapse into the "not an
        // LWT" None case.
        let rows = single_column_rows(
            "[applied]",
            ColType::Bigint,
            vec![Some(1i64.to_be_bytes().to_vec())],
        );
        let err = detect_applied_flag(&rows).unwrap_err();
        assert!(
            err.to_string().contains("failed to decode"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn count_decode_reads_bigint_first_column() {
        let rows = single_column_rows(
            "count",
            ColType::Bigint,
            vec![Some(42i64.to_be_bytes().to_vec())],
        );
        let wrapped = wrapped_rows(&rows);
        assert_eq!(decode_count(&wrapped).unwrap(), 42);
    }

    #[test]
    fn count_decode_errors_without_rows() {
        let err = decode_count(&[]).expect_err("no rows must error");
        assert!(err.to_string().contains("count returned no row"));
    }

    #[test]
    fn count_decode_errors_on_wrong_type() {
        let rows = single_column_rows("count", ColType::Varchar, vec![Some(b"nope".to_vec())]);
        let wrapped = wrapped_rows(&rows);
        assert!(decode_count(&wrapped).is_err());
    }

    #[test]
    fn filter_value_to_value_encodes_wire_bytes() {
        use cdrs_tokio::types::value::Value;
        assert_eq!(
            filter_value_to_value(&FilterValue::Null).unwrap(),
            Value::Null
        );
        assert_eq!(
            filter_value_to_value(&FilterValue::Bool(true)).unwrap(),
            Value::Some(vec![1])
        );
        assert_eq!(
            filter_value_to_value(&FilterValue::Int(42)).unwrap(),
            Value::Some(42i64.to_be_bytes().to_vec())
        );
        assert_eq!(
            filter_value_to_value(&FilterValue::String("hi".into())).unwrap(),
            Value::Some(b"hi".to_vec())
        );
    }

    #[test]
    fn filter_value_list_encodes_collection_wire_format() {
        let value = filter_value_to_value(&FilterValue::List(vec![
            FilterValue::Int(1),
            FilterValue::Int(2),
        ]))
        .unwrap();
        let mut expected = Vec::new();
        expected.extend_from_slice(&2i32.to_be_bytes()); // element count
        expected.extend_from_slice(&8i32.to_be_bytes()); // len of element 1
        expected.extend_from_slice(&1i64.to_be_bytes());
        expected.extend_from_slice(&8i32.to_be_bytes()); // len of element 2
        expected.extend_from_slice(&2i64.to_be_bytes());
        assert_eq!(value, cdrs_tokio::types::value::Value::Some(expected));
    }

    #[test]
    fn filter_value_bytes_is_raw_blob_and_array_matches_list() {
        use cdrs_tokio::types::value::Value;
        assert_eq!(
            filter_value_to_value(&FilterValue::Bytes(vec![0, 255])).unwrap(),
            Value::Some(vec![0, 255])
        );
        let items = vec![FilterValue::Int(1), FilterValue::Int(2)];
        assert_eq!(
            filter_value_to_value(&FilterValue::Array(items.clone())).unwrap(),
            filter_value_to_value(&FilterValue::List(items)).unwrap()
        );
    }

    #[test]
    fn filter_value_null_list_element_is_rejected() {
        assert!(filter_value_to_value(&FilterValue::List(vec![FilterValue::Null])).is_err());
    }

    #[test]
    fn params_to_query_values_preserves_order() {
        let values = params_to_query_values(&[
            FilterValue::Int(7),
            FilterValue::String("x".into()),
            FilterValue::Bool(false),
        ])
        .unwrap();
        match values {
            cdrs_tokio::query::QueryValues::SimpleValues(values) => {
                assert_eq!(values.len(), 3);
                assert_eq!(
                    values[0],
                    cdrs_tokio::types::value::Value::Some(7i64.to_be_bytes().to_vec())
                );
                assert_eq!(
                    values[1],
                    cdrs_tokio::types::value::Value::Some(b"x".to_vec())
                );
                assert_eq!(values[2], cdrs_tokio::types::value::Value::Some(vec![0]));
            }
            other => panic!("expected simple values, got {other:?}"),
        }
    }
}
