//! Stored procedure and function call support.
//!
//! This module provides a type-safe way to call stored procedures and functions
//! across different database backends.
//!
//! # Supported Features
//!
//! | Feature                  | PostgreSQL | MySQL | MSSQL | SQLite | MongoDB |
//! |--------------------------|------------|-------|-------|--------|---------|
//! | Stored Procedures        | ✅         | ✅    | ✅    | ❌     | ❌      |
//! | User-Defined Functions   | ✅         | ✅    | ✅    | ✅*    | ✅      |
//! | Table-Valued Functions   | ✅         | ❌    | ✅    | ❌     | ❌      |
//! | IN/OUT/INOUT Parameters  | ✅         | ✅    | ✅    | ❌     | ❌      |
//!
//! > *SQLite requires Rust UDFs via `rusqlite::functions`
//!
//! # Example Usage
//!
//! ```rust,ignore
//! use prax_query::procedure::{ProcedureCall, ParameterMode};
//!
//! // Call a stored procedure
//! let result = client
//!     .call("get_user_orders")
//!     .param("user_id", 42)
//!     .exec::<OrderResult>()
//!     .await?;
//!
//! // Call a procedure with OUT parameters
//! let result = client
//!     .call("calculate_totals")
//!     .in_param("order_id", 123)
//!     .out_param::<i64>("total_amount")
//!     .out_param::<i32>("item_count")
//!     .exec()
//!     .await?;
//!
//! // Call a function
//! let result = client
//!     .function("calculate_tax")
//!     .param("amount", 100.0)
//!     .param("rate", 0.08)
//!     .exec::<f64>()
//!     .await?;
//! ```

use std::borrow::Cow;
use std::collections::HashMap;
use std::marker::PhantomData;

use serde::{Deserialize, Serialize};

use crate::error::{QueryError, QueryResult};
use crate::filter::FilterValue;
use crate::sql::DatabaseType;
use crate::traits::{BoxFuture, QueryEngine};

/// Parameter direction mode for stored procedures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ParameterMode {
    /// Input parameter (default).
    In,
    /// Output parameter.
    Out,
    /// Input/Output parameter.
    InOut,
}

impl Default for ParameterMode {
    fn default() -> Self {
        Self::In
    }
}

/// A parameter for a stored procedure or function call.
#[derive(Debug, Clone)]
pub struct Parameter {
    /// Parameter name.
    pub name: String,
    /// Parameter value (None for OUT parameters without initial value).
    pub value: Option<FilterValue>,
    /// Parameter mode (IN, OUT, INOUT).
    pub mode: ParameterMode,
    /// Expected type name for OUT parameters.
    pub type_hint: Option<String>,
}

impl Parameter {
    /// Create a new input parameter.
    pub fn input(name: impl Into<String>, value: impl Into<FilterValue>) -> Self {
        Self {
            name: name.into(),
            value: Some(value.into()),
            mode: ParameterMode::In,
            type_hint: None,
        }
    }

    /// Create a new output parameter.
    pub fn output(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: None,
            mode: ParameterMode::Out,
            type_hint: None,
        }
    }

    /// Create a new input/output parameter.
    pub fn inout(name: impl Into<String>, value: impl Into<FilterValue>) -> Self {
        Self {
            name: name.into(),
            value: Some(value.into()),
            mode: ParameterMode::InOut,
            type_hint: None,
        }
    }

    /// Set a type hint for the parameter.
    pub fn with_type_hint(mut self, type_name: impl Into<String>) -> Self {
        self.type_hint = Some(type_name.into());
        self
    }
}

/// Result from a procedure call with OUT/INOUT parameters.
#[derive(Debug, Clone, Default)]
pub struct ProcedureResult {
    /// Output parameter values by name.
    pub outputs: HashMap<String, FilterValue>,
    /// Return value (for functions).
    pub return_value: Option<FilterValue>,
    /// Number of rows affected (if applicable).
    pub rows_affected: Option<u64>,
}

impl ProcedureResult {
    /// Get an output parameter value.
    pub fn get(&self, name: &str) -> Option<&FilterValue> {
        self.outputs.get(name)
    }

    /// Get an output parameter as a specific type.
    pub fn get_as<T>(&self, name: &str) -> Option<T>
    where
        T: TryFrom<FilterValue>,
    {
        self.outputs
            .get(name)
            .and_then(|v| T::try_from(v.clone()).ok())
    }

    /// Get the return value.
    pub fn return_value(&self) -> Option<&FilterValue> {
        self.return_value.as_ref()
    }

    /// Get the return value as a specific type.
    pub fn return_value_as<T>(&self) -> Option<T>
    where
        T: TryFrom<FilterValue>,
    {
        self.return_value.clone().and_then(|v| T::try_from(v).ok())
    }
}

/// Builder for stored procedure calls.
#[derive(Debug, Clone)]
pub struct ProcedureCall {
    /// Procedure/function name.
    pub name: String,
    /// Schema name (optional).
    pub schema: Option<String>,
    /// Parameters.
    pub parameters: Vec<Parameter>,
    /// Database type for SQL generation.
    pub db_type: DatabaseType,
    /// Whether this is a function call (vs procedure).
    pub is_function: bool,
}

impl ProcedureCall {
    /// Create a new procedure call.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            schema: None,
            parameters: Vec::new(),
            db_type: DatabaseType::PostgreSQL,
            is_function: false,
        }
    }

    /// Create a new function call.
    pub fn function(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            schema: None,
            parameters: Vec::new(),
            db_type: DatabaseType::PostgreSQL,
            is_function: true,
        }
    }

    /// Set the schema name.
    pub fn schema(mut self, schema: impl Into<String>) -> Self {
        self.schema = Some(schema.into());
        self
    }

    /// Set the database type.
    pub fn with_db_type(mut self, db_type: DatabaseType) -> Self {
        self.db_type = db_type;
        self
    }

    /// Add an input parameter.
    pub fn param(mut self, name: impl Into<String>, value: impl Into<FilterValue>) -> Self {
        self.parameters.push(Parameter::input(name, value));
        self
    }

    /// Add an input parameter (alias for param).
    pub fn in_param(self, name: impl Into<String>, value: impl Into<FilterValue>) -> Self {
        self.param(name, value)
    }

    /// Add an output parameter.
    pub fn out_param(mut self, name: impl Into<String>) -> Self {
        self.parameters.push(Parameter::output(name));
        self
    }

    /// Add an output parameter with type hint.
    pub fn out_param_typed(
        mut self,
        name: impl Into<String>,
        type_hint: impl Into<String>,
    ) -> Self {
        self.parameters
            .push(Parameter::output(name).with_type_hint(type_hint));
        self
    }

    /// Add an input/output parameter.
    pub fn inout_param(mut self, name: impl Into<String>, value: impl Into<FilterValue>) -> Self {
        self.parameters.push(Parameter::inout(name, value));
        self
    }

    /// Add a raw parameter.
    pub fn add_parameter(mut self, param: Parameter) -> Self {
        self.parameters.push(param);
        self
    }

    /// Get the fully qualified name.
    pub fn qualified_name(&self) -> Cow<'_, str> {
        match &self.schema {
            Some(schema) => Cow::Owned(format!("{}.{}", schema, self.name)),
            None => Cow::Borrowed(&self.name),
        }
    }

    /// Check if any parameters are OUT or INOUT.
    pub fn has_outputs(&self) -> bool {
        self.parameters
            .iter()
            .any(|p| matches!(p.mode, ParameterMode::Out | ParameterMode::InOut))
    }

    /// Get input parameter values.
    pub fn input_values(&self) -> Vec<FilterValue> {
        self.parameters
            .iter()
            .filter(|p| matches!(p.mode, ParameterMode::In | ParameterMode::InOut))
            .filter_map(|p| p.value.clone())
            .collect()
    }

    /// Generate SQL for PostgreSQL.
    pub fn to_postgres_sql(&self) -> (String, Vec<FilterValue>) {
        let name = self.qualified_name();
        let params = self.input_values();
        let placeholders: Vec<String> = (1..=params.len()).map(|i| format!("${}", i)).collect();

        let sql = if self.is_function {
            format!("SELECT {}({})", name, placeholders.join(", "))
        } else {
            format!("CALL {}({})", name, placeholders.join(", "))
        };

        (sql, params)
    }

    /// Generate SQL for MySQL.
    pub fn to_mysql_sql(&self) -> (String, Vec<FilterValue>) {
        let name = self.qualified_name();
        let params = self.input_values();
        let placeholders = vec!["?"; params.len()].join(", ");

        let sql = if self.is_function {
            format!("SELECT {}({})", name, placeholders)
        } else {
            format!("CALL {}({})", name, placeholders)
        };

        (sql, params)
    }

    /// Generate SQL for MSSQL.
    pub fn to_mssql_sql(&self) -> (String, Vec<FilterValue>) {
        let name = self.qualified_name();
        let params = self.input_values();
        let placeholders: Vec<String> = (1..=params.len()).map(|i| format!("@P{}", i)).collect();

        if self.is_function {
            (
                format!("SELECT {}({})", name, placeholders.join(", ")),
                params,
            )
        } else if self.has_outputs() {
            // For procedures with OUT params, use EXEC with output variable declarations
            let mut parts = vec![String::from("DECLARE ")];

            // Declare output variables
            let out_params: Vec<_> = self
                .parameters
                .iter()
                .filter(|p| matches!(p.mode, ParameterMode::Out | ParameterMode::InOut))
                .collect();

            for (i, param) in out_params.iter().enumerate() {
                if i > 0 {
                    parts.push(String::from(", "));
                }
                let type_name = param.type_hint.as_deref().unwrap_or("SQL_VARIANT");
                parts.push(format!("@{} {}", param.name, type_name));
            }
            parts.push(String::from("; "));

            // Build EXEC statement
            parts.push(format!("EXEC {} ", name));

            let param_parts: Vec<String> = self
                .parameters
                .iter()
                .enumerate()
                .map(|(i, p)| match p.mode {
                    ParameterMode::In => format!("@P{}", i + 1),
                    ParameterMode::Out => format!("@{} OUTPUT", p.name),
                    ParameterMode::InOut => format!("@P{} = @{} OUTPUT", i + 1, p.name),
                })
                .collect();

            parts.push(param_parts.join(", "));
            parts.push(String::from("; "));

            // Select output values
            let select_parts: Vec<String> = out_params
                .iter()
                .map(|p| format!("@{} AS {}", p.name, p.name))
                .collect();
            parts.push(format!("SELECT {}", select_parts.join(", ")));

            (parts.join(""), params)
        } else {
            (format!("EXEC {} {}", name, placeholders.join(", ")), params)
        }
    }

    /// Generate SQL for SQLite (only functions supported).
    pub fn to_sqlite_sql(&self) -> QueryResult<(String, Vec<FilterValue>)> {
        if !self.is_function {
            return Err(QueryError::unsupported(
                "SQLite does not support stored procedures. Use Rust UDFs instead.",
            ));
        }

        let name = self.qualified_name();
        let params = self.input_values();
        let placeholders = vec!["?"; params.len()].join(", ");

        Ok((format!("SELECT {}({})", name, placeholders), params))
    }

    /// Generate SQL for the configured database type.
    pub fn to_sql(&self) -> QueryResult<(String, Vec<FilterValue>)> {
        match self.db_type {
            DatabaseType::PostgreSQL => Ok(self.to_postgres_sql()),
            DatabaseType::MySQL => Ok(self.to_mysql_sql()),
            DatabaseType::SQLite => self.to_sqlite_sql(),
            DatabaseType::MSSQL => Ok(self.to_mssql_sql()),
        }
    }
}

/// Operation for executing a procedure call.
pub struct ProcedureCallOperation<E: QueryEngine> {
    engine: E,
    call: ProcedureCall,
}

impl<E: QueryEngine> ProcedureCallOperation<E> {
    /// Create a new procedure call operation.
    pub fn new(engine: E, call: ProcedureCall) -> Self {
        Self { engine, call }
    }

    /// Execute the procedure and return the result.
    pub async fn exec(self) -> QueryResult<ProcedureResult> {
        let (sql, params) = self.call.to_sql()?;
        let affected = self.engine.execute_raw(&sql, params).await?;

        Ok(ProcedureResult {
            outputs: HashMap::new(),
            return_value: None,
            rows_affected: Some(affected),
        })
    }

    /// Execute the procedure and return typed results.
    pub async fn exec_returning<T>(self) -> QueryResult<Vec<T>>
    where
        T: crate::traits::Model + crate::row::FromRow + Send + 'static,
    {
        let (sql, params) = self.call.to_sql()?;
        self.engine.query_many(&sql, params).await
    }

    /// Execute a function and return a single value.
    ///
    /// Routes through [`QueryEngine::aggregate_query`] so the scalar
    /// return lands in the first column of the first row as a
    /// [`FilterValue`]. The caller's `T: TryFrom<FilterValue>` impl
    /// handles the final type coercion — e.g., `T = i64` succeeds on
    /// `FilterValue::Int`, errors on `FilterValue::String`.
    pub async fn exec_scalar<T>(self) -> QueryResult<T>
    where
        T: TryFrom<FilterValue, Error = String> + Send + 'static,
    {
        let (sql, params) = self.call.to_sql()?;
        let mut rows = self.engine.aggregate_query(&sql, params).await?;
        let first = rows
            .drain(..)
            .next()
            .ok_or_else(|| QueryError::not_found("scalar function returned no row".to_string()))?;
        // Take any value from the map — scalar functions produce a
        // single column, but the column name is driver-dependent.
        let value = first.into_values().next().ok_or_else(|| {
            QueryError::deserialization(
                "scalar function returned a row with no columns".to_string(),
            )
        })?;
        T::try_from(value).map_err(QueryError::deserialization)
    }
}

/// Operation for executing a function call that returns a value.
#[allow(dead_code)]
pub struct FunctionCallOperation<E: QueryEngine, T> {
    engine: E,
    call: ProcedureCall,
    _marker: PhantomData<T>,
}

impl<E: QueryEngine, T> FunctionCallOperation<E, T> {
    /// Create a new function call operation.
    pub fn new(engine: E, call: ProcedureCall) -> Self {
        Self {
            engine,
            call,
            _marker: PhantomData,
        }
    }
}

/// Extension trait for query engines to support procedure calls.
pub trait ProcedureEngine: QueryEngine {
    /// Call a stored procedure.
    fn call(&self, name: impl Into<String>) -> ProcedureCall {
        ProcedureCall::new(name)
    }

    /// Call a function.
    fn function(&self, name: impl Into<String>) -> ProcedureCall {
        ProcedureCall::function(name)
    }

    /// Execute a procedure call.
    fn execute_procedure(&self, call: ProcedureCall) -> BoxFuture<'_, QueryResult<ProcedureResult>>
    where
        Self: Clone + 'static,
    {
        let engine = self.clone();
        Box::pin(async move {
            let op = ProcedureCallOperation::new(engine, call);
            op.exec().await
        })
    }
}

// Implement ProcedureEngine for all QueryEngine implementations
impl<T: QueryEngine + Clone + 'static> ProcedureEngine for T {}

/// SQLite-specific UDF registration support.
pub mod sqlite_udf {
    #[allow(unused_imports)]
    use super::*;

    /// A Rust function that can be registered as a SQLite UDF.
    pub trait SqliteFunction: Send + Sync + 'static {
        /// The name of the function.
        fn name(&self) -> &str;

        /// The number of arguments (-1 for variadic).
        fn num_args(&self) -> i32;

        /// Whether the function is deterministic.
        fn deterministic(&self) -> bool {
            true
        }
    }

    /// A scalar UDF definition.
    #[derive(Debug, Clone)]
    pub struct ScalarUdf {
        /// Function name.
        pub name: String,
        /// Number of arguments.
        pub num_args: i32,
        /// Whether deterministic.
        pub deterministic: bool,
    }

    impl ScalarUdf {
        /// Create a new scalar UDF definition.
        pub fn new(name: impl Into<String>, num_args: i32) -> Self {
            Self {
                name: name.into(),
                num_args,
                deterministic: true,
            }
        }

        /// Set whether the function is deterministic.
        pub fn deterministic(mut self, deterministic: bool) -> Self {
            self.deterministic = deterministic;
            self
        }
    }

    /// An aggregate UDF definition.
    #[derive(Debug, Clone)]
    pub struct AggregateUdf {
        /// Function name.
        pub name: String,
        /// Number of arguments.
        pub num_args: i32,
    }

    impl AggregateUdf {
        /// Create a new aggregate UDF definition.
        pub fn new(name: impl Into<String>, num_args: i32) -> Self {
            Self {
                name: name.into(),
                num_args,
            }
        }
    }

    /// A window UDF definition.
    #[derive(Debug, Clone)]
    pub struct WindowUdf {
        /// Function name.
        pub name: String,
        /// Number of arguments.
        pub num_args: i32,
    }

    impl WindowUdf {
        /// Create a new window UDF definition.
        pub fn new(name: impl Into<String>, num_args: i32) -> Self {
            Self {
                name: name.into(),
                num_args,
            }
        }
    }
}

/// MongoDB-specific function support.
pub mod mongodb_func {
    use super::*;

    /// A MongoDB `$function` expression for custom JavaScript functions.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct MongoFunction {
        /// JavaScript function body.
        pub body: String,
        /// Function arguments (field references or values).
        pub args: Vec<String>,
        /// Language (always "js" for now).
        pub lang: String,
    }

    impl MongoFunction {
        /// Create a new MongoDB function.
        pub fn new(body: impl Into<String>, args: Vec<impl Into<String>>) -> Self {
            Self {
                body: body.into(),
                args: args.into_iter().map(Into::into).collect(),
                lang: "js".to_string(),
            }
        }

        /// Convert to a BSON document for use in aggregation.
        #[cfg(feature = "mongodb")]
        pub fn to_bson(&self) -> bson::Document {
            use bson::doc;
            doc! {
                "$function": {
                    "body": &self.body,
                    "args": &self.args,
                    "lang": &self.lang,
                }
            }
        }
    }

    /// A MongoDB `$accumulator` expression for custom aggregation.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct MongoAccumulator {
        /// Initialize the accumulator state.
        pub init: String,
        /// Initialize arguments.
        pub init_args: Vec<String>,
        /// Accumulate function.
        pub accumulate: String,
        /// Accumulate arguments.
        pub accumulate_args: Vec<String>,
        /// Merge function.
        pub merge: String,
        /// Finalize function (optional).
        pub finalize: Option<String>,
        /// Language.
        pub lang: String,
    }

    impl MongoAccumulator {
        /// Create a new MongoDB accumulator.
        pub fn new(
            init: impl Into<String>,
            accumulate: impl Into<String>,
            merge: impl Into<String>,
        ) -> Self {
            Self {
                init: init.into(),
                init_args: Vec::new(),
                accumulate: accumulate.into(),
                accumulate_args: Vec::new(),
                merge: merge.into(),
                finalize: None,
                lang: "js".to_string(),
            }
        }

        /// Set init arguments.
        pub fn with_init_args(mut self, args: Vec<impl Into<String>>) -> Self {
            self.init_args = args.into_iter().map(Into::into).collect();
            self
        }

        /// Set accumulate arguments.
        pub fn with_accumulate_args(mut self, args: Vec<impl Into<String>>) -> Self {
            self.accumulate_args = args.into_iter().map(Into::into).collect();
            self
        }

        /// Set finalize function.
        pub fn with_finalize(mut self, finalize: impl Into<String>) -> Self {
            self.finalize = Some(finalize.into());
            self
        }

        /// Convert to a BSON document for use in aggregation.
        #[cfg(feature = "mongodb")]
        pub fn to_bson(&self) -> bson::Document {
            use bson::doc;
            let mut doc = doc! {
                "$accumulator": {
                    "init": &self.init,
                    "accumulate": &self.accumulate,
                    "accumulateArgs": &self.accumulate_args,
                    "merge": &self.merge,
                    "lang": &self.lang,
                }
            };

            if !self.init_args.is_empty() {
                doc.get_document_mut("$accumulator")
                    .unwrap()
                    .insert("initArgs", &self.init_args);
            }

            if let Some(ref finalize) = self.finalize {
                doc.get_document_mut("$accumulator")
                    .unwrap()
                    .insert("finalize", finalize);
            }

            doc
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_procedure_call_basic() {
        let call = ProcedureCall::new("get_user")
            .param("id", 42i32)
            .param("active", true);

        assert_eq!(call.name, "get_user");
        assert_eq!(call.parameters.len(), 2);
        assert!(!call.is_function);
    }

    #[test]
    fn test_function_call() {
        let call = ProcedureCall::function("calculate_tax")
            .param("amount", 100.0f64)
            .param("rate", 0.08f64);

        assert_eq!(call.name, "calculate_tax");
        assert!(call.is_function);
    }

    #[test]
    fn test_postgres_sql_generation() {
        let call = ProcedureCall::new("get_orders")
            .param("user_id", 42i32)
            .param("status", "pending".to_string());

        let (sql, params) = call.to_postgres_sql();
        assert_eq!(sql, "CALL get_orders($1, $2)");
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_postgres_function_sql() {
        let call = ProcedureCall::function("calculate_total").param("order_id", 123i32);

        let (sql, params) = call.to_postgres_sql();
        assert_eq!(sql, "SELECT calculate_total($1)");
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_mysql_sql_generation() {
        let call = ProcedureCall::new("get_orders")
            .with_db_type(DatabaseType::MySQL)
            .param("user_id", 42i32);

        let (sql, params) = call.to_mysql_sql();
        assert_eq!(sql, "CALL get_orders(?)");
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_mssql_sql_generation() {
        let call = ProcedureCall::new("GetOrders")
            .schema("dbo")
            .with_db_type(DatabaseType::MSSQL)
            .param("UserId", 42i32);

        let (sql, params) = call.to_mssql_sql();
        assert!(sql.contains("EXEC dbo.GetOrders"));
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_mssql_with_output_params() {
        let call = ProcedureCall::new("CalculateTotals")
            .with_db_type(DatabaseType::MSSQL)
            .in_param("OrderId", 123i32)
            .out_param_typed("TotalAmount", "DECIMAL(18,2)")
            .out_param_typed("ItemCount", "INT");

        let (sql, _params) = call.to_mssql_sql();
        assert!(sql.contains("DECLARE"));
        assert!(sql.contains("OUTPUT"));
        assert!(sql.contains("SELECT"));
    }

    #[test]
    fn test_sqlite_function() {
        let call = ProcedureCall::function("custom_hash")
            .with_db_type(DatabaseType::SQLite)
            .param("input", "test".to_string());

        let result = call.to_sqlite_sql();
        assert!(result.is_ok());

        let (sql, params) = result.unwrap();
        assert_eq!(sql, "SELECT custom_hash(?)");
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_sqlite_procedure_error() {
        let call = ProcedureCall::new("some_procedure")
            .with_db_type(DatabaseType::SQLite)
            .param("id", 42i32);

        let result = call.to_sqlite_sql();
        assert!(result.is_err());
    }

    #[test]
    fn test_qualified_name() {
        let call = ProcedureCall::new("get_user").schema("public");
        assert_eq!(call.qualified_name(), "public.get_user");

        let call = ProcedureCall::new("get_user");
        assert_eq!(call.qualified_name(), "get_user");
    }

    #[test]
    fn test_parameter_modes() {
        let call = ProcedureCall::new("calculate")
            .in_param("input", 100i32)
            .out_param("result")
            .inout_param("running_total", 50i32);

        assert_eq!(call.parameters.len(), 3);
        assert_eq!(call.parameters[0].mode, ParameterMode::In);
        assert_eq!(call.parameters[1].mode, ParameterMode::Out);
        assert_eq!(call.parameters[2].mode, ParameterMode::InOut);
        assert!(call.has_outputs());
    }

    #[test]
    fn test_procedure_result() {
        let mut result = ProcedureResult::default();
        result
            .outputs
            .insert("total".to_string(), FilterValue::Int(100));
        result.return_value = Some(FilterValue::Bool(true));

        assert!(result.get("total").is_some());
        assert!(result.get("nonexistent").is_none());
        assert!(result.return_value().is_some());
    }

    #[test]
    fn test_mongo_function() {
        use mongodb_func::MongoFunction;

        let func = MongoFunction::new(
            "function(x, y) { return x + y; }",
            vec!["$field1", "$field2"],
        );

        assert_eq!(func.lang, "js");
        assert_eq!(func.args.len(), 2);
    }

    #[test]
    fn test_mongo_accumulator() {
        use mongodb_func::MongoAccumulator;

        let acc = MongoAccumulator::new(
            "function() { return { sum: 0, count: 0 }; }",
            "function(state, value) { state.sum += value; state.count++; return state; }",
            "function(s1, s2) { return { sum: s1.sum + s2.sum, count: s1.count + s2.count }; }",
        )
        .with_finalize("function(state) { return state.sum / state.count; }")
        .with_accumulate_args(vec!["$value"]);

        assert!(acc.finalize.is_some());
        assert_eq!(acc.accumulate_args.len(), 1);
    }

    #[test]
    fn test_sqlite_udf_definitions() {
        use sqlite_udf::{AggregateUdf, ScalarUdf, WindowUdf};

        let scalar = ScalarUdf::new("my_hash", 1).deterministic(true);
        assert!(scalar.deterministic);

        let aggregate = AggregateUdf::new("my_sum", 1);
        assert_eq!(aggregate.num_args, 1);

        let window = WindowUdf::new("my_rank", 0);
        assert_eq!(window.num_args, 0);
    }
}
