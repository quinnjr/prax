//! Benchmarks for pagination operations (ordering, cursor, offset)

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use prax_query::{
    filter::FilterValue,
    pagination::{Cursor, CursorDirection, CursorValue, Pagination},
    sql::{FastSqlBuilder, QueryCapacity, SqlBuilder},
    types::{NullsOrder, OrderBy, OrderByBuilder, OrderByField, SortOrder, order_patterns},
};

// ============================================================================
// Order By Field Creation Benchmarks
// ============================================================================

fn bench_order_by_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("order_by_creation");

    group.bench_function("create_asc", |b| {
        b.iter(|| black_box(OrderByField::asc("created_at")))
    });

    group.bench_function("create_desc", |b| {
        b.iter(|| black_box(OrderByField::desc("updated_at")))
    });

    group.bench_function("create_with_new", |b| {
        b.iter(|| black_box(OrderByField::new("name", SortOrder::Asc)))
    });

    group.bench_function("create_with_nulls", |b| {
        b.iter(|| black_box(OrderByField::new("name", SortOrder::Asc).nulls(NullsOrder::First)))
    });

    group.finish();
}

// ============================================================================
// Sort Order Benchmarks
// ============================================================================

fn bench_sort_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("sort_operations");

    group.bench_function("sort_order_as_sql", |b| {
        let order = SortOrder::Desc;
        b.iter(|| black_box(order.as_sql()))
    });

    group.bench_function("nulls_order_as_sql", |b| {
        let nulls = NullsOrder::First;
        b.iter(|| black_box(nulls.as_sql()))
    });

    group.bench_function("order_field_to_sql", |b| {
        let order = OrderByField::desc("created_at");
        b.iter(|| black_box(order.to_sql()))
    });

    group.bench_function("order_field_with_nulls_to_sql", |b| {
        let order = OrderByField::desc("updated_at").nulls(NullsOrder::Last);
        b.iter(|| black_box(order.to_sql()))
    });

    group.finish();
}

// ============================================================================
// Cursor Creation Benchmarks
// ============================================================================

fn bench_cursor_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("cursor_creation");

    group.bench_function("create_cursor_int", |b| {
        b.iter(|| {
            black_box(Cursor::new(
                "id",
                CursorValue::Int(100),
                CursorDirection::After,
            ))
        })
    });

    group.bench_function("create_cursor_string", |b| {
        b.iter(|| {
            black_box(Cursor::new(
                "email",
                CursorValue::String("user@example.com".into()),
                CursorDirection::Before,
            ))
        })
    });

    group.bench_function("create_cursor_after", |b| {
        b.iter(|| {
            black_box(Cursor::new(
                "id",
                CursorValue::Int(50),
                CursorDirection::After,
            ))
        })
    });

    group.bench_function("create_cursor_before", |b| {
        b.iter(|| {
            black_box(Cursor::new(
                "id",
                CursorValue::Int(50),
                CursorDirection::Before,
            ))
        })
    });

    group.finish();
}

// ============================================================================
// Pagination Benchmarks
// ============================================================================

fn bench_pagination(c: &mut Criterion) {
    let mut group = c.benchmark_group("pagination");

    group.bench_function("create_new_pagination", |b| {
        b.iter(|| black_box(Pagination::new()))
    });

    group.bench_function("pagination_with_skip", |b| {
        b.iter(|| {
            let p = Pagination::new().skip(10);
            black_box(p)
        })
    });

    group.bench_function("pagination_with_take", |b| {
        b.iter(|| {
            let p = Pagination::new().take(20);
            black_box(p)
        })
    });

    group.bench_function("pagination_with_skip_take", |b| {
        b.iter(|| {
            let p = Pagination::new().skip(100).take(25);
            black_box(p)
        })
    });

    group.bench_function("pagination_with_cursor", |b| {
        let cursor = Cursor::new("id", CursorValue::Int(100), CursorDirection::After);
        b.iter(|| {
            let p = Pagination::new().cursor(cursor.clone());
            black_box(p)
        })
    });

    group.finish();
}

// ============================================================================
// Pagination SQL Generation Benchmarks
// ============================================================================

fn bench_pagination_sql_generation(c: &mut Criterion) {
    let mut group = c.benchmark_group("pagination_sql_generation");

    group.bench_function("skip_sql", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT * FROM users OFFSET ");
            builder.push_param(FilterValue::Int(10));
            black_box(builder.build())
        })
    });

    group.bench_function("take_sql", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT * FROM users LIMIT ");
            builder.push_param(FilterValue::Int(20));
            black_box(builder.build())
        })
    });

    group.bench_function("skip_take_sql", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT * FROM users LIMIT ");
            builder.push_param(FilterValue::Int(25));
            builder.push(" OFFSET ");
            builder.push_param(FilterValue::Int(100));
            black_box(builder.build())
        })
    });

    group.bench_function("paginated_with_order_sql", |b| {
        b.iter(|| {
            let order = OrderByField::desc("created_at");
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT * FROM users ORDER BY ");
            builder.push(order.to_sql());
            builder.push(" LIMIT ");
            builder.push_param(FilterValue::Int(10));
            builder.push(" OFFSET ");
            builder.push_param(FilterValue::Int(50));
            black_box(builder.build())
        })
    });

    group.bench_function("cursor_pagination_sql", |b| {
        b.iter(|| {
            // Cursor-based pagination generates WHERE conditions
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT * FROM posts WHERE id > ");
            builder.push_param(FilterValue::Int(100));
            builder.push(" ORDER BY id ASC LIMIT ");
            builder.push_param(FilterValue::Int(10));
            black_box(builder.build())
        })
    });

    group.finish();
}

// ============================================================================
// Real-World Pagination Scenarios
// ============================================================================

fn bench_real_world_pagination(c: &mut Criterion) {
    let mut group = c.benchmark_group("real_world_pagination");

    // Blog posts listing with cursor pagination
    group.bench_function("blog_posts_cursor_sql", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT * FROM posts WHERE published_at < ");
            builder.push_param(FilterValue::String("2024-01-15T10:30:00Z".into()));
            builder.push(" AND status = ");
            builder.push_param(FilterValue::String("published".into()));
            builder.push(" ORDER BY published_at DESC LIMIT ");
            builder.push_param(FilterValue::Int(15));
            black_box(builder.build())
        })
    });

    // E-commerce product listing
    group.bench_function("product_listing_sql", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT * FROM products WHERE category_id = ");
            builder.push_param(FilterValue::Int(42));
            builder.push(" AND in_stock = ");
            builder.push_param(FilterValue::Bool(true));
            builder.push(" AND price >= ");
            builder.push_param(FilterValue::Float(10.0));
            builder.push(" AND price <= ");
            builder.push_param(FilterValue::Float(500.0));
            builder.push(" ORDER BY popularity_score DESC, price ASC LIMIT ");
            builder.push_param(FilterValue::Int(24));
            builder.push(" OFFSET ");
            builder.push_param(FilterValue::Int(0));
            black_box(builder.build())
        })
    });

    // Admin user management table
    group.bench_function("admin_user_table_sql", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT * FROM users WHERE (email LIKE ");
            builder.push_param(FilterValue::String("%admin%".into()));
            builder.push(" OR name LIKE ");
            builder.push_param(FilterValue::String("%admin%".into()));
            builder.push(") ORDER BY last_login_at DESC NULLS LAST LIMIT ");
            builder.push_param(FilterValue::Int(25));
            builder.push(" OFFSET ");
            builder.push_param(FilterValue::Int(50));
            black_box(builder.build())
        })
    });

    // Activity feed with keyset pagination
    group.bench_function("activity_feed_keyset_sql", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT * FROM activities WHERE activity_id > ");
            builder.push_param(FilterValue::String("act_1234567890".into()));
            builder.push(" AND type IN (");
            builder.push_param(FilterValue::String("comment".into()));
            builder.push(", ");
            builder.push_param(FilterValue::String("like".into()));
            builder.push(", ");
            builder.push_param(FilterValue::String("share".into()));
            builder.push(") ORDER BY created_at DESC LIMIT ");
            builder.push_param(FilterValue::Int(30));
            black_box(builder.build())
        })
    });

    group.finish();
}

// ============================================================================
// Batch Pagination Benchmarks
// ============================================================================

fn bench_batch_pagination(c: &mut Criterion) {
    let mut group = c.benchmark_group("batch_pagination");

    // Simulate paginating through large result sets
    for page_size in [10, 25, 50, 100].iter() {
        group.throughput(Throughput::Elements(*page_size as u64));

        group.bench_with_input(
            BenchmarkId::new("create_page_query_sql", page_size),
            page_size,
            |b, &page_size| {
                b.iter(|| {
                    // Simulate page 5
                    let offset = 4 * page_size;
                    let mut builder = SqlBuilder::postgres();
                    builder.push("SELECT * FROM items ORDER BY id ASC LIMIT ");
                    builder.push_param(FilterValue::Int(page_size as i64));
                    builder.push(" OFFSET ");
                    builder.push_param(FilterValue::Int(offset as i64));
                    black_box(builder.build())
                })
            },
        );
    }

    // Simulate cursor pagination through pages
    for cursor_val in [10i64, 50, 100, 500, 1000].iter() {
        group.bench_with_input(
            BenchmarkId::new("cursor_at_position_sql", cursor_val),
            cursor_val,
            |b, &cursor_val| {
                b.iter(|| {
                    let cursor =
                        Cursor::new("id", CursorValue::Int(cursor_val), CursorDirection::After);
                    let mut builder = SqlBuilder::postgres();
                    builder.push("SELECT * FROM items WHERE id > ");
                    builder.push_param(FilterValue::Int(cursor_val));
                    builder.push(" ORDER BY id ASC LIMIT ");
                    builder.push_param(FilterValue::Int(20));
                    black_box((cursor, builder.build()))
                })
            },
        );
    }

    group.finish();
}

// ============================================================================
// Multiple Ordering Benchmarks
// ============================================================================

fn bench_multiple_ordering(c: &mut Criterion) {
    let mut group = c.benchmark_group("multiple_ordering");

    // Original approach: format! concatenation
    group.bench_function("single_order_by", |b| {
        b.iter(|| {
            let order = OrderByField::desc("created_at");
            black_box(order.to_sql())
        })
    });

    group.bench_function("two_order_by_fields_format", |b| {
        b.iter(|| {
            let order1 = OrderByField::desc("created_at");
            let order2 = OrderByField::asc("id");
            black_box(format!("{}, {}", order1.to_sql(), order2.to_sql()))
        })
    });

    group.bench_function("three_order_by_fields_format", |b| {
        b.iter(|| {
            let order1 = OrderByField::asc("category");
            let order2 = OrderByField::desc("price");
            let order3 = OrderByField::asc("name");
            black_box(format!(
                "{}, {}, {}",
                order1.to_sql(),
                order2.to_sql(),
                order3.to_sql()
            ))
        })
    });

    // Optimized: write_sql to buffer
    group.bench_function("single_order_by_write", |b| {
        b.iter(|| {
            let order = OrderByField::desc("created_at");
            let mut buffer = String::with_capacity(32);
            order.write_sql(&mut buffer);
            black_box(buffer)
        })
    });

    group.bench_function("two_order_by_write", |b| {
        b.iter(|| {
            let order =
                OrderBy::from_fields([OrderByField::desc("created_at"), OrderByField::asc("id")]);
            let mut buffer = String::with_capacity(64);
            order.write_sql(&mut buffer);
            black_box(buffer)
        })
    });

    group.bench_function("three_order_by_write", |b| {
        b.iter(|| {
            let order = OrderBy::from_fields([
                OrderByField::asc("category"),
                OrderByField::desc("price"),
                OrderByField::asc("name"),
            ]);
            let mut buffer = String::with_capacity(64);
            order.write_sql(&mut buffer);
            black_box(buffer)
        })
    });

    // Optimized: OrderByBuilder with pre-allocation
    group.bench_function("three_order_by_builder", |b| {
        b.iter(|| {
            let order = OrderByBuilder::with_capacity(3)
                .desc("category")
                .desc("price")
                .asc("name")
                .build();
            black_box(order.to_sql())
        })
    });

    // Static patterns (zero allocation for field construction)
    group.bench_function("static_created_at_desc", |b| {
        b.iter(|| black_box(order_patterns::CREATED_AT_DESC.to_sql()))
    });

    group.bench_function("static_id_asc", |b| {
        b.iter(|| black_box(order_patterns::ID_ASC.to_sql()))
    });

    group.finish();
}

// ============================================================================
// Optimized Pagination SQL Generation
// ============================================================================

fn bench_optimized_pagination(c: &mut Criterion) {
    let mut group = c.benchmark_group("optimized_pagination");

    // Original Pagination.to_sql()
    group.bench_function("pagination_to_sql_skip_take", |b| {
        let pagination = Pagination::new().skip(100).take(25);
        b.iter(|| black_box(pagination.to_sql()))
    });

    // Optimized: write_sql to buffer
    group.bench_function("pagination_write_sql_skip_take", |b| {
        let pagination = Pagination::new().skip(100).take(25);
        b.iter(|| {
            let mut buffer = String::with_capacity(32);
            pagination.write_sql(&mut buffer);
            black_box(buffer)
        })
    });

    // Activity feed keyset comparison: original vs optimized
    group.bench_function("activity_feed_keyset_original", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT * FROM activities WHERE activity_id > ");
            builder.push_param(FilterValue::String("act_1234567890".into()));
            builder.push(" AND type IN (");
            builder.push_param(FilterValue::String("comment".into()));
            builder.push(", ");
            builder.push_param(FilterValue::String("like".into()));
            builder.push(", ");
            builder.push_param(FilterValue::String("share".into()));
            builder.push(") ORDER BY created_at DESC LIMIT ");
            builder.push_param(FilterValue::Int(30));
            black_box(builder.build())
        })
    });

    group.bench_function("activity_feed_keyset_fast", |b| {
        b.iter(|| {
            let mut builder = FastSqlBuilder::postgres(QueryCapacity::Custom(256));
            builder.push_str("SELECT * FROM activities WHERE activity_id > ");
            builder.bind(FilterValue::String("act_1234567890".into()));
            builder.push_str(" AND type IN (");
            builder.bind_in_clause([
                FilterValue::String("comment".into()),
                FilterValue::String("like".into()),
                FilterValue::String("share".into()),
            ]);
            // Use static pattern string directly
            builder.push_str(") ORDER BY created_at DESC LIMIT ");
            builder.bind(30i64);
            black_box(builder.build())
        })
    });

    // Product listing comparison
    group.bench_function("product_listing_original", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT * FROM products WHERE category_id = ");
            builder.push_param(FilterValue::Int(42));
            builder.push(" AND in_stock = ");
            builder.push_param(FilterValue::Bool(true));
            builder.push(" AND price >= ");
            builder.push_param(FilterValue::Float(10.0));
            builder.push(" AND price <= ");
            builder.push_param(FilterValue::Float(500.0));
            builder.push(" ORDER BY popularity_score DESC, price ASC LIMIT ");
            builder.push_param(FilterValue::Int(24));
            builder.push(" OFFSET ");
            builder.push_param(FilterValue::Int(0));
            black_box(builder.build())
        })
    });

    group.bench_function("product_listing_fast", |b| {
        b.iter(|| {
            let mut builder = FastSqlBuilder::postgres(QueryCapacity::SelectWithFilters(6));
            builder
                .push_str("SELECT * FROM products WHERE category_id = ")
                .bind(42i64)
                .push_str(" AND in_stock = ")
                .bind(true)
                .push_str(" AND price >= ")
                .bind(10.0f64)
                .push_str(" AND price <= ")
                .bind(500.0f64)
                .push_str(" ORDER BY popularity_score DESC, price ASC LIMIT ")
                .bind(24i64)
                .push_str(" OFFSET ")
                .bind(0i64);
            black_box(builder.build())
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_order_by_creation,
    bench_sort_operations,
    bench_cursor_creation,
    bench_pagination,
    bench_pagination_sql_generation,
    bench_real_world_pagination,
    bench_batch_pagination,
    bench_multiple_ordering,
    bench_optimized_pagination,
);

criterion_main!(benches);
