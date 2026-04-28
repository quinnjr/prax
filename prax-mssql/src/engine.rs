//! Microsoft SQL Server query engine implementation.

use std::marker::PhantomData;

use prax_query::QueryResult;
use prax_query::filter::FilterValue;
use prax_query::row::FromRow;
use prax_query::traits::{BoxFuture, Model, QueryEngine};
use tracing::debug;

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
                filter_value_to_sql(v).map_err(|e| prax_query::QueryError::database(e.to_string()))
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

    /// Decode a single row via the MssqlRowRef bridge.
    fn decode_row<T: FromRow>(row: &tiberius::Row) -> prax_query::QueryResult<T> {
        let row_ref = MssqlRowRef::from_row(row)
            .map_err(|e| prax_query::QueryError::deserialization(e.to_string()))?;
        T::from_row(&row_ref).map_err(|e| prax_query::QueryError::deserialization(e.to_string()))
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
            debug!(sql = %sql, "Executing query_many");

            let mut conn = self
                .pool
                .get()
                .await
                .map_err(|e| prax_query::QueryError::connection(e.to_string()))?;

            let mssql_params = Self::to_params(&params)?;
            let param_refs: Vec<&dyn tiberius::ToSql> =
                mssql_params.iter().map(|p| p.as_ref()).collect();

            let rows = conn
                .query(&sql, &param_refs)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

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
            debug!(sql = %sql, "Executing query_one");

            let mut conn = self
                .pool
                .get()
                .await
                .map_err(|e| prax_query::QueryError::connection(e.to_string()))?;

            let mssql_params = Self::to_params(&params)?;
            let param_refs: Vec<&dyn tiberius::ToSql> =
                mssql_params.iter().map(|p| p.as_ref()).collect();

            let row = conn
                .query_one(&sql, &param_refs)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

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
            debug!(sql = %sql, "Executing query_optional");

            let mut conn = self
                .pool
                .get()
                .await
                .map_err(|e| prax_query::QueryError::connection(e.to_string()))?;

            let mssql_params = Self::to_params(&params)?;
            let param_refs: Vec<&dyn tiberius::ToSql> =
                mssql_params.iter().map(|p| p.as_ref()).collect();

            let row = conn
                .query_opt(&sql, &param_refs)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

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
        let sql = Self::convert_params(sql);
        Box::pin(async move {
            debug!(sql = %sql, "Executing insert");

            let mut conn = self
                .pool
                .get()
                .await
                .map_err(|e| prax_query::QueryError::connection(e.to_string()))?;

            let mssql_params = Self::to_params(&params)?;
            let param_refs: Vec<&dyn tiberius::ToSql> =
                mssql_params.iter().map(|p| p.as_ref()).collect();

            // For INSERT with RETURNING, MSSQL uses OUTPUT clause which returns rows.
            let row = conn
                .query_one(&sql, &param_refs)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            Self::decode_row(&row)
        })
    }

    fn execute_update<T: Model + FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
        let sql = Self::convert_params(sql);
        Box::pin(async move {
            debug!(sql = %sql, "Executing update");

            let mut conn = self
                .pool
                .get()
                .await
                .map_err(|e| prax_query::QueryError::connection(e.to_string()))?;

            let mssql_params = Self::to_params(&params)?;
            let param_refs: Vec<&dyn tiberius::ToSql> =
                mssql_params.iter().map(|p| p.as_ref()).collect();

            let rows = conn
                .query(&sql, &param_refs)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

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
            debug!(sql = %sql, "Executing delete");

            let mut conn = self
                .pool
                .get()
                .await
                .map_err(|e| prax_query::QueryError::connection(e.to_string()))?;

            let mssql_params = Self::to_params(&params)?;
            let param_refs: Vec<&dyn tiberius::ToSql> =
                mssql_params.iter().map(|p| p.as_ref()).collect();

            let count = conn
                .execute(&sql, &param_refs)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            Ok(count)
        })
    }

    fn execute_raw(&self, sql: &str, params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        let sql = Self::convert_params(sql);
        Box::pin(async move {
            debug!(sql = %sql, "Executing raw SQL");

            let mut conn = self
                .pool
                .get()
                .await
                .map_err(|e| prax_query::QueryError::connection(e.to_string()))?;

            let mssql_params = Self::to_params(&params)?;
            let param_refs: Vec<&dyn tiberius::ToSql> =
                mssql_params.iter().map(|p| p.as_ref()).collect();

            let count = conn
                .execute(&sql, &param_refs)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            Ok(count)
        })
    }

    fn count(&self, sql: &str, params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        let sql = Self::convert_params(sql);
        Box::pin(async move {
            debug!(sql = %sql, "Executing count");

            let mut conn = self
                .pool
                .get()
                .await
                .map_err(|e| prax_query::QueryError::connection(e.to_string()))?;

            let mssql_params = Self::to_params(&params)?;
            let param_refs: Vec<&dyn tiberius::ToSql> =
                mssql_params.iter().map(|p| p.as_ref()).collect();

            let row = conn
                .query_one(&sql, &param_refs)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            let count: i32 = row.get(0).unwrap_or(0);
            Ok(count as u64)
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
