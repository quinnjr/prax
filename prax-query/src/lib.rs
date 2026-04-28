//! # prax-query
//!
//! Type-safe query builder for the Prax ORM.
//!
//! This crate provides the core query building functionality, including:
//! - Fluent API for building queries (`find_many`, `find_unique`, `create`, `update`, `delete`)
//! - Type-safe filtering with `where` clauses
//! - Sorting and pagination
//! - Relation loading (`include`, `select`)
//! - Transaction support
//! - Raw SQL escape hatch
//! - Middleware system
//! - Multi-tenant support
//!
//! ## Filters
//!
//! Build type-safe filters for queries:
//!
//! ```rust
//! use prax_query::{Filter, FilterValue};
//!
//! // Equality filter
//! let filter = Filter::Equals("email".into(), FilterValue::String("test@example.com".into()));
//!
//! // Greater than filter
//! let filter = Filter::Gt("age".into(), FilterValue::Int(18));
//!
//! // Contains filter (for strings)
//! let filter = Filter::Contains("name".into(), FilterValue::String("john".into()));
//!
//! // Combine filters with AND/OR
//! let combined = Filter::and([
//!     Filter::Equals("active".into(), FilterValue::Bool(true)),
//!     Filter::Gt("age".into(), FilterValue::Int(18)),
//! ]);
//!
//! let either = Filter::or([
//!     Filter::Equals("role".into(), FilterValue::String("admin".into())),
//!     Filter::Equals("role".into(), FilterValue::String("moderator".into())),
//! ]);
//! ```
//!
//! ## Filter Values
//!
//! Convert Rust types to filter values:
//!
//! ```rust
//! use prax_query::FilterValue;
//!
//! // Integer values
//! let val: FilterValue = 42.into();
//! assert!(matches!(val, FilterValue::Int(42)));
//!
//! // String values
//! let val: FilterValue = "hello".into();
//! assert!(matches!(val, FilterValue::String(_)));
//!
//! // Boolean values
//! let val: FilterValue = true.into();
//! assert!(matches!(val, FilterValue::Bool(true)));
//!
//! // Float values
//! let val: FilterValue = 3.14f64.into();
//! assert!(matches!(val, FilterValue::Float(_)));
//!
//! // Null values
//! let val = FilterValue::Null;
//! ```
//!
//! ## Sorting
//!
//! Build sort specifications:
//!
//! ```rust
//! use prax_query::{OrderBy, OrderByField, NullsOrder};
//!
//! // Ascending order
//! let order = OrderByField::asc("created_at");
//!
//! // Descending order
//! let order = OrderByField::desc("updated_at");
//!
//! // With NULLS FIRST/LAST
//! let order = OrderByField::asc("name").nulls(NullsOrder::First);
//! let order = OrderByField::desc("score").nulls(NullsOrder::Last);
//!
//! // Combine multiple orderings
//! let orders = OrderBy::Field(OrderByField::asc("name"))
//!     .then(OrderByField::desc("created_at"));
//! ```
//!
//! ## Raw SQL
//!
//! Build raw SQL queries with parameter binding:
//!
//! ```rust
//! use prax_query::Sql;
//!
//! // Simple query
//! let sql = Sql::new("SELECT * FROM users");
//! assert_eq!(sql.sql(), "SELECT * FROM users");
//!
//! // Query with parameter - bind appends placeholder
//! let sql = Sql::new("SELECT * FROM users WHERE id = ")
//!     .bind(42);
//! assert_eq!(sql.params().len(), 1);
//! ```
//!
//! ## Connection Strings
//!
//! Parse database connection strings:
//!
//! ```rust
//! use prax_query::ConnectionString;
//!
//! // PostgreSQL
//! let conn = ConnectionString::parse("postgres://user:pass@localhost:5432/mydb").unwrap();
//! assert_eq!(conn.host(), Some("localhost"));
//! assert_eq!(conn.port(), Some(5432));
//! assert_eq!(conn.database(), Some("mydb"));
//!
//! // MySQL
//! let conn = ConnectionString::parse("mysql://user:pass@localhost:3306/mydb").unwrap();
//! ```
//!
//! ## Transaction Config
//!
//! Configure transaction behavior:
//!
//! ```rust
//! use prax_query::IsolationLevel;
//!
//! let level = IsolationLevel::Serializable;
//! assert_eq!(level.as_sql(), "SERIALIZABLE");
//! ```
//!
//! ## Error Handling
//!
//! Work with query errors:
//!
//! ```rust
//! use prax_query::{QueryError, ErrorCode};
//!
//! // Create errors
//! let err = QueryError::not_found("User");
//! assert_eq!(err.code, ErrorCode::RecordNotFound);
//! ```

pub mod advanced;
pub mod async_optimize;
pub mod batch;
pub mod builder;
pub mod cache;
pub mod connection;
pub mod cte;
pub mod data;
#[allow(dead_code, unused_imports)]
pub mod data_cache;
pub mod db_optimize;
pub mod dialect;
pub mod error;
pub mod extension;
pub mod filter;
pub mod intern;
pub mod introspection;
pub mod json;
pub mod lazy;
pub mod logging;
#[macro_use]
pub mod macros;
pub mod mem_optimize;
pub mod memory;
pub mod middleware;
pub mod nested;
pub mod operations;
pub mod pagination;
pub mod partition;
pub mod pool;
pub mod procedure;
pub mod profiling;
pub mod query;
pub mod raw;
pub mod relations;
pub mod replication;
pub mod row;
pub mod search;
pub mod security;
pub mod sequence;
pub mod sql;
pub mod static_filter;
pub mod tenant;
pub mod traits;
pub mod transaction;
pub mod trigger;
pub mod typed_filter;
pub mod types;
pub mod upsert;
pub mod window;
pub mod zero_copy;

pub use error::{ErrorCode, ErrorContext, QueryError, QueryResult, Suggestion};
pub use extension::{Extension, ExtensionBuilder, Point, Polygon};
pub use filter::{
    AndFilterBuilder, FieldName, Filter, FilterValue, FluentFilterBuilder, LargeValueList,
    OrFilterBuilder, ScalarFilter, SmallValueList, ToFilterValue, ValueList,
};
pub use json::{JsonAgg, JsonFilter, JsonIndex, JsonIndexBuilder, JsonOp, JsonPath, PathSegment};
pub use nested::{NestedWrite, NestedWriteBuilder, NestedWriteOperations};
pub use operations::{
    AggregateField,
    AggregateOperation,
    AggregateResult,
    CountOperation,
    CreateManyOperation,
    CreateOperation,
    DeleteManyOperation,
    DeleteOperation,
    FindFirstOperation,
    FindManyOperation,
    FindUniqueOperation,
    GroupByOperation,
    GroupByResult,
    HavingCondition,
    HavingOp,
    // View operations
    MaterializedViewAccessor,
    RefreshMaterializedViewOperation,
    UpdateManyOperation,
    UpdateOperation,
    UpsertOperation,
    ViewAccessor,
    ViewCountOperation,
    ViewFindFirstOperation,
    ViewFindManyOperation,
    ViewQueryBuilder,
    having,
};
pub use pagination::{Cursor, CursorDirection, Pagination};
pub use partition::{
    HashPartitionDef, ListPartitionDef, Partition, PartitionBuilder, PartitionDef, PartitionType,
    RangeBound, RangePartitionDef,
};
pub use procedure::{
    Parameter, ParameterMode, ProcedureCall, ProcedureCallOperation, ProcedureEngine,
    ProcedureResult,
};
pub use query::QueryBuilder;
pub use raw::{RawExecuteOperation, RawQueryOperation, Sql};
pub use relations::{Include, IncludeSpec, RelationLoader, RelationSpec, SelectSpec};
pub use search::{
    FullTextIndex, FullTextIndexBuilder, FuzzyOptions, HighlightOptions, RankingOptions,
    SearchLanguage, SearchMode, SearchQuery, SearchQueryBuilder, SearchSql,
};
pub use security::{
    ConnectionProfile, ConnectionProfileBuilder, DataMask, Grant, GrantBuilder, GrantObject,
    MaskFunction, PolicyCommand, Privilege, RlsPolicy, RlsPolicyBuilder, Role, RoleBuilder,
    TenantPolicy, TenantSource,
};
pub use sequence::{OwnedBy, Sequence, SequenceBuilder};
pub use traits::{
    Executable, IntoFilter, MaterializedView, Model, ModelWithPk, QueryEngine, View,
    ViewQueryEngine,
};
pub use transaction::{IsolationLevel, Transaction, TransactionConfig};
pub use trigger::{
    Trigger, TriggerAction, TriggerBuilder, TriggerCondition, TriggerEvent, TriggerLevel,
    TriggerTiming, UpdateOf,
};
pub use types::{
    NullsOrder, OrderBy, OrderByBuilder, OrderByField, Select, SortOrder, order_patterns,
};
pub use upsert::{
    Assignment, AssignmentValue, ConflictAction, ConflictTarget, UpdateSpec, Upsert, UpsertBuilder,
};
pub use window::{
    FrameBound, FrameClause, FrameExclude, FrameType, NamedWindow, NullsPosition, OrderSpec,
    WindowFn, WindowFunction, WindowFunctionBuilder, WindowSpec,
};

// Re-export middleware types
pub use middleware::{
    LoggingMiddleware, MetricsMiddleware, Middleware, MiddlewareBuilder, MiddlewareChain,
    MiddlewareStack, QueryContext, QueryMetadata, QueryMetrics, QueryType, RetryMiddleware,
    TimingMiddleware,
};

// Re-export dialect types
pub use dialect::{Mssql, Mysql, NotSql, Postgres, SqlDialect, Sqlite};

// Re-export connection types
pub use connection::{
    ConnectionError, ConnectionOptions, ConnectionString, DatabaseConfig, Driver, EnvExpander,
    MultiDatabaseConfig, PoolConfig, PoolOptions, SslConfig, SslMode,
};
pub use cte::{
    Cte, CteBuilder, CycleClause, Materialized, SearchClause, SearchMethod, WithClause,
    WithQueryBuilder,
};

// Re-export advanced query types
pub use advanced::{
    BulkOperation, DistinctOn, LateralJoin, LateralJoinBuilder, LateralJoinType, LockStrength,
    LockWait, ReturnOperation, Returning, ReturningColumn, RowLock, RowLockBuilder, SampleMethod,
    SampleSize, TableSample, TableSampleBuilder,
};

// Re-export data types
pub use data::{
    BatchCreate, ConnectData, CreateData, DataBuilder, FieldValue, IntoData, UpdateData,
};

// Re-export introspection types
pub use introspection::{
    CheckConstraint, ColumnInfo, DatabaseSchema, EnumInfo, ForeignKeyInfo, IndexColumn, IndexInfo,
    NormalizedType, ReferentialAction, SequenceInfo, TableInfo, UniqueConstraint, ViewInfo,
    generate_prax_schema, normalize_type,
};

// Re-export tenant types
pub use tenant::{
    DynamicResolver, IsolationStrategy, RowLevelConfig, SchemaConfig, StaticResolver, TenantConfig,
    TenantConfigBuilder, TenantContext, TenantId, TenantInfo, TenantMiddleware, TenantResolver,
};

// Re-export intern types
pub use intern::{clear_interned, fields, intern, intern_cow, interned_count};

// Re-export pool types
pub use pool::{FilterBuilder, FilterPool, IntoPooledValue, PooledFilter, PooledValue};

// Re-export SQL builder types
pub use sql::{
    AdvancedQueryCapacity, CachedSql, DatabaseType, FastSqlBuilder, LazySql, QueryCapacity,
    SqlBuilder, SqlTemplateCache as SqlCache, global_sql_cache, keywords, templates,
};

// Re-export optimized builder types
pub use builder::{
    BuilderPool,
    ColumnList,
    ColumnNameList,
    CowColumnList,
    CowIdentifier,
    ExprList,
    // Note: FrameBound and FrameType are also defined in window module
    FrameBound as BuilderFrameBound,
    FrameType as BuilderFrameType,
    Identifier,
    OptimizedWindowSpec,
    OrderByList,
    PartitionByList,
    ReusableBuilder,
    WindowFrame,
};

// Re-export database optimization types
pub use db_optimize::{
    BatchConfig, CachedStatement, IndexHint, IndexHintType, JoinHint, JoinMethod,
    MongoPipelineBuilder, PipelineStage, PreparedStatementCache, PreparedStatementStats,
    QueryHints, global_statement_cache,
};

// Re-export zero-copy types
pub use zero_copy::{
    CteRef, FrameBoundRef, FrameRef, FrameTypeRef, JsonPathRef, PathSegmentRef, WindowSpecRef,
    WithClauseRef,
};

// Re-export cache types
pub use cache::{
    CacheStats, CachedQuery, ExecutionPlan, ExecutionPlanCache, PlanHint, QueryCache, QueryHash,
    QueryKey, SqlTemplate, SqlTemplateCache, get_global_template, global_template_cache,
    patterns as cache_patterns, precompute_query_hash, register_global_template,
};

// Re-export batch types
pub use batch::{
    Batch, BatchBuilder, BatchOperation, BatchResult, OperationResult, Pipeline, PipelineBuilder,
    PipelineQuery, PipelineResult, QueryResult as PipelineQueryResult,
};

// Re-export row deserialization types
pub use row::{FromColumn, FromRow, FromRowRef, RowData, RowError, RowRef, RowRefIter};

// Re-export lazy loading types
pub use lazy::{Lazy, LazyRelation, ManyToOneLoader, OneToManyLoader};

// Re-export static filter utilities
pub use static_filter::{
    CompactValue, StaticFilter, and2, and3, and4, and5, contains, ends_with, eq,
    fields as static_fields, gt, gte, in_list, is_not_null, is_null, lt, lte, ne, not, not_in_list,
    or2, or3, or4, or5, starts_with,
};

// Re-export typed filter utilities
pub use typed_filter::{
    And, AndN, Contains, DirectSql, EndsWith, Eq, Gt, Gte, InI64, InI64Slice, InStr, InStrSlice,
    IsNotNull, IsNull, LazyFilter, Lt, Lte, Maybe, Ne, Not as TypedNot, NotInI64Slice, Or, OrN,
    StartsWith, TypedFilter, and_n, eq as typed_eq, gt as typed_gt, gte as typed_gte,
    in_i64 as typed_in_i64, in_i64_slice, in_str as typed_in_str, in_str_slice,
    is_not_null as typed_is_not_null, is_null as typed_is_null, lazy, lt as typed_lt,
    lte as typed_lte, ne as typed_ne, not_in_i64_slice, or_n,
};

// Re-export memory optimization utilities
pub use memory::{
    BufferPool, CompactFilter, GLOBAL_BUFFER_POOL, GLOBAL_STRING_POOL, MemoryStats, PoolStats,
    PooledBuffer, StringPool, get_buffer, intern as memory_intern,
};

// Re-export logging utilities
pub use logging::{
    get_log_format, get_log_level, init as init_logging, init_debug, init_with_level,
    is_debug_enabled,
};

// Re-export replication types
pub use replication::{
    ConnectionRouter, HealthStatus, LagMeasurement, LagMonitor, ReadPreference, ReplicaConfig,
    ReplicaHealth, ReplicaRole, ReplicaSetBuilder, ReplicaSetConfig,
};

// Re-export async optimization types
pub use async_optimize::{
    ConcurrencyConfig, ConcurrentExecutor, ExecutionStats, IntrospectionConfig,
    IntrospectionResult, PipelineConfig, PipelineError, PipelineResult as AsyncPipelineResult,
    QueryPipeline, TaskError, TaskResult,
    concurrent::execute_batch as async_execute_batch,
    concurrent::execute_chunked as async_execute_chunked,
    introspect::{
        BatchIntrospector, ColumnMetadata, ConcurrentIntrospector, ForeignKeyMetadata,
        IndexMetadata, IntrospectionError, IntrospectionPhase, IntrospectorBuilder, TableMetadata,
    },
    pipeline::{BulkInsertPipeline, BulkUpdatePipeline, SimulatedExecutor},
};

// Re-export memory optimization types
pub use mem_optimize::{
    GlobalInterner, IdentifierCache, InternedStr, ScopedInterner,
    arena::{ArenaScope, ArenaStats, QueryArena, ScopedFilter, ScopedQuery, ScopedValue},
    interning::{get_interned, intern as global_intern, intern_component, intern_qualified},
    lazy::{LazyColumn, LazyForeignKey, LazyIndex, LazySchema, LazySchemaStats, LazyTable},
};

// Re-export profiling types
pub use profiling::{
    AllocationRecord, AllocationStats, AllocationTracker, HeapProfiler, HeapReport, HeapStats,
    LeakDetector, LeakReport, LeakSeverity, MemoryProfiler, MemoryReport, MemorySnapshot,
    PotentialLeak, SnapshotDiff, TrackedAllocator, disable_profiling, enable_profiling,
    is_profiling_enabled, with_profiling,
};

// Re-export smallvec for macros
pub use smallvec;

/// Prelude module for convenient imports.
pub mod prelude {
    pub use crate::advanced::{LateralJoin, Returning, RowLock, TableSample};
    pub use crate::cte::{Cte, CteBuilder, WithClause};
    pub use crate::error::{QueryError, QueryResult};
    pub use crate::extension::{Extension, Point, Polygon};
    pub use crate::filter::{Filter, FilterValue, ScalarFilter};
    pub use crate::introspection::{DatabaseSchema, TableInfo, generate_prax_schema};
    pub use crate::json::{JsonFilter, JsonOp, JsonPath};
    pub use crate::nested::{NestedWrite, NestedWriteBuilder, NestedWriteOperations};
    pub use crate::operations::*;
    pub use crate::pagination::{Cursor, CursorDirection, Pagination};
    pub use crate::partition::{Partition, PartitionBuilder, PartitionType, RangeBound};
    pub use crate::procedure::{
        Parameter, ParameterMode, ProcedureCall, ProcedureEngine, ProcedureResult,
    };
    pub use crate::query::QueryBuilder;
    pub use crate::raw::{RawExecuteOperation, RawQueryOperation, Sql};
    pub use crate::raw_query;
    pub use crate::relations::{Include, IncludeSpec, RelationSpec, SelectSpec};
    pub use crate::replication::{ConnectionRouter, ReadPreference, ReplicaSetConfig};
    pub use crate::search::{FullTextIndex, SearchMode, SearchQuery, SearchQueryBuilder};
    pub use crate::security::{Grant, GrantBuilder, RlsPolicy, Role, RoleBuilder};
    pub use crate::sequence::{Sequence, SequenceBuilder};
    pub use crate::traits::{
        Executable, IntoFilter, MaterializedView, Model, QueryEngine, View, ViewQueryEngine,
    };
    pub use crate::transaction::{IsolationLevel, Transaction, TransactionConfig};
    pub use crate::trigger::{
        Trigger, TriggerAction, TriggerBuilder, TriggerCondition, TriggerEvent, TriggerLevel,
        TriggerTiming,
    };
    pub use crate::types::{OrderBy, Select, SortOrder};
    pub use crate::upsert::{ConflictAction, ConflictTarget, Upsert, UpsertBuilder};
    pub use crate::window::{WindowFn, WindowFunction, WindowSpec};

    // Tenant types
    pub use crate::tenant::{IsolationStrategy, TenantConfig, TenantContext, TenantMiddleware};

    // Async optimization types
    pub use crate::async_optimize::{
        ConcurrencyConfig, ConcurrentExecutor, IntrospectionConfig, PipelineConfig, QueryPipeline,
    };

    // Memory optimization types
    pub use crate::mem_optimize::{GlobalInterner, InternedStr, LazySchema, QueryArena};

    // Profiling types
    pub use crate::profiling::{
        LeakDetector, MemoryProfiler, MemorySnapshot, enable_profiling, with_profiling,
    };
}
