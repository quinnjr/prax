# Contributing to Prax ORM

Thank you for your interest in contributing to Prax ORM! 🦀

## Quick Links

- [Project Board](https://github.com/orgs/pegasusheavy/projects/2) - See current tasks and roadmap
- [Issues](https://github.com/pegasusheavy/prax-orm/issues) - Bug reports and feature requests
- [Discussions](https://github.com/pegasusheavy/prax-orm/discussions) - Questions and community chat

## Getting Started

### 1. Find an Issue

Look for issues labeled:
- `good first issue` - Great for newcomers
- `help wanted` - We'd love your help
- `status:ready` - Ready to be worked on

### 2. Claim the Issue

Comment on the issue to let others know you're working on it:

```
I'd like to work on this! 🙋
```

### 3. Fork and Clone

```bash
# Fork on GitHub, then:
git clone https://github.com/YOUR_USERNAME/prax-orm.git
cd prax-orm

# Add upstream remote
git remote add upstream https://github.com/pegasusheavy/prax-orm.git
```

### 4. Create a Branch

Follow our [Git-Flow workflow](.cursor/rules/git-flow.mdc):

```bash
git checkout develop
git pull upstream develop
git checkout -b feature/your-feature-name
```

Branch naming:
- `feature/*` - New features
- `bugfix/*` - Bug fixes
- `docs/*` - Documentation
- `refactor/*` - Code refactoring

### 5. Make Your Changes

```bash
# Run tests
cargo test --all-features

# Check formatting
cargo fmt --all

# Run lints
cargo clippy --all-targets --all-features
```

### 6. Commit Your Changes

We use [Conventional Commits](https://www.conventionalcommits.org/):

```bash
git commit -m "feat(query): add soft delete support"
git commit -m "fix(postgres): handle null values in JSON columns"
git commit -m "docs: update getting started guide"
```

Types: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`

### 7. Push and Create PR

```bash
git push origin feature/your-feature-name
```

Then create a Pull Request on GitHub targeting `develop`.

## Development Setup

### Prerequisites

- Rust 1.89+ (2024 edition)
- Docker (for database testing)
- PostgreSQL, MySQL, or SQLite (for local testing)

### Building

```bash
# Build all crates
cargo build --all

# Build with all features
cargo build --all-features
```

### Testing

```bash
# Run all tests
cargo test --all-features

# Run specific crate tests
cargo test -p prax-query

# Run with coverage
cargo llvm-cov --all-features
```

### Docker Development

```bash
# Start databases
docker-compose up -d

# Run integration tests
cargo test --features integration
```

## Code Guidelines

### Rust Style

- Follow Rust 2024 edition idioms
- Use `async`/`await` for all I/O
- Prefer `?` operator for error propagation
- Add documentation comments for public APIs

### Testing

- Aim for 90%+ coverage
- Write unit tests for all public functions
- Include integration tests for database operations
- Use `#[tokio::test]` for async tests

### Documentation

- Document all public items
- Include examples in doc comments
- Update CHANGELOG.md for user-facing changes

## Pull Request Checklist

- [ ] Tests pass (`cargo test --all-features`)
- [ ] Code is formatted (`cargo fmt`)
- [ ] No clippy warnings (`cargo clippy`)
- [ ] Documentation updated
- [ ] CHANGELOG.md updated (if applicable)
- [ ] PR title follows conventional commits
- [ ] PR references related issue(s)

## Releasing (Maintainers)

### Preparing a Release

```bash
# Create release branch from develop
git checkout develop
git pull
git checkout -b release/0.2.0

# Update versions and CHANGELOG
./scripts/release.sh 0.2.0 --no-push

# Review changes, then push
git push -u origin release/0.2.0
```

### Publishing to crates.io

After the release PR is merged to `main`:

```bash
# Dry run first to verify
./scripts/publish.sh --dry-run

# Publish all crates (in dependency order)
./scripts/publish.sh
```

The publish script handles:
- Publishing crates in correct dependency order
- Waiting for crates.io to index between tiers
- Verifying tests pass before publishing

**Publish Order:**
1. Tier 1 (no deps): `prax-schema`, `prax-query`
2. Tier 2 (tier 1 deps): `prax-codegen`, `prax-migrate`, `prax-postgres`, `prax-mysql`, `prax-sqlite`, `prax-duckdb`, `prax-sqlx`
3. Tier 3 (tier 2 deps): `prax-armature`, `prax-axum`, `prax-actix`, `prax-orm-cli`
4. Tier 4 (main crate): `prax-orm`

## Code of Conduct

Be respectful and inclusive. We follow the [Rust Code of Conduct](https://www.rust-lang.org/policies/code-of-conduct).

## Questions?

- Open a [Discussion](https://github.com/pegasusheavy/prax-orm/discussions)
- Check existing issues and discussions first
- Tag maintainers if urgent

---

Thank you for contributing! 🙏
