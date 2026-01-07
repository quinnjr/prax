//! Benchmarks for AST operations including creation, traversal, and manipulation.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use prax_schema::ast::{
    Attribute, AttributeArg, AttributeValue, Enum, EnumVariant, Field, FieldType, Ident, Model,
    Relation, RelationType, ScalarType, Schema, Span, TypeModifier, View,
};

// ============================================================================
// Helper Functions
// ============================================================================

fn make_span() -> Span {
    Span::new(0, 0)
}

fn make_ident(name: &str) -> Ident {
    Ident::new(name, make_span())
}

fn make_attribute(name: &str) -> Attribute {
    Attribute::simple(make_ident(name), make_span())
}

fn make_field(name: &str, scalar: ScalarType) -> Field {
    Field::new(
        make_ident(name),
        FieldType::Scalar(scalar),
        TypeModifier::Required,
        vec![],
        make_span(),
    )
}

// ============================================================================
// Model Creation Benchmarks
// ============================================================================

fn bench_model_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("model_creation");

    group.bench_function("create_empty_model", |b| {
        b.iter(|| black_box(Model::new(make_ident("User"), make_span())))
    });

    group.bench_function("create_model_with_5_fields", |b| {
        b.iter(|| {
            let mut model = Model::new(make_ident("User"), make_span());
            model.add_field(make_field("id", ScalarType::Int));
            model.add_field(make_field("email", ScalarType::String));
            model.add_field(make_field("name", ScalarType::String));
            model.add_field(make_field("age", ScalarType::Int));
            model.add_field(make_field("active", ScalarType::Boolean));
            black_box(model)
        })
    });

    group.bench_function("create_model_with_10_fields", |b| {
        b.iter(|| {
            let mut model = Model::new(make_ident("User"), make_span());
            for i in 0..10 {
                model.add_field(make_field(&format!("field_{}", i), ScalarType::String));
            }
            black_box(model)
        })
    });

    group.bench_function("create_model_with_attributes", |b| {
        b.iter(|| {
            let mut model = Model::new(make_ident("User"), make_span());
            model.attributes.push(make_attribute("map"));
            model.attributes.push(make_attribute("index"));

            let mut field = make_field("id", ScalarType::Int);
            field.attributes.push(make_attribute("id"));
            field.attributes.push(make_attribute("auto"));
            model.add_field(field);

            let mut email_field = make_field("email", ScalarType::String);
            email_field.attributes.push(make_attribute("unique"));
            model.add_field(email_field);

            black_box(model)
        })
    });

    group.finish();
}

// ============================================================================
// Schema Building Benchmarks
// ============================================================================

fn bench_schema_building(c: &mut Criterion) {
    let mut group = c.benchmark_group("schema_building");

    group.bench_function("create_empty_schema", |b| {
        b.iter(|| black_box(Schema::new()))
    });

    group.bench_function("schema_with_5_models", |b| {
        b.iter(|| {
            let mut schema = Schema::new();
            for i in 0..5 {
                let mut model = Model::new(make_ident(&format!("Model{}", i)), make_span());
                model.add_field(make_field("id", ScalarType::Int));
                model.add_field(make_field("name", ScalarType::String));
                schema.add_model(model);
            }
            black_box(schema)
        })
    });

    for count in [10, 25, 50, 100].iter() {
        group.throughput(Throughput::Elements(*count as u64));
        group.bench_with_input(
            BenchmarkId::new("schema_with_n_models", count),
            count,
            |b, &count| {
                b.iter(|| {
                    let mut schema = Schema::new();
                    for i in 0..count {
                        let mut model = Model::new(make_ident(&format!("Model{}", i)), make_span());
                        model.add_field(make_field("id", ScalarType::Int));
                        model.add_field(make_field("name", ScalarType::String));
                        schema.add_model(model);
                    }
                    black_box(schema)
                })
            },
        );
    }

    group.finish();
}

// ============================================================================
// Enum Operations Benchmarks
// ============================================================================

fn bench_enum_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("enum_operations");

    group.bench_function("create_enum_3_variants", |b| {
        b.iter(|| {
            let mut enum_def = Enum::new(make_ident("Status"), make_span());
            enum_def.add_variant(EnumVariant::new(make_ident("Active"), make_span()));
            enum_def.add_variant(EnumVariant::new(make_ident("Inactive"), make_span()));
            enum_def.add_variant(EnumVariant::new(make_ident("Pending"), make_span()));
            black_box(enum_def)
        })
    });

    group.bench_function("create_enum_10_variants", |b| {
        b.iter(|| {
            let mut enum_def = Enum::new(make_ident("LargeEnum"), make_span());
            for i in 0..10 {
                enum_def.add_variant(EnumVariant::new(
                    make_ident(&format!("Variant{}", i)),
                    make_span(),
                ));
            }
            black_box(enum_def)
        })
    });

    group.finish();
}

// ============================================================================
// Field Type Benchmarks
// ============================================================================

fn bench_field_types(c: &mut Criterion) {
    let mut group = c.benchmark_group("field_types");

    group.bench_function("scalar_int", |b| {
        b.iter(|| black_box(FieldType::Scalar(ScalarType::Int)))
    });

    group.bench_function("scalar_string", |b| {
        b.iter(|| black_box(FieldType::Scalar(ScalarType::String)))
    });

    group.bench_function("scalar_datetime", |b| {
        b.iter(|| black_box(FieldType::Scalar(ScalarType::DateTime)))
    });

    group.bench_function("model_reference", |b| {
        b.iter(|| black_box(FieldType::Model("User".into())))
    });

    group.bench_function("enum_reference", |b| {
        b.iter(|| black_box(FieldType::Enum("Status".into())))
    });

    group.finish();
}

// ============================================================================
// Type Modifier Benchmarks
// ============================================================================

fn bench_type_modifiers(c: &mut Criterion) {
    let mut group = c.benchmark_group("type_modifiers");

    group.bench_function("required", |b| b.iter(|| black_box(TypeModifier::Required)));

    group.bench_function("optional", |b| b.iter(|| black_box(TypeModifier::Optional)));

    group.bench_function("list", |b| b.iter(|| black_box(TypeModifier::List)));

    group.bench_function("is_optional_check", |b| {
        let modifier = TypeModifier::Optional;
        b.iter(|| black_box(modifier.is_optional()))
    });

    group.bench_function("is_list_check", |b| {
        let modifier = TypeModifier::List;
        b.iter(|| black_box(modifier.is_list()))
    });

    group.finish();
}

// ============================================================================
// Attribute Benchmarks
// ============================================================================

fn bench_attribute_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("attribute_operations");

    group.bench_function("create_simple_attribute", |b| {
        b.iter(|| black_box(Attribute::simple(make_ident("id"), make_span())))
    });

    group.bench_function("create_attribute_with_args", |b| {
        b.iter(|| {
            black_box(Attribute::new(
                make_ident("default"),
                vec![AttributeArg::positional(
                    AttributeValue::String("now()".into()),
                    make_span(),
                )],
                make_span(),
            ))
        })
    });

    group.bench_function("attribute_name_lookup", |b| {
        let attr = Attribute::simple(make_ident("unique"), make_span());
        b.iter(|| black_box(attr.name()))
    });

    group.bench_function("attribute_args_is_empty_check", |b| {
        let attr = Attribute::new(
            make_ident("relation"),
            vec![AttributeArg::positional(
                AttributeValue::String("test".into()),
                make_span(),
            )],
            make_span(),
        );
        b.iter(|| black_box(!attr.args.is_empty()))
    });

    group.finish();
}

// ============================================================================
// View Benchmarks
// ============================================================================

fn bench_view_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("view_operations");

    group.bench_function("create_view_3_fields", |b| {
        b.iter(|| {
            let mut view = View::new(make_ident("UserStats"), make_span());
            view.add_field(make_field("userId", ScalarType::Int));
            view.add_field(make_field("userName", ScalarType::String));
            view.add_field(make_field("postCount", ScalarType::Int));
            black_box(view)
        })
    });

    group.bench_function("create_view_10_fields", |b| {
        b.iter(|| {
            let mut view = View::new(make_ident("ComplexView"), make_span());
            for i in 0..10 {
                view.add_field(make_field(&format!("field_{}", i), ScalarType::String));
            }
            black_box(view)
        })
    });

    group.finish();
}

// ============================================================================
// Schema Traversal Benchmarks
// ============================================================================

fn bench_schema_traversal(c: &mut Criterion) {
    let mut group = c.benchmark_group("schema_traversal");

    // Build a schema for traversal tests
    let mut schema = Schema::new();
    for i in 0..20 {
        let mut model = Model::new(make_ident(&format!("Model{}", i)), make_span());
        for j in 0..10 {
            model.add_field(make_field(&format!("field_{}", j), ScalarType::String));
        }
        schema.add_model(model);
    }

    group.bench_function("iterate_all_models", |b| {
        b.iter(|| {
            let mut count = 0;
            for (_, model) in &schema.models {
                count += model.fields.len();
            }
            black_box(count)
        })
    });

    group.bench_function("find_model_by_name", |b| {
        b.iter(|| black_box(schema.models.get("Model10")))
    });

    group.bench_function("count_all_fields", |b| {
        b.iter(|| {
            let count: usize = schema.models.values().map(|m| m.fields.len()).sum();
            black_box(count)
        })
    });

    group.finish();
}

// ============================================================================
// Clone Operations Benchmarks
// ============================================================================

fn bench_clone_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("clone_operations");

    // Create models for cloning
    let mut simple_model = Model::new(make_ident("Simple"), make_span());
    simple_model.add_field(make_field("id", ScalarType::Int));

    let mut complex_model = Model::new(make_ident("Complex"), make_span());
    for i in 0..20 {
        let mut field = make_field(&format!("field_{}", i), ScalarType::String);
        field.attributes.push(make_attribute("default"));
        complex_model.add_field(field);
    }
    complex_model.attributes.push(make_attribute("index"));
    complex_model.attributes.push(make_attribute("map"));

    group.bench_function("clone_simple_model", |b| {
        b.iter(|| black_box(simple_model.clone()))
    });

    group.bench_function("clone_complex_model", |b| {
        b.iter(|| black_box(complex_model.clone()))
    });

    let mut schema = Schema::new();
    for i in 0..10 {
        let mut model = Model::new(make_ident(&format!("Model{}", i)), make_span());
        model.add_field(make_field("id", ScalarType::Int));
        model.add_field(make_field("name", ScalarType::String));
        schema.add_model(model);
    }

    group.bench_function("clone_schema_10_models", |b| {
        b.iter(|| black_box(schema.clone()))
    });

    group.finish();
}

// ============================================================================
// Relation Building Benchmarks
// ============================================================================

fn bench_relation_building(c: &mut Criterion) {
    let mut group = c.benchmark_group("relation_building");

    group.bench_function("create_relation_type", |b| {
        b.iter(|| black_box(RelationType::OneToMany))
    });

    group.bench_function("create_full_relation", |b| {
        b.iter(|| {
            black_box(Relation::new(
                "User",
                "posts",
                "Post",
                RelationType::OneToMany,
            ))
        })
    });

    group.bench_function("create_many_to_one_relation", |b| {
        b.iter(|| {
            black_box(Relation::new(
                "Post",
                "author",
                "User",
                RelationType::ManyToOne,
            ))
        })
    });

    group.bench_function("create_many_to_many_relation", |b| {
        b.iter(|| {
            black_box(Relation::new(
                "Post",
                "tags",
                "Tag",
                RelationType::ManyToMany,
            ))
        })
    });

    group.bench_function("relation_type_check_to_many", |b| {
        let rel_type = RelationType::OneToMany;
        b.iter(|| black_box(rel_type.is_to_many()))
    });

    group.bench_function("relation_type_check_to_one", |b| {
        let rel_type = RelationType::ManyToOne;
        b.iter(|| black_box(rel_type.is_to_one()))
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_model_creation,
    bench_schema_building,
    bench_enum_operations,
    bench_field_types,
    bench_type_modifiers,
    bench_attribute_operations,
    bench_view_operations,
    bench_schema_traversal,
    bench_clone_operations,
    bench_relation_building,
);

criterion_main!(benches);
