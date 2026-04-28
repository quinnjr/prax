//! Create operation for inserting new records.

use std::marker::PhantomData;

use crate::error::QueryResult;
use crate::filter::FilterValue;
use crate::traits::{Model, QueryEngine};
use crate::types::Select;

/// A create operation for inserting a new record.
///
/// # Example
///
/// ```rust,ignore
/// let user = client
///     .user()
///     .create(user::Create {
///         email: "new@example.com".into(),
///         name: Some("New User".into()),
///     })
///     .exec()
///     .await?;
/// ```
pub struct CreateOperation<E: QueryEngine, M: Model> {
    engine: E,
    columns: Vec<String>,
    values: Vec<FilterValue>,
    select: Select,
    _model: PhantomData<M>,
}

impl<E: QueryEngine, M: Model + crate::row::FromRow> CreateOperation<E, M> {
    /// Create a new Create operation.
    pub fn new(engine: E) -> Self {
        Self {
            engine,
            columns: Vec::new(),
            values: Vec::new(),
            select: Select::All,
            _model: PhantomData,
        }
    }

    /// Set a column value.
    pub fn set(mut self, column: impl Into<String>, value: impl Into<FilterValue>) -> Self {
        self.columns.push(column.into());
        self.values.push(value.into());
        self
    }

    /// Set multiple column values from an iterator.
    pub fn set_many(
        mut self,
        values: impl IntoIterator<Item = (impl Into<String>, impl Into<FilterValue>)>,
    ) -> Self {
        for (col, val) in values {
            self.columns.push(col.into());
            self.values.push(val.into());
        }
        self
    }

    /// Select specific fields to return.
    pub fn select(mut self, select: impl Into<Select>) -> Self {
        self.select = select.into();
        self
    }

    /// Build the SQL query.
    pub fn build_sql(
        &self,
        dialect: &dyn crate::dialect::SqlDialect,
    ) -> (String, Vec<FilterValue>) {
        let mut sql = String::new();

        // INSERT INTO clause
        sql.push_str("INSERT INTO ");
        sql.push_str(M::TABLE_NAME);

        // Columns
        sql.push_str(" (");
        sql.push_str(&self.columns.join(", "));
        sql.push(')');

        // VALUES
        sql.push_str(" VALUES (");
        let placeholders: Vec<_> = (1..=self.values.len())
            .map(|i| dialect.placeholder(i))
            .collect();
        sql.push_str(&placeholders.join(", "));
        sql.push(')');

        // RETURNING clause
        sql.push_str(&dialect.returning_clause(&self.select.to_sql()));

        (sql, self.values.clone())
    }

    /// Execute the create operation and return the created record.
    pub async fn exec(self) -> QueryResult<M>
    where
        M: Send + 'static,
    {
        let dialect = self.engine.dialect();
        let (sql, params) = self.build_sql(dialect);
        self.engine.execute_insert::<M>(&sql, params).await
    }
}

/// Create many records at once.
pub struct CreateManyOperation<E: QueryEngine, M: Model> {
    engine: E,
    columns: Vec<String>,
    rows: Vec<Vec<FilterValue>>,
    skip_duplicates: bool,
    _model: PhantomData<M>,
}

impl<E: QueryEngine, M: Model> CreateManyOperation<E, M> {
    /// Create a new CreateMany operation.
    pub fn new(engine: E) -> Self {
        Self {
            engine,
            columns: Vec::new(),
            rows: Vec::new(),
            skip_duplicates: false,
            _model: PhantomData,
        }
    }

    /// Set the columns for insertion.
    pub fn columns(mut self, columns: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.columns = columns.into_iter().map(Into::into).collect();
        self
    }

    /// Add a row of values.
    pub fn row(mut self, values: impl IntoIterator<Item = impl Into<FilterValue>>) -> Self {
        self.rows.push(values.into_iter().map(Into::into).collect());
        self
    }

    /// Add multiple rows.
    pub fn rows(
        mut self,
        rows: impl IntoIterator<Item = impl IntoIterator<Item = impl Into<FilterValue>>>,
    ) -> Self {
        for row in rows {
            self.rows.push(row.into_iter().map(Into::into).collect());
        }
        self
    }

    /// Skip records that violate unique constraints.
    pub fn skip_duplicates(mut self) -> Self {
        self.skip_duplicates = true;
        self
    }

    /// Build the SQL query.
    pub fn build_sql(
        &self,
        dialect: &dyn crate::dialect::SqlDialect,
    ) -> (String, Vec<FilterValue>) {
        let mut sql = String::new();
        let mut all_params = Vec::new();

        // INSERT INTO clause
        sql.push_str("INSERT INTO ");
        sql.push_str(M::TABLE_NAME);

        // Columns
        sql.push_str(" (");
        sql.push_str(&self.columns.join(", "));
        sql.push(')');

        // VALUES
        sql.push_str(" VALUES ");

        let mut value_groups = Vec::new();
        let mut param_idx = 1;

        for row in &self.rows {
            let placeholders: Vec<_> = row
                .iter()
                .map(|v| {
                    all_params.push(v.clone());
                    let placeholder = dialect.placeholder(param_idx);
                    param_idx += 1;
                    placeholder
                })
                .collect();
            value_groups.push(format!("({})", placeholders.join(", ")));
        }

        sql.push_str(&value_groups.join(", "));

        // ON CONFLICT for skip_duplicates
        if self.skip_duplicates {
            sql.push_str(" ON CONFLICT DO NOTHING");
        }

        (sql, all_params)
    }

    /// Execute the create operation and return the number of created records.
    pub async fn exec(self) -> QueryResult<u64> {
        let dialect = self.engine.dialect();
        let (sql, params) = self.build_sql(dialect);
        self.engine.execute_raw(&sql, params).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::QueryError;

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

    #[derive(Clone)]
    struct MockEngine {
        insert_count: u64,
    }

    impl MockEngine {
        fn new() -> Self {
            Self { insert_count: 0 }
        }

        fn with_count(count: u64) -> Self {
            Self {
                insert_count: count,
            }
        }
    }

    impl QueryEngine for MockEngine {
        fn dialect(&self) -> &dyn crate::dialect::SqlDialect {
            &crate::dialect::Postgres
        }

        fn query_many<T: Model + crate::row::FromRow + Send + 'static>(
            &self,
            _sql: &str,
            _params: Vec<FilterValue>,
        ) -> crate::traits::BoxFuture<'_, QueryResult<Vec<T>>> {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn query_one<T: Model + crate::row::FromRow + Send + 'static>(
            &self,
            _sql: &str,
            _params: Vec<FilterValue>,
        ) -> crate::traits::BoxFuture<'_, QueryResult<T>> {
            Box::pin(async { Err(QueryError::not_found("test")) })
        }

        fn query_optional<T: Model + crate::row::FromRow + Send + 'static>(
            &self,
            _sql: &str,
            _params: Vec<FilterValue>,
        ) -> crate::traits::BoxFuture<'_, QueryResult<Option<T>>> {
            Box::pin(async { Ok(None) })
        }

        fn execute_insert<T: Model + crate::row::FromRow + Send + 'static>(
            &self,
            _sql: &str,
            _params: Vec<FilterValue>,
        ) -> crate::traits::BoxFuture<'_, QueryResult<T>> {
            Box::pin(async { Err(QueryError::not_found("test")) })
        }

        fn execute_update<T: Model + crate::row::FromRow + Send + 'static>(
            &self,
            _sql: &str,
            _params: Vec<FilterValue>,
        ) -> crate::traits::BoxFuture<'_, QueryResult<Vec<T>>> {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn execute_delete(
            &self,
            _sql: &str,
            _params: Vec<FilterValue>,
        ) -> crate::traits::BoxFuture<'_, QueryResult<u64>> {
            Box::pin(async { Ok(0) })
        }

        fn execute_raw(
            &self,
            _sql: &str,
            _params: Vec<FilterValue>,
        ) -> crate::traits::BoxFuture<'_, QueryResult<u64>> {
            let count = self.insert_count;
            Box::pin(async move { Ok(count) })
        }

        fn count(
            &self,
            _sql: &str,
            _params: Vec<FilterValue>,
        ) -> crate::traits::BoxFuture<'_, QueryResult<u64>> {
            Box::pin(async { Ok(0) })
        }
    }

    // ========== CreateOperation Tests ==========

    #[test]
    fn test_create_new() {
        let op = CreateOperation::<MockEngine, TestModel>::new(MockEngine::new());
        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("INSERT INTO test_models"));
        assert!(sql.contains("RETURNING *"));
        assert!(params.is_empty());
    }

    #[test]
    fn test_create_basic() {
        let op = CreateOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .set("name", "Alice")
            .set("email", "alice@example.com");

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("INSERT INTO test_models"));
        assert!(sql.contains("(name, email)"));
        assert!(sql.contains("VALUES ($1, $2)"));
        assert!(sql.contains("RETURNING *"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_create_single_field() {
        let op =
            CreateOperation::<MockEngine, TestModel>::new(MockEngine::new()).set("name", "Alice");

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("(name)"));
        assert!(sql.contains("VALUES ($1)"));
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_create_with_set_many() {
        let values = vec![
            ("name", FilterValue::String("Bob".to_string())),
            ("email", FilterValue::String("bob@test.com".to_string())),
            ("age", FilterValue::Int(25)),
        ];
        let op = CreateOperation::<MockEngine, TestModel>::new(MockEngine::new()).set_many(values);

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("(name, email, age)"));
        assert!(sql.contains("VALUES ($1, $2, $3)"));
        assert_eq!(params.len(), 3);
    }

    #[test]
    fn test_create_with_select() {
        let op = CreateOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .set("name", "Alice")
            .select(Select::fields(["id", "name"]));

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("RETURNING id, name"));
        assert!(!sql.contains("RETURNING *"));
    }

    #[test]
    fn test_create_with_null_value() {
        let op = CreateOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .set("name", "Alice")
            .set("nickname", FilterValue::Null);

        let (_sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert_eq!(params.len(), 2);
        assert_eq!(params[1], FilterValue::Null);
    }

    #[test]
    fn test_create_with_boolean_value() {
        let op = CreateOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .set("active", FilterValue::Bool(true));

        let (_, params) = op.build_sql(&crate::dialect::Postgres);

        assert_eq!(params[0], FilterValue::Bool(true));
    }

    #[test]
    fn test_create_with_numeric_values() {
        let op = CreateOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .set("count", FilterValue::Int(42))
            .set("price", FilterValue::Float(99.99));

        let (_, params) = op.build_sql(&crate::dialect::Postgres);

        assert_eq!(params[0], FilterValue::Int(42));
        assert_eq!(params[1], FilterValue::Float(99.99));
    }

    #[test]
    fn test_create_with_json_value() {
        let json = serde_json::json!({"key": "value", "nested": {"a": 1}});
        let op = CreateOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .set("metadata", FilterValue::Json(json.clone()));

        let (_, params) = op.build_sql(&crate::dialect::Postgres);

        assert_eq!(params[0], FilterValue::Json(json));
    }

    #[tokio::test]
    async fn test_create_exec() {
        let op =
            CreateOperation::<MockEngine, TestModel>::new(MockEngine::new()).set("name", "Alice");

        let result = op.exec().await;

        // MockEngine returns not_found error for execute_insert
        assert!(result.is_err());
    }

    // ========== CreateManyOperation Tests ==========

    #[test]
    fn test_create_many_new() {
        let op = CreateManyOperation::<MockEngine, TestModel>::new(MockEngine::new());
        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("INSERT INTO test_models"));
        assert!(!sql.contains("RETURNING")); // CreateMany doesn't return
        assert!(params.is_empty());
    }

    #[test]
    fn test_create_many() {
        let op = CreateManyOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .columns(["name", "email"])
            .row(["Alice", "alice@example.com"])
            .row(["Bob", "bob@example.com"]);

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("INSERT INTO test_models"));
        assert!(sql.contains("(name, email)"));
        assert!(sql.contains("VALUES ($1, $2), ($3, $4)"));
        assert_eq!(params.len(), 4);
    }

    #[test]
    fn test_create_many_single_row() {
        let op = CreateManyOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .columns(["name"])
            .row(["Alice"]);

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("VALUES ($1)"));
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_create_many_skip_duplicates() {
        let op = CreateManyOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .columns(["name", "email"])
            .row(["Alice", "alice@example.com"])
            .skip_duplicates();

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("ON CONFLICT DO NOTHING"));
    }

    #[test]
    fn test_create_many_without_skip_duplicates() {
        let op = CreateManyOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .columns(["name"])
            .row(["Alice"]);

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(!sql.contains("ON CONFLICT"));
    }

    #[test]
    fn test_create_many_with_rows() {
        let rows = vec![
            vec!["Alice", "alice@test.com"],
            vec!["Bob", "bob@test.com"],
            vec!["Charlie", "charlie@test.com"],
        ];
        let op = CreateManyOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .columns(["name", "email"])
            .rows(rows);

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("VALUES ($1, $2), ($3, $4), ($5, $6)"));
        assert_eq!(params.len(), 6);
    }

    #[test]
    fn test_create_many_param_ordering() {
        let op = CreateManyOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .columns(["a", "b"])
            .row(["1", "2"])
            .row(["3", "4"]);

        let (_, params) = op.build_sql(&crate::dialect::Postgres);

        // Params should be ordered: row1.a, row1.b, row2.a, row2.b
        assert_eq!(params[0], FilterValue::String("1".to_string()));
        assert_eq!(params[1], FilterValue::String("2".to_string()));
        assert_eq!(params[2], FilterValue::String("3".to_string()));
        assert_eq!(params[3], FilterValue::String("4".to_string()));
    }

    #[tokio::test]
    async fn test_create_many_exec() {
        let op = CreateManyOperation::<MockEngine, TestModel>::new(MockEngine::with_count(3))
            .columns(["name"])
            .row(["Alice"])
            .row(["Bob"])
            .row(["Charlie"]);

        let result = op.exec().await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 3);
    }

    // ========== SQL Structure Tests ==========

    #[test]
    fn test_create_sql_structure() {
        let op = CreateOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .set("name", "Alice")
            .select(Select::fields(["id"]));

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        let insert_pos = sql.find("INSERT INTO").unwrap();
        let columns_pos = sql.find("(name)").unwrap();
        let values_pos = sql.find("VALUES").unwrap();
        let returning_pos = sql.find("RETURNING").unwrap();

        assert!(insert_pos < columns_pos);
        assert!(columns_pos < values_pos);
        assert!(values_pos < returning_pos);
    }

    #[test]
    fn test_create_many_sql_structure() {
        let op = CreateManyOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .columns(["name", "email"])
            .row(["Alice", "alice@test.com"])
            .skip_duplicates();

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        let insert_pos = sql.find("INSERT INTO").unwrap();
        let columns_pos = sql.find("(name, email)").unwrap();
        let values_pos = sql.find("VALUES").unwrap();
        let conflict_pos = sql.find("ON CONFLICT").unwrap();

        assert!(insert_pos < columns_pos);
        assert!(columns_pos < values_pos);
        assert!(values_pos < conflict_pos);
    }

    #[test]
    fn test_create_table_name() {
        let op = CreateOperation::<MockEngine, TestModel>::new(MockEngine::new());
        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("test_models"));
    }

    // ========== Method Chaining Tests ==========

    #[test]
    fn test_create_method_chaining() {
        let op = CreateOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .set("name", "Alice")
            .set("email", "alice@test.com")
            .select(Select::fields(["id", "name"]));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("(name, email)"));
        assert!(sql.contains("VALUES ($1, $2)"));
        assert!(sql.contains("RETURNING id, name"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_create_many_method_chaining() {
        let op = CreateManyOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .columns(["a", "b"])
            .row(["1", "2"])
            .row(["3", "4"])
            .skip_duplicates();

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("ON CONFLICT DO NOTHING"));
        assert_eq!(params.len(), 4);
    }

    // ========== Cross-Dialect Tests ==========

    #[test]
    fn create_mssql_emits_output_inserted() {
        let op =
            CreateOperation::<MockEngine, TestModel>::new(MockEngine::new()).set("name", "Alice");
        let (sql, _) = op.build_sql(&crate::dialect::Mssql);
        assert!(
            sql.contains(" OUTPUT INSERTED.*"),
            "expected OUTPUT INSERTED.*, got: {sql}"
        );
    }

    #[test]
    fn create_mssql_emits_output_inserted_for_multiple_columns() {
        // Regression guard: the dialect-level test at
        // `dialect::tests::returning_mssql_is_output_inserted` verifies the
        // per-column prefix expansion of `Mssql::returning_clause`, but not
        // the wiring from the operation builder's `Select` list into that
        // clause. If a future refactor fails to pass the selected columns
        // through to the dialect, that path would silently fall back to
        // `OUTPUT INSERTED.*`. This test pins the end-to-end SQL emitted by
        // `CreateOperation::build_sql` when a narrow column list is set.
        let op = CreateOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .set("name", "Alice")
            .set("email", "alice@example.com")
            .select(Select::fields(["id", "email"]));

        let (sql, params) = op.build_sql(&crate::dialect::Mssql);
        assert!(
            sql.contains(" OUTPUT INSERTED.id, INSERTED.email"),
            "expected OUTPUT INSERTED.id, INSERTED.email, got: {sql}"
        );
        assert!(
            !sql.contains("INSERTED.*"),
            "narrow Select must not fall back to INSERTED.*: {sql}"
        );
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn create_postgres_emits_returning() {
        let op =
            CreateOperation::<MockEngine, TestModel>::new(MockEngine::new()).set("name", "Alice");
        let (sql, _) = op.build_sql(&crate::dialect::Postgres);
        assert!(sql.contains("RETURNING "), "expected RETURNING, got: {sql}");
    }
}
