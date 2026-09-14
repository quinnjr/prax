//! Map a `prax-query` introspection result to a `prax_schema::Schema`.
//!
//! `prax migrate dev`/`diff` need the *current* database structure expressed
//! as a `prax_schema::Schema` so it can be fed to `prax_migrate::SchemaDiffer`
//! as the diff **source**. The database driver (see [`crate::commands::introspect`])
//! produces a [`prax_query::introspection::DatabaseSchema`]; the migration
//! engine's [`prax_migrate::SchemaBuilder`] already knows how to turn its own
//! raw introspection structs (`TableInfo`/`ColumnInfo`/`ConstraintInfo`/
//! `IndexInfo`/`EnumInfo`) into a `Schema`, complete with foreign-key relation
//! synthesis, `@map`, `@@index`/`@@unique`, and primary-key detection — and
//! that output is proven to round-trip cleanly with the differ.
//!
//! This module bridges the two: it translates the query-layer `DatabaseSchema`
//! into the migration engine's introspection structs and runs them through
//! `SchemaBuilder`. Keeping the translation here (rather than in `prax-migrate`)
//! preserves the crate layering — `prax-migrate` depends only on `prax-schema`,
//! while the CLI already depends on both `prax-migrate` and `prax-query`.
//!
//! ## Round-trip fidelity
//!
//! The differ compares fields by name (SQL type, nullability, default),
//! foreign keys by constraint name, and indexes by name. For a database that
//! already matches its `.prax` schema to diff to *empty* (the "no spurious
//! churn" property), the mapped source must reproduce the same constructs the
//! target schema produces. Two mismatches are inherent to reverse-engineering
//! and documented as limitations rather than papered over:
//!
//! - **Field vs column names.** A `.prax` field `authorId` mapped to column
//!   `author_id` reverse-engineers to a field named `author_id`. Schemas whose
//!   field names differ from their column names will show spurious add/drop
//!   churn; schemas whose field names match their columns (snake_case
//!   throughout) round-trip cleanly.
//! - **Foreign-key constraint names.** The target auto-derives `fk_<table>_<cols>`
//!   unless the relation carries `@relation(map: "...")`. Introspection reports
//!   the *real* database constraint name. When they differ the differ proposes
//!   dropping/adding the FK; pin the name with `@relation(map: ...)` to avoid it.

use prax_migrate::{
    ColumnInfo as MigrateColumn, ConstraintInfo, EnumInfo as MigrateEnum,
    IndexInfo as MigrateIndex, IntrospectionConfig, IntrospectionResult, SchemaBuilder,
    TableInfo as MigrateTable,
};
use prax_query::introspection::{
    ColumnInfo, DatabaseSchema, ForeignKeyInfo, IndexInfo, NormalizedType, ReferentialAction,
    TableInfo,
};

use crate::error::CliResult;

/// Translate a query-layer [`DatabaseSchema`] into a `prax_schema::Schema`
/// suitable as a diff source, using the migration engine's `SchemaBuilder`.
///
/// `config` controls which tables are included/excluded (e.g. a foreign
/// runner's migration bookkeeping table); pass
/// [`IntrospectionConfig::default`] for the standard exclusions.
pub fn schema_from_database(
    db: &DatabaseSchema,
    config: IntrospectionConfig,
) -> CliResult<IntrospectionResult> {
    let mut builder = SchemaBuilder::new(config).with_tables(map_tables(db));

    for table in &db.tables {
        builder = builder
            .with_columns(&table.name, map_columns(&table.columns))
            .with_constraints(&table.name, map_constraints(table))
            .with_indexes(&table.name, map_indexes(&table.indexes, &table.name));
    }

    builder = builder.with_enums(map_enums(db));

    builder.build().map_err(|e| {
        crate::error::CliError::Migration(format!(
            "Failed to build schema from database introspection: {e}"
        ))
    })
}

/// Map every discovered table (base tables only; the query layer's `db pull`
/// separates views into `DatabaseSchema::views`, so anything in `tables` is a
/// base table) to the engine's `TableInfo`.
fn map_tables(db: &DatabaseSchema) -> Vec<MigrateTable> {
    db.tables
        .iter()
        .map(|t| MigrateTable {
            name: t.name.clone(),
            schema: t
                .schema
                .clone()
                .or_else(|| db.schema.clone())
                .unwrap_or_else(|| "public".to_string()),
            table_type: "BASE TABLE".to_string(),
            comment: t.comment.clone(),
        })
        .collect()
}

/// Map columns, deriving a canonical `udt_name` from the normalized type so
/// the engine's `sql_type_to_prax` lands on the same `ScalarType` the target
/// schema produces.
fn map_columns(columns: &[ColumnInfo]) -> Vec<MigrateColumn> {
    columns
        .iter()
        .enumerate()
        .map(|(i, c)| MigrateColumn {
            name: c.name.clone(),
            data_type: c.db_type.clone(),
            udt_name: udt_name_for(&c.normalized_type, &c.db_type),
            character_maximum_length: c.max_length,
            numeric_precision: c.precision,
            is_nullable: c.nullable,
            column_default: c.default.clone(),
            ordinal_position: i as i32,
            comment: c.comment.clone(),
        })
        .collect()
}

/// Derive a PostgreSQL `udt_name`-equivalent for a normalized type.
///
/// The engine's `SchemaBuilder::sql_type_to_prax` matches on `udt_name`
/// first (falling back to `data_type`). Mapping the normalized type to the
/// canonical short udt string it recognizes keeps type resolution robust
/// even when `db_type` carries a display form (e.g. "character varying").
fn udt_name_for(normalized: &NormalizedType, db_type: &str) -> String {
    match normalized {
        NormalizedType::Int | NormalizedType::SmallInt => "int4".to_string(),
        NormalizedType::BigInt => "int8".to_string(),
        NormalizedType::Float => "float4".to_string(),
        NormalizedType::Double => "float8".to_string(),
        NormalizedType::Decimal { .. } => "numeric".to_string(),
        NormalizedType::String
        | NormalizedType::Text
        | NormalizedType::VarChar { .. }
        | NormalizedType::Char { .. } => "text".to_string(),
        NormalizedType::Bytes => "bytea".to_string(),
        NormalizedType::Boolean => "bool".to_string(),
        NormalizedType::DateTime | NormalizedType::Timestamp => "timestamptz".to_string(),
        NormalizedType::Date => "date".to_string(),
        NormalizedType::Time => "time".to_string(),
        NormalizedType::Json => "jsonb".to_string(),
        NormalizedType::Uuid => "uuid".to_string(),
        // Enum reference: the engine matches the udt_name against known enum
        // names, so the enum type name must be passed through verbatim.
        NormalizedType::Enum(name) => name.clone(),
        // Arrays have no first-class Prax scalar; the engine treats the
        // "ARRAY" data_type as Json. Fall through to db_type so its fallback
        // path applies.
        NormalizedType::Array(_) => "ARRAY".to_string(),
        NormalizedType::Unknown(_) => db_type.to_string(),
    }
}

/// Map a table's primary key, foreign keys, and unique constraints to the
/// engine's flat `ConstraintInfo` list. Single-column primary keys become
/// `@id`; multi-column primary keys are carried as one PRIMARY KEY constraint
/// (the engine reads all its columns). Unique constraints and foreign keys are
/// mapped through so they are not re-proposed by the differ.
fn map_constraints(table: &TableInfo) -> Vec<ConstraintInfo> {
    let mut constraints = Vec::new();

    if !table.primary_key.is_empty() {
        constraints.push(ConstraintInfo {
            name: format!("{}_pkey", table.name),
            constraint_type: "PRIMARY KEY".to_string(),
            table_name: table.name.clone(),
            columns: table.primary_key.clone(),
            referenced_table: None,
            referenced_columns: None,
            on_delete: None,
            on_update: None,
        });
    }

    for uc in &table.unique_constraints {
        constraints.push(ConstraintInfo {
            name: uc.name.clone(),
            constraint_type: "UNIQUE".to_string(),
            table_name: table.name.clone(),
            columns: uc.columns.clone(),
            referenced_table: None,
            referenced_columns: None,
            on_delete: None,
            on_update: None,
        });
    }

    for fk in &table.foreign_keys {
        constraints.push(map_foreign_key(fk, &table.name));
    }

    constraints
}

/// Map a foreign key, translating referential actions to the SQL keyword
/// form the engine expects (`NoAction` collapses to `None` — the SQL default
/// — so it is not rendered redundantly).
fn map_foreign_key(fk: &ForeignKeyInfo, table_name: &str) -> ConstraintInfo {
    ConstraintInfo {
        name: fk.name.clone(),
        constraint_type: "FOREIGN KEY".to_string(),
        table_name: table_name.to_string(),
        columns: fk.columns.clone(),
        referenced_table: Some(fk.referenced_table.clone()),
        referenced_columns: Some(fk.referenced_columns.clone()),
        on_delete: referential_action_sql(fk.on_delete),
        on_update: referential_action_sql(fk.on_update),
    }
}

/// Render a referential action as the SQL keyword the engine stores, or
/// `None` for the default `NO ACTION` (which needs no clause).
fn referential_action_sql(action: ReferentialAction) -> Option<String> {
    match action {
        ReferentialAction::NoAction => None,
        ReferentialAction::Restrict => Some("RESTRICT".to_string()),
        ReferentialAction::Cascade => Some("CASCADE".to_string()),
        ReferentialAction::SetNull => Some("SET NULL".to_string()),
        ReferentialAction::SetDefault => Some("SET DEFAULT".to_string()),
    }
}

/// Map indexes, flattening the query layer's `IndexColumn` (which carries sort
/// order/nulls position) to the engine's plain column-name list.
fn map_indexes(indexes: &[IndexInfo], table_name: &str) -> Vec<MigrateIndex> {
    indexes
        .iter()
        .map(|idx| MigrateIndex {
            name: idx.name.clone(),
            table_name: table_name.to_string(),
            columns: idx.columns.iter().map(|c| c.name.clone()).collect(),
            is_unique: idx.is_unique,
            is_primary: idx.is_primary,
            index_method: idx
                .index_type
                .clone()
                .unwrap_or_else(|| "btree".to_string()),
        })
        .collect()
}

/// Map enum types, carrying the schema-qualified name through.
fn map_enums(db: &DatabaseSchema) -> Vec<MigrateEnum> {
    db.enums
        .iter()
        .map(|e| MigrateEnum {
            name: e.name.clone(),
            values: e.values.clone(),
            schema: e
                .schema
                .clone()
                .or_else(|| db.schema.clone())
                .unwrap_or_else(|| "public".to_string()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use prax_query::introspection::{EnumInfo, IndexColumn, UniqueConstraint};
    use prax_schema::ast::{FieldType, ScalarType, TypeModifier};

    fn column(name: &str, normalized: NormalizedType, nullable: bool) -> ColumnInfo {
        ColumnInfo {
            name: name.to_string(),
            db_type: "".to_string(),
            normalized_type: normalized,
            nullable,
            ..Default::default()
        }
    }

    #[test]
    fn udt_name_maps_normalized_types_to_recognized_short_names() {
        assert_eq!(udt_name_for(&NormalizedType::Int, ""), "int4");
        assert_eq!(udt_name_for(&NormalizedType::BigInt, ""), "int8");
        assert_eq!(udt_name_for(&NormalizedType::Boolean, ""), "bool");
        assert_eq!(udt_name_for(&NormalizedType::DateTime, ""), "timestamptz");
        assert_eq!(udt_name_for(&NormalizedType::Uuid, ""), "uuid");
        assert_eq!(udt_name_for(&NormalizedType::Json, ""), "jsonb");
        assert_eq!(
            udt_name_for(&NormalizedType::VarChar { length: Some(255) }, ""),
            "text"
        );
        assert_eq!(
            udt_name_for(&NormalizedType::Enum("Role".to_string()), ""),
            "Role"
        );
        // Unknown falls back to the raw db_type so the engine's data_type path applies.
        assert_eq!(
            udt_name_for(
                &NormalizedType::Unknown("geography".to_string()),
                "geography"
            ),
            "geography"
        );
    }

    #[test]
    fn referential_actions_map_to_sql_keywords() {
        assert_eq!(referential_action_sql(ReferentialAction::NoAction), None);
        assert_eq!(
            referential_action_sql(ReferentialAction::Cascade),
            Some("CASCADE".to_string())
        );
        assert_eq!(
            referential_action_sql(ReferentialAction::SetNull),
            Some("SET NULL".to_string())
        );
    }

    #[test]
    fn maps_a_simple_table_to_a_model_with_columns_and_pk() {
        let db = DatabaseSchema {
            name: "db".to_string(),
            schema: Some("public".to_string()),
            tables: vec![TableInfo {
                name: "users".to_string(),
                schema: Some("public".to_string()),
                columns: vec![
                    column("id", NormalizedType::BigInt, false),
                    column("email", NormalizedType::Text, false),
                    column("name", NormalizedType::Text, true),
                ],
                primary_key: vec!["id".to_string()],
                ..Default::default()
            }],
            ..Default::default()
        };

        let result = schema_from_database(&db, IntrospectionConfig::default()).unwrap();
        let model = result.schema.get_model("Users").expect("Users model");

        let id = model.get_field("id").expect("id field");
        assert!(id.has_attribute("id"));
        assert!(matches!(
            &id.field_type,
            FieldType::Scalar(ScalarType::BigInt)
        ));

        let email = model.get_field("email").expect("email field");
        assert_eq!(email.modifier, TypeModifier::Required);
        assert!(matches!(
            &email.field_type,
            FieldType::Scalar(ScalarType::String)
        ));

        let name = model.get_field("name").expect("name field");
        assert_eq!(name.modifier, TypeModifier::Optional);
    }

    #[test]
    fn maps_foreign_keys_to_relation_fields() {
        let db = DatabaseSchema {
            name: "db".to_string(),
            schema: Some("public".to_string()),
            tables: vec![
                TableInfo {
                    name: "users".to_string(),
                    columns: vec![column("id", NormalizedType::BigInt, false)],
                    primary_key: vec!["id".to_string()],
                    ..Default::default()
                },
                TableInfo {
                    name: "posts".to_string(),
                    columns: vec![
                        column("id", NormalizedType::BigInt, false),
                        column("author_id", NormalizedType::BigInt, false),
                    ],
                    primary_key: vec!["id".to_string()],
                    foreign_keys: vec![ForeignKeyInfo {
                        name: "posts_author_id_fkey".to_string(),
                        columns: vec!["author_id".to_string()],
                        referenced_table: "users".to_string(),
                        referenced_schema: None,
                        referenced_columns: vec!["id".to_string()],
                        on_delete: ReferentialAction::Cascade,
                        on_update: ReferentialAction::NoAction,
                    }],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let result = schema_from_database(&db, IntrospectionConfig::default()).unwrap();
        let posts = result.schema.get_model("Posts").expect("Posts model");
        let author = posts.get_field("author").expect("relation field");
        let rel = author
            .extract_attributes()
            .relation
            .expect("@relation present");
        assert_eq!(rel.fields, ["author_id"]);
        assert_eq!(rel.references, ["id"]);
    }

    #[test]
    fn maps_enums_and_enum_typed_columns() {
        let db = DatabaseSchema {
            name: "db".to_string(),
            schema: Some("public".to_string()),
            tables: vec![TableInfo {
                name: "users".to_string(),
                columns: vec![
                    column("id", NormalizedType::BigInt, false),
                    column("role", NormalizedType::Enum("role".to_string()), false),
                ],
                primary_key: vec!["id".to_string()],
                ..Default::default()
            }],
            enums: vec![EnumInfo {
                name: "role".to_string(),
                schema: Some("public".to_string()),
                values: vec!["ADMIN".to_string(), "USER".to_string()],
            }],
            ..Default::default()
        };

        let result = schema_from_database(&db, IntrospectionConfig::default()).unwrap();
        assert!(result.schema.get_enum("Role").is_some());
        let users = result.schema.get_model("Users").expect("Users model");
        let role = users.get_field("role").expect("role field");
        assert!(matches!(&role.field_type, FieldType::Enum(_)));
    }

    #[test]
    fn maps_multi_column_unique_index() {
        let db = DatabaseSchema {
            name: "db".to_string(),
            schema: Some("public".to_string()),
            tables: vec![TableInfo {
                name: "memberships".to_string(),
                columns: vec![
                    column("team_id", NormalizedType::BigInt, false),
                    column("user_id", NormalizedType::BigInt, false),
                ],
                primary_key: vec!["team_id".to_string(), "user_id".to_string()],
                indexes: vec![IndexInfo {
                    name: "uq_membership".to_string(),
                    columns: vec![
                        IndexColumn {
                            name: "team_id".to_string(),
                            ..Default::default()
                        },
                        IndexColumn {
                            name: "user_id".to_string(),
                            ..Default::default()
                        },
                    ],
                    is_unique: true,
                    is_primary: false,
                    index_type: Some("btree".to_string()),
                    filter: None,
                }],
                ..Default::default()
            }],
            ..Default::default()
        };

        let result = schema_from_database(&db, IntrospectionConfig::default()).unwrap();
        let model = result
            .schema
            .get_model("Memberships")
            .expect("Memberships model");
        // Composite PK -> both id fields carry @id.
        assert!(model.get_field("team_id").unwrap().has_attribute("id"));
        assert!(model.get_field("user_id").unwrap().has_attribute("id"));
        // Multi-column unique index -> @@unique.
        assert!(model.get_attribute("unique").is_some());
    }

    #[test]
    fn excluded_tables_are_skipped() {
        let db = DatabaseSchema {
            name: "db".to_string(),
            schema: Some("public".to_string()),
            tables: vec![TableInfo {
                name: "_prax_migrations".to_string(),
                columns: vec![column("id", NormalizedType::BigInt, false)],
                primary_key: vec!["id".to_string()],
                ..Default::default()
            }],
            ..Default::default()
        };

        let result = schema_from_database(&db, IntrospectionConfig::default()).unwrap();
        assert!(result.schema.get_model("PraxMigrations").is_none());
        assert!(result.schema.models.is_empty());
    }

    // -- Introspection round-trip (the single most important property) -------
    //
    // A database already at the target schema must diff to *empty* — no
    // spurious churn. This is exercised purely in-memory (no live DB) by
    // constructing a DatabaseSchema that mirrors a `.prax`, mapping it to the
    // diff source, and diffing the parsed `.prax` (target) against it. It
    // doubles as the foreign-history case: the mapped source is derived from
    // real structure, never from prax migration history.

    /// A `.prax` whose field names and constraint names line up with what a
    /// snake_case Postgres database reports, so the round-trip is clean. FK
    /// constraint name is pinned with `@relation(map:)` to match the DB.
    const ROUNDTRIP_PRAX: &str = r#"
        model User {
            id    BigInt @id
            email String @unique

            @@map("users")
        }

        model Post {
            id        BigInt @id
            title     String
            author_id BigInt
            author    User   @relation(fields: [author_id], references: [id], map: "posts_author_id_fkey")

            @@map("posts")
        }
    "#;

    /// The `DatabaseSchema` a Postgres introspection of `ROUNDTRIP_PRAX` would
    /// produce (snake_case columns, real pkey/fkey constraint names).
    fn roundtrip_database() -> DatabaseSchema {
        DatabaseSchema {
            name: "db".to_string(),
            schema: Some("public".to_string()),
            tables: vec![
                TableInfo {
                    name: "users".to_string(),
                    schema: Some("public".to_string()),
                    columns: vec![
                        column("id", NormalizedType::BigInt, false),
                        column("email", NormalizedType::Text, false),
                    ],
                    primary_key: vec!["id".to_string()],
                    unique_constraints: vec![UniqueConstraint {
                        name: "users_email_key".to_string(),
                        columns: vec!["email".to_string()],
                    }],
                    indexes: vec![IndexInfo {
                        name: "users_email_key".to_string(),
                        columns: vec![IndexColumn {
                            name: "email".to_string(),
                            ..Default::default()
                        }],
                        is_unique: true,
                        is_primary: false,
                        index_type: Some("btree".to_string()),
                        filter: None,
                    }],
                    ..Default::default()
                },
                TableInfo {
                    name: "posts".to_string(),
                    schema: Some("public".to_string()),
                    columns: vec![
                        column("id", NormalizedType::BigInt, false),
                        column("title", NormalizedType::Text, false),
                        column("author_id", NormalizedType::BigInt, false),
                    ],
                    primary_key: vec!["id".to_string()],
                    foreign_keys: vec![ForeignKeyInfo {
                        name: "posts_author_id_fkey".to_string(),
                        columns: vec!["author_id".to_string()],
                        referenced_table: "users".to_string(),
                        referenced_schema: None,
                        referenced_columns: vec!["id".to_string()],
                        on_delete: ReferentialAction::NoAction,
                        on_update: ReferentialAction::NoAction,
                    }],
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn introspected_source_matching_target_yields_empty_diff() {
        use prax_migrate::SchemaDiffer;

        let target = prax_schema::parse_schema(ROUNDTRIP_PRAX).unwrap();
        let source = schema_from_database(&roundtrip_database(), IntrospectionConfig::default())
            .unwrap()
            .schema;

        let diff = SchemaDiffer::new(target)
            .with_source(source)
            .diff()
            .unwrap();
        assert!(
            diff.is_empty(),
            "expected no spurious churn, got: {}",
            diff.summary()
        );
    }

    #[test]
    fn introspected_source_missing_column_yields_only_that_delta() {
        // Foreign-history case: the DB (mapped source) predates a new column;
        // diffing the newer .prax against it must yield exactly one added
        // field and nothing else.
        use prax_migrate::SchemaDiffer;

        let mut db = roundtrip_database();
        // Target gains a `bio` column on users that the DB does not have.
        let target = prax_schema::parse_schema(
            r#"
            model User {
                id    BigInt  @id
                email String  @unique
                bio   String?

                @@map("users")
            }

            model Post {
                id        BigInt @id
                title     String
                author_id BigInt
                author    User   @relation(fields: [author_id], references: [id], map: "posts_author_id_fkey")

                @@map("posts")
            }
            "#,
        )
        .unwrap();
        // Ensure the DB users table lacks `bio`.
        db.tables[0].columns.retain(|c| c.name != "bio");

        let source = schema_from_database(&db, IntrospectionConfig::default())
            .unwrap()
            .schema;
        let diff = SchemaDiffer::new(target)
            .with_source(source)
            .diff()
            .unwrap();

        assert!(diff.create_models.is_empty(), "no new tables expected");
        assert_eq!(diff.alter_models.len(), 1, "exactly one altered model");
        let alter = &diff.alter_models[0];
        assert_eq!(alter.table_name, "users");
        assert_eq!(alter.add_fields.len(), 1);
        assert_eq!(alter.add_fields[0].column_name, "bio");
        assert!(alter.drop_fields.is_empty());
    }
}
