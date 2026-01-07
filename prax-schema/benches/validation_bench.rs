//! Benchmarks for validation operations.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use prax_schema::ast::{Field, FieldType, Ident, ScalarType, Span, TypeModifier};
use prax_schema::{EnhancedDocumentation, FieldMetadata, ValidationType};

// ============================================================================
// Helper Functions
// ============================================================================

fn make_span() -> Span {
    Span::new(0, 0)
}

fn make_ident(name: &str) -> Ident {
    Ident::new(name, make_span())
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
// Validation Type Creation Benchmarks
// ============================================================================

fn bench_validation_type_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("validation_type_creation");

    group.bench_function("create_required", |b| {
        b.iter(|| black_box(ValidationType::Required))
    });

    group.bench_function("create_email", |b| {
        b.iter(|| black_box(ValidationType::Email))
    });

    group.bench_function("create_min_length", |b| {
        b.iter(|| black_box(ValidationType::MinLength(10)))
    });

    group.bench_function("create_max_length", |b| {
        b.iter(|| black_box(ValidationType::MaxLength(255)))
    });

    group.bench_function("create_length_range", |b| {
        b.iter(|| black_box(ValidationType::Length { min: 5, max: 100 }))
    });

    group.bench_function("create_regex_pattern", |b| {
        b.iter(|| black_box(ValidationType::Regex(r"^\+?[0-9]{10,15}$".to_string())))
    });

    group.bench_function("create_min_max_numeric", |b| {
        b.iter(|| black_box((ValidationType::Min(0.0), ValidationType::Max(100.0))))
    });

    group.bench_function("create_range", |b| {
        b.iter(|| {
            black_box(ValidationType::Range {
                min: 0.0,
                max: 1000.0,
            })
        })
    });

    group.finish();
}

// ============================================================================
// Validation Type Checks Benchmarks
// ============================================================================

fn bench_validation_type_checks(c: &mut Criterion) {
    let mut group = c.benchmark_group("validation_type_checks");

    group.bench_function("check_is_string_rule", |b| {
        let rule_type = ValidationType::MinLength(10);
        b.iter(|| black_box(rule_type.is_string_rule()))
    });

    group.bench_function("check_is_numeric_rule", |b| {
        let rule_type = ValidationType::Min(0.0);
        b.iter(|| black_box(rule_type.is_numeric_rule()))
    });

    group.bench_function("check_is_array_rule", |b| {
        let rule_type = ValidationType::MinItems(1);
        b.iter(|| black_box(rule_type.is_array_rule()))
    });

    group.bench_function("get_default_message", |b| {
        let rule_type = ValidationType::Email;
        b.iter(|| black_box(rule_type.default_message("email")))
    });

    group.bench_function("get_validator_name", |b| {
        let rule_type = ValidationType::MinLength(10);
        b.iter(|| black_box(rule_type.validator_name()))
    });

    group.finish();
}

// ============================================================================
// Enhanced Documentation Benchmarks
// ============================================================================

fn bench_enhanced_documentation(c: &mut Criterion) {
    let mut group = c.benchmark_group("enhanced_documentation");

    let span = make_span();

    group.bench_function("parse_simple_doc", |b| {
        let doc = "The user's email address";
        b.iter(|| black_box(EnhancedDocumentation::parse(doc, span)))
    });

    group.bench_function("parse_doc_with_validation", |b| {
        let doc = r#"The user's email address
@validate(email)
@validate(maxLength: 255)"#;
        b.iter(|| black_box(EnhancedDocumentation::parse(doc, span)))
    });

    group.bench_function("parse_doc_with_tags", |b| {
        let doc = r#"The user's password hash
@hidden
@sensitive
@example("$2a$10$...")"#;
        b.iter(|| black_box(EnhancedDocumentation::parse(doc, span)))
    });

    group.bench_function("parse_complex_doc", |b| {
        let doc = r#"The user's full profile information
@validate(minLength: 2)
@validate(maxLength: 1000)
@example("John Doe is a software engineer...")
@deprecated("Use 'profile' relation instead")
@see(Profile)
@label("User Bio")"#;
        b.iter(|| black_box(EnhancedDocumentation::parse(doc, span)))
    });

    // Create a parsed documentation for extraction benchmarks
    let parsed_doc = EnhancedDocumentation::parse(
        r#"User email
@validate(email)
@validate(maxLength: 255)
@example("user@example.com")"#,
        span,
    );

    group.bench_function("extract_validation_rules", |b| {
        b.iter(|| black_box(parsed_doc.validation_rules()))
    });

    group.bench_function("extract_metadata", |b| {
        b.iter(|| black_box(parsed_doc.extract_metadata()))
    });

    group.finish();
}

// ============================================================================
// Field Metadata Benchmarks
// ============================================================================

fn bench_field_metadata(c: &mut Criterion) {
    let mut group = c.benchmark_group("field_metadata");

    group.bench_function("create_default_metadata", |b| {
        b.iter(|| black_box(FieldMetadata::default()))
    });

    group.bench_function("create_metadata_with_properties", |b| {
        b.iter(|| {
            black_box(FieldMetadata {
                hidden: true,
                sensitive: true,
                label: Some("Email Address".to_string()),
                description: Some("The user's primary email".to_string()),
                placeholder: Some("Enter your email".to_string()),
                examples: vec!["example@email.com".to_string()],
                see_also: vec!["Profile".to_string()],
                ..Default::default()
            })
        })
    });

    // Create metadata for property access benchmarks
    let metadata = FieldMetadata {
        hidden: true,
        sensitive: true,
        label: Some("Email".to_string()),
        examples: vec!["test@example.com".to_string()],
        ..Default::default()
    };

    group.bench_function("check_hidden_flag", |b| {
        b.iter(|| black_box(metadata.hidden))
    });

    group.bench_function("check_sensitive_flag", |b| {
        b.iter(|| black_box(metadata.sensitive))
    });

    group.bench_function("access_examples", |b| {
        b.iter(|| black_box(&metadata.examples))
    });

    group.finish();
}

// ============================================================================
// Batch Validation Type Benchmarks
// ============================================================================

fn bench_batch_validation_types(c: &mut Criterion) {
    let mut group = c.benchmark_group("batch_validation_types");

    for count in [5, 10, 25, 50].iter() {
        group.throughput(Throughput::Elements(*count as u64));

        group.bench_with_input(
            BenchmarkId::new("create_n_validation_types", count),
            count,
            |b, &count| {
                b.iter(|| {
                    let types: Vec<ValidationType> = (0..count)
                        .map(|i| match i % 5 {
                            0 => ValidationType::Required,
                            1 => ValidationType::MinLength(i),
                            2 => ValidationType::MaxLength(100 + i),
                            3 => ValidationType::Email,
                            _ => ValidationType::Min(i as f64),
                        })
                        .collect();
                    black_box(types)
                })
            },
        );

        group.bench_with_input(
            BenchmarkId::new("check_n_rule_types", count),
            count,
            |b, &count| {
                let types: Vec<ValidationType> = (0..count)
                    .map(|i| match i % 4 {
                        0 => ValidationType::MinLength(i),
                        1 => ValidationType::Min(i as f64),
                        2 => ValidationType::MinItems(i),
                        _ => ValidationType::Required,
                    })
                    .collect();
                b.iter(|| {
                    let mut string_count = 0;
                    let mut numeric_count = 0;
                    let mut array_count = 0;
                    for t in &types {
                        if t.is_string_rule() {
                            string_count += 1;
                        }
                        if t.is_numeric_rule() {
                            numeric_count += 1;
                        }
                        if t.is_array_rule() {
                            array_count += 1;
                        }
                    }
                    black_box((string_count, numeric_count, array_count))
                })
            },
        );
    }

    group.finish();
}

// ============================================================================
// Field with Validation Integration Benchmarks
// ============================================================================

fn bench_field_with_validation(c: &mut Criterion) {
    let mut group = c.benchmark_group("field_with_validation");

    let span = make_span();

    group.bench_function("create_field_with_enhanced_doc", |b| {
        b.iter(|| {
            let mut field = make_field("email", ScalarType::String);
            let doc = EnhancedDocumentation::parse(
                r#"User email
@validate(email)
@validate(maxLength: 255)"#,
                span,
            );
            field = field.with_enhanced_documentation(doc);
            black_box(field)
        })
    });

    // Create a field with validation for query benchmarks
    let mut validated_field = make_field("email", ScalarType::String);
    let doc = EnhancedDocumentation::parse(
        r#"User email
@validate(required)
@validate(email)
@validate(minLength: 5)
@validate(maxLength: 255)"#,
        span,
    );
    validated_field = validated_field.with_enhanced_documentation(doc);

    group.bench_function("check_field_has_validation", |b| {
        b.iter(|| black_box(validated_field.has_validation()))
    });

    group.bench_function("check_field_is_validated_required", |b| {
        b.iter(|| black_box(validated_field.is_validated_required()))
    });

    group.bench_function("get_field_validation_rules", |b| {
        b.iter(|| black_box(validated_field.validation_rules()))
    });

    group.finish();
}

// ============================================================================
// Real-World Validation Scenarios
// ============================================================================

fn bench_real_world_validation(c: &mut Criterion) {
    let mut group = c.benchmark_group("real_world_validation");

    // User registration form validation types
    group.bench_function("user_registration_fields", |b| {
        b.iter(|| {
            let username = vec![
                ValidationType::Required,
                ValidationType::MinLength(3),
                ValidationType::MaxLength(20),
                ValidationType::Alphanumeric,
            ];
            let email = vec![ValidationType::Required, ValidationType::Email];
            let password = vec![
                ValidationType::Required,
                ValidationType::MinLength(8),
                ValidationType::MaxLength(128),
            ];
            let age = vec![
                ValidationType::Required,
                ValidationType::Range {
                    min: 13.0,
                    max: 120.0,
                },
            ];
            black_box((username, email, password, age))
        })
    });

    // E-commerce product validation
    group.bench_function("product_validation_fields", |b| {
        b.iter(|| {
            let name = vec![
                ValidationType::Required,
                ValidationType::MinLength(2),
                ValidationType::MaxLength(100),
            ];
            let price = vec![ValidationType::Required, ValidationType::Positive];
            let quantity = vec![ValidationType::Required, ValidationType::NonNegative];
            let sku = vec![
                ValidationType::Required,
                ValidationType::Regex(r"^[A-Z0-9-]{6,20}$".to_string()),
            ];
            black_box((name, price, quantity, sku))
        })
    });

    // API response validation
    group.bench_function("api_response_validation", |b| {
        b.iter(|| {
            let items = vec![
                ValidationType::Required,
                ValidationType::MinItems(1),
                ValidationType::MaxItems(100),
            ];
            let page_size = vec![ValidationType::Range {
                min: 1.0,
                max: 100.0,
            }];
            let timestamp = vec![ValidationType::Required];
            black_box((items, page_size, timestamp))
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_validation_type_creation,
    bench_validation_type_checks,
    bench_enhanced_documentation,
    bench_field_metadata,
    bench_batch_validation_types,
    bench_field_with_validation,
    bench_real_world_validation,
);

criterion_main!(benches);
