//! Aggregation query operations.
//!
//! This module provides aggregate operations like `count`, `sum`, `avg`, `min`, `max`,
//! and `groupBy` for performing statistical queries on the database.
//!
//! # Example
//!
//! ```rust,ignore
//! // Count users
//! let count = client
//!     .user()
//!     .aggregate()
//!     .count()
//!     .r#where(user::active::equals(true))
//!     .exec()
//!     .await?;
//!
//! // Get aggregated statistics
//! let stats = client
//!     .user()
//!     .aggregate()
//!     .count()
//!     .avg(user::age())
//!     .min(user::age())
//!     .max(user::age())
//!     .sum(user::age())
//!     .r#where(user::active::equals(true))
//!     .exec()
//!     .await?;
//!
//! // Group by with aggregation
//! let by_country = client
//!     .user()
//!     .group_by(user::country())
//!     .count()
//!     .avg(user::age())
//!     .having(aggregate::count::gt(10))
//!     .exec()
//!     .await?;
//! ```

use std::marker::PhantomData;

use crate::error::QueryResult;
use crate::filter::Filter;
use crate::sql::quote_identifier;
use crate::traits::{Model, QueryEngine};
use crate::types::OrderByField;

/// An aggregation field specifier.
#[derive(Debug, Clone)]
pub enum AggregateField {
    /// Count all rows.
    CountAll,
    /// Count non-null values in a column.
    CountColumn(String),
    /// Count distinct values in a column.
    CountDistinct(String),
    /// Sum of a numeric column.
    Sum(String),
    /// Average of a numeric column.
    Avg(String),
    /// Minimum value in a column.
    Min(String),
    /// Maximum value in a column.
    Max(String),
}

impl AggregateField {
    /// Build the SQL expression for this aggregate.
    pub fn to_sql(&self) -> String {
        match self {
            Self::CountAll => "COUNT(*)".to_string(),
            Self::CountColumn(col) => format!("COUNT({})", quote_identifier(col)),
            Self::CountDistinct(col) => format!("COUNT(DISTINCT {})", quote_identifier(col)),
            Self::Sum(col) => format!("SUM({})", quote_identifier(col)),
            Self::Avg(col) => format!("AVG({})", quote_identifier(col)),
            Self::Min(col) => format!("MIN({})", quote_identifier(col)),
            Self::Max(col) => format!("MAX({})", quote_identifier(col)),
        }
    }

    /// Get the alias for this aggregate.
    pub fn alias(&self) -> String {
        match self {
            Self::CountAll => "_count".to_string(),
            Self::CountColumn(col) => format!("_count_{}", col),
            Self::CountDistinct(col) => format!("_count_distinct_{}", col),
            Self::Sum(col) => format!("_sum_{}", col),
            Self::Avg(col) => format!("_avg_{}", col),
            Self::Min(col) => format!("_min_{}", col),
            Self::Max(col) => format!("_max_{}", col),
        }
    }
}

/// Result of an aggregation query.
#[derive(Debug, Clone, Default)]
pub struct AggregateResult {
    /// Total count (if requested).
    pub count: Option<i64>,
    /// Sum results keyed by column name.
    pub sum: std::collections::HashMap<String, f64>,
    /// Average results keyed by column name.
    pub avg: std::collections::HashMap<String, f64>,
    /// Minimum results keyed by column name.
    pub min: std::collections::HashMap<String, serde_json::Value>,
    /// Maximum results keyed by column name.
    pub max: std::collections::HashMap<String, serde_json::Value>,
}

/// Aggregate operation builder.
#[derive(Debug)]
pub struct AggregateOperation<M: Model, E: QueryEngine> {
    /// Phantom data for model type.
    _model: PhantomData<M>,
    /// Phantom data for engine type.
    _engine: PhantomData<E>,
    /// Aggregate fields to compute.
    fields: Vec<AggregateField>,
    /// Filter conditions.
    filter: Option<Filter>,
}

impl<M: Model, E: QueryEngine> AggregateOperation<M, E> {
    /// Create a new aggregate operation.
    pub fn new() -> Self {
        Self {
            _model: PhantomData,
            _engine: PhantomData,
            fields: Vec::new(),
            filter: None,
        }
    }

    /// Add a count of all rows.
    pub fn count(mut self) -> Self {
        self.fields.push(AggregateField::CountAll);
        self
    }

    /// Add a count of non-null values in a column.
    pub fn count_column(mut self, column: impl Into<String>) -> Self {
        self.fields.push(AggregateField::CountColumn(column.into()));
        self
    }

    /// Add a count of distinct values in a column.
    pub fn count_distinct(mut self, column: impl Into<String>) -> Self {
        self.fields
            .push(AggregateField::CountDistinct(column.into()));
        self
    }

    /// Add sum of a numeric column.
    pub fn sum(mut self, column: impl Into<String>) -> Self {
        self.fields.push(AggregateField::Sum(column.into()));
        self
    }

    /// Add average of a numeric column.
    pub fn avg(mut self, column: impl Into<String>) -> Self {
        self.fields.push(AggregateField::Avg(column.into()));
        self
    }

    /// Add minimum of a column.
    pub fn min(mut self, column: impl Into<String>) -> Self {
        self.fields.push(AggregateField::Min(column.into()));
        self
    }

    /// Add maximum of a column.
    pub fn max(mut self, column: impl Into<String>) -> Self {
        self.fields.push(AggregateField::Max(column.into()));
        self
    }

    /// Add a filter condition.
    pub fn r#where(mut self, filter: impl Into<Filter>) -> Self {
        self.filter = Some(filter.into());
        self
    }

    /// Build the SQL for this operation.
    pub fn build_sql(&self) -> (String, Vec<crate::filter::FilterValue>) {
        let mut params = Vec::new();

        // If no fields specified, default to count
        let fields = if self.fields.is_empty() {
            vec![AggregateField::CountAll]
        } else {
            self.fields.clone()
        };

        let select_parts: Vec<String> = fields
            .iter()
            .map(|f| format!("{} AS {}", f.to_sql(), quote_identifier(&f.alias())))
            .collect();

        let mut sql = format!(
            "SELECT {} FROM {}",
            select_parts.join(", "),
            quote_identifier(M::TABLE_NAME)
        );

        // Add WHERE clause
        if let Some(filter) = &self.filter {
            let (where_sql, where_params) = filter.to_sql(params.len() + 1);
            sql.push_str(&format!(" WHERE {}", where_sql));
            params.extend(where_params);
        }

        (sql, params)
    }

    /// Execute the aggregate operation.
    pub async fn exec(self, _engine: &E) -> QueryResult<AggregateResult> {
        let (_sql, _params) = self.build_sql();
        // In a real implementation, this would execute the query
        // For now, return a placeholder
        Ok(AggregateResult::default())
    }
}

impl<M: Model, E: QueryEngine> Default for AggregateOperation<M, E> {
    fn default() -> Self {
        Self::new()
    }
}

/// Group by operation builder.
#[derive(Debug)]
pub struct GroupByOperation<M: Model, E: QueryEngine> {
    /// Phantom data for model type.
    _model: PhantomData<M>,
    /// Phantom data for engine type.
    _engine: PhantomData<E>,
    /// Columns to group by.
    group_columns: Vec<String>,
    /// Aggregate fields to compute.
    agg_fields: Vec<AggregateField>,
    /// Filter conditions (WHERE).
    filter: Option<Filter>,
    /// Having conditions.
    having: Option<HavingCondition>,
    /// Order by clauses.
    order_by: Vec<OrderByField>,
    /// Skip count.
    skip: Option<usize>,
    /// Take count.
    take: Option<usize>,
}

/// A condition for the HAVING clause.
#[derive(Debug, Clone)]
pub struct HavingCondition {
    /// The aggregate field to check.
    pub field: AggregateField,
    /// The comparison operator.
    pub op: HavingOp,
    /// The value to compare against.
    pub value: f64,
}

/// Operators for HAVING conditions.
#[derive(Debug, Clone, Copy)]
pub enum HavingOp {
    /// Greater than.
    Gt,
    /// Greater than or equal.
    Gte,
    /// Less than.
    Lt,
    /// Less than or equal.
    Lte,
    /// Equal.
    Eq,
    /// Not equal.
    Ne,
}

impl HavingOp {
    /// Get the SQL operator string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Gt => ">",
            Self::Gte => ">=",
            Self::Lt => "<",
            Self::Lte => "<=",
            Self::Eq => "=",
            Self::Ne => "<>",
        }
    }
}

impl<M: Model, E: QueryEngine> GroupByOperation<M, E> {
    /// Create a new group by operation.
    pub fn new(columns: Vec<String>) -> Self {
        Self {
            _model: PhantomData,
            _engine: PhantomData,
            group_columns: columns,
            agg_fields: Vec::new(),
            filter: None,
            having: None,
            order_by: Vec::new(),
            skip: None,
            take: None,
        }
    }

    /// Add a count aggregate.
    pub fn count(mut self) -> Self {
        self.agg_fields.push(AggregateField::CountAll);
        self
    }

    /// Add sum of a column.
    pub fn sum(mut self, column: impl Into<String>) -> Self {
        self.agg_fields.push(AggregateField::Sum(column.into()));
        self
    }

    /// Add average of a column.
    pub fn avg(mut self, column: impl Into<String>) -> Self {
        self.agg_fields.push(AggregateField::Avg(column.into()));
        self
    }

    /// Add minimum of a column.
    pub fn min(mut self, column: impl Into<String>) -> Self {
        self.agg_fields.push(AggregateField::Min(column.into()));
        self
    }

    /// Add maximum of a column.
    pub fn max(mut self, column: impl Into<String>) -> Self {
        self.agg_fields.push(AggregateField::Max(column.into()));
        self
    }

    /// Add a filter condition.
    pub fn r#where(mut self, filter: impl Into<Filter>) -> Self {
        self.filter = Some(filter.into());
        self
    }

    /// Add a having condition.
    pub fn having(mut self, condition: HavingCondition) -> Self {
        self.having = Some(condition);
        self
    }

    /// Add ordering.
    pub fn order_by(mut self, order: impl Into<OrderByField>) -> Self {
        self.order_by.push(order.into());
        self
    }

    /// Set skip count.
    pub fn skip(mut self, count: usize) -> Self {
        self.skip = Some(count);
        self
    }

    /// Set take count.
    pub fn take(mut self, count: usize) -> Self {
        self.take = Some(count);
        self
    }

    /// Build the SQL for this operation.
    pub fn build_sql(&self) -> (String, Vec<crate::filter::FilterValue>) {
        let mut params = Vec::new();

        // Build SELECT clause
        let mut select_parts: Vec<String> = self
            .group_columns
            .iter()
            .map(|c| quote_identifier(c))
            .collect();

        for field in &self.agg_fields {
            select_parts.push(format!(
                "{} AS {}",
                field.to_sql(),
                quote_identifier(&field.alias())
            ));
        }

        let mut sql = format!(
            "SELECT {} FROM {}",
            select_parts.join(", "),
            quote_identifier(M::TABLE_NAME)
        );

        // Add WHERE clause
        if let Some(filter) = &self.filter {
            let (where_sql, where_params) = filter.to_sql(params.len() + 1);
            sql.push_str(&format!(" WHERE {}", where_sql));
            params.extend(where_params);
        }

        // Add GROUP BY clause
        if !self.group_columns.is_empty() {
            let group_cols: Vec<String> = self
                .group_columns
                .iter()
                .map(|c| quote_identifier(c))
                .collect();
            sql.push_str(&format!(" GROUP BY {}", group_cols.join(", ")));
        }

        // Add HAVING clause
        if let Some(having) = &self.having {
            sql.push_str(&format!(
                " HAVING {} {} {}",
                having.field.to_sql(),
                having.op.as_str(),
                having.value
            ));
        }

        // Add ORDER BY clause
        if !self.order_by.is_empty() {
            let order_parts: Vec<String> = self
                .order_by
                .iter()
                .map(|o| {
                    let mut part = format!("{} {}", quote_identifier(&o.column), o.order.as_sql());
                    if let Some(nulls) = o.nulls {
                        part.push(' ');
                        part.push_str(nulls.as_sql());
                    }
                    part
                })
                .collect();
            sql.push_str(&format!(" ORDER BY {}", order_parts.join(", ")));
        }

        // Add LIMIT/OFFSET
        if let Some(take) = self.take {
            sql.push_str(&format!(" LIMIT {}", take));
        }
        if let Some(skip) = self.skip {
            sql.push_str(&format!(" OFFSET {}", skip));
        }

        (sql, params)
    }

    /// Execute the group by operation.
    pub async fn exec(self, _engine: &E) -> QueryResult<Vec<GroupByResult>> {
        let (_sql, _params) = self.build_sql();
        // In a real implementation, this would execute the query
        Ok(Vec::new())
    }
}

/// Result of a group by query.
#[derive(Debug, Clone)]
pub struct GroupByResult {
    /// The grouped column values.
    pub group_values: std::collections::HashMap<String, serde_json::Value>,
    /// The aggregate results.
    pub aggregates: AggregateResult,
}

/// Helper for creating having conditions.
pub mod having {
    use super::*;

    /// Create a having condition for count > value.
    pub fn count_gt(value: f64) -> HavingCondition {
        HavingCondition {
            field: AggregateField::CountAll,
            op: HavingOp::Gt,
            value,
        }
    }

    /// Create a having condition for count >= value.
    pub fn count_gte(value: f64) -> HavingCondition {
        HavingCondition {
            field: AggregateField::CountAll,
            op: HavingOp::Gte,
            value,
        }
    }

    /// Create a having condition for count < value.
    pub fn count_lt(value: f64) -> HavingCondition {
        HavingCondition {
            field: AggregateField::CountAll,
            op: HavingOp::Lt,
            value,
        }
    }

    /// Create a having condition for sum > value.
    pub fn sum_gt(column: impl Into<String>, value: f64) -> HavingCondition {
        HavingCondition {
            field: AggregateField::Sum(column.into()),
            op: HavingOp::Gt,
            value,
        }
    }

    /// Create a having condition for avg > value.
    pub fn avg_gt(column: impl Into<String>, value: f64) -> HavingCondition {
        HavingCondition {
            field: AggregateField::Avg(column.into()),
            op: HavingOp::Gt,
            value,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::{Filter, FilterValue};
    use crate::types::NullsOrder;

    // A simple test model
    struct TestModel;

    impl Model for TestModel {
        const MODEL_NAME: &'static str = "TestModel";
        const TABLE_NAME: &'static str = "test_models";
        const PRIMARY_KEY: &'static [&'static str] = &["id"];
        const COLUMNS: &'static [&'static str] = &["id", "name", "age", "score"];
    }

    impl crate::row::FromRow for TestModel {
        fn from_row(_row: &impl crate::row::RowRef) -> Result<Self, crate::row::RowError> {
            Ok(TestModel)
        }
    }

    // A mock query engine
    #[derive(Clone)]
    struct MockEngine;

    impl QueryEngine for MockEngine {
        fn dialect(&self) -> &dyn crate::dialect::SqlDialect {
            &crate::dialect::Postgres
        }

        fn query_many<T: Model + crate::row::FromRow + Send + 'static>(
            &self,
            _sql: &str,
            _params: Vec<crate::filter::FilterValue>,
        ) -> crate::traits::BoxFuture<'_, QueryResult<Vec<T>>> {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn query_one<T: Model + crate::row::FromRow + Send + 'static>(
            &self,
            _sql: &str,
            _params: Vec<crate::filter::FilterValue>,
        ) -> crate::traits::BoxFuture<'_, QueryResult<T>> {
            Box::pin(async { Err(crate::error::QueryError::not_found("Not implemented")) })
        }

        fn query_optional<T: Model + crate::row::FromRow + Send + 'static>(
            &self,
            _sql: &str,
            _params: Vec<crate::filter::FilterValue>,
        ) -> crate::traits::BoxFuture<'_, QueryResult<Option<T>>> {
            Box::pin(async { Ok(None) })
        }

        fn execute_insert<T: Model + crate::row::FromRow + Send + 'static>(
            &self,
            _sql: &str,
            _params: Vec<crate::filter::FilterValue>,
        ) -> crate::traits::BoxFuture<'_, QueryResult<T>> {
            Box::pin(async { Err(crate::error::QueryError::not_found("Not implemented")) })
        }

        fn execute_update<T: Model + crate::row::FromRow + Send + 'static>(
            &self,
            _sql: &str,
            _params: Vec<crate::filter::FilterValue>,
        ) -> crate::traits::BoxFuture<'_, QueryResult<Vec<T>>> {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn execute_delete(
            &self,
            _sql: &str,
            _params: Vec<crate::filter::FilterValue>,
        ) -> crate::traits::BoxFuture<'_, QueryResult<u64>> {
            Box::pin(async { Ok(0) })
        }

        fn execute_raw(
            &self,
            _sql: &str,
            _params: Vec<crate::filter::FilterValue>,
        ) -> crate::traits::BoxFuture<'_, QueryResult<u64>> {
            Box::pin(async { Ok(0) })
        }

        fn count(
            &self,
            _sql: &str,
            _params: Vec<crate::filter::FilterValue>,
        ) -> crate::traits::BoxFuture<'_, QueryResult<u64>> {
            Box::pin(async { Ok(0) })
        }
    }

    // ========== AggregateField Tests ==========

    #[test]
    fn test_aggregate_field_sql() {
        // Note: quote_identifier only quotes when needed (reserved words, special chars)
        assert_eq!(AggregateField::CountAll.to_sql(), "COUNT(*)");
        assert_eq!(
            AggregateField::CountColumn("id".into()).to_sql(),
            "COUNT(id)"
        );
        assert_eq!(
            AggregateField::CountDistinct("email".into()).to_sql(),
            "COUNT(DISTINCT email)"
        );
        assert_eq!(AggregateField::Sum("amount".into()).to_sql(), "SUM(amount)");
        assert_eq!(
            AggregateField::Avg("score".to_string()).to_sql(),
            "AVG(score)"
        );
        assert_eq!(AggregateField::Min("age".into()).to_sql(), "MIN(age)");
        assert_eq!(AggregateField::Max("age".into()).to_sql(), "MAX(age)");
        // Test with reserved word - should be quoted
        assert_eq!(
            AggregateField::CountColumn("user".to_string()).to_sql(),
            "COUNT(\"user\")"
        );
    }

    #[test]
    fn test_aggregate_field_alias() {
        assert_eq!(AggregateField::CountAll.alias(), "_count");
        assert_eq!(
            AggregateField::CountColumn("id".into()).alias(),
            "_count_id"
        );
        assert_eq!(
            AggregateField::CountDistinct("email".into()).alias(),
            "_count_distinct_email"
        );
        assert_eq!(AggregateField::Sum("amount".into()).alias(), "_sum_amount");
        assert_eq!(
            AggregateField::Avg("score".to_string()).alias(),
            "_avg_score"
        );
        assert_eq!(AggregateField::Min("age".into()).alias(), "_min_age");
        assert_eq!(
            AggregateField::Max("salary".to_string()).alias(),
            "_max_salary"
        );
    }

    // ========== AggregateResult Tests ==========

    #[test]
    fn test_aggregate_result_default() {
        let result = AggregateResult::default();
        assert!(result.count.is_none());
        assert!(result.sum.is_empty());
        assert!(result.avg.is_empty());
        assert!(result.min.is_empty());
        assert!(result.max.is_empty());
    }

    #[test]
    fn test_aggregate_result_debug() {
        let result = AggregateResult::default();
        let debug_str = format!("{:?}", result);
        assert!(debug_str.contains("AggregateResult"));
    }

    #[test]
    fn test_aggregate_result_clone() {
        let mut result = AggregateResult::default();
        result.count = Some(42);
        result.sum.insert("amount".into(), 1000.0);

        let cloned = result.clone();
        assert_eq!(cloned.count, Some(42));
        assert_eq!(cloned.sum.get("amount"), Some(&1000.0));
    }

    // ========== AggregateOperation Tests ==========

    #[test]
    fn test_aggregate_operation_new() {
        let op: AggregateOperation<TestModel, MockEngine> = AggregateOperation::new();
        let (sql, params) = op.build_sql();

        // Default should be count all
        assert!(sql.contains("COUNT(*)"));
        assert!(params.is_empty());
    }

    #[test]
    fn test_aggregate_operation_default() {
        let op: AggregateOperation<TestModel, MockEngine> = AggregateOperation::default();
        let (sql, params) = op.build_sql();

        assert!(sql.contains("COUNT(*)"));
        assert!(params.is_empty());
    }

    #[test]
    fn test_aggregate_operation_build_sql() {
        let op: AggregateOperation<TestModel, MockEngine> =
            AggregateOperation::new().count().sum("score").avg("age");

        let (sql, params) = op.build_sql();

        assert!(sql.contains("SELECT"));
        assert!(sql.contains("COUNT(*)"));
        assert!(sql.contains("SUM(score)"));
        assert!(sql.contains("AVG(age)"));
        assert!(sql.contains("FROM test_models"));
        assert!(params.is_empty());
    }

    #[test]
    fn test_aggregate_operation_count_column() {
        let op: AggregateOperation<TestModel, MockEngine> =
            AggregateOperation::new().count_column("email");

        let (sql, _) = op.build_sql();

        assert!(sql.contains("COUNT(email)"));
    }

    #[test]
    fn test_aggregate_operation_count_distinct() {
        let op: AggregateOperation<TestModel, MockEngine> =
            AggregateOperation::new().count_distinct("email");

        let (sql, _) = op.build_sql();

        assert!(sql.contains("COUNT(DISTINCT email)"));
    }

    #[test]
    fn test_aggregate_operation_min_max() {
        let op: AggregateOperation<TestModel, MockEngine> =
            AggregateOperation::new().min("age").max("age");

        let (sql, _) = op.build_sql();

        assert!(sql.contains("MIN(age)"));
        assert!(sql.contains("MAX(age)"));
    }

    #[test]
    fn test_aggregate_with_where() {
        let op: AggregateOperation<TestModel, MockEngine> = AggregateOperation::new()
            .count()
            .r#where(Filter::Gt("age".into(), FilterValue::Int(18)));

        let (sql, params) = op.build_sql();

        assert!(sql.contains("WHERE"));
        assert!(sql.contains("age")); // Not quoted since "age" is not a reserved word
        assert!(sql.contains(">"));
        assert!(!params.is_empty());
    }

    #[test]
    fn test_aggregate_with_complex_filter() {
        let op: AggregateOperation<TestModel, MockEngine> = AggregateOperation::new()
            .sum("score")
            .avg("age")
            .r#where(Filter::and([
                Filter::Gte("age".into(), FilterValue::Int(18)),
                Filter::Equals("active".into(), FilterValue::Bool(true)),
            ]));

        let (sql, params) = op.build_sql();

        assert!(sql.contains("WHERE"));
        assert!(sql.contains("AND"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_aggregate_all_methods() {
        let op: AggregateOperation<TestModel, MockEngine> = AggregateOperation::new()
            .count()
            .count_column("name")
            .count_distinct("email")
            .sum("score")
            .avg("score")
            .min("age")
            .max("age");

        let (sql, _) = op.build_sql();

        assert!(sql.contains("COUNT(*)"));
        assert!(sql.contains("COUNT(name)"));
        assert!(sql.contains("COUNT(DISTINCT email)"));
        assert!(sql.contains("SUM(score)"));
        assert!(sql.contains("AVG(score)"));
        assert!(sql.contains("MIN(age)"));
        assert!(sql.contains("MAX(age)"));
    }

    #[tokio::test]
    async fn test_aggregate_exec() {
        let engine = MockEngine;
        let op: AggregateOperation<TestModel, MockEngine> = AggregateOperation::new().count();

        let result = op.exec(&engine).await;
        assert!(result.is_ok());
    }

    // ========== GroupByOperation Tests ==========

    #[test]
    fn test_group_by_new() {
        let op: GroupByOperation<TestModel, MockEngine> =
            GroupByOperation::new(vec!["department".into()]);

        let (sql, _) = op.build_sql();

        assert!(sql.contains("GROUP BY department"));
    }

    #[test]
    fn test_group_by_build_sql() {
        let op: GroupByOperation<TestModel, MockEngine> =
            GroupByOperation::new(vec!["name".to_string()])
                .count()
                .avg("score");

        let (sql, params) = op.build_sql();

        assert!(sql.contains("SELECT"));
        assert!(sql.contains("name")); // Not quoted since "name" is not a reserved word
        assert!(sql.contains("COUNT(*)"));
        assert!(sql.contains("AVG(score)"));
        assert!(sql.contains("GROUP BY name"));
        assert!(params.is_empty());
    }

    #[test]
    fn test_group_by_multiple_columns() {
        let op: GroupByOperation<TestModel, MockEngine> =
            GroupByOperation::new(vec!["department".into(), "role".into()]).count();

        let (sql, _) = op.build_sql();

        assert!(sql.contains("GROUP BY department, role"));
    }

    #[test]
    fn test_group_by_with_sum() {
        let op: GroupByOperation<TestModel, MockEngine> =
            GroupByOperation::new(vec!["category".into()]).sum("amount");

        let (sql, _) = op.build_sql();

        assert!(sql.contains("SUM(amount)"));
    }

    #[test]
    fn test_group_by_with_min_max() {
        let op: GroupByOperation<TestModel, MockEngine> =
            GroupByOperation::new(vec!["category".into()])
                .min("price")
                .max("price");

        let (sql, _) = op.build_sql();

        assert!(sql.contains("MIN(price)"));
        assert!(sql.contains("MAX(price)"));
    }

    #[test]
    fn test_group_by_with_where() {
        let op: GroupByOperation<TestModel, MockEngine> =
            GroupByOperation::new(vec!["department".into()])
                .count()
                .r#where(Filter::Equals("active".into(), FilterValue::Bool(true)));

        let (sql, params) = op.build_sql();

        assert!(sql.contains("WHERE"));
        assert!(sql.contains("GROUP BY"));
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_group_by_with_having() {
        let op: GroupByOperation<TestModel, MockEngine> =
            GroupByOperation::new(vec!["name".to_string()])
                .count()
                .having(having::count_gt(5.0));

        let (sql, _params) = op.build_sql();

        assert!(sql.contains("HAVING COUNT(*) > 5"));
    }

    #[test]
    fn test_group_by_with_order_and_limit() {
        let op: GroupByOperation<TestModel, MockEngine> =
            GroupByOperation::new(vec!["name".to_string()])
                .count()
                .order_by(OrderByField::desc("_count"))
                .take(10)
                .skip(5);

        let (sql, _params) = op.build_sql();

        assert!(sql.contains("ORDER BY _count DESC")); // Not quoted since "_count" is not a reserved word
        assert!(sql.contains("LIMIT 10"));
        assert!(sql.contains("OFFSET 5"));
    }

    #[test]
    fn test_group_by_order_with_nulls() {
        let op: GroupByOperation<TestModel, MockEngine> =
            GroupByOperation::new(vec!["department".into()])
                .count()
                .order_by(OrderByField::asc("name").nulls(NullsOrder::First));

        let (sql, _) = op.build_sql();

        assert!(sql.contains("ORDER BY"));
        assert!(sql.contains("NULLS FIRST"));
    }

    #[test]
    fn test_group_by_skip_only() {
        let op: GroupByOperation<TestModel, MockEngine> =
            GroupByOperation::new(vec!["department".into()])
                .count()
                .skip(20);

        let (sql, _) = op.build_sql();

        assert!(sql.contains("OFFSET 20"));
        assert!(!sql.contains("LIMIT"));
    }

    #[test]
    fn test_group_by_take_only() {
        let op: GroupByOperation<TestModel, MockEngine> =
            GroupByOperation::new(vec!["department".into()])
                .count()
                .take(50);

        let (sql, _) = op.build_sql();

        assert!(sql.contains("LIMIT 50"));
        assert!(!sql.contains("OFFSET"));
    }

    #[tokio::test]
    async fn test_group_by_exec() {
        let engine = MockEngine;
        let op: GroupByOperation<TestModel, MockEngine> =
            GroupByOperation::new(vec!["department".into()]).count();

        let result = op.exec(&engine).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    // ========== HavingOp Tests ==========

    #[test]
    fn test_having_op_as_str() {
        assert_eq!(HavingOp::Gt.as_str(), ">");
        assert_eq!(HavingOp::Gte.as_str(), ">=");
        assert_eq!(HavingOp::Lt.as_str(), "<");
        assert_eq!(HavingOp::Lte.as_str(), "<=");
        assert_eq!(HavingOp::Eq.as_str(), "=");
        assert_eq!(HavingOp::Ne.as_str(), "<>");
    }

    // ========== HavingCondition Tests ==========

    #[test]
    fn test_having_condition_debug() {
        let cond = HavingCondition {
            field: AggregateField::CountAll,
            op: HavingOp::Gt,
            value: 10.0,
        };
        let debug_str = format!("{:?}", cond);
        assert!(debug_str.contains("HavingCondition"));
    }

    #[test]
    fn test_having_condition_clone() {
        let cond = HavingCondition {
            field: AggregateField::Sum("amount".into()),
            op: HavingOp::Gte,
            value: 1000.0,
        };
        let cloned = cond.clone();
        assert!((cloned.value - 1000.0).abs() < f64::EPSILON);
    }

    // ========== Having Helper Tests ==========

    #[test]
    fn test_having_helpers() {
        let cond = having::count_gt(10.0);
        assert!(matches!(cond.field, AggregateField::CountAll));
        assert!(matches!(cond.op, HavingOp::Gt));
        assert!((cond.value - 10.0).abs() < f64::EPSILON);

        let cond = having::sum_gt("amount", 1000.0);
        if let AggregateField::Sum(col) = cond.field {
            assert_eq!(col, "amount");
        } else {
            panic!("Expected Sum");
        }
    }

    #[test]
    fn test_having_count_gte() {
        let cond = having::count_gte(5.0);
        assert!(matches!(cond.field, AggregateField::CountAll));
        assert!(matches!(cond.op, HavingOp::Gte));
        assert!((cond.value - 5.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_having_count_lt() {
        let cond = having::count_lt(100.0);
        assert!(matches!(cond.field, AggregateField::CountAll));
        assert!(matches!(cond.op, HavingOp::Lt));
        assert!((cond.value - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_having_avg_gt() {
        let cond = having::avg_gt("score", 75.5);
        assert!(matches!(cond.op, HavingOp::Gt));
        assert!((cond.value - 75.5).abs() < f64::EPSILON);
        if let AggregateField::Avg(col) = cond.field {
            assert_eq!(col, "score");
        } else {
            panic!("Expected Avg");
        }
    }

    #[test]
    fn test_having_sum_gt_with_different_columns() {
        let cond1 = having::sum_gt("revenue", 50000.0);
        let cond2 = having::sum_gt("cost", 10000.0);

        if let AggregateField::Sum(col) = &cond1.field {
            assert_eq!(col, "revenue");
        }
        if let AggregateField::Sum(col) = &cond2.field {
            assert_eq!(col, "cost");
        }
    }

    // ========== GroupByResult Tests ==========

    #[test]
    fn test_group_by_result_debug() {
        let result = GroupByResult {
            group_values: std::collections::HashMap::new(),
            aggregates: AggregateResult::default(),
        };
        let debug_str = format!("{:?}", result);
        assert!(debug_str.contains("GroupByResult"));
    }

    #[test]
    fn test_group_by_result_clone() {
        let mut result = GroupByResult {
            group_values: std::collections::HashMap::new(),
            aggregates: AggregateResult::default(),
        };
        result
            .group_values
            .insert("category".into(), serde_json::json!("electronics"));
        result.aggregates.count = Some(50);

        let cloned = result.clone();
        assert_eq!(cloned.aggregates.count, Some(50));
        assert!(cloned.group_values.contains_key("category"));
    }

    // ========== SQL Structure Tests ==========

    #[test]
    fn test_group_by_sql_structure() {
        let op: GroupByOperation<TestModel, MockEngine> =
            GroupByOperation::new(vec!["department".into()])
                .count()
                .r#where(Filter::Equals("active".into(), FilterValue::Bool(true)))
                .having(having::count_gt(5.0))
                .order_by(OrderByField::desc("_count"))
                .take(10)
                .skip(5);

        let (sql, _) = op.build_sql();

        // Check SQL clause ordering: SELECT, FROM, WHERE, GROUP BY, HAVING, ORDER BY, LIMIT, OFFSET
        let select_pos = sql.find("SELECT").unwrap();
        let from_pos = sql.find("FROM").unwrap();
        let where_pos = sql.find("WHERE").unwrap();
        let group_pos = sql.find("GROUP BY").unwrap();
        let having_pos = sql.find("HAVING").unwrap();
        let order_pos = sql.find("ORDER BY").unwrap();
        let limit_pos = sql.find("LIMIT").unwrap();
        let offset_pos = sql.find("OFFSET").unwrap();

        assert!(select_pos < from_pos);
        assert!(from_pos < where_pos);
        assert!(where_pos < group_pos);
        assert!(group_pos < having_pos);
        assert!(having_pos < order_pos);
        assert!(order_pos < limit_pos);
        assert!(limit_pos < offset_pos);
    }

    #[test]
    fn test_aggregate_no_group_by() {
        let op: AggregateOperation<TestModel, MockEngine> =
            AggregateOperation::new().count().sum("score");

        let (sql, _) = op.build_sql();

        assert!(!sql.contains("GROUP BY"));
    }

    #[test]
    fn test_group_by_empty_columns() {
        let op: GroupByOperation<TestModel, MockEngine> = GroupByOperation::new(vec![]).count();

        let (sql, _) = op.build_sql();

        // Empty group columns should not produce GROUP BY
        assert!(!sql.contains("GROUP BY"));
    }
}
