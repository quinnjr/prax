#![allow(dead_code, unused, clippy::type_complexity)]
//! Example Rust seed script for Prax
//!
//! This file demonstrates how to create a Rust seed script.
//! To use this in your project:
//!
//! 1. Copy this file to your project root or `prax/` directory
//! 2. Add a [[bin]] entry to your Cargo.toml:
//!    ```toml
//!    [[bin]]
//!    name = "seed"
//!    path = "seed.rs"  # or "prax/seed.rs"
//!    ```
//! 3. Run: `prax db seed`
//!
//! Or run directly: `cargo run --bin seed`

use std::env;

// In a real project, you would import from your generated client:
// use crate::generated::*;
// For this example, we'll use prax-query directly

fn main() {
    // Initialize logging
    prax_query::logging::init();

    // Get database URL from environment
    let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| {
        eprintln!("Error: DATABASE_URL environment variable not set");
        eprintln!("Set it with: export DATABASE_URL=postgres://user:pass@localhost/db");
        std::process::exit(1);
    });

    // Get environment
    let environment = env::var("PRAX_ENV").unwrap_or_else(|_| "development".to_string());

    println!("🌱 Prax Database Seeder");
    println!("   Environment: {}", environment);
    println!("   Database: {}", mask_url(&database_url));
    println!();

    // In a real seed script, you would use the generated Prax client:
    //
    // ```rust
    // use tokio;
    // use your_app::generated::*;
    //
    // #[tokio::main]
    // async fn main() -> Result<(), Box<dyn std::error::Error>> {
    //     let client = PraxClient::new(&database_url).await?;
    //
    //     // Create admin user
    //     let admin = client.user().create(
    //         user::email::set("admin@example.com".to_string()),
    //         user::name::set("Admin User".to_string()),
    //         vec![user::role::set(Role::Admin)],
    //     ).exec().await?;
    //     println!("Created admin user: {:?}", admin);
    //
    //     // Create sample posts
    //     for i in 1..=5 {
    //         client.post().create(
    //             post::title::set(format!("Sample Post {}", i)),
    //             post::author::connect(user::id::equals(admin.id)),
    //             vec![
    //                 post::content::set(format!("Content for post {}", i)),
    //                 post::published::set(i % 2 == 0),
    //             ],
    //         ).exec().await?;
    //     }
    //     println!("Created 5 sample posts");
    //
    //     Ok(())
    // }
    // ```

    // For this example, we just demonstrate the structure
    println!("📦 Seeding users...");
    seed_users();

    println!("📝 Seeding posts...");
    seed_posts();

    println!("💬 Seeding comments...");
    seed_comments();

    println!();
    println!("✅ Database seeded successfully!");
    println!("   Created 5 users");
    println!("   Created 5 posts");
    println!("   Created 4 comments");
}

fn seed_users() {
    let users = vec![
        ("admin@example.com", "Admin User", "ADMIN"),
        ("john@example.com", "John Doe", "USER"),
        ("jane@example.com", "Jane Smith", "USER"),
        ("bob@example.com", "Bob Wilson", "USER"),
        ("alice@example.com", "Alice Brown", "MODERATOR"),
    ];

    for (email, name, role) in users {
        println!("  + {} ({}) - {}", name, email, role);
    }
}

fn seed_posts() {
    let posts = vec![
        ("Welcome to Prax", true),
        ("Getting Started Guide", true),
        ("My First Post", true),
        ("Draft Post", false),
        ("Tips and Tricks", true),
    ];

    for (title, published) in posts {
        let status = if published { "published" } else { "draft" };
        println!("  + {} [{}]", title, status);
    }
}

fn seed_comments() {
    let comments = vec![
        ("Great post!", "post #1"),
        ("Thanks for sharing!", "post #1"),
        ("Very helpful guide.", "post #2"),
        ("Welcome John!", "post #3"),
    ];

    for (content, post) in comments {
        println!("  + \"{}\" on {}", content, post);
    }
}

fn mask_url(url: &str) -> String {
    // Simple URL masking - hide password portion
    if let Some(at_pos) = url.find('@')
        && let Some(scheme_end) = url.find("://") {
            let scheme = &url[..scheme_end + 3];
            let after_at = &url[at_pos..];
            // Check if there's a password (contains :)
            let user_info = &url[scheme_end + 3..at_pos];
            if user_info.contains(':')
                && let Some(colon_pos) = user_info.find(':') {
                    let user = &user_info[..colon_pos];
                    return format!("{}{}:****{}", scheme, user, after_at);
                }
        }
    // No password found, return truncated URL
    if url.len() > 50 {
        format!("{}...", &url[..50])
    } else {
        url.to_string()
    }
}
