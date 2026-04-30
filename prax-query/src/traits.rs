//! Core traits for the query builder.

use std::future::Future;
use std::pin::Pin;

use crate::error::QueryResult;
use crate::filter::Filter;

/// A model that can be queried.
pub trait Model: Sized + Send + Sync {
    /// The name of the model (used for table name).
    const MODEL_NAME: &'static str;

    /// The name of the database table.
    const TABLE_NAME: &'static str;

    /// The primary key column name(s).
    const PRIMARY_KEY: &'static [&'static str];

    /// All column names for this model.
    const COLUMNS: &'static [&'static str];
}

/// Runtime access to a model's primary key and column values.
///
/// Used by relation loaders (parent → child FK bucketing) and by upsert
/// to build the conflict-row lookup. Implemented by codegen for every
/// `#[derive(Model)]` struct and every `prax_schema!`-generated model.
///
/// Both methods return a [`crate::filter::FilterValue`] that mirrors
/// exactly what the matching `From<T>` impl on the binding side would
/// produce, so a PK value extracted here is a drop-in replacement for
/// the same value produced by an equivalent type-checked filter.
pub trait ModelWithPk: Model {
    /// Primary-key value for this row.
    ///
    /// Single-column PKs return the appropriate scalar variant.
    /// Composite PKs collapse to [`crate::filter::FilterValue::List`]
    /// in the same declaration order as [`Model::PRIMARY_KEY`].
    fn pk_value(&self) -> crate::filter::FilterValue;

    /// Look up a column by its SQL name.
    ///
    /// Returns `None` for column names not present in [`Model::COLUMNS`].
    /// The relation executor uses this to extract foreign-key values
    /// from a fetched parent row without knowing the concrete FK type.
    fn get_column_value(&self, column: &str) -> Option<crate::filter::FilterValue>;
}

/// A database view that can be queried (read-only).
///
/// Views are similar to models but only support read operations.
/// They cannot be inserted into, updated, or deleted from directly.
pub trait View: Sized + Send + Sync {
    /// The name of the view.
    const VIEW_NAME: &'static str;

    /// The name of the database view.
    const DB_VIEW_NAME: &'static str;

    /// All column names for this view.
    const COLUMNS: &'static [&'static str];

    /// Whether this is a materialized view.
    const IS_MATERIALIZED: bool;
}

/// A materialized view that supports refresh operations.
pub trait MaterializedView: View {
    /// Whether concurrent refresh is supported.
    const SUPPORTS_CONCURRENT_REFRESH: bool = true;
}

/// A type that can be converted into a filter.
pub trait IntoFilter {
    /// Convert this type into a filter.
    fn into_filter(self) -> Filter;
}

impl IntoFilter for Filter {
    fn into_filter(self) -> Filter {
        self
    }
}

impl<F: FnOnce() -> Filter> IntoFilter for F {
    fn into_filter(self) -> Filter {
        self()
    }
}

/// A query that can be executed.
pub trait Executable {
    /// The output type of the query.
    type Output;

    /// Execute the query and return the result.
    fn exec(self) -> impl Future<Output = QueryResult<Self::Output>> + Send;
}

/// A boxed future for async operations.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The query engine abstraction.
///
/// This trait defines how queries are executed against a database.
/// Different implementations can be provided for different databases
/// (PostgreSQL, MySQL, SQLite, etc.).
pub trait QueryEngine: Send + Sync + Clone + 'static {
    /// The SQL dialect this engine targets.
    ///
    /// Drivers that emit SQL (Postgres, MySQL, SQLite, MSSQL) override this
    /// to return their matching dialect so the shared `Operation` builders
    /// emit dialect-appropriate placeholders, `RETURNING` clauses, identifier
    /// quoting, and upsert syntax.
    ///
    /// The default returns `&crate::dialect::NotSql`, the inert dialect
    /// whose methods all panic if called. Non-SQL engines (MongoDB,
    /// document stores) can leave the default in place — their own
    /// operations never call SQL builders, so the panicking dialect is
    /// never invoked. If you implement `QueryEngine` for a SQL backend
    /// and forget to override this method, every attempt to build SQL
    /// through your engine will panic, which is the intended loud-failure
    /// mode.
    fn dialect(&self) -> &dyn crate::dialect::SqlDialect {
        &crate::dialect::NotSql
    }

    /// Execute a SELECT query and return rows.
    fn query_many<T: Model + crate::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<crate::filter::FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>>;

    /// Execute a SELECT query expecting one result.
    fn query_one<T: Model + crate::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<crate::filter::FilterValue>,
    ) -> BoxFuture<'_, QueryResult<T>>;

    /// Execute a SELECT query expecting zero or one result.
    fn query_optional<T: Model + crate::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<crate::filter::FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Option<T>>>;

    /// Execute an INSERT query and return the created row.
    fn execute_insert<T: Model + crate::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<crate::filter::FilterValue>,
    ) -> BoxFuture<'_, QueryResult<T>>;

    /// Execute an UPDATE query and return affected rows.
    fn execute_update<T: Model + crate::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<crate::filter::FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>>;

    /// Execute a DELETE query and return affected rows count.
    fn execute_delete(
        &self,
        sql: &str,
        params: Vec<crate::filter::FilterValue>,
    ) -> BoxFuture<'_, QueryResult<u64>>;

    /// Execute a raw SQL query.
    fn execute_raw(
        &self,
        sql: &str,
        params: Vec<crate::filter::FilterValue>,
    ) -> BoxFuture<'_, QueryResult<u64>>;

    /// Get a count of records.
    fn count(
        &self,
        sql: &str,
        params: Vec<crate::filter::FilterValue>,
    ) -> BoxFuture<'_, QueryResult<u64>>;

    /// Execute an aggregate query (COUNT/SUM/AVG/MIN/MAX/GROUP BY) and
    /// return one map of column name → [`crate::filter::FilterValue`]
    /// per result row.
    ///
    /// Used by [`crate::operations::AggregateOperation`] and
    /// [`crate::operations::GroupByOperation`] because aggregate result
    /// sets don't fit a single `Model` schema: their columns are
    /// dialect-chosen aliases (`_count`, `_sum_views`, …) whose types
    /// depend on the aggregate function, and group-by queries also
    /// include the grouped columns themselves. Returning untyped
    /// column-value maps lets the aggregate builders adapt the shape
    /// without every driver needing to generate a fresh `FromRow` impl
    /// per query.
    ///
    /// The default returns
    /// [`crate::error::QueryError::unsupported`], so non-SQL engines
    /// (MongoDB, document stores) that never build aggregate queries
    /// through the SQL operation builders don't have to implement this.
    /// SQL engines must override.
    fn aggregate_query(
        &self,
        sql: &str,
        params: Vec<crate::filter::FilterValue>,
    ) -> BoxFuture<
        '_,
        QueryResult<Vec<std::collections::HashMap<String, crate::filter::FilterValue>>>,
    > {
        let _ = (sql, params);
        Box::pin(async {
            Err(crate::error::QueryError::unsupported(
                "aggregate_query is not implemented for this engine",
            ))
        })
    }

    /// Refresh a materialized view.
    ///
    /// For PostgreSQL, this executes `REFRESH MATERIALIZED VIEW`.
    /// For MSSQL, this rebuilds the indexed view.
    /// For databases that don't support materialized views, this returns an error.
    fn refresh_materialized_view(
        &self,
        view_name: &str,
        concurrently: bool,
    ) -> BoxFuture<'_, QueryResult<()>> {
        let view_name = view_name.to_string();
        Box::pin(async move {
            let _ = (view_name, concurrently);
            Err(crate::error::QueryError::unsupported(
                "Materialized view refresh is not supported by this database",
            ))
        })
    }

    /// Run the closure inside a transaction.
    ///
    /// Drivers that support real transactions override this to issue
    /// `BEGIN` / `COMMIT` / `ROLLBACK` and route every query emitted
    /// by the closure through the same underlying transaction. The
    /// default below simply hands the closure a clone of the current
    /// engine and executes it inline — it has **no transactional
    /// semantics** on its own, so drivers that care about atomicity
    /// must override. The default exists so non-SQL backends
    /// (MongoDB, document stores) don't have to stub a method they
    /// don't care about.
    ///
    /// The `Self: Clone` bound lets the default clone the engine into
    /// the closure; every concrete `QueryEngine` already needs `Clone`
    /// for [`ModelAccessor`] routing, so it's free in practice.
    fn transaction<'a, R, Fut, F>(&'a self, f: F) -> BoxFuture<'a, QueryResult<R>>
    where
        F: FnOnce(Self) -> Fut + Send + 'a,
        Fut: Future<Output = QueryResult<R>> + Send + 'a,
        R: Send + 'a,
        Self: Clone,
    {
        let me = self.clone();
        Box::pin(async move { f(me).await })
    }
}

/// Query engine extension for view operations.
pub trait ViewQueryEngine: QueryEngine {
    /// Query rows from a view.
    fn query_view_many<V: View + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<crate::filter::FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<V>>>;

    /// Query a single row from a view.
    fn query_view_optional<V: View + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<crate::filter::FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Option<V>>>;

    /// Count rows in a view.
    fn count_view(
        &self,
        sql: &str,
        params: Vec<crate::filter::FilterValue>,
    ) -> BoxFuture<'_, QueryResult<u64>> {
        self.count(sql, params)
    }
}

/// A model accessor that provides query operations.
///
/// This is typically generated by the proc-macro for each model.
pub trait ModelAccessor<E: QueryEngine>: Send + Sync {
    /// The model type.
    type Model: Model;

    /// Get the query engine.
    fn engine(&self) -> &E;

    /// Start a find_many query.
    fn find_many(&self) -> crate::operations::FindManyOperation<E, Self::Model>;

    /// Start a find_unique query.
    fn find_unique(&self) -> crate::operations::FindUniqueOperation<E, Self::Model>;

    /// Start a find_first query.
    fn find_first(&self) -> crate::operations::FindFirstOperation<E, Self::Model>;

    /// Start a create operation.
    fn create(
        &self,
        data: <Self::Model as CreateData>::Data,
    ) -> crate::operations::CreateOperation<E, Self::Model>
    where
        Self::Model: CreateData;

    /// Start an update operation.
    fn update(&self) -> crate::operations::UpdateOperation<E, Self::Model>;

    /// Start a delete operation.
    fn delete(&self) -> crate::operations::DeleteOperation<E, Self::Model>;

    /// Start an upsert operation.
    fn upsert(
        &self,
        create: <Self::Model as CreateData>::Data,
        update: <Self::Model as UpdateData>::Data,
    ) -> crate::operations::UpsertOperation<E, Self::Model>
    where
        Self::Model: CreateData + UpdateData;

    /// Count records matching a filter.
    fn count(&self) -> crate::operations::CountOperation<E, Self::Model>;
}

/// Data for creating a new record.
pub trait CreateData: Model {
    /// The type that holds create data.
    type Data: Send + Sync;
}

/// Data for updating an existing record.
pub trait UpdateData: Model {
    /// The type that holds update data.
    type Data: Send + Sync;
}

/// Data for upserting a record.
pub trait UpsertData: CreateData + UpdateData {}

impl<T: CreateData + UpdateData> UpsertData for T {}

/// Trait for models that support eager loading of relations.
pub trait WithRelations: Model {
    /// The type of include specification.
    type Include;

    /// The type of select specification.
    type Select;
}

/// Routes a relation-include request to the right executor call.
///
/// Every `#[derive(Model)]` (and `prax_schema!`-generated model) emits
/// an impl of this trait. Models with no relations get a trivial impl
/// that errors on any unknown relation name; models with relations
/// dispatch each name to [`crate::relations::executor::load_has_many`]
/// and splice the results onto the parent slice.
///
/// Implementing this as a model-side trait — rather than carrying a
/// `Vec<Box<dyn Loader>>` on the find-operation builder — keeps the
/// executor fully monomorphic and lets `include(...)` remain a simple
/// `String`-keyed lookup against the model's match arms.
pub trait ModelRelationLoader<E: QueryEngine>: Sized {
    /// Load every relation named by `spec` onto the `parents` slice.
    ///
    /// The slice is mutated in place — each parent's relation field is
    /// set to the bucketed child collection. Models with no relations
    /// return an `internal` [`crate::error::QueryError`] for any name.
    fn load_relation<'a>(
        engine: &'a E,
        parents: &'a mut [Self],
        spec: &'a crate::relations::IncludeSpec,
    ) -> BoxFuture<'a, crate::error::QueryResult<()>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestModel;

    impl Model for TestModel {
        const MODEL_NAME: &'static str = "TestModel";
        const TABLE_NAME: &'static str = "test_models";
        const PRIMARY_KEY: &'static [&'static str] = &["id"];
        const COLUMNS: &'static [&'static str] = &["id", "name", "email"];
    }

    impl crate::row::FromRow for TestModel {
        fn from_row(_row: &impl crate::row::RowRef) -> Result<Self, crate::row::RowError> {
            Ok(TestModel)
        }
    }

    #[test]
    fn test_model_trait() {
        assert_eq!(TestModel::MODEL_NAME, "TestModel");
        assert_eq!(TestModel::TABLE_NAME, "test_models");
        assert_eq!(TestModel::PRIMARY_KEY, &["id"]);
    }

    #[test]
    fn test_into_filter() {
        let filter = Filter::Equals("id".into(), crate::filter::FilterValue::Int(1));
        let converted = filter.clone().into_filter();
        assert_eq!(converted, filter);
    }

    #[test]
    #[should_panic(expected = "NotSql dialect does not emit SQL")]
    fn query_engine_dialect_defaults_to_not_sql() {
        // A minimal QueryEngine impl that doesn't override dialect() should
        // inherit the NotSql default so external implementors aren't forced
        // to add a method they don't care about.
        use crate::filter::FilterValue;

        #[derive(Clone)]
        struct DefaultEngine;

        impl QueryEngine for DefaultEngine {
            fn query_many<T: Model + crate::row::FromRow + Send + 'static>(
                &self,
                _sql: &str,
                _params: Vec<FilterValue>,
            ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
                Box::pin(async { Ok(Vec::new()) })
            }

            fn query_one<T: Model + crate::row::FromRow + Send + 'static>(
                &self,
                _sql: &str,
                _params: Vec<FilterValue>,
            ) -> BoxFuture<'_, QueryResult<T>> {
                Box::pin(async { Err(crate::error::QueryError::not_found("test")) })
            }

            fn query_optional<T: Model + crate::row::FromRow + Send + 'static>(
                &self,
                _sql: &str,
                _params: Vec<FilterValue>,
            ) -> BoxFuture<'_, QueryResult<Option<T>>> {
                Box::pin(async { Ok(None) })
            }

            fn execute_insert<T: Model + crate::row::FromRow + Send + 'static>(
                &self,
                _sql: &str,
                _params: Vec<FilterValue>,
            ) -> BoxFuture<'_, QueryResult<T>> {
                Box::pin(async { Err(crate::error::QueryError::not_found("test")) })
            }

            fn execute_update<T: Model + crate::row::FromRow + Send + 'static>(
                &self,
                _sql: &str,
                _params: Vec<FilterValue>,
            ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
                Box::pin(async { Ok(Vec::new()) })
            }

            fn execute_delete(
                &self,
                _sql: &str,
                _params: Vec<FilterValue>,
            ) -> BoxFuture<'_, QueryResult<u64>> {
                Box::pin(async { Ok(0) })
            }

            fn execute_raw(
                &self,
                _sql: &str,
                _params: Vec<FilterValue>,
            ) -> BoxFuture<'_, QueryResult<u64>> {
                Box::pin(async { Ok(0) })
            }

            fn count(
                &self,
                _sql: &str,
                _params: Vec<FilterValue>,
            ) -> BoxFuture<'_, QueryResult<u64>> {
                Box::pin(async { Ok(0) })
            }

            // Note: dialect() is NOT overridden - we're testing the default
        }

        let e = DefaultEngine;
        // If the default ever regresses back to a SQL-emitting dialect, this
        // test will fail because placeholder() won't panic.
        let _ = e.dialect().placeholder(1);
    }
}
