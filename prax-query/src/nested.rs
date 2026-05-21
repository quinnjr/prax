#![allow(dead_code)]

//! Nested write operations for managing relations in a single mutation.
//!
//! This module provides support for creating, connecting, disconnecting, and updating
//! related records within a single create or update operation.
//!
//! # Example
//!
//! ```rust,ignore
//! use prax_query::nested::*;
//!
//! // Create a user with nested posts
//! let user = client
//!     .user()
//!     .create(user::create::Data {
//!         email: "user@example.com".into(),
//!         name: Some("John Doe".into()),
//!         posts: Some(NestedWrite::create_many(vec![
//!             post::create::Data { title: "First Post".into(), content: None },
//!             post::create::Data { title: "Second Post".into(), content: None },
//!         ])),
//!     })
//!     .exec()
//!     .await?;
//!
//! // Connect existing posts to a user
//! let user = client
//!     .user()
//!     .update(user::id::equals(1))
//!     .data(user::update::Data {
//!         posts: Some(NestedWrite::connect(vec![
//!             post::id::equals(10),
//!             post::id::equals(20),
//!         ])),
//!         ..Default::default()
//!     })
//!     .exec()
//!     .await?;
//!
//! // Disconnect posts from a user
//! let user = client
//!     .user()
//!     .update(user::id::equals(1))
//!     .data(user::update::Data {
//!         posts: Some(NestedWrite::disconnect(vec![
//!             post::id::equals(10),
//!         ])),
//!         ..Default::default()
//!     })
//!     .exec()
//!     .await?;
//! ```

use std::fmt::Debug;
use std::marker::PhantomData;

use crate::error::QueryResult;
use crate::filter::{Filter, FilterValue};
use crate::sql::quote_identifier;
use crate::traits::{Model, QueryEngine};

/// Represents a nested write operation for relations.
#[derive(Debug, Clone)]
pub enum NestedWrite<T: Model> {
    /// Create new related records.
    Create(Vec<NestedCreateData<T>>),
    /// Create new records or connect existing ones.
    CreateOrConnect(Vec<NestedCreateOrConnectData<T>>),
    /// Connect existing records by their unique identifier.
    Connect(Vec<Filter>),
    /// Disconnect records from the relation.
    Disconnect(Vec<Filter>),
    /// Set the relation to exactly these records (disconnect all others).
    Set(Vec<Filter>),
    /// Delete related records.
    Delete(Vec<Filter>),
    /// Update related records.
    Update(Vec<NestedUpdateData<T>>),
    /// Update or create related records.
    Upsert(Vec<NestedUpsertData<T>>),
    /// Update many related records matching a filter.
    UpdateMany(NestedUpdateManyData<T>),
    /// Delete many related records matching a filter.
    DeleteMany(Filter),
}

impl<T: Model> NestedWrite<T> {
    /// Create a new related record.
    pub fn create(data: NestedCreateData<T>) -> Self {
        Self::Create(vec![data])
    }

    /// Create multiple new related records.
    pub fn create_many(data: Vec<NestedCreateData<T>>) -> Self {
        Self::Create(data)
    }

    /// Connect an existing record by filter.
    pub fn connect_one(filter: impl Into<Filter>) -> Self {
        Self::Connect(vec![filter.into()])
    }

    /// Connect multiple existing records by filters.
    pub fn connect(filters: Vec<impl Into<Filter>>) -> Self {
        Self::Connect(filters.into_iter().map(Into::into).collect())
    }

    /// Disconnect a record by filter.
    pub fn disconnect_one(filter: impl Into<Filter>) -> Self {
        Self::Disconnect(vec![filter.into()])
    }

    /// Disconnect multiple records by filters.
    pub fn disconnect(filters: Vec<impl Into<Filter>>) -> Self {
        Self::Disconnect(filters.into_iter().map(Into::into).collect())
    }

    /// Set the relation to exactly these records.
    pub fn set(filters: Vec<impl Into<Filter>>) -> Self {
        Self::Set(filters.into_iter().map(Into::into).collect())
    }

    /// Delete related records.
    pub fn delete(filters: Vec<impl Into<Filter>>) -> Self {
        Self::Delete(filters.into_iter().map(Into::into).collect())
    }

    /// Delete many related records matching a filter.
    pub fn delete_many(filter: impl Into<Filter>) -> Self {
        Self::DeleteMany(filter.into())
    }
}

/// Data for creating a nested record.
#[derive(Debug, Clone)]
pub struct NestedCreateData<T: Model> {
    /// The create data fields.
    pub data: Vec<(String, FilterValue)>,
    /// Marker for the model type.
    _model: PhantomData<T>,
}

impl<T: Model> NestedCreateData<T> {
    /// Create new nested create data.
    pub fn new(data: Vec<(String, FilterValue)>) -> Self {
        Self {
            data,
            _model: PhantomData,
        }
    }

    /// Create from field-value pairs.
    pub fn from_pairs(
        pairs: impl IntoIterator<Item = (impl Into<String>, impl Into<FilterValue>)>,
    ) -> Self {
        Self::new(
            pairs
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        )
    }
}

impl<T: Model> Default for NestedCreateData<T> {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

/// Data for creating or connecting a nested record.
#[derive(Debug, Clone)]
pub struct NestedCreateOrConnectData<T: Model> {
    /// Filter to find existing record.
    pub filter: Filter,
    /// Data to create if not found.
    pub create: NestedCreateData<T>,
}

impl<T: Model> NestedCreateOrConnectData<T> {
    /// Create new create-or-connect data.
    pub fn new(filter: impl Into<Filter>, create: NestedCreateData<T>) -> Self {
        Self {
            filter: filter.into(),
            create,
        }
    }
}

/// Data for updating a nested record.
#[derive(Debug, Clone)]
pub struct NestedUpdateData<T: Model> {
    /// Filter to find the record to update.
    pub filter: Filter,
    /// The update data fields.
    pub data: Vec<(String, FilterValue)>,
    /// Marker for the model type.
    _model: PhantomData<T>,
}

impl<T: Model> NestedUpdateData<T> {
    /// Create new nested update data.
    pub fn new(filter: impl Into<Filter>, data: Vec<(String, FilterValue)>) -> Self {
        Self {
            filter: filter.into(),
            data,
            _model: PhantomData,
        }
    }

    /// Create from filter and field-value pairs.
    pub fn from_pairs(
        filter: impl Into<Filter>,
        pairs: impl IntoIterator<Item = (impl Into<String>, impl Into<FilterValue>)>,
    ) -> Self {
        Self::new(
            filter,
            pairs
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        )
    }
}

/// Data for upserting a nested record.
#[derive(Debug, Clone)]
pub struct NestedUpsertData<T: Model> {
    /// Filter to find existing record.
    pub filter: Filter,
    /// Data to create if not found.
    pub create: NestedCreateData<T>,
    /// Data to update if found.
    pub update: Vec<(String, FilterValue)>,
    /// Marker for the model type.
    _model: PhantomData<T>,
}

impl<T: Model> NestedUpsertData<T> {
    /// Create new nested upsert data.
    pub fn new(
        filter: impl Into<Filter>,
        create: NestedCreateData<T>,
        update: Vec<(String, FilterValue)>,
    ) -> Self {
        Self {
            filter: filter.into(),
            create,
            update,
            _model: PhantomData,
        }
    }
}

/// Data for updating many nested records.
#[derive(Debug, Clone)]
pub struct NestedUpdateManyData<T: Model> {
    /// Filter to match records.
    pub filter: Filter,
    /// The update data fields.
    pub data: Vec<(String, FilterValue)>,
    /// Marker for the model type.
    _model: PhantomData<T>,
}

impl<T: Model> NestedUpdateManyData<T> {
    /// Create new nested update-many data.
    pub fn new(filter: impl Into<Filter>, data: Vec<(String, FilterValue)>) -> Self {
        Self {
            filter: filter.into(),
            data,
            _model: PhantomData,
        }
    }
}

/// Builder for nested write SQL operations.
///
/// The SQL emitters here currently bake in [`crate::dialect::Postgres`] —
/// nested writes are not yet wired into a live client, and the placeholder
/// syntax (`$N`) is Postgres-shaped. When this builder lands on the live
/// client path the dialect should thread through from the engine.
#[derive(Debug)]
pub struct NestedWriteBuilder {
    /// The parent table name.
    parent_table: String,
    /// The parent primary key column(s).
    parent_pk: Vec<String>,
    /// The related table name.
    related_table: String,
    /// The foreign key column on the related table.
    foreign_key: String,
    /// Whether this is a one-to-many (true) or many-to-many (false) relation.
    is_one_to_many: bool,
    /// Join table for many-to-many relations.
    join_table: Option<JoinTableInfo>,
}

/// Information about a join table for many-to-many relations.
#[derive(Debug, Clone)]
pub struct JoinTableInfo {
    /// The join table name.
    pub table_name: String,
    /// Column referencing the parent table.
    pub parent_column: String,
    /// Column referencing the related table.
    pub related_column: String,
}

impl NestedWriteBuilder {
    /// Create a builder for a one-to-many relation.
    pub fn one_to_many(
        parent_table: impl Into<String>,
        parent_pk: Vec<String>,
        related_table: impl Into<String>,
        foreign_key: impl Into<String>,
    ) -> Self {
        Self {
            parent_table: parent_table.into(),
            parent_pk,
            related_table: related_table.into(),
            foreign_key: foreign_key.into(),
            is_one_to_many: true,
            join_table: None,
        }
    }

    /// Create a builder for a many-to-many relation.
    pub fn many_to_many(
        parent_table: impl Into<String>,
        parent_pk: Vec<String>,
        related_table: impl Into<String>,
        join_table: JoinTableInfo,
    ) -> Self {
        Self {
            parent_table: parent_table.into(),
            parent_pk,
            related_table: related_table.into(),
            foreign_key: String::new(), // Not used for many-to-many
            is_one_to_many: false,
            join_table: Some(join_table),
        }
    }

    /// Build SQL for connecting records.
    pub fn build_connect_sql<T: Model>(
        &self,
        parent_id: &FilterValue,
        filters: &[Filter],
    ) -> Vec<(String, Vec<FilterValue>)> {
        let mut statements = Vec::new();

        if self.is_one_to_many {
            // For one-to-many, update the foreign key on related records
            for filter in filters {
                let (where_sql, mut params) = filter.to_sql(1, &crate::dialect::Postgres);
                let sql = format!(
                    "UPDATE {} SET {} = ${} WHERE {}",
                    quote_identifier(&self.related_table),
                    quote_identifier(&self.foreign_key),
                    params.len() + 1,
                    where_sql
                );
                params.push(parent_id.clone());
                statements.push((sql, params));
            }
        } else if let Some(join) = &self.join_table {
            // For many-to-many, insert into join table
            // First, we need to get the IDs of the related records
            for filter in filters {
                let (where_sql, mut params) = filter.to_sql(1, &crate::dialect::Postgres);

                // Get the related record ID (assuming single-column PK for now)
                let select_sql = format!(
                    "SELECT {} FROM {} WHERE {}",
                    quote_identifier(T::PRIMARY_KEY.first().unwrap_or(&"id")),
                    quote_identifier(&self.related_table),
                    where_sql
                );

                // Insert into join table
                let insert_sql = format!(
                    "INSERT INTO {} ({}, {}) SELECT ${}, {} FROM {} WHERE {} ON CONFLICT DO NOTHING",
                    quote_identifier(&join.table_name),
                    quote_identifier(&join.parent_column),
                    quote_identifier(&join.related_column),
                    params.len() + 1,
                    quote_identifier(T::PRIMARY_KEY.first().unwrap_or(&"id")),
                    quote_identifier(&self.related_table),
                    where_sql
                );
                params.push(parent_id.clone());
                statements.push((insert_sql, params));
                // Keep select_sql for potential subquery use
                let _ = select_sql;
            }
        }

        statements
    }

    /// Build SQL for disconnecting records.
    pub fn build_disconnect_sql(
        &self,
        parent_id: &FilterValue,
        filters: &[Filter],
    ) -> Vec<(String, Vec<FilterValue>)> {
        let mut statements = Vec::new();

        if self.is_one_to_many {
            // For one-to-many, set the foreign key to NULL
            for filter in filters {
                let (where_sql, mut params) = filter.to_sql(1, &crate::dialect::Postgres);
                let sql = format!(
                    "UPDATE {} SET {} = NULL WHERE {} AND {} = ${}",
                    quote_identifier(&self.related_table),
                    quote_identifier(&self.foreign_key),
                    where_sql,
                    quote_identifier(&self.foreign_key),
                    params.len() + 1
                );
                params.push(parent_id.clone());
                statements.push((sql, params));
            }
        } else if let Some(join) = &self.join_table {
            // For many-to-many, delete from join table
            for filter in filters {
                let (where_sql, mut params) = filter.to_sql(2, &crate::dialect::Postgres);
                let sql = format!(
                    "DELETE FROM {} WHERE {} = $1 AND {} IN (SELECT id FROM {} WHERE {})",
                    quote_identifier(&join.table_name),
                    quote_identifier(&join.parent_column),
                    quote_identifier(&join.related_column),
                    quote_identifier(&self.related_table),
                    where_sql
                );
                let mut final_params = vec![parent_id.clone()];
                final_params.extend(params);
                params = final_params;
                statements.push((sql, params));
            }
        }

        statements
    }

    /// Build SQL for setting the relation (disconnect all, then connect specified).
    pub fn build_set_sql<T: Model>(
        &self,
        parent_id: &FilterValue,
        filters: &[Filter],
    ) -> Vec<(String, Vec<FilterValue>)> {
        let mut statements = Vec::new();

        // First, disconnect all existing relations
        if self.is_one_to_many {
            let sql = format!(
                "UPDATE {} SET {} = NULL WHERE {} = $1",
                quote_identifier(&self.related_table),
                quote_identifier(&self.foreign_key),
                quote_identifier(&self.foreign_key)
            );
            statements.push((sql, vec![parent_id.clone()]));
        } else if let Some(join) = &self.join_table {
            let sql = format!(
                "DELETE FROM {} WHERE {} = $1",
                quote_identifier(&join.table_name),
                quote_identifier(&join.parent_column)
            );
            statements.push((sql, vec![parent_id.clone()]));
        }

        // Then connect the specified records
        statements.extend(self.build_connect_sql::<T>(parent_id, filters));

        statements
    }

    /// Build SQL for creating nested records.
    pub fn build_create_sql<T: Model>(
        &self,
        parent_id: &FilterValue,
        creates: &[NestedCreateData<T>],
    ) -> Vec<(String, Vec<FilterValue>)> {
        let mut statements = Vec::with_capacity(creates.len());
        let quoted_table = quote_identifier(&self.related_table);

        for create in creates {
            let row_len = create.data.len() + 1;
            let mut columns: Vec<String> = Vec::with_capacity(row_len);
            let mut values: Vec<FilterValue> = Vec::with_capacity(row_len);
            for (k, v) in &create.data {
                columns.push(k.clone());
                values.push(v.clone());
            }

            columns.push(self.foreign_key.clone());
            values.push(parent_id.clone());

            let mut col_list = String::new();
            let mut placeholders = String::new();
            for (i, c) in columns.iter().enumerate() {
                if i > 0 {
                    col_list.push_str(", ");
                    placeholders.push_str(", ");
                }
                col_list.push_str(&quote_identifier(c));
                use std::fmt::Write;
                let _ = write!(placeholders, "${}", i + 1);
            }

            let sql = format!(
                "INSERT INTO {} ({}) VALUES ({}) RETURNING *",
                quoted_table, col_list, placeholders,
            );

            statements.push((sql, values));
        }

        statements
    }

    /// Build SQL for deleting nested records.
    pub fn build_delete_sql(
        &self,
        parent_id: &FilterValue,
        filters: &[Filter],
    ) -> Vec<(String, Vec<FilterValue>)> {
        let mut statements = Vec::new();

        for filter in filters {
            let (where_sql, mut params) = filter.to_sql(1, &crate::dialect::Postgres);
            let sql = format!(
                "DELETE FROM {} WHERE {} AND {} = ${}",
                quote_identifier(&self.related_table),
                where_sql,
                quote_identifier(&self.foreign_key),
                params.len() + 1
            );
            params.push(parent_id.clone());
            statements.push((sql, params));
        }

        statements
    }
}

/// A container for collecting all nested write operations to execute.
#[derive(Debug, Default)]
pub struct NestedWriteOperations {
    /// SQL statements to execute before the main operation.
    pub pre_statements: Vec<(String, Vec<FilterValue>)>,
    /// SQL statements to execute after the main operation.
    pub post_statements: Vec<(String, Vec<FilterValue>)>,
}

impl NestedWriteOperations {
    /// Create a new empty container.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a pre-operation statement.
    pub fn add_pre(&mut self, sql: String, params: Vec<FilterValue>) {
        self.pre_statements.push((sql, params));
    }

    /// Add a post-operation statement.
    pub fn add_post(&mut self, sql: String, params: Vec<FilterValue>) {
        self.post_statements.push((sql, params));
    }

    /// Extend with statements from another container.
    pub fn extend(&mut self, other: Self) {
        self.pre_statements.extend(other.pre_statements);
        self.post_statements.extend(other.post_statements);
    }

    /// Check if there are any operations.
    pub fn is_empty(&self) -> bool {
        self.pre_statements.is_empty() && self.post_statements.is_empty()
    }

    /// Get total number of statements.
    pub fn len(&self) -> usize {
        self.pre_statements.len() + self.post_statements.len()
    }
}

/// Model-erased nested write op used by `CreateOperation::with(...)`.
///
/// The type-parameterized [`NestedWrite`] above is keyed on the parent
/// model and doesn't compose across heterogeneous child types — a
/// `CreateOperation<E, User>.with(posts_write)` needs to carry child
/// writes for a different model (`Post`) than the parent, so `User`'s
/// `NestedWrite<User>` can't encode them. This sibling enum drops the
/// model type parameter and carries only the runtime metadata the
/// execution path actually needs: the target table, the foreign-key
/// column on that table, and the raw child-column payload.
///
/// Emitted by the codegen's per-relation `create()` / `connect()`
/// helpers on `user::posts::*`. Payloads are a nested
/// `Vec<Vec<(String, FilterValue)>>` rather than a strongly-typed
/// `CreateInput` because the derive path doesn't currently emit a
/// `CreateInput` struct per model — see the task docs for the trade-off
/// and the upgrade path.
#[derive(Debug, Clone)]
pub enum NestedWriteOp {
    /// Create children whose FK column points at the parent's PK.
    ///
    /// `relation` is retained for diagnostics/debugging; the executor
    /// only needs `target_table`, `foreign_key`, and `payload`.
    Create {
        relation: &'static str,
        target_table: &'static str,
        foreign_key: &'static str,
        /// One `Vec<(column, value)>` per child row. The FK column +
        /// parent PK are appended by [`NestedWriteOp::execute`].
        payload: Vec<Vec<(String, FilterValue)>>,
    },
    /// Connect an existing child row by its primary-key value.
    ///
    /// Lowers to
    /// `UPDATE <target_table> SET <foreign_key> = <parent_pk> WHERE <target_pk> = <pk>`
    /// at execute time. The identifier fields are `&'static str` because
    /// they come from codegen-emitted constants on the per-relation
    /// `RelationMeta` / `Model` types — the type itself enforces the
    /// SQL-safety boundary (see `.cursor/rules/sql-safety.mdc`). Only
    /// `parent_pk` and `pk` flow as `$N`-bound parameters.
    Connect {
        relation: &'static str,
        target_table: &'static str,
        foreign_key: &'static str,
        target_pk: &'static str,
        pk: FilterValue,
    },
    /// Disconnect a child row by clearing its FK column to `NULL`.
    ///
    /// Lowers to `UPDATE <target_table> SET <foreign_key> = NULL WHERE <target_pk> = <pk>`.
    /// The child row persists; only the FK is cleared. Use
    /// [`NestedWriteOp::Delete`] to remove the row entirely.
    Disconnect {
        relation: &'static str,
        target_table: &'static str,
        foreign_key: &'static str,
        target_pk: &'static str,
        pk: FilterValue,
    },
    /// Delete a child row by its primary key.
    ///
    /// Lowers to `DELETE FROM <target_table> WHERE <target_pk> = <pk>`.
    /// Returns `QueryError::not_found` when the PK doesn't match any row,
    /// matching the Connect-batch affected-rows contract.
    Delete {
        relation: &'static str,
        target_table: &'static str,
        target_pk: &'static str,
        pk: FilterValue,
    },
    /// Delete many child rows matching a scalar filter, scoped to the
    /// parent's children only.
    ///
    /// Lowers to `DELETE FROM <target_table> WHERE <foreign_key> = <parent_pk> AND <filter>`.
    /// The AND-with-parent-FK clause is a safety bound enforced at SQL
    /// emit time — the user-supplied filter cannot remove rows belonging
    /// to other parents.
    DeleteMany {
        relation: &'static str,
        target_table: &'static str,
        foreign_key: &'static str,
        filter: Filter,
    },
    /// Update a child row by its primary key.
    ///
    /// Lowers to
    /// `UPDATE <target_table> SET <writeop-fragments> WHERE <target_pk> = $1`.
    /// Each entry in `payload` contributes one column assignment whose
    /// shape is determined by the [`crate::inputs::WriteOp`] variant
    /// (plain set, atomic increment/decrement/multiply/divide, or
    /// null-out via Unset). Returns `QueryError::not_found` when the PK
    /// doesn't match any row, mirroring [`NestedWriteOp::Delete`]'s
    /// affected-rows contract.
    Update {
        relation: &'static str,
        target_table: &'static str,
        target_pk: &'static str,
        pk: FilterValue,
        payload: Vec<(String, crate::inputs::WriteOp)>,
    },
}

impl NestedWriteOp {
    /// Execute this nested write inside `engine`, using `parent_pk`
    /// as the foreign-key value to splice into each child row.
    ///
    /// For `Create`, this emits one `INSERT INTO <target_table> (...)`
    /// per child, appending the FK column + parent PK to whatever
    /// columns/values the caller supplied.
    pub async fn execute<E>(self, engine: &E, parent_pk: &FilterValue) -> QueryResult<()>
    where
        E: QueryEngine,
    {
        match self {
            NestedWriteOp::Connect {
                relation: _,
                target_table,
                foreign_key,
                target_pk,
                pk,
            } => {
                let dialect = engine.dialect();
                let sql = format!(
                    "UPDATE {} SET {} = {} WHERE {} = {}",
                    dialect.quote_ident(target_table),
                    dialect.quote_ident(foreign_key),
                    dialect.placeholder(1),
                    dialect.quote_ident(target_pk),
                    dialect.placeholder(2),
                );
                engine
                    .execute_raw(&sql, vec![parent_pk.clone(), pk])
                    .await?;
                Ok(())
            }
            NestedWriteOp::Disconnect {
                relation: _,
                target_table,
                foreign_key,
                target_pk,
                pk,
            } => {
                let dialect = engine.dialect();
                let sql = format!(
                    "UPDATE {} SET {} = NULL WHERE {} = {}",
                    dialect.quote_ident(target_table),
                    dialect.quote_ident(foreign_key),
                    dialect.quote_ident(target_pk),
                    dialect.placeholder(1),
                );
                engine.execute_raw(&sql, vec![pk]).await?;
                Ok(())
            }
            NestedWriteOp::Delete {
                relation: _,
                target_table,
                target_pk,
                pk,
            } => {
                let dialect = engine.dialect();
                let sql = format!(
                    "DELETE FROM {} WHERE {} = {}",
                    dialect.quote_ident(target_table),
                    dialect.quote_ident(target_pk),
                    dialect.placeholder(1),
                );
                let affected = engine.execute_raw(&sql, vec![pk]).await?;
                if affected != 1 {
                    return Err(crate::error::QueryError::not_found(target_table)
                        .with_context("Nested Delete by PK"));
                }
                Ok(())
            }
            NestedWriteOp::DeleteMany {
                relation: _,
                target_table,
                foreign_key,
                filter,
            } => {
                let dialect = engine.dialect();
                let is_unconstrained = matches!(filter, Filter::None);
                let sql = if is_unconstrained {
                    format!(
                        "DELETE FROM {} WHERE {} = {}",
                        dialect.quote_ident(target_table),
                        dialect.quote_ident(foreign_key),
                        dialect.placeholder(1),
                    )
                } else {
                    let (filter_sql, params_tail) = filter.to_sql(2, &crate::dialect::Postgres);
                    let sql = format!(
                        "DELETE FROM {} WHERE {} = {} AND ({})",
                        dialect.quote_ident(target_table),
                        dialect.quote_ident(foreign_key),
                        dialect.placeholder(1),
                        filter_sql,
                    );
                    let mut params = Vec::with_capacity(params_tail.len() + 1);
                    params.push(parent_pk.clone());
                    params.extend(params_tail);
                    return engine.execute_raw(&sql, params).await.map(|_| ());
                };
                engine.execute_raw(&sql, vec![parent_pk.clone()]).await?;
                Ok(())
            }
            NestedWriteOp::Update {
                relation: _,
                target_table,
                target_pk,
                pk,
                payload,
            } => {
                if payload.is_empty() {
                    return Ok(());
                }
                let dialect = engine.dialect();
                let mut set_fragments: Vec<String> = Vec::with_capacity(payload.len());
                let mut params: Vec<FilterValue> = Vec::with_capacity(payload.len() + 1);
                let mut next_placeholder = 1usize;
                for (col, op) in payload {
                    let (frag, maybe_val) = op.to_set_fragment(
                        &dialect.quote_ident(&col),
                        &dialect.placeholder(next_placeholder),
                    );
                    set_fragments.push(frag);
                    if let Some(val) = maybe_val {
                        params.push(val);
                        next_placeholder += 1;
                    }
                }
                params.push(pk);
                let sql = format!(
                    "UPDATE {} SET {} WHERE {} = {}",
                    dialect.quote_ident(target_table),
                    set_fragments.join(", "),
                    dialect.quote_ident(target_pk),
                    dialect.placeholder(next_placeholder),
                );
                let affected = engine.execute_raw(&sql, params).await?;
                if affected != 1 {
                    return Err(crate::error::QueryError::not_found(target_table)
                        .with_context("Nested Update by PK"));
                }
                Ok(())
            }
            NestedWriteOp::Create {
                relation: _,
                target_table,
                foreign_key,
                payload,
            } => {
                if payload.is_empty() {
                    return Ok(());
                }

                let dialect = engine.dialect();

                // All rows in a single `Create` op share the same column
                // set (codegen guarantee). Derive columns from the first
                // row, then append the FK column once. Each row
                // contributes its values + the parent PK.
                let first = &payload[0];
                let mut columns: Vec<String> = first.iter().map(|(c, _)| c.clone()).collect();
                columns.push(foreign_key.to_string());
                let cols_per_row = columns.len();

                let quoted_cols: Vec<String> =
                    columns.iter().map(|c| dialect.quote_ident(c)).collect();

                let mut values: Vec<FilterValue> = Vec::with_capacity(payload.len() * cols_per_row);
                let mut row_placeholders: Vec<String> = Vec::with_capacity(payload.len());
                let mut next_placeholder = 1usize;

                for child in payload {
                    let mut row_phs: Vec<String> = Vec::with_capacity(cols_per_row);
                    for (_, v) in child {
                        values.push(v);
                        row_phs.push(dialect.placeholder(next_placeholder));
                        next_placeholder += 1;
                    }
                    values.push(parent_pk.clone());
                    row_phs.push(dialect.placeholder(next_placeholder));
                    next_placeholder += 1;
                    row_placeholders.push(format!("({})", row_phs.join(", ")));
                }

                // NOTE: Combining all rows into a single multi-VALUES
                // INSERT means any constraint failure rolls back the
                // entire batch, not just the failing row. This matches
                // typical Prisma semantics for nested writes.
                let sql = format!(
                    "INSERT INTO {} ({}) VALUES {}",
                    dialect.quote_ident(target_table),
                    quoted_cols.join(", "),
                    row_placeholders.join(", "),
                );

                engine.execute_raw(&sql, values).await?;
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use crate::error::QueryError;
    use crate::traits::BoxFuture;

    /// Captured (sql, params) entries from the mock engine.
    type StatementLog = Arc<Mutex<Vec<(String, Vec<FilterValue>)>>>;

    /// Recording mock engine for [`NestedWriteOp::execute`] tests.
    ///
    /// Captures the (sql, params) of every [`QueryEngine::execute_raw`]
    /// call so tests can assert the lowered shape.
    #[derive(Clone)]
    struct RecordingEngine {
        recorded: StatementLog,
    }

    impl RecordingEngine {
        fn new() -> Self {
            Self {
                recorded: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn statements(&self) -> Vec<(String, Vec<FilterValue>)> {
            self.recorded.lock().unwrap().clone()
        }
    }

    impl crate::traits::QueryEngine for RecordingEngine {
        fn dialect(&self) -> &dyn crate::dialect::SqlDialect {
            &crate::dialect::Postgres
        }

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
            Box::pin(async { Err(QueryError::not_found("test")) })
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
            Box::pin(async { Err(QueryError::not_found("test")) })
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
            sql: &str,
            params: Vec<FilterValue>,
        ) -> BoxFuture<'_, QueryResult<u64>> {
            let recorded = self.recorded.clone();
            let sql = sql.to_string();
            Box::pin(async move {
                recorded.lock().unwrap().push((sql, params));
                Ok(1)
            })
        }

        fn count(&self, _sql: &str, _params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
            Box::pin(async { Ok(0) })
        }
    }

    struct TestModel;

    impl Model for TestModel {
        const MODEL_NAME: &'static str = "Post";
        const TABLE_NAME: &'static str = "posts";
        const PRIMARY_KEY: &'static [&'static str] = &["id"];
        const COLUMNS: &'static [&'static str] = &["id", "title", "user_id"];
    }

    struct TagModel;

    impl Model for TagModel {
        const MODEL_NAME: &'static str = "Tag";
        const TABLE_NAME: &'static str = "tags";
        const PRIMARY_KEY: &'static [&'static str] = &["id"];
        const COLUMNS: &'static [&'static str] = &["id", "name"];
    }

    #[test]
    fn test_nested_create_data() {
        let data: NestedCreateData<TestModel> =
            NestedCreateData::from_pairs([("title", FilterValue::String("Test Post".to_string()))]);

        assert_eq!(data.data.len(), 1);
        assert_eq!(data.data[0].0, "title");
    }

    #[test]
    fn test_nested_write_create() {
        let data: NestedCreateData<TestModel> =
            NestedCreateData::from_pairs([("title", FilterValue::String("Test Post".to_string()))]);

        let write: NestedWrite<TestModel> = NestedWrite::create(data);

        match write {
            NestedWrite::Create(creates) => assert_eq!(creates.len(), 1),
            _ => panic!("Expected Create variant"),
        }
    }

    #[test]
    fn test_nested_write_connect() {
        let write: NestedWrite<TestModel> = NestedWrite::connect(vec![
            Filter::Equals("id".into(), FilterValue::Int(1)),
            Filter::Equals("id".into(), FilterValue::Int(2)),
        ]);

        match write {
            NestedWrite::Connect(filters) => assert_eq!(filters.len(), 2),
            _ => panic!("Expected Connect variant"),
        }
    }

    #[test]
    fn test_nested_write_disconnect() {
        let write: NestedWrite<TestModel> =
            NestedWrite::disconnect_one(Filter::Equals("id".into(), FilterValue::Int(1)));

        match write {
            NestedWrite::Disconnect(filters) => assert_eq!(filters.len(), 1),
            _ => panic!("Expected Disconnect variant"),
        }
    }

    #[test]
    fn test_nested_write_set() {
        let write: NestedWrite<TestModel> =
            NestedWrite::set(vec![Filter::Equals("id".into(), FilterValue::Int(1))]);

        match write {
            NestedWrite::Set(filters) => assert_eq!(filters.len(), 1),
            _ => panic!("Expected Set variant"),
        }
    }

    #[test]
    fn test_builder_one_to_many_connect() {
        let builder =
            NestedWriteBuilder::one_to_many("users", vec!["id".to_string()], "posts", "user_id");

        let parent_id = FilterValue::Int(1);
        let filters = vec![Filter::Equals("id".into(), FilterValue::Int(10))];

        let statements = builder.build_connect_sql::<TestModel>(&parent_id, &filters);

        assert_eq!(statements.len(), 1);
        let (sql, params) = &statements[0];
        assert!(sql.contains("UPDATE"));
        assert!(sql.contains("posts"));
        assert!(sql.contains("user_id"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_builder_one_to_many_disconnect() {
        let builder =
            NestedWriteBuilder::one_to_many("users", vec!["id".to_string()], "posts", "user_id");

        let parent_id = FilterValue::Int(1);
        let filters = vec![Filter::Equals("id".into(), FilterValue::Int(10))];

        let statements = builder.build_disconnect_sql(&parent_id, &filters);

        assert_eq!(statements.len(), 1);
        let (sql, params) = &statements[0];
        assert!(sql.contains("UPDATE"));
        assert!(sql.contains("SET"));
        assert!(sql.contains("NULL"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_builder_many_to_many_connect() {
        let builder = NestedWriteBuilder::many_to_many(
            "posts",
            vec!["id".to_string()],
            "tags",
            JoinTableInfo {
                table_name: "post_tags".to_string(),
                parent_column: "post_id".to_string(),
                related_column: "tag_id".to_string(),
            },
        );

        let parent_id = FilterValue::Int(1);
        let filters = vec![Filter::Equals("id".into(), FilterValue::Int(10))];

        let statements = builder.build_connect_sql::<TagModel>(&parent_id, &filters);

        assert_eq!(statements.len(), 1);
        let (sql, _params) = &statements[0];
        assert!(sql.contains("INSERT INTO"));
        assert!(sql.contains("post_tags"));
        assert!(sql.contains("ON CONFLICT DO NOTHING"));
    }

    #[test]
    fn test_builder_create() {
        let builder =
            NestedWriteBuilder::one_to_many("users", vec!["id".to_string()], "posts", "user_id");

        let parent_id = FilterValue::Int(1);
        let creates = vec![NestedCreateData::<TestModel>::from_pairs([(
            "title",
            FilterValue::String("New Post".to_string()),
        )])];

        let statements = builder.build_create_sql::<TestModel>(&parent_id, &creates);

        assert_eq!(statements.len(), 1);
        let (sql, params) = &statements[0];
        assert!(sql.contains("INSERT INTO"));
        assert!(sql.contains("posts"));
        assert!(sql.contains("RETURNING"));
        assert_eq!(params.len(), 2); // title + user_id
    }

    #[test]
    fn test_builder_set() {
        let builder =
            NestedWriteBuilder::one_to_many("users", vec!["id".to_string()], "posts", "user_id");

        let parent_id = FilterValue::Int(1);
        let filters = vec![Filter::Equals("id".into(), FilterValue::Int(10))];

        let statements = builder.build_set_sql::<TestModel>(&parent_id, &filters);

        // Should have disconnect all + connect statements
        assert!(statements.len() >= 2);

        // First statement should disconnect all
        let (first_sql, _) = &statements[0];
        assert!(first_sql.contains("UPDATE"));
        assert!(first_sql.contains("NULL"));
    }

    #[test]
    fn test_nested_write_operations() {
        let mut ops = NestedWriteOperations::new();
        assert!(ops.is_empty());
        assert_eq!(ops.len(), 0);

        ops.add_pre("SELECT 1".to_string(), vec![]);
        ops.add_post("SELECT 2".to_string(), vec![]);

        assert!(!ops.is_empty());
        assert_eq!(ops.len(), 2);
    }

    #[test]
    fn test_nested_create_or_connect() {
        let create_data: NestedCreateData<TestModel> =
            NestedCreateData::from_pairs([("title", FilterValue::String("New Post".to_string()))]);

        let create_or_connect = NestedCreateOrConnectData::new(
            Filter::Equals("title".into(), FilterValue::String("Existing".to_string())),
            create_data,
        );

        assert!(matches!(create_or_connect.filter, Filter::Equals(..)));
        assert_eq!(create_or_connect.create.data.len(), 1);
    }

    #[test]
    fn test_nested_update_data() {
        let update: NestedUpdateData<TestModel> = NestedUpdateData::from_pairs(
            Filter::Equals("id".into(), FilterValue::Int(1)),
            [("title", FilterValue::String("Updated".to_string()))],
        );

        assert!(matches!(update.filter, Filter::Equals(..)));
        assert_eq!(update.data.len(), 1);
        assert_eq!(update.data[0].0, "title");
    }

    #[tokio::test]
    async fn nested_op_connect_emits_update_set_where() {
        let engine = RecordingEngine::new();
        let op = NestedWriteOp::Connect {
            relation: "posts",
            target_table: "posts",
            foreign_key: "author_id",
            target_pk: "id",
            pk: FilterValue::Int(42),
        };
        let parent_pk = FilterValue::Int(7);
        op.execute(&engine, &parent_pk).await.unwrap();

        let stmts = engine.statements();
        assert_eq!(stmts.len(), 1, "expected one UPDATE statement");
        let (sql, params) = &stmts[0];
        // Postgres dialect quotes idents with double quotes.
        assert!(sql.contains("UPDATE"), "got: {sql}");
        assert!(sql.contains("posts"), "got: {sql}");
        assert!(sql.contains("author_id"), "got: {sql}");
        assert!(sql.contains("SET"), "got: {sql}");
        assert!(sql.contains("WHERE"), "got: {sql}");
        assert!(sql.contains("$1"), "got: {sql}");
        assert!(sql.contains("$2"), "got: {sql}");
        assert_eq!(params, &vec![FilterValue::Int(7), FilterValue::Int(42)]);
    }

    #[tokio::test]
    async fn nested_op_delete_many_with_filter_emits_fk_and_filter_clause() {
        let engine = RecordingEngine::new();
        let op = NestedWriteOp::DeleteMany {
            relation: "posts",
            target_table: "posts",
            foreign_key: "author_id",
            filter: Filter::Equals("published".into(), FilterValue::Bool(false)),
        };
        op.execute(&engine, &FilterValue::Int(7)).await.unwrap();

        let stmts = engine.statements();
        assert_eq!(stmts.len(), 1);
        let (sql, params) = &stmts[0];
        assert!(sql.contains("DELETE FROM"), "got: {sql}");
        assert!(sql.contains("author_id"), "got: {sql}");
        assert!(sql.contains("$1"), "got: {sql}");
        assert!(sql.contains("AND"), "got: {sql}");
        assert!(sql.contains("published"), "got: {sql}");
        assert_eq!(params.len(), 2);
        assert!(matches!(params[0], FilterValue::Int(7)));
        assert!(matches!(params[1], FilterValue::Bool(false)));
    }

    #[tokio::test]
    async fn nested_op_delete_many_with_empty_filter_omits_and_clause() {
        let engine = RecordingEngine::new();
        let op = NestedWriteOp::DeleteMany {
            relation: "posts",
            target_table: "posts",
            foreign_key: "author_id",
            filter: Filter::None,
        };
        op.execute(&engine, &FilterValue::Int(7)).await.unwrap();

        let stmts = engine.statements();
        let (sql, params) = &stmts[0];
        assert!(sql.contains("DELETE FROM"), "got: {sql}");
        assert!(
            !sql.contains("AND"),
            "should omit AND when filter empty: {sql}"
        );
        assert_eq!(params.len(), 1);
    }

    #[tokio::test]
    async fn nested_op_delete_emits_delete_where_pk() {
        let engine = RecordingEngine::new();
        let op = NestedWriteOp::Delete {
            relation: "posts",
            target_table: "posts",
            target_pk: "id",
            pk: FilterValue::Int(42),
        };
        op.execute(&engine, &FilterValue::Int(7)).await.unwrap();

        let stmts = engine.statements();
        assert_eq!(stmts.len(), 1);
        let (sql, params) = &stmts[0];
        assert!(sql.contains("DELETE FROM"), "got: {sql}");
        assert!(sql.contains("posts"), "got: {sql}");
        assert!(sql.contains("WHERE"), "got: {sql}");
        assert_eq!(params, &vec![FilterValue::Int(42)]);
    }

    #[tokio::test]
    async fn nested_op_disconnect_emits_update_set_null() {
        let engine = RecordingEngine::new();
        let op = NestedWriteOp::Disconnect {
            relation: "posts",
            target_table: "posts",
            foreign_key: "author_id",
            target_pk: "id",
            pk: FilterValue::Int(42),
        };
        op.execute(&engine, &FilterValue::Int(7)).await.unwrap();

        let stmts = engine.statements();
        assert_eq!(stmts.len(), 1);
        let (sql, params) = &stmts[0];
        assert!(sql.contains("UPDATE"), "got: {sql}");
        assert!(sql.contains("posts"), "got: {sql}");
        assert!(sql.contains("author_id"), "got: {sql}");
        assert!(sql.contains("NULL"), "got: {sql}");
        assert!(sql.contains("WHERE"), "got: {sql}");
        assert_eq!(params, &vec![FilterValue::Int(42)]);
    }

    #[tokio::test]
    async fn nested_op_update_plain_set() {
        use crate::inputs::WriteOp;
        let engine = RecordingEngine::new();
        let op = NestedWriteOp::Update {
            relation: "posts",
            target_table: "posts",
            target_pk: "id",
            pk: FilterValue::Int(42),
            payload: vec![(
                "title".to_string(),
                WriteOp::Set(FilterValue::String("renamed".to_string())),
            )],
        };
        op.execute(&engine, &FilterValue::Int(7)).await.unwrap();

        let stmts = engine.statements();
        assert_eq!(stmts.len(), 1);
        let (sql, params) = &stmts[0];
        assert!(sql.contains("UPDATE"), "got: {sql}");
        assert!(sql.contains("posts"), "got: {sql}");
        assert!(sql.contains("title"), "got: {sql}");
        assert!(sql.contains("SET"), "got: {sql}");
        assert!(sql.contains("WHERE"), "got: {sql}");
        assert!(sql.contains("$1"), "got: {sql}");
        assert!(sql.contains("$2"), "got: {sql}");
        assert_eq!(params.len(), 2);
        assert!(matches!(params[0], FilterValue::String(_)));
        assert_eq!(params[1], FilterValue::Int(42));
    }

    #[tokio::test]
    async fn nested_op_update_increment() {
        use crate::inputs::WriteOp;
        let engine = RecordingEngine::new();
        let op = NestedWriteOp::Update {
            relation: "posts",
            target_table: "posts",
            target_pk: "id",
            pk: FilterValue::Int(42),
            payload: vec![("views".to_string(), WriteOp::Increment(FilterValue::Int(1)))],
        };
        op.execute(&engine, &FilterValue::Int(7)).await.unwrap();

        let stmts = engine.statements();
        let (sql, _) = &stmts[0];
        // Postgres dialect quotes idents — fragment will read `"views" = "views" + $1`.
        assert!(sql.contains("+"), "got: {sql}");
        assert!(sql.contains("views"), "got: {sql}");
    }

    #[tokio::test]
    async fn nested_op_update_mixed_set_and_increment() {
        use crate::inputs::WriteOp;
        let engine = RecordingEngine::new();
        let op = NestedWriteOp::Update {
            relation: "posts",
            target_table: "posts",
            target_pk: "id",
            pk: FilterValue::Int(42),
            payload: vec![
                (
                    "title".to_string(),
                    WriteOp::Set(FilterValue::String("renamed".to_string())),
                ),
                ("views".to_string(), WriteOp::Increment(FilterValue::Int(1))),
            ],
        };
        op.execute(&engine, &FilterValue::Int(7)).await.unwrap();

        let stmts = engine.statements();
        let (sql, params) = &stmts[0];
        assert!(sql.contains("title"), "got: {sql}");
        assert!(sql.contains("views"), "got: {sql}");
        assert!(sql.contains("+"), "got: {sql}");
        // 2 SET params + 1 PK = 3 placeholders.
        assert!(sql.contains("$3"), "got: {sql}");
        assert_eq!(params.len(), 3);
    }

    #[tokio::test]
    async fn nested_op_update_empty_payload_is_noop() {
        let engine = RecordingEngine::new();
        let op = NestedWriteOp::Update {
            relation: "posts",
            target_table: "posts",
            target_pk: "id",
            pk: FilterValue::Int(42),
            payload: vec![],
        };
        op.execute(&engine, &FilterValue::Int(7)).await.unwrap();
        assert!(
            engine.statements().is_empty(),
            "empty payload should emit no SQL"
        );
    }

    #[test]
    fn test_nested_upsert_data() {
        let create: NestedCreateData<TestModel> =
            NestedCreateData::from_pairs([("title", FilterValue::String("New".to_string()))]);

        let upsert: NestedUpsertData<TestModel> = NestedUpsertData::new(
            Filter::Equals("id".into(), FilterValue::Int(1)),
            create,
            vec![(
                "title".to_string(),
                FilterValue::String("Updated".to_string()),
            )],
        );

        assert!(matches!(upsert.filter, Filter::Equals(..)));
        assert_eq!(upsert.create.data.len(), 1);
        assert_eq!(upsert.update.len(), 1);
    }
}
