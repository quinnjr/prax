//! Top-level Prax client grouping per-model accessors.
//!
//! A `PraxClient<E>` owns a `QueryEngine` and routes operations to the
//! per-model `Client<E>` values emitted by `#[derive(Model)]` or
//! `prax_schema!`. The `prax::client!(Foo, Bar, ...)` declarative macro
//! attaches one accessor per model to `PraxClient`:
//!
//! ```rust,ignore
//! use prax_orm::{client, Model, PraxClient};
//!
//! #[derive(Model)]
//! #[prax(table = "users")]
//! struct User { #[prax(id, auto)] id: i32, email: String }
//!
//! #[derive(Model)]
//! #[prax(table = "posts")]
//! struct Post { #[prax(id, auto)] id: i32, title: String }
//!
//! // Declares `trait PraxClientExt` with `user()`/`post()` accessors
//! // and implements it for `PraxClient<E>`. Call site has the trait in
//! // scope automatically because the macro emits it right there.
//! client!(User, Post);
//!
//! # async fn go<E: prax_query::traits::QueryEngine>(engine: E) {
//! let prax = PraxClient::new(engine);
//! let _ = prax.user().find_many();
//! let _ = prax.post().find_many();
//! # }
//! ```

use prax_query::error::QueryResult;
use prax_query::raw::Sql;
use prax_query::row::FromRow;
use prax_query::traits::{Model, QueryEngine};

/// Top-level client grouping every model's per-model `Client<E>`.
#[derive(Clone)]
pub struct PraxClient<E: QueryEngine> {
    engine: E,
}

impl<E: QueryEngine> PraxClient<E> {
    /// Create a new top-level client wrapping the given engine.
    pub fn new(engine: E) -> Self {
        Self { engine }
    }

    /// Borrow the underlying engine. Accessor macros clone it per call.
    pub fn engine(&self) -> &E {
        &self.engine
    }

    /// Execute a typed raw SQL query, decoding each returned row as `T`.
    ///
    /// The typed Client API covers the common cases, but every ORM
    /// eventually hits something it doesn't yet model — window functions,
    /// vendor-specific extensions, recursive CTEs, bespoke aggregates.
    /// `query_raw` is the escape hatch: build a parameterized
    /// [`prax_query::raw::Sql`] and route the result through the same
    /// [`FromRow`] bridge the derived models use, so the returned
    /// records stay typed.
    ///
    /// `T` must implement both [`Model`] (so the driver can associate
    /// the query with a table) and [`FromRow`] (so each row can be
    /// decoded). Both are provided automatically by `#[derive(Model)]`.
    ///
    /// ```rust,ignore
    /// use prax_query::raw::Sql;
    ///
    /// let users: Vec<User> = client
    ///     .query_raw(
    ///         Sql::new("SELECT id, email FROM users WHERE email = ")
    ///             .bind("alice@example.com"),
    ///     )
    ///     .await?;
    /// ```
    pub async fn query_raw<T>(&self, sql: Sql) -> QueryResult<Vec<T>>
    where
        T: Model + FromRow + Send + 'static,
    {
        let (s, p) = sql.build();
        self.engine.query_many::<T>(&s, p).await
    }

    /// Execute a raw statement that doesn't return rows.
    ///
    /// Use this for `INSERT` / `UPDATE` / `DELETE` / DDL when the typed
    /// Client API doesn't model what you need. Returns the
    /// driver-reported affected-row count.
    ///
    /// ```rust,ignore
    /// use prax_query::raw::Sql;
    ///
    /// let n = client
    ///     .execute_raw(
    ///         Sql::new("UPDATE users SET verified = TRUE WHERE id = ")
    ///             .bind(user_id),
    ///     )
    ///     .await?;
    /// assert_eq!(n, 1);
    /// ```
    pub async fn execute_raw(&self, sql: Sql) -> QueryResult<u64> {
        let (s, p) = sql.build();
        self.engine.execute_raw(&s, p).await
    }
}

/// Attach per-model accessors to `PraxClient<E>`.
///
/// Each identifier must name a model declared via `#[derive(Model)]` or
/// `prax_schema!`. For each `Foo` the macro emits a sealed extension
/// trait `PraxClientExt` with `fn foo(&self) -> foo::Client<E>` and
/// implements it for `PraxClient<E>`.
///
/// The extension-trait detour exists because Rust's orphan rule bans
/// downstream crates from writing inherent `impl` blocks for types they
/// do not own — callers use `PraxClient` from `prax_orm`, so they must
/// go through a trait. The `PraxClientExt` name is fixed; the trait is
/// brought into scope at the call site by the macro.
#[macro_export]
macro_rules! client {
    ($($model:ident),+ $(,)?) => {
        /// Generated per-application extension trait on `PraxClient<E>`.
        /// Calls like `client.user()` / `client.post()` dispatch through
        /// this trait.
        pub trait PraxClientExt<E: $crate::__prelude::QueryEngine> {
            $( $crate::__client_accessor_trait!($model); )+
        }

        impl<E: $crate::__prelude::QueryEngine> PraxClientExt<E>
            for $crate::PraxClient<E>
        {
            $( $crate::__client_accessor_impl!($model); )+
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __client_accessor_trait {
    ($model:ident) => {
        $crate::__paste::paste! {
            fn [<$model:snake>](&self) -> [<$model:snake>]::Client<E>;
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __client_accessor_impl {
    ($model:ident) => {
        $crate::__paste::paste! {
            fn [<$model:snake>](&self) -> [<$model:snake>]::Client<E> {
                [<$model:snake>]::Client::new(self.engine().clone())
            }
        }
    };
}

#[doc(hidden)]
pub use ::paste as __paste;

/// Re-exports used by the `client!` macro expansion. Keeps callers from
/// needing to import `prax_query::traits::QueryEngine` themselves.
#[doc(hidden)]
pub mod __prelude {
    pub use prax_query::traits::QueryEngine;
}
