#![allow(dead_code, unused, clippy::type_complexity)]
//! # Raw SQL Examples
//!
//! This example demonstrates raw SQL capabilities in Prax:
//! - Simple raw queries
//! - Parameterized queries
//! - Type-safe parameter binding
//! - Raw execute for mutations
//! - SQL interpolation safety
//!
//! ## Running this example
//!
//! ```bash
//! cargo run --example raw_sql
//! ```

use std::collections::HashMap;

// Mock types for demonstration
#[derive(Debug, Clone, Default)]
struct User {
    id: i32,
    email: String,
    name: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct UserStats {
    total_users: i64,
    active_users: i64,
    new_this_month: i64,
}

// Raw SQL builder
struct Sql {
    query: String,
    params: Vec<SqlParam>,
}

#[derive(Debug, Clone)]
enum SqlParam {
    Int(i32),
    BigInt(i64),
    Float(f64),
    String(String),
    Bool(bool),
    Null,
    IntArray(Vec<i32>),
    StringArray(Vec<String>),
}

impl Sql {
    fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            params: Vec::new(),
        }
    }

    fn bind<T: IntoSqlParam>(mut self, value: T) -> Self {
        self.params.push(value.into_sql_param());
        self
    }

    fn query(&self) -> &str {
        &self.query
    }

    fn params(&self) -> &[SqlParam] {
        &self.params
    }
}

trait IntoSqlParam {
    fn into_sql_param(self) -> SqlParam;
}

impl IntoSqlParam for i32 {
    fn into_sql_param(self) -> SqlParam {
        SqlParam::Int(self)
    }
}

impl IntoSqlParam for i64 {
    fn into_sql_param(self) -> SqlParam {
        SqlParam::BigInt(self)
    }
}

impl IntoSqlParam for f64 {
    fn into_sql_param(self) -> SqlParam {
        SqlParam::Float(self)
    }
}

impl IntoSqlParam for &str {
    fn into_sql_param(self) -> SqlParam {
        SqlParam::String(self.to_string())
    }
}

impl IntoSqlParam for String {
    fn into_sql_param(self) -> SqlParam {
        SqlParam::String(self)
    }
}

impl IntoSqlParam for bool {
    fn into_sql_param(self) -> SqlParam {
        SqlParam::Bool(self)
    }
}

impl<T: IntoSqlParam> IntoSqlParam for Option<T> {
    fn into_sql_param(self) -> SqlParam {
        match self {
            Some(v) => v.into_sql_param(),
            None => SqlParam::Null,
        }
    }
}

impl IntoSqlParam for Vec<i32> {
    fn into_sql_param(self) -> SqlParam {
        SqlParam::IntArray(self)
    }
}

impl IntoSqlParam for Vec<String> {
    fn into_sql_param(self) -> SqlParam {
        SqlParam::StringArray(self)
    }
}

// Mock client
struct MockClient;

impl MockClient {
    async fn query_raw<T>(&self, sql: Sql) -> Result<Vec<T>, Box<dyn std::error::Error>>
    where
        T: Default,
    {
        println!("  Executing: {}", sql.query());
        println!("  Parameters: {:?}", sql.params());
        Ok(vec![])
    }

    async fn query_raw_one<T>(&self, sql: Sql) -> Result<Option<T>, Box<dyn std::error::Error>>
    where
        T: Default,
    {
        println!("  Executing: {}", sql.query());
        println!("  Parameters: {:?}", sql.params());
        Ok(None)
    }

    async fn execute_raw(&self, sql: Sql) -> Result<u64, Box<dyn std::error::Error>> {
        println!("  Executing: {}", sql.query());
        println!("  Parameters: {:?}", sql.params());
        Ok(1)
    }
}

/// Macro for type-safe SQL with interpolation
macro_rules! raw_query {
    ($query:expr $(, $param:expr)*) => {{
        let sql = Sql::new($query);
        $(
            let sql = sql.bind($param);
        )*
        sql
    }};
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Prax Raw SQL Examples ===\n");

    let client = MockClient;

    // =========================================================================
    // SIMPLE RAW QUERY
    // =========================================================================
    println!("--- Simple Raw Query ---");

    let sql = Sql::new("SELECT * FROM users WHERE active = true ORDER BY created_at DESC");
    let _users: Vec<User> = client.query_raw(sql).await?;
    println!();

    // =========================================================================
    // PARAMETERIZED QUERY
    // =========================================================================
    println!("--- Parameterized Query ---");

    let sql = Sql::new("SELECT * FROM users WHERE id = $1").bind(42i32);
    let _user: Option<User> = client.query_raw_one(sql).await?;
    println!();

    // Multiple parameters
    println!("--- Multiple Parameters ---");

    let sql = Sql::new("SELECT * FROM users WHERE email = $1 AND active = $2")
        .bind("test@example.com")
        .bind(true);
    let _users: Vec<User> = client.query_raw(sql).await?;
    println!();

    // =========================================================================
    // USING THE MACRO
    // =========================================================================
    println!("--- Using raw_query! Macro ---");

    let email = "alice@example.com";
    let sql = raw_query!("SELECT * FROM users WHERE email = $1", email);
    let _users: Vec<User> = client.query_raw(sql).await?;
    println!();

    // Multiple parameters with macro
    println!("--- Macro with Multiple Parameters ---");

    let status = "active";
    let min_age = 18;
    let sql = raw_query!(
        "SELECT * FROM users WHERE status = $1 AND age >= $2",
        status,
        min_age
    );
    let _users: Vec<User> = client.query_raw(sql).await?;
    println!();

    // =========================================================================
    // ARRAY PARAMETERS
    // =========================================================================
    println!("--- Array Parameters ---");
    // A `Vec<T>` binds as a single `FilterValue::Array` parameter, so it works
    // with `= ANY($1)` on Postgres.

    let ids = vec![1, 2, 3, 4, 5];
    let sql = Sql::new("SELECT * FROM users WHERE id = ANY($1)").bind(ids);
    let _users: Vec<User> = client.query_raw(sql).await?;
    println!();

    // =========================================================================
    // RAW EXECUTE (INSERT/UPDATE/DELETE)
    // =========================================================================
    println!("--- Raw Execute (Mutations) ---");

    // Insert
    let sql = Sql::new("INSERT INTO users (email, name, active) VALUES ($1, $2, $3)")
        .bind("new@example.com")
        .bind("New User")
        .bind(true);
    let affected = client.execute_raw(sql).await?;
    println!("  Rows affected: {}", affected);
    println!();

    // Update
    println!("--- Raw Update ---");
    let sql = Sql::new("UPDATE users SET active = $1 WHERE last_login < $2")
        .bind(false)
        .bind("2024-01-01");
    let affected = client.execute_raw(sql).await?;
    println!("  Rows affected: {}", affected);
    println!();

    // Delete
    println!("--- Raw Delete ---");
    let sql = Sql::new("DELETE FROM users WHERE deleted_at IS NOT NULL AND deleted_at < $1")
        .bind("2024-01-01");
    let affected = client.execute_raw(sql).await?;
    println!("  Rows affected: {}", affected);
    println!();

    // =========================================================================
    // COMPLEX QUERIES
    // =========================================================================
    println!("--- Complex Query with Joins ---");

    let sql = Sql::new(
        r#"
        SELECT u.id, u.email, COUNT(p.id) as post_count
        FROM users u
        LEFT JOIN posts p ON p.author_id = u.id
        WHERE u.active = $1
        GROUP BY u.id, u.email
        HAVING COUNT(p.id) >= $2
        ORDER BY post_count DESC
        LIMIT $3
    "#,
    )
    .bind(true)
    .bind(5i32)
    .bind(10i32);

    let _results: Vec<HashMap<String, String>> = client.query_raw(sql).await?;
    println!();

    // =========================================================================
    // AGGREGATE QUERIES
    // =========================================================================
    println!("--- Aggregate Query ---");

    let sql = Sql::new(
        r#"
        SELECT
            COUNT(*) as total_users,
            COUNT(*) FILTER (WHERE active = true) as active_users,
            COUNT(*) FILTER (WHERE created_at > NOW() - INTERVAL '30 days') as new_this_month
        FROM users
    "#,
    );

    let _stats: Option<UserStats> = client.query_raw_one(sql).await?;
    println!();

    // =========================================================================
    // NULL HANDLING
    // =========================================================================
    println!("--- NULL Parameter Handling ---");

    let name: Option<&str> = None;
    let sql = Sql::new("UPDATE users SET name = $1 WHERE id = $2")
        .bind(name)
        .bind(42i32);
    let _affected = client.execute_raw(sql).await?;
    println!();

    // =========================================================================
    // SQL INJECTION PREVENTION
    // =========================================================================
    println!("--- SQL Injection Prevention ---");

    println!("❌ NEVER do this (vulnerable to SQL injection):");
    println!("   let query = format!(\"SELECT * FROM users WHERE email = '{{}}'\", user_input);");
    println!();

    println!("✓ ALWAYS use parameterized queries:");
    let user_input = "alice@example.com'; DROP TABLE users; --";
    let sql = Sql::new("SELECT * FROM users WHERE email = $1").bind(user_input);
    println!("  Safe query with potentially malicious input:");
    let _users: Vec<User> = client.query_raw(sql).await?;
    println!();

    // =========================================================================
    // DATABASE-SPECIFIC FEATURES
    // =========================================================================
    println!("--- Database-Specific Features ---");

    // PostgreSQL JSON operations
    println!("PostgreSQL JSON query:");
    let sql = Sql::new(
        r#"
        SELECT * FROM users
        WHERE metadata->>'role' = $1
        AND (metadata->'permissions') @> $2::jsonb
    "#,
    )
    .bind("admin")
    .bind(r#"["write", "delete"]"#);
    let _users: Vec<User> = client.query_raw(sql).await?;
    println!();

    // Full-text search
    println!("PostgreSQL Full-text Search:");
    let sql = Sql::new(r#"
        SELECT * FROM posts
        WHERE to_tsvector('english', title || ' ' || content) @@ plainto_tsquery('english', $1)
        ORDER BY ts_rank(to_tsvector('english', title || ' ' || content), plainto_tsquery('english', $1)) DESC
    "#)
        .bind("rust async programming");
    let _posts: Vec<HashMap<String, String>> = client.query_raw(sql).await?;
    println!();

    // =========================================================================
    // UPSERT EXAMPLE
    // =========================================================================
    println!("--- Upsert (ON CONFLICT) ---");

    let sql = Sql::new(
        r#"
        INSERT INTO users (email, name, active)
        VALUES ($1, $2, $3)
        ON CONFLICT (email)
        DO UPDATE SET
            name = EXCLUDED.name,
            updated_at = NOW()
        RETURNING *
    "#,
    )
    .bind("alice@example.com")
    .bind("Alice Updated")
    .bind(true);
    let _user: Option<User> = client.query_raw_one(sql).await?;
    println!();

    // =========================================================================
    // CTE (Common Table Expressions)
    // =========================================================================
    println!("--- CTE (WITH clause) ---");

    let sql = Sql::new(
        r#"
        WITH active_users AS (
            SELECT * FROM users WHERE active = true
        ),
        user_post_counts AS (
            SELECT author_id, COUNT(*) as count
            FROM posts
            GROUP BY author_id
        )
        SELECT u.*, COALESCE(p.count, 0) as post_count
        FROM active_users u
        LEFT JOIN user_post_counts p ON u.id = p.author_id
        ORDER BY post_count DESC
        LIMIT $1
    "#,
    )
    .bind(10i32);
    let _results: Vec<HashMap<String, String>> = client.query_raw(sql).await?;
    println!();

    // =========================================================================
    // API REFERENCE
    // =========================================================================
    println!("--- API Reference ---");
    println!(
        r#"
Raw SQL API:

// Query returning multiple rows
let users: Vec<User> = client
    .query_raw(Sql::new("SELECT * FROM users WHERE active = $1")
        .bind(true))
    .await?;

// Query returning single row
let user: Option<User> = client
    .query_raw_one(Sql::new("SELECT * FROM users WHERE id = $1")
        .bind(user_id))
    .await?;

// Execute mutation (INSERT/UPDATE/DELETE)
let affected: u64 = client
    .execute_raw(Sql::new("DELETE FROM users WHERE id = $1")
        .bind(user_id))
    .await?;

// Using the macro
let users: Vec<User> = client
    .query_raw(raw_query!(
        "SELECT * FROM users WHERE email LIKE $1 AND age > $2",
        "%@example.com",
        18
    ))
    .await?;

Supported parameter types:
- i32, i64 (integers)
- f64 (floating point)
- &str, String (text)
- bool (boolean)
- Option<T> (nullable)
- Vec<i32>, Vec<String> (arrays)
- chrono::DateTime, chrono::NaiveDate (dates)
- uuid::Uuid (UUIDs)
- serde_json::Value (JSON)
"#
    );

    println!("=== All examples completed successfully! ===");

    Ok(())
}
