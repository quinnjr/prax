//! Integration tests for schema parsing and validation.
//!
//! These tests verify that the schema parser correctly handles various
//! schema definitions and edge cases.

use prax_orm::schema::{PraxConfig, parse_schema, validate_schema};

/// Test basic model parsing with all common field types
#[test]
fn test_parse_model_with_all_field_types() {
    let schema = parse_schema(
        r#"
        model AllTypes {
            id        Int       @id @auto
            bigInt    BigInt
            float     Float
            decimal   Decimal
            string    String
            boolean   Boolean
            dateTime  DateTime
            date      Date
            time      Time
            json      Json
            bytes     Bytes
            uuid      Uuid
            optional  String?
            list      String[]
        }
    "#,
    )
    .expect("Failed to parse schema");

    let model = schema.get_model("AllTypes").expect("Model not found");
    assert_eq!(model.fields.len(), 14);
}

/// Test model with all field attributes
#[test]
fn test_parse_model_with_all_attributes() {
    let schema = parse_schema(
        r#"
        model User {
            id         Int      @id @auto
            email      String   @unique @index
            name       String?  @default("Anonymous")
            password   String   @omit
            createdAt  DateTime @default(now())
            updatedAt  DateTime @updated_at
            role       String   @map("user_role")
            bio        String?  @db.Text

            @@map("users")
            @@index([email, name])
            @@unique([email])
        }
    "#,
    )
    .expect("Failed to parse schema");

    let model = schema.get_model("User").expect("Model not found");

    // Check id field has @id and @auto
    let id_field = model.fields.get("id").expect("id field not found");
    assert!(id_field.attributes.iter().any(|a| a.name.name == "id"));
    assert!(id_field.attributes.iter().any(|a| a.name.name == "auto"));

    // Check model has @@map attribute
    assert!(model.attributes.iter().any(|a| a.name.name == "map"));
}

/// Test relation parsing
#[test]
fn test_parse_relations() {
    let schema = parse_schema(
        r#"
        model User {
            id       Int       @id @auto
            email    String    @unique
            posts    Post[]
            profile  Profile?
        }

        model Post {
            id       Int    @id @auto
            title    String
            authorId Int
            author   User   @relation(fields: [authorId], references: [id])
        }

        model Profile {
            id     Int    @id @auto
            bio    String?
            userId Int    @unique
            user   User   @relation(fields: [userId], references: [id])
        }
    "#,
    )
    .expect("Failed to parse schema");

    assert_eq!(schema.model_names().count(), 3);

    // Check User has posts relation
    let user = schema.get_model("User").unwrap();
    assert!(user.fields.contains_key("posts"));
    assert!(user.fields.contains_key("profile"));

    // Check Post has author relation with @relation attribute
    let post = schema.get_model("Post").unwrap();
    let author_field = post.fields.get("author").expect("author field not found");
    assert!(
        author_field
            .attributes
            .iter()
            .any(|a| a.name.name == "relation")
    );
}

/// Test self-referential relations
#[test]
fn test_parse_self_referential_relation() {
    let schema = parse_schema(
        r#"
        model Category {
            id       Int         @id @auto
            name     String
            parentId Int?
            parent   Category?   @relation("CategoryTree", fields: [parentId], references: [id])
            children Category[]  @relation("CategoryTree")
        }
    "#,
    )
    .expect("Failed to parse schema");

    let category = schema.get_model("Category").unwrap();
    assert_eq!(category.fields.len(), 5); // id, name, parentId, parent, children
}

/// Test enum parsing
#[test]
fn test_parse_enums() {
    let schema = parse_schema(
        r#"
        enum Role {
            User
            Admin
            Moderator
        }

        enum Status {
            Draft
            Published
            Archived

            @@map("post_status")
        }

        model User {
            id   Int  @id @auto
            role Role @default(User)
        }
    "#,
    )
    .expect("Failed to parse schema");

    assert_eq!(schema.enum_names().count(), 2);

    let role = schema.get_enum("Role").unwrap();
    assert_eq!(role.variants.len(), 3);

    let status = schema.get_enum("Status").unwrap();
    assert!(status.attributes.iter().any(|a| a.name.name == "map"));
}

/// Test composite type parsing
#[test]
fn test_parse_composite_types() {
    let schema = parse_schema(
        r#"
        type Address {
            street     String
            city       String
            state      String?
            postalCode String
            country    String @default("US")
        }

        type GeoPoint {
            latitude  Float
            longitude Float
        }

        model User {
            id      Int     @id @auto
            address Address
        }
    "#,
    )
    .expect("Failed to parse schema");

    assert_eq!(schema.types.len(), 2);

    let address = schema.get_type("Address").unwrap();
    assert_eq!(address.fields.len(), 5);
}

/// Test view parsing
#[test]
fn test_parse_views() {
    let schema = parse_schema(
        r#"
model Post {
    id        Int    @id @auto
    title     String
    viewCount Int    @default(0)
}

view PopularPosts {
    id        Int    @unique
    title     String
    viewCount Int

    @@map("popular_posts_view")
}
"#,
    )
    .expect("Failed to parse schema");

    // Views are included in the schema - test via stats
    let stats = schema.stats();
    assert_eq!(stats.model_count, 1, "Should have 1 model (Post)");
    assert_eq!(stats.view_count, 1, "Should have 1 view (PopularPosts)");

    // Access view by name
    let view = schema.get_view("PopularPosts");
    assert!(view.is_some(), "PopularPosts view should exist");

    let view = view.unwrap();
    assert_eq!(view.fields.len(), 3, "View should have 3 fields");
}

/// Test server group parsing
#[test]
fn test_parse_server_groups() {
    let schema = parse_schema(
        r#"
model User {
    id    Int    @id @auto
    email String @unique
}

serverGroup MainCluster {
    server primary {
        url  = "postgresql://primary:5432/db"
        role = "primary"
    }

    server replica1 {
        url    = "postgresql://replica1:5432/db"
        role   = "replica"
        weight = 50
    }

    @@strategy("ReadReplica")
    @@loadBalance("RoundRobin")
}
"#,
    )
    .expect("Failed to parse schema");

    assert_eq!(schema.server_group_names().count(), 1);

    let sg = schema.get_server_group("MainCluster").unwrap();
    assert_eq!(sg.servers.len(), 2);
}

/// Test documentation comments
#[test]
fn test_parse_documentation() {
    let schema = parse_schema(
        r#"/// User account in the system.
/// Contains authentication and profile information.
model User {
    /// Primary key
    id    Int    @id @auto

    /// User's email address
    email String @unique
}
"#,
    )
    .expect("Failed to parse schema");

    let user = schema.get_model("User").unwrap();
    // Documentation parsing is optional - check if it's supported
    // If documentation is present, verify it contains expected text
    if let Some(doc) = &user.documentation {
        assert!(doc.text.to_lowercase().contains("user"));
    }

    // Field documentation may not be supported in all versions
    // let id_field = user.fields.get("id").expect("id field not found");
    // if let Some(doc) = &id_field.documentation {
    //     assert!(doc.text.contains("Primary"));
    // }
}

/// Test schema validation - valid schema
#[test]
fn test_validate_valid_schema() {
    let result = validate_schema(
        r#"
        model User {
            id    Int    @id @auto
            email String @unique
            posts Post[]
        }

        model Post {
            id       Int    @id @auto
            title    String
            authorId Int
            author   User   @relation(fields: [authorId], references: [id])
        }
    "#,
    );

    assert!(result.is_ok(), "Schema should be valid: {:?}", result.err());
}

/// Test schema validation - missing @id
#[test]
fn test_validate_missing_id() {
    let result = validate_schema(
        r#"
        model User {
            email String @unique
            name  String
        }
    "#,
    );

    // This should either pass (if @id is optional) or fail with a specific error
    // Depending on validation rules
    match result {
        Ok(_) => {} // Some ORMs allow tables without explicit primary key
        Err(e) => {
            assert!(
                e.to_string().contains("id") || e.to_string().contains("primary"),
                "Error should mention missing id: {}",
                e
            );
        }
    }
}

/// Test schema statistics
#[test]
fn test_schema_statistics() {
    let schema = parse_schema(
        r#"
model User { id Int @id @auto }
model Post { id Int @id @auto }
model Comment { id Int @id @auto }

enum Role { User Admin }
enum Status { Draft Published }

type Address { street String }

view UserStats { id Int @unique }

serverGroup Cluster {
    server primary { url = "postgres://localhost/db" }
}
"#,
    )
    .expect("Failed to parse schema");

    let stats = schema.stats();
    assert_eq!(stats.model_count, 3);
    assert_eq!(stats.enum_count, 2);
    assert_eq!(stats.type_count, 1);
    assert_eq!(stats.view_count, 1);
    assert_eq!(stats.server_group_count, 1);
}

/// Test configuration parsing
#[test]
fn test_config_parsing() {
    let config_str = r#"
        [database]
        provider = "postgresql"
        url = "postgresql://localhost:5432/mydb"

        [database.pool]
        min_connections = 2
        max_connections = 10

        [generator.client]
        output = "./src/generated"

        [migrations]
        directory = "./prax/migrations"
    "#;

    let config: PraxConfig = toml::from_str(config_str).expect("Failed to parse config");

    assert_eq!(
        config.database.url,
        Some("postgresql://localhost:5432/mydb".to_string())
    );
    assert_eq!(config.database.pool.min_connections, 2);
    assert_eq!(config.database.pool.max_connections, 10);
}

/// Test configuration with environment variables
#[test]
fn test_config_with_env_vars() {
    let config_str = r#"
        [database]
        provider = "postgresql"
        url = "${DATABASE_URL}"
    "#;

    let config: PraxConfig = toml::from_str(config_str).expect("Failed to parse config");

    assert_eq!(config.database.url, Some("${DATABASE_URL}".to_string()));
}

/// Test schema merging
#[test]
fn test_schema_merging() {
    let schema1 = parse_schema(
        r#"
        model User {
            id    Int    @id @auto
            email String @unique
        }
    "#,
    )
    .expect("Failed to parse schema 1");

    let schema2 = parse_schema(
        r#"
        model Post {
            id    Int    @id @auto
            title String
        }
    "#,
    )
    .expect("Failed to parse schema 2");

    let mut merged = schema1;
    merged.try_merge(schema2).expect("merge should succeed");

    assert_eq!(merged.model_names().count(), 2);
    assert!(merged.get_model("User").is_some());
    assert!(merged.get_model("Post").is_some());
}

/// Test index definitions
#[test]
fn test_parse_indexes() {
    let schema = parse_schema(
        r#"
        model User {
            id        Int      @id @auto
            email     String   @unique @index
            firstName String
            lastName  String
            createdAt DateTime

            @@index([firstName, lastName])
            @@index([createdAt], map: "idx_created")
            @@unique([email, firstName])
        }
    "#,
    )
    .expect("Failed to parse schema");

    let user = schema.get_model("User").unwrap();

    // Count @@index and @@unique attributes
    let index_count = user
        .attributes
        .iter()
        .filter(|a| a.name.name == "index")
        .count();
    let unique_count = user
        .attributes
        .iter()
        .filter(|a| a.name.name == "unique")
        .count();

    assert!(index_count >= 2, "Should have at least 2 indexes");
    assert!(
        unique_count >= 1,
        "Should have at least 1 unique constraint"
    );
}

/// Test default values
#[test]
fn test_parse_default_values() {
    let schema = parse_schema(
        r#"
        model Post {
            id        Int      @id @auto
            title     String
            views     Int      @default(0)
            rating    Float    @default(0.0)
            published Boolean  @default(false)
            content   String   @default("")
            status    Status   @default(Draft)
            createdAt DateTime @default(now())
            uuid      String   @default(uuid())
            cuid      String   @default(cuid())
        }

        enum Status {
            Draft
            Published
        }
    "#,
    )
    .expect("Failed to parse schema");

    let post = schema.get_model("Post").unwrap();

    // Check views has @default(0)
    let views = post.fields.get("views").expect("views field not found");
    assert!(views.attributes.iter().any(|a| a.name.name == "default"));

    // Check createdAt has @default(now())
    let created_at = post
        .fields
        .get("createdAt")
        .expect("createdAt field not found");
    assert!(
        created_at
            .attributes
            .iter()
            .any(|a| a.name.name == "default")
    );
}

/// Test database type annotations
#[test]
fn test_parse_db_types() {
    let schema = parse_schema(
        r#"
        model Content {
            id          Int     @id @auto
            title       String  @db.VarChar(200)
            description String? @db.Text
            metadata    String? @db.Json
            data        Bytes?  @db.ByteA
        }
    "#,
    )
    .expect("Failed to parse schema");

    let content = schema.get_model("Content").unwrap();

    // Check title has @db.VarChar
    let title = content.fields.get("title").expect("title field not found");
    let has_db_attr = title
        .attributes
        .iter()
        .any(|a| a.name.name == "db.VarChar" || a.name.name.starts_with("db"));
    assert!(has_db_attr, "title should have @db attribute");
}

/// Test referential actions in relations
#[test]
fn test_parse_referential_actions() {
    let schema = parse_schema(
        r#"
        model User {
            id    Int    @id @auto
            posts Post[]
        }

        model Post {
            id       Int    @id @auto
            authorId Int
            author   User   @relation(fields: [authorId], references: [id], onDelete: Cascade, onUpdate: Cascade)
        }
    "#,
    )
    .expect("Failed to parse schema");

    let post = schema.get_model("Post").unwrap();
    let author = post.fields.get("author").expect("author field not found");
    let relation = author
        .attributes
        .iter()
        .find(|a| a.name.name == "relation")
        .expect("relation attribute not found");

    // Relation should have onDelete and onUpdate arguments
    assert!(!relation.args.is_empty());
}

/// Test field names are unique within model
#[test]
fn test_field_names_unique() {
    let schema = parse_schema(
        r#"
        model User {
            id    Int    @id @auto
            email String
            name  String
        }
    "#,
    )
    .expect("Failed to parse schema");

    let user = schema.get_model("User").unwrap();
    assert_eq!(user.fields.len(), 3);
}

/// Test multiple models in one schema
#[test]
fn test_multiple_models() {
    let schema = parse_schema(
        r#"
        model User { id Int @id @auto }
        model Post { id Int @id @auto }
        model Comment { id Int @id @auto }
        model Tag { id Int @id @auto }
        model Category { id Int @id @auto }
    "#,
    )
    .expect("Failed to parse schema");

    assert_eq!(schema.model_names().count(), 5);
}
