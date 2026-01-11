# =============================================================================
# Prax ORM Development & Testing Dockerfile
# =============================================================================
# Multi-stage build for efficient caching and smaller final image

# -----------------------------------------------------------------------------
# Stage 1: Chef - Dependency caching
# -----------------------------------------------------------------------------
FROM rust:1.89-bookworm AS chef
RUN cargo install cargo-chef
WORKDIR /app

# -----------------------------------------------------------------------------
# Stage 2: Planner - Create dependency recipe
# -----------------------------------------------------------------------------
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# -----------------------------------------------------------------------------
# Stage 3: Builder - Build dependencies and project
# -----------------------------------------------------------------------------
FROM chef AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    libpq-dev \
    default-libmysqlclient-dev \
    && rm -rf /var/lib/apt/lists/*

# Build dependencies (cached layer)
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# Build the project
COPY . .
RUN cargo build --release --workspace

# -----------------------------------------------------------------------------
# Stage 4: Test Runner - For running integration tests
# -----------------------------------------------------------------------------
FROM rust:1.89-bookworm AS test-runner

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    libpq-dev \
    default-libmysqlclient-dev \
    postgresql-client \
    default-mysql-client \
    sqlite3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy source code for tests
COPY . .

# Install cargo tools for testing
RUN cargo install cargo-nextest cargo-llvm-cov

# Default command runs all tests
CMD ["cargo", "test", "--workspace", "--all-features"]

# -----------------------------------------------------------------------------
# Stage 5: Development - For local development with hot reload
# -----------------------------------------------------------------------------
FROM rust:1.89-bookworm AS development

# Install development dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    libpq-dev \
    default-libmysqlclient-dev \
    postgresql-client \
    default-mysql-client \
    sqlite3 \
    git \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Install useful cargo tools
RUN cargo install cargo-watch cargo-nextest cargo-llvm-cov cargo-deny

WORKDIR /app

# Mount point for source code
VOLUME ["/app"]

# Default command for development
CMD ["cargo", "watch", "-x", "check", "-x", "test"]

# -----------------------------------------------------------------------------
# Stage 6: CI - Minimal image for CI/CD pipelines
# -----------------------------------------------------------------------------
FROM rust:1.89-slim-bookworm AS ci

# Install minimal runtime dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    libpq-dev \
    default-libmysqlclient-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY . .

# Pre-fetch dependencies
RUN cargo fetch

CMD ["cargo", "test", "--workspace"]

