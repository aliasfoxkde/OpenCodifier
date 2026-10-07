//! Shape-agreement refusals of the two graph transports' validators.
//!
//! `kai::validate_shapes` and `julia::validate_shapes` are the last gate
//! before a tensor set reaches a graph. A disagreeing shape is the
//! "wrong base" class of failure that marker readouts turn into
//! confidently nonsense, so each refusal is a typed error naming the
//! offending input. These tests pin the two refusal arms the crate's own
//! unit tests cannot reach, and the exact message each one reports.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

use opencodifier_runtime::{DenseTensor, julia, kai};

#[test]
fn kai_option_positions_must_be_rank_two() {
    let ids = DenseTensor::new(vec![2, 5], vec![0.0; 10]).unwrap();
    let mask = DenseTensor::new(vec![2, 5], vec![0.0; 10]).unwrap();
    let answer = DenseTensor::new(vec![2], vec![0.0, 0.0]).unwrap();
    // A rank-1 option lane is refused before any pointer is read: a flat
    // lane has no row-per-option geometry to check the batch against.
    let options = DenseTensor::new(vec![4], vec![0.0; 4]).unwrap();

    let error = kai::validate_shapes(&ids, &mask, &answer, &options).unwrap_err();
    assert_eq!(error.code(), "runtime.invalid_tensor");
    assert!(error.to_string().contains("`option_pos` must be rank 2, got [4]"), "{error}");
}

#[test]
fn julia_marker_positions_must_be_rank_two() {
    let ids = DenseTensor::new(vec![1, 5], vec![0.0; 5]).unwrap();
    let mask = DenseTensor::new(vec![1, 5], vec![0.0; 5]).unwrap();
    let pos = DenseTensor::new(vec![3], vec![0.0; 3]).unwrap();
    let flag = DenseTensor::new(vec![1, 3], vec![0.0; 3]).unwrap();
    let kind = DenseTensor::new(vec![1], vec![0.0]).unwrap();

    let error = julia::validate_shapes(&ids, &mask, &pos, &flag, &kind).unwrap_err();
    assert_eq!(error.code(), "runtime.invalid_tensor");
    assert!(error.to_string().contains("`marker_pos` must be rank 2, got [3]"), "{error}");
}

#[test]
fn julia_marker_rows_must_match_the_input_batch() {
    // `marker_mask` is checked before the batch, so a row-count mismatch
    // needs a marker set that agrees with itself in full but not with
    // the prompt: batch 2 of prompts against batch 3 of markers.
    let ids = DenseTensor::new(vec![2, 5], vec![0.0; 10]).unwrap();
    let mask = DenseTensor::new(vec![2, 5], vec![0.0; 10]).unwrap();
    let pos = DenseTensor::new(vec![3, 4], vec![0.0; 12]).unwrap();
    let flag = DenseTensor::new(vec![3, 4], vec![0.0; 12]).unwrap();
    let kind = DenseTensor::new(vec![2], vec![0.0, 0.0]).unwrap();

    let error = julia::validate_shapes(&ids, &mask, &pos, &flag, &kind).unwrap_err();
    assert!(
        error.to_string().contains("`marker_pos` batch 3 must equal `input_ids` batch 2"),
        "{error}"
    );
}
