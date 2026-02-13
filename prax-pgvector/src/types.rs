//! Core vector types wrapping pgvector with Prax ORM integration.
//!
//! This module provides newtype wrappers around pgvector's types that integrate
//! seamlessly with prax-postgres for query parameter binding and row extraction.
//!
//! # Supported Types
//!
//! | Type | pgvector | Description |
//! |------|----------|-------------|
//! | [`Embedding`] | `vector` | Dense float32 vector |
//! | [`SparseEmbedding`] | `sparsevec` | Sparse vector with indices |
//! | [`BinaryVector`] | `bit` | Binary/boolean vector |
//! | [`HalfEmbedding`] | `halfvec` | Dense float16 vector (feature-gated) |

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::{VectorError, VectorResult};

// ============================================================================
// Embedding (dense float32 vector)
// ============================================================================

/// A dense vector embedding for use with pgvector.
///
/// This wraps [`pgvector::Vector`] and provides additional methods for
/// ORM integration, validation, and conversion.
///
/// # Examples
///
/// ```rust
/// use prax_pgvector::Embedding;
///
/// // From a Vec<f32>
/// let embedding = Embedding::new(vec![0.1, 0.2, 0.3]);
///
/// // From a slice
/// let embedding = Embedding::from_slice(&[0.1, 0.2, 0.3]);
///
/// // Access dimensions
/// assert_eq!(embedding.len(), 3);
/// assert_eq!(embedding.as_slice()[0], 0.1);
/// ```
#[derive(Clone, PartialEq)]
pub struct Embedding {
    inner: pgvector::Vector,
}

impl Embedding {
    /// Create a new embedding from a vector of floats.
    pub fn new(dimensions: Vec<f32>) -> Self {
        Self {
            inner: pgvector::Vector::from(dimensions),
        }
    }

    /// Create an embedding from a float slice.
    pub fn from_slice(slice: &[f32]) -> Self {
        Self {
            inner: pgvector::Vector::from(slice.to_vec()),
        }
    }

    /// Create a zero vector with the given number of dimensions.
    pub fn zeros(dimensions: usize) -> Self {
        Self::new(vec![0.0; dimensions])
    }

    /// Create a validated embedding, ensuring it's non-empty.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError::EmptyVector`] if the input is empty.
    pub fn try_new(dimensions: Vec<f32>) -> VectorResult<Self> {
        if dimensions.is_empty() {
            return Err(VectorError::EmptyVector);
        }
        Ok(Self::new(dimensions))
    }

    /// Validate that this embedding has the expected number of dimensions.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError::DimensionMismatch`] if the dimensions don't match.
    pub fn validate_dimensions(&self, expected: usize) -> VectorResult<()> {
        let actual = self.len();
        if actual != expected {
            return Err(VectorError::dimension_mismatch(expected, actual));
        }
        Ok(())
    }

    /// Get the number of dimensions.
    pub fn len(&self) -> usize {
        self.as_slice().len()
    }

    /// Check if the vector is empty.
    pub fn is_empty(&self) -> bool {
        self.as_slice().is_empty()
    }

    /// Get the dimensions as a slice.
    pub fn as_slice(&self) -> &[f32] {
        self.inner.as_slice()
    }

    /// Convert to a `Vec<f32>`.
    pub fn to_vec(&self) -> Vec<f32> {
        self.as_slice().to_vec()
    }

    /// Get the inner pgvector type.
    pub fn into_inner(self) -> pgvector::Vector {
        self.inner
    }

    /// Get a reference to the inner pgvector type.
    pub fn inner(&self) -> &pgvector::Vector {
        &self.inner
    }

    /// Compute the L2 (Euclidean) norm of this vector.
    pub fn l2_norm(&self) -> f32 {
        self.as_slice().iter().map(|x| x * x).sum::<f32>().sqrt()
    }

    /// Normalize this vector to unit length (L2 normalization).
    ///
    /// Returns `None` if the vector is a zero vector.
    pub fn normalize(&self) -> Option<Self> {
        let norm = self.l2_norm();
        if norm == 0.0 {
            return None;
        }
        let normalized: Vec<f32> = self.as_slice().iter().map(|x| x / norm).collect();
        Some(Self::new(normalized))
    }

    /// Compute the dot product with another embedding.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError::DimensionMismatch`] if the dimensions differ.
    pub fn dot_product(&self, other: &Self) -> VectorResult<f32> {
        if self.len() != other.len() {
            return Err(VectorError::dimension_mismatch(self.len(), other.len()));
        }
        Ok(self
            .as_slice()
            .iter()
            .zip(other.as_slice().iter())
            .map(|(a, b)| a * b)
            .sum())
    }

    /// Compute the cosine similarity with another embedding.
    ///
    /// Returns a value between -1.0 and 1.0.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError::DimensionMismatch`] if the dimensions differ.
    pub fn cosine_similarity(&self, other: &Self) -> VectorResult<f32> {
        let dot = self.dot_product(other)?;
        let norm_a = self.l2_norm();
        let norm_b = other.l2_norm();

        if norm_a == 0.0 || norm_b == 0.0 {
            return Ok(0.0);
        }

        Ok(dot / (norm_a * norm_b))
    }

    /// Compute the Euclidean (L2) distance to another embedding.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError::DimensionMismatch`] if the dimensions differ.
    pub fn l2_distance(&self, other: &Self) -> VectorResult<f32> {
        if self.len() != other.len() {
            return Err(VectorError::dimension_mismatch(self.len(), other.len()));
        }
        Ok(self
            .as_slice()
            .iter()
            .zip(other.as_slice().iter())
            .map(|(a, b)| (a - b) * (a - b))
            .sum::<f32>()
            .sqrt())
    }

    /// Generate the PostgreSQL literal representation.
    ///
    /// This produces a string like `'[0.1,0.2,0.3]'::vector`.
    pub fn to_sql_literal(&self) -> String {
        let nums: Vec<String> = self.as_slice().iter().map(|f| f.to_string()).collect();
        format!("'[{}]'::vector", nums.join(","))
    }

    /// Generate the PostgreSQL literal with explicit dimension.
    ///
    /// This produces a string like `'[0.1,0.2,0.3]'::vector(3)`.
    pub fn to_sql_literal_typed(&self) -> String {
        let nums: Vec<String> = self.as_slice().iter().map(|f| f.to_string()).collect();
        format!("'[{}]'::vector({})", nums.join(","), self.len())
    }
}

impl fmt::Debug for Embedding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Embedding({:?})", self.as_slice())
    }
}

impl fmt::Display for Embedding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let nums: Vec<String> = self.as_slice().iter().map(|x| format!("{x:.4}")).collect();
        write!(f, "[{}]", nums.join(", "))
    }
}

impl From<Vec<f32>> for Embedding {
    fn from(v: Vec<f32>) -> Self {
        Self::new(v)
    }
}

impl From<&[f32]> for Embedding {
    fn from(s: &[f32]) -> Self {
        Self::from_slice(s)
    }
}

impl From<pgvector::Vector> for Embedding {
    fn from(v: pgvector::Vector) -> Self {
        Self { inner: v }
    }
}

impl From<Embedding> for pgvector::Vector {
    fn from(e: Embedding) -> Self {
        e.inner
    }
}

impl From<Embedding> for Vec<f32> {
    fn from(e: Embedding) -> Self {
        e.to_vec()
    }
}

impl Serialize for Embedding {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.as_slice().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Embedding {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let v = Vec::<f32>::deserialize(deserializer)?;
        Ok(Self::new(v))
    }
}

// ============================================================================
// SparseEmbedding (sparse vector)
// ============================================================================

/// A sparse vector embedding for use with pgvector's `sparsevec` type.
///
/// Sparse vectors are efficient for high-dimensional data where most values are zero,
/// common in text embeddings, bag-of-words representations, and learned sparse retrievers.
///
/// # Examples
///
/// ```rust
/// use prax_pgvector::SparseEmbedding;
///
/// // From a dense vector (zeros are stripped)
/// let sparse = SparseEmbedding::from_dense(vec![1.0, 0.0, 2.0, 0.0, 3.0]);
///
/// // From indices and values
/// let sparse = SparseEmbedding::from_parts(&[0, 2, 4], &[1.0, 2.0, 3.0], 5).unwrap();
/// ```
#[derive(Clone, PartialEq)]
pub struct SparseEmbedding {
    inner: pgvector::SparseVector,
}

impl SparseEmbedding {
    /// Create a sparse embedding from a dense vector.
    ///
    /// Zero values are automatically removed.
    pub fn from_dense(values: Vec<f32>) -> Self {
        Self {
            inner: pgvector::SparseVector::from_dense(&values),
        }
    }

    /// Create a sparse embedding from indices, values, and total dimensions.
    ///
    /// # Errors
    ///
    /// Returns an error if indices and values have different lengths,
    /// or if any index is out of bounds.
    pub fn from_parts(indices: &[i32], values: &[f32], dimensions: usize) -> VectorResult<Self> {
        if indices.len() != values.len() {
            return Err(VectorError::InvalidDimensions(format!(
                "indices length ({}) must match values length ({})",
                indices.len(),
                values.len()
            )));
        }

        for &idx in indices {
            if idx < 0 || idx as usize >= dimensions {
                return Err(VectorError::InvalidDimensions(format!(
                    "index {idx} out of bounds for {dimensions} dimensions"
                )));
            }
        }

        // Build via dense vector (pgvector::SparseVector doesn't expose parts constructor)
        let mut dense = vec![0.0f32; dimensions];
        for (&idx, &val) in indices.iter().zip(values.iter()) {
            dense[idx as usize] = val;
        }
        Ok(Self::from_dense(dense))
    }

    /// Get the total number of dimensions.
    pub fn dimensions(&self) -> i32 {
        self.inner.dimensions()
    }

    /// Get the indices of non-zero elements.
    pub fn indices(&self) -> &[i32] {
        self.inner.indices()
    }

    /// Get the values of non-zero elements.
    pub fn values(&self) -> &[f32] {
        self.inner.values()
    }

    /// Get the number of non-zero elements.
    pub fn nnz(&self) -> usize {
        self.inner.indices().len()
    }

    /// Convert to a dense vector.
    pub fn to_dense(&self) -> Vec<f32> {
        self.inner.to_vec()
    }

    /// Get the inner pgvector type.
    pub fn into_inner(self) -> pgvector::SparseVector {
        self.inner
    }

    /// Get a reference to the inner pgvector type.
    pub fn inner(&self) -> &pgvector::SparseVector {
        &self.inner
    }
}

impl fmt::Debug for SparseEmbedding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "SparseEmbedding(dims={}, nnz={})",
            self.dimensions(),
            self.nnz()
        )
    }
}

impl fmt::Display for SparseEmbedding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sparse[dims={}, nnz={}]", self.dimensions(), self.nnz())
    }
}

impl From<pgvector::SparseVector> for SparseEmbedding {
    fn from(v: pgvector::SparseVector) -> Self {
        Self { inner: v }
    }
}

impl From<SparseEmbedding> for pgvector::SparseVector {
    fn from(e: SparseEmbedding) -> Self {
        e.inner
    }
}

impl Serialize for SparseEmbedding {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Serialize as a dense array for JSON compatibility
        self.to_dense().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SparseEmbedding {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let v = Vec::<f32>::deserialize(deserializer)?;
        Ok(Self::from_dense(v))
    }
}

// ============================================================================
// BinaryVector (bit vector)
// ============================================================================

/// A binary vector for use with pgvector's `bit` type.
///
/// Binary vectors are useful for binary embeddings (e.g., from Cohere)
/// and Hamming distance comparisons.
///
/// # Examples
///
/// ```rust
/// use prax_pgvector::BinaryVector;
///
/// let bv = BinaryVector::from_bools(&[true, false, true, true]);
/// assert_eq!(bv.len(), 4);
/// ```
#[derive(Clone, PartialEq)]
pub struct BinaryVector {
    inner: pgvector::Bit,
}

impl BinaryVector {
    /// Create a binary vector from a slice of booleans.
    pub fn from_bools(bits: &[bool]) -> Self {
        Self {
            inner: pgvector::Bit::new(bits),
        }
    }

    /// Create a binary vector from a byte slice.
    ///
    /// Each byte represents 8 bits, MSB first.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self {
            inner: pgvector::Bit::from_bytes(bytes),
        }
    }

    /// Get the number of bits.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Check if the vector is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.len() == 0
    }

    /// Get the underlying bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.inner.as_bytes()
    }

    /// Get the inner pgvector type.
    pub fn into_inner(self) -> pgvector::Bit {
        self.inner
    }

    /// Get a reference to the inner pgvector type.
    pub fn inner(&self) -> &pgvector::Bit {
        &self.inner
    }
}

impl fmt::Debug for BinaryVector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BinaryVector(len={})", self.len())
    }
}

impl fmt::Display for BinaryVector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "bit[{}]", self.len())
    }
}

impl From<pgvector::Bit> for BinaryVector {
    fn from(v: pgvector::Bit) -> Self {
        Self { inner: v }
    }
}

impl From<BinaryVector> for pgvector::Bit {
    fn from(e: BinaryVector) -> Self {
        e.inner
    }
}

// ============================================================================
// HalfEmbedding (float16 vector, feature-gated)
// ============================================================================

/// A half-precision (float16) vector embedding for use with pgvector's `halfvec` type.
///
/// This type is only available when the `halfvec` feature is enabled.
/// Half vectors use less memory and bandwidth while maintaining reasonable precision
/// for many embedding use cases.
///
/// # Examples
///
/// ```rust,ignore
/// use prax_pgvector::HalfEmbedding;
///
/// let embedding = HalfEmbedding::from_f32_slice(&[0.1, 0.2, 0.3]);
/// assert_eq!(embedding.len(), 3);
/// ```
#[cfg(feature = "halfvec")]
#[derive(Clone, PartialEq)]
pub struct HalfEmbedding {
    inner: pgvector::HalfVector,
}

#[cfg(feature = "halfvec")]
impl HalfEmbedding {
    /// Create a half embedding from a slice of f32 values.
    ///
    /// Values are converted from f32 to f16.
    pub fn from_f32_slice(values: &[f32]) -> Self {
        Self {
            inner: pgvector::HalfVector::from_f32_slice(values),
        }
    }

    /// Get the number of dimensions.
    pub fn len(&self) -> usize {
        self.as_slice().len()
    }

    /// Check if the vector is empty.
    pub fn is_empty(&self) -> bool {
        self.as_slice().is_empty()
    }

    /// Get the dimensions as a slice of f16 values.
    pub fn as_slice(&self) -> &[half::f16] {
        self.inner.as_slice()
    }

    /// Get the inner pgvector type.
    pub fn into_inner(self) -> pgvector::HalfVector {
        self.inner
    }

    /// Get a reference to the inner pgvector type.
    pub fn inner(&self) -> &pgvector::HalfVector {
        &self.inner
    }
}

#[cfg(feature = "halfvec")]
impl fmt::Debug for HalfEmbedding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HalfEmbedding(len={})", self.len())
    }
}

#[cfg(feature = "halfvec")]
impl fmt::Display for HalfEmbedding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "halfvec[{}]", self.len())
    }
}

#[cfg(feature = "halfvec")]
impl From<pgvector::HalfVector> for HalfEmbedding {
    fn from(v: pgvector::HalfVector) -> Self {
        Self { inner: v }
    }
}

#[cfg(feature = "halfvec")]
impl From<HalfEmbedding> for pgvector::HalfVector {
    fn from(e: HalfEmbedding) -> Self {
        e.inner
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedding_new() {
        let embedding = Embedding::new(vec![0.1, 0.2, 0.3]);
        assert_eq!(embedding.len(), 3);
        assert!(!embedding.is_empty());
    }

    #[test]
    fn test_embedding_from_slice() {
        let embedding = Embedding::from_slice(&[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(embedding.len(), 4);
        assert_eq!(embedding.as_slice()[0], 1.0);
    }

    #[test]
    fn test_embedding_zeros() {
        let embedding = Embedding::zeros(5);
        assert_eq!(embedding.len(), 5);
        assert!(embedding.as_slice().iter().all(|&x| x == 0.0));
    }

    #[test]
    fn test_embedding_try_new_empty() {
        let result = Embedding::try_new(vec![]);
        assert!(result.is_err());
    }

    #[test]
    fn test_embedding_try_new_valid() {
        let result = Embedding::try_new(vec![1.0, 2.0]);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 2);
    }

    #[test]
    fn test_embedding_validate_dimensions() {
        let embedding = Embedding::new(vec![1.0, 2.0, 3.0]);
        assert!(embedding.validate_dimensions(3).is_ok());
        assert!(embedding.validate_dimensions(5).is_err());
    }

    #[test]
    fn test_embedding_l2_norm() {
        let embedding = Embedding::new(vec![3.0, 4.0]);
        let norm = embedding.l2_norm();
        assert!((norm - 5.0).abs() < 1e-6);
    }

    #[test]
    fn test_embedding_normalize() {
        let embedding = Embedding::new(vec![3.0, 4.0]);
        let normalized = embedding.normalize().unwrap();
        let norm = normalized.l2_norm();
        assert!((norm - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_embedding_normalize_zero() {
        let embedding = Embedding::zeros(3);
        assert!(embedding.normalize().is_none());
    }

    #[test]
    fn test_embedding_dot_product() {
        let a = Embedding::new(vec![1.0, 2.0, 3.0]);
        let b = Embedding::new(vec![4.0, 5.0, 6.0]);
        let dot = a.dot_product(&b).unwrap();
        assert!((dot - 32.0).abs() < 1e-6);
    }

    #[test]
    fn test_embedding_dot_product_dimension_mismatch() {
        let a = Embedding::new(vec![1.0, 2.0]);
        let b = Embedding::new(vec![1.0, 2.0, 3.0]);
        assert!(a.dot_product(&b).is_err());
    }

    #[test]
    fn test_embedding_cosine_similarity() {
        let a = Embedding::new(vec![1.0, 0.0]);
        let b = Embedding::new(vec![1.0, 0.0]);
        let sim = a.cosine_similarity(&b).unwrap();
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_embedding_cosine_similarity_orthogonal() {
        let a = Embedding::new(vec![1.0, 0.0]);
        let b = Embedding::new(vec![0.0, 1.0]);
        let sim = a.cosine_similarity(&b).unwrap();
        assert!(sim.abs() < 1e-6);
    }

    #[test]
    fn test_embedding_l2_distance() {
        let a = Embedding::new(vec![0.0, 0.0]);
        let b = Embedding::new(vec![3.0, 4.0]);
        let dist = a.l2_distance(&b).unwrap();
        assert!((dist - 5.0).abs() < 1e-6);
    }

    #[test]
    fn test_embedding_to_sql_literal() {
        let embedding = Embedding::new(vec![0.1, 0.2, 0.3]);
        let sql = embedding.to_sql_literal();
        assert!(sql.contains("::vector"));
        assert!(sql.contains("0.1"));
    }

    #[test]
    fn test_embedding_to_sql_literal_typed() {
        let embedding = Embedding::new(vec![0.1, 0.2, 0.3]);
        let sql = embedding.to_sql_literal_typed();
        assert!(sql.contains("::vector(3)"));
    }

    #[test]
    fn test_embedding_display() {
        let embedding = Embedding::new(vec![0.1, 0.2]);
        let display = format!("{embedding}");
        assert!(display.contains("0.1000"));
    }

    #[test]
    fn test_embedding_from_vec() {
        let embedding: Embedding = vec![1.0, 2.0, 3.0].into();
        assert_eq!(embedding.len(), 3);
    }

    #[test]
    fn test_embedding_to_vec() {
        let embedding = Embedding::new(vec![1.0, 2.0, 3.0]);
        let v: Vec<f32> = embedding.into();
        assert_eq!(v, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn test_embedding_serde_roundtrip() {
        let embedding = Embedding::new(vec![0.1, 0.2, 0.3]);
        let json = serde_json::to_string(&embedding).unwrap();
        let deserialized: Embedding = serde_json::from_str(&json).unwrap();
        assert_eq!(embedding, deserialized);
    }

    #[test]
    fn test_embedding_pgvector_roundtrip() {
        let embedding = Embedding::new(vec![1.0, 2.0, 3.0]);
        let pgvec: pgvector::Vector = embedding.clone().into();
        let back: Embedding = pgvec.into();
        assert_eq!(embedding, back);
    }

    #[test]
    fn test_sparse_embedding_from_dense() {
        let sparse = SparseEmbedding::from_dense(vec![1.0, 0.0, 2.0, 0.0, 3.0]);
        assert_eq!(sparse.dimensions(), 5);
        assert_eq!(sparse.nnz(), 3);
    }

    #[test]
    fn test_sparse_embedding_from_parts() {
        let sparse = SparseEmbedding::from_parts(&[0, 2, 4], &[1.0, 2.0, 3.0], 5).unwrap();
        assert_eq!(sparse.dimensions(), 5);
        assert_eq!(sparse.nnz(), 3);
    }

    #[test]
    fn test_sparse_embedding_from_parts_mismatched() {
        let result = SparseEmbedding::from_parts(&[0, 2], &[1.0], 5);
        assert!(result.is_err());
    }

    #[test]
    fn test_sparse_embedding_from_parts_out_of_bounds() {
        let result = SparseEmbedding::from_parts(&[10], &[1.0], 5);
        assert!(result.is_err());
    }

    #[test]
    fn test_sparse_embedding_to_dense() {
        let sparse = SparseEmbedding::from_dense(vec![1.0, 0.0, 2.0]);
        let dense = sparse.to_dense();
        assert_eq!(dense, vec![1.0, 0.0, 2.0]);
    }

    #[test]
    fn test_sparse_embedding_serde_roundtrip() {
        let sparse = SparseEmbedding::from_dense(vec![1.0, 0.0, 2.0]);
        let json = serde_json::to_string(&sparse).unwrap();
        let deserialized: SparseEmbedding = serde_json::from_str(&json).unwrap();
        assert_eq!(sparse.to_dense(), deserialized.to_dense());
    }

    #[test]
    fn test_binary_vector_from_bools() {
        let bv = BinaryVector::from_bools(&[true, false, true, true]);
        assert_eq!(bv.len(), 4);
        assert!(!bv.is_empty());
    }

    #[test]
    fn test_binary_vector_from_bytes() {
        let bv = BinaryVector::from_bytes(&[0b10110000]);
        assert_eq!(bv.len(), 8);
    }

    #[test]
    fn test_binary_vector_display() {
        let bv = BinaryVector::from_bools(&[true, false, true]);
        assert!(format!("{bv}").contains("3"));
    }

    #[test]
    fn test_binary_vector_pgvector_roundtrip() {
        let bv = BinaryVector::from_bools(&[true, false, true, false]);
        let inner: pgvector::Bit = bv.clone().into();
        let back: BinaryVector = inner.into();
        assert_eq!(bv, back);
    }
}
