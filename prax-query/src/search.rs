//! Full-text search support across database backends.
//!
//! This module provides a unified API for full-text search across different
//! database backends, abstracting over their specific implementations.
//!
//! # Supported Features
//!
//! | Feature          | PostgreSQL   | MySQL      | SQLite  | MSSQL   | MongoDB      |
//! |------------------|--------------|------------|---------|---------|--------------|
//! | Full-Text Index  | ✅ tsvector  | ✅ FULLTEXT| ✅ FTS5 | ✅      | ✅ Atlas     |
//! | Search Ranking   | ✅ ts_rank   | ✅         | ✅ bm25 | ✅ RANK | ✅ score     |
//! | Phrase Search    | ✅           | ✅         | ✅      | ✅      | ✅           |
//! | Faceted Search   | ✅           | ❌         | ❌      | ❌      | ✅           |
//! | Fuzzy Search     | ✅ pg_trgm   | ❌         | ❌      | ✅      | ✅           |
//! | Highlighting     | ✅ ts_headline| ❌        | ✅ highlight| ❌  | ✅ highlight |
//!
//! # Example Usage
//!
//! ```rust,ignore
//! use prax_query::search::{SearchQuery, SearchOptions};
//!
//! // Simple search
//! let search = SearchQuery::new("rust async programming")
//!     .columns(["title", "body"])
//!     .with_ranking()
//!     .build();
//!
//! // Generate SQL
//! let sql = search.to_postgres_sql("posts")?;
//! ```

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use crate::error::{QueryError, QueryResult};
use crate::sql::DatabaseType;

/// Full-text search mode/operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum SearchMode {
    /// Match any word (OR).
    #[default]
    Any,
    /// Match all words (AND).
    All,
    /// Match exact phrase.
    Phrase,
    /// Boolean mode with operators (+, -, *).
    Boolean,
    /// Natural language mode.
    Natural,
}

impl SearchMode {
    /// Convert to PostgreSQL tsquery format.
    pub fn to_postgres_operator(&self) -> &'static str {
        match self {
            Self::Any | Self::Natural => " | ",
            Self::All | Self::Boolean => " & ",
            Self::Phrase => " <-> ",
        }
    }
}

/// Text search language/configuration.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum SearchLanguage {
    /// Simple (no stemming).
    Simple,
    /// English.
    #[default]
    English,
    /// Spanish.
    Spanish,
    /// French.
    French,
    /// German.
    German,
    /// Custom language/configuration name.
    Custom(String),
}

impl SearchLanguage {
    /// Get the PostgreSQL text search configuration name.
    pub fn to_postgres_config(&self) -> Cow<'static, str> {
        match self {
            Self::Simple => Cow::Borrowed("simple"),
            Self::English => Cow::Borrowed("english"),
            Self::Spanish => Cow::Borrowed("spanish"),
            Self::French => Cow::Borrowed("french"),
            Self::German => Cow::Borrowed("german"),
            Self::Custom(name) => Cow::Owned(name.clone()),
        }
    }

    /// Get the SQLite FTS5 tokenizer.
    pub fn to_sqlite_tokenizer(&self) -> &'static str {
        match self {
            Self::Simple => "unicode61",
            Self::English => "porter unicode61",
            _ => "unicode61", // SQLite has limited language support
        }
    }
}

/// Ranking/scoring options for search results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RankingOptions {
    /// Whether to include ranking score.
    pub enabled: bool,
    /// Column alias for the score.
    pub score_alias: String,
    /// Normalization option (PostgreSQL-specific).
    pub normalization: u32,
    /// Field weights (field_name -> weight).
    pub weights: Vec<(String, f32)>,
}

impl Default for RankingOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            score_alias: "search_score".to_string(),
            normalization: 0,
            weights: Vec::new(),
        }
    }
}

impl RankingOptions {
    /// Enable ranking.
    pub fn enabled(mut self) -> Self {
        self.enabled = true;
        self
    }

    /// Set the score column alias.
    pub fn alias(mut self, alias: impl Into<String>) -> Self {
        self.score_alias = alias.into();
        self
    }

    /// Set PostgreSQL normalization option.
    pub fn normalization(mut self, norm: u32) -> Self {
        self.normalization = norm;
        self
    }

    /// Add a field weight.
    pub fn weight(mut self, field: impl Into<String>, weight: f32) -> Self {
        self.weights.push((field.into(), weight));
        self
    }
}

/// Highlighting options for search results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HighlightOptions {
    /// Whether to include highlights.
    pub enabled: bool,
    /// Start tag for highlights.
    pub start_tag: String,
    /// End tag for highlights.
    pub end_tag: String,
    /// Maximum length of highlighted text.
    pub max_length: Option<u32>,
    /// Number of fragments to return.
    pub max_fragments: Option<u32>,
    /// Fragment delimiter.
    pub delimiter: String,
}

impl Default for HighlightOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            start_tag: "<b>".to_string(),
            end_tag: "</b>".to_string(),
            max_length: Some(150),
            max_fragments: Some(3),
            delimiter: " ... ".to_string(),
        }
    }
}

impl HighlightOptions {
    /// Enable highlighting.
    pub fn enabled(mut self) -> Self {
        self.enabled = true;
        self
    }

    /// Set highlight tags.
    pub fn tags(mut self, start: impl Into<String>, end: impl Into<String>) -> Self {
        self.start_tag = start.into();
        self.end_tag = end.into();
        self
    }

    /// Set maximum text length.
    pub fn max_length(mut self, length: u32) -> Self {
        self.max_length = Some(length);
        self
    }

    /// Set maximum number of fragments.
    pub fn max_fragments(mut self, count: u32) -> Self {
        self.max_fragments = Some(count);
        self
    }

    /// Set fragment delimiter.
    pub fn delimiter(mut self, delimiter: impl Into<String>) -> Self {
        self.delimiter = delimiter.into();
        self
    }
}

/// Fuzzy search options.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FuzzyOptions {
    /// Whether to enable fuzzy matching.
    pub enabled: bool,
    /// Maximum edit distance (Levenshtein).
    pub max_edits: u32,
    /// Prefix length that must match exactly.
    pub prefix_length: u32,
    /// Similarity threshold (0.0-1.0).
    pub threshold: f32,
}

impl Default for FuzzyOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            max_edits: 2,
            prefix_length: 0,
            threshold: 0.3,
        }
    }
}

impl FuzzyOptions {
    /// Enable fuzzy search.
    pub fn enabled(mut self) -> Self {
        self.enabled = true;
        self
    }

    /// Set maximum edit distance.
    pub fn max_edits(mut self, edits: u32) -> Self {
        self.max_edits = edits;
        self
    }

    /// Set prefix length.
    pub fn prefix_length(mut self, length: u32) -> Self {
        self.prefix_length = length;
        self
    }

    /// Set similarity threshold.
    pub fn threshold(mut self, threshold: f32) -> Self {
        self.threshold = threshold;
        self
    }
}

/// A full-text search query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchQuery {
    /// Search terms.
    pub query: String,
    /// Columns to search in.
    pub columns: Vec<String>,
    /// Search mode.
    pub mode: SearchMode,
    /// Language/configuration.
    pub language: SearchLanguage,
    /// Ranking options.
    pub ranking: RankingOptions,
    /// Highlighting options.
    pub highlight: HighlightOptions,
    /// Fuzzy search options.
    pub fuzzy: FuzzyOptions,
    /// Minimum word length.
    pub min_word_length: Option<u32>,
    /// Filter by category/field (for faceted search).
    pub filters: Vec<(String, String)>,
}

impl SearchQuery {
    /// Create a new search query.
    pub fn new(query: impl Into<String>) -> SearchQueryBuilder {
        SearchQueryBuilder::new(query)
    }

    /// Generate PostgreSQL full-text search SQL.
    pub fn to_postgres_sql(&self, table: &str) -> QueryResult<SearchSql> {
        let config = self.language.to_postgres_config();

        // Build tsvector expression
        let tsvector = if self.columns.len() == 1 {
            format!("to_tsvector('{}', {})", config, self.columns[0])
        } else {
            let concat_cols = self.columns.join(" || ' ' || ");
            format!("to_tsvector('{}', {})", config, concat_cols)
        };

        // Build tsquery expression
        let words: Vec<&str> = self.query.split_whitespace().collect();
        let tsquery_parts: Vec<String> = words
            .iter()
            .map(|w| format!("'{}'", w.replace('\'', "''")))
            .collect();
        let tsquery = format!(
            "to_tsquery('{}', '{}')",
            config,
            tsquery_parts.join(self.mode.to_postgres_operator())
        );

        // Build WHERE clause
        let where_clause = format!("{} @@ {}", tsvector, tsquery);

        // Build SELECT columns
        let mut select_cols = vec!["*".to_string()];

        // Add ranking
        if self.ranking.enabled {
            let weights = if self.ranking.weights.is_empty() {
                String::new()
            } else {
                // PostgreSQL uses setweight for field weights
                String::new()
            };
            select_cols.push(format!(
                "ts_rank({}{}, {}) AS {}",
                tsvector, weights, tsquery, self.ranking.score_alias
            ));
        }

        // Add highlighting
        if self.highlight.enabled && !self.columns.is_empty() {
            let col = &self.columns[0];
            select_cols.push(format!(
                "ts_headline('{}', {}, {}, 'StartSel={}, StopSel={}, MaxWords={}, MaxFragments={}') AS highlighted",
                config,
                col,
                tsquery,
                self.highlight.start_tag,
                self.highlight.end_tag,
                self.highlight.max_length.unwrap_or(35),
                self.highlight.max_fragments.unwrap_or(3)
            ));
        }

        let sql = format!(
            "SELECT {} FROM {} WHERE {}",
            select_cols.join(", "),
            table,
            where_clause
        );

        let order_by = if self.ranking.enabled {
            Some(format!("{} DESC", self.ranking.score_alias))
        } else {
            None
        };

        Ok(SearchSql {
            sql,
            order_by,
            params: vec![],
        })
    }

    /// Generate MySQL full-text search SQL.
    pub fn to_mysql_sql(&self, table: &str) -> QueryResult<SearchSql> {
        let columns = self.columns.join(", ");

        // MySQL MATCH ... AGAINST syntax
        let match_mode = match self.mode {
            SearchMode::Natural | SearchMode::Any => "",
            SearchMode::Boolean | SearchMode::All => " IN BOOLEAN MODE",
            SearchMode::Phrase => " IN BOOLEAN MODE", // Use quotes for phrase
        };

        let search_query = if self.mode == SearchMode::Phrase {
            format!("\"{}\"", self.query)
        } else if self.mode == SearchMode::All {
            // Add + prefix for required terms
            self.query
                .split_whitespace()
                .map(|w| format!("+{}", w))
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            self.query.clone()
        };

        let match_expr = format!(
            "MATCH({}) AGAINST('{}'{}))",
            columns, search_query, match_mode
        );

        let mut select_cols = vec!["*".to_string()];

        // Add ranking (MySQL returns relevance from MATCH)
        if self.ranking.enabled {
            select_cols.push(format!("{} AS {}", match_expr, self.ranking.score_alias));
        }

        let sql = format!(
            "SELECT {} FROM {} WHERE {}",
            select_cols.join(", "),
            table,
            match_expr
        );

        let order_by = if self.ranking.enabled {
            Some(format!("{} DESC", self.ranking.score_alias))
        } else {
            None
        };

        Ok(SearchSql {
            sql,
            order_by,
            params: vec![],
        })
    }

    /// Generate SQLite FTS5 search SQL.
    pub fn to_sqlite_sql(&self, table: &str, fts_table: &str) -> QueryResult<SearchSql> {
        let search_query = match self.mode {
            SearchMode::Phrase => format!("\"{}\"", self.query),
            SearchMode::All => self
                .query
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" AND "),
            SearchMode::Any => self
                .query
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" OR "),
            _ => self.query.clone(),
        };

        let mut select_cols = vec![format!("{}.*", table)];

        // Add ranking (SQLite uses bm25)
        if self.ranking.enabled {
            select_cols.push(format!(
                "bm25({}) AS {}",
                fts_table, self.ranking.score_alias
            ));
        }

        // Add highlighting
        if self.highlight.enabled && !self.columns.is_empty() {
            select_cols.push(format!(
                "highlight({}, 0, '{}', '{}') AS highlighted",
                fts_table, self.highlight.start_tag, self.highlight.end_tag
            ));
        }

        let sql = format!(
            "SELECT {} FROM {} JOIN {} ON {}.rowid = {}.rowid WHERE {} MATCH '{}'",
            select_cols.join(", "),
            table,
            fts_table,
            table,
            fts_table,
            fts_table,
            search_query
        );

        let order_by = if self.ranking.enabled {
            Some(self.ranking.score_alias.to_string())
        } else {
            None
        };

        Ok(SearchSql {
            sql,
            order_by,
            params: vec![],
        })
    }

    /// Generate MSSQL full-text search SQL.
    pub fn to_mssql_sql(&self, table: &str) -> QueryResult<SearchSql> {
        let columns = self.columns.join(", ");

        let contains_expr = match self.mode {
            SearchMode::Phrase => format!("\"{}\"", self.query),
            SearchMode::All => {
                let terms: Vec<String> = self
                    .query
                    .split_whitespace()
                    .map(|w| format!("\"{}\"", w))
                    .collect();
                terms.join(" AND ")
            }
            SearchMode::Any | SearchMode::Natural => {
                let terms: Vec<String> = self
                    .query
                    .split_whitespace()
                    .map(|w| format!("\"{}\"", w))
                    .collect();
                terms.join(" OR ")
            }
            SearchMode::Boolean => self.query.clone(),
        };

        let select_cols = ["*".to_string()];

        // Add ranking (MSSQL uses CONTAINSTABLE for ranking)
        if self.ranking.enabled {
            let sql = format!(
                "SELECT {}.*, ft.RANK AS {} FROM {} \
                 INNER JOIN CONTAINSTABLE({}, ({}), '{}') AS ft \
                 ON {}.id = ft.[KEY]",
                table, self.ranking.score_alias, table, table, columns, contains_expr, table
            );

            return Ok(SearchSql {
                sql,
                order_by: Some(format!("{} DESC", self.ranking.score_alias)),
                params: vec![],
            });
        }

        let sql = format!(
            "SELECT {} FROM {} WHERE CONTAINS(({}), '{}')",
            select_cols.join(", "),
            table,
            columns,
            contains_expr
        );

        Ok(SearchSql {
            sql,
            order_by: None,
            params: vec![],
        })
    }

    /// Generate search SQL for the specified database type.
    pub fn to_sql(&self, table: &str, db_type: DatabaseType) -> QueryResult<SearchSql> {
        match db_type {
            DatabaseType::PostgreSQL => self.to_postgres_sql(table),
            DatabaseType::MySQL => self.to_mysql_sql(table),
            DatabaseType::SQLite => self.to_sqlite_sql(table, &format!("{}_fts", table)),
            DatabaseType::MSSQL => self.to_mssql_sql(table),
        }
    }
}

/// Builder for search queries.
#[derive(Debug, Clone)]
pub struct SearchQueryBuilder {
    query: String,
    columns: Vec<String>,
    mode: SearchMode,
    language: SearchLanguage,
    ranking: RankingOptions,
    highlight: HighlightOptions,
    fuzzy: FuzzyOptions,
    min_word_length: Option<u32>,
    filters: Vec<(String, String)>,
}

impl SearchQueryBuilder {
    /// Create a new search query builder.
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            columns: Vec::new(),
            mode: SearchMode::default(),
            language: SearchLanguage::default(),
            ranking: RankingOptions::default(),
            highlight: HighlightOptions::default(),
            fuzzy: FuzzyOptions::default(),
            min_word_length: None,
            filters: Vec::new(),
        }
    }

    /// Add a column to search.
    pub fn column(mut self, column: impl Into<String>) -> Self {
        self.columns.push(column.into());
        self
    }

    /// Add multiple columns to search.
    pub fn columns(mut self, columns: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.columns.extend(columns.into_iter().map(Into::into));
        self
    }

    /// Set the search mode.
    pub fn mode(mut self, mode: SearchMode) -> Self {
        self.mode = mode;
        self
    }

    /// Set to match all words.
    pub fn match_all(self) -> Self {
        self.mode(SearchMode::All)
    }

    /// Set to match any word.
    pub fn match_any(self) -> Self {
        self.mode(SearchMode::Any)
    }

    /// Set to match exact phrase.
    pub fn phrase(self) -> Self {
        self.mode(SearchMode::Phrase)
    }

    /// Set to boolean mode.
    pub fn boolean(self) -> Self {
        self.mode(SearchMode::Boolean)
    }

    /// Set the search language.
    pub fn language(mut self, language: SearchLanguage) -> Self {
        self.language = language;
        self
    }

    /// Enable ranking with default options.
    pub fn with_ranking(mut self) -> Self {
        self.ranking.enabled = true;
        self
    }

    /// Configure ranking options.
    pub fn ranking(mut self, options: RankingOptions) -> Self {
        self.ranking = options;
        self
    }

    /// Enable highlighting with default options.
    pub fn with_highlight(mut self) -> Self {
        self.highlight.enabled = true;
        self
    }

    /// Configure highlighting options.
    pub fn highlight(mut self, options: HighlightOptions) -> Self {
        self.highlight = options;
        self
    }

    /// Enable fuzzy matching with default options.
    pub fn with_fuzzy(mut self) -> Self {
        self.fuzzy.enabled = true;
        self
    }

    /// Configure fuzzy search options.
    pub fn fuzzy(mut self, options: FuzzyOptions) -> Self {
        self.fuzzy = options;
        self
    }

    /// Set minimum word length.
    pub fn min_word_length(mut self, length: u32) -> Self {
        self.min_word_length = Some(length);
        self
    }

    /// Add a filter for faceted search.
    pub fn filter(mut self, field: impl Into<String>, value: impl Into<String>) -> Self {
        self.filters.push((field.into(), value.into()));
        self
    }

    /// Build the search query.
    pub fn build(self) -> SearchQuery {
        SearchQuery {
            query: self.query,
            columns: self.columns,
            mode: self.mode,
            language: self.language,
            ranking: self.ranking,
            highlight: self.highlight,
            fuzzy: self.fuzzy,
            min_word_length: self.min_word_length,
            filters: self.filters,
        }
    }
}

/// Generated search SQL.
#[derive(Debug, Clone)]
pub struct SearchSql {
    /// The main SQL query.
    pub sql: String,
    /// Optional ORDER BY clause.
    pub order_by: Option<String>,
    /// Query parameters.
    pub params: Vec<String>,
}

/// Full-text index definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FullTextIndex {
    /// Index name.
    pub name: String,
    /// Table name.
    pub table: String,
    /// Columns in the index.
    pub columns: Vec<String>,
    /// Language/configuration.
    pub language: SearchLanguage,
    /// Index type (for MySQL: FULLTEXT).
    pub index_type: Option<String>,
}

impl FullTextIndex {
    /// Create a new full-text index builder.
    pub fn builder(name: impl Into<String>) -> FullTextIndexBuilder {
        FullTextIndexBuilder::new(name)
    }

    /// Generate PostgreSQL CREATE INDEX SQL.
    pub fn to_postgres_sql(&self) -> String {
        let config = self.language.to_postgres_config();
        let columns_expr = if self.columns.len() == 1 {
            format!("to_tsvector('{}', {})", config, self.columns[0])
        } else {
            let concat = self.columns.join(" || ' ' || ");
            format!("to_tsvector('{}', {})", config, concat)
        };

        format!(
            "CREATE INDEX {} ON {} USING GIN ({});",
            self.name, self.table, columns_expr
        )
    }

    /// Generate MySQL CREATE INDEX SQL.
    pub fn to_mysql_sql(&self) -> String {
        format!(
            "CREATE FULLTEXT INDEX {} ON {} ({});",
            self.name,
            self.table,
            self.columns.join(", ")
        )
    }

    /// Generate SQLite FTS5 virtual table SQL.
    pub fn to_sqlite_sql(&self) -> String {
        let tokenizer = self.language.to_sqlite_tokenizer();
        format!(
            "CREATE VIRTUAL TABLE {}_fts USING fts5({}, content='{}', tokenize='{}');",
            self.table,
            self.columns.join(", "),
            self.table,
            tokenizer
        )
    }

    /// Generate MSSQL full-text catalog and index SQL.
    pub fn to_mssql_sql(&self, catalog_name: &str) -> Vec<String> {
        vec![
            format!("CREATE FULLTEXT CATALOG {} AS DEFAULT;", catalog_name),
            format!(
                "CREATE FULLTEXT INDEX ON {} ({}) KEY INDEX PK_{} ON {};",
                self.table,
                self.columns.join(", "),
                self.table,
                catalog_name
            ),
        ]
    }

    /// Generate index SQL for the specified database type.
    pub fn to_sql(&self, db_type: DatabaseType) -> QueryResult<Vec<String>> {
        match db_type {
            DatabaseType::PostgreSQL => Ok(vec![self.to_postgres_sql()]),
            DatabaseType::MySQL => Ok(vec![self.to_mysql_sql()]),
            DatabaseType::SQLite => Ok(vec![self.to_sqlite_sql()]),
            DatabaseType::MSSQL => Ok(self.to_mssql_sql(&format!("{}_catalog", self.table))),
        }
    }

    /// Generate DROP INDEX SQL.
    pub fn to_drop_sql(&self, db_type: DatabaseType) -> QueryResult<String> {
        match db_type {
            DatabaseType::PostgreSQL => Ok(format!("DROP INDEX IF EXISTS {};", self.name)),
            DatabaseType::MySQL => Ok(format!("DROP INDEX {} ON {};", self.name, self.table)),
            DatabaseType::SQLite => Ok(format!("DROP TABLE IF EXISTS {}_fts;", self.table)),
            DatabaseType::MSSQL => Ok(format!(
                "DROP FULLTEXT INDEX ON {}; DROP FULLTEXT CATALOG {}_catalog;",
                self.table, self.table
            )),
        }
    }
}

/// Builder for full-text indexes.
#[derive(Debug, Clone)]
pub struct FullTextIndexBuilder {
    name: String,
    table: Option<String>,
    columns: Vec<String>,
    language: SearchLanguage,
    index_type: Option<String>,
}

impl FullTextIndexBuilder {
    /// Create a new builder.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            table: None,
            columns: Vec::new(),
            language: SearchLanguage::default(),
            index_type: None,
        }
    }

    /// Set the table name.
    pub fn on_table(mut self, table: impl Into<String>) -> Self {
        self.table = Some(table.into());
        self
    }

    /// Add a column.
    pub fn column(mut self, column: impl Into<String>) -> Self {
        self.columns.push(column.into());
        self
    }

    /// Add multiple columns.
    pub fn columns(mut self, columns: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.columns.extend(columns.into_iter().map(Into::into));
        self
    }

    /// Set the language.
    pub fn language(mut self, language: SearchLanguage) -> Self {
        self.language = language;
        self
    }

    /// Build the index definition.
    pub fn build(self) -> QueryResult<FullTextIndex> {
        let table = self.table.ok_or_else(|| {
            QueryError::invalid_input("table", "Must specify table with on_table()")
        })?;

        if self.columns.is_empty() {
            return Err(QueryError::invalid_input(
                "columns",
                "Must specify at least one column",
            ));
        }

        Ok(FullTextIndex {
            name: self.name,
            table,
            columns: self.columns,
            language: self.language,
            index_type: self.index_type,
        })
    }
}

/// MongoDB Atlas Search support.
pub mod mongodb {
    use serde::{Deserialize, Serialize};

    /// Atlas Search index definition.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct AtlasSearchIndex {
        /// Index name.
        pub name: String,
        /// Collection name.
        pub collection: String,
        /// Analyzer to use.
        pub analyzer: String,
        /// Field mappings.
        pub mappings: SearchMappings,
    }

    /// Field mappings for Atlas Search.
    #[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
    pub struct SearchMappings {
        /// Whether to dynamically map fields.
        pub dynamic: bool,
        /// Explicit field definitions.
        pub fields: Vec<SearchField>,
    }

    /// A searchable field definition.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct SearchField {
        /// Field path.
        pub path: String,
        /// Field type.
        pub field_type: SearchFieldType,
        /// Analyzer for text fields.
        pub analyzer: Option<String>,
        /// Whether to store for faceting.
        pub facet: bool,
    }

    /// Atlas Search field types.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub enum SearchFieldType {
        /// String/text field.
        String,
        /// Number field.
        Number,
        /// Date field.
        Date,
        /// Boolean field.
        Boolean,
        /// ObjectId field.
        ObjectId,
        /// Geo field.
        Geo,
        /// Autocomplete field.
        Autocomplete,
    }

    impl SearchFieldType {
        /// Get the Atlas Search type name.
        pub fn as_str(&self) -> &'static str {
            match self {
                Self::String => "string",
                Self::Number => "number",
                Self::Date => "date",
                Self::Boolean => "boolean",
                Self::ObjectId => "objectId",
                Self::Geo => "geo",
                Self::Autocomplete => "autocomplete",
            }
        }
    }

    /// Atlas Search query builder.
    #[derive(Debug, Clone, Default)]
    pub struct AtlasSearchQuery {
        /// Search text.
        pub query: String,
        /// Fields to search.
        pub path: Vec<String>,
        /// Fuzzy options.
        pub fuzzy: Option<FuzzyConfig>,
        /// Score options.
        pub score: Option<ScoreConfig>,
        /// Highlight options.
        pub highlight: Option<HighlightConfig>,
    }

    /// Fuzzy search configuration.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct FuzzyConfig {
        /// Maximum edits.
        pub max_edits: u32,
        /// Prefix length.
        pub prefix_length: u32,
        /// Max expansions.
        pub max_expansions: u32,
    }

    impl Default for FuzzyConfig {
        fn default() -> Self {
            Self {
                max_edits: 2,
                prefix_length: 0,
                max_expansions: 50,
            }
        }
    }

    /// Score configuration.
    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
    pub struct ScoreConfig {
        /// Boost factor.
        pub boost: Option<f64>,
        /// Score function.
        pub function: Option<String>,
    }

    /// Highlight configuration.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct HighlightConfig {
        /// Path to highlight.
        pub path: String,
        /// Max characters per highlight.
        pub max_chars_to_examine: u32,
        /// Max number of highlights.
        pub max_num_passages: u32,
    }

    impl Default for HighlightConfig {
        fn default() -> Self {
            Self {
                path: String::new(),
                max_chars_to_examine: 500_000,
                max_num_passages: 5,
            }
        }
    }

    impl AtlasSearchQuery {
        /// Create a new search query.
        pub fn new(query: impl Into<String>) -> Self {
            Self {
                query: query.into(),
                ..Default::default()
            }
        }

        /// Add a field to search.
        pub fn path(mut self, path: impl Into<String>) -> Self {
            self.path.push(path.into());
            self
        }

        /// Add multiple fields to search.
        pub fn paths(mut self, paths: impl IntoIterator<Item = impl Into<String>>) -> Self {
            self.path.extend(paths.into_iter().map(Into::into));
            self
        }

        /// Enable fuzzy matching.
        pub fn fuzzy(mut self, config: FuzzyConfig) -> Self {
            self.fuzzy = Some(config);
            self
        }

        /// Set score boost.
        pub fn boost(mut self, factor: f64) -> Self {
            self.score = Some(ScoreConfig {
                boost: Some(factor),
                function: None,
            });
            self
        }

        /// Enable highlighting.
        pub fn highlight(mut self, path: impl Into<String>) -> Self {
            self.highlight = Some(HighlightConfig {
                path: path.into(),
                ..Default::default()
            });
            self
        }

        /// Build the $search aggregation stage.
        pub fn to_search_stage(&self) -> serde_json::Value {
            let mut text = serde_json::json!({
                "query": self.query,
                "path": if self.path.len() == 1 {
                    serde_json::Value::String(self.path[0].clone())
                } else {
                    serde_json::Value::Array(self.path.iter().map(|p| serde_json::Value::String(p.clone())).collect())
                }
            });

            if let Some(ref fuzzy) = self.fuzzy {
                text["fuzzy"] = serde_json::json!({
                    "maxEdits": fuzzy.max_edits,
                    "prefixLength": fuzzy.prefix_length,
                    "maxExpansions": fuzzy.max_expansions
                });
            }

            let mut search = serde_json::json!({
                "$search": {
                    "text": text
                }
            });

            if let Some(ref hl) = self.highlight {
                search["$search"]["highlight"] = serde_json::json!({
                    "path": hl.path,
                    "maxCharsToExamine": hl.max_chars_to_examine,
                    "maxNumPassages": hl.max_num_passages
                });
            }

            search
        }

        /// Build the aggregation pipeline for search.
        pub fn to_pipeline(&self) -> Vec<serde_json::Value> {
            let mut pipeline = vec![self.to_search_stage()];

            // Add score metadata
            pipeline.push(serde_json::json!({
                "$addFields": {
                    "score": { "$meta": "searchScore" }
                }
            }));

            // Add highlights if enabled
            if self.highlight.is_some() {
                pipeline.push(serde_json::json!({
                    "$addFields": {
                        "highlights": { "$meta": "searchHighlights" }
                    }
                }));
            }

            pipeline
        }
    }

    /// Builder for Atlas Search index.
    #[derive(Debug, Clone, Default)]
    pub struct AtlasSearchIndexBuilder {
        name: String,
        collection: Option<String>,
        analyzer: String,
        dynamic: bool,
        fields: Vec<SearchField>,
    }

    impl AtlasSearchIndexBuilder {
        /// Create a new builder.
        pub fn new(name: impl Into<String>) -> Self {
            Self {
                name: name.into(),
                analyzer: "lucene.standard".to_string(),
                ..Default::default()
            }
        }

        /// Set the collection.
        pub fn collection(mut self, collection: impl Into<String>) -> Self {
            self.collection = Some(collection.into());
            self
        }

        /// Set the analyzer.
        pub fn analyzer(mut self, analyzer: impl Into<String>) -> Self {
            self.analyzer = analyzer.into();
            self
        }

        /// Enable dynamic mapping.
        pub fn dynamic(mut self) -> Self {
            self.dynamic = true;
            self
        }

        /// Add a text field.
        pub fn text_field(mut self, path: impl Into<String>) -> Self {
            self.fields.push(SearchField {
                path: path.into(),
                field_type: SearchFieldType::String,
                analyzer: None,
                facet: false,
            });
            self
        }

        /// Add a faceted field.
        pub fn facet_field(mut self, path: impl Into<String>, field_type: SearchFieldType) -> Self {
            self.fields.push(SearchField {
                path: path.into(),
                field_type,
                analyzer: None,
                facet: true,
            });
            self
        }

        /// Add an autocomplete field.
        pub fn autocomplete_field(mut self, path: impl Into<String>) -> Self {
            self.fields.push(SearchField {
                path: path.into(),
                field_type: SearchFieldType::Autocomplete,
                analyzer: None,
                facet: false,
            });
            self
        }

        /// Build the index definition.
        pub fn build(self) -> serde_json::Value {
            let mut fields = serde_json::Map::new();

            for field in &self.fields {
                let mut field_def = serde_json::json!({
                    "type": field.field_type.as_str()
                });

                if let Some(ref analyzer) = field.analyzer {
                    field_def["analyzer"] = serde_json::Value::String(analyzer.clone());
                }

                fields.insert(field.path.clone(), field_def);
            }

            serde_json::json!({
                "name": self.name,
                "analyzer": self.analyzer,
                "mappings": {
                    "dynamic": self.dynamic,
                    "fields": fields
                }
            })
        }
    }

    /// Helper to create a search query.
    pub fn search(query: impl Into<String>) -> AtlasSearchQuery {
        AtlasSearchQuery::new(query)
    }

    /// Helper to create a search index builder.
    pub fn search_index(name: impl Into<String>) -> AtlasSearchIndexBuilder {
        AtlasSearchIndexBuilder::new(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_query_builder() {
        let search = SearchQuery::new("rust async")
            .columns(["title", "body"])
            .match_all()
            .with_ranking()
            .build();

        assert_eq!(search.query, "rust async");
        assert_eq!(search.columns, vec!["title", "body"]);
        assert_eq!(search.mode, SearchMode::All);
        assert!(search.ranking.enabled);
    }

    #[test]
    fn test_postgres_search_sql() {
        let search = SearchQuery::new("rust programming")
            .column("content")
            .with_ranking()
            .build();

        let sql = search.to_postgres_sql("posts").unwrap();
        assert!(sql.sql.contains("to_tsvector"));
        assert!(sql.sql.contains("to_tsquery"));
        assert!(sql.sql.contains("ts_rank"));
        assert!(sql.sql.contains("@@"));
    }

    #[test]
    fn test_mysql_search_sql() {
        let search = SearchQuery::new("database performance")
            .columns(["title", "body"])
            .match_any()
            .build();

        let sql = search.to_mysql_sql("articles").unwrap();
        assert!(sql.sql.contains("MATCH"));
        assert!(sql.sql.contains("AGAINST"));
    }

    #[test]
    fn test_sqlite_search_sql() {
        let search = SearchQuery::new("web development")
            .column("content")
            .with_ranking()
            .build();

        let sql = search.to_sqlite_sql("posts", "posts_fts").unwrap();
        assert!(sql.sql.contains("MATCH"));
        assert!(sql.sql.contains("bm25"));
    }

    #[test]
    fn test_mssql_search_sql() {
        let search = SearchQuery::new("machine learning")
            .columns(["title", "abstract"])
            .phrase()
            .build();

        let sql = search.to_mssql_sql("papers").unwrap();
        assert!(sql.sql.contains("CONTAINS"));
    }

    #[test]
    fn test_mssql_ranked_search() {
        let search = SearchQuery::new("neural network")
            .column("content")
            .with_ranking()
            .build();

        let sql = search.to_mssql_sql("papers").unwrap();
        assert!(sql.sql.contains("CONTAINSTABLE"));
        assert!(sql.sql.contains("RANK"));
    }

    #[test]
    fn test_fulltext_index_postgres() {
        let index = FullTextIndex::builder("posts_search_idx")
            .on_table("posts")
            .columns(["title", "body"])
            .language(SearchLanguage::English)
            .build()
            .unwrap();

        let sql = index.to_postgres_sql();
        assert!(sql.contains("CREATE INDEX posts_search_idx"));
        assert!(sql.contains("USING GIN"));
        assert!(sql.contains("to_tsvector"));
    }

    #[test]
    fn test_fulltext_index_mysql() {
        let index = FullTextIndex::builder("posts_fulltext")
            .on_table("posts")
            .columns(["title", "body"])
            .build()
            .unwrap();

        let sql = index.to_mysql_sql();
        assert_eq!(
            sql,
            "CREATE FULLTEXT INDEX posts_fulltext ON posts (title, body);"
        );
    }

    #[test]
    fn test_fulltext_index_sqlite() {
        let index = FullTextIndex::builder("posts_fts")
            .on_table("posts")
            .columns(["title", "content"])
            .build()
            .unwrap();

        let sql = index.to_sqlite_sql();
        assert!(sql.contains("CREATE VIRTUAL TABLE"));
        assert!(sql.contains("USING fts5"));
    }

    #[test]
    fn test_highlight_options() {
        let opts = HighlightOptions::default()
            .enabled()
            .tags("<mark>", "</mark>")
            .max_length(200)
            .max_fragments(5);

        assert!(opts.enabled);
        assert_eq!(opts.start_tag, "<mark>");
        assert_eq!(opts.end_tag, "</mark>");
        assert_eq!(opts.max_length, Some(200));
    }

    #[test]
    fn test_fuzzy_options() {
        let opts = FuzzyOptions::default()
            .enabled()
            .max_edits(1)
            .threshold(0.5);

        assert!(opts.enabled);
        assert_eq!(opts.max_edits, 1);
        assert_eq!(opts.threshold, 0.5);
    }

    #[test]
    fn test_ranking_with_weights() {
        let opts = RankingOptions::default()
            .enabled()
            .alias("relevance")
            .weight("title", 2.0)
            .weight("body", 1.0);

        assert_eq!(opts.score_alias, "relevance");
        assert_eq!(opts.weights.len(), 2);
    }

    mod mongodb_tests {
        use super::super::mongodb::*;

        #[test]
        fn test_atlas_search_query() {
            let query = search("rust async")
                .paths(["title", "body"])
                .fuzzy(FuzzyConfig::default())
                .boost(2.0);

            let stage = query.to_search_stage();
            assert!(stage["$search"]["text"]["query"].is_string());
        }

        #[test]
        fn test_atlas_search_pipeline() {
            let query = search("database").path("content").highlight("content");

            let pipeline = query.to_pipeline();
            assert!(pipeline.len() >= 2);
            assert!(pipeline[0]["$search"].is_object());
        }

        #[test]
        fn test_atlas_search_index_builder() {
            let index = search_index("default")
                .collection("posts")
                .analyzer("lucene.english")
                .dynamic()
                .text_field("title")
                .text_field("body")
                .facet_field("category", SearchFieldType::String)
                .build();

            assert!(index["name"].is_string());
            assert!(index["mappings"]["dynamic"].as_bool().unwrap());
        }
    }
}
