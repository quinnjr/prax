//! Create operation for inserting new records.

use std::marker::PhantomData;

use crate::error::QueryResult;
use crate::filter::FilterValue;
use crate::nested::NestedWriteOp;
use crate::traits::{Model, ModelWithPk, QueryEngine};
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
    /// Queued nested-write ops run after the parent INSERT inside an
    /// implicit transaction. Populated by [`CreateOperation::with`].
    /// Empty on the fast path (single INSERT, no transaction wrap).
    nested: Vec<NestedWriteOp>,
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
            nested: Vec::new(),
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

    /// Apply a typed `SelectInput`.
    pub fn with_select_input<S: crate::inputs::SelectInput<Model = M>>(mut self, s: S) -> Self {
        self.select = s.into_ir();
        self
    }

    /// Apply a typed `CreateInput`.
    ///
    /// The input's `into_ir` produces a flat `Vec<(column, value)>`
    /// (per `prax_query::inputs::CreatePayload`), which is appended to
    /// the operation's columns + values just like the existing
    /// `set_many`. Phase 5a does not surface nested writes through
    /// this path — relation operators inside `data:` are rejected by
    /// codegen with a "phase 5b" diagnostic before reaching the
    /// runtime.
    pub fn with_create_input<I>(mut self, input: I) -> Self
    where
        I: crate::inputs::CreateInput<Model = M, Data = crate::inputs::CreatePayload>,
    {
        let data: crate::inputs::CreatePayload = input.into_ir();
        for (col, val) in data {
            self.columns.push(col);
            self.values.push(val);
        }
        self
    }

    /// Queue a nested write to run alongside this create.
    ///
    /// The parent `INSERT` and every queued nested op execute inside a
    /// single implicit transaction — any failure rolls back the parent
    /// INSERT too. Typical use is via the codegen-emitted per-relation
    /// helpers:
    ///
    /// ```rust,ignore
    /// c.user().create()
    ///     .set("email", "u@x.com")
    ///     .with(user::posts::create(vec![
    ///         vec![("title".into(), "p1".into())],
    ///     ]))
    ///     .exec().await?;
    /// ```
    pub fn with(mut self, nw: NestedWriteOp) -> Self {
        self.nested.push(nw);
        self
    }

    /// Build the SQL query.
    pub fn build_sql(
        &self,
        dialect: &dyn crate::dialect::SqlDialect,
    ) -> (String, Vec<FilterValue>) {
        Self::build_insert_sql(&self.columns, &self.values, &self.select, dialect)
    }

    /// Free-function form of [`Self::build_sql`] — takes the pieces by
    /// reference so the `exec` path can reuse it after destructuring
    /// `self` to move the captured state into the transaction closure.
    fn build_insert_sql(
        columns: &[String],
        values: &[FilterValue],
        select: &Select,
        dialect: &dyn crate::dialect::SqlDialect,
    ) -> (String, Vec<FilterValue>) {
        let mut sql = String::new();

        // INSERT INTO clause
        sql.push_str("INSERT INTO ");
        sql.push_str(M::TABLE_NAME);

        // Columns
        sql.push_str(" (");
        sql.push_str(&columns.join(", "));
        sql.push(')');

        // VALUES
        sql.push_str(" VALUES (");
        let placeholders: Vec<_> = (1..=values.len()).map(|i| dialect.placeholder(i)).collect();
        sql.push_str(&placeholders.join(", "));
        sql.push(')');

        // RETURNING clause
        sql.push_str(&dialect.returning_clause(&select.to_sql()));

        (sql, values.to_vec())
    }

    /// Execute the create operation and return the created record.
    ///
    /// When no nested writes have been queued via [`Self::with`], this
    /// runs a single `INSERT ... RETURNING` (or equivalent) against the
    /// engine. When nested writes are queued, the whole operation is
    /// wrapped in a transaction — the parent `INSERT` runs first, then
    /// each nested op in order; if any nested op fails the parent
    /// insert is rolled back too.
    ///
    /// The `ModelWithPk` bound on the transactional branch is what
    /// gives the nested-write executor the parent's primary-key value
    /// to splice into child rows' foreign-key columns.
    pub async fn exec(self) -> QueryResult<M>
    where
        M: Send + 'static + ModelWithPk,
    {
        let CreateOperation {
            engine,
            columns,
            values,
            select,
            nested,
            _model,
        } = self;

        // Fast path: no nested writes, run the INSERT directly.
        if nested.is_empty() {
            let dialect = engine.dialect();
            let (sql, params) = Self::build_insert_sql(&columns, &values, &select, dialect);
            return engine.execute_insert::<M>(&sql, params).await;
        }

        // Slow path: wrap the INSERT + nested writes in a transaction.
        // `engine.transaction` clones the engine into the closure and
        // routes every query emitted inside through the same `BEGIN`
        // block. A non-Ok return from the closure triggers ROLLBACK.
        engine
            .transaction(move |tx| async move {
                let dialect = tx.dialect();
                let (sql, params) = Self::build_insert_sql(&columns, &values, &select, dialect);
                let parent: M = tx.execute_insert::<M>(&sql, params).await?;
                let parent_pk = parent.pk_value();
                for nw in nested {
                    nw.execute(&tx, &parent_pk).await?;
                }
                Ok(parent)
            })
            .await
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

    /// Toggle `skip_duplicates` via a runtime flag.
    ///
    /// The bare [`Self::skip_duplicates`] is a builder-style "enable
    /// it" call. The macros emit `with_skip_duplicates(<bool-expr>)`
    /// so the DSL's `skip_duplicates: false` shortcut produces a
    /// statement-level no-op without conditional macro emission.
    pub fn with_skip_duplicates(mut self, flag: bool) -> Self {
        self.skip_duplicates = flag;
        self
    }

    /// Apply a batch of typed `CreateInput`s.
    ///
    /// Each input lowers to its own `CreatePayload`
    /// (`Vec<(column, value)>`). The full set of columns across every
    /// input becomes the operation's column list (first occurrence
    /// wins for ordering); rows missing a column get `FilterValue::Null`
    /// in that slot. This matches Prisma's `createMany` semantics,
    /// where omitted optional fields are inserted as NULL.
    pub fn with_create_inputs<I, T>(mut self, inputs: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: crate::inputs::CreateInput<Model = M, Data = crate::inputs::CreatePayload>,
    {
        // Lower every input first so we can compute the union column
        // set before deciding the row layout.
        let lowered: Vec<crate::inputs::CreatePayload> =
            inputs.into_iter().map(|i| i.into_ir()).collect();

        if lowered.is_empty() {
            return self;
        }

        // Seed columns from existing state (preserves any prior
        // `.columns(...)` call) and append new columns in first-seen
        // order.
        let mut columns: Vec<String> = self.columns.clone();
        for row in &lowered {
            for (col, _) in row {
                if !columns.iter().any(|c| c == col) {
                    columns.push(col.clone());
                }
            }
        }

        // Build each row in the canonical column order, padding missing
        // entries with NULL.
        let mut rows: Vec<Vec<FilterValue>> = Vec::with_capacity(lowered.len());
        for row in lowered {
            let mut out: Vec<FilterValue> = Vec::with_capacity(columns.len());
            for col in &columns {
                let v = row
                    .iter()
                    .find(|(c, _)| c == col)
                    .map(|(_, v)| v.clone())
                    .unwrap_or(FilterValue::Null);
                out.push(v);
            }
            rows.push(out);
        }

        self.columns = columns;
        self.rows.extend(rows);
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

    // Gate the transactional `CreateOperation::exec` path in tests:
    // the new nested-write wiring requires `ModelWithPk` on the return
    // type. A fixed constant PK is fine because these tests never
    // exercise the nested path — they only need exec() to compile.
    impl crate::traits::ModelWithPk for TestModel {
        fn pk_value(&self) -> FilterValue {
            FilterValue::Int(0)
        }
        fn get_column_value(&self, _column: &str) -> Option<FilterValue> {
            None
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

    // ========== Phase 5a: typed-input wiring ==========

    /// Mock `CreateInput` used by the `with_create_input(s)` tests.
    struct MockCreateInput(Vec<(String, FilterValue)>);

    impl crate::inputs::CreateInput for MockCreateInput {
        type Model = TestModel;
        type Data = crate::inputs::CreatePayload;
        fn into_ir(self) -> Self::Data {
            self.0
        }
    }

    #[test]
    fn with_create_input_appends_columns_and_values() {
        let input = MockCreateInput(vec![
            ("name".into(), FilterValue::String("Alice".into())),
            ("email".into(), FilterValue::String("a@x.com".into())),
        ]);
        let op = CreateOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .with_create_input(input);

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);
        // The chain should produce identical SQL to the existing
        // `.set(...).set(...)` chain — that's the contract.
        assert!(sql.contains("(name, email)"), "got: {sql}");
        assert!(sql.contains("VALUES ($1, $2)"), "got: {sql}");
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn with_create_inputs_pads_missing_columns_with_null() {
        let row1 = MockCreateInput(vec![
            ("name".into(), FilterValue::String("Alice".into())),
            ("email".into(), FilterValue::String("a@x.com".into())),
        ]);
        // Second input omits `email` — codegen does this for inputs
        // where the optional `email` field was left as `None`.
        let row2 = MockCreateInput(vec![("name".into(), FilterValue::String("Bob".into()))]);

        let op = CreateManyOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .with_create_inputs(vec![row1, row2]);

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);
        assert!(sql.contains("(name, email)"), "got: {sql}");
        assert!(sql.contains("VALUES ($1, $2), ($3, $4)"), "got: {sql}");
        assert_eq!(params.len(), 4);
        assert_eq!(params[3], FilterValue::Null);
    }
}
