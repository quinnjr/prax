//! Distance metrics and vector element types.

/// Distance metric used for similarity search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistanceMetric {
    /// Cosine distance.
    Cosine,
    /// L2 (Euclidean) distance.
    L2,
    /// Negative inner product (smaller = more similar).
    InnerProduct,
}

impl DistanceMetric {
    /// Lowercase string used in sqlite-vector-rs DDL and SQL.
    pub fn as_sql(&self) -> &'static str {
        match self {
            DistanceMetric::Cosine => "cosine",
            DistanceMetric::L2 => "l2",
            DistanceMetric::InnerProduct => "inner",
        }
    }
}

/// Vector element type supported by sqlite-vector-rs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorElementType {
    /// 16-bit float (half precision).
    Float2,
    /// 32-bit float.
    Float4,
    /// 64-bit float.
    Float8,
    /// 8-bit signed integer.
    Int1,
    /// 16-bit signed integer.
    Int2,
    /// 32-bit signed integer.
    Int4,
}

impl VectorElementType {
    /// Lowercase identifier used in sqlite-vector-rs DDL.
    pub fn as_sql(&self) -> &'static str {
        match self {
            VectorElementType::Float2 => "float2",
            VectorElementType::Float4 => "float4",
            VectorElementType::Float8 => "float8",
            VectorElementType::Int1 => "int1",
            VectorElementType::Int2 => "int2",
            VectorElementType::Int4 => "int4",
        }
    }
}

/// Vector index kinds supported by sqlite-vector-rs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorIndexKind {
    /// HNSW (Hierarchical Navigable Small World) graph index.
    Hnsw,
}

impl VectorIndexKind {
    /// Lowercase identifier used in sqlite-vector-rs DDL.
    pub fn as_sql(&self) -> &'static str {
        match self {
            VectorIndexKind::Hnsw => "hnsw",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_distance_metric_as_sql() {
        assert_eq!(DistanceMetric::Cosine.as_sql(), "cosine");
        assert_eq!(DistanceMetric::L2.as_sql(), "l2");
        assert_eq!(DistanceMetric::InnerProduct.as_sql(), "inner");
    }

    #[test]
    fn test_element_type_as_sql() {
        assert_eq!(VectorElementType::Float2.as_sql(), "float2");
        assert_eq!(VectorElementType::Float4.as_sql(), "float4");
        assert_eq!(VectorElementType::Float8.as_sql(), "float8");
        assert_eq!(VectorElementType::Int1.as_sql(), "int1");
        assert_eq!(VectorElementType::Int2.as_sql(), "int2");
        assert_eq!(VectorElementType::Int4.as_sql(), "int4");
    }

    #[test]
    fn test_index_kind_as_sql() {
        assert_eq!(VectorIndexKind::Hnsw.as_sql(), "hnsw");
    }
}
