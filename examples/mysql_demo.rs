//! # MySQL Demo Example
//!
//! This example demonstrates real database connectivity with MySQL
//! using the Prax ORM MySQL driver.
//!
//! ## Prerequisites
//!
//! Start MySQL using docker compose:
//! ```bash
//! docker compose up -d mysql
//! ```
//!
//! ## Running this example
//!
//! ```bash
//! cargo run --example mysql_demo
//! ```

use prax_mysql::{MysqlConfig, MysqlPool, MysqlRawEngine};
use prax_query::filter::FilterValue;
use std::collections::HashMap;

const DATABASE_URL: &str = "mysql://prax:prax_test_password@localhost:3307/prax_test";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing for query logging
    tracing_subscriber::fmt()
        .with_env_filter("prax_mysql=debug,mysql_demo=info")
        .init();

    println!("🚀 Prax MySQL Demo\n");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    // =========================================================================
    // STEP 1: Create connection pool
    // =========================================================================
    println!("📦 Creating connection pool...");

    let config = MysqlConfig::from_url(DATABASE_URL)?;
    let pool = MysqlPool::new(config).await?;

    println!("   ✓ Connection pool created\n");

    // =========================================================================
    // STEP 2: Create the MySQL engine
    // =========================================================================
    println!("⚙️  Creating Prax MySQL engine...");

    let engine = MysqlRawEngine::new(pool.clone());

    println!("   ✓ Engine created and ready\n");

    // =========================================================================
    // STEP 3: Verify database connection
    // =========================================================================
    println!("🔌 Verifying database connection...");

    let version_result = engine.raw_sql_scalar("SELECT VERSION()", &[]).await?;
    println!(
        "   ✓ Connected to: MySQL {}\n",
        version_result.as_str().unwrap_or("unknown")
    );

    // =========================================================================
    // STEP 4: Check existing tables
    // =========================================================================
    println!("📊 Checking database schema...");

    let tables_result = engine
        .raw_sql_query(
            "SELECT TABLE_NAME FROM information_schema.TABLES WHERE TABLE_SCHEMA = DATABASE()",
            &[],
        )
        .await?;
    println!("   ✓ Found {} tables\n", tables_result.len());

    println!("   Tables:");
    for table in &tables_result {
        if let Some(name) = table.json().get("TABLE_NAME").and_then(|v| v.as_str()) {
            println!("     • {}", name);
        }
    }
    println!();

    // =========================================================================
    // STEP 5: Count existing users
    // =========================================================================
    println!("📝 Querying data via Prax engine...\n");

    let user_count = engine.count("users", &HashMap::new()).await?;
    println!("   Current user count: {}", user_count);

    // =========================================================================
    // STEP 6: Insert a test user
    // =========================================================================
    println!("   Inserting test user...");

    // Check if user exists first
    let existing = engine
        .raw_sql_optional(
            "SELECT id FROM users WHERE email = ?",
            &[FilterValue::String("demo@prax.dev".to_string())],
        )
        .await?;

    if existing.is_none() {
        let mut user_data = HashMap::new();
        user_data.insert(
            "name".to_string(),
            FilterValue::String("Prax Demo User".to_string()),
        );
        user_data.insert(
            "email".to_string(),
            FilterValue::String("demo@prax.dev".to_string()),
        );
        user_data.insert(
            "status".to_string(),
            FilterValue::String("active".to_string()),
        );
        user_data.insert("role".to_string(), FilterValue::String("admin".to_string()));
        user_data.insert("verified".to_string(), FilterValue::Bool(true));

        let result = engine.execute_insert("users", &user_data).await?;
        let new_id = result
            .json()
            .get("id")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        println!("   ✓ Created user with id: {}", new_id);
    } else {
        println!("   ✓ Demo user already exists");
    }

    let new_count = engine.count("users", &HashMap::new()).await?;
    println!("   New user count: {}\n", new_count);

    // =========================================================================
    // STEP 7: Query users with filters
    // =========================================================================
    println!("🔍 Querying with filters...\n");

    let mut filters = HashMap::new();
    filters.insert(
        "status".to_string(),
        FilterValue::String("active".to_string()),
    );

    let active_users = engine
        .query_many("users", &[], &filters, &[], Some(5), None)
        .await?;

    println!("   Active users (first 5):");
    for user in &active_users {
        let name = user
            .json()
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let email = user
            .json()
            .get("email")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        println!("     • {} - {}", name, email);
    }
    println!();

    // =========================================================================
    // STEP 8: Raw SQL query with parameters
    // =========================================================================
    println!("📚 Executing raw SQL with parameters...\n");

    let admins = engine
        .raw_sql_query(
            "SELECT name, email, role FROM users WHERE role = ? ORDER BY name LIMIT 5",
            &[FilterValue::String("admin".to_string())],
        )
        .await?;

    println!("   Admin users:");
    for admin in &admins {
        let name = admin
            .json()
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let role = admin
            .json()
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        println!("     • {} ({})", name, role);
    }
    println!();

    // =========================================================================
    // STEP 9: Update a user
    // =========================================================================
    println!("✏️  Updating user...\n");

    let mut update_data = HashMap::new();
    update_data.insert("score".to_string(), FilterValue::Int(100));

    let mut update_filters = HashMap::new();
    update_filters.insert(
        "email".to_string(),
        FilterValue::String("demo@prax.dev".to_string()),
    );

    let affected = engine
        .execute_update("users", &update_data, &update_filters)
        .await?;
    println!("   ✓ Updated {} row(s)\n", affected);

    // =========================================================================
    // STEP 10: Aggregation query
    // =========================================================================
    println!("📈 Running aggregation query...\n");

    let stats = engine
        .raw_sql_query(
            "SELECT role, COUNT(*) as count, AVG(score) as avg_score FROM users GROUP BY role",
            &[],
        )
        .await?;

    println!("   User statistics by role:");
    for stat in &stats {
        let role = stat
            .json()
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let count = stat
            .json()
            .get("count")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let avg_score = stat
            .json()
            .get("avg_score")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        println!(
            "     • {}: {} users, avg score: {:.1}",
            role, count, avg_score
        );
    }
    println!();

    // =========================================================================
    // STEP 11: Join query
    // =========================================================================
    println!("🔗 Running join query...\n");

    let posts_with_authors = engine
        .raw_sql_query(
            r#"
            SELECT p.title, p.view_count, u.name as author
            FROM posts p
            JOIN users u ON p.user_id = u.id
            WHERE p.published = true
            ORDER BY p.view_count DESC
            LIMIT 5
            "#,
            &[],
        )
        .await?;

    println!("   Top 5 published posts:");
    for post in &posts_with_authors {
        let title = post
            .json()
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let views = post
            .json()
            .get("view_count")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let author = post
            .json()
            .get("author")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        println!("     • {} by {} ({} views)", title, author, views);
    }
    println!();

    // =========================================================================
    // DONE
    // =========================================================================
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");
    println!("✅ MySQL Demo completed successfully!\n");
    println!("📋 Summary:");
    println!("   • Connected to MySQL with connection pooling");
    println!("   • Queried and filtered data");
    println!("   • Executed raw SQL with parameters");
    println!("   • Performed aggregations and joins");
    println!("\n🔗 Next steps:");
    println!("   • Try 'cargo run --example mssql_demo' for SQL Server");
    println!("   • Try 'cargo run --example mongodb_demo' for MongoDB");

    Ok(())
}
