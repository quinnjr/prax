//! MongoDB query engine implementation.

use std::marker::PhantomData;

use bson::{Document, doc};
use futures::TryStreamExt;
use mongodb::Collection;
use prax_query::QueryResult;
use prax_query::filter::FilterValue;
use prax_query::traits::{BoxFuture, Model, QueryEngine};
use tracing::debug;

use crate::client::MongoClient;
use crate::error::MongoError;
use crate::types::filter_value_to_bson;

/// MongoDB query engine that implements the Prax QueryEngine trait.
///
/// Note: MongoDB is a document database, so the SQL-oriented QueryEngine
/// trait methods are adapted to work with MongoDB operations.
#[derive(Clone)]
pub struct MongoEngine {
    client: MongoClient,
}

impl MongoEngine {
    /// Create a new MongoDB engine with the given client.
    pub fn new(client: MongoClient) -> Self {
        Self { client }
    }

    /// Get a reference to the client.
    pub fn client(&self) -> &MongoClient {
        &self.client
    }

    /// Get a typed collection for a model.
    pub fn collection<T>(&self) -> Collection<T>
    where
        T: Model + Send + Sync,
    {
        // Use the model name as the collection name (lowercase, pluralized)
        let collection_name = format!("{}s", T::MODEL_NAME.to_lowercase());
        self.client.collection(&collection_name)
    }

    /// Get a collection by explicit name.
    pub fn collection_by_name<T>(&self, name: &str) -> Collection<T>
    where
        T: Send + Sync,
    {
        self.client.collection(name)
    }

    /// Convert filter values to a MongoDB filter document.
    fn build_filter(sql: &str, params: &[FilterValue]) -> MongoResult<Document> {
        // For MongoDB, we expect the "sql" to actually be a JSON representation
        // of the filter document, or we parse it from a simple query format
        if sql.starts_with('{') {
            // JSON filter
            let filter: Document = serde_json::from_str(sql)
                .map_err(|e| MongoError::query(format!("invalid filter JSON: {}", e)))?;
            Ok(filter)
        } else if sql.is_empty() {
            // Empty filter - match all
            Ok(doc! {})
        } else {
            // Try to parse as a simple field=value format
            // For more complex queries, use the FilterBuilder
            let mut filter = Document::new();

            // Simple parsing: "field1=$1 AND field2=$2"
            for part in sql.split(" AND ") {
                let part = part.trim();
                if let Some(eq_pos) = part.find('=') {
                    let field = part[..eq_pos].trim();
                    let value_placeholder = part[eq_pos + 1..].trim();

                    // Check if it's a parameter placeholder ($1, $2, etc.)
                    if let Some(stripped) = value_placeholder.strip_prefix('$') {
                        if let Ok(param_idx) = stripped.parse::<usize>() {
                            if param_idx > 0 && param_idx <= params.len() {
                                let bson_value = filter_value_to_bson(&params[param_idx - 1])?;
                                filter.insert(field, bson_value);
                            }
                        }
                    } else {
                        // Direct value
                        filter.insert(field, value_placeholder);
                    }
                }
            }

            Ok(filter)
        }
    }
}

use crate::error::MongoResult;

impl QueryEngine for MongoEngine {
    fn query_many<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(filter = %sql, "Executing query_many");

            let filter = Self::build_filter(&sql, &params)
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            let collection = self
                .client
                .collection_doc(&format!("{}s", T::MODEL_NAME.to_lowercase()));

            let cursor = collection
                .find(filter, None)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            let docs: Vec<Document> = cursor
                .try_collect()
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            docs.iter()
                .map(|d| {
                    let row = crate::row_ref::BsonRowRef::new(d);
                    T::from_row(&row).map_err(|e| {
                        let msg = e.to_string();
                        prax_query::QueryError::deserialization(msg).with_source(e)
                    })
                })
                .collect()
        })
    }

    fn query_one<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<T>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(filter = %sql, "Executing query_one");

            let filter = Self::build_filter(&sql, &params)
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            let collection = self
                .client
                .collection_doc(&format!("{}s", T::MODEL_NAME.to_lowercase()));

            let doc = collection
                .find_one(filter, None)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?
                .ok_or_else(|| prax_query::QueryError::not_found(T::MODEL_NAME))?;

            let row = crate::row_ref::BsonRowRef::new(&doc);
            T::from_row(&row).map_err(|e| {
                let msg = e.to_string();
                prax_query::QueryError::deserialization(msg).with_source(e)
            })
        })
    }

    fn query_optional<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Option<T>>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(filter = %sql, "Executing query_optional");

            let filter = Self::build_filter(&sql, &params)
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            let collection = self
                .client
                .collection_doc(&format!("{}s", T::MODEL_NAME.to_lowercase()));

            let doc = collection
                .find_one(filter, None)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            match doc {
                Some(doc) => {
                    let row = crate::row_ref::BsonRowRef::new(&doc);
                    T::from_row(&row).map(Some).map_err(|e| {
                        let msg = e.to_string();
                        prax_query::QueryError::deserialization(msg).with_source(e)
                    })
                }
                None => Ok(None),
            }
        })
    }

    fn execute_insert<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<T>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(data = %sql, "Executing insert");

            let doc: Document = if sql.starts_with('{') {
                serde_json::from_str(&sql)
                    .map_err(|e| prax_query::QueryError::database(e.to_string()))?
            } else {
                let mut doc = Document::new();
                for (i, param) in params.iter().enumerate() {
                    let bson_value = filter_value_to_bson(param)
                        .map_err(|e| prax_query::QueryError::database(e.to_string()))?;
                    doc.insert(format!("field{}", i), bson_value);
                }
                doc
            };

            let collection = self
                .client
                .collection_doc(&format!("{}s", T::MODEL_NAME.to_lowercase()));

            let result = collection
                .insert_one(doc.clone(), None)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            // Re-fetch the inserted document keyed on the server-assigned
            // `_id` (or on the client-supplied `_id` if the caller set
            // one) so the return value is the actual persisted row,
            // including server-generated fields.
            let id_filter = bson::doc! { "_id": result.inserted_id };
            let inserted = collection
                .find_one(id_filter, None)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?
                .ok_or_else(|| prax_query::QueryError::not_found(T::MODEL_NAME))?;

            let row = crate::row_ref::BsonRowRef::new(&inserted);
            T::from_row(&row).map_err(|e| {
                let msg = e.to_string();
                prax_query::QueryError::deserialization(msg).with_source(e)
            })
        })
    }

    fn execute_update<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(data = %sql, "Executing update");

            let filter = Self::build_filter(&sql, &params)
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;
            let set_doc = build_set_doc(&sql, &params)
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            let collection = self
                .client
                .collection_doc(&format!("{}s", T::MODEL_NAME.to_lowercase()));

            // Mongo's `update_many` doesn't hand back the affected
            // documents, so we have to re-fetch. The original filter
            // still selects the same rows post-update (the SET can't
            // un-match them for the filters the Client API emits —
            // we set columns, not the filter columns). One update +
            // one find instead of three round-trips.
            if !set_doc.is_empty() {
                let update = doc! { "$set": set_doc };
                collection
                    .update_many(filter.clone(), update, None)
                    .await
                    .map_err(|e| prax_query::QueryError::database(e.to_string()))?;
            }

            let cursor = collection
                .find(filter, None)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;
            let updated: Vec<Document> = cursor
                .try_collect()
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            updated
                .iter()
                .map(|d| {
                    let row = crate::row_ref::BsonRowRef::new(d);
                    T::from_row(&row).map_err(|e| {
                        let msg = e.to_string();
                        prax_query::QueryError::deserialization(msg).with_source(e)
                    })
                })
                .collect()
        })
    }

    fn execute_delete(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<u64>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(filter = %sql, "Executing delete");

            let filter = Self::build_filter(&sql, &params)
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            // Get collection name from somewhere - would need model info
            let collection = self.client.collection_doc("documents");

            let result = collection
                .delete_many(filter, None)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            Ok(result.deleted_count)
        })
    }

    fn execute_raw(&self, sql: &str, _params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(command = %sql, "Executing raw command");

            // For MongoDB, raw execution means running a database command
            let command: Document = serde_json::from_str(&sql)
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            let _result = self
                .client
                .run_command(command)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            Ok(1)
        })
    }

    fn count(&self, sql: &str, params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(filter = %sql, "Executing count");

            let filter = Self::build_filter(&sql, &params)
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            // Get collection name - would need to parse from sql or context
            let collection = self.client.collection_doc("documents");

            let count = collection
                .count_documents(filter, None)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            Ok(count)
        })
    }
}

/// A typed query builder that uses the MongoDB engine.
pub struct MongoQueryBuilder<T: Model> {
    engine: MongoEngine,
    _marker: PhantomData<T>,
}

impl<T: Model> MongoQueryBuilder<T> {
    /// Create a new query builder.
    pub fn new(engine: MongoEngine) -> Self {
        Self {
            engine,
            _marker: PhantomData,
        }
    }

    /// Get the underlying engine.
    pub fn engine(&self) -> &MongoEngine {
        &self.engine
    }

    /// Get a typed collection for this model.
    pub fn collection(&self) -> Collection<T>
    where
        T: Send + Sync,
    {
        self.engine.collection::<T>()
    }
}

/// Parse `SET col1 = $1, col2 = $2` from a SQL-ish UPDATE statement
/// and bind each placeholder to the matching entry in `params`. Returns
/// a BSON `$set` document suitable for [`update_many`]. Tolerant of
/// an absent SET clause (returns empty doc, caller treats that as a
/// filter-only "update nothing" no-op).
fn build_set_doc(sql: &str, params: &[FilterValue]) -> MongoResult<Document> {
    // Locate the SET … WHERE window in the SQL string.
    let lower = sql.to_ascii_lowercase();
    let Some(set_start) = lower.find(" set ") else {
        return Ok(Document::new());
    };
    let set_body_start = set_start + " set ".len();
    let set_body_end = lower[set_body_start..]
        .find(" where ")
        .map(|i| set_body_start + i)
        .unwrap_or(sql.len());
    let body = &sql[set_body_start..set_body_end];

    let mut out = Document::new();
    for assignment in body.split(',') {
        let assignment = assignment.trim();
        let Some(eq) = assignment.find('=') else {
            continue;
        };
        let col = assignment[..eq].trim();
        let rhs = assignment[eq + 1..].trim();
        let Some(idx_str) = rhs.strip_prefix('$') else {
            continue;
        };
        let Ok(idx) = idx_str.parse::<usize>() else {
            continue;
        };
        if idx == 0 || idx > params.len() {
            continue;
        }
        let value = filter_value_to_bson(&params[idx - 1])?;
        out.insert(col, value);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_filter_json() {
        let filter = MongoEngine::build_filter(r#"{"name": "Alice"}"#, &[]).unwrap();
        assert_eq!(filter.get_str("name").unwrap(), "Alice");
    }

    #[test]
    fn test_build_filter_empty() {
        let filter = MongoEngine::build_filter("", &[]).unwrap();
        assert!(filter.is_empty());
    }
}
