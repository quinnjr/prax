//! MongoDB view support using aggregation pipelines.
//!
//! MongoDB views are read-only collections backed by aggregation pipelines.
//! This module provides:
//! - View creation and management
//! - Type-safe aggregation pipeline building
//! - View querying with the same API as collections
//!
//! # Example
//!
//! ```rust,ignore
//! use prax_mongodb::{MongoClient, view::AggregationView};
//! use bson::doc;
//!
//! // Create a view definition
//! let view = AggregationView::builder("active_users")
//!     .source_collection("users")
//!     .pipeline(vec![
//!         doc! { "$match": { "status": "active" } },
//!         doc! { "$project": { "name": 1, "email": 1, "created_at": 1 } },
//!     ])
//!     .build();
//!
//! // Create the view in the database
//! client.create_view(&view).await?;
//!
//! // Query the view like a regular collection
//! let users = client.view_collection::<User>("active_users")
//!     .find(doc! { "created_at": { "$gt": last_week } }, None)
//!     .await?;
//! ```

use std::time::Duration;

use bson::{Bson, Document, doc};
use serde::{Deserialize, Serialize};

use crate::client::MongoClient;
use crate::error::{MongoError, MongoResult};

/// An aggregation view definition.
///
/// MongoDB views are virtual collections defined by an aggregation pipeline
/// that runs against a source collection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregationView {
    /// The name of the view.
    pub name: String,
    /// The source collection this view is based on.
    pub source_collection: String,
    /// The aggregation pipeline that defines the view.
    pub pipeline: Vec<Document>,
    /// Optional collation for string comparisons.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collation: Option<Document>,
}

impl AggregationView {
    /// Create a new aggregation view.
    pub fn new(
        name: impl Into<String>,
        source_collection: impl Into<String>,
        pipeline: Vec<Document>,
    ) -> Self {
        Self {
            name: name.into(),
            source_collection: source_collection.into(),
            pipeline,
            collation: None,
        }
    }

    /// Create a builder for an aggregation view.
    pub fn builder(name: impl Into<String>) -> AggregationViewBuilder {
        AggregationViewBuilder::new(name)
    }

    /// Set the collation for string comparisons.
    pub fn with_collation(mut self, collation: Document) -> Self {
        self.collation = Some(collation);
        self
    }

    /// Generate the MongoDB command to create this view.
    pub fn to_create_command(&self, _database: &str) -> Document {
        let mut cmd = doc! {
            "create": &self.name,
            "viewOn": &self.source_collection,
            "pipeline": self.pipeline.iter().cloned().map(Bson::Document).collect::<Vec<_>>(),
        };

        if let Some(ref collation) = self.collation {
            cmd.insert("collation", collation.clone());
        }

        cmd
    }
}

/// Builder for creating aggregation views.
#[derive(Debug, Default)]
pub struct AggregationViewBuilder {
    name: String,
    source_collection: Option<String>,
    pipeline: Vec<Document>,
    collation: Option<Document>,
}

impl AggregationViewBuilder {
    /// Create a new builder with the given view name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    /// Set the source collection.
    pub fn source_collection(mut self, collection: impl Into<String>) -> Self {
        self.source_collection = Some(collection.into());
        self
    }

    /// Set the aggregation pipeline.
    pub fn pipeline(mut self, pipeline: Vec<Document>) -> Self {
        self.pipeline = pipeline;
        self
    }

    /// Add a stage to the pipeline.
    pub fn add_stage(mut self, stage: Document) -> Self {
        self.pipeline.push(stage);
        self
    }

    /// Add a $match stage.
    pub fn match_stage(mut self, filter: Document) -> Self {
        self.pipeline.push(doc! { "$match": filter });
        self
    }

    /// Add a $project stage.
    pub fn project_stage(mut self, projection: Document) -> Self {
        self.pipeline.push(doc! { "$project": projection });
        self
    }

    /// Add a $group stage.
    pub fn group_stage(mut self, group: Document) -> Self {
        self.pipeline.push(doc! { "$group": group });
        self
    }

    /// Add a $sort stage.
    pub fn sort_stage(mut self, sort: Document) -> Self {
        self.pipeline.push(doc! { "$sort": sort });
        self
    }

    /// Add a $limit stage.
    pub fn limit_stage(mut self, limit: i64) -> Self {
        self.pipeline.push(doc! { "$limit": limit });
        self
    }

    /// Add a $skip stage.
    pub fn skip_stage(mut self, skip: i64) -> Self {
        self.pipeline.push(doc! { "$skip": skip });
        self
    }

    /// Add a $lookup stage (join).
    pub fn lookup_stage(
        mut self,
        from: impl Into<String>,
        local_field: impl Into<String>,
        foreign_field: impl Into<String>,
        as_field: impl Into<String>,
    ) -> Self {
        self.pipeline.push(doc! {
            "$lookup": {
                "from": from.into(),
                "localField": local_field.into(),
                "foreignField": foreign_field.into(),
                "as": as_field.into(),
            }
        });
        self
    }

    /// Add an $unwind stage.
    pub fn unwind_stage(mut self, path: impl Into<String>) -> Self {
        self.pipeline.push(doc! { "$unwind": path.into() });
        self
    }

    /// Add a $count stage.
    pub fn count_stage(mut self, field: impl Into<String>) -> Self {
        self.pipeline.push(doc! { "$count": field.into() });
        self
    }

    /// Add a $addFields stage.
    pub fn add_fields_stage(mut self, fields: Document) -> Self {
        self.pipeline.push(doc! { "$addFields": fields });
        self
    }

    /// Set collation for string comparisons.
    pub fn collation(mut self, collation: Document) -> Self {
        self.collation = Some(collation);
        self
    }

    /// Build the aggregation view.
    pub fn build(self) -> AggregationView {
        AggregationView {
            name: self.name,
            source_collection: self.source_collection.unwrap_or_default(),
            pipeline: self.pipeline,
            collation: self.collation,
        }
    }
}

/// A materialized view using $merge or $out.
///
/// Unlike regular views, materialized views persist their results
/// and need to be refreshed periodically.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterializedAggregationView {
    /// The name of the output collection.
    pub name: String,
    /// The source collection.
    pub source_collection: String,
    /// The aggregation pipeline.
    pub pipeline: Vec<Document>,
    /// Whether to use $merge (update) or $out (replace).
    pub use_merge: bool,
    /// Merge options when using $merge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_options: Option<MergeOptions>,
    /// Refresh interval (for application-level scheduling).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_interval: Option<Duration>,
}

/// Options for $merge stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeOptions {
    /// Field(s) to use for matching.
    pub on: Vec<String>,
    /// Action when document matches.
    pub when_matched: MergeAction,
    /// Action when document doesn't match.
    pub when_not_matched: MergeNotMatchedAction,
}

/// Action when a document matches during $merge.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MergeAction {
    /// Replace the existing document.
    Replace,
    /// Keep the existing document.
    KeepExisting,
    /// Merge fields into the existing document.
    Merge,
    /// Fail the operation.
    Fail,
    /// Use a custom pipeline.
    Pipeline(Vec<Document>),
}

/// Action when a document doesn't match during $merge.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MergeNotMatchedAction {
    /// Insert the new document.
    Insert,
    /// Discard the document.
    Discard,
    /// Fail the operation.
    Fail,
}

impl MaterializedAggregationView {
    /// Create a new materialized view using $out (full replacement).
    pub fn with_out(
        name: impl Into<String>,
        source_collection: impl Into<String>,
        pipeline: Vec<Document>,
    ) -> Self {
        Self {
            name: name.into(),
            source_collection: source_collection.into(),
            pipeline,
            use_merge: false,
            merge_options: None,
            refresh_interval: None,
        }
    }

    /// Create a new materialized view using $merge (incremental update).
    pub fn with_merge(
        name: impl Into<String>,
        source_collection: impl Into<String>,
        pipeline: Vec<Document>,
        merge_options: MergeOptions,
    ) -> Self {
        Self {
            name: name.into(),
            source_collection: source_collection.into(),
            pipeline,
            use_merge: true,
            merge_options: Some(merge_options),
            refresh_interval: None,
        }
    }

    /// Set the refresh interval.
    pub fn with_refresh_interval(mut self, interval: Duration) -> Self {
        self.refresh_interval = Some(interval);
        self
    }

    /// Generate the aggregation pipeline with $out or $merge stage.
    pub fn to_pipeline(&self) -> Vec<Document> {
        let mut pipeline = self.pipeline.clone();

        if self.use_merge {
            let merge_opts = self.merge_options.as_ref().unwrap();
            let when_matched = match &merge_opts.when_matched {
                MergeAction::Replace => Bson::String("replace".to_string()),
                MergeAction::KeepExisting => Bson::String("keepExisting".to_string()),
                MergeAction::Merge => Bson::String("merge".to_string()),
                MergeAction::Fail => Bson::String("fail".to_string()),
                MergeAction::Pipeline(p) => {
                    Bson::Array(p.iter().cloned().map(Bson::Document).collect())
                }
            };

            let when_not_matched = match merge_opts.when_not_matched {
                MergeNotMatchedAction::Insert => "insert",
                MergeNotMatchedAction::Discard => "discard",
                MergeNotMatchedAction::Fail => "fail",
            };

            pipeline.push(doc! {
                "$merge": {
                    "into": &self.name,
                    "on": &merge_opts.on,
                    "whenMatched": when_matched,
                    "whenNotMatched": when_not_matched,
                }
            });
        } else {
            pipeline.push(doc! { "$out": &self.name });
        }

        pipeline
    }
}

impl MongoClient {
    /// Create a view in the database.
    pub async fn create_view(&self, view: &AggregationView) -> MongoResult<()> {
        let cmd = view.to_create_command(&self.config().database);
        self.run_command(cmd).await?;
        Ok(())
    }

    /// Drop a view from the database.
    pub async fn drop_view(&self, name: &str) -> MongoResult<()> {
        self.drop_collection(name).await
    }

    /// List all views in the database.
    pub async fn list_views(&self) -> MongoResult<Vec<String>> {
        let result = self
            .run_command(doc! {
                "listCollections": 1,
                "filter": { "type": "view" }
            })
            .await?;

        let cursor = result
            .get_document("cursor")
            .map_err(|e| MongoError::query(format!("invalid response: {}", e)))?;

        let first_batch = cursor
            .get_array("firstBatch")
            .map_err(|e| MongoError::query(format!("invalid response: {}", e)))?;

        let views = first_batch
            .iter()
            .filter_map(|doc| {
                doc.as_document()
                    .and_then(|d| d.get_str("name").ok())
                    .map(String::from)
            })
            .collect();

        Ok(views)
    }

    /// Get the definition of a view.
    pub async fn get_view_definition(&self, name: &str) -> MongoResult<Option<AggregationView>> {
        let result = self
            .run_command(doc! {
                "listCollections": 1,
                "filter": { "name": name, "type": "view" }
            })
            .await?;

        let cursor = result
            .get_document("cursor")
            .map_err(|e| MongoError::query(format!("invalid response: {}", e)))?;

        let first_batch = cursor
            .get_array("firstBatch")
            .map_err(|e| MongoError::query(format!("invalid response: {}", e)))?;

        if first_batch.is_empty() {
            return Ok(None);
        }

        let doc = first_batch[0]
            .as_document()
            .ok_or_else(|| MongoError::query("invalid view definition"))?;

        let options = doc
            .get_document("options")
            .map_err(|e| MongoError::query(format!("missing options: {}", e)))?;

        let view_on = options
            .get_str("viewOn")
            .map_err(|e| MongoError::query(format!("missing viewOn: {}", e)))?;

        let pipeline = options
            .get_array("pipeline")
            .map_err(|e| MongoError::query(format!("missing pipeline: {}", e)))?
            .iter()
            .filter_map(|b| b.as_document().cloned())
            .collect();

        Ok(Some(AggregationView {
            name: name.to_string(),
            source_collection: view_on.to_string(),
            pipeline,
            collation: options.get_document("collation").ok().cloned(),
        }))
    }

    /// Refresh a materialized view.
    ///
    /// This runs the aggregation pipeline and outputs results to the target collection.
    pub async fn refresh_materialized_view(
        &self,
        view: &MaterializedAggregationView,
    ) -> MongoResult<u64> {
        use futures::TryStreamExt;

        let collection = self.collection_doc(&view.source_collection);
        let pipeline = view.to_pipeline();

        let cursor = collection
            .aggregate(pipeline, None)
            .await
            .map_err(MongoError::from)?;

        // Drain the cursor to execute the pipeline
        let docs: Vec<Document> = cursor.try_collect().await.map_err(MongoError::from)?;

        Ok(docs.len() as u64)
    }
}

/// Helper functions for common aggregation stages.
pub mod stages {
    use bson::{Bson, Document, doc};

    /// Create a $match stage.
    pub fn match_stage(filter: Document) -> Document {
        doc! { "$match": filter }
    }

    /// Create a $project stage.
    pub fn project(fields: Document) -> Document {
        doc! { "$project": fields }
    }

    /// Create a $group stage.
    pub fn group(id: impl Into<Bson>, accumulators: Document) -> Document {
        let mut group_doc = doc! { "_id": id.into() };
        group_doc.extend(accumulators);
        doc! { "$group": group_doc }
    }

    /// Create a $sort stage.
    pub fn sort(fields: Document) -> Document {
        doc! { "$sort": fields }
    }

    /// Create a $limit stage.
    pub fn limit(n: i64) -> Document {
        doc! { "$limit": n }
    }

    /// Create a $skip stage.
    pub fn skip(n: i64) -> Document {
        doc! { "$skip": n }
    }

    /// Create a $lookup stage (left outer join).
    pub fn lookup(
        from: impl Into<String>,
        local_field: impl Into<String>,
        foreign_field: impl Into<String>,
        as_field: impl Into<String>,
    ) -> Document {
        doc! {
            "$lookup": {
                "from": from.into(),
                "localField": local_field.into(),
                "foreignField": foreign_field.into(),
                "as": as_field.into(),
            }
        }
    }

    /// Create an $unwind stage.
    pub fn unwind(path: impl Into<String>) -> Document {
        doc! { "$unwind": path.into() }
    }

    /// Create an $unwind stage with options.
    pub fn unwind_with_options(
        path: impl Into<String>,
        preserve_null: bool,
        include_array_index: Option<&str>,
    ) -> Document {
        let mut unwind_doc = doc! { "path": path.into() };
        unwind_doc.insert("preserveNullAndEmptyArrays", preserve_null);
        if let Some(index_field) = include_array_index {
            unwind_doc.insert("includeArrayIndex", index_field);
        }
        doc! { "$unwind": unwind_doc }
    }

    /// Create a $count stage.
    pub fn count(field: impl Into<String>) -> Document {
        doc! { "$count": field.into() }
    }

    /// Create an $addFields stage.
    pub fn add_fields(fields: Document) -> Document {
        doc! { "$addFields": fields }
    }

    /// Create a $set stage (alias for $addFields).
    pub fn set(fields: Document) -> Document {
        doc! { "$set": fields }
    }

    /// Create an $unset stage.
    pub fn unset(fields: Vec<&str>) -> Document {
        if fields.len() == 1 {
            doc! { "$unset": fields[0] }
        } else {
            doc! { "$unset": fields }
        }
    }

    /// Create a $replaceRoot stage.
    pub fn replace_root(new_root: impl Into<Bson>) -> Document {
        doc! { "$replaceRoot": { "newRoot": new_root.into() } }
    }

    /// Create a $facet stage.
    pub fn facet(facets: Document) -> Document {
        doc! { "$facet": facets }
    }

    /// Create a $bucket stage.
    pub fn bucket(
        group_by: impl Into<Bson>,
        boundaries: Vec<impl Into<Bson>>,
        default_bucket: impl Into<Bson>,
        output: Document,
    ) -> Document {
        doc! {
            "$bucket": {
                "groupBy": group_by.into(),
                "boundaries": boundaries.into_iter().map(|b| b.into()).collect::<Vec<_>>(),
                "default": default_bucket.into(),
                "output": output,
            }
        }
    }

    /// Create a $bucketAuto stage.
    pub fn bucket_auto(group_by: impl Into<Bson>, buckets: i32, output: Document) -> Document {
        doc! {
            "$bucketAuto": {
                "groupBy": group_by.into(),
                "buckets": buckets,
                "output": output,
            }
        }
    }

    /// Create a $sample stage.
    pub fn sample(size: i64) -> Document {
        doc! { "$sample": { "size": size } }
    }
}

/// Aggregation accumulators for use in $group stages.
pub mod accumulators {
    use bson::{Bson, doc};

    /// Sum accumulator.
    pub fn sum(expr: impl Into<Bson>) -> Bson {
        Bson::Document(doc! { "$sum": expr.into() })
    }

    /// Average accumulator.
    pub fn avg(expr: impl Into<Bson>) -> Bson {
        Bson::Document(doc! { "$avg": expr.into() })
    }

    /// Minimum accumulator.
    pub fn min(expr: impl Into<Bson>) -> Bson {
        Bson::Document(doc! { "$min": expr.into() })
    }

    /// Maximum accumulator.
    pub fn max(expr: impl Into<Bson>) -> Bson {
        Bson::Document(doc! { "$max": expr.into() })
    }

    /// First accumulator.
    pub fn first(expr: impl Into<Bson>) -> Bson {
        Bson::Document(doc! { "$first": expr.into() })
    }

    /// Last accumulator.
    pub fn last(expr: impl Into<Bson>) -> Bson {
        Bson::Document(doc! { "$last": expr.into() })
    }

    /// Push accumulator (creates array).
    pub fn push(expr: impl Into<Bson>) -> Bson {
        Bson::Document(doc! { "$push": expr.into() })
    }

    /// AddToSet accumulator (creates unique array).
    pub fn add_to_set(expr: impl Into<Bson>) -> Bson {
        Bson::Document(doc! { "$addToSet": expr.into() })
    }

    /// Count accumulator.
    pub fn count() -> Bson {
        Bson::Document(doc! { "$sum": 1 })
    }

    /// Standard deviation (population) accumulator.
    pub fn std_dev_pop(expr: impl Into<Bson>) -> Bson {
        Bson::Document(doc! { "$stdDevPop": expr.into() })
    }

    /// Standard deviation (sample) accumulator.
    pub fn std_dev_samp(expr: impl Into<Bson>) -> Bson {
        Bson::Document(doc! { "$stdDevSamp": expr.into() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aggregation_view_builder() {
        let view = AggregationView::builder("active_users")
            .source_collection("users")
            .match_stage(doc! { "status": "active" })
            .project_stage(doc! { "name": 1, "email": 1 })
            .build();

        assert_eq!(view.name, "active_users");
        assert_eq!(view.source_collection, "users");
        assert_eq!(view.pipeline.len(), 2);
    }

    #[test]
    fn test_view_create_command() {
        let view = AggregationView::new(
            "test_view",
            "source_col",
            vec![doc! { "$match": { "active": true } }],
        );

        let cmd = view.to_create_command("testdb");
        assert_eq!(cmd.get_str("create").unwrap(), "test_view");
        assert_eq!(cmd.get_str("viewOn").unwrap(), "source_col");
    }

    #[test]
    fn test_materialized_view_out() {
        let view = MaterializedAggregationView::with_out(
            "user_stats",
            "users",
            vec![
                doc! { "$match": { "status": "active" } },
                doc! { "$group": { "_id": "$department", "count": { "$sum": 1 } } },
            ],
        );

        let pipeline = view.to_pipeline();
        assert_eq!(pipeline.len(), 3);
        assert!(pipeline.last().unwrap().contains_key("$out"));
    }

    #[test]
    fn test_materialized_view_merge() {
        let view = MaterializedAggregationView::with_merge(
            "user_stats",
            "users",
            vec![doc! { "$group": { "_id": "$department", "count": { "$sum": 1 } } }],
            MergeOptions {
                on: vec!["_id".to_string()],
                when_matched: MergeAction::Replace,
                when_not_matched: MergeNotMatchedAction::Insert,
            },
        );

        let pipeline = view.to_pipeline();
        assert_eq!(pipeline.len(), 2);
        assert!(pipeline.last().unwrap().contains_key("$merge"));
    }

    #[test]
    fn test_stages_helpers() {
        let match_doc = stages::match_stage(doc! { "status": "active" });
        assert!(match_doc.contains_key("$match"));

        let group_doc = stages::group("$department", doc! { "count": accumulators::count() });
        assert!(group_doc.contains_key("$group"));

        let lookup_doc = stages::lookup("orders", "user_id", "_id", "user_orders");
        assert!(lookup_doc.contains_key("$lookup"));
    }

    #[test]
    fn test_accumulators() {
        let sum = accumulators::sum("$amount");
        assert!(sum.as_document().unwrap().contains_key("$sum"));

        let avg = accumulators::avg("$price");
        assert!(avg.as_document().unwrap().contains_key("$avg"));

        let count = accumulators::count();
        assert_eq!(count.as_document().unwrap().get_i32("$sum").unwrap(), 1);
    }
}
