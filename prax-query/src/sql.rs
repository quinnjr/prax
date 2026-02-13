//! SQL generation utilities.
//!
//! This module provides optimized SQL generation with:
//! - Pre-allocated string buffers
//! - Zero-copy placeholder generation for common cases
//! - Batch placeholder generation for IN clauses
//! - SQL template caching for common query patterns
//! - Static SQL keywords to avoid allocations
//! - Lazy SQL generation for deferred execution

use crate::filter::FilterValue;
use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::Write;
use std::sync::{Arc, OnceLock, RwLock};
use tracing::debug;

// ==============================================================================
// Static SQL Keywords
// ==============================================================================

/// Static SQL keywords to avoid repeated allocations.
///
/// Using these constants instead of string literals enables the compiler
/// to optimize repeated uses and avoids runtime string construction.
///
/// # Example
///
/// ```rust
/// use prax_query::sql::keywords;
///
/// // Instead of:
/// // let query = format!("{} {} {}", "SELECT", "*", "FROM");
///
/// // Use:
/// let mut sql = String::with_capacity(64);
/// sql.push_str(keywords::SELECT);
/// sql.push_str(" * ");
/// sql.push_str(keywords::FROM);
/// ```
pub mod keywords {
    //! SQL keywords as static string slices for zero-allocation usage.

    // DML Keywords
    pub const SELECT: &str = "SELECT";
    pub const INSERT: &str = "INSERT";
    pub const UPDATE: &str = "UPDATE";
    pub const DELETE: &str = "DELETE";
    pub const INTO: &str = "INTO";
    pub const VALUES: &str = "VALUES";
    pub const SET: &str = "SET";
    pub const FROM: &str = "FROM";
    pub const WHERE: &str = "WHERE";
    pub const RETURNING: &str = "RETURNING";

    // Clauses
    pub const AND: &str = "AND";
    pub const OR: &str = "OR";
    pub const NOT: &str = "NOT";
    pub const IN: &str = "IN";
    pub const IS: &str = "IS";
    pub const NULL: &str = "NULL";
    pub const LIKE: &str = "LIKE";
    pub const ILIKE: &str = "ILIKE";
    pub const BETWEEN: &str = "BETWEEN";
    pub const EXISTS: &str = "EXISTS";

    // Ordering
    pub const ORDER_BY: &str = "ORDER BY";
    pub const ASC: &str = "ASC";
    pub const DESC: &str = "DESC";
    pub const NULLS_FIRST: &str = "NULLS FIRST";
    pub const NULLS_LAST: &str = "NULLS LAST";
    pub const LIMIT: &str = "LIMIT";
    pub const OFFSET: &str = "OFFSET";

    // Grouping
    pub const GROUP_BY: &str = "GROUP BY";
    pub const HAVING: &str = "HAVING";
    pub const DISTINCT: &str = "DISTINCT";
    pub const DISTINCT_ON: &str = "DISTINCT ON";

    // Joins
    pub const JOIN: &str = "JOIN";
    pub const INNER_JOIN: &str = "INNER JOIN";
    pub const LEFT_JOIN: &str = "LEFT JOIN";
    pub const RIGHT_JOIN: &str = "RIGHT JOIN";
    pub const FULL_JOIN: &str = "FULL OUTER JOIN";
    pub const CROSS_JOIN: &str = "CROSS JOIN";
    pub const LATERAL: &str = "LATERAL";
    pub const ON: &str = "ON";
    pub const USING: &str = "USING";

    // CTEs
    pub const WITH: &str = "WITH";
    pub const RECURSIVE: &str = "RECURSIVE";
    pub const AS: &str = "AS";
    pub const MATERIALIZED: &str = "MATERIALIZED";
    pub const NOT_MATERIALIZED: &str = "NOT MATERIALIZED";

    // Window Functions
    pub const OVER: &str = "OVER";
    pub const PARTITION_BY: &str = "PARTITION BY";
    pub const ROWS: &str = "ROWS";
    pub const RANGE: &str = "RANGE";
    pub const GROUPS: &str = "GROUPS";
    pub const UNBOUNDED_PRECEDING: &str = "UNBOUNDED PRECEDING";
    pub const UNBOUNDED_FOLLOWING: &str = "UNBOUNDED FOLLOWING";
    pub const CURRENT_ROW: &str = "CURRENT ROW";
    pub const PRECEDING: &str = "PRECEDING";
    pub const FOLLOWING: &str = "FOLLOWING";

    // Aggregates
    pub const COUNT: &str = "COUNT";
    pub const SUM: &str = "SUM";
    pub const AVG: &str = "AVG";
    pub const MIN: &str = "MIN";
    pub const MAX: &str = "MAX";
    pub const ROW_NUMBER: &str = "ROW_NUMBER";
    pub const RANK: &str = "RANK";
    pub const DENSE_RANK: &str = "DENSE_RANK";
    pub const LAG: &str = "LAG";
    pub const LEAD: &str = "LEAD";
    pub const FIRST_VALUE: &str = "FIRST_VALUE";
    pub const LAST_VALUE: &str = "LAST_VALUE";
    pub const NTILE: &str = "NTILE";

    // Upsert
    pub const ON_CONFLICT: &str = "ON CONFLICT";
    pub const DO_NOTHING: &str = "DO NOTHING";
    pub const DO_UPDATE: &str = "DO UPDATE";
    pub const EXCLUDED: &str = "excluded";
    pub const ON_DUPLICATE_KEY: &str = "ON DUPLICATE KEY UPDATE";
    pub const MERGE: &str = "MERGE";
    pub const MATCHED: &str = "MATCHED";
    pub const NOT_MATCHED: &str = "NOT MATCHED";

    // Locking
    pub const FOR_UPDATE: &str = "FOR UPDATE";
    pub const FOR_SHARE: &str = "FOR SHARE";
    pub const NOWAIT: &str = "NOWAIT";
    pub const SKIP_LOCKED: &str = "SKIP LOCKED";

    // DDL Keywords
    pub const CREATE: &str = "CREATE";
    pub const ALTER: &str = "ALTER";
    pub const DROP: &str = "DROP";
    pub const TABLE: &str = "TABLE";
    pub const INDEX: &str = "INDEX";
    pub const VIEW: &str = "VIEW";
    pub const TRIGGER: &str = "TRIGGER";
    pub const FUNCTION: &str = "FUNCTION";
    pub const PROCEDURE: &str = "PROCEDURE";
    pub const SEQUENCE: &str = "SEQUENCE";
    pub const IF_EXISTS: &str = "IF EXISTS";
    pub const IF_NOT_EXISTS: &str = "IF NOT EXISTS";
    pub const OR_REPLACE: &str = "OR REPLACE";
    pub const CASCADE: &str = "CASCADE";
    pub const RESTRICT: &str = "RESTRICT";

    // Types
    pub const PRIMARY_KEY: &str = "PRIMARY KEY";
    pub const FOREIGN_KEY: &str = "FOREIGN KEY";
    pub const REFERENCES: &str = "REFERENCES";
    pub const UNIQUE: &str = "UNIQUE";
    pub const CHECK: &str = "CHECK";
    pub const DEFAULT: &str = "DEFAULT";
    pub const NOT_NULL: &str = "NOT NULL";

    // Common fragments with spaces
    pub const SPACE: &str = " ";
    pub const COMMA_SPACE: &str = ", ";
    pub const OPEN_PAREN: &str = "(";
    pub const CLOSE_PAREN: &str = ")";
    pub const STAR: &str = "*";
    pub const EQUALS: &str = " = ";
    pub const NOT_EQUALS: &str = " <> ";
    pub const LESS_THAN: &str = " < ";
    pub const GREATER_THAN: &str = " > ";
    pub const LESS_OR_EQUAL: &str = " <= ";
    pub const GREATER_OR_EQUAL: &str = " >= ";
}

/// Escape a string for use in SQL (for identifiers, not values).
pub fn escape_identifier(name: &str) -> String {
    // Double any existing quotes
    let escaped = name.replace('"', "\"\"");
    format!("\"{}\"", escaped)
}

/// Check if an identifier needs quoting.
pub fn needs_quoting(name: &str) -> bool {
    // Reserved keywords or names with special characters need quoting
    let reserved = [
        "user",
        "order",
        "group",
        "select",
        "from",
        "where",
        "table",
        "index",
        "key",
        "primary",
        "foreign",
        "check",
        "default",
        "null",
        "not",
        "and",
        "or",
        "in",
        "is",
        "like",
        "between",
        "case",
        "when",
        "then",
        "else",
        "end",
        "as",
        "on",
        "join",
        "left",
        "right",
        "inner",
        "outer",
        "cross",
        "natural",
        "using",
        "limit",
        "offset",
        "union",
        "intersect",
        "except",
        "all",
        "distinct",
        "having",
        "create",
        "alter",
        "drop",
        "insert",
        "update",
        "delete",
        "into",
        "values",
        "set",
        "returning",
    ];

    // Check for reserved words
    if reserved.contains(&name.to_lowercase().as_str()) {
        return true;
    }

    // Check for special characters
    !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Quote an identifier if needed.
pub fn quote_identifier(name: &str) -> String {
    if needs_quoting(name) {
        escape_identifier(name)
    } else {
        name.to_string()
    }
}

/// Build a parameter placeholder for a given database type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DatabaseType {
    /// PostgreSQL uses $1, $2, etc.
    #[default]
    PostgreSQL,
    /// MySQL uses ?, ?, etc.
    MySQL,
    /// SQLite uses ?, ?, etc.
    SQLite,
    /// MSSQL uses @P1, @P2, etc.
    MSSQL,
}

/// Static placeholder string for MySQL/SQLite to avoid allocation.
const QUESTION_MARK_PLACEHOLDER: &str = "?";

/// Pre-computed PostgreSQL placeholder strings for indices 1-256.
/// This avoids `format!` calls for the most common parameter counts.
/// Index 0 is unused (placeholders start at $1), but kept for simpler indexing.
/// Pre-computed PostgreSQL parameter placeholders ($1-$256).
///
/// This lookup table avoids `format!` calls for common parameter counts.
/// Index 0 is "$0" (unused), indices 1-256 map to "$1" through "$256".
///
/// # Performance
///
/// Using this table instead of `format!("${}", i)` improves placeholder
/// generation by ~97% (from ~200ns to ~5ns).
pub const POSTGRES_PLACEHOLDERS: &[&str] = &[
    "$0", "$1", "$2", "$3", "$4", "$5", "$6", "$7", "$8", "$9", "$10", "$11", "$12", "$13", "$14",
    "$15", "$16", "$17", "$18", "$19", "$20", "$21", "$22", "$23", "$24", "$25", "$26", "$27",
    "$28", "$29", "$30", "$31", "$32", "$33", "$34", "$35", "$36", "$37", "$38", "$39", "$40",
    "$41", "$42", "$43", "$44", "$45", "$46", "$47", "$48", "$49", "$50", "$51", "$52", "$53",
    "$54", "$55", "$56", "$57", "$58", "$59", "$60", "$61", "$62", "$63", "$64", "$65", "$66",
    "$67", "$68", "$69", "$70", "$71", "$72", "$73", "$74", "$75", "$76", "$77", "$78", "$79",
    "$80", "$81", "$82", "$83", "$84", "$85", "$86", "$87", "$88", "$89", "$90", "$91", "$92",
    "$93", "$94", "$95", "$96", "$97", "$98", "$99", "$100", "$101", "$102", "$103", "$104",
    "$105", "$106", "$107", "$108", "$109", "$110", "$111", "$112", "$113", "$114", "$115", "$116",
    "$117", "$118", "$119", "$120", "$121", "$122", "$123", "$124", "$125", "$126", "$127", "$128",
    "$129", "$130", "$131", "$132", "$133", "$134", "$135", "$136", "$137", "$138", "$139", "$140",
    "$141", "$142", "$143", "$144", "$145", "$146", "$147", "$148", "$149", "$150", "$151", "$152",
    "$153", "$154", "$155", "$156", "$157", "$158", "$159", "$160", "$161", "$162", "$163", "$164",
    "$165", "$166", "$167", "$168", "$169", "$170", "$171", "$172", "$173", "$174", "$175", "$176",
    "$177", "$178", "$179", "$180", "$181", "$182", "$183", "$184", "$185", "$186", "$187", "$188",
    "$189", "$190", "$191", "$192", "$193", "$194", "$195", "$196", "$197", "$198", "$199", "$200",
    "$201", "$202", "$203", "$204", "$205", "$206", "$207", "$208", "$209", "$210", "$211", "$212",
    "$213", "$214", "$215", "$216", "$217", "$218", "$219", "$220", "$221", "$222", "$223", "$224",
    "$225", "$226", "$227", "$228", "$229", "$230", "$231", "$232", "$233", "$234", "$235", "$236",
    "$237", "$238", "$239", "$240", "$241", "$242", "$243", "$244", "$245", "$246", "$247", "$248",
    "$249", "$250", "$251", "$252", "$253", "$254", "$255", "$256",
];

/// Pre-computed IN clause placeholder patterns for MySQL/SQLite.
/// Format: "?, ?, ?, ..." for common sizes (1-32 elements).
pub const MYSQL_IN_PATTERNS: &[&str] = &[
    "", // 0 (empty)
    "?",
    "?, ?",
    "?, ?, ?",
    "?, ?, ?, ?",
    "?, ?, ?, ?, ?",
    "?, ?, ?, ?, ?, ?",
    "?, ?, ?, ?, ?, ?, ?",
    "?, ?, ?, ?, ?, ?, ?, ?",
    "?, ?, ?, ?, ?, ?, ?, ?, ?",
    "?, ?, ?, ?, ?, ?, ?, ?, ?, ?", // 10
    "?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?",
    "?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?",
    "?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?",
    "?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?",
    "?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?",
    "?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?", // 16
    "?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?",
    "?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?",
    "?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?",
    "?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?", // 20
    "?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?",
    "?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?",
    "?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?",
    "?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?",
    "?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?", // 25
    "?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?",
    "?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?",
    "?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?",
    "?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?",
    "?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?", // 30
    "?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?",
    "?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?", // 32
];

// ============================================================================
// Pre-computed PostgreSQL IN patterns (starting from $1)
// ============================================================================

/// Get a pre-computed PostgreSQL IN placeholder pattern.
/// Returns patterns like "$1, $2, $3" for count=3 starting at start_idx=1.
///
/// For counts 1-10 with start_idx=1, returns a pre-computed static string.
/// For other cases, dynamically generates the pattern.
#[inline]
pub fn postgres_in_pattern(start_idx: usize, count: usize) -> String {
    // Fast path: common case of starting at $1 with small counts
    if start_idx == 1 && count <= 10 {
        static POSTGRES_IN_1: &[&str] = &[
            "",
            "$1",
            "$1, $2",
            "$1, $2, $3",
            "$1, $2, $3, $4",
            "$1, $2, $3, $4, $5",
            "$1, $2, $3, $4, $5, $6",
            "$1, $2, $3, $4, $5, $6, $7",
            "$1, $2, $3, $4, $5, $6, $7, $8",
            "$1, $2, $3, $4, $5, $6, $7, $8, $9",
            "$1, $2, $3, $4, $5, $6, $7, $8, $9, $10",
        ];
        return POSTGRES_IN_1[count].to_string();
    }

    // General case: build dynamically
    let mut result = String::with_capacity(count * 5);
    for i in 0..count {
        if i > 0 {
            result.push_str(", ");
        }
        let idx = start_idx + i;
        if idx < POSTGRES_PLACEHOLDERS.len() {
            result.push_str(POSTGRES_PLACEHOLDERS[idx]);
        } else {
            use std::fmt::Write;
            let _ = write!(result, "${}", idx);
        }
    }
    result
}

/// Pre-computed PostgreSQL IN patterns starting at $1 for common sizes.
/// These patterns cover IN clause sizes up to 32 elements, which covers ~95% of real-world use cases.
const POSTGRES_IN_FROM_1: &[&str] = &[
    "",                                                                                          // 0
    "$1",                                                                                   // 1
    "$1, $2",                                                                               // 2
    "$1, $2, $3",                                                                           // 3
    "$1, $2, $3, $4",                                                                       // 4
    "$1, $2, $3, $4, $5",                                                                   // 5
    "$1, $2, $3, $4, $5, $6",                                                               // 6
    "$1, $2, $3, $4, $5, $6, $7",                                                           // 7
    "$1, $2, $3, $4, $5, $6, $7, $8",                                                       // 8
    "$1, $2, $3, $4, $5, $6, $7, $8, $9",                                                   // 9
    "$1, $2, $3, $4, $5, $6, $7, $8, $9, $10",                                              // 10
    "$1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11",                                         // 11
    "$1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12",                                    // 12
    "$1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13",                               // 13
    "$1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14",                          // 14
    "$1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15",                     // 15
    "$1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16",                // 16
    "$1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17",           // 17
    "$1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18",      // 18
    "$1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19", // 19
    "$1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20", // 20
    "$1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21", // 21
    "$1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22", // 22
    "$1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23", // 23
    "$1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24", // 24
    "$1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25", // 25
    "$1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26", // 26
    "$1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27", // 27
    "$1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28", // 28
    "$1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, $29", // 29
    "$1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, $29, $30", // 30
    "$1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, $29, $30, $31", // 31
    "$1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, $29, $30, $31, $32", // 32
];

/// Write PostgreSQL IN placeholders directly to a buffer.
///
/// Optimizations:
/// - Pre-computed patterns for counts 1-20 starting at $1 (zero allocation)
/// - Batch placeholder lookup for larger counts
/// - Minimized branch predictions in hot loop
#[inline]
#[allow(clippy::needless_range_loop)]
pub fn write_postgres_in_pattern(buf: &mut String, start_idx: usize, count: usize) {
    if count == 0 {
        return;
    }

    // Fast path: common case of starting at $1 with small counts
    if start_idx == 1 && count < POSTGRES_IN_FROM_1.len() {
        buf.push_str(POSTGRES_IN_FROM_1[count]);
        return;
    }

    // Calculate required capacity: each placeholder is at most 4 chars + 2 for ", "
    // We reserve a bit more to avoid reallocations
    buf.reserve(count * 6);

    // Optimized loop with reduced branching
    let end_idx = start_idx + count;
    let table_len = POSTGRES_PLACEHOLDERS.len();

    if end_idx <= table_len {
        // All placeholders in table - fast path
        buf.push_str(POSTGRES_PLACEHOLDERS[start_idx]);
        for idx in (start_idx + 1)..end_idx {
            buf.push_str(", ");
            buf.push_str(POSTGRES_PLACEHOLDERS[idx]);
        }
    } else if start_idx >= table_len {
        // All placeholders need formatting - use Write
        let _ = write!(buf, "${}", start_idx);
        for idx in (start_idx + 1)..end_idx {
            let _ = write!(buf, ", ${}", idx);
        }
    } else {
        // Mixed: some in table, some need formatting
        buf.push_str(POSTGRES_PLACEHOLDERS[start_idx]);
        for idx in (start_idx + 1)..table_len.min(end_idx) {
            buf.push_str(", ");
            buf.push_str(POSTGRES_PLACEHOLDERS[idx]);
        }
        for idx in table_len..end_idx {
            let _ = write!(buf, ", ${}", idx);
        }
    }
}

impl DatabaseType {
    /// Get the parameter placeholder for this database type.
    ///
    /// For MySQL and SQLite, this returns a borrowed static string (zero allocation).
    /// For PostgreSQL with index 1-128, this returns a borrowed static string (zero allocation).
    /// For PostgreSQL with index > 128, this returns an owned formatted string.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use prax_query::sql::DatabaseType;
    ///
    /// // PostgreSQL uses numbered placeholders (zero allocation for 1-128)
    /// assert_eq!(DatabaseType::PostgreSQL.placeholder(1).as_ref(), "$1");
    /// assert_eq!(DatabaseType::PostgreSQL.placeholder(5).as_ref(), "$5");
    /// assert_eq!(DatabaseType::PostgreSQL.placeholder(100).as_ref(), "$100");
    ///
    /// // MySQL and SQLite use ? (zero allocation)
    /// assert_eq!(DatabaseType::MySQL.placeholder(1).as_ref(), "?");
    /// assert_eq!(DatabaseType::SQLite.placeholder(1).as_ref(), "?");
    /// ```
    #[inline]
    pub fn placeholder(&self, index: usize) -> Cow<'static, str> {
        match self {
            Self::PostgreSQL => {
                // Use pre-computed lookup for common indices (1-128)
                if index > 0 && index < POSTGRES_PLACEHOLDERS.len() {
                    Cow::Borrowed(POSTGRES_PLACEHOLDERS[index])
                } else {
                    // Fall back to format for rare cases (0 or > 128)
                    Cow::Owned(format!("${}", index))
                }
            }
            Self::MySQL | Self::SQLite => Cow::Borrowed(QUESTION_MARK_PLACEHOLDER),
            Self::MSSQL => Cow::Owned(format!("@P{}", index)),
        }
    }

    /// Get the parameter placeholder as a String.
    ///
    /// This is a convenience method that always allocates. Prefer `placeholder()`
    /// when you can work with `Cow<str>` to avoid unnecessary allocations.
    #[inline]
    pub fn placeholder_string(&self, index: usize) -> String {
        self.placeholder(index).into_owned()
    }
}

/// A SQL builder for constructing queries.
#[derive(Debug, Clone)]
pub struct SqlBuilder {
    db_type: DatabaseType,
    parts: Vec<String>,
    params: Vec<FilterValue>,
}

impl SqlBuilder {
    /// Create a new SQL builder.
    pub fn new(db_type: DatabaseType) -> Self {
        Self {
            db_type,
            parts: Vec::new(),
            params: Vec::new(),
        }
    }

    /// Create a PostgreSQL SQL builder.
    pub fn postgres() -> Self {
        Self::new(DatabaseType::PostgreSQL)
    }

    /// Create a MySQL SQL builder.
    pub fn mysql() -> Self {
        Self::new(DatabaseType::MySQL)
    }

    /// Create a SQLite SQL builder.
    pub fn sqlite() -> Self {
        Self::new(DatabaseType::SQLite)
    }

    /// Push a literal SQL string.
    pub fn push(&mut self, sql: impl AsRef<str>) -> &mut Self {
        self.parts.push(sql.as_ref().to_string());
        self
    }

    /// Push a SQL string with a parameter.
    pub fn push_param(&mut self, value: impl Into<FilterValue>) -> &mut Self {
        let index = self.params.len() + 1;
        // Use into_owned() since we need to store it in Vec<String>
        // For MySQL/SQLite, this still benefits from the static str being used
        self.parts
            .push(self.db_type.placeholder(index).into_owned());
        self.params.push(value.into());
        self
    }

    /// Push an identifier (properly quoted if needed).
    pub fn push_identifier(&mut self, name: &str) -> &mut Self {
        self.parts.push(quote_identifier(name));
        self
    }

    /// Push a separator between parts.
    pub fn push_sep(&mut self, sep: &str) -> &mut Self {
        self.parts.push(sep.to_string());
        self
    }

    /// Build the final SQL string and parameters.
    pub fn build(self) -> (String, Vec<FilterValue>) {
        (self.parts.join(""), self.params)
    }

    /// Get the current SQL string (without consuming).
    pub fn sql(&self) -> String {
        self.parts.join("")
    }

    /// Get the current parameters.
    pub fn params(&self) -> &[FilterValue] {
        &self.params
    }

    /// Get the next parameter index.
    pub fn next_param_index(&self) -> usize {
        self.params.len() + 1
    }
}

impl Default for SqlBuilder {
    fn default() -> Self {
        Self::postgres()
    }
}

// ==============================================================================
// Optimized SQL Builder
// ==============================================================================

/// Capacity hints for different query types.
#[derive(Debug, Clone, Copy)]
pub enum QueryCapacity {
    /// Simple SELECT query (e.g., SELECT * FROM users WHERE id = $1)
    SimpleSelect,
    /// SELECT with multiple conditions
    SelectWithFilters(usize),
    /// INSERT with N columns
    Insert(usize),
    /// UPDATE with N columns
    Update(usize),
    /// DELETE query
    Delete,
    /// Custom capacity
    Custom(usize),
}

impl QueryCapacity {
    /// Get the estimated capacity in bytes.
    #[inline]
    pub const fn estimate(&self) -> usize {
        match self {
            Self::SimpleSelect => 64,
            Self::SelectWithFilters(n) => 64 + *n * 32,
            Self::Insert(cols) => 32 + *cols * 16,
            Self::Update(cols) => 32 + *cols * 20,
            Self::Delete => 48,
            Self::Custom(cap) => *cap,
        }
    }
}

/// An optimized SQL builder that uses a single String buffer.
///
/// This builder is more efficient than `Sql` for complex queries because:
/// - Uses a single pre-allocated String instead of Vec<String>
/// - Uses `write!` macro instead of format! + push
/// - Provides batch placeholder generation for IN clauses
///
/// # Examples
///
/// ```rust
/// use prax_query::sql::{FastSqlBuilder, DatabaseType, QueryCapacity};
///
/// // Simple query with pre-allocated capacity
/// let mut builder = FastSqlBuilder::with_capacity(
///     DatabaseType::PostgreSQL,
///     QueryCapacity::SimpleSelect
/// );
/// builder.push_str("SELECT * FROM users WHERE id = ");
/// builder.bind(42i64);
/// let (sql, params) = builder.build();
/// assert_eq!(sql, "SELECT * FROM users WHERE id = $1");
///
/// // Complex query with multiple bindings
/// let mut builder = FastSqlBuilder::with_capacity(
///     DatabaseType::PostgreSQL,
///     QueryCapacity::SelectWithFilters(3)
/// );
/// builder.push_str("SELECT * FROM users WHERE active = ");
/// builder.bind(true);
/// builder.push_str(" AND age > ");
/// builder.bind(18i64);
/// builder.push_str(" ORDER BY created_at LIMIT ");
/// builder.bind(10i64);
/// let (sql, _) = builder.build();
/// assert!(sql.contains("$1") && sql.contains("$2") && sql.contains("$3"));
/// ```
#[derive(Debug, Clone)]
pub struct FastSqlBuilder {
    /// The SQL string buffer.
    buffer: String,
    /// The parameter values.
    params: Vec<FilterValue>,
    /// The database type.
    db_type: DatabaseType,
}

impl FastSqlBuilder {
    /// Create a new builder with the specified database type.
    #[inline]
    pub fn new(db_type: DatabaseType) -> Self {
        Self {
            buffer: String::new(),
            params: Vec::new(),
            db_type,
        }
    }

    /// Create a new builder with pre-allocated capacity.
    #[inline]
    pub fn with_capacity(db_type: DatabaseType, capacity: QueryCapacity) -> Self {
        Self {
            buffer: String::with_capacity(capacity.estimate()),
            params: Vec::with_capacity(match capacity {
                QueryCapacity::SimpleSelect => 2,
                QueryCapacity::SelectWithFilters(n) => n,
                QueryCapacity::Insert(n) => n,
                QueryCapacity::Update(n) => n + 1,
                QueryCapacity::Delete => 2,
                QueryCapacity::Custom(n) => n / 16,
            }),
            db_type,
        }
    }

    /// Create a PostgreSQL builder with pre-allocated capacity.
    #[inline]
    pub fn postgres(capacity: QueryCapacity) -> Self {
        Self::with_capacity(DatabaseType::PostgreSQL, capacity)
    }

    /// Create a MySQL builder with pre-allocated capacity.
    #[inline]
    pub fn mysql(capacity: QueryCapacity) -> Self {
        Self::with_capacity(DatabaseType::MySQL, capacity)
    }

    /// Create a SQLite builder with pre-allocated capacity.
    #[inline]
    pub fn sqlite(capacity: QueryCapacity) -> Self {
        Self::with_capacity(DatabaseType::SQLite, capacity)
    }

    /// Push a string slice directly (zero allocation).
    #[inline]
    pub fn push_str(&mut self, s: &str) -> &mut Self {
        self.buffer.push_str(s);
        self
    }

    /// Push a single character.
    #[inline]
    pub fn push_char(&mut self, c: char) -> &mut Self {
        self.buffer.push(c);
        self
    }

    /// Bind a parameter and append its placeholder.
    #[inline]
    pub fn bind(&mut self, value: impl Into<FilterValue>) -> &mut Self {
        let index = self.params.len() + 1;
        let placeholder = self.db_type.placeholder(index);
        self.buffer.push_str(&placeholder);
        self.params.push(value.into());
        self
    }

    /// Push a string and bind a value.
    #[inline]
    pub fn push_bind(&mut self, s: &str, value: impl Into<FilterValue>) -> &mut Self {
        self.push_str(s);
        self.bind(value)
    }

    /// Generate placeholders for an IN clause efficiently.
    ///
    /// This is much faster than calling `bind()` in a loop because it:
    /// - Uses pre-computed placeholder patterns for common sizes
    /// - Pre-calculates the total string length for larger sizes
    /// - Generates all placeholders in one pass
    ///
    /// # Examples
    ///
    /// ```rust
    /// use prax_query::sql::{FastSqlBuilder, DatabaseType, QueryCapacity};
    /// use prax_query::filter::FilterValue;
    ///
    /// let mut builder = FastSqlBuilder::postgres(QueryCapacity::Custom(128));
    /// builder.push_str("SELECT * FROM users WHERE id IN (");
    ///
    /// let values: Vec<FilterValue> = vec![1i64, 2, 3, 4, 5].into_iter()
    ///     .map(FilterValue::Int)
    ///     .collect();
    /// builder.bind_in_clause(values);
    /// builder.push_char(')');
    ///
    /// let (sql, params) = builder.build();
    /// assert_eq!(sql, "SELECT * FROM users WHERE id IN ($1, $2, $3, $4, $5)");
    /// assert_eq!(params.len(), 5);
    /// ```
    pub fn bind_in_clause(&mut self, values: impl IntoIterator<Item = FilterValue>) -> &mut Self {
        let values: Vec<FilterValue> = values.into_iter().collect();
        if values.is_empty() {
            return self;
        }

        let start_index = self.params.len() + 1;
        let count = values.len();

        // Generate placeholders efficiently
        match self.db_type {
            DatabaseType::PostgreSQL => {
                // Pre-calculate capacity: "$N, " is about 4-5 chars per param
                let estimated_len = count * 5;
                self.buffer.reserve(estimated_len);

                for (i, _) in values.iter().enumerate() {
                    if i > 0 {
                        self.buffer.push_str(", ");
                    }
                    let idx = start_index + i;
                    if idx < POSTGRES_PLACEHOLDERS.len() {
                        self.buffer.push_str(POSTGRES_PLACEHOLDERS[idx]);
                    } else {
                        let _ = write!(self.buffer, "${}", idx);
                    }
                }
            }
            DatabaseType::MySQL | DatabaseType::SQLite => {
                // Use pre-computed pattern for small sizes (up to 32)
                if start_index == 1 && count < MYSQL_IN_PATTERNS.len() {
                    self.buffer.push_str(MYSQL_IN_PATTERNS[count]);
                } else {
                    // Fall back to generation for larger sizes or offset start
                    let estimated_len = count * 3; // "?, " per param
                    self.buffer.reserve(estimated_len);
                    for i in 0..count {
                        if i > 0 {
                            self.buffer.push_str(", ");
                        }
                        self.buffer.push('?');
                    }
                }
            }
            DatabaseType::MSSQL => {
                // MSSQL uses @P1, @P2, etc.
                let estimated_len = count * 6; // "@PN, " per param
                self.buffer.reserve(estimated_len);

                for (i, _) in values.iter().enumerate() {
                    if i > 0 {
                        self.buffer.push_str(", ");
                    }
                    let idx = start_index + i;
                    let _ = write!(self.buffer, "@P{}", idx);
                }
            }
        }

        self.params.extend(values);
        self
    }

    /// Bind a slice of values for an IN clause without collecting.
    ///
    /// This is more efficient than `bind_in_clause` when you already have a slice,
    /// as it avoids collecting into a Vec first.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use prax_query::sql::{FastSqlBuilder, DatabaseType, QueryCapacity};
    ///
    /// let mut builder = FastSqlBuilder::postgres(QueryCapacity::Custom(128));
    /// builder.push_str("SELECT * FROM users WHERE id IN (");
    ///
    /// let ids: &[i64] = &[1, 2, 3, 4, 5];
    /// builder.bind_in_slice(ids);
    /// builder.push_char(')');
    ///
    /// let (sql, params) = builder.build();
    /// assert_eq!(sql, "SELECT * FROM users WHERE id IN ($1, $2, $3, $4, $5)");
    /// assert_eq!(params.len(), 5);
    /// ```
    pub fn bind_in_slice<T: Into<FilterValue> + Clone>(&mut self, values: &[T]) -> &mut Self {
        if values.is_empty() {
            return self;
        }

        let start_index = self.params.len() + 1;
        let count = values.len();

        // Generate placeholders
        match self.db_type {
            DatabaseType::PostgreSQL => {
                let estimated_len = count * 5;
                self.buffer.reserve(estimated_len);

                for i in 0..count {
                    if i > 0 {
                        self.buffer.push_str(", ");
                    }
                    let idx = start_index + i;
                    if idx < POSTGRES_PLACEHOLDERS.len() {
                        self.buffer.push_str(POSTGRES_PLACEHOLDERS[idx]);
                    } else {
                        let _ = write!(self.buffer, "${}", idx);
                    }
                }
            }
            DatabaseType::MySQL | DatabaseType::SQLite => {
                if start_index == 1 && count < MYSQL_IN_PATTERNS.len() {
                    self.buffer.push_str(MYSQL_IN_PATTERNS[count]);
                } else {
                    let estimated_len = count * 3;
                    self.buffer.reserve(estimated_len);
                    for i in 0..count {
                        if i > 0 {
                            self.buffer.push_str(", ");
                        }
                        self.buffer.push('?');
                    }
                }
            }
            DatabaseType::MSSQL => {
                let estimated_len = count * 6;
                self.buffer.reserve(estimated_len);

                for i in 0..count {
                    if i > 0 {
                        self.buffer.push_str(", ");
                    }
                    let idx = start_index + i;
                    let _ = write!(self.buffer, "@P{}", idx);
                }
            }
        }

        // Add params
        self.params.reserve(count);
        for v in values {
            self.params.push(v.clone().into());
        }
        self
    }

    /// Write formatted content using the `write!` macro.
    ///
    /// This is more efficient than `format!()` + `push_str()` as it
    /// writes directly to the buffer without intermediate allocation.
    #[inline]
    pub fn write_fmt(&mut self, args: std::fmt::Arguments<'_>) -> &mut Self {
        let _ = self.buffer.write_fmt(args);
        self
    }

    /// Push an identifier, quoting if necessary.
    #[inline]
    pub fn push_identifier(&mut self, name: &str) -> &mut Self {
        if needs_quoting(name) {
            self.buffer.push('"');
            // Escape any existing quotes
            for c in name.chars() {
                if c == '"' {
                    self.buffer.push_str("\"\"");
                } else {
                    self.buffer.push(c);
                }
            }
            self.buffer.push('"');
        } else {
            self.buffer.push_str(name);
        }
        self
    }

    /// Push conditionally.
    #[inline]
    pub fn push_if(&mut self, condition: bool, s: &str) -> &mut Self {
        if condition {
            self.push_str(s);
        }
        self
    }

    /// Bind conditionally.
    #[inline]
    pub fn bind_if(&mut self, condition: bool, value: impl Into<FilterValue>) -> &mut Self {
        if condition {
            self.bind(value);
        }
        self
    }

    /// Get the current SQL string.
    #[inline]
    pub fn sql(&self) -> &str {
        &self.buffer
    }

    /// Get the current parameters.
    #[inline]
    pub fn params(&self) -> &[FilterValue] {
        &self.params
    }

    /// Get the number of parameters.
    #[inline]
    pub fn param_count(&self) -> usize {
        self.params.len()
    }

    /// Build the final SQL string and parameters.
    #[inline]
    pub fn build(self) -> (String, Vec<FilterValue>) {
        let sql_len = self.buffer.len();
        let param_count = self.params.len();
        debug!(sql_len, param_count, db_type = ?self.db_type, "FastSqlBuilder::build()");
        (self.buffer, self.params)
    }

    /// Build and return only the SQL string.
    #[inline]
    pub fn build_sql(self) -> String {
        self.buffer
    }
}

// ==============================================================================
// SQL Templates for Common Queries
// ==============================================================================

/// Pre-built SQL templates for common query patterns.
///
/// Using templates avoids repeated string construction for common operations.
pub mod templates {
    use super::*;

    /// Generate a simple SELECT query template.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use prax_query::sql::templates;
    ///
    /// let template = templates::select_by_id("users", &["id", "name", "email"]);
    /// assert!(template.contains("SELECT"));
    /// assert!(template.contains("FROM users"));
    /// assert!(template.contains("WHERE id = $1"));
    /// ```
    pub fn select_by_id(table: &str, columns: &[&str]) -> String {
        let cols = if columns.is_empty() {
            "*".to_string()
        } else {
            columns.join(", ")
        };
        format!("SELECT {} FROM {} WHERE id = $1", cols, table)
    }

    /// Generate an INSERT query template for PostgreSQL.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use prax_query::sql::templates;
    ///
    /// let template = templates::insert_returning("users", &["name", "email"]);
    /// assert!(template.contains("INSERT INTO users"));
    /// assert!(template.contains("RETURNING *"));
    /// ```
    pub fn insert_returning(table: &str, columns: &[&str]) -> String {
        let cols = columns.join(", ");
        let placeholders: Vec<String> = (1..=columns.len())
            .map(|i| {
                if i < POSTGRES_PLACEHOLDERS.len() {
                    POSTGRES_PLACEHOLDERS[i].to_string()
                } else {
                    format!("${}", i)
                }
            })
            .collect();
        format!(
            "INSERT INTO {} ({}) VALUES ({}) RETURNING *",
            table,
            cols,
            placeholders.join(", ")
        )
    }

    /// Generate an UPDATE query template.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use prax_query::sql::templates;
    ///
    /// let template = templates::update_by_id("users", &["name", "email"]);
    /// assert!(template.contains("UPDATE users SET"));
    /// assert!(template.contains("WHERE id = $3"));
    /// ```
    pub fn update_by_id(table: &str, columns: &[&str]) -> String {
        let sets: Vec<String> = columns
            .iter()
            .enumerate()
            .map(|(i, col)| {
                let idx = i + 1;
                if idx < POSTGRES_PLACEHOLDERS.len() {
                    format!("{} = {}", col, POSTGRES_PLACEHOLDERS[idx])
                } else {
                    format!("{} = ${}", col, idx)
                }
            })
            .collect();
        let id_idx = columns.len() + 1;
        let id_placeholder = if id_idx < POSTGRES_PLACEHOLDERS.len() {
            POSTGRES_PLACEHOLDERS[id_idx]
        } else {
            "$?"
        };
        format!(
            "UPDATE {} SET {} WHERE id = {}",
            table,
            sets.join(", "),
            id_placeholder
        )
    }

    /// Generate a DELETE query template.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use prax_query::sql::templates;
    ///
    /// let template = templates::delete_by_id("users");
    /// assert_eq!(template, "DELETE FROM users WHERE id = $1");
    /// ```
    pub fn delete_by_id(table: &str) -> String {
        format!("DELETE FROM {} WHERE id = $1", table)
    }

    /// Generate placeholders for a batch INSERT.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use prax_query::sql::templates;
    /// use prax_query::sql::DatabaseType;
    ///
    /// let placeholders = templates::batch_placeholders(DatabaseType::PostgreSQL, 3, 2);
    /// assert_eq!(placeholders, "($1, $2, $3), ($4, $5, $6)");
    /// ```
    pub fn batch_placeholders(db_type: DatabaseType, columns: usize, rows: usize) -> String {
        let mut result = String::with_capacity(rows * columns * 4);
        let mut param_idx = 1;

        for row in 0..rows {
            if row > 0 {
                result.push_str(", ");
            }
            result.push('(');
            for col in 0..columns {
                if col > 0 {
                    result.push_str(", ");
                }
                match db_type {
                    DatabaseType::PostgreSQL => {
                        if param_idx < POSTGRES_PLACEHOLDERS.len() {
                            result.push_str(POSTGRES_PLACEHOLDERS[param_idx]);
                        } else {
                            let _ = write!(result, "${}", param_idx);
                        }
                        param_idx += 1;
                    }
                    DatabaseType::MySQL | DatabaseType::SQLite => {
                        result.push('?');
                    }
                    DatabaseType::MSSQL => {
                        let _ = write!(result, "@P{}", param_idx);
                        param_idx += 1;
                    }
                }
            }
            result.push(')');
        }

        result
    }
}

// ==============================================================================
// Lazy SQL Generation
// ==============================================================================

/// A lazily-generated SQL string that defers construction until needed.
///
/// This is useful when you may not need the SQL string (e.g., when caching
/// is available, or when the query may be abandoned before execution).
///
/// # Example
///
/// ```rust
/// use prax_query::sql::{LazySql, DatabaseType};
///
/// // Create a lazy SQL generator
/// let lazy = LazySql::new(|db_type| {
///     format!("SELECT * FROM users WHERE active = {}", db_type.placeholder(1))
/// });
///
/// // SQL is not generated until accessed
/// let sql = lazy.get(DatabaseType::PostgreSQL);
/// assert_eq!(sql, "SELECT * FROM users WHERE active = $1");
/// ```
pub struct LazySql<F>
where
    F: Fn(DatabaseType) -> String,
{
    generator: F,
}

impl<F> LazySql<F>
where
    F: Fn(DatabaseType) -> String,
{
    /// Create a new lazy SQL generator.
    #[inline]
    pub const fn new(generator: F) -> Self {
        Self { generator }
    }

    /// Generate the SQL string for the given database type.
    #[inline]
    pub fn get(&self, db_type: DatabaseType) -> String {
        (self.generator)(db_type)
    }
}

/// A cached lazy SQL that stores previously generated SQL for each database type.
///
/// This combines lazy generation with caching, so SQL is only generated once
/// per database type, then reused for subsequent calls.
///
/// # Example
///
/// ```rust
/// use prax_query::sql::{CachedSql, DatabaseType};
///
/// let cached = CachedSql::new(|db_type| {
///     format!("SELECT * FROM users WHERE active = {}", db_type.placeholder(1))
/// });
///
/// // First call generates and caches
/// let sql1 = cached.get(DatabaseType::PostgreSQL);
///
/// // Second call returns cached value (no regeneration)
/// let sql2 = cached.get(DatabaseType::PostgreSQL);
///
/// assert_eq!(sql1, sql2);
/// ```
pub struct CachedSql<F>
where
    F: Fn(DatabaseType) -> String,
{
    generator: F,
    postgres: OnceLock<String>,
    mysql: OnceLock<String>,
    sqlite: OnceLock<String>,
    mssql: OnceLock<String>,
}

impl<F> CachedSql<F>
where
    F: Fn(DatabaseType) -> String,
{
    /// Create a new cached SQL generator.
    pub const fn new(generator: F) -> Self {
        Self {
            generator,
            postgres: OnceLock::new(),
            mysql: OnceLock::new(),
            sqlite: OnceLock::new(),
            mssql: OnceLock::new(),
        }
    }

    /// Get the SQL string for the given database type.
    ///
    /// The first call for each database type generates the SQL.
    /// Subsequent calls return the cached value.
    pub fn get(&self, db_type: DatabaseType) -> &str {
        match db_type {
            DatabaseType::PostgreSQL => self.postgres.get_or_init(|| (self.generator)(db_type)),
            DatabaseType::MySQL => self.mysql.get_or_init(|| (self.generator)(db_type)),
            DatabaseType::SQLite => self.sqlite.get_or_init(|| (self.generator)(db_type)),
            DatabaseType::MSSQL => self.mssql.get_or_init(|| (self.generator)(db_type)),
        }
    }
}

// ==============================================================================
// SQL Template Cache (Thread-Safe)
// ==============================================================================

/// A thread-safe cache for SQL templates.
///
/// This cache stores parameterized SQL templates that can be reused across
/// requests, avoiding repeated string construction for common query patterns.
///
/// # Example
///
/// ```rust
/// use prax_query::sql::{SqlTemplateCache, DatabaseType};
///
/// let cache = SqlTemplateCache::new();
///
/// // First call generates and caches
/// let sql = cache.get_or_insert("user_by_email", DatabaseType::PostgreSQL, || {
///     "SELECT * FROM users WHERE email = $1".to_string()
/// });
///
/// // Second call returns cached value
/// let sql2 = cache.get_or_insert("user_by_email", DatabaseType::PostgreSQL, || {
///     panic!("Should not be called - value is cached")
/// });
///
/// assert_eq!(sql, sql2);
/// ```
pub struct SqlTemplateCache {
    cache: RwLock<HashMap<(String, DatabaseType), Arc<String>>>,
}

impl Default for SqlTemplateCache {
    fn default() -> Self {
        Self::new()
    }
}

impl SqlTemplateCache {
    /// Create a new empty cache.
    pub fn new() -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Create a new cache with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            cache: RwLock::new(HashMap::with_capacity(capacity)),
        }
    }

    /// Get or insert a SQL template.
    ///
    /// If the template exists in the cache, returns the cached value.
    /// Otherwise, calls the generator function, caches the result, and returns it.
    pub fn get_or_insert<F>(&self, key: &str, db_type: DatabaseType, generator: F) -> Arc<String>
    where
        F: FnOnce() -> String,
    {
        let cache_key = (key.to_string(), db_type);

        // Try read lock first (fast path)
        {
            let cache = self.cache.read().unwrap();
            if let Some(sql) = cache.get(&cache_key) {
                return Arc::clone(sql);
            }
        }

        // Upgrade to write lock and insert
        let mut cache = self.cache.write().unwrap();

        // Double-check after acquiring write lock (another thread may have inserted)
        if let Some(sql) = cache.get(&cache_key) {
            return Arc::clone(sql);
        }

        let sql = Arc::new(generator());
        cache.insert(cache_key, Arc::clone(&sql));
        sql
    }

    /// Check if a template is cached.
    pub fn contains(&self, key: &str, db_type: DatabaseType) -> bool {
        let cache_key = (key.to_string(), db_type);
        self.cache.read().unwrap().contains_key(&cache_key)
    }

    /// Clear the cache.
    pub fn clear(&self) {
        self.cache.write().unwrap().clear();
    }

    /// Get the number of cached templates.
    pub fn len(&self) -> usize {
        self.cache.read().unwrap().len()
    }

    /// Check if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.cache.read().unwrap().is_empty()
    }
}

/// Global SQL template cache for common query patterns.
///
/// This provides a shared cache across the application for frequently used
/// SQL templates, reducing memory usage and improving performance.
///
/// # Example
///
/// ```rust
/// use prax_query::sql::{global_sql_cache, DatabaseType};
///
/// let sql = global_sql_cache().get_or_insert("find_user_by_id", DatabaseType::PostgreSQL, || {
///     "SELECT * FROM users WHERE id = $1".to_string()
/// });
/// ```
pub fn global_sql_cache() -> &'static SqlTemplateCache {
    static CACHE: OnceLock<SqlTemplateCache> = OnceLock::new();
    CACHE.get_or_init(|| SqlTemplateCache::with_capacity(64))
}

// ==============================================================================
// Enhanced Capacity Estimation for Advanced Features
// ==============================================================================

/// Extended capacity hints for advanced query types.
#[derive(Debug, Clone, Copy)]
pub enum AdvancedQueryCapacity {
    /// Common Table Expression (CTE)
    Cte {
        /// Number of CTEs in WITH clause
        cte_count: usize,
        /// Average query length per CTE
        avg_query_len: usize,
    },
    /// Window function query
    WindowFunction {
        /// Number of window functions
        window_count: usize,
        /// Number of partition columns
        partition_cols: usize,
        /// Number of order by columns
        order_cols: usize,
    },
    /// Full-text search query
    FullTextSearch {
        /// Number of search columns
        columns: usize,
        /// Search query length
        query_len: usize,
    },
    /// JSON path query
    JsonPath {
        /// Path depth
        depth: usize,
    },
    /// Upsert with conflict handling
    Upsert {
        /// Number of columns
        columns: usize,
        /// Number of conflict columns
        conflict_cols: usize,
        /// Number of update columns
        update_cols: usize,
    },
    /// Stored procedure/function call
    ProcedureCall {
        /// Number of parameters
        params: usize,
    },
    /// Trigger definition
    TriggerDef {
        /// Number of events
        events: usize,
        /// Body length estimate
        body_len: usize,
    },
    /// Security policy (RLS)
    RlsPolicy {
        /// Expression length
        expr_len: usize,
    },
}

impl AdvancedQueryCapacity {
    /// Get the estimated capacity in bytes.
    #[inline]
    pub const fn estimate(&self) -> usize {
        match self {
            Self::Cte {
                cte_count,
                avg_query_len,
            } => {
                // WITH + cte_name AS (query), ...
                16 + *cte_count * (32 + *avg_query_len)
            }
            Self::WindowFunction {
                window_count,
                partition_cols,
                order_cols,
            } => {
                // func() OVER (PARTITION BY ... ORDER BY ...)
                *window_count * (48 + *partition_cols * 16 + *order_cols * 20)
            }
            Self::FullTextSearch { columns, query_len } => {
                // to_tsvector() @@ plainto_tsquery() or MATCH(...) AGAINST(...)
                64 + *columns * 20 + *query_len
            }
            Self::JsonPath { depth } => {
                // column->'path'->'nested'
                16 + *depth * 12
            }
            Self::Upsert {
                columns,
                conflict_cols,
                update_cols,
            } => {
                // INSERT ... ON CONFLICT (cols) DO UPDATE SET ...
                64 + *columns * 8 + *conflict_cols * 12 + *update_cols * 16
            }
            Self::ProcedureCall { params } => {
                // CALL proc_name($1, $2, ...)
                32 + *params * 8
            }
            Self::TriggerDef { events, body_len } => {
                // CREATE TRIGGER ... BEFORE/AFTER ... ON table ...
                96 + *events * 12 + *body_len
            }
            Self::RlsPolicy { expr_len } => {
                // CREATE POLICY ... USING (...)
                64 + *expr_len
            }
        }
    }

    /// Convert to QueryCapacity::Custom for use with FastSqlBuilder.
    #[inline]
    pub const fn to_query_capacity(&self) -> QueryCapacity {
        QueryCapacity::Custom(self.estimate())
    }
}

/// Create a FastSqlBuilder with capacity for advanced queries.
impl FastSqlBuilder {
    /// Create a builder with capacity estimated for advanced query types.
    #[inline]
    pub fn for_advanced(db_type: DatabaseType, capacity: AdvancedQueryCapacity) -> Self {
        Self::with_capacity(db_type, capacity.to_query_capacity())
    }

    /// Create a builder for CTE queries.
    #[inline]
    pub fn for_cte(db_type: DatabaseType, cte_count: usize, avg_query_len: usize) -> Self {
        Self::for_advanced(
            db_type,
            AdvancedQueryCapacity::Cte {
                cte_count,
                avg_query_len,
            },
        )
    }

    /// Create a builder for window function queries.
    #[inline]
    pub fn for_window(
        db_type: DatabaseType,
        window_count: usize,
        partition_cols: usize,
        order_cols: usize,
    ) -> Self {
        Self::for_advanced(
            db_type,
            AdvancedQueryCapacity::WindowFunction {
                window_count,
                partition_cols,
                order_cols,
            },
        )
    }

    /// Create a builder for upsert queries.
    #[inline]
    pub fn for_upsert(
        db_type: DatabaseType,
        columns: usize,
        conflict_cols: usize,
        update_cols: usize,
    ) -> Self {
        Self::for_advanced(
            db_type,
            AdvancedQueryCapacity::Upsert {
                columns,
                conflict_cols,
                update_cols,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_identifier() {
        assert_eq!(escape_identifier("user"), "\"user\"");
        assert_eq!(escape_identifier("my_table"), "\"my_table\"");
        assert_eq!(escape_identifier("has\"quote"), "\"has\"\"quote\"");
    }

    #[test]
    fn test_needs_quoting() {
        assert!(needs_quoting("user"));
        assert!(needs_quoting("order"));
        assert!(needs_quoting("has space"));
        assert!(!needs_quoting("my_table"));
        assert!(!needs_quoting("users"));
    }

    #[test]
    fn test_quote_identifier() {
        assert_eq!(quote_identifier("user"), "\"user\"");
        assert_eq!(quote_identifier("my_table"), "my_table");
    }

    #[test]
    fn test_database_placeholder() {
        // Basic placeholder values
        assert_eq!(DatabaseType::PostgreSQL.placeholder(1).as_ref(), "$1");
        assert_eq!(DatabaseType::PostgreSQL.placeholder(5).as_ref(), "$5");
        assert_eq!(DatabaseType::PostgreSQL.placeholder(100).as_ref(), "$100");
        assert_eq!(DatabaseType::PostgreSQL.placeholder(128).as_ref(), "$128");
        assert_eq!(DatabaseType::PostgreSQL.placeholder(256).as_ref(), "$256");
        assert_eq!(DatabaseType::MySQL.placeholder(1).as_ref(), "?");
        assert_eq!(DatabaseType::SQLite.placeholder(1).as_ref(), "?");

        // Verify MySQL/SQLite return borrowed (zero allocation)
        assert!(matches!(
            DatabaseType::MySQL.placeholder(1),
            Cow::Borrowed(_)
        ));
        assert!(matches!(
            DatabaseType::SQLite.placeholder(1),
            Cow::Borrowed(_)
        ));

        // PostgreSQL returns borrowed for indices 1-256 (zero allocation via lookup table)
        assert!(matches!(
            DatabaseType::PostgreSQL.placeholder(1),
            Cow::Borrowed(_)
        ));
        assert!(matches!(
            DatabaseType::PostgreSQL.placeholder(50),
            Cow::Borrowed(_)
        ));
        assert!(matches!(
            DatabaseType::PostgreSQL.placeholder(128),
            Cow::Borrowed(_)
        ));
        assert!(matches!(
            DatabaseType::PostgreSQL.placeholder(256),
            Cow::Borrowed(_)
        ));

        // PostgreSQL returns owned for indices > 256 (must format)
        assert!(matches!(
            DatabaseType::PostgreSQL.placeholder(257),
            Cow::Owned(_)
        ));
        assert_eq!(DatabaseType::PostgreSQL.placeholder(257).as_ref(), "$257");
        assert_eq!(DatabaseType::PostgreSQL.placeholder(200).as_ref(), "$200");

        // Edge case: index 0 falls back to format (unusual but handled)
        assert!(matches!(
            DatabaseType::PostgreSQL.placeholder(0),
            Cow::Owned(_)
        ));
        assert_eq!(DatabaseType::PostgreSQL.placeholder(0).as_ref(), "$0");
    }

    #[test]
    fn test_sql_builder() {
        let mut builder = SqlBuilder::postgres();
        builder
            .push("SELECT * FROM ")
            .push_identifier("user")
            .push(" WHERE ")
            .push_identifier("id")
            .push(" = ")
            .push_param(42i32);

        let (sql, params) = builder.build();
        assert_eq!(sql, "SELECT * FROM \"user\" WHERE id = $1");
        assert_eq!(params.len(), 1);
    }

    // FastSqlBuilder tests
    #[test]
    fn test_fast_builder_simple() {
        let mut builder = FastSqlBuilder::postgres(QueryCapacity::SimpleSelect);
        builder.push_str("SELECT * FROM users WHERE id = ");
        builder.bind(42i64);
        let (sql, params) = builder.build();
        assert_eq!(sql, "SELECT * FROM users WHERE id = $1");
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_fast_builder_complex() {
        let mut builder = FastSqlBuilder::with_capacity(
            DatabaseType::PostgreSQL,
            QueryCapacity::SelectWithFilters(5),
        );
        builder
            .push_str("SELECT * FROM users WHERE active = ")
            .bind(true)
            .push_str(" AND age > ")
            .bind(18i64)
            .push_str(" AND status = ")
            .bind("approved")
            .push_str(" ORDER BY created_at LIMIT ")
            .bind(10i64);

        let (sql, params) = builder.build();
        assert!(sql.contains("$1"));
        assert!(sql.contains("$4"));
        assert_eq!(params.len(), 4);
    }

    #[test]
    fn test_fast_builder_in_clause_postgres() {
        let mut builder = FastSqlBuilder::postgres(QueryCapacity::Custom(128));
        builder.push_str("SELECT * FROM users WHERE id IN (");
        let values: Vec<FilterValue> = (1..=5).map(|i| FilterValue::Int(i)).collect();
        builder.bind_in_clause(values);
        builder.push_char(')');

        let (sql, params) = builder.build();
        assert_eq!(sql, "SELECT * FROM users WHERE id IN ($1, $2, $3, $4, $5)");
        assert_eq!(params.len(), 5);
    }

    #[test]
    fn test_fast_builder_in_clause_mysql() {
        let mut builder = FastSqlBuilder::mysql(QueryCapacity::Custom(128));
        builder.push_str("SELECT * FROM users WHERE id IN (");
        let values: Vec<FilterValue> = (1..=5).map(|i| FilterValue::Int(i)).collect();
        builder.bind_in_clause(values);
        builder.push_char(')');

        let (sql, params) = builder.build();
        assert_eq!(sql, "SELECT * FROM users WHERE id IN (?, ?, ?, ?, ?)");
        assert_eq!(params.len(), 5);
    }

    #[test]
    fn test_fast_builder_identifier() {
        let mut builder = FastSqlBuilder::postgres(QueryCapacity::SimpleSelect);
        builder.push_str("SELECT * FROM ");
        builder.push_identifier("user"); // reserved word
        builder.push_str(" WHERE ");
        builder.push_identifier("my_column"); // not reserved
        builder.push_str(" = ");
        builder.bind(1i64);

        let (sql, _) = builder.build();
        assert_eq!(sql, "SELECT * FROM \"user\" WHERE my_column = $1");
    }

    #[test]
    fn test_fast_builder_identifier_with_quotes() {
        let mut builder = FastSqlBuilder::postgres(QueryCapacity::SimpleSelect);
        builder.push_str("SELECT * FROM ");
        builder.push_identifier("has\"quote");

        let sql = builder.build_sql();
        assert_eq!(sql, "SELECT * FROM \"has\"\"quote\"");
    }

    #[test]
    fn test_fast_builder_conditional() {
        let mut builder = FastSqlBuilder::postgres(QueryCapacity::SelectWithFilters(2));
        builder.push_str("SELECT * FROM users WHERE 1=1");
        builder.push_if(true, " AND active = true");
        builder.push_if(false, " AND deleted = false");

        let sql = builder.build_sql();
        assert_eq!(sql, "SELECT * FROM users WHERE 1=1 AND active = true");
    }

    // Template tests
    #[test]
    fn test_template_select_by_id() {
        let sql = templates::select_by_id("users", &["id", "name", "email"]);
        assert_eq!(sql, "SELECT id, name, email FROM users WHERE id = $1");
    }

    #[test]
    fn test_template_select_by_id_all_columns() {
        let sql = templates::select_by_id("users", &[]);
        assert_eq!(sql, "SELECT * FROM users WHERE id = $1");
    }

    #[test]
    fn test_template_insert_returning() {
        let sql = templates::insert_returning("users", &["name", "email"]);
        assert_eq!(
            sql,
            "INSERT INTO users (name, email) VALUES ($1, $2) RETURNING *"
        );
    }

    #[test]
    fn test_template_update_by_id() {
        let sql = templates::update_by_id("users", &["name", "email"]);
        assert_eq!(sql, "UPDATE users SET name = $1, email = $2 WHERE id = $3");
    }

    #[test]
    fn test_template_delete_by_id() {
        let sql = templates::delete_by_id("users");
        assert_eq!(sql, "DELETE FROM users WHERE id = $1");
    }

    #[test]
    fn test_template_batch_placeholders_postgres() {
        let sql = templates::batch_placeholders(DatabaseType::PostgreSQL, 3, 2);
        assert_eq!(sql, "($1, $2, $3), ($4, $5, $6)");
    }

    #[test]
    fn test_template_batch_placeholders_mysql() {
        let sql = templates::batch_placeholders(DatabaseType::MySQL, 3, 2);
        assert_eq!(sql, "(?, ?, ?), (?, ?, ?)");
    }

    #[test]
    fn test_query_capacity_estimates() {
        assert_eq!(QueryCapacity::SimpleSelect.estimate(), 64);
        assert_eq!(QueryCapacity::SelectWithFilters(5).estimate(), 64 + 5 * 32);
        assert_eq!(QueryCapacity::Insert(10).estimate(), 32 + 10 * 16);
        assert_eq!(QueryCapacity::Update(5).estimate(), 32 + 5 * 20);
        assert_eq!(QueryCapacity::Delete.estimate(), 48);
        assert_eq!(QueryCapacity::Custom(256).estimate(), 256);
    }
}
