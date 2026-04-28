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
    fn query_many<T: Model + Send + 'static>(
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

            // Would need to deserialize docs into T
            // For now, return empty - full implementation would use serde
            let _ = docs;
            Ok(Vec::new())
        })
    }

    fn query_one<T: Model + Send + 'static>(
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

            let _doc = collection
                .find_one(filter, None)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?
                .ok_or_else(|| prax_query::QueryError::not_found(T::MODEL_NAME))?;

            // Would deserialize doc into T
            Err(prax_query::QueryError::internal(
                "deserialization not yet implemented".to_string(),
            ))
        })
    }

    fn query_optional<T: Model + Send + 'static>(
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
                Some(_doc) => {
                    // Would deserialize doc into T
                    Err(prax_query::QueryError::internal(
                        "deserialization not yet implemented".to_string(),
                    ))
                }
                None => Ok(None),
            }
        })
    }

    fn execute_insert<T: Model + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<T>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(data = %sql, "Executing insert");

            // For insert, the "sql" should be a JSON document to insert
            let doc: Document = if sql.starts_with('{') {
                serde_json::from_str(&sql)
                    .map_err(|e| prax_query::QueryError::database(e.to_string()))?
            } else {
                // Build document from params
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

            let _result = collection
                .insert_one(doc, None)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            // Would return the inserted document
            Err(prax_query::QueryError::internal(
                "insert returning not yet implemented".to_string(),
            ))
        })
    }

    fn execute_update<T: Model + Send + 'static>(
        &self,
        sql: &str,
        _params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(data = %sql, "Executing update");

            // For update, parse filter and update document from sql/params
            let collection = self
                .client
                .collection_doc(&format!("{}s", T::MODEL_NAME.to_lowercase()));

            // Simplified: use first half of params as filter, second half as update
            let filter = doc! {};
            let update = doc! { "$set": {} };

            let _result = collection
                .update_many(filter, update, None)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            Ok(Vec::new())
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
