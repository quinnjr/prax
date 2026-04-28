//! Upsert operation for creating or updating records.

use std::marker::PhantomData;

use crate::error::QueryResult;
use crate::filter::{Filter, FilterValue};
use crate::traits::{Model, QueryEngine};
use crate::types::Select;

/// An upsert (insert or update) operation.
///
/// # Example
///
/// ```rust,ignore
/// let user = client
///     .user()
///     .upsert()
///     .r#where(user::email::equals("test@example.com"))
///     .create(user::Create { email: "test@example.com".into(), name: Some("Test".into()) })
///     .update(user::Update { name: Some("Updated".into()), ..Default::default() })
///     .exec()
///     .await?;
/// ```
pub struct UpsertOperation<E: QueryEngine, M: Model> {
    engine: E,
    filter: Filter,
    create_columns: Vec<String>,
    create_values: Vec<FilterValue>,
    update_columns: Vec<String>,
    update_values: Vec<FilterValue>,
    conflict_columns: Vec<String>,
    select: Select,
    _model: PhantomData<M>,
}

impl<E: QueryEngine, M: Model + crate::row::FromRow> UpsertOperation<E, M> {
    /// Create a new Upsert operation.
    pub fn new(engine: E) -> Self {
        Self {
            engine,
            filter: Filter::None,
            create_columns: Vec::new(),
            create_values: Vec::new(),
            update_columns: Vec::new(),
            update_values: Vec::new(),
            conflict_columns: Vec::new(),
            select: Select::All,
            _model: PhantomData,
        }
    }

    /// Add a filter condition (identifies the record to upsert).
    pub fn r#where(mut self, filter: impl Into<Filter>) -> Self {
        self.filter = filter.into();
        self
    }

    /// Set the columns to check for conflict.
    pub fn on_conflict(mut self, columns: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.conflict_columns = columns.into_iter().map(Into::into).collect();
        self
    }

    /// Set the create data.
    pub fn create(
        mut self,
        values: impl IntoIterator<Item = (impl Into<String>, impl Into<FilterValue>)>,
    ) -> Self {
        for (col, val) in values {
            self.create_columns.push(col.into());
            self.create_values.push(val.into());
        }
        self
    }

    /// Set a single create column.
    pub fn create_set(mut self, column: impl Into<String>, value: impl Into<FilterValue>) -> Self {
        self.create_columns.push(column.into());
        self.create_values.push(value.into());
        self
    }

    /// Set the update data.
    pub fn update(
        mut self,
        values: impl IntoIterator<Item = (impl Into<String>, impl Into<FilterValue>)>,
    ) -> Self {
        for (col, val) in values {
            self.update_columns.push(col.into());
            self.update_values.push(val.into());
        }
        self
    }

    /// Set a single update column.
    pub fn update_set(mut self, column: impl Into<String>, value: impl Into<FilterValue>) -> Self {
        self.update_columns.push(column.into());
        self.update_values.push(value.into());
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
        let mut params = Vec::new();
        let mut param_idx = 1;

        // INSERT INTO clause
        sql.push_str("INSERT INTO ");
        sql.push_str(M::TABLE_NAME);

        // Columns
        sql.push_str(" (");
        sql.push_str(&self.create_columns.join(", "));
        sql.push(')');

        // VALUES
        sql.push_str(" VALUES (");
        let placeholders: Vec<_> = self
            .create_values
            .iter()
            .map(|v| {
                params.push(v.clone());
                let p = dialect.placeholder(param_idx);
                param_idx += 1;
                p
            })
            .collect();
        sql.push_str(&placeholders.join(", "));
        sql.push(')');

        // Upsert clause (ON CONFLICT / ON DUPLICATE KEY)
        // Build update SET clause
        let update_set = if !self.update_columns.is_empty() {
            let update_parts: Vec<_> = self
                .update_columns
                .iter()
                .zip(self.update_values.iter())
                .map(|(col, val)| {
                    params.push(val.clone());
                    let part = format!("{} = {}", col, dialect.placeholder(param_idx));
                    param_idx += 1;
                    part
                })
                .collect();
            update_parts.join(", ")
        } else {
            String::new()
        };

        let conflict_cols: Vec<&str> = self.conflict_columns.iter().map(|s| s.as_str()).collect();

        if update_set.is_empty() {
            // DO NOTHING variant
            if conflict_cols.is_empty() {
                sql.push_str(" ON CONFLICT DO NOTHING");
            } else {
                sql.push_str(" ON CONFLICT (");
                sql.push_str(&conflict_cols.join(", "));
                sql.push_str(") DO NOTHING");
            }
        } else {
            // Use dialect's upsert_clause for DO UPDATE SET
            sql.push_str(&dialect.upsert_clause(&conflict_cols, &update_set));
        }

        // RETURNING clause
        sql.push_str(&dialect.returning_clause(&self.select.to_sql()));

        (sql, params)
    }

    /// Execute the upsert and return the record.
    pub async fn exec(self) -> QueryResult<M>
    where
        M: Send + 'static,
    {
        let dialect = self.engine.dialect();
        let (sql, params) = self.build_sql(dialect);
        self.engine.execute_insert::<M>(&sql, params).await
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
    struct MockEngine;

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
            Box::pin(async { Ok(0) })
        }

        fn count(
            &self,
            _sql: &str,
            _params: Vec<FilterValue>,
        ) -> crate::traits::BoxFuture<'_, QueryResult<u64>> {
            Box::pin(async { Ok(0) })
        }
    }

    // ========== Construction Tests ==========

    #[test]
    fn test_upsert_new() {
        let op = UpsertOperation::<MockEngine, TestModel>::new(MockEngine);
        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("INSERT INTO test_models"));
        assert!(sql.contains("ON CONFLICT"));
        assert!(sql.contains("RETURNING *"));
        assert!(params.is_empty());
    }

    #[test]
    fn test_upsert_basic() {
        let op = UpsertOperation::<MockEngine, TestModel>::new(MockEngine)
            .on_conflict(["email"])
            .create_set("email", "test@example.com")
            .create_set("name", "Test")
            .update_set("name", "Updated");

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("INSERT INTO test_models"));
        assert!(sql.contains("ON CONFLICT (email)"));
        assert!(sql.contains("DO UPDATE SET"));
        assert!(sql.contains("RETURNING *"));
        assert_eq!(params.len(), 3); // 2 create + 1 update
    }

    // ========== Conflict Column Tests ==========

    #[test]
    fn test_upsert_single_conflict_column() {
        let op = UpsertOperation::<MockEngine, TestModel>::new(MockEngine)
            .on_conflict(["id"])
            .create_set("id", FilterValue::Int(1));

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("ON CONFLICT (id)"));
    }

    #[test]
    fn test_upsert_multiple_conflict_columns() {
        let op = UpsertOperation::<MockEngine, TestModel>::new(MockEngine)
            .on_conflict(["tenant_id", "email"])
            .create_set("email", "test@example.com")
            .create_set("tenant_id", FilterValue::Int(1));

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("ON CONFLICT (tenant_id, email)"));
    }

    #[test]
    fn test_upsert_without_conflict_columns() {
        let op = UpsertOperation::<MockEngine, TestModel>::new(MockEngine)
            .create_set("email", "test@example.com");

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("ON CONFLICT"));
        assert!(!sql.contains("ON CONFLICT ("));
    }

    // ========== Create Tests ==========

    #[test]
    fn test_upsert_create_with_set() {
        let op = UpsertOperation::<MockEngine, TestModel>::new(MockEngine)
            .on_conflict(["email"])
            .create_set("email", "test@example.com")
            .create_set("name", "Test User");

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("(email, name)"));
        assert!(sql.contains("VALUES ($1, $2)"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_upsert_create_with_iterator() {
        let create_data = vec![
            ("email", FilterValue::String("test@example.com".to_string())),
            ("name", FilterValue::String("Test User".to_string())),
            ("age", FilterValue::Int(25)),
        ];
        let op = UpsertOperation::<MockEngine, TestModel>::new(MockEngine)
            .on_conflict(["email"])
            .create(create_data);

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("(email, name, age)"));
        assert!(sql.contains("VALUES ($1, $2, $3)"));
        assert_eq!(params.len(), 3);
    }

    // ========== Update Tests ==========

    #[test]
    fn test_upsert_update_with_set() {
        let op = UpsertOperation::<MockEngine, TestModel>::new(MockEngine)
            .on_conflict(["email"])
            .create_set("email", "test@example.com")
            .update_set("name", "Updated Name")
            .update_set("updated_at", "2024-01-01");

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("DO UPDATE SET"));
        assert!(sql.contains("name = $"));
        assert!(sql.contains("updated_at = $"));
        assert_eq!(params.len(), 3); // 1 create + 2 update
    }

    #[test]
    fn test_upsert_update_with_iterator() {
        let update_data = vec![
            ("name", FilterValue::String("Updated".to_string())),
            ("status", FilterValue::String("active".to_string())),
        ];
        let op = UpsertOperation::<MockEngine, TestModel>::new(MockEngine)
            .on_conflict(["id"])
            .create_set("id", FilterValue::Int(1))
            .update(update_data);

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("DO UPDATE SET"));
        assert_eq!(params.len(), 3); // 1 create + 2 update
    }

    // ========== Do Nothing Tests ==========

    #[test]
    fn test_upsert_do_nothing() {
        let op = UpsertOperation::<MockEngine, TestModel>::new(MockEngine)
            .on_conflict(["email"])
            .create_set("email", "test@example.com");

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("DO NOTHING"));
        assert!(!sql.contains("DO UPDATE"));
    }

    #[test]
    fn test_upsert_do_nothing_multiple_create() {
        let op = UpsertOperation::<MockEngine, TestModel>::new(MockEngine)
            .on_conflict(["email"])
            .create_set("email", "test@example.com")
            .create_set("name", "Test");

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("DO NOTHING"));
        assert_eq!(params.len(), 2);
    }

    // ========== Select Tests ==========

    #[test]
    fn test_upsert_with_select() {
        let op = UpsertOperation::<MockEngine, TestModel>::new(MockEngine)
            .on_conflict(["email"])
            .create_set("email", "test@example.com")
            .update_set("name", "Updated")
            .select(Select::fields(["id", "email"]));

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("RETURNING id, email"));
        assert!(!sql.contains("RETURNING *"));
    }

    #[test]
    fn test_upsert_select_all() {
        let op = UpsertOperation::<MockEngine, TestModel>::new(MockEngine)
            .on_conflict(["email"])
            .create_set("email", "test@example.com")
            .select(Select::All);

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("RETURNING *"));
    }

    // ========== Where Filter Tests ==========

    #[test]
    fn test_upsert_with_where() {
        let op = UpsertOperation::<MockEngine, TestModel>::new(MockEngine)
            .r#where(Filter::Equals(
                "email".into(),
                FilterValue::String("test@example.com".to_string()),
            ))
            .on_conflict(["email"])
            .create_set("email", "test@example.com");

        let (_, _) = op.build_sql(&crate::dialect::Postgres);
        // where_ sets the filter but doesn't affect upsert SQL directly
    }

    // ========== SQL Structure Tests ==========

    #[test]
    fn test_upsert_sql_structure() {
        let op = UpsertOperation::<MockEngine, TestModel>::new(MockEngine)
            .on_conflict(["email"])
            .create_set("email", "test@example.com")
            .update_set("name", "Updated")
            .select(Select::fields(["id"]));

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        let insert_pos = sql.find("INSERT INTO").unwrap();
        let values_pos = sql.find("VALUES").unwrap();
        let conflict_pos = sql.find("ON CONFLICT").unwrap();
        let update_pos = sql.find("DO UPDATE SET").unwrap();
        let returning_pos = sql.find("RETURNING").unwrap();

        assert!(insert_pos < values_pos);
        assert!(values_pos < conflict_pos);
        assert!(conflict_pos < update_pos);
        assert!(update_pos < returning_pos);
    }

    #[test]
    fn test_upsert_table_name() {
        let op = UpsertOperation::<MockEngine, TestModel>::new(MockEngine);
        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("test_models"));
    }

    // ========== Param Ordering Tests ==========

    #[test]
    fn test_upsert_param_ordering() {
        let op = UpsertOperation::<MockEngine, TestModel>::new(MockEngine)
            .on_conflict(["email"])
            .create_set("email", "create@test.com")
            .create_set("name", "Create Name")
            .update_set("name", "Update Name");

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        // Create params first, then update params
        assert!(sql.contains("VALUES ($1, $2)"));
        assert!(sql.contains("name = $3"));
        assert_eq!(params.len(), 3);
        assert_eq!(
            params[0],
            FilterValue::String("create@test.com".to_string())
        );
        assert_eq!(params[1], FilterValue::String("Create Name".to_string()));
        assert_eq!(params[2], FilterValue::String("Update Name".to_string()));
    }

    // ========== Async Execution Tests ==========

    #[tokio::test]
    async fn test_upsert_exec() {
        let op = UpsertOperation::<MockEngine, TestModel>::new(MockEngine)
            .on_conflict(["email"])
            .create_set("email", "test@example.com");

        let result = op.exec().await;

        // MockEngine returns not_found for execute_insert
        assert!(result.is_err());
    }

    // ========== Method Chaining Tests ==========

    #[test]
    fn test_upsert_full_chain() {
        let op = UpsertOperation::<MockEngine, TestModel>::new(MockEngine)
            .r#where(Filter::Equals(
                "email".into(),
                FilterValue::String("test@example.com".to_string()),
            ))
            .on_conflict(["email"])
            .create_set("email", "test@example.com")
            .create_set("name", "Test User")
            .update_set("name", "Updated User")
            .select(Select::fields(["id", "name", "email"]));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("INSERT INTO test_models"));
        assert!(sql.contains("ON CONFLICT (email)"));
        assert!(sql.contains("DO UPDATE SET"));
        assert!(sql.contains("RETURNING id, name, email"));
        assert_eq!(params.len(), 3);
    }

    // ========== Value Type Tests ==========

    #[test]
    fn test_upsert_with_null_value() {
        let op = UpsertOperation::<MockEngine, TestModel>::new(MockEngine)
            .on_conflict(["id"])
            .create_set("id", FilterValue::Int(1))
            .create_set("nickname", FilterValue::Null);

        let (_, params) = op.build_sql(&crate::dialect::Postgres);

        assert_eq!(params[1], FilterValue::Null);
    }

    #[test]
    fn test_upsert_with_boolean_value() {
        let op = UpsertOperation::<MockEngine, TestModel>::new(MockEngine)
            .on_conflict(["id"])
            .create_set("id", FilterValue::Int(1))
            .create_set("active", FilterValue::Bool(true))
            .update_set("active", FilterValue::Bool(false));

        let (_, params) = op.build_sql(&crate::dialect::Postgres);

        assert_eq!(params[1], FilterValue::Bool(true));
        assert_eq!(params[2], FilterValue::Bool(false));
    }

    #[test]
    fn test_upsert_with_numeric_values() {
        let op = UpsertOperation::<MockEngine, TestModel>::new(MockEngine)
            .on_conflict(["id"])
            .create_set("id", FilterValue::Int(1))
            .create_set("score", FilterValue::Float(99.5));

        let (_, params) = op.build_sql(&crate::dialect::Postgres);

        assert_eq!(params[0], FilterValue::Int(1));
        assert_eq!(params[1], FilterValue::Float(99.5));
    }

    #[test]
    fn test_upsert_with_json_value() {
        let json = serde_json::json!({"key": "value"});
        let op = UpsertOperation::<MockEngine, TestModel>::new(MockEngine)
            .on_conflict(["id"])
            .create_set("id", FilterValue::Int(1))
            .create_set("metadata", FilterValue::Json(json.clone()));

        let (_, params) = op.build_sql(&crate::dialect::Postgres);

        assert_eq!(params[1], FilterValue::Json(json));
    }
}
