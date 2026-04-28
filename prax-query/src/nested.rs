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

use crate::filter::{Filter, FilterValue};
use crate::sql::quote_identifier;
use crate::traits::Model;

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
        let mut statements = Vec::new();

        for create in creates {
            let mut columns: Vec<String> = create.data.iter().map(|(k, _)| k.clone()).collect();
            let mut values: Vec<FilterValue> = create.data.iter().map(|(_, v)| v.clone()).collect();

            // Add the foreign key column
            columns.push(self.foreign_key.clone());
            values.push(parent_id.clone());

            let placeholders: Vec<String> = (1..=values.len()).map(|i| format!("${}", i)).collect();

            let sql = format!(
                "INSERT INTO {} ({}) VALUES ({}) RETURNING *",
                quote_identifier(&self.related_table),
                columns
                    .iter()
                    .map(|c| quote_identifier(c))
                    .collect::<Vec<_>>()
                    .join(", "),
                placeholders.join(", ")
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

#[cfg(test)]
mod tests {
    use super::*;

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
