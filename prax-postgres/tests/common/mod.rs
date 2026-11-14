//! Shared helpers for the PRAX_E2E-gated Postgres tests.

/// Default URL of the `prax-postgres` docker-compose service.
const DEFAULT_POSTGRES_URL: &str = "postgres://prax:prax_test_password@localhost:5432/prax_test";

/// The single e2e gating policy: `None` unless `PRAX_E2E=1`; otherwise
/// `POSTGRES_URL`, defaulting to the docker-compose Postgres.
pub fn e2e_url() -> Option<String> {
    if std::env::var("PRAX_E2E").ok().as_deref() != Some("1") {
        return None;
    }
    Some(std::env::var("POSTGRES_URL").unwrap_or_else(|_| DEFAULT_POSTGRES_URL.to_owned()))
}
