//! # PostgreSQL Demo Example
//!
//! This example demonstrates real database connectivity with PostgreSQL
//! using the Prax ORM PostgreSQL driver.
//!
//! ## Prerequisites
//!
//! Start PostgreSQL using docker:
//! ```bash
//! docker run --name prax-postgres -e POSTGRES_USER=prax \
//!     -e POSTGRES_PASSWORD=prax_test_password \
//!     -e POSTGRES_DB=prax_test --network host -d postgres:16-alpine
//! ```
//!
//! ## Running this example
//!
//! ```bash
//! cargo run --example postgres_demo
//! ```

use prax_postgres::{PgEngine, PgPool};
use prax_query::filter::{Filter, FilterValue};
use prax_query::raw::sql_with_params;
use prax_query::traits::QueryEngine;

const DATABASE_URL: &str = "postgresql://prax:prax_test_password@localhost:5432/prax_test";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing for query logging
    tracing_subscriber::fmt()
        .with_env_filter("prax_postgres=debug,postgres_demo=info")
        .init();

    println!("🚀 Prax PostgreSQL Demo\n");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    // =========================================================================
    // STEP 1: Create connection pool
    // =========================================================================
    println!("📦 Creating connection pool...");

    // Use PgConfig directly to avoid runtime configuration issues
    let config = prax_postgres::PgConfig::from_url(DATABASE_URL)?;
    let pool = PgPool::new(config).await?;

    println!("   ✓ Connection pool created\n");

    // =========================================================================
    // STEP 2: Create the PostgreSQL engine
    // =========================================================================
    println!("⚙️  Creating Prax PostgreSQL engine...");

    let engine = PgEngine::new(pool.clone());

    println!("   ✓ Engine created and ready\n");

    // =========================================================================
    // STEP 3: Verify database connection
    // =========================================================================
    println!("🔌 Verifying database connection...");

    let conn = pool.get().await?;
    let row = conn.query_one("SELECT version()", &[]).await?;
    let version: &str = row.get(0);
    println!(
        "   ✓ Connected to: {}\n",
        version.split(" on ").next().unwrap_or(version)
    );

    // =========================================================================
    // STEP 4: Check existing tables (from migration)
    // =========================================================================
    println!("📊 Checking database schema...");

    let tables_row = conn.query_one(
        "SELECT count(*) FROM information_schema.tables WHERE table_schema = 'public' AND table_type = 'BASE TABLE'",
        &[],
    ).await?;
    let table_count: i64 = tables_row.get(0);
    println!("   ✓ Found {} tables in public schema\n", table_count);

    // List the tables
    let tables = conn.query(
        "SELECT table_name FROM information_schema.tables WHERE table_schema = 'public' AND table_type = 'BASE TABLE' ORDER BY table_name",
        &[],
    ).await?;

    println!("   Tables:");
    for table in &tables {
        let name: &str = table.get(0);
        println!("     • {}", name);
    }
    println!();

    // =========================================================================
    // STEP 5: Execute raw SQL queries using the engine
    // =========================================================================
    println!("📝 Executing raw SQL via Prax engine...\n");

    // Count existing users
    let count = engine.count("SELECT COUNT(*) FROM users", vec![]).await?;
    println!("   Current user count: {}", count);

    // Insert a test user using raw SQL
    println!("\n   Inserting test user...");
    let insert_result = engine
        .execute_raw(
            "INSERT INTO users (email, name, active, created_at, updated_at) \
             VALUES ($1, $2, $3, NOW(), NOW()) \
             ON CONFLICT (email) DO NOTHING",
            vec![
                prax_query::filter::FilterValue::String("demo@prax.dev".to_string()),
                prax_query::filter::FilterValue::String("Prax Demo User".to_string()),
                prax_query::filter::FilterValue::Bool(true),
            ],
        )
        .await?;

    println!("   ✓ Insert affected {} row(s)", insert_result);

    // Count users again
    let new_count = engine.count("SELECT COUNT(*) FROM users", vec![]).await?;
    println!("   New user count: {}\n", new_count);

    // =========================================================================
    // STEP 6: Query with the raw SQL builder
    // =========================================================================
    println!("🔍 Using the Prax raw SQL builder...\n");

    // Build a raw SQL query with parameters
    let query = sql_with_params(
        "SELECT id, email, name, active FROM users WHERE active = $1",
        vec![FilterValue::Bool(true)],
    );

    let (sql_str, params) = query.build();
    println!("   Generated SQL: {}", sql_str);
    println!("   Parameters: {:?}\n", params);

    // Execute using the connection directly to get actual results
    let rows = conn
        .query(
            "SELECT id, email, name, active FROM users WHERE active = true LIMIT 5",
            &[],
        )
        .await?;

    println!("   Active users (first 5):");
    for row in rows {
        let id: i64 = row.get(0);
        let email: &str = row.get(1);
        let name: Option<&str> = row.get(2);
        let active: bool = row.get(3);
        println!(
            "     • [{}] {} - {} (active: {})",
            id,
            email,
            name.unwrap_or("(no name)"),
            active
        );
    }
    println!();

    // =========================================================================
    // STEP 7: Test posts table (from schema)
    // =========================================================================
    println!("📚 Working with Posts table...\n");

    // Insert a post for the demo user
    let user_row = conn
        .query_opt("SELECT id FROM users WHERE email = $1", &[&"demo@prax.dev"])
        .await?;

    if let Some(user_row) = user_row {
        let user_id: i64 = user_row.get(0);
        println!("   Found demo user with id: {}", user_id);

        // Insert a post
        let post_result = conn
            .execute(
                "INSERT INTO posts (title, content, published, view_count, created_at, updated_at, user_id) \
                 VALUES ($1, $2, $3, $4, NOW(), NOW(), $5) \
                 ON CONFLICT DO NOTHING",
                &[
                    &"Hello from Prax!",
                    &"This is a demo post created by the Prax PostgreSQL demo.",
                    &true,
                    &0i32,
                    &user_id,
                ],
            )
            .await?;

        println!("   ✓ Created {} post(s)", post_result);

        // Count posts
        let post_count = engine.count("SELECT COUNT(*) FROM posts", vec![]).await?;
        println!("   Total posts in database: {}\n", post_count);
    }

    // =========================================================================
    // STEP 8: Test filter operations
    // =========================================================================
    println!("🎯 Testing filter operations...\n");

    // Build a filter using the Prax filter API
    let filter = Filter::And(
        vec![
            Filter::Equals("active".into(), FilterValue::Bool(true)),
            Filter::Contains("email".into(), FilterValue::String("@".to_string())),
        ]
        .into_boxed_slice(),
    );

    println!("   Filter structure: {:?}", filter);

    // Convert to SQL (Postgres dialect — $N placeholders, double-quoted idents).
    let (where_clause, filter_params) = filter.to_sql(1, &prax_query::dialect::Postgres);
    println!("   Generated WHERE: {}", where_clause);
    println!("   Filter params: {:?}\n", filter_params);

    // =========================================================================
    // STEP 9: Demonstrate connection pooling
    // =========================================================================
    println!("🏊 Connection pool statistics...\n");

    let status = pool.status();
    println!("   Pool size: {}", status.size);
    println!("   Available connections: {}", status.available);
    println!("   Waiting clients: {}\n", status.waiting);

    // =========================================================================
    // STEP 10: Cleanup and summary
    // =========================================================================
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");
    println!("✅ Demo completed successfully!\n");
    println!("📋 Summary:");
    println!("   • Connected to PostgreSQL with connection pooling");
    println!("   • Verified schema tables created by migrations");
    println!("   • Executed raw SQL queries via Prax engine");
    println!("   • Demonstrated filter building and SQL generation");
    println!("   • Created test data (user and post)");
    println!();
    println!("🔗 Next steps:");
    println!("   • Run 'prax generate' to create typed model code");
    println!("   • Use generated code for type-safe queries");
    println!("   • Check prax-query for advanced query building");
    println!();

    Ok(())
}
