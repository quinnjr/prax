//! Upsert operation for creating or updating records.

use std::marker::PhantomData;

use crate::error::QueryResult;
use crate::filter::{Filter, FilterValue};
use crate::inputs::WriteOp;
use crate::nested::NestedWriteOp;
use crate::traits::{Model, ModelWithPk, QueryEngine};
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
    /// Update-path entries pushed via [`Self::with_update_input`] or
    /// [`Self::update_set_op`]. When non-empty these take precedence
    /// over `update_columns`/`update_values` because they carry atomic
    /// operators (`Increment`/`Decrement`/`Unset`) the flat
    /// column/value pair can't express.
    update_ops: Vec<(String, WriteOp)>,
    conflict_columns: Vec<String>,
    select: Select,
    /// Nested-write ops to run when the *create* branch fires (the
    /// row didn't previously exist). Empty on the fast path.
    create_nested: Vec<NestedWriteOp>,
    /// Nested-write ops to run when the *update* branch fires (the
    /// row already existed). Empty on the fast path.
    update_nested: Vec<NestedWriteOp>,
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
            update_ops: Vec::new(),
            conflict_columns: Vec::new(),
            select: Select::All,
            create_nested: Vec::new(),
            update_nested: Vec::new(),
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

    /// Apply a typed `WhereUniqueInput`. Overwrites the existing filter.
    pub fn with_where_input<W: crate::inputs::WhereUniqueInput<Model = M>>(mut self, w: W) -> Self {
        self.filter = w.into_ir();
        self
    }

    /// Apply a typed `SelectInput`.
    pub fn with_select_input<S: crate::inputs::SelectInput<Model = M>>(mut self, s: S) -> Self {
        self.select = s.into_ir();
        self
    }

    /// Apply a typed `CreateInput` to the upsert's create path.
    ///
    /// The columns / values produced by the input are appended to the
    /// existing `create_columns` / `create_values` lists. Phase 5a's
    /// codegen ensures every `<Model>CreateInput` carries the
    /// model's `@unique` conflict column, so [`Self::on_conflict`]
    /// remains useful with this method.
    pub fn with_create_input<I>(mut self, input: I) -> Self
    where
        I: crate::inputs::CreateInput<Model = M, Data = crate::inputs::CreatePayload>,
    {
        let data: crate::inputs::CreatePayload = input.into_ir();
        for (col, val) in data {
            self.create_columns.push(col);
            self.create_values.push(val);
        }
        self
    }

    /// Apply a typed `UpdateInput` to the upsert's update path.
    ///
    /// Atomic operators are preserved — when the update branch fires,
    /// `Increment(n)` emits `col = col + $n` in the `DO UPDATE SET`
    /// clause, etc. Setting any input via this method overrides any
    /// flat update columns recorded by [`Self::update`] or
    /// [`Self::update_set`].
    pub fn with_update_input<I>(mut self, input: I) -> Self
    where
        I: crate::inputs::UpdateInput<Model = M, Data = crate::inputs::UpdatePayload>,
    {
        let data: crate::inputs::UpdatePayload = input.into_ir();
        for (col, op) in data {
            self.update_ops.push((col, op));
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
        // Build update SET clause. When `update_ops` is non-empty it
        // supplants the legacy flat column/value list — typed inputs
        // can carry atomic operators (`col = col + $n`) the flat
        // pair can't represent.
        let update_set = if !self.update_ops.is_empty() {
            let update_parts: Vec<String> = self
                .update_ops
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
            update_parts.join(", ")
        } else if !self.update_columns.is_empty() {
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

    /// Queue a nested write to fire when the *create* branch runs
    /// (i.e. no existing row matched).
    pub fn with_create_nested(mut self, nw: NestedWriteOp) -> Self
    where
        E: crate::capabilities::SupportsNestedWrites,
    {
        self.create_nested.push(nw);
        self
    }

    /// Queue a nested write to fire when the *update* branch runs
    /// (i.e. an existing row was found and updated).
    pub fn with_update_nested(mut self, nw: NestedWriteOp) -> Self
    where
        E: crate::capabilities::SupportsNestedWrites,
    {
        self.update_nested.push(nw);
        self
    }

    /// Execute the upsert and return the record.
    ///
    /// Fast path (no nested writes queued): runs a single
    /// vendor-specific upsert (`INSERT ... ON CONFLICT DO UPDATE` on
    /// Postgres, the dialect's equivalent elsewhere).
    ///
    /// Slow path (nested writes queued via `with_create_nested` /
    /// `with_update_nested`): runs a two-statement
    /// engine-agnostic upsert inside a transaction so we can tell which
    /// branch fired:
    /// 1. `UPDATE` the row by primary key. If `affected > 0`, the
    ///    update branch ran — fire `update_nested` with the PK we
    ///    already have from `where:`.
    /// 2. Otherwise `INSERT` the row, take the PK from the inserted
    ///    model, and fire `create_nested`.
    pub async fn exec(self) -> QueryResult<M>
    where
        M: Send + 'static + ModelWithPk,
    {
        // Fast path: single-statement vendor-specific upsert.
        if self.create_nested.is_empty() && self.update_nested.is_empty() {
            let dialect = self.engine.dialect();
            let (sql, params) = self.build_sql(dialect);
            return self.engine.execute_insert::<M>(&sql, params).await;
        }

        // Nested writes are queued — the existing where-unique must
        // equal-match the primary key column. This is the same
        // restriction as `update!`'s nested-write path.
        let parent_pk =
            crate::operations::update::extract_pk_from_filter(&self.filter, M::PRIMARY_KEY[0])
                .ok_or_else(|| {
                    crate::error::QueryError::invalid_input(
                        "where",
                        "nested writes inside `upsert!` require the `where:` clause to equal-match \
                 the primary-key column",
                    )
                    .with_help(format!(
                        "expected `where: {{ {pk}: <value> }}` on `{table}` — non-PK unique \
                 columns are not yet supported for nested writes inside upsert!. \
                 Lift this restriction by running the nested ops in a separate operation \
                 after looking up the row's PK.",
                        pk = M::PRIMARY_KEY[0],
                        table = M::TABLE_NAME,
                    ))
                })?;

        let UpsertOperation {
            engine,
            filter,
            create_columns,
            create_values,
            update_columns,
            update_values,
            update_ops,
            conflict_columns: _,
            select,
            create_nested,
            update_nested,
            _model,
        } = self;

        engine
            .transaction(move |tx| async move {
                let dialect = tx.dialect();

                // Phase 1: try UPDATE first.
                let (update_sql, update_params) = build_update_sql::<M>(
                    &filter,
                    &update_columns,
                    &update_values,
                    &update_ops,
                    dialect,
                );
                let affected = tx.execute_raw(&update_sql, update_params).await?;

                let (row, fired_nested): (M, Vec<NestedWriteOp>) = if affected > 0 {
                    // Update branch — fetch the row back via SELECT so
                    // the caller sees the freshly-updated columns. We
                    // know the PK from the where filter.
                    let (sel_sql, sel_params) =
                        build_select_by_pk_sql::<M>(parent_pk.clone(), &select, dialect);
                    let fetched: M = tx.query_one::<M>(&sel_sql, sel_params).await?;
                    (fetched, update_nested)
                } else {
                    // Create branch — INSERT and capture the returned row.
                    let (ins_sql, ins_params) =
                        build_insert_sql::<M>(&create_columns, &create_values, &select, dialect);
                    let inserted: M = tx.execute_insert::<M>(&ins_sql, ins_params).await?;
                    (inserted, create_nested)
                };

                // Dispatch the chosen nested-op vec, sharing the same
                // partition-by-target-Connect batching as create.rs.
                let parent_pk_for_nested = if affected > 0 {
                    parent_pk
                } else {
                    row.pk_value()
                };
                run_nested_ops(&tx, dialect, fired_nested, &parent_pk_for_nested).await?;

                Ok(row)
            })
            .await
    }
}

/// Build a two-statement-style UPDATE for the upsert's "update branch".
///
/// Uses `update_ops` when populated (carries atomic operators), else
/// falls back to the legacy flat `update_columns`/`update_values` pair.
/// Always emits a `WHERE` clause from `filter` (the where-unique).
fn build_update_sql<M: Model>(
    filter: &Filter,
    update_columns: &[String],
    update_values: &[FilterValue],
    update_ops: &[(String, WriteOp)],
    dialect: &dyn crate::dialect::SqlDialect,
) -> (String, Vec<FilterValue>) {
    let mut sql = String::new();
    let mut params = Vec::new();
    let mut param_idx = 1;

    sql.push_str("UPDATE ");
    sql.push_str(M::TABLE_NAME);
    sql.push_str(" SET ");

    let set_parts: Vec<String> = if !update_ops.is_empty() {
        update_ops
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
            .collect()
    } else {
        update_columns
            .iter()
            .zip(update_values.iter())
            .map(|(col, val)| {
                params.push(val.clone());
                let part = format!("{} = {}", col, dialect.placeholder(param_idx));
                param_idx += 1;
                part
            })
            .collect()
    };
    sql.push_str(&set_parts.join(", "));

    if !filter.is_none() {
        let (where_sql, where_params) = filter.to_sql(param_idx - 1, dialect);
        sql.push_str(" WHERE ");
        sql.push_str(&where_sql);
        params.extend(where_params);
    }

    (sql, params)
}

/// Build the create-branch INSERT used by the two-statement upsert.
fn build_insert_sql<M: Model>(
    columns: &[String],
    values: &[FilterValue],
    select: &Select,
    dialect: &dyn crate::dialect::SqlDialect,
) -> (String, Vec<FilterValue>) {
    let mut sql = String::new();
    sql.push_str("INSERT INTO ");
    sql.push_str(M::TABLE_NAME);
    sql.push_str(" (");
    sql.push_str(&columns.join(", "));
    sql.push(')');
    sql.push_str(" VALUES (");
    let placeholders: Vec<_> = (1..=values.len()).map(|i| dialect.placeholder(i)).collect();
    sql.push_str(&placeholders.join(", "));
    sql.push(')');
    sql.push_str(&dialect.returning_clause(&select.to_sql()));
    (sql, values.to_vec())
}

/// Build the SELECT-by-pk used to re-fetch the row after the update
/// branch ran.
fn build_select_by_pk_sql<M: Model>(
    pk: FilterValue,
    select: &Select,
    dialect: &dyn crate::dialect::SqlDialect,
) -> (String, Vec<FilterValue>) {
    let cols = select.to_sql();
    let sql = format!(
        "SELECT {} FROM {} WHERE {} = {}",
        if cols.is_empty() || cols == "*" {
            "*".to_string()
        } else {
            cols
        },
        M::TABLE_NAME,
        dialect.quote_ident(M::PRIMARY_KEY[0]),
        dialect.placeholder(1),
    );
    (sql, vec![pk])
}

/// Iterate `nested` against `tx`, batching consecutive Connect ops with
/// the same target — mirrors the create.rs partition loop.
async fn run_nested_ops<E: QueryEngine>(
    tx: &E,
    dialect: &dyn crate::dialect::SqlDialect,
    nested: Vec<NestedWriteOp>,
    parent_pk: &FilterValue,
) -> QueryResult<()> {
    let mut idx = 0;
    while idx < nested.len() {
        if let NestedWriteOp::Connect {
            target_table: run_table,
            foreign_key: run_fk,
            target_pk: run_target_pk,
            ..
        } = &nested[idx]
        {
            let run_table = *run_table;
            let run_fk = *run_fk;
            let run_target_pk = *run_target_pk;
            let mut end = idx + 1;
            while end < nested.len() {
                match &nested[end] {
                    NestedWriteOp::Connect {
                        target_table,
                        foreign_key,
                        target_pk,
                        ..
                    } if *target_table == run_table
                        && *foreign_key == run_fk
                        && *target_pk == run_target_pk =>
                    {
                        end += 1;
                    }
                    _ => break,
                }
            }

            if end - idx == 1 {
                let op = nested[idx].clone();
                op.execute(tx, parent_pk).await?;
            } else {
                let expected = (end - idx) as u64;
                let mut pks: Vec<FilterValue> = Vec::with_capacity(end - idx + 1);
                pks.push(parent_pk.clone());
                for op in &nested[idx..end] {
                    if let NestedWriteOp::Connect { pk, .. } = op {
                        pks.push(pk.clone());
                    }
                }
                let placeholders: Vec<String> =
                    (2..=pks.len()).map(|i| dialect.placeholder(i)).collect();
                let sql = format!(
                    "UPDATE {} SET {} = {} WHERE {} IN ({})",
                    dialect.quote_ident(run_table),
                    dialect.quote_ident(run_fk),
                    dialect.placeholder(1),
                    dialect.quote_ident(run_target_pk),
                    placeholders.join(", "),
                );
                let affected = tx.execute_raw(&sql, pks).await?;
                if affected != expected {
                    return Err(crate::error::QueryError::not_found(run_table)
                        .with_context("Nested Connect batch")
                        .with_help(format!(
                            "Expected {} matching rows but UPDATE affected {}",
                            expected, affected
                        )));
                }
            }
            idx = end;
        } else {
            let op = nested[idx].clone();
            op.execute(tx, parent_pk).await?;
            idx += 1;
        }
    }
    Ok(())
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

    // Phase-5c slow-path nested-write wiring requires `ModelWithPk` on
    // the return type. The constant PK is fine because the legacy
    // single-statement tests never exercise the slow path.
    impl crate::traits::ModelWithPk for TestModel {
        fn pk_value(&self) -> FilterValue {
            FilterValue::Int(0)
        }
        fn get_column_value(&self, _column: &str) -> Option<FilterValue> {
            None
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
        assert!(sql.contains("ON CONFLICT (\"email\")"));
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
        assert!(sql.contains("ON CONFLICT (\"email\")"));
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

    // ========== Phase 5a: typed-input wiring ==========

    struct MockCreateInput(Vec<(String, FilterValue)>);

    impl crate::inputs::CreateInput for MockCreateInput {
        type Model = TestModel;
        type Data = crate::inputs::CreatePayload;
        fn into_ir(self) -> Self::Data {
            self.0
        }
    }

    struct MockUpdateInput(Vec<(String, WriteOp)>);

    impl crate::inputs::UpdateInput for MockUpdateInput {
        type Model = TestModel;
        type Data = crate::inputs::UpdatePayload;
        fn into_ir(self) -> Self::Data {
            self.0
        }
    }

    #[test]
    fn upsert_with_create_input_appends_create_columns() {
        let input = MockCreateInput(vec![
            ("email".into(), FilterValue::String("a@x.com".into())),
            ("name".into(), FilterValue::String("Alice".into())),
        ]);
        let op = UpsertOperation::<MockEngine, TestModel>::new(MockEngine)
            .on_conflict(["email"])
            .with_create_input(input)
            .update_set("name", "Updated");

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);
        assert!(sql.contains("(email, name)"), "got: {sql}");
        assert!(sql.contains("VALUES ($1, $2)"), "got: {sql}");
        assert!(sql.contains("ON CONFLICT (\"email\")"));
        // 2 create params + 1 update param.
        assert_eq!(params.len(), 3);
    }

    #[test]
    fn upsert_with_update_input_uses_atomic_ops() {
        let create = MockCreateInput(vec![("id".into(), FilterValue::Int(1))]);
        let update = MockUpdateInput(vec![
            (
                "name".into(),
                WriteOp::Set(FilterValue::String("Renamed".into())),
            ),
            (
                "login_count".into(),
                WriteOp::Increment(FilterValue::Int(1)),
            ),
        ]);
        let op = UpsertOperation::<MockEngine, TestModel>::new(MockEngine)
            .on_conflict(["id"])
            .with_create_input(create)
            .with_update_input(update);

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);
        // Atomic operator must round-trip through the upsert path.
        assert!(sql.contains("login_count = login_count + $"), "got: {sql}");
        // 1 create + 2 update params.
        assert_eq!(params.len(), 3);
    }

    // ========== Phase 5c: nested-write wiring on upsert! ==========

    use std::sync::{Arc, Mutex};

    type StatementLog = Arc<Mutex<Vec<(String, Vec<FilterValue>)>>>;

    /// Recording engine that exposes a settable `affected` sequence and
    /// returns a default `TestModel` from `execute_insert` / `query_one`
    /// (the two paths the nested-upsert exec consumes).
    #[derive(Clone)]
    struct RecordingEngine {
        recorded: StatementLog,
        affected: Arc<Mutex<Vec<u64>>>,
    }

    impl RecordingEngine {
        fn with_affected(seq: Vec<u64>) -> Self {
            let mut rev = seq;
            rev.reverse();
            Self {
                recorded: Arc::new(Mutex::new(Vec::new())),
                affected: Arc::new(Mutex::new(rev)),
            }
        }

        fn statements(&self) -> Vec<(String, Vec<FilterValue>)> {
            self.recorded.lock().unwrap().clone()
        }
    }

    impl crate::capabilities::SupportsNestedWrites for RecordingEngine {}

    impl QueryEngine for RecordingEngine {
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
            sql: &str,
            params: Vec<FilterValue>,
        ) -> crate::traits::BoxFuture<'_, QueryResult<T>> {
            let recorded = self.recorded.clone();
            let sql = sql.to_string();
            Box::pin(async move {
                recorded.lock().unwrap().push((sql, params));
                T::from_row(&CannedRow).map_err(|e| QueryError::internal(e.to_string()))
            })
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
            sql: &str,
            params: Vec<FilterValue>,
        ) -> crate::traits::BoxFuture<'_, QueryResult<T>> {
            let recorded = self.recorded.clone();
            let sql = sql.to_string();
            Box::pin(async move {
                recorded.lock().unwrap().push((sql, params));
                T::from_row(&CannedRow).map_err(|e| QueryError::internal(e.to_string()))
            })
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
            sql: &str,
            params: Vec<FilterValue>,
        ) -> crate::traits::BoxFuture<'_, QueryResult<u64>> {
            let recorded = self.recorded.clone();
            let affected = self.affected.clone();
            let sql_string = sql.to_string();
            let default = if sql.contains(" IN (") {
                (params.len() as u64).saturating_sub(1)
            } else {
                1
            };
            Box::pin(async move {
                recorded.lock().unwrap().push((sql_string, params));
                Ok(affected.lock().unwrap().pop().unwrap_or(default))
            })
        }

        fn count(
            &self,
            _sql: &str,
            _params: Vec<FilterValue>,
        ) -> crate::traits::BoxFuture<'_, QueryResult<u64>> {
            Box::pin(async { Ok(0) })
        }
    }

    /// Stand-in RowRef so `execute_insert` / `query_one` can synthesise
    /// a `TestModel` value without a live database.
    struct CannedRow;

    impl crate::row::RowRef for CannedRow {
        fn get_i32(&self, _column: &str) -> Result<i32, crate::row::RowError> {
            Ok(0)
        }
        fn get_i32_opt(&self, _column: &str) -> Result<Option<i32>, crate::row::RowError> {
            Ok(Some(0))
        }
        fn get_i64(&self, _column: &str) -> Result<i64, crate::row::RowError> {
            Ok(0)
        }
        fn get_i64_opt(&self, _column: &str) -> Result<Option<i64>, crate::row::RowError> {
            Ok(None)
        }
        fn get_f64(&self, _column: &str) -> Result<f64, crate::row::RowError> {
            Ok(0.0)
        }
        fn get_f64_opt(&self, _column: &str) -> Result<Option<f64>, crate::row::RowError> {
            Ok(None)
        }
        fn get_bool(&self, _column: &str) -> Result<bool, crate::row::RowError> {
            Ok(false)
        }
        fn get_bool_opt(&self, _column: &str) -> Result<Option<bool>, crate::row::RowError> {
            Ok(None)
        }
        fn get_str(&self, _column: &str) -> Result<&str, crate::row::RowError> {
            Ok("canned")
        }
        fn get_str_opt(&self, _column: &str) -> Result<Option<&str>, crate::row::RowError> {
            Ok(Some("canned"))
        }
        fn get_bytes(&self, _column: &str) -> Result<&[u8], crate::row::RowError> {
            Ok(b"")
        }
        fn get_bytes_opt(&self, _column: &str) -> Result<Option<&[u8]>, crate::row::RowError> {
            Ok(None)
        }
    }

    #[tokio::test]
    async fn upsert_with_nested_in_update_branch_fires_update_nested_only() {
        // affected=1 on the UPDATE → update branch.
        let engine = RecordingEngine::with_affected(vec![1]);
        let op = UpsertOperation::<RecordingEngine, TestModel>::new(engine.clone())
            .r#where(Filter::Equals("id".into(), FilterValue::Int(7)))
            .create_set("id", FilterValue::Int(7))
            .create_set("email", "new@x.com")
            .update_set("name", "Renamed")
            .with_update_nested(NestedWriteOp::Disconnect {
                relation: "posts",
                target_table: "posts",
                foreign_key: "author_id",
                target_pk: "id",
                pk: FilterValue::Int(42),
            })
            .with_create_nested(NestedWriteOp::Create {
                relation: "posts",
                target_table: "posts",
                foreign_key: "author_id",
                payload: vec![vec![("title".into(), FilterValue::String("p1".into()))]],
            });

        let _ = op.exec().await.expect("upsert update branch");

        let stmts = engine.statements();
        // Expect: UPDATE (affected=1) + SELECT (re-fetch) + nested Disconnect UPDATE
        assert_eq!(
            stmts.len(),
            3,
            "UPDATE + SELECT + nested disconnect; got {stmts:#?}"
        );
        assert!(
            stmts[0].0.starts_with("UPDATE test_models"),
            "first stmt should be parent UPDATE: {}",
            stmts[0].0
        );
        assert!(
            stmts[1].0.starts_with("SELECT"),
            "second stmt should re-fetch the row: {}",
            stmts[1].0
        );
        // Third stmt is the nested Disconnect — UPDATE child + NULL.
        assert!(stmts[2].0.contains("UPDATE"), "got: {}", stmts[2].0);
        assert!(stmts[2].0.contains("posts"), "got: {}", stmts[2].0);
        assert!(stmts[2].0.contains("NULL"), "got: {}", stmts[2].0);
        // Verify no INSERT (no create branch) and no nested Create (FK splicing).
        assert!(
            !stmts.iter().any(|(s, _)| s.starts_with("INSERT INTO")),
            "no INSERT must fire on update branch: {stmts:#?}"
        );
    }

    #[tokio::test]
    async fn upsert_with_nested_in_create_branch_fires_create_nested_only() {
        // affected=0 on UPDATE → create branch (INSERT runs).
        let engine = RecordingEngine::with_affected(vec![0]);
        let op = UpsertOperation::<RecordingEngine, TestModel>::new(engine.clone())
            .r#where(Filter::Equals("id".into(), FilterValue::Int(7)))
            .create_set("id", FilterValue::Int(7))
            .create_set("email", "new@x.com")
            .update_set("name", "Renamed")
            .with_update_nested(NestedWriteOp::Disconnect {
                relation: "posts",
                target_table: "posts",
                foreign_key: "author_id",
                target_pk: "id",
                pk: FilterValue::Int(42),
            })
            .with_create_nested(NestedWriteOp::Create {
                relation: "posts",
                target_table: "posts",
                foreign_key: "author_id",
                payload: vec![vec![("title".into(), FilterValue::String("p1".into()))]],
            });

        let _ = op.exec().await.expect("upsert create branch");

        let stmts = engine.statements();
        // Expect: UPDATE (affected=0) + INSERT + nested Create child INSERT
        assert_eq!(
            stmts.len(),
            3,
            "UPDATE + INSERT + nested child INSERT; got {stmts:#?}"
        );
        assert!(
            stmts[0].0.starts_with("UPDATE test_models"),
            "first stmt should be parent UPDATE: {}",
            stmts[0].0
        );
        assert!(
            stmts[1].0.contains("INSERT INTO test_models"),
            "second stmt should be the create-branch INSERT: {}",
            stmts[1].0
        );
        assert!(
            stmts[2].0.contains("INSERT INTO"),
            "third stmt should be the nested Create child INSERT: {}",
            stmts[2].0
        );
        assert!(
            stmts[2].0.contains("posts"),
            "nested INSERT targets posts: {}",
            stmts[2].0
        );
        // No SELECT (we got the row directly from the INSERT) and no
        // nested Disconnect UPDATE on posts table.
        assert!(
            !stmts.iter().any(|(s, _)| s.starts_with("SELECT")),
            "no SELECT must fire on create branch: {stmts:#?}"
        );
        assert!(
            !stmts
                .iter()
                .any(|(s, _)| s.contains("UPDATE \"posts\"") || s.contains("UPDATE posts")),
            "no nested Disconnect on update_nested must fire: {stmts:#?}"
        );
    }

    #[tokio::test]
    async fn upsert_both_branches_carry_nested_only_one_fires() {
        // Run twice with two engines — once for each branch — and
        // confirm only the branch-appropriate nested ops execute.
        // Branch 1: update.
        let engine_u = RecordingEngine::with_affected(vec![1]);
        let _ = UpsertOperation::<RecordingEngine, TestModel>::new(engine_u.clone())
            .r#where(Filter::Equals("id".into(), FilterValue::Int(7)))
            .create_set("id", FilterValue::Int(7))
            .update_set("name", "Renamed")
            .with_update_nested(NestedWriteOp::Disconnect {
                relation: "posts",
                target_table: "posts",
                foreign_key: "author_id",
                target_pk: "id",
                pk: FilterValue::Int(42),
            })
            .with_create_nested(NestedWriteOp::Create {
                relation: "posts",
                target_table: "posts",
                foreign_key: "author_id",
                payload: vec![vec![("title".into(), FilterValue::String("p".into()))]],
            })
            .exec()
            .await
            .expect("update branch");
        let u_stmts = engine_u.statements();
        // Disconnect must fire, child INSERT (nested Create) must not.
        assert!(
            u_stmts.iter().any(|(s, _)| s.contains("NULL")),
            "expected nested Disconnect: {u_stmts:#?}"
        );
        assert!(
            !u_stmts
                .iter()
                .any(|(s, _)| s.contains("INSERT INTO") && s.contains("posts")),
            "no nested Create child INSERT on update branch: {u_stmts:#?}"
        );

        // Branch 2: create.
        let engine_c = RecordingEngine::with_affected(vec![0]);
        let _ = UpsertOperation::<RecordingEngine, TestModel>::new(engine_c.clone())
            .r#where(Filter::Equals("id".into(), FilterValue::Int(7)))
            .create_set("id", FilterValue::Int(7))
            .update_set("name", "Renamed")
            .with_update_nested(NestedWriteOp::Disconnect {
                relation: "posts",
                target_table: "posts",
                foreign_key: "author_id",
                target_pk: "id",
                pk: FilterValue::Int(42),
            })
            .with_create_nested(NestedWriteOp::Create {
                relation: "posts",
                target_table: "posts",
                foreign_key: "author_id",
                payload: vec![vec![("title".into(), FilterValue::String("p".into()))]],
            })
            .exec()
            .await
            .expect("create branch");
        let c_stmts = engine_c.statements();
        // Child INSERT (nested Create on posts) must fire, Disconnect must not.
        assert!(
            c_stmts
                .iter()
                .any(|(s, _)| s.contains("INSERT INTO") && s.contains("posts")),
            "expected nested Create child INSERT: {c_stmts:#?}"
        );
        assert!(
            !c_stmts.iter().any(|(s, _)| s.contains("NULL")),
            "no nested Disconnect on create branch: {c_stmts:#?}"
        );
    }
}
