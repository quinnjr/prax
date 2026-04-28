//! Microsoft SQL Server query engine implementation.

use std::marker::PhantomData;

use prax_query::QueryResult;
use prax_query::filter::FilterValue;
use prax_query::row::FromRow;
use prax_query::traits::{BoxFuture, Model, QueryEngine};
use tracing::trace;

use crate::pool::MssqlPool;
use crate::row_ref::MssqlRowRef;
use crate::types::filter_value_to_sql;

/// Microsoft SQL Server query engine that implements the Prax QueryEngine trait.
#[derive(Clone)]
pub struct MssqlEngine {
    pool: MssqlPool,
}

impl MssqlEngine {
    /// Create a new MSSQL engine with the given connection pool.
    pub fn new(pool: MssqlPool) -> Self {
        Self { pool }
    }

    /// Get a reference to the connection pool.
    pub fn pool(&self) -> &MssqlPool {
        &self.pool
    }

    /// Convert filter values to MSSQL parameters.
    fn to_params(
        values: &[FilterValue],
    ) -> Result<Vec<Box<dyn tiberius::ToSql>>, prax_query::QueryError> {
        values
            .iter()
            .map(|v| {
                filter_value_to_sql(v).map_err(|e| {
                    let msg = e.to_string();
                    prax_query::QueryError::database(msg).with_source(e)
                })
            })
            .collect()
    }

    /// Convert PostgreSQL-style parameter placeholders ($1, $2) to MSSQL-style (@P1, @P2).
    fn convert_params(sql: &str) -> String {
        let mut result = sql.to_string();
        let mut i = 1;

        while result.contains(&format!("${}", i)) {
            result = result.replace(&format!("${}", i), &format!("@P{}", i));
            i += 1;
        }

        result
    }

    /// T-SQL places `OUTPUT INSERTED.*` between the column list and
    /// `VALUES ...`, not at the end of the statement. The cross-dialect
    /// `build_sql` in prax-query appends the dialect's returning clause
    /// after `VALUES (...)`, which works for Postgres / SQLite / (future)
    /// MySQL RETURNING but yields a 102 "Incorrect syntax near 'OUTPUT'"
    /// error against SQL Server. We rearrange the statement here so the
    /// generic builder doesn't need to know about T-SQL clause ordering.
    ///
    /// Input:  `INSERT INTO t (c1,c2) VALUES (@P1,@P2) OUTPUT INSERTED.*`
    /// Output: `INSERT INTO t (c1,c2) OUTPUT INSERTED.* VALUES (@P1,@P2)`
    ///
    /// Leaves the SQL untouched if no ` OUTPUT ` clause is present (e.g.
    /// raw SQL from `QueryEngine::execute_raw`) or if the clause is
    /// already correctly positioned before `VALUES`.
    fn rearrange_output_for_insert(sql: &str) -> String {
        let Some(output_idx) = sql.rfind(" OUTPUT ") else {
            return sql.to_string();
        };
        let Some(values_idx) = sql.find(" VALUES ") else {
            return sql.to_string();
        };
        if output_idx < values_idx {
            // already in T-SQL order
            return sql.to_string();
        }
        let prefix = &sql[..values_idx];
        let output_clause = &sql[output_idx..];
        let values_clause = &sql[values_idx..output_idx];
        format!("{prefix}{output_clause}{values_clause}")
    }

    /// For UPDATE, T-SQL places `OUTPUT INSERTED.*` between the SET
    /// clause and the WHERE clause. Mirrors `rearrange_output_for_insert`
    /// but anchors on ` WHERE ` instead of ` VALUES `. Update statements
    /// without a WHERE clause leave the trailing OUTPUT in place — that's
    /// already a T-SQL-legal form (`UPDATE t SET c=v OUTPUT INSERTED.*`).
    fn rearrange_output_for_update(sql: &str) -> String {
        let Some(output_idx) = sql.rfind(" OUTPUT ") else {
            return sql.to_string();
        };
        let Some(where_idx) = sql.find(" WHERE ") else {
            // OUTPUT at the end of a WHERE-less UPDATE is already legal.
            return sql.to_string();
        };
        if output_idx < where_idx {
            return sql.to_string();
        }
        let prefix = &sql[..where_idx];
        let output_clause = &sql[output_idx..];
        let where_clause = &sql[where_idx..output_idx];
        format!("{prefix}{output_clause}{where_clause}")
    }

    /// Decode a single row via the MssqlRowRef bridge.
    ///
    /// # Short-circuit on decode error
    ///
    /// When called via `.iter().map(Self::decode_row).collect()`, the
    /// iterator short-circuits on the first decode error and discards
    /// every successfully-decoded row before it. A row-level type
    /// mismatch therefore aborts the whole batch rather than returning
    /// partial results. Callers that want per-row recovery should
    /// manually iterate and handle each result.
    fn decode_row<T: FromRow>(row: &tiberius::Row) -> prax_query::QueryResult<T> {
        let row_ref = MssqlRowRef::from_row(row).map_err(|e| {
            let msg = e.to_string();
            prax_query::QueryError::deserialization(msg).with_source(e)
        })?;
        T::from_row(&row_ref).map_err(|e| {
            let msg = e.to_string();
            prax_query::QueryError::deserialization(msg).with_source(e)
        })
    }
}

impl QueryEngine for MssqlEngine {
    fn dialect(&self) -> &dyn prax_query::dialect::SqlDialect {
        &prax_query::dialect::Mssql
    }

    fn query_many<T: Model + FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
        let sql = Self::convert_params(sql);
        Box::pin(async move {
            trace!(sql = %sql, "Executing query_many");

            let mut conn =
                self.pool.get().await.map_err(|e| {
                    prax_query::QueryError::connection(e.to_string()).with_source(e)
                })?;

            let mssql_params = Self::to_params(&params)?;
            let param_refs: Vec<&dyn tiberius::ToSql> =
                mssql_params.iter().map(|p| p.as_ref()).collect();

            let rows = conn
                .query(&sql, &param_refs)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;

            rows.iter().map(Self::decode_row).collect()
        })
    }

    fn query_one<T: Model + FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<T>> {
        let sql = Self::convert_params(sql);
        Box::pin(async move {
            trace!(sql = %sql, "Executing query_one");

            let mut conn =
                self.pool.get().await.map_err(|e| {
                    prax_query::QueryError::connection(e.to_string()).with_source(e)
                })?;

            let mssql_params = Self::to_params(&params)?;
            let param_refs: Vec<&dyn tiberius::ToSql> =
                mssql_params.iter().map(|p| p.as_ref()).collect();

            let row = conn.query_one(&sql, &param_refs).await.map_err(|e| {
                let msg = e.to_string();
                if msg.contains("no rows") || msg.contains("returned no rows") {
                    prax_query::QueryError::not_found(T::MODEL_NAME)
                } else {
                    prax_query::QueryError::database(msg).with_source(e)
                }
            })?;

            Self::decode_row(&row)
        })
    }

    fn query_optional<T: Model + FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Option<T>>> {
        let sql = Self::convert_params(sql);
        Box::pin(async move {
            trace!(sql = %sql, "Executing query_optional");

            let mut conn =
                self.pool.get().await.map_err(|e| {
                    prax_query::QueryError::connection(e.to_string()).with_source(e)
                })?;

            let mssql_params = Self::to_params(&params)?;
            let param_refs: Vec<&dyn tiberius::ToSql> =
                mssql_params.iter().map(|p| p.as_ref()).collect();

            let row = conn
                .query_opt(&sql, &param_refs)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;

            match row {
                Some(r) => Self::decode_row(&r).map(Some),
                None => Ok(None),
            }
        })
    }

    fn execute_insert<T: Model + FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<T>> {
        let sql = Self::rearrange_output_for_insert(&Self::convert_params(sql));
        Box::pin(async move {
            trace!(sql = %sql, "Executing insert");

            let mut conn =
                self.pool.get().await.map_err(|e| {
                    prax_query::QueryError::connection(e.to_string()).with_source(e)
                })?;

            let mssql_params = Self::to_params(&params)?;
            let param_refs: Vec<&dyn tiberius::ToSql> =
                mssql_params.iter().map(|p| p.as_ref()).collect();

            // For INSERT with RETURNING, MSSQL uses OUTPUT clause which returns rows.
            let row = conn
                .query_one(&sql, &param_refs)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;

            Self::decode_row(&row)
        })
    }

    fn execute_update<T: Model + FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
        let sql = Self::rearrange_output_for_update(&Self::convert_params(sql));
        Box::pin(async move {
            trace!(sql = %sql, "Executing update");

            let mut conn =
                self.pool.get().await.map_err(|e| {
                    prax_query::QueryError::connection(e.to_string()).with_source(e)
                })?;

            let mssql_params = Self::to_params(&params)?;
            let param_refs: Vec<&dyn tiberius::ToSql> =
                mssql_params.iter().map(|p| p.as_ref()).collect();

            let rows = conn
                .query(&sql, &param_refs)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;

            rows.iter().map(Self::decode_row).collect()
        })
    }

    fn execute_delete(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<u64>> {
        let sql = Self::convert_params(sql);
        Box::pin(async move {
            trace!(sql = %sql, "Executing delete");

            let mut conn =
                self.pool.get().await.map_err(|e| {
                    prax_query::QueryError::connection(e.to_string()).with_source(e)
                })?;

            let mssql_params = Self::to_params(&params)?;
            let param_refs: Vec<&dyn tiberius::ToSql> =
                mssql_params.iter().map(|p| p.as_ref()).collect();

            let count = conn
                .execute(&sql, &param_refs)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;

            Ok(count)
        })
    }

    fn execute_raw(&self, sql: &str, params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        let sql = Self::convert_params(sql);
        Box::pin(async move {
            trace!(sql = %sql, "Executing raw SQL");

            let mut conn =
                self.pool.get().await.map_err(|e| {
                    prax_query::QueryError::connection(e.to_string()).with_source(e)
                })?;

            let mssql_params = Self::to_params(&params)?;
            let param_refs: Vec<&dyn tiberius::ToSql> =
                mssql_params.iter().map(|p| p.as_ref()).collect();

            let count = conn
                .execute(&sql, &param_refs)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;

            Ok(count)
        })
    }

    fn count(&self, sql: &str, params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        let sql = Self::convert_params(sql);
        Box::pin(async move {
            trace!(sql = %sql, "Executing count");

            let mut conn =
                self.pool.get().await.map_err(|e| {
                    prax_query::QueryError::connection(e.to_string()).with_source(e)
                })?;

            let mssql_params = Self::to_params(&params)?;
            let param_refs: Vec<&dyn tiberius::ToSql> =
                mssql_params.iter().map(|p| p.as_ref()).collect();

            let row = conn
                .query_one(&sql, &param_refs)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;

            // COUNT is always INT in SQL Server (COUNT_BIG is BIGINT). Probe i64
            // first (handles COUNT_BIG), fall back to i32 for COUNT. Use try_get so
            // a type mismatch surfaces cleanly rather than being conflated with a
            // NULL column.
            match row.try_get::<i64, _>(0) {
                Ok(Some(n)) => return Ok(n as u64),
                Ok(None) => {
                    return Err(prax_query::QueryError::deserialization(
                        "count query column 0 is NULL".to_string(),
                    ));
                }
                Err(_) => {} // wrong type, fall through to i32
            }
            match row.try_get::<i32, _>(0) {
                Ok(Some(n)) => Ok(n as u64),
                Ok(None) => Err(prax_query::QueryError::deserialization(
                    "count query column 0 is NULL".to_string(),
                )),
                Err(e) => {
                    let msg = format!("count query column 0 is not an integer: {e}");
                    Err(prax_query::QueryError::deserialization(msg).with_source(e))
                }
            }
        })
    }
}

/// A typed query builder that uses the MSSQL engine.
pub struct MssqlQueryBuilder<T: Model> {
    engine: MssqlEngine,
    _marker: PhantomData<T>,
}

impl<T: Model> MssqlQueryBuilder<T> {
    /// Create a new query builder.
    pub fn new(engine: MssqlEngine) -> Self {
        Self {
            engine,
            _marker: PhantomData,
        }
    }

    /// Get the underlying engine.
    pub fn engine(&self) -> &MssqlEngine {
        &self.engine
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_params() {
        assert_eq!(
            MssqlEngine::convert_params("SELECT * FROM users WHERE id = $1"),
            "SELECT * FROM users WHERE id = @P1"
        );

        assert_eq!(
            MssqlEngine::convert_params("SELECT * FROM users WHERE id = $1 AND name = $2"),
            "SELECT * FROM users WHERE id = @P1 AND name = @P2"
        );

        assert_eq!(
            MssqlEngine::convert_params("SELECT * FROM users"),
            "SELECT * FROM users"
        );
    }
}
