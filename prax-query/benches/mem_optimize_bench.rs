//! Benchmarks for memory optimizations.
//!
//! Run with: `cargo bench --package prax-query --bench mem_optimize_bench`

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;
use prax_query::mem_optimize::{
    arena::QueryArena,
    interning::{GlobalInterner, ScopedInterner, IdentifierCache},
    lazy::LazySchema,
};
use std::collections::HashMap;

// ============================================================================
// String Interning Benchmarks
// ============================================================================

fn bench_global_interner(c: &mut Criterion) {
    let mut group = c.benchmark_group("global_interner");

    // Pre-warm the interner
    let interner = GlobalInterner::get();
    for i in 0..100 {
        interner.intern(&format!("field_{}", i));
    }

    // Benchmark cache hits
    group.bench_function("cache_hit", |b| {
        b.iter(|| {
            let s = interner.intern("id");
            black_box(s);
        });
    });

    // Benchmark cache misses (unique strings)
    let mut counter = 0u64;
    group.bench_function("cache_miss", |b| {
        b.iter(|| {
            counter += 1;
            let s = interner.intern(&format!("unique_field_{}", counter));
            black_box(s);
        });
    });

    // Benchmark mixed workload
    group.bench_function("mixed_workload", |b| {
        let common = ["id", "email", "name", "created_at", "user_id"];
        let mut i = 0;
        b.iter(|| {
            i += 1;
            let s = if i % 3 == 0 {
                interner.intern(&format!("dynamic_{}", i))
            } else {
                interner.intern(common[i % common.len()])
            };
            black_box(s);
        });
    });

    group.finish();
}

fn bench_scoped_interner(c: &mut Criterion) {
    let mut group = c.benchmark_group("scoped_interner");

    group.bench_function("intern_100_strings", |b| {
        b.iter(|| {
            let mut interner = ScopedInterner::with_capacity(100);
            for i in 0..100 {
                let s = interner.intern(&format!("field_{}", i));
                black_box(s);
            }
            // Interner dropped here, freeing memory
        });
    });

    group.bench_function("intern_with_reuse", |b| {
        b.iter(|| {
            let mut interner = ScopedInterner::with_capacity(20);
            // Simulate query building with repeated field names
            for _ in 0..10 {
                for field in ["id", "email", "name", "active", "created_at"] {
                    let s = interner.intern(field);
                    black_box(s);
                }
            }
        });
    });

    group.finish();
}

fn bench_identifier_cache(c: &mut Criterion) {
    let mut group = c.benchmark_group("identifier_cache");

    let cache = IdentifierCache::new();

    // Pre-populate
    for table in ["users", "posts", "comments"] {
        for col in ["id", "name", "created_at"] {
            cache.intern_qualified(table, col);
        }
    }

    group.bench_function("qualified_hit", |b| {
        b.iter(|| {
            let id = cache.intern_qualified("users", "email");
            black_box(id);
        });
    });

    group.bench_function("component_intern", |b| {
        b.iter(|| {
            let id = cache.intern_component("user_id");
            black_box(id);
        });
    });

    group.finish();
}

// ============================================================================
// Arena Allocation Benchmarks
// ============================================================================

fn bench_arena_allocation(c: &mut Criterion) {
    let mut group = c.benchmark_group("arena_allocation");

    group.bench_function("simple_filter", |b| {
        let arena = QueryArena::new();
        b.iter(|| {
            let sql = arena.scope(|scope| {
                scope.build_select("users", scope.eq("id", 42))
            });
            black_box(sql);
        });
    });

    group.bench_function("complex_filter", |b| {
        let arena = QueryArena::new();
        b.iter(|| {
            let sql = arena.scope(|scope| {
                let filter = scope.and(vec![
                    scope.eq("active", true),
                    scope.or(vec![
                        scope.gt("age", 18),
                        scope.is_not_null("verified_at"),
                    ]),
                    scope.is_in("status", vec!["pending".into(), "active".into()]),
                ]);
                scope.build_select("users", filter)
            });
            black_box(sql);
        });
    });

    group.bench_function("reuse_arena", |b| {
        let mut arena = QueryArena::with_capacity(4096);
        b.iter(|| {
            let sql = arena.scope(|scope| {
                scope.build_select("users", scope.eq("id", 1))
            });
            arena.reset();
            black_box(sql);
        });
    });

    group.finish();
}

fn bench_arena_vs_heap(c: &mut Criterion) {
    let mut group = c.benchmark_group("arena_vs_heap");

    // Arena-based filter construction
    group.bench_function("arena_10_filters", |b| {
        let arena = QueryArena::new();
        b.iter(|| {
            arena.scope(|scope| {
                let mut filters = Vec::with_capacity(10);
                for i in 0..10 {
                    filters.push(scope.eq(&format!("field_{}", i), i as i64));
                }
                let filter = scope.and(filters);
                scope.build_select("test", filter)
            })
        });
    });

    // Heap-based filter construction (using standard types)
    group.bench_function("heap_10_filters", |b| {
        b.iter(|| {
            let mut filters: Vec<(String, i64)> = Vec::with_capacity(10);
            for i in 0..10 {
                filters.push((format!("field_{}", i), i as i64));
            }
            // Build SQL manually
            let mut sql = String::from("SELECT * FROM test WHERE ");
            for (i, (field, _)) in filters.iter().enumerate() {
                if i > 0 {
                    sql.push_str(" AND ");
                }
                sql.push_str(field);
                sql.push_str(" = ?");
            }
            black_box(sql)
        });
    });

    group.finish();
}

// ============================================================================
// Lazy Schema Benchmarks
// ============================================================================

fn create_large_schema_json(num_tables: usize, columns_per_table: usize) -> String {
    let mut json = String::from(r#"{"name":"test_db","schema":"public","tables":["#);

    for t in 0..num_tables {
        if t > 0 {
            json.push(',');
        }
        json.push_str(&format!(r#"{{"name":"table_{}","columns":["#, t));

        for c in 0..columns_per_table {
            if c > 0 {
                json.push(',');
            }
            json.push_str(&format!(
                r#"{{"name":"col_{}","db_type":"varchar(255)","nullable":true}}"#,
                c
            ));
        }

        json.push_str(r#"],"primary_key":["col_0"],"indexes":[],"foreign_keys":[]}"#);
    }

    json.push_str(r#"],"enums":[]}"#);
    json
}

fn bench_lazy_schema(c: &mut Criterion) {
    let mut group = c.benchmark_group("lazy_schema");
    group.sample_size(20);

    // Create test schemas
    let small_schema = create_large_schema_json(10, 10);
    let medium_schema = create_large_schema_json(50, 20);
    let large_schema = create_large_schema_json(100, 30);

    // Benchmark schema loading
    group.bench_function("load_small_schema", |b| {
        b.iter(|| {
            let schema = LazySchema::from_json(&small_schema).unwrap();
            black_box(schema.table_count());
        });
    });

    group.bench_function("load_large_schema", |b| {
        b.iter(|| {
            let schema = LazySchema::from_json(&large_schema).unwrap();
            black_box(schema.table_count());
        });
    });

    // Benchmark table name access (no parsing)
    let schema = LazySchema::from_json(&medium_schema).unwrap();
    group.bench_function("table_names_only", |b| {
        b.iter(|| {
            let names = schema.table_names();
            black_box(names.len());
        });
    });

    // Benchmark single table access
    group.bench_function("single_table_access", |b| {
        let schema = LazySchema::from_json(&medium_schema).unwrap();
        b.iter(|| {
            let table = schema.get_table("table_25").unwrap();
            black_box(table.name());
        });
    });

    // Benchmark column access (triggers parsing)
    group.bench_function("column_access", |b| {
        b.iter(|| {
            let schema = LazySchema::from_json(&medium_schema).unwrap();
            let table = schema.get_table("table_25").unwrap();
            let columns = table.columns();
            black_box(columns.len());
        });
    });

    group.finish();
}

fn bench_lazy_vs_eager(c: &mut Criterion) {
    let mut group = c.benchmark_group("lazy_vs_eager");
    group.sample_size(20);

    let schema_json = create_large_schema_json(100, 20);

    // Lazy: Load schema and access one table
    group.bench_function("lazy_one_table", |b| {
        b.iter(|| {
            let schema = LazySchema::from_json(&schema_json).unwrap();
            let table = schema.get_table("table_50").unwrap();
            let columns = table.columns();
            black_box(columns.len());
        });
    });

    // Simulate eager: Load and parse all tables
    group.bench_function("eager_all_tables", |b| {
        b.iter(|| {
            let schema = LazySchema::from_json(&schema_json).unwrap();
            // Force parsing of all tables
            for name in schema.table_names() {
                if let Some(table) = schema.get_table(name) {
                    let _ = table.columns();
                }
            }
            black_box(schema.table_count());
        });
    });

    group.finish();
}

// ============================================================================
// Memory Usage Comparison
// ============================================================================

fn bench_memory_efficiency(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_efficiency");

    // Interning vs no interning
    group.throughput(Throughput::Elements(1000));

    group.bench_function("without_interning", |b| {
        b.iter(|| {
            let mut strings: Vec<String> = Vec::with_capacity(1000);
            for i in 0..1000 {
                strings.push(format!("field_{}", i % 50)); // 50 unique, repeated
            }
            black_box(strings);
        });
    });

    group.bench_function("with_interning", |b| {
        b.iter(|| {
            let mut interner = ScopedInterner::with_capacity(50);
            let mut strings = Vec::with_capacity(1000);
            for i in 0..1000 {
                strings.push(interner.intern(&format!("field_{}", i % 50)));
            }
            black_box(strings);
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_global_interner,
    bench_scoped_interner,
    bench_identifier_cache,
    bench_arena_allocation,
    bench_arena_vs_heap,
    bench_lazy_schema,
    bench_lazy_vs_eager,
    bench_memory_efficiency,
);

criterion_main!(benches);

