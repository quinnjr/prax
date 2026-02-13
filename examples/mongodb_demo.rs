//! # MongoDB Demo Example
//!
//! This example demonstrates real database connectivity with MongoDB
//! using the Prax ORM MongoDB driver.
//!
//! ## Prerequisites
//!
//! Start MongoDB using docker compose:
//! ```bash
//! docker compose up -d mongodb
//! ```
//!
//! ## Running this example
//!
//! ```bash
//! cargo run --example mongodb_demo
//! ```

use prax_mongodb::{Document, MongoClient, ObjectId, doc};
use serde::{Deserialize, Serialize};

const MONGODB_URI: &str =
    "mongodb://prax:prax_test_password@localhost:27017/prax_test?authSource=admin";

/// User document model for MongoDB
#[derive(Debug, Clone, Serialize, Deserialize)]
struct User {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    id: Option<ObjectId>,
    email: String,
    name: Option<String>,
    role: String,
    active: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    created_at: chrono::DateTime<chrono::Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    updated_at: chrono::DateTime<chrono::Utc>,
}

/// Post document model for MongoDB
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Post {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    id: Option<ObjectId>,
    title: String,
    content: Option<String>,
    status: String,
    published: bool,
    views: i32,
    author_id: ObjectId,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    created_at: chrono::DateTime<chrono::Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    updated_at: chrono::DateTime<chrono::Utc>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing for query logging
    tracing_subscriber::fmt()
        .with_env_filter("prax_mongodb=debug,mongodb_demo=info")
        .init();

    println!("🚀 Prax MongoDB Demo\n");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    // =========================================================================
    // STEP 1: Create client connection
    // =========================================================================
    println!("📦 Creating MongoDB client...");

    let client = MongoClient::builder()
        .uri(MONGODB_URI)
        .database("prax_test")
        .app_name("prax-demo")
        .max_pool_size(10)
        .build()
        .await?;

    println!("   ✓ Client created\n");

    // =========================================================================
    // STEP 2: Verify database connection
    // =========================================================================
    println!("🔌 Verifying database connection...");

    let is_healthy = client.is_healthy().await;
    if is_healthy {
        println!("   ✓ Connected to MongoDB\n");
    } else {
        return Err("Failed to connect to MongoDB".into());
    }

    // =========================================================================
    // STEP 3: Check existing collections
    // =========================================================================
    println!("📊 Checking database schema...");

    let collections = client.list_collections().await?;
    println!("   ✓ Found {} collections\n", collections.len());

    println!("   Collections:");
    for collection in &collections {
        println!("     • {}", collection);
    }
    println!();

    // =========================================================================
    // STEP 4: Get typed collections
    // =========================================================================
    println!("📝 Working with typed collections...\n");

    let users_collection = client.collection::<User>("users");
    let posts_collection = client.collection::<Post>("posts");

    // =========================================================================
    // STEP 5: Count existing users
    // =========================================================================
    let user_count = users_collection.count_documents(doc! {}, None).await?;
    println!("   Current user count: {}", user_count);

    // =========================================================================
    // STEP 6: Insert a test user
    // =========================================================================
    println!("   Inserting test user...");

    // Check if user exists first
    let existing = users_collection
        .find_one(doc! { "email": "demo@prax.dev" }, None)
        .await?;

    let demo_user_id = if let Some(user) = existing {
        println!("   ✓ Demo user already exists");
        user.id.unwrap()
    } else {
        let now = chrono::Utc::now();
        let new_user = User {
            id: None,
            email: "demo@prax.dev".to_string(),
            name: Some("Prax Demo User".to_string()),
            role: "Admin".to_string(),
            active: true,
            created_at: now,
            updated_at: now,
        };

        let result = users_collection.insert_one(new_user, None).await?;
        let new_id = result.inserted_id.as_object_id().unwrap();
        println!("   ✓ Created user with id: {}", new_id);
        new_id
    };

    let new_count = users_collection.count_documents(doc! {}, None).await?;
    println!("   New user count: {}\n", new_count);

    // =========================================================================
    // STEP 7: Insert a test post
    // =========================================================================
    println!("📚 Working with posts...\n");

    let post_count = posts_collection.count_documents(doc! {}, None).await?;
    println!("   Current post count: {}", post_count);

    // Check if post exists
    let existing_post = posts_collection
        .find_one(doc! { "title": "MongoDB with Prax" }, None)
        .await?;

    if existing_post.is_none() {
        let now = chrono::Utc::now();
        let new_post = Post {
            id: None,
            title: "MongoDB with Prax".to_string(),
            content: Some("Learn how to use MongoDB with the Prax ORM!".to_string()),
            status: "Published".to_string(),
            published: true,
            views: 0,
            author_id: demo_user_id,
            created_at: now,
            updated_at: now,
        };

        posts_collection.insert_one(new_post, None).await?;
        println!("   ✓ Created demo post");
    } else {
        println!("   ✓ Demo post already exists");
    }

    let new_post_count = posts_collection.count_documents(doc! {}, None).await?;
    println!("   New post count: {}\n", new_post_count);

    // =========================================================================
    // STEP 8: Query users with filters
    // =========================================================================
    println!("🔍 Querying with filters...\n");

    let mut cursor = users_collection.find(doc! { "active": true }, None).await?;

    println!("   Active users (first 5):");
    let mut count = 0;
    while cursor.advance().await? && count < 5 {
        let user = cursor.deserialize_current()?;
        println!(
            "     • {} - {} ({})",
            user.email,
            user.name.as_deref().unwrap_or("unknown"),
            user.role
        );
        count += 1;
    }
    println!();

    // =========================================================================
    // STEP 9: Update a document
    // =========================================================================
    println!("✏️  Updating document...\n");

    let update_result = users_collection
        .update_one(
            doc! { "email": "demo@prax.dev" },
            doc! { "$set": { "updated_at": bson::DateTime::now() } },
            None,
        )
        .await?;
    println!("   ✓ Matched {} document(s)", update_result.matched_count);
    println!(
        "   ✓ Modified {} document(s)\n",
        update_result.modified_count
    );

    // =========================================================================
    // STEP 10: Aggregation pipeline
    // =========================================================================
    println!("📈 Running aggregation pipeline...\n");

    let pipeline = vec![
        doc! {
            "$group": {
                "_id": "$role",
                "count": { "$sum": 1 },
                "active_count": {
                    "$sum": { "$cond": ["$active", 1, 0] }
                }
            }
        },
        doc! {
            "$sort": { "count": -1 }
        },
    ];

    let users_doc = client.collection_doc("users");
    let mut agg_cursor = users_doc.aggregate(pipeline, None).await?;

    println!("   User statistics by role:");
    while agg_cursor.advance().await? {
        let stat = agg_cursor.deserialize_current()?;
        let role = stat.get_str("_id").unwrap_or("unknown");
        let count = stat.get_i32("count").unwrap_or(0);
        let active = stat.get_i32("active_count").unwrap_or(0);
        println!("     • {}: {} total, {} active", role, count, active);
    }
    println!();

    // =========================================================================
    // STEP 11: Lookup (join) aggregation
    // =========================================================================
    println!("🔗 Running lookup (join) aggregation...\n");

    let join_pipeline = vec![
        doc! {
            "$lookup": {
                "from": "users",
                "localField": "author_id",
                "foreignField": "_id",
                "as": "author"
            }
        },
        doc! {
            "$unwind": {
                "path": "$author",
                "preserveNullAndEmptyArrays": true
            }
        },
        doc! {
            "$project": {
                "title": 1,
                "views": 1,
                "status": 1,
                "author_name": "$author.name",
                "author_email": "$author.email"
            }
        },
        doc! {
            "$limit": 5
        },
    ];

    let posts_doc = client.collection_doc("posts");
    let mut join_cursor = posts_doc.aggregate(join_pipeline, None).await?;

    println!("   Posts with authors:");
    while join_cursor.advance().await? {
        let post = join_cursor.deserialize_current()?;
        let title = post.get_str("title").unwrap_or("unknown");
        let views = post.get_i32("views").unwrap_or(0);
        let author = post.get_str("author_name").unwrap_or("unknown");
        println!("     • {} by {} ({} views)", title, author, views);
    }
    println!();

    // =========================================================================
    // STEP 12: Create an index
    // =========================================================================
    println!("📇 Creating index...\n");

    let index_name = client
        .create_index("users", doc! { "email": 1 }, true)
        .await
        .unwrap_or_else(|_| "email_1 (already exists)".to_string());
    println!("   ✓ Index created: {}\n", index_name);

    // =========================================================================
    // STEP 13: Run a database command
    // =========================================================================
    println!("🔧 Running database command...\n");

    let stats = client.run_command(doc! { "dbStats": 1 }).await?;
    let num_collections = stats.get_i32("collections").unwrap_or(0);
    let data_size = stats.get_i64("dataSize").unwrap_or(0);
    println!("   Database stats:");
    println!("     • Collections: {}", num_collections);
    println!("     • Data size: {} bytes\n", data_size);

    // =========================================================================
    // DONE
    // =========================================================================
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");
    println!("✅ MongoDB Demo completed successfully!\n");
    println!("📋 Summary:");
    println!("   • Connected to MongoDB with connection pooling");
    println!("   • Used typed collections with Serde");
    println!("   • Demonstrated CRUD operations");
    println!("   • Ran aggregation pipelines with $lookup joins");
    println!("\n🔗 Next steps:");
    println!("   • Try 'cargo run --example mysql_demo' for MySQL");
    println!("   • Try 'cargo run --example mssql_demo' for SQL Server");
    println!("   • Check prax_mongodb::view for aggregation views");

    Ok(())
}
