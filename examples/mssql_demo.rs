//! # Microsoft SQL Server Demo Example
//!
//! This example demonstrates real database connectivity with SQL Server
//! using the Prax ORM MSSQL driver.
//!
//! ## Prerequisites
//!
//! Start SQL Server using docker compose:
//! ```bash
//! docker compose up -d mssql
//! ```
//!
//! Wait for SQL Server to be ready (about 30 seconds), then run the init script:
//! ```bash
//! docker exec -it prax-mssql /opt/mssql-tools18/bin/sqlcmd -S localhost -U sa \
//!     -P 'Prax_Test_Password123!' -C -i /docker-entrypoint-initdb.d/init.sql
//! ```
//!
//! ## Running this example
//!
//! ```bash
//! cargo run --example mssql_demo
//! ```

use prax_mssql::{MssqlConnection, MssqlEngine, MssqlPool, Row};
use prax_query::traits::QueryEngine;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing for query logging
    tracing_subscriber::fmt()
        .with_env_filter("prax_mssql=debug,mssql_demo=info")
        .init();

    println!("🚀 Prax Microsoft SQL Server Demo\n");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    // =========================================================================
    // STEP 1: Create connection pool
    // =========================================================================
    println!("📦 Creating connection pool...");

    let pool: MssqlPool = MssqlPool::builder()
        .host("localhost")
        .port(1433)
        .database("prax_test")
        .username("sa")
        .password("Prax_Test_Password123!")
        .trust_cert(true)
        .max_connections(10)
        .build()
        .await?;

    println!("   ✓ Connection pool created\n");

    // =========================================================================
    // STEP 2: Create the MSSQL engine
    // =========================================================================
    println!("⚙️  Creating Prax MSSQL engine...");

    let engine: MssqlEngine = MssqlEngine::new(pool.clone());

    println!("   ✓ Engine created and ready\n");

    // =========================================================================
    // STEP 3: Verify database connection
    // =========================================================================
    println!("🔌 Verifying database connection...");

    // Check if pool is healthy
    if pool.is_healthy().await {
        println!("   ✓ Connection pool is healthy\n");
    } else {
        println!("   ✗ Connection pool is not healthy\n");
        return Err("Failed to connect to SQL Server".into());
    }

    // =========================================================================
    // STEP 4: Check existing tables using raw connection
    // =========================================================================
    println!("📊 Checking database schema...");

    {
        let mut conn: MssqlConnection<'_> = pool.get().await?;
        let tables: Vec<Row> = conn
            .query(
                "SELECT TABLE_NAME FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_TYPE = 'BASE TABLE' ORDER BY TABLE_NAME",
                &[],
            )
            .await?;

        println!("   ✓ Found {} tables\n", tables.len());
        println!("   Tables:");
        for table in &tables {
            let name: Option<&str> = table.get(0);
            println!("     • {}", name.unwrap_or("unknown"));
        }
        println!();
    }

    // =========================================================================
    // STEP 5: Count existing users via engine
    // =========================================================================
    println!("📝 Querying data via Prax engine...\n");

    let count: u64 = engine.count("SELECT COUNT(*) FROM users", vec![]).await?;
    println!("   Current user count: {}", count);

    // =========================================================================
    // STEP 6: Insert a test user
    // =========================================================================
    println!("   Inserting test user...");

    {
        let mut conn: MssqlConnection<'_> = pool.get().await?;

        // Check if user exists first
        let existing: Option<Row> = conn
            .query_opt(
                "SELECT id FROM users WHERE email = @P1",
                &[&"demo@prax.dev"],
            )
            .await?;

        if existing.is_none() {
            conn.execute(
                r#"
                INSERT INTO users (email, name, role, active, created_at, updated_at)
                VALUES (@P1, @P2, @P3, 1, GETUTCDATE(), GETUTCDATE())
                "#,
                &[&"demo@prax.dev", &"Prax Demo User", &"Admin"],
            )
            .await?;
            println!("   ✓ Created demo user");
        } else {
            println!("   ✓ Demo user already exists");
        }
    }

    let new_count: u64 = engine.count("SELECT COUNT(*) FROM users", vec![]).await?;
    println!("   New user count: {}\n", new_count);

    // =========================================================================
    // STEP 7: Query users with filters
    // =========================================================================
    println!("🔍 Querying with filters...\n");

    {
        let mut conn: MssqlConnection<'_> = pool.get().await?;
        let active_users: Vec<Row> = conn
            .query(
                "SELECT TOP 5 id, email, name, role FROM users WHERE active = 1 ORDER BY id",
                &[],
            )
            .await?;

        println!("   Active users (first 5):");
        for user in &active_users {
            let id: i32 = user.get(0).unwrap_or(0);
            let email: Option<&str> = user.get(1);
            let name: Option<&str> = user.get(2);
            let role: Option<&str> = user.get(3);
            println!(
                "     • [{}] {} - {} ({})",
                id,
                email.unwrap_or("unknown"),
                name.unwrap_or("unknown"),
                role.unwrap_or("unknown")
            );
        }
        println!();
    }

    // =========================================================================
    // STEP 8: Update a user
    // =========================================================================
    println!("✏️  Updating user...\n");

    {
        let mut conn: MssqlConnection<'_> = pool.get().await?;
        let affected: u64 = conn
            .execute(
                "UPDATE users SET updated_at = GETUTCDATE() WHERE email = @P1",
                &[&"demo@prax.dev"],
            )
            .await?;
        println!("   ✓ Updated {} row(s)\n", affected);
    }

    // =========================================================================
    // STEP 9: Aggregation query
    // =========================================================================
    println!("📈 Running aggregation query...\n");

    {
        let mut conn: MssqlConnection<'_> = pool.get().await?;
        let stats: Vec<Row> = conn
            .query(
                "SELECT role, COUNT(*) as count FROM users GROUP BY role ORDER BY count DESC",
                &[],
            )
            .await?;

        println!("   User statistics by role:");
        for stat in &stats {
            let role: Option<&str> = stat.get(0);
            let count: i32 = stat.get(1).unwrap_or(0);
            println!("     • {}: {} users", role.unwrap_or("unknown"), count);
        }
        println!();
    }

    // =========================================================================
    // STEP 10: Session context (RLS support)
    // =========================================================================
    println!("🔐 Testing session context (RLS support)...\n");

    {
        let mut conn: MssqlConnection<'_> = pool.get().await?;
        conn.set_session_context("tenant_id", "tenant_123").await?;
        let tenant_id: Option<String> = conn.get_session_context("tenant_id").await?;
        println!(
            "   ✓ Session context 'tenant_id' = {}\n",
            tenant_id.as_deref().unwrap_or("(not set)")
        );
    }

    // =========================================================================
    // STEP 11: Transaction example
    // =========================================================================
    println!("💾 Testing transaction...\n");

    {
        let mut conn: MssqlConnection<'_> = pool.get().await?;
        conn.begin_transaction().await?;

        // Create a savepoint
        conn.savepoint("before_update").await?;

        // Make some changes
        let updated: u64 = conn
            .execute(
                "UPDATE users SET name = @P1 WHERE email = @P2",
                &[&"Prax Demo (Updated)", &"demo@prax.dev"],
            )
            .await?;
        println!("   ✓ Updated {} row(s) within transaction", updated);

        // Rollback to savepoint
        conn.rollback_to("before_update").await?;
        println!("   ✓ Rolled back to savepoint");

        // Commit the transaction (no changes since we rolled back)
        conn.commit().await?;
        println!("   ✓ Transaction committed\n");
    }

    // =========================================================================
    // STEP 12: Pool statistics
    // =========================================================================
    println!("🏊 Connection pool statistics...\n");

    let status = pool.status();
    println!("   Connections: {}", status.connections);
    println!("   Idle connections: {}", status.idle_connections);
    println!("   Max size: {}", status.max_size);
    println!();

    // =========================================================================
    // DONE
    // =========================================================================
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");
    println!("✅ MSSQL Demo completed successfully!\n");
    println!("📋 Summary:");
    println!("   • Connected to SQL Server with connection pooling");
    println!("   • Queried and filtered data");
    println!("   • Demonstrated transactions with savepoints");
    println!("   • Used session context for RLS support");
    println!("\n🔗 Next steps:");
    println!("   • Try 'cargo run --example mysql_demo' for MySQL");
    println!("   • Try 'cargo run --example mongodb_demo' for MongoDB");
    println!("   • Check prax_mssql::rls module for Row-Level Security");

    Ok(())
}
