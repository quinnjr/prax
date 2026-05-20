//! Update operation for modifying existing records.

use std::marker::PhantomData;

use crate::error::QueryResult;
use crate::filter::{Filter, FilterValue};
use crate::inputs::WriteOp;
use crate::traits::{Model, QueryEngine};
use crate::types::Select;

/// An update operation for modifying existing records.
///
/// # Example
///
/// ```rust,ignore
/// let users = client
///     .user()
///     .update()
///     .r#where(user::id::equals(1))
///     .set("name", "Updated Name")
///     .exec()
///     .await?;
/// ```
pub struct UpdateOperation<E: QueryEngine, M: Model> {
    engine: E,
    filter: Filter,
    updates: Vec<(String, WriteOp)>,
    select: Select,
    _model: PhantomData<M>,
}

impl<E: QueryEngine, M: Model + crate::row::FromRow> UpdateOperation<E, M> {
    /// Create a new Update operation.
    pub fn new(engine: E) -> Self {
        Self {
            engine,
            filter: Filter::None,
            updates: Vec::new(),
            select: Select::All,
            _model: PhantomData,
        }
    }

    /// Add a filter condition.
    pub fn r#where(mut self, filter: impl Into<Filter>) -> Self {
        let new_filter = filter.into();
        self.filter = self.filter.and_then(new_filter);
        self
    }

    /// Set a column to a new value.
    pub fn set(mut self, column: impl Into<String>, value: impl Into<FilterValue>) -> Self {
        self.updates
            .push((column.into(), WriteOp::Set(value.into())));
        self
    }

    /// Set multiple columns from an iterator.
    pub fn set_many(
        mut self,
        values: impl IntoIterator<Item = (impl Into<String>, impl Into<FilterValue>)>,
    ) -> Self {
        for (col, val) in values {
            self.updates.push((col.into(), WriteOp::Set(val.into())));
        }
        self
    }

    /// Increment a numeric column.
    pub fn increment(mut self, column: impl Into<String>, amount: i64) -> Self {
        self.updates
            .push((column.into(), WriteOp::Increment(FilterValue::Int(amount))));
        self
    }

    /// Apply a column-keyed [`WriteOp`].
    ///
    /// Used by `with_update_input` (and tests) to push an arbitrary
    /// scalar atomic operator onto the update list. The DSL surface
    /// for these operators is the `*FieldUpdate` wrappers in
    /// [`crate::inputs::scalar_update`].
    pub fn set_op(mut self, column: impl Into<String>, op: WriteOp) -> Self {
        self.updates.push((column.into(), op));
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

        // UPDATE clause
        sql.push_str("UPDATE ");
        sql.push_str(M::TABLE_NAME);

        // SET clause
        sql.push_str(" SET ");
        let set_parts: Vec<String> = self
            .updates
            .iter()
            .map(|(col, op)| {
                let placeholder = dialect.placeholder(param_idx);
                let (fragment, value) = op.to_set_fragment(col, &placeholder);
                if let Some(v) = value {
                    params.push(v);
                    param_idx += 1;
                }
                fragment
            })
            .collect();
        sql.push_str(&set_parts.join(", "));

        // WHERE clause
        if !self.filter.is_none() {
            let (where_sql, where_params) = self.filter.to_sql(param_idx - 1, dialect);
            sql.push_str(" WHERE ");
            sql.push_str(&where_sql);
            params.extend(where_params);
        }

        // RETURNING clause
        sql.push_str(&dialect.returning_clause(&self.select.to_sql()));

        (sql, params)
    }

    /// Execute the update and return modified records.
    pub async fn exec(self) -> QueryResult<Vec<M>>
    where
        M: Send + 'static,
    {
        let dialect = self.engine.dialect();
        let (sql, params) = self.build_sql(dialect);
        self.engine.execute_update::<M>(&sql, params).await
    }

    /// Execute the update and return the first modified record.
    pub async fn exec_one(self) -> QueryResult<M>
    where
        M: Send + 'static,
    {
        let dialect = self.engine.dialect();
        let (sql, params) = self.build_sql(dialect);
        self.engine.query_one::<M>(&sql, params).await
    }

    /// Apply a typed `WhereUniqueInput`. AND-composes with any
    /// previously set filter so callers can combine the unique key
    /// with side conditions when they need to.
    pub fn with_where_input<W: crate::inputs::WhereUniqueInput<Model = M>>(mut self, w: W) -> Self {
        let f = w.into_ir();
        self.filter = self.filter.and_then(f);
        self
    }

    /// Apply a typed `SelectInput`.
    pub fn with_select_input<S: crate::inputs::SelectInput<Model = M>>(mut self, s: S) -> Self {
        self.select = s.into_ir();
        self
    }

    /// Apply a typed `UpdateInput`.
    ///
    /// The input's `into_ir` produces a `Vec<(column, WriteOp)>` —
    /// each entry is appended to the operation's SET list. Atomic
    /// operators (`Increment`/`Decrement`/`Multiply`/`Divide`) emit
    /// `col = col <op> $n` in the resulting SQL; `Set` emits
    /// `col = $n`; `Unset` emits `col = NULL` with no placeholder.
    pub fn with_update_input<I>(mut self, input: I) -> Self
    where
        I: crate::inputs::UpdateInput<Model = M, Data = crate::inputs::UpdatePayload>,
    {
        let data: crate::inputs::UpdatePayload = input.into_ir();
        for (col, op) in data {
            self.updates.push((col, op));
        }
        self
    }

    /// Doc-hidden accessor for the current filter.
    #[doc(hidden)]
    pub fn filter_for_test(&self) -> &Filter {
        &self.filter
    }
}

/// Update many records at once.
pub struct UpdateManyOperation<E: QueryEngine, M: Model> {
    engine: E,
    filter: Filter,
    updates: Vec<(String, WriteOp)>,
    _model: PhantomData<M>,
}

impl<E: QueryEngine, M: Model> UpdateManyOperation<E, M> {
    /// Create a new UpdateMany operation.
    pub fn new(engine: E) -> Self {
        Self {
            engine,
            filter: Filter::None,
            updates: Vec::new(),
            _model: PhantomData,
        }
    }

    /// Add a filter condition.
    pub fn r#where(mut self, filter: impl Into<Filter>) -> Self {
        let new_filter = filter.into();
        self.filter = self.filter.and_then(new_filter);
        self
    }

    /// Set a column to a new value.
    pub fn set(mut self, column: impl Into<String>, value: impl Into<FilterValue>) -> Self {
        self.updates
            .push((column.into(), WriteOp::Set(value.into())));
        self
    }

    /// Apply a column-keyed [`WriteOp`].
    pub fn set_op(mut self, column: impl Into<String>, op: WriteOp) -> Self {
        self.updates.push((column.into(), op));
        self
    }

    /// Apply a typed `WhereInput`. AND-composes with the existing filter.
    pub fn with_where_input<W: crate::inputs::WhereInput<Model = M>>(mut self, w: W) -> Self {
        let f = w.into_ir();
        self.filter = self.filter.and_then(f);
        self
    }

    /// Apply a typed `UpdateInput`.
    ///
    /// See [`UpdateOperation::with_update_input`] for the lowering
    /// semantics — the only difference here is that `update_many` does
    /// not return rows.
    pub fn with_update_input<I>(mut self, input: I) -> Self
    where
        I: crate::inputs::UpdateInput<Model = M, Data = crate::inputs::UpdatePayload>,
    {
        let data: crate::inputs::UpdatePayload = input.into_ir();
        for (col, op) in data {
            self.updates.push((col, op));
        }
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

        // UPDATE clause
        sql.push_str("UPDATE ");
        sql.push_str(M::TABLE_NAME);

        // SET clause
        sql.push_str(" SET ");
        let set_parts: Vec<String> = self
            .updates
            .iter()
            .map(|(col, op)| {
                let placeholder = dialect.placeholder(param_idx);
                let (fragment, value) = op.to_set_fragment(col, &placeholder);
                if let Some(v) = value {
                    params.push(v);
                    param_idx += 1;
                }
                fragment
            })
            .collect();
        sql.push_str(&set_parts.join(", "));

        // WHERE clause
        if !self.filter.is_none() {
            let (where_sql, where_params) = self.filter.to_sql(param_idx - 1, dialect);
            sql.push_str(" WHERE ");
            sql.push_str(&where_sql);
            params.extend(where_params);
        }

        (sql, params)
    }

    /// Execute the update and return the count of modified records.
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
    use crate::types::Select;

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
        return_count: u64,
    }

    impl MockEngine {
        fn new() -> Self {
            Self { return_count: 0 }
        }

        fn with_count(count: u64) -> Self {
            Self {
                return_count: count,
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
            let count = self.return_count;
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

    // ========== UpdateOperation Tests ==========

    #[test]
    fn test_update_new() {
        let op = UpdateOperation::<MockEngine, TestModel>::new(MockEngine::new());
        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("UPDATE test_models SET"));
        assert!(sql.contains("RETURNING *"));
        assert!(params.is_empty());
    }

    #[test]
    fn test_update_basic() {
        let op = UpdateOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .r#where(Filter::Equals("id".into(), FilterValue::Int(1)))
            .set("name", "Updated");

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("UPDATE test_models SET"));
        assert!(sql.contains("name = $1"));
        assert!(sql.contains("WHERE"));
        assert!(sql.contains("RETURNING *"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_update_many_fields() {
        let op = UpdateOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .set("name", "Updated")
            .set("email", "updated@example.com");

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("name = $1"));
        assert!(sql.contains("email = $2"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_update_with_set_many() {
        let updates = vec![
            ("name", FilterValue::String("Alice".to_string())),
            ("email", FilterValue::String("alice@test.com".to_string())),
            ("age", FilterValue::Int(30)),
        ];
        let op = UpdateOperation::<MockEngine, TestModel>::new(MockEngine::new()).set_many(updates);

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("name = $1"));
        assert!(sql.contains("email = $2"));
        assert!(sql.contains("age = $3"));
        assert_eq!(params.len(), 3);
    }

    #[test]
    fn test_update_increment() {
        let op = UpdateOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .increment("counter", 5);

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        // `increment` now lowers to a true `col = col + $n` atomic
        // operator (the prior implementation collapsed it to a plain
        // `set`, which was a documented bug).
        assert!(
            sql.contains("counter = counter + $1"),
            "expected `counter = counter + $1`, got: {sql}"
        );
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], FilterValue::Int(5));
    }

    #[test]
    fn test_update_with_select() {
        let op = UpdateOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .set("name", "Updated")
            .select(Select::fields(["id", "name"]));

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("RETURNING id, name"));
    }

    #[test]
    fn test_update_with_complex_filter() {
        let op = UpdateOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .r#where(Filter::Equals(
                "status".into(),
                FilterValue::String("active".to_string()),
            ))
            .r#where(Filter::Gt("age".into(), FilterValue::Int(18)))
            .set("verified", FilterValue::Bool(true));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("WHERE"));
        assert!(sql.contains("AND"));
        assert_eq!(params.len(), 3); // 1 set + 2 where
    }

    #[test]
    fn test_update_without_filter() {
        let op = UpdateOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .set("status", "updated");

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        // Should not have WHERE clause
        assert!(!sql.contains("WHERE"));
        assert!(sql.contains("UPDATE test_models SET"));
    }

    #[test]
    fn test_update_with_null_value() {
        let op = UpdateOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .set("deleted_at", FilterValue::Null);

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("deleted_at = $1"));
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], FilterValue::Null);
    }

    #[test]
    fn test_update_with_boolean() {
        let op = UpdateOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .set("active", FilterValue::Bool(true))
            .set("verified", FilterValue::Bool(false));

        let (_sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert_eq!(params.len(), 2);
        assert_eq!(params[0], FilterValue::Bool(true));
        assert_eq!(params[1], FilterValue::Bool(false));
    }

    #[tokio::test]
    async fn test_update_exec() {
        let op =
            UpdateOperation::<MockEngine, TestModel>::new(MockEngine::new()).set("name", "Updated");

        let result = op.exec().await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_update_exec_one() {
        let op = UpdateOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .r#where(Filter::Equals("id".into(), FilterValue::Int(1)))
            .set("name", "Updated");

        let result = op.exec_one().await;
        assert!(result.is_err()); // MockEngine returns not_found
    }

    // ========== UpdateManyOperation Tests ==========

    #[test]
    fn test_update_many_new() {
        let op = UpdateManyOperation::<MockEngine, TestModel>::new(MockEngine::new());
        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("UPDATE test_models SET"));
        assert!(!sql.contains("RETURNING")); // UpdateMany doesn't return records
        assert!(params.is_empty());
    }

    #[test]
    fn test_update_many_basic() {
        let op = UpdateManyOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .r#where(Filter::In(
                "id".into(),
                vec![
                    FilterValue::Int(1),
                    FilterValue::Int(2),
                    FilterValue::Int(3),
                ],
            ))
            .set("status", "processed");

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("UPDATE test_models SET"));
        assert!(sql.contains("status = $1"));
        assert!(sql.contains("WHERE"));
        assert!(sql.contains("IN"));
        assert_eq!(params.len(), 4); // 1 set + 3 IN values
    }

    #[test]
    fn test_update_many_with_multiple_conditions() {
        let op = UpdateManyOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .r#where(Filter::Equals(
                "department".into(),
                FilterValue::String("engineering".to_string()),
            ))
            .r#where(Filter::Equals("active".into(), FilterValue::Bool(true)))
            .set("reviewed", FilterValue::Bool(true));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("AND"));
        assert_eq!(params.len(), 3);
    }

    #[test]
    fn test_update_many_without_where() {
        let op = UpdateManyOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .set("reset_password", FilterValue::Bool(true));

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(!sql.contains("WHERE"));
    }

    #[tokio::test]
    async fn test_update_many_exec() {
        let op = UpdateManyOperation::<MockEngine, TestModel>::new(MockEngine::with_count(5))
            .set("status", "updated");

        let result = op.exec().await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 5);
    }

    // ========== SQL Generation Edge Cases ==========

    #[test]
    fn test_update_param_ordering() {
        let op = UpdateOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .set("field1", "value1")
            .set("field2", "value2")
            .r#where(Filter::Equals("id".into(), FilterValue::Int(1)));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        // SET params come first, then WHERE params
        assert!(sql.contains("field1 = $1"));
        assert!(sql.contains("field2 = $2"));
        assert!(sql.contains(r#""id" = $3"#));
        assert_eq!(params.len(), 3);
    }

    #[test]
    fn test_update_many_param_ordering() {
        let op = UpdateManyOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .set("field1", "value1")
            .r#where(Filter::Equals("id".into(), FilterValue::Int(1)));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("field1 = $1"));
        assert!(sql.contains(r#""id" = $2"#));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_update_with_float_value() {
        let op = UpdateOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .set("price", FilterValue::Float(99.99));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("price = $1"));
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_update_with_json_value() {
        let json_value = serde_json::json!({"key": "value"});
        let op = UpdateOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .set("metadata", FilterValue::Json(json_value.clone()));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("metadata = $1"));
        assert_eq!(params[0], FilterValue::Json(json_value));
    }

    // ========== Phase 5a: typed-input wiring ==========

    /// Mock `UpdateInput` used by the `with_update_input` tests. The
    /// codegen-emitted equivalent isn't available inside `prax-query`,
    /// so we hand-roll the trait impl against `TestModel`.
    struct MockUpdateInput(Vec<(String, WriteOp)>);

    impl crate::inputs::UpdateInput for MockUpdateInput {
        type Model = TestModel;
        type Data = crate::inputs::UpdatePayload;
        fn into_ir(self) -> Self::Data {
            self.0
        }
    }

    #[test]
    fn with_update_input_appends_set_ops() {
        let input = MockUpdateInput(vec![
            (
                "name".into(),
                WriteOp::Set(FilterValue::String("Bob".into())),
            ),
            ("age".into(), WriteOp::Increment(FilterValue::Int(1))),
        ]);

        let op = UpdateOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .r#where(Filter::Equals("id".into(), FilterValue::Int(1)))
            .with_update_input(input);

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        // `Set` emits the plain `col = $n` form; `Increment` emits the
        // atomic-operator `col = col + $n` fragment.
        assert!(sql.contains("name = $1"), "got: {sql}");
        assert!(sql.contains("age = age + $2"), "got: {sql}");
        // 2 SET params + 1 WHERE param.
        assert_eq!(params.len(), 3);
        assert_eq!(params[0], FilterValue::String("Bob".into()));
        assert_eq!(params[1], FilterValue::Int(1));
    }

    #[test]
    fn with_update_input_unset_emits_null_no_param() {
        let input = MockUpdateInput(vec![("nickname".into(), WriteOp::Unset)]);

        let op = UpdateOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .with_update_input(input);

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("nickname = NULL"), "got: {sql}");
        // Unset emits no placeholder, so no parameter is pushed.
        assert!(params.is_empty(), "expected no params, got: {params:?}");
    }

    #[test]
    fn update_many_with_update_input_appends() {
        let input = MockUpdateInput(vec![(
            "name".into(),
            WriteOp::Set(FilterValue::String("Bob".into())),
        )]);

        let op = UpdateManyOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .r#where(Filter::Equals("active".into(), FilterValue::Bool(true)))
            .with_update_input(input);

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("UPDATE test_models SET"));
        assert!(sql.contains("name = $1"), "got: {sql}");
        assert!(sql.contains("WHERE"));
        assert_eq!(params.len(), 2);
    }
}
