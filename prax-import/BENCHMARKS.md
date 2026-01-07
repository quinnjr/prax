# Prax Import Performance Benchmarks

This document contains performance benchmark results for the `prax-import` crate.

## Running Benchmarks

```bash
cargo bench -p prax-import --all-features
```

## Latest Results

### Performance Summary

| Parser | Small Schema | Medium Schema | Large Schema | Avg Speedup |
|--------|--------------|---------------|--------------|-------------|
| **Prisma** | 13.0 µs (2.31x) | 81.9 µs (1.74x) | 217.3 µs (1.74x) | **1.93x** |
| **Diesel** | 12.3 µs (1.80x) | 61.6 µs (1.28x) | 148.2 µs (1.17x) | **1.42x** |
| **SeaORM** | 12.8 µs | 19.8 µs | 32.0 µs | **1.00x** |

*Speedup values shown in parentheses compared to baseline (before optimization)*

### Detailed Benchmarks

#### Prisma Import

```
prisma_import/small     time:   [12.950 µs 13.030 µs 13.114 µs]
                        change: [-57.0% -56.7% -56.4%] (p = 0.00 < 0.05)
                        Performance has improved.

prisma_import/medium    time:   [81.361 µs 81.883 µs 82.468 µs]
                        change: [-42.8% -42.5% -42.2%] (p = 0.00 < 0.05)
                        Performance has improved.

prisma_import/large     time:   [215.73 µs 217.32 µs 219.14 µs]
                        change: [-42.7% -42.5% -42.2%] (p = 0.00 < 0.05)
                        Performance has improved.
```

**Throughput:**
- Small: ~7,675 schemas/second
- Medium: ~1,221 schemas/second
- Large: ~460 schemas/second

#### Diesel Import

```
diesel_import/small     time:   [12.227 µs 12.293 µs 12.369 µs]
                        change: [-44.8% -44.5% -44.2%] (p = 0.00 < 0.05)
                        Performance has improved.

diesel_import/medium    time:   [61.253 µs 61.554 µs 61.855 µs]
                        change: [-22.1% -22.0% -21.9%] (p = 0.00 < 0.05)
                        Performance has improved.

diesel_import/large     time:   [147.07 µs 148.23 µs 149.48 µs]
                        change: [-15.1% -14.7% -14.3%] (p = 0.00 < 0.05)
                        Performance has improved.
```

**Throughput:**
- Small: ~8,135 schemas/second
- Medium: ~1,625 schemas/second
- Large: ~675 schemas/second

#### SeaORM Import

```
seaorm_import/small     time:   [12.716 µs 12.823 µs 12.949 µs]
                        change: [-4.2% -4.0% -3.7%] (p = 0.00 < 0.05)
                        Performance has improved slightly.

seaorm_import/medium    time:   [19.638 µs 19.808 µs 20.007 µs]
                        change: [+1.2% +1.5% +1.8%] (p = 0.00 < 0.05)
                        Change within noise threshold.

seaorm_import/large     time:   [31.698 µs 32.002 µs 32.329 µs]
                        change: [+0.9% +1.2% +1.5%] (p = 0.00 < 0.05)
                        Change within noise threshold.
```

**Throughput:**
- Small: ~7,799 schemas/second
- Medium: ~5,049 schemas/second
- Large: ~3,125 schemas/second

## Optimization History

### v0.4.1 - Regex Compilation Caching (2026-01-07)

**Optimization:** Pre-compile regex patterns using `once_cell::sync::Lazy`

**Impact:**
- Prisma: 42-57% faster (2.31x speedup on small schemas)
- Diesel: 15-45% faster (1.80x speedup on small schemas)
- SeaORM: No change (already uses AST-based parsing)

**Details:**
- Cached 17 regex patterns in Prisma parser
- Cached 3 regex patterns in Diesel parser
- Eliminated repeated regex compilation overhead

**Files Changed:**
- `src/prisma/parser.rs`: Added `Lazy<Regex>` static patterns
- `src/diesel/parser.rs`: Added `Lazy<Regex>` static patterns
- `Cargo.toml`: Added `once_cell` dependency

## Benchmark Methodology

### Test Schemas

**Small Schema:**
- 1 model with 3 fields
- Basic types and attributes
- Minimal complexity

**Medium Schema:**
- 3 models with 5-7 fields each
- Relations between models
- Mixed types and attributes

**Large Schema:**
- 5+ models with 8-12 fields each
- Multiple relations
- Complex attributes (indexes, unique constraints)
- Enums

### Environment

- **CPU**: Varies by system
- **Rust**: 1.85+ (2024 edition)
- **Criterion**: 0.5
- **Iterations**: 100 samples per benchmark
- **Warmup**: 3 seconds

### Measurement

- Uses Criterion.rs for statistical analysis
- Reports mean, standard deviation, and outliers
- Compares against saved baseline
- HTML reports generated in `target/criterion/`

## Performance Tips

### For Library Users

1. **Reuse parsers**: If parsing multiple files, the regex cache is shared
2. **Batch operations**: Parse multiple schemas in one process to amortize startup cost
3. **Profile your usage**: Use `cargo flamegraph` to identify bottlenecks in your workflow

### For Contributors

1. **Avoid regex in hot paths**: Use AST-based parsing when possible (like SeaORM)
2. **Cache compiled patterns**: Use `once_cell::sync::Lazy` for regex
3. **Minimize allocations**: Reuse buffers and use `&str` over `String` when possible
4. **Benchmark changes**: Run `cargo bench` before and after optimizations

## Future Optimization Opportunities

1. **String interning**: Use `SmolStr` more aggressively for repeated strings
2. **Arena allocation**: Consider using an arena allocator for AST nodes
3. **Parallel parsing**: Parse multiple models concurrently for large schemas
4. **Lazy evaluation**: Defer expensive operations until actually needed
5. **SIMD**: Explore SIMD for string scanning operations

## Viewing Benchmark Reports

HTML reports with detailed graphs are generated in:
```
target/criterion/
├── prisma_import/
│   ├── small/report/index.html
│   ├── medium/report/index.html
│   └── large/report/index.html
├── diesel_import/
│   └── ...
└── seaorm_import/
    └── ...
```

Open these files in a browser to see:
- Performance over time
- Distribution plots
- Comparison charts
- Statistical analysis

## Contributing Benchmarks

When adding new features:

1. Add corresponding benchmarks in `benches/import_benchmarks.rs`
2. Run baseline: `cargo bench --all-features -- --save-baseline before`
3. Implement your changes
4. Run comparison: `cargo bench --all-features -- --baseline before`
5. Document results in this file

## License

Same as the main Prax ORM project (MIT OR Apache-2.0).
