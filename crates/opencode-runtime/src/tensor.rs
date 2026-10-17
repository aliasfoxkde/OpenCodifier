//! Validated dense f32 tensors crossing the backend boundary.
//!
//! The runtime layer's only data shape is a row-major dense `f32` tensor.
//! Keeping the wire type this small is deliberate (DECISIONS.md D7): the
//! decision model consumes *logits*, and everything above this layer —
//! softmax, calibration, confidence — happens in Rust, never inside the
//! backend, so no numerical policy is delegated to an opaque library.

/// Hard upper bound on tensor elements (16 Mi values = 64 MiB of `f32`).
///
/// A defensive ceiling, not a tuning knob: it turns a pathological shape
/// (or a hostile caller) into a typed error instead of an allocation
/// failure. Real decision-model tensors are orders of magnitude smaller.
pub const MAX_TENSOR_ELEMENTS: usize = 1 << 24;

use crate::error::RuntimeError;

/// A dense, row-major, fully validated `f32` tensor.
///
/// Construction is the only way in: shape and data are checked against
/// each other, every dimension must be positive, the element count is
/// capped at [`MAX_TENSOR_ELEMENTS`], and every value must be finite.
/// Deserialization is deliberately not implemented — tensors originate
/// from local callers, and a hostile payload must be validated by
/// [`DenseTensor::new`] like any other input.
#[derive(Debug, Clone, PartialEq)]
pub struct DenseTensor {
    shape: Vec<usize>,
    data: Vec<f32>,
}

impl DenseTensor {
    /// Validates and constructs a tensor.
    ///
    /// Invariants: at least one dimension, every dimension positive,
    /// `data.len() == shape.product()`, at most
    /// [`MAX_TENSOR_ELEMENTS`] elements, all values finite.
    pub fn new(shape: Vec<usize>, data: Vec<f32>) -> Result<Self, RuntimeError> {
        if shape.is_empty() {
            return Err(RuntimeError::InvalidTensor { reason: "shape is empty".into() });
        }
        let mut expected = 1usize;
        for (index, dim) in shape.iter().enumerate() {
            if *dim == 0 {
                return Err(RuntimeError::InvalidTensor {
                    reason: format!("dimension {index} is zero"),
                });
            }
            expected = expected.saturating_mul(*dim);
            if expected > MAX_TENSOR_ELEMENTS {
                return Err(RuntimeError::InvalidTensor {
                    reason: format!(
                        "shape {shape:?} implies {expected} elements, above the {MAX_TENSOR_ELEMENTS} limit"
                    ),
                });
            }
        }
        if data.len() != expected {
            return Err(RuntimeError::InvalidTensor {
                reason: format!(
                    "shape {shape:?} implies {expected} elements, data carries {}",
                    data.len()
                ),
            });
        }
        if let Some(position) = data.iter().position(|value| !value.is_finite()) {
            return Err(RuntimeError::NonFiniteValue {
                where_: format!("tensor data at flat index {position}"),
            });
        }
        Ok(Self { shape, data })
    }

    /// The tensor shape, outermost dimension first.
    pub fn shape(&self) -> &[usize] {
        &self.shape
    }

    /// The flat row-major data.
    pub fn data(&self) -> &[f32] {
        &self.data
    }

    /// The number of rows (the outermost dimension).
    pub fn rows(&self) -> usize {
        self.shape[0]
    }

    /// A row of a rank-2 tensor, or `None` when the tensor is not rank 2
    /// or `row` is out of range.
    pub fn row(&self, row: usize) -> Option<&[f32]> {
        if self.shape.len() != 2 || row >= self.shape[0] {
            return None;
        }
        let width = self.shape[1];
        Some(&self.data[row * width..(row + 1) * width])
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;

    #[test]
    fn accepts_and_exposes_a_valid_rank2_tensor() {
        let tensor = DenseTensor::new(vec![2, 3], vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0]).unwrap();
        assert_eq!(tensor.shape(), &[2, 3]);
        assert_eq!(tensor.rows(), 2);
        assert_eq!(tensor.data(), &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
        assert_eq!(tensor.row(0).unwrap(), &[0.0, 1.0, 2.0]);
        assert_eq!(tensor.row(1).unwrap(), &[3.0, 4.0, 5.0]);
        assert_eq!(tensor.row(2), None, "out-of-range row");
    }

    #[test]
    fn rank1_tensors_have_no_rows_view() {
        let tensor = DenseTensor::new(vec![4], vec![0.0, 1.0, 2.0, 3.0]).unwrap();
        assert_eq!(tensor.row(0), None, "rank 1 has no rank-2 row view");
        assert_eq!(tensor.rows(), 4);
    }

    #[test]
    fn rejects_empty_zero_and_mismatched_shapes() {
        let empty = DenseTensor::new(Vec::new(), Vec::new()).unwrap_err();
        assert_eq!(empty.code(), "runtime.invalid_tensor");
        assert!(empty.to_string().contains("shape is empty"), "{empty}");

        let zero = DenseTensor::new(vec![2, 0], Vec::new()).unwrap_err();
        assert!(zero.to_string().contains("dimension 1 is zero"), "{zero}");

        let mismatch = DenseTensor::new(vec![2, 2], vec![0.0, 1.0, 2.0]).unwrap_err();
        assert!(mismatch.to_string().contains("implies 4 elements, data carries 3"), "{mismatch}");
    }

    #[test]
    fn rejects_shapes_above_the_element_ceiling() {
        let huge = DenseTensor::new(vec![MAX_TENSOR_ELEMENTS + 1], Vec::new()).unwrap_err();
        assert!(huge.to_string().contains("above the"), "{huge}");
        // Saturating multiplication must not panic on extreme shapes.
        let extreme = DenseTensor::new(vec![usize::MAX, usize::MAX], Vec::new()).unwrap_err();
        assert!(extreme.to_string().contains("above the"), "{extreme}");
    }

    #[test]
    fn rejects_non_finite_values_at_a_reported_position() {
        let nan = DenseTensor::new(vec![2], vec![1.0, f32::NAN]).unwrap_err();
        assert_eq!(nan.code(), "runtime.non_finite");
        assert!(nan.to_string().contains("flat index 1"), "{nan}");
        let infinite = DenseTensor::new(vec![1], vec![f32::INFINITY]).unwrap_err();
        assert!(infinite.to_string().contains("flat index 0"), "{infinite}");
    }
}
