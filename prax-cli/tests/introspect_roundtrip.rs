//! Docker-gated round-trip introspection integration tests.
//!
//! For each SQL backend these tests seed a known schema in a live database,
//! introspect it through the CLI's `introspect_database` dispatcher, map the
//! result to a `prax_schema::Schema`, and assert:
//!
//! 1. Diffing a matching `.prax` against the introspected source is **empty**
//!    (the no-spurious-churn property — the single most important test).
//! 2. Diffing a `.prax` that adds one column yields exactly that delta.
//!
//! Every test is `#[ignore]` and self-skips unless `PRAX_E2E=1`, following the
//! workspace convention. Run via docker compose:
//!
//! ```sh
//! docker compose up -d postgres mysql mssql
//! PRAX_E2E=1 POSTGRES_URL=... MYSQL_URL=... MSSQL_URL=... SQLITE_URL=file:./e2e.db \
//!   cargo test -p prax-orm-cli --features postgres,mysql,sqlite,mssql \
//!   --test introspect_roundtrip -- --ignored
//! ```

#![cfg(test)]

#[allow(unused_imports)]
use prax_cli::commands::introspect::{IntrospectionOptions, introspect_database};
#[allow(unused_imports)]
use prax_cli::commands::schema_from_db::schema_from_database;
#[allow(unused_imports)]
use prax_migrate::{IntrospectionConfig, SchemaDiffer};

/// Whether E2E tests should run.
#[allow(dead_code)]
fn e2e() -> bool {
    std::env::var("PRAX_E2E").ok().as_deref() == Some("1")
}

/// Build the diff of `target_prax` against a `.prax` introspected from `db`.
#[allow(dead_code)]
fn diff_against_source(target_prax: &str, source: prax_schema::Schema) -> prax_migrate::SchemaDiff {
    let target = prax_schema::parse_schema(target_prax).expect("target .prax parses");
    SchemaDiffer::new(target)
        .with_source(source)
        .diff()
        .expect("diff")
}

// ============================================================================
// PostgreSQL
// ============================================================================

#[cfg(feature = "postgres")]
mod postgres {
    use super::*;

    fn url() -> Option<String> {
        std::env::var("POSTGRES_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .ok()
    }

    async fn exec(url: &str, sql: &str) {
        let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
            .await
            .expect("connect postgres");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        client.batch_execute(sql).await.expect("exec ddl");
    }

    const SEED: &str = "
        DROP TABLE IF EXISTS pg_rt_posts CASCADE;
        DROP TABLE IF EXISTS pg_rt_users CASCADE;
        CREATE TABLE pg_rt_users (
            id BIGINT PRIMARY KEY,
            email TEXT NOT NULL UNIQUE
        );
        CREATE TABLE pg_rt_posts (
            id BIGINT PRIMARY KEY,
            title TEXT NOT NULL,
            author_id BIGINT NOT NULL,
            CONSTRAINT pg_rt_posts_author_fk FOREIGN KEY (author_id) REFERENCES pg_rt_users (id)
        );
    ";

    fn prax_v1() -> &'static str {
        r#"
        model User {
            id    BigInt @id
            email String @unique
            @@map("pg_rt_users")
        }
        model Post {
            id        BigInt @id
            title     String
            author_id BigInt
            author    User   @relation(fields: [author_id], references: [id], map: "pg_rt_posts_author_fk")
            @@map("pg_rt_posts")
        }
        "#
    }

    #[tokio::test]
    #[ignore = "requires running PostgreSQL via docker-compose"]
    async fn roundtrip_empty_and_delta() {
        if !e2e() {
            eprintln!("skipping: PRAX_E2E not set");
            return;
        }
        let url = url().expect("POSTGRES_URL required");
        exec(&url, SEED).await;

        let opts = IntrospectionOptions {
            table_filter: Some("pg_rt_*".to_string()),
            ..Default::default()
        };
        let db = introspect_database("postgres", &url, &opts)
            .await
            .expect("introspect");
        let source = schema_from_database(&db, IntrospectionConfig::default())
            .expect("map")
            .schema;

        let empty = diff_against_source(prax_v1(), source.clone());
        assert!(
            empty.is_empty(),
            "expected empty diff, got: {}",
            empty.summary()
        );

        // v2 adds a nullable column on users.
        let v2 = r#"
        model User {
            id    BigInt  @id
            email String  @unique
            bio   String?
            @@map("pg_rt_users")
        }
        model Post {
            id        BigInt @id
            title     String
            author_id BigInt
            author    User   @relation(fields: [author_id], references: [id], map: "pg_rt_posts_author_fk")
            @@map("pg_rt_posts")
        }
        "#;
        let delta = diff_against_source(v2, source);
        assert!(delta.create_models.is_empty(), "no new tables");
        assert_eq!(delta.alter_models.len(), 1, "one altered model");
        assert_eq!(delta.alter_models[0].add_fields.len(), 1);
        assert_eq!(delta.alter_models[0].add_fields[0].column_name, "bio");
    }
}

// ============================================================================
// MySQL
// ============================================================================

#[cfg(feature = "mysql")]
mod mysql {
    use super::*;
    use prax_mysql::{MysqlPool, MysqlRawEngine};

    fn url() -> Option<String> {
        std::env::var("MYSQL_URL").ok()
    }

    async fn engine(url: &str) -> MysqlRawEngine {
        let pool = MysqlPool::builder()
            .url(url)
            .build()
            .await
            .expect("connect mysql");
        MysqlRawEngine::new(pool)
    }

    fn prax_v1() -> &'static str {
        r#"
        model User {
            id    BigInt @id
            email String @unique
            @@map("my_rt_users")
        }
        model Post {
            id        BigInt @id
            title     String
            author_id BigInt
            author    User   @relation(fields: [author_id], references: [id], map: "my_rt_posts_author_fk")
            @@map("my_rt_posts")
        }
        "#
    }

    #[tokio::test]
    #[ignore = "requires running MySQL via docker-compose"]
    async fn roundtrip_empty_and_delta() {
        if !e2e() {
            eprintln!("skipping: PRAX_E2E not set");
            return;
        }
        let url = url().expect("MYSQL_URL required");
        let eng = engine(&url).await;
        for stmt in [
            "DROP TABLE IF EXISTS my_rt_posts",
            "DROP TABLE IF EXISTS my_rt_users",
            "CREATE TABLE my_rt_users (id BIGINT PRIMARY KEY, email VARCHAR(255) NOT NULL UNIQUE)",
            "CREATE TABLE my_rt_posts (id BIGINT PRIMARY KEY, title TEXT NOT NULL, author_id BIGINT NOT NULL, \
             CONSTRAINT my_rt_posts_author_fk FOREIGN KEY (author_id) REFERENCES my_rt_users (id))",
        ] {
            eng.raw_sql_execute(stmt, &[]).await.expect("seed ddl");
        }

        // MySQL scopes information_schema by database; pass the DB name.
        let db_name = url
            .rsplit('/')
            .next()
            .and_then(|s| s.split('?').next())
            .unwrap_or("prax_test")
            .to_string();
        let opts = IntrospectionOptions {
            schema: Some(db_name),
            table_filter: Some("my_rt_*".to_string()),
            ..Default::default()
        };
        let db = introspect_database("mysql", &url, &opts)
            .await
            .expect("introspect");
        let source = schema_from_database(&db, IntrospectionConfig::default())
            .expect("map")
            .schema;

        let empty = diff_against_source(prax_v1(), source.clone());
        assert!(
            empty.is_empty(),
            "expected empty diff, got: {}",
            empty.summary()
        );

        let v2 = r#"
        model User {
            id    BigInt  @id
            email String  @unique
            bio   String?
            @@map("my_rt_users")
        }
        model Post {
            id        BigInt @id
            title     String
            author_id BigInt
            author    User   @relation(fields: [author_id], references: [id], map: "my_rt_posts_author_fk")
            @@map("my_rt_posts")
        }
        "#;
        let delta = diff_against_source(v2, source);
        assert!(delta.create_models.is_empty(), "no new tables");
        assert_eq!(delta.alter_models.len(), 1, "one altered model");
        assert_eq!(delta.alter_models[0].add_fields[0].column_name, "bio");
    }
}

// ============================================================================
// SQLite
// ============================================================================

#[cfg(feature = "sqlite")]
mod sqlite {
    use super::*;
    use prax_sqlite::{SqlitePool, SqliteRawEngine};

    #[tokio::test]
    #[ignore = "requires PRAX_E2E (uses a temp file DB, no external service)"]
    async fn roundtrip_empty_and_delta() {
        if !e2e() {
            eprintln!("skipping: PRAX_E2E not set");
            return;
        }
        // A dedicated temp file DB — no external service needed, but still
        // gated so the default `cargo test` stays hermetic.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rt.db");
        let url = format!("file:{}", path.display());

        let pool = SqlitePool::builder()
            .url(&url)
            .build()
            .await
            .expect("open sqlite");
        let eng = SqliteRawEngine::new(pool);
        for stmt in [
            "CREATE TABLE sq_rt_users (id INTEGER PRIMARY KEY, email TEXT NOT NULL UNIQUE)",
            "CREATE TABLE sq_rt_posts (id INTEGER PRIMARY KEY, title TEXT NOT NULL, author_id INTEGER NOT NULL, \
             CONSTRAINT sq_rt_posts_author_fk FOREIGN KEY (author_id) REFERENCES sq_rt_users (id))",
        ] {
            eng.raw_sql_execute(stmt, &[]).await.expect("seed ddl");
        }

        // SQLite: single INTEGER PRIMARY KEY auto-increments (rowid alias);
        // the .prax marks id @auto to match. FK is synthesized as
        // fk_<table>_<col> by the introspector (SQLite FKs are unnamed), so
        // the .prax pins that name via @relation(map:).
        let prax_v1 = r#"
        model User {
            id    Int    @id @auto
            email String @unique
            @@map("sq_rt_users")
        }
        model Post {
            id        Int    @id @auto
            title     String
            author_id Int
            author    User   @relation(fields: [author_id], references: [id], map: "fk_sq_rt_posts_author_id")
            @@map("sq_rt_posts")
        }
        "#;

        let opts = IntrospectionOptions {
            table_filter: Some("sq_rt_*".to_string()),
            ..Default::default()
        };
        let db = introspect_database("sqlite", &url, &opts)
            .await
            .expect("introspect");
        let source = schema_from_database(&db, IntrospectionConfig::default())
            .expect("map")
            .schema;

        let empty = diff_against_source(prax_v1, source.clone());
        assert!(
            empty.is_empty(),
            "expected empty diff, got: {}",
            empty.summary()
        );

        let v2 = r#"
        model User {
            id    Int     @id @auto
            email String  @unique
            bio   String?
            @@map("sq_rt_users")
        }
        model Post {
            id        Int    @id @auto
            title     String
            author_id Int
            author    User   @relation(fields: [author_id], references: [id], map: "fk_sq_rt_posts_author_id")
            @@map("sq_rt_posts")
        }
        "#;
        let delta = diff_against_source(v2, source);
        assert!(delta.create_models.is_empty(), "no new tables");
        assert_eq!(delta.alter_models.len(), 1, "one altered model");
        assert_eq!(delta.alter_models[0].add_fields[0].column_name, "bio");
    }
}

// ============================================================================
// MSSQL
// ============================================================================

#[cfg(feature = "mssql")]
mod mssql {
    use super::*;
    use prax_mssql::MssqlPool;

    fn url() -> Option<String> {
        std::env::var("MSSQL_URL").ok()
    }

    fn prax_v1() -> &'static str {
        r#"
        model User {
            id    BigInt @id
            email String @unique
            @@map("ms_rt_users")
        }
        model Post {
            id        BigInt @id
            title     String
            author_id BigInt
            author    User   @relation(fields: [author_id], references: [id], map: "ms_rt_posts_author_fk")
            @@map("ms_rt_posts")
        }
        "#
    }

    #[tokio::test]
    #[ignore = "requires running MSSQL via docker-compose"]
    async fn roundtrip_empty_and_delta() {
        if !e2e() {
            eprintln!("skipping: PRAX_E2E not set");
            return;
        }
        let url = url().expect("MSSQL_URL required");
        let pool = MssqlPool::builder()
            .connection_string(url.clone())
            .build()
            .await
            .expect("connect mssql");
        {
            let mut conn = pool.get().await.expect("conn");
            // Drop child first, then parent.
            for stmt in [
                "IF OBJECT_ID('ms_rt_posts','U') IS NOT NULL DROP TABLE ms_rt_posts",
                "IF OBJECT_ID('ms_rt_users','U') IS NOT NULL DROP TABLE ms_rt_users",
                "CREATE TABLE ms_rt_users (id BIGINT PRIMARY KEY, email NVARCHAR(255) NOT NULL UNIQUE)",
                "CREATE TABLE ms_rt_posts (id BIGINT PRIMARY KEY, title NVARCHAR(MAX) NOT NULL, author_id BIGINT NOT NULL, \
                 CONSTRAINT ms_rt_posts_author_fk FOREIGN KEY (author_id) REFERENCES ms_rt_users (id))",
            ] {
                conn.execute(stmt, &[]).await.expect("seed ddl");
            }
        }

        let opts = IntrospectionOptions {
            schema: Some("dbo".to_string()),
            table_filter: Some("ms_rt_*".to_string()),
            ..Default::default()
        };
        let db = introspect_database("mssql", &url, &opts)
            .await
            .expect("introspect");
        let source = schema_from_database(&db, IntrospectionConfig::default())
            .expect("map")
            .schema;

        let empty = diff_against_source(prax_v1(), source.clone());
        assert!(
            empty.is_empty(),
            "expected empty diff, got: {}",
            empty.summary()
        );

        let v2 = r#"
        model User {
            id    BigInt  @id
            email String  @unique
            bio   String?
            @@map("ms_rt_users")
        }
        model Post {
            id        BigInt @id
            title     String
            author_id BigInt
            author    User   @relation(fields: [author_id], references: [id], map: "ms_rt_posts_author_fk")
            @@map("ms_rt_posts")
        }
        "#;
        let delta = diff_against_source(v2, source);
        assert!(delta.create_models.is_empty(), "no new tables");
        assert_eq!(delta.alter_models.len(), 1, "one altered model");
        assert_eq!(delta.alter_models[0].add_fields[0].column_name, "bio");
    }
}
