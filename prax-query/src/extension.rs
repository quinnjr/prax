//! Database extensions and plugins support.
//!
//! This module provides types for managing database extensions and
//! specialized functionality like geospatial, UUID, cryptography, and vector search.
//!
//! # Database Support
//!
//! | Feature        | PostgreSQL       | MySQL      | SQLite        | MSSQL      | MongoDB        |
//! |----------------|------------------|------------|---------------|------------|----------------|
//! | Extensions     | ✅ CREATE EXT    | ❌         | ✅ load_ext   | ❌         | ❌             |
//! | Geospatial     | ✅ PostGIS       | ✅ Spatial | ✅ SpatiaLite | ✅         | ✅ GeoJSON     |
//! | UUID           | ✅ uuid-ossp     | ✅ built-in| ❌            | ✅ NEWID() | ✅ UUID()      |
//! | Cryptography   | ✅ pgcrypto      | ✅ built-in| ❌            | ✅         | ✅             |
//! | Vector Search  | ✅ pgvector      | ❌         | ❌            | ❌         | ✅ Atlas Vector|

use serde::{Deserialize, Serialize};

use crate::sql::DatabaseType;

// ============================================================================
// Extension Management
// ============================================================================

/// A database extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Extension {
    /// Extension name.
    pub name: String,
    /// Schema to install in (PostgreSQL).
    pub schema: Option<String>,
    /// Version to install.
    pub version: Option<String>,
    /// Whether to cascade dependencies.
    pub cascade: bool,
}

impl Extension {
    /// Create a new extension.
    pub fn new(name: impl Into<String>) -> ExtensionBuilder {
        ExtensionBuilder::new(name)
    }

    /// Common PostgreSQL extensions.
    pub fn postgis() -> Self {
        Self::new("postgis").build()
    }

    pub fn pgvector() -> Self {
        Self::new("vector").build()
    }

    pub fn uuid_ossp() -> Self {
        Self::new("uuid-ossp").build()
    }

    pub fn pgcrypto() -> Self {
        Self::new("pgcrypto").build()
    }

    pub fn pg_trgm() -> Self {
        Self::new("pg_trgm").build()
    }

    pub fn hstore() -> Self {
        Self::new("hstore").build()
    }

    pub fn ltree() -> Self {
        Self::new("ltree").build()
    }

    /// Generate PostgreSQL CREATE EXTENSION SQL.
    pub fn to_postgres_create(&self) -> String {
        let mut sql = format!("CREATE EXTENSION IF NOT EXISTS \"{}\"", self.name);

        if let Some(ref schema) = self.schema {
            sql.push_str(&format!(" SCHEMA {}", schema));
        }

        if let Some(ref version) = self.version {
            sql.push_str(&format!(" VERSION '{}'", version));
        }

        if self.cascade {
            sql.push_str(" CASCADE");
        }

        sql
    }

    /// Generate DROP EXTENSION SQL.
    pub fn to_postgres_drop(&self) -> String {
        let mut sql = format!("DROP EXTENSION IF EXISTS \"{}\"", self.name);
        if self.cascade {
            sql.push_str(" CASCADE");
        }
        sql
    }

    /// Generate SQLite load extension command.
    pub fn to_sqlite_load(&self) -> String {
        format!("SELECT load_extension('{}')", self.name)
    }
}

/// Builder for extensions.
#[derive(Debug, Clone)]
pub struct ExtensionBuilder {
    name: String,
    schema: Option<String>,
    version: Option<String>,
    cascade: bool,
}

impl ExtensionBuilder {
    /// Create a new builder.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            schema: None,
            version: None,
            cascade: false,
        }
    }

    /// Set the schema.
    pub fn schema(mut self, schema: impl Into<String>) -> Self {
        self.schema = Some(schema.into());
        self
    }

    /// Set the version.
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    /// Enable CASCADE.
    pub fn cascade(mut self) -> Self {
        self.cascade = true;
        self
    }

    /// Build the extension.
    pub fn build(self) -> Extension {
        Extension {
            name: self.name,
            schema: self.schema,
            version: self.version,
            cascade: self.cascade,
        }
    }
}

// ============================================================================
// Geospatial Types
// ============================================================================

/// A geographic point (longitude, latitude).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    /// Longitude (-180 to 180).
    pub longitude: f64,
    /// Latitude (-90 to 90).
    pub latitude: f64,
    /// Optional SRID (spatial reference ID).
    pub srid: Option<i32>,
}

impl Point {
    /// Create a new point.
    pub fn new(longitude: f64, latitude: f64) -> Self {
        Self {
            longitude,
            latitude,
            srid: None,
        }
    }

    /// Create with SRID.
    pub fn with_srid(longitude: f64, latitude: f64, srid: i32) -> Self {
        Self {
            longitude,
            latitude,
            srid: Some(srid),
        }
    }

    /// WGS84 SRID (standard GPS).
    pub fn wgs84(longitude: f64, latitude: f64) -> Self {
        Self::with_srid(longitude, latitude, 4326)
    }

    /// Generate PostGIS point.
    pub fn to_postgis(&self) -> String {
        if let Some(srid) = self.srid {
            format!(
                "ST_SetSRID(ST_MakePoint({}, {}), {})",
                self.longitude, self.latitude, srid
            )
        } else {
            format!("ST_MakePoint({}, {})", self.longitude, self.latitude)
        }
    }

    /// Generate MySQL spatial point.
    pub fn to_mysql(&self) -> String {
        if let Some(srid) = self.srid {
            format!(
                "ST_GeomFromText('POINT({} {})', {})",
                self.longitude, self.latitude, srid
            )
        } else {
            format!(
                "ST_GeomFromText('POINT({} {})')",
                self.longitude, self.latitude
            )
        }
    }

    /// Generate MSSQL geography point.
    pub fn to_mssql(&self) -> String {
        format!(
            "geography::Point({}, {}, {})",
            self.latitude,
            self.longitude,
            self.srid.unwrap_or(4326)
        )
    }

    /// Generate GeoJSON.
    pub fn to_geojson(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "Point",
            "coordinates": [self.longitude, self.latitude]
        })
    }

    /// Generate SQL for database type.
    pub fn to_sql(&self, db_type: DatabaseType) -> String {
        match db_type {
            DatabaseType::PostgreSQL => self.to_postgis(),
            DatabaseType::MySQL => self.to_mysql(),
            DatabaseType::MSSQL => self.to_mssql(),
            DatabaseType::SQLite => format!("MakePoint({}, {})", self.longitude, self.latitude),
        }
    }
}

/// A polygon (list of points forming a closed ring).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Polygon {
    /// Exterior ring coordinates.
    pub exterior: Vec<(f64, f64)>,
    /// Interior rings (holes).
    pub interiors: Vec<Vec<(f64, f64)>>,
    /// SRID.
    pub srid: Option<i32>,
}

impl Polygon {
    /// Create a new polygon from coordinates.
    pub fn new(exterior: Vec<(f64, f64)>) -> Self {
        Self {
            exterior,
            interiors: Vec::new(),
            srid: None,
        }
    }

    /// Add an interior ring (hole).
    pub fn with_hole(mut self, hole: Vec<(f64, f64)>) -> Self {
        self.interiors.push(hole);
        self
    }

    /// Set SRID.
    pub fn with_srid(mut self, srid: i32) -> Self {
        self.srid = Some(srid);
        self
    }

    /// Generate WKT (Well-Known Text).
    pub fn to_wkt(&self) -> String {
        let ext_coords: Vec<String> = self
            .exterior
            .iter()
            .map(|(x, y)| format!("{} {}", x, y))
            .collect();

        let mut wkt = format!("POLYGON(({})", ext_coords.join(", "));

        for interior in &self.interiors {
            let int_coords: Vec<String> = interior
                .iter()
                .map(|(x, y)| format!("{} {}", x, y))
                .collect();
            wkt.push_str(&format!(", ({})", int_coords.join(", ")));
        }

        wkt.push(')');
        wkt
    }

    /// Generate PostGIS polygon.
    pub fn to_postgis(&self) -> String {
        if let Some(srid) = self.srid {
            format!("ST_GeomFromText('{}', {})", self.to_wkt(), srid)
        } else {
            format!("ST_GeomFromText('{}')", self.to_wkt())
        }
    }

    /// Generate GeoJSON.
    pub fn to_geojson(&self) -> serde_json::Value {
        let mut coordinates = vec![
            self.exterior
                .iter()
                .map(|(x, y)| vec![*x, *y])
                .collect::<Vec<_>>(),
        ];

        for interior in &self.interiors {
            coordinates.push(interior.iter().map(|(x, y)| vec![*x, *y]).collect());
        }

        serde_json::json!({
            "type": "Polygon",
            "coordinates": coordinates
        })
    }
}

/// Geospatial operations.
pub mod geo {
    use super::*;

    /// Distance calculation.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub enum DistanceUnit {
        /// Meters.
        Meters,
        /// Kilometers.
        Kilometers,
        /// Miles.
        Miles,
        /// Feet.
        Feet,
    }

    impl DistanceUnit {
        /// Conversion factor from meters.
        pub fn from_meters(&self) -> f64 {
            match self {
                Self::Meters => 1.0,
                Self::Kilometers => 0.001,
                Self::Miles => 0.000621371,
                Self::Feet => 3.28084,
            }
        }
    }

    /// Generate distance SQL between two columns.
    pub fn distance_sql(col1: &str, col2: &str, db_type: DatabaseType) -> String {
        match db_type {
            DatabaseType::PostgreSQL => {
                format!("ST_Distance({}::geography, {}::geography)", col1, col2)
            }
            DatabaseType::MySQL => format!("ST_Distance_Sphere({}, {})", col1, col2),
            DatabaseType::MSSQL => format!("{}.STDistance({})", col1, col2),
            DatabaseType::SQLite => format!("Distance({}, {})", col1, col2),
        }
    }

    /// Generate distance from point SQL.
    pub fn distance_from_point_sql(col: &str, point: &Point, db_type: DatabaseType) -> String {
        let point_sql = point.to_sql(db_type);
        match db_type {
            DatabaseType::PostgreSQL => {
                format!("ST_Distance({}::geography, {}::geography)", col, point_sql)
            }
            DatabaseType::MySQL => format!("ST_Distance_Sphere({}, {})", col, point_sql),
            DatabaseType::MSSQL => format!("{}.STDistance({})", col, point_sql),
            DatabaseType::SQLite => format!("Distance({}, {})", col, point_sql),
        }
    }

    /// Generate "within distance" filter SQL.
    pub fn within_distance_sql(
        col: &str,
        point: &Point,
        distance_meters: f64,
        db_type: DatabaseType,
    ) -> String {
        let point_sql = point.to_sql(db_type);
        match db_type {
            DatabaseType::PostgreSQL => {
                format!(
                    "ST_DWithin({}::geography, {}::geography, {})",
                    col, point_sql, distance_meters
                )
            }
            DatabaseType::MySQL => {
                format!(
                    "ST_Distance_Sphere({}, {}) <= {}",
                    col, point_sql, distance_meters
                )
            }
            DatabaseType::MSSQL => {
                format!("{}.STDistance({}) <= {}", col, point_sql, distance_meters)
            }
            DatabaseType::SQLite => {
                format!("Distance({}, {}) <= {}", col, point_sql, distance_meters)
            }
        }
    }

    /// Generate "contains" filter SQL.
    pub fn contains_sql(geom_col: &str, point: &Point, db_type: DatabaseType) -> String {
        let point_sql = point.to_sql(db_type);
        match db_type {
            DatabaseType::PostgreSQL => format!("ST_Contains({}, {})", geom_col, point_sql),
            DatabaseType::MySQL => format!("ST_Contains({}, {})", geom_col, point_sql),
            DatabaseType::MSSQL => format!("{}.STContains({})", geom_col, point_sql),
            DatabaseType::SQLite => format!("Contains({}, {})", geom_col, point_sql),
        }
    }

    /// Generate bounding box filter SQL.
    pub fn bbox_sql(
        col: &str,
        min_lon: f64,
        min_lat: f64,
        max_lon: f64,
        max_lat: f64,
        db_type: DatabaseType,
    ) -> String {
        match db_type {
            DatabaseType::PostgreSQL => {
                format!(
                    "{} && ST_MakeEnvelope({}, {}, {}, {}, 4326)",
                    col, min_lon, min_lat, max_lon, max_lat
                )
            }
            DatabaseType::MySQL => {
                format!(
                    "MBRContains(ST_GeomFromText('POLYGON(({} {}, {} {}, {} {}, {} {}, {} {}))'), {})",
                    min_lon,
                    min_lat,
                    max_lon,
                    min_lat,
                    max_lon,
                    max_lat,
                    min_lon,
                    max_lat,
                    min_lon,
                    min_lat,
                    col
                )
            }
            _ => "1=1".to_string(),
        }
    }
}

// ============================================================================
// UUID Support
// ============================================================================

/// UUID generation helpers.
pub mod uuid {
    use super::*;

    /// Generate UUID v4 SQL.
    pub fn generate_v4(db_type: DatabaseType) -> String {
        match db_type {
            DatabaseType::PostgreSQL => "gen_random_uuid()".to_string(),
            DatabaseType::MySQL => "UUID()".to_string(),
            DatabaseType::MSSQL => "NEWID()".to_string(),
            DatabaseType::SQLite => {
                // SQLite needs custom function or hex/randomblob
                "lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-4' || \
                 substr(lower(hex(randomblob(2))), 2) || '-' || \
                 substr('89ab', abs(random()) % 4 + 1, 1) || \
                 substr(lower(hex(randomblob(2))), 2) || '-' || lower(hex(randomblob(6)))"
                    .to_string()
            }
        }
    }

    /// Generate UUID v7 SQL (PostgreSQL with uuid-ossp or pg_uuidv7).
    pub fn generate_v7_postgres() -> String {
        "uuid_generate_v7()".to_string()
    }

    /// Generate UUID from string SQL.
    pub fn from_string(value: &str, db_type: DatabaseType) -> String {
        match db_type {
            DatabaseType::PostgreSQL => format!("'{}'::uuid", value),
            DatabaseType::MySQL => format!("UUID_TO_BIN('{}')", value),
            DatabaseType::MSSQL => format!("CONVERT(UNIQUEIDENTIFIER, '{}')", value),
            DatabaseType::SQLite => format!("'{}'", value),
        }
    }

    /// Check if valid UUID SQL.
    pub fn is_valid_sql(col: &str, db_type: DatabaseType) -> String {
        match db_type {
            DatabaseType::PostgreSQL => format!(
                "{} ~ '^[0-9a-f]{{8}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{12}}$'",
                col
            ),
            DatabaseType::MySQL => format!(
                "{} REGEXP '^[0-9a-f]{{8}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{12}}$'",
                col
            ),
            _ => format!("LEN({}) = 36", col),
        }
    }
}

// ============================================================================
// Cryptography
// ============================================================================

/// Cryptographic functions.
pub mod crypto {
    use super::*;

    /// Hash algorithms.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub enum HashAlgorithm {
        Md5,
        Sha1,
        Sha256,
        Sha384,
        Sha512,
    }

    impl HashAlgorithm {
        /// PostgreSQL algorithm name.
        pub fn postgres_name(&self) -> &'static str {
            match self {
                Self::Md5 => "md5",
                Self::Sha1 => "sha1",
                Self::Sha256 => "sha256",
                Self::Sha384 => "sha384",
                Self::Sha512 => "sha512",
            }
        }
    }

    /// Generate hash SQL.
    pub fn hash_sql(expr: &str, algorithm: HashAlgorithm, db_type: DatabaseType) -> String {
        match db_type {
            DatabaseType::PostgreSQL => {
                if algorithm == HashAlgorithm::Md5 {
                    format!("md5({})", expr)
                } else {
                    format!(
                        "encode(digest({}, '{}'), 'hex')",
                        expr,
                        algorithm.postgres_name()
                    )
                }
            }
            DatabaseType::MySQL => match algorithm {
                HashAlgorithm::Md5 => format!("MD5({})", expr),
                HashAlgorithm::Sha1 => format!("SHA1({})", expr),
                HashAlgorithm::Sha256 => format!("SHA2({}, 256)", expr),
                HashAlgorithm::Sha384 => format!("SHA2({}, 384)", expr),
                HashAlgorithm::Sha512 => format!("SHA2({}, 512)", expr),
            },
            DatabaseType::MSSQL => {
                let algo = match algorithm {
                    HashAlgorithm::Md5 => "MD5",
                    HashAlgorithm::Sha1 => "SHA1",
                    HashAlgorithm::Sha256 => "SHA2_256",
                    HashAlgorithm::Sha384 => "SHA2_384",
                    HashAlgorithm::Sha512 => "SHA2_512",
                };
                format!("CONVERT(VARCHAR(MAX), HASHBYTES('{}', {}), 2)", algo, expr)
            }
            DatabaseType::SQLite => {
                // SQLite doesn't have built-in hashing
                format!("-- SQLite requires extension for hashing: {}", expr)
            }
        }
    }

    /// Generate bcrypt hash SQL (PostgreSQL with pgcrypto).
    pub fn bcrypt_hash_postgres(password: &str) -> String {
        format!("crypt('{}', gen_salt('bf'))", password)
    }

    /// Generate bcrypt verify SQL (PostgreSQL).
    pub fn bcrypt_verify_postgres(password: &str, hash_col: &str) -> String {
        format!("{} = crypt('{}', {})", hash_col, password, hash_col)
    }

    /// Generate random bytes SQL.
    pub fn random_bytes_sql(length: usize, db_type: DatabaseType) -> String {
        match db_type {
            DatabaseType::PostgreSQL => format!("gen_random_bytes({})", length),
            DatabaseType::MySQL => format!("RANDOM_BYTES({})", length),
            DatabaseType::MSSQL => format!("CRYPT_GEN_RANDOM({})", length),
            DatabaseType::SQLite => format!("randomblob({})", length),
        }
    }

    /// Generate AES encrypt SQL (PostgreSQL with pgcrypto).
    pub fn aes_encrypt_postgres(data: &str, key: &str) -> String {
        format!("pgp_sym_encrypt('{}', '{}')", data, key)
    }

    /// Generate AES decrypt SQL (PostgreSQL with pgcrypto).
    pub fn aes_decrypt_postgres(encrypted_col: &str, key: &str) -> String {
        format!("pgp_sym_decrypt({}, '{}')", encrypted_col, key)
    }
}

// ============================================================================
// Vector / Embeddings (pgvector, MongoDB Atlas Vector)
// ============================================================================

/// Vector operations for AI/ML embeddings.
pub mod vector {
    use super::*;

    /// A vector embedding.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct Vector {
        /// Vector dimensions.
        pub dimensions: Vec<f32>,
    }

    impl Vector {
        /// Create a new vector.
        pub fn new(dimensions: Vec<f32>) -> Self {
            Self { dimensions }
        }

        /// Create from slice.
        pub fn from_slice(slice: &[f32]) -> Self {
            Self {
                dimensions: slice.to_vec(),
            }
        }

        /// Get dimension count.
        pub fn len(&self) -> usize {
            self.dimensions.len()
        }

        /// Check if empty.
        pub fn is_empty(&self) -> bool {
            self.dimensions.is_empty()
        }

        /// Generate PostgreSQL pgvector literal.
        pub fn to_pgvector(&self) -> String {
            let nums: Vec<String> = self.dimensions.iter().map(|f| f.to_string()).collect();
            format!("'[{}]'::vector", nums.join(","))
        }

        /// Generate MongoDB array.
        pub fn to_mongodb(&self) -> serde_json::Value {
            serde_json::json!(self.dimensions)
        }
    }

    /// Vector similarity metrics.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub enum SimilarityMetric {
        /// Euclidean distance (L2).
        L2,
        /// Inner product.
        InnerProduct,
        /// Cosine similarity.
        Cosine,
    }

    impl SimilarityMetric {
        /// PostgreSQL operator.
        pub fn postgres_operator(&self) -> &'static str {
            match self {
                Self::L2 => "<->",
                Self::InnerProduct => "<#>",
                Self::Cosine => "<=>",
            }
        }

        /// MongoDB $vectorSearch similarity.
        pub fn mongodb_name(&self) -> &'static str {
            match self {
                Self::L2 => "euclidean",
                Self::InnerProduct => "dotProduct",
                Self::Cosine => "cosine",
            }
        }
    }

    /// Generate vector similarity search SQL (PostgreSQL pgvector).
    pub fn similarity_search_postgres(
        col: &str,
        query_vector: &Vector,
        metric: SimilarityMetric,
        limit: usize,
    ) -> String {
        format!(
            "SELECT *, {} {} {} AS distance FROM {{table}} ORDER BY distance LIMIT {}",
            col,
            metric.postgres_operator(),
            query_vector.to_pgvector(),
            limit
        )
    }

    /// Generate vector distance SQL.
    pub fn distance_sql(col: &str, query_vector: &Vector, metric: SimilarityMetric) -> String {
        format!(
            "{} {} {}",
            col,
            metric.postgres_operator(),
            query_vector.to_pgvector()
        )
    }

    /// Generate vector index SQL (PostgreSQL).
    pub fn create_index_postgres(
        index_name: &str,
        table: &str,
        column: &str,
        metric: SimilarityMetric,
        lists: Option<usize>,
    ) -> String {
        let ops = match metric {
            SimilarityMetric::L2 => "vector_l2_ops",
            SimilarityMetric::InnerProduct => "vector_ip_ops",
            SimilarityMetric::Cosine => "vector_cosine_ops",
        };

        let lists_clause = lists
            .map(|l| format!(" WITH (lists = {})", l))
            .unwrap_or_default();

        format!(
            "CREATE INDEX {} ON {} USING ivfflat ({} {}){}",
            index_name, table, column, ops, lists_clause
        )
    }

    /// Create HNSW index (PostgreSQL pgvector 0.5+).
    pub fn create_hnsw_index_postgres(
        index_name: &str,
        table: &str,
        column: &str,
        metric: SimilarityMetric,
        m: Option<usize>,
        ef_construction: Option<usize>,
    ) -> String {
        let ops = match metric {
            SimilarityMetric::L2 => "vector_l2_ops",
            SimilarityMetric::InnerProduct => "vector_ip_ops",
            SimilarityMetric::Cosine => "vector_cosine_ops",
        };

        let mut with_clauses = Vec::new();
        if let Some(m_val) = m {
            with_clauses.push(format!("m = {}", m_val));
        }
        if let Some(ef) = ef_construction {
            with_clauses.push(format!("ef_construction = {}", ef));
        }

        let with_clause = if with_clauses.is_empty() {
            String::new()
        } else {
            format!(" WITH ({})", with_clauses.join(", "))
        };

        format!(
            "CREATE INDEX {} ON {} USING hnsw ({} {}){}",
            index_name, table, column, ops, with_clause
        )
    }
}

/// MongoDB Atlas Vector Search support.
pub mod mongodb {
    use serde::{Deserialize, Serialize};
    use serde_json::Value as JsonValue;

    use super::vector::SimilarityMetric;

    /// MongoDB Atlas Vector Search query.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct VectorSearch {
        /// Index name.
        pub index: String,
        /// Path to vector field.
        pub path: String,
        /// Query vector.
        pub query_vector: Vec<f32>,
        /// Number of results.
        pub num_candidates: usize,
        /// Limit.
        pub limit: usize,
        /// Optional filter.
        pub filter: Option<JsonValue>,
    }

    impl VectorSearch {
        /// Create a new vector search.
        pub fn new(
            index: impl Into<String>,
            path: impl Into<String>,
            query: Vec<f32>,
        ) -> VectorSearchBuilder {
            VectorSearchBuilder::new(index, path, query)
        }

        /// Convert to $vectorSearch stage.
        pub fn to_stage(&self) -> JsonValue {
            let mut search = serde_json::json!({
                "index": self.index,
                "path": self.path,
                "queryVector": self.query_vector,
                "numCandidates": self.num_candidates,
                "limit": self.limit
            });

            if let Some(ref filter) = self.filter {
                search["filter"] = filter.clone();
            }

            serde_json::json!({ "$vectorSearch": search })
        }
    }

    /// Builder for vector search.
    #[derive(Debug, Clone)]
    pub struct VectorSearchBuilder {
        index: String,
        path: String,
        query_vector: Vec<f32>,
        num_candidates: usize,
        limit: usize,
        filter: Option<JsonValue>,
    }

    impl VectorSearchBuilder {
        /// Create a new builder.
        pub fn new(index: impl Into<String>, path: impl Into<String>, query: Vec<f32>) -> Self {
            Self {
                index: index.into(),
                path: path.into(),
                query_vector: query,
                num_candidates: 100,
                limit: 10,
                filter: None,
            }
        }

        /// Set number of candidates.
        pub fn num_candidates(mut self, n: usize) -> Self {
            self.num_candidates = n;
            self
        }

        /// Set limit.
        pub fn limit(mut self, n: usize) -> Self {
            self.limit = n;
            self
        }

        /// Add filter.
        pub fn filter(mut self, filter: JsonValue) -> Self {
            self.filter = Some(filter);
            self
        }

        /// Build the search.
        pub fn build(self) -> VectorSearch {
            VectorSearch {
                index: self.index,
                path: self.path,
                query_vector: self.query_vector,
                num_candidates: self.num_candidates,
                limit: self.limit,
                filter: self.filter,
            }
        }
    }

    /// MongoDB Atlas Search index definition for vectors.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct VectorIndex {
        /// Index name.
        pub name: String,
        /// Collection name.
        pub collection: String,
        /// Vector field definitions.
        pub fields: Vec<VectorField>,
    }

    /// Vector field definition.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct VectorField {
        /// Field path.
        pub path: String,
        /// Number of dimensions.
        pub dimensions: usize,
        /// Similarity metric.
        pub similarity: String,
    }

    impl VectorIndex {
        /// Create a new vector index definition.
        pub fn new(name: impl Into<String>, collection: impl Into<String>) -> VectorIndexBuilder {
            VectorIndexBuilder::new(name, collection)
        }

        /// Convert to index definition.
        pub fn to_definition(&self) -> JsonValue {
            let fields: Vec<JsonValue> = self
                .fields
                .iter()
                .map(|f| {
                    serde_json::json!({
                        "type": "vector",
                        "path": f.path,
                        "numDimensions": f.dimensions,
                        "similarity": f.similarity
                    })
                })
                .collect();

            serde_json::json!({
                "name": self.name,
                "type": "vectorSearch",
                "fields": fields
            })
        }
    }

    /// Builder for vector index.
    #[derive(Debug, Clone)]
    pub struct VectorIndexBuilder {
        name: String,
        collection: String,
        fields: Vec<VectorField>,
    }

    impl VectorIndexBuilder {
        /// Create a new builder.
        pub fn new(name: impl Into<String>, collection: impl Into<String>) -> Self {
            Self {
                name: name.into(),
                collection: collection.into(),
                fields: Vec::new(),
            }
        }

        /// Add a vector field.
        pub fn field(
            mut self,
            path: impl Into<String>,
            dimensions: usize,
            similarity: SimilarityMetric,
        ) -> Self {
            self.fields.push(VectorField {
                path: path.into(),
                dimensions,
                similarity: similarity.mongodb_name().to_string(),
            });
            self
        }

        /// Build the index.
        pub fn build(self) -> VectorIndex {
            VectorIndex {
                name: self.name,
                collection: self.collection,
                fields: self.fields,
            }
        }
    }

    /// Helper to create a vector search.
    pub fn vector_search(index: &str, path: &str, query: Vec<f32>) -> VectorSearchBuilder {
        VectorSearch::new(index, path, query)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extension_postgres() {
        let ext = Extension::new("postgis").schema("public").cascade().build();
        let sql = ext.to_postgres_create();

        assert!(sql.contains("CREATE EXTENSION IF NOT EXISTS \"postgis\""));
        assert!(sql.contains("SCHEMA public"));
        assert!(sql.contains("CASCADE"));
    }

    #[test]
    fn test_extension_drop() {
        let ext = Extension::postgis();
        let sql = ext.to_postgres_drop();

        assert!(sql.contains("DROP EXTENSION IF EXISTS \"postgis\""));
    }

    #[test]
    fn test_point_postgis() {
        let point = Point::wgs84(-122.4194, 37.7749);
        let sql = point.to_postgis();

        assert!(sql.contains("ST_SetSRID"));
        assert!(sql.contains("-122.4194"));
        assert!(sql.contains("37.7749"));
        assert!(sql.contains("4326"));
    }

    #[test]
    fn test_point_geojson() {
        let point = Point::new(-122.4194, 37.7749);
        let geojson = point.to_geojson();

        assert_eq!(geojson["type"], "Point");
        assert_eq!(geojson["coordinates"][0], -122.4194);
    }

    #[test]
    fn test_polygon_wkt() {
        let polygon = Polygon::new(vec![
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
            (0.0, 10.0),
            (0.0, 0.0),
        ]);

        let wkt = polygon.to_wkt();
        assert!(wkt.starts_with("POLYGON(("));
    }

    #[test]
    fn test_distance_sql() {
        let sql = geo::distance_sql("location", "target", DatabaseType::PostgreSQL);
        assert!(sql.contains("ST_Distance"));
    }

    #[test]
    fn test_within_distance() {
        let point = Point::wgs84(-122.4194, 37.7749);
        let sql = geo::within_distance_sql("location", &point, 1000.0, DatabaseType::PostgreSQL);

        assert!(sql.contains("ST_DWithin"));
        assert!(sql.contains("1000"));
    }

    #[test]
    fn test_uuid_generation() {
        let pg = uuid::generate_v4(DatabaseType::PostgreSQL);
        assert_eq!(pg, "gen_random_uuid()");

        let mysql = uuid::generate_v4(DatabaseType::MySQL);
        assert_eq!(mysql, "UUID()");

        let mssql = uuid::generate_v4(DatabaseType::MSSQL);
        assert_eq!(mssql, "NEWID()");
    }

    #[test]
    fn test_hash_sql() {
        let pg = crypto::hash_sql(
            "password",
            crypto::HashAlgorithm::Sha256,
            DatabaseType::PostgreSQL,
        );
        assert!(pg.contains("digest"));
        assert!(pg.contains("sha256"));

        let mysql = crypto::hash_sql(
            "password",
            crypto::HashAlgorithm::Sha256,
            DatabaseType::MySQL,
        );
        assert!(mysql.contains("SHA2"));
        assert!(mysql.contains("256"));
    }

    #[test]
    fn test_vector_pgvector() {
        let vec = vector::Vector::new(vec![0.1, 0.2, 0.3, 0.4]);
        let sql = vec.to_pgvector();

        assert!(sql.contains("'[0.1,0.2,0.3,0.4]'::vector"));
    }

    #[test]
    fn test_vector_index() {
        let sql = vector::create_index_postgres(
            "embeddings_idx",
            "documents",
            "embedding",
            vector::SimilarityMetric::Cosine,
            Some(100),
        );

        assert!(sql.contains("CREATE INDEX embeddings_idx"));
        assert!(sql.contains("USING ivfflat"));
        assert!(sql.contains("vector_cosine_ops"));
        assert!(sql.contains("lists = 100"));
    }

    #[test]
    fn test_hnsw_index() {
        let sql = vector::create_hnsw_index_postgres(
            "embeddings_hnsw",
            "documents",
            "embedding",
            vector::SimilarityMetric::L2,
            Some(16),
            Some(64),
        );

        assert!(sql.contains("USING hnsw"));
        assert!(sql.contains("m = 16"));
        assert!(sql.contains("ef_construction = 64"));
    }

    mod mongodb_tests {
        use super::super::mongodb::*;
        use super::super::vector::SimilarityMetric;

        #[test]
        fn test_vector_search() {
            let search = vector_search("vector_index", "embedding", vec![0.1, 0.2, 0.3])
                .num_candidates(200)
                .limit(20)
                .build();

            let stage = search.to_stage();
            assert!(stage["$vectorSearch"]["index"].is_string());
            assert_eq!(stage["$vectorSearch"]["numCandidates"], 200);
            assert_eq!(stage["$vectorSearch"]["limit"], 20);
        }

        #[test]
        fn test_vector_index_definition() {
            let index = VectorIndex::new("my_vector_index", "documents")
                .field("embedding", 1536, SimilarityMetric::Cosine)
                .build();

            let def = index.to_definition();
            assert_eq!(def["name"], "my_vector_index");
            assert_eq!(def["type"], "vectorSearch");
            assert!(def["fields"].is_array());
        }
    }
}
