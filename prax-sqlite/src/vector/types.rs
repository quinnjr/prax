//! Runtime vector types (Embedding, DoubleEmbedding, IntVector).

use crate::vector::error::{VectorError, VectorResult};
use crate::vector::metric::VectorElementType;

/// Format a slice as a JSON array string suitable for `vector_from_json(...)`.
///
/// Elements are written via the provided `append` callback so both numeric
/// primitives and trait-object elements can share the same loop.
fn format_json_array<T, F: FnMut(&T, &mut String)>(data: &[T], mut append: F) -> String {
    let mut s = String::from("[");
    for (i, v) in data.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        append(v, &mut s);
    }
    s.push(']');
    s
}

/// 32-bit float vector (element type float4) — the most common form.
#[derive(Debug, Clone, PartialEq)]
pub struct Embedding {
    data: Vec<f32>,
}

impl Embedding {
    /// Create a new embedding from a Vec<f32>. Rejects empty vectors.
    pub fn new(data: Vec<f32>) -> VectorResult<Self> {
        if data.is_empty() {
            return Err(VectorError::DimensionMismatch {
                expected: 1,
                got: 0,
            });
        }
        Ok(Self { data })
    }

    /// Borrow the underlying float32 data.
    pub fn as_slice(&self) -> &[f32] {
        &self.data
    }

    /// Number of dimensions.
    pub fn dimensions(&self) -> usize {
        self.data.len()
    }

    /// Element type for DDL (always Float4).
    pub fn element_type() -> VectorElementType {
        VectorElementType::Float4
    }

    /// Serialize to a JSON array string suitable for `vector_from_json`.
    pub fn to_json(&self) -> String {
        format_json_array(&self.data, |v, out| out.push_str(&v.to_string()))
    }
}

/// 64-bit float vector (element type float8) for high-precision workloads.
#[derive(Debug, Clone, PartialEq)]
pub struct DoubleEmbedding {
    data: Vec<f64>,
}

impl DoubleEmbedding {
    /// Create a new double embedding from a Vec<f64>. Rejects empty vectors.
    pub fn new(data: Vec<f64>) -> VectorResult<Self> {
        if data.is_empty() {
            return Err(VectorError::DimensionMismatch {
                expected: 1,
                got: 0,
            });
        }
        Ok(Self { data })
    }

    /// Borrow the underlying float64 data.
    pub fn as_slice(&self) -> &[f64] {
        &self.data
    }

    /// Number of dimensions.
    pub fn dimensions(&self) -> usize {
        self.data.len()
    }

    /// Element type for DDL (always Float8).
    pub fn element_type() -> VectorElementType {
        VectorElementType::Float8
    }

    /// Serialize to a JSON array string.
    pub fn to_json(&self) -> String {
        format_json_array(&self.data, |v, out| out.push_str(&v.to_string()))
    }
}

mod sealed {
    pub trait Sealed {}
}

/// Elements that can be stored in an [`IntVector`].
pub trait IntVectorElement: Copy + std::fmt::Debug + sealed::Sealed {
    /// Element type identifier.
    const TYPE: VectorElementType;
    /// Write the element as a JSON number into the given string.
    fn write_json(self, out: &mut String);
}

impl sealed::Sealed for i8 {}
impl IntVectorElement for i8 {
    const TYPE: VectorElementType = VectorElementType::Int1;
    fn write_json(self, out: &mut String) {
        out.push_str(&self.to_string());
    }
}

impl sealed::Sealed for i16 {}
impl IntVectorElement for i16 {
    const TYPE: VectorElementType = VectorElementType::Int2;
    fn write_json(self, out: &mut String) {
        out.push_str(&self.to_string());
    }
}

impl sealed::Sealed for i32 {}
impl IntVectorElement for i32 {
    const TYPE: VectorElementType = VectorElementType::Int4;
    fn write_json(self, out: &mut String) {
        out.push_str(&self.to_string());
    }
}

/// Integer vector (element type int1/int2/int4).
#[derive(Debug, Clone, PartialEq)]
pub struct IntVector<T: IntVectorElement> {
    data: Vec<T>,
}

impl<T: IntVectorElement> IntVector<T> {
    /// Create a new int vector. Rejects empty.
    pub fn new(data: Vec<T>) -> VectorResult<Self> {
        if data.is_empty() {
            return Err(VectorError::DimensionMismatch {
                expected: 1,
                got: 0,
            });
        }
        Ok(Self { data })
    }

    /// Borrow the underlying data.
    pub fn as_slice(&self) -> &[T] {
        &self.data
    }

    /// Number of dimensions.
    pub fn dimensions(&self) -> usize {
        self.data.len()
    }

    /// Element type for DDL.
    pub fn element_type() -> VectorElementType {
        T::TYPE
    }

    /// Serialize to a JSON array string.
    pub fn to_json(&self) -> String {
        format_json_array(&self.data, |v, out| v.write_json(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedding_new_rejects_empty() {
        let result = Embedding::new(Vec::new());
        assert!(result.is_err());
    }

    #[test]
    fn test_embedding_stores_dimensions() {
        let emb = Embedding::new(vec![0.1, 0.2, 0.3]).unwrap();
        assert_eq!(emb.dimensions(), 3);
        assert_eq!(emb.as_slice(), &[0.1, 0.2, 0.3]);
    }

    #[test]
    fn test_embedding_to_json() {
        let emb = Embedding::new(vec![1.0, -2.5, 3.0]).unwrap();
        assert_eq!(emb.to_json(), "[1,-2.5,3]");
    }

    #[test]
    fn test_embedding_element_type() {
        assert_eq!(Embedding::element_type(), VectorElementType::Float4);
    }

    #[test]
    fn test_double_embedding_element_type() {
        assert_eq!(DoubleEmbedding::element_type(), VectorElementType::Float8);
    }

    #[test]
    fn test_int_vector_i8() {
        let v = IntVector::<i8>::new(vec![1, -2, 3]).unwrap();
        assert_eq!(v.dimensions(), 3);
        assert_eq!(IntVector::<i8>::element_type(), VectorElementType::Int1);
        assert_eq!(v.to_json(), "[1,-2,3]");
    }

    #[test]
    fn test_int_vector_i16_i32_element_types() {
        assert_eq!(IntVector::<i16>::element_type(), VectorElementType::Int2);
        assert_eq!(IntVector::<i32>::element_type(), VectorElementType::Int4);
    }
}
