//! Benchmarks for multi-tenancy features.
//!
//! Run with: cargo bench --package prax-query --bench tenant_bench

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;
use std::time::Duration;

use prax_query::tenant::{
    cache::{CacheConfig, CacheLookup, ShardedTenantCache, TenantCache},
    pool::{PoolConfig, TenantPoolManager},
    prepared::{StatementCache, StatementKey},
    rls::RlsManager,
    task_local::{current_tenant_id, has_tenant, with_tenant},
    TenantContext, TenantId,
};

fn bench_tenant_context(c: &mut Criterion) {
    let mut group = c.benchmark_group("tenant/context");

    group.bench_function("TenantId::new", |b| {
        b.iter(|| black_box(TenantId::new("tenant-123")))
    });

    group.bench_function("TenantContext::new", |b| {
        b.iter(|| black_box(TenantContext::new("tenant-123")))
    });

    group.bench_function("TenantId::clone", |b| {
        let id = TenantId::new("tenant-123");
        b.iter(|| black_box(id.clone()))
    });

    group.bench_function("TenantContext::clone", |b| {
        let ctx = TenantContext::new("tenant-123");
        b.iter(|| black_box(ctx.clone()))
    });

    group.finish();
}

fn bench_task_local(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();

    let mut group = c.benchmark_group("tenant/task_local");

    group.bench_function("with_tenant_overhead", |b| {
        b.to_async(&rt).iter(|| async {
            with_tenant("tenant-123", async {
                black_box(())
            })
            .await
        })
    });

    group.bench_function("current_tenant_id_hit", |b| {
        b.to_async(&rt).iter(|| async {
            with_tenant("tenant-123", async {
                black_box(current_tenant_id())
            })
            .await
        })
    });

    group.bench_function("current_tenant_id_miss", |b| {
        b.to_async(&rt).iter(|| async { black_box(current_tenant_id()) })
    });

    group.bench_function("has_tenant_check", |b| {
        b.to_async(&rt).iter(|| async {
            with_tenant("tenant-123", async {
                black_box(has_tenant())
            })
            .await
        })
    });

    group.bench_function("nested_context_3_levels", |b| {
        b.to_async(&rt).iter(|| async {
            with_tenant("level-1", async {
                with_tenant("level-2", async {
                    with_tenant("level-3", async {
                        black_box(current_tenant_id())
                    })
                    .await
                })
                .await
            })
            .await
        })
    });

    group.finish();
}

fn bench_tenant_cache(c: &mut Criterion) {
    let mut group = c.benchmark_group("tenant/cache");
    group.throughput(Throughput::Elements(1));

    // Setup cache with some entries
    let cache = TenantCache::new(CacheConfig::new(10000));
    for i in 0..1000 {
        let id = TenantId::new(format!("tenant-{}", i));
        cache.insert(id.clone(), TenantContext::new(id));
    }

    group.bench_function("lookup_hit", |b| {
        let id = TenantId::new("tenant-500");
        b.iter(|| black_box(cache.lookup(&id)))
    });

    group.bench_function("lookup_miss", |b| {
        let id = TenantId::new("unknown-tenant");
        b.iter(|| black_box(cache.lookup(&id)))
    });

    group.bench_function("insert", |b| {
        let mut i = 2000u64;
        b.iter(|| {
            let id = TenantId::new(format!("new-tenant-{}", i));
            cache.insert(id.clone(), TenantContext::new(id));
            i += 1;
        })
    });

    group.bench_function("invalidate", |b| {
        let id = TenantId::new("tenant-100");
        b.iter(|| {
            cache.invalidate(&id);
            // Re-insert for next iteration
            cache.insert(id.clone(), TenantContext::new(id.clone()));
        })
    });

    group.finish();
}

fn bench_sharded_cache(c: &mut Criterion) {
    let mut group = c.benchmark_group("tenant/sharded_cache");
    group.throughput(Throughput::Elements(1));

    // Create sharded cache
    let cache = ShardedTenantCache::new(8, CacheConfig::new(10000));
    for i in 0..1000 {
        let id = TenantId::new(format!("tenant-{}", i));
        cache.insert(id.clone(), TenantContext::new(id));
    }

    group.bench_function("lookup_hit", |b| {
        let id = TenantId::new("tenant-500");
        b.iter(|| black_box(cache.lookup(&id)))
    });

    // Benchmark different shard counts
    for shards in [2, 4, 8, 16].iter() {
        let cache = ShardedTenantCache::new(*shards, CacheConfig::new(10000));
        for i in 0..1000 {
            let id = TenantId::new(format!("tenant-{}", i));
            cache.insert(id.clone(), TenantContext::new(id));
        }

        group.bench_with_input(
            BenchmarkId::new("lookup_shards", shards),
            shards,
            |b, _| {
                let id = TenantId::new("tenant-500");
                b.iter(|| black_box(cache.lookup(&id)))
            },
        );
    }

    group.finish();
}

fn bench_statement_cache(c: &mut Criterion) {
    let mut group = c.benchmark_group("tenant/statement_cache");

    // Global cache
    let global_cache: StatementCache<String> = StatementCache::global(1000);
    for i in 0..500 {
        let key = StatementKey::new(format!("stmt_{}", i), format!("SELECT * FROM t{}", i));
        global_cache.insert(key, format!("handle_{}", i));
    }

    group.bench_function("global_lookup_hit", |b| {
        let key = StatementKey::new("stmt_250", "SELECT * FROM t250");
        b.iter(|| black_box(global_cache.get(&key)))
    });

    group.bench_function("global_lookup_miss", |b| {
        let key = StatementKey::new("unknown", "SELECT 1");
        b.iter(|| black_box(global_cache.get(&key)))
    });

    group.bench_function("global_insert", |b| {
        let mut i = 1000u64;
        b.iter(|| {
            let key = StatementKey::new(format!("new_{}", i), format!("SELECT {}", i));
            global_cache.insert(key, format!("handle_{}", i));
            i += 1;
        })
    });

    // Per-tenant cache
    let tenant_cache: StatementCache<String> = StatementCache::per_tenant(100, 100);
    for t in 0..10 {
        let tenant_id = TenantId::new(format!("tenant-{}", t));
        for i in 0..50 {
            let key = StatementKey::new(format!("stmt_{}", i), format!("SELECT * FROM t{}", i));
            tenant_cache.insert_for_tenant(&tenant_id, key, format!("handle_{}_{}", t, i));
        }
    }

    group.bench_function("per_tenant_lookup_hit", |b| {
        let tenant_id = TenantId::new("tenant-5");
        let key = StatementKey::new("stmt_25", "SELECT * FROM t25");
        b.iter(|| black_box(tenant_cache.get_for_tenant(&tenant_id, &key)))
    });

    group.bench_function("per_tenant_insert", |b| {
        let tenant_id = TenantId::new("tenant-5");
        let mut i = 1000u64;
        b.iter(|| {
            let key = StatementKey::new(format!("new_{}", i), format!("SELECT {}", i));
            tenant_cache.insert_for_tenant(&tenant_id, key, format!("handle_{}", i));
            i += 1;
        })
    });

    group.finish();
}

fn bench_rls_sql_generation(c: &mut Criterion) {
    let mut group = c.benchmark_group("tenant/rls");

    let manager = RlsManager::simple("tenant_id", "app.current_tenant");

    group.bench_function("set_tenant_sql", |b| {
        b.iter(|| black_box(manager.set_tenant_sql("tenant-123")))
    });

    group.bench_function("set_tenant_local_sql", |b| {
        b.iter(|| black_box(manager.set_tenant_local_sql("tenant-123")))
    });

    group.bench_function("create_policy_sql", |b| {
        b.iter(|| black_box(manager.create_policy_sql("users")))
    });

    group.bench_function("enable_rls_sql", |b| {
        b.iter(|| black_box(manager.enable_rls_sql("users")))
    });

    group.finish();
}

fn bench_pool_manager(c: &mut Criterion) {
    let mut group = c.benchmark_group("tenant/pool");

    let manager = TenantPoolManager::builder()
        .per_tenant(1000, 5)
        .build();

    group.bench_function("get_or_create_new", |b| {
        let mut i = 0u64;
        b.iter(|| {
            let id = TenantId::new(format!("new-tenant-{}", i));
            black_box(manager.get_or_create(&id));
            i += 1;
        })
    });

    // Pre-populate some entries
    for i in 0..100 {
        let id = TenantId::new(format!("existing-{}", i));
        manager.get_or_create(&id);
    }

    group.bench_function("get_or_create_existing", |b| {
        let id = TenantId::new("existing-50");
        b.iter(|| black_box(manager.get_or_create(&id)))
    });

    group.bench_function("global_stats", |b| {
        b.iter(|| black_box(manager.global_stats()))
    });

    group.finish();
}

fn bench_cache_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("tenant/cache_throughput");
    group.measurement_time(Duration::from_secs(5));

    for size in [100, 1000, 10000].iter() {
        let cache = TenantCache::new(CacheConfig::new(*size));

        // Fill to 80% capacity
        let fill_count = (*size as f64 * 0.8) as usize;
        for i in 0..fill_count {
            let id = TenantId::new(format!("tenant-{}", i));
            cache.insert(id.clone(), TenantContext::new(id));
        }

        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::new("mixed_ops", size), size, |b, &size| {
            let mut i = 0u64;
            b.iter(|| {
                // 80% hits, 20% misses (realistic workload)
                let idx = i % 100;
                let id = if idx < 80 {
                    TenantId::new(format!("tenant-{}", i % (size as u64 / 2)))
                } else {
                    TenantId::new(format!("unknown-{}", i))
                };
                black_box(cache.lookup(&id));
                i += 1;
            })
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_tenant_context,
    bench_task_local,
    bench_tenant_cache,
    bench_sharded_cache,
    bench_statement_cache,
    bench_rls_sql_generation,
    bench_pool_manager,
    bench_cache_throughput,
);

criterion_main!(benches);


