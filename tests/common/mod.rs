//! Shared helpers for the PRAX_E2E-gated live-database tests.

#![allow(dead_code)]

/// Default URL of the `prax-postgres` docker-compose service.
const DEFAULT_POSTGRES_URL: &str = "postgres://prax:prax_test_password@localhost:5432/prax_test";

/// Whether live e2e tests are enabled (`PRAX_E2E=1`).
pub fn e2e_enabled() -> bool {
    std::env::var("PRAX_E2E").ok().as_deref() == Some("1")
}

/// The single e2e gating policy for Postgres: `None` unless `PRAX_E2E=1`;
/// otherwise `POSTGRES_URL`, defaulting to the docker-compose Postgres.
pub fn e2e_url() -> Option<String> {
    if !e2e_enabled() {
        return None;
    }
    Some(std::env::var("POSTGRES_URL").unwrap_or_else(|_| DEFAULT_POSTGRES_URL.to_owned()))
}
