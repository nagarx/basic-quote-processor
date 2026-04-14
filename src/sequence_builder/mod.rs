//! Sequence building for off-exchange feature vectors.
//!
//! Provides the `FeatureVec` type alias (Arc<Vec<f64>>) for zero-copy sharing
//! between overlapping sliding windows, and `build_all_from_slice()` for batch
//! sequence construction from a complete day's feature vectors.
//!
//! Phase 4 uses batch-only construction. Streaming sequence building
//! (push/try_build pattern) is deferred to Phase 5 if needed.
//!
//! Source: docs/design/06_INTEGRATION_POINTS.md §1.3

use std::sync::Arc;

#[cfg(test)]
use crate::contract::TOTAL_FEATURES;

/// Feature vector for one time bin.
///
/// `Arc` enables zero-copy sharing between overlapping sliding windows.
/// The underlying `Vec<f64>` has exactly `TOTAL_FEATURES` (34) elements.
///
/// Matches the MBO pipeline's `FeatureVec` type alias at
/// `feature-extractor-MBO-LOB/src/sequence_builder/builder.rs`.
pub type FeatureVec = Arc<Vec<f64>>;

/// Build all sequences from a complete slice of feature vectors.
///
/// Batch-only for Phase 4 (streaming deferred to Phase 5).
/// This function serves as a testable reference implementation for the
/// simpler case (contiguous valid range, no mask). The pipeline orchestrator
/// (`pipeline.rs`) builds sequences directly using `valid_mask` filtering
/// for the more complex case.
///
/// # Arguments
///
/// * `bins` — Ordered feature vectors (post-warmup, may include label-truncated bins)
/// * `window_size` — Number of bins per sequence (sliding window length)
/// * `stride` — Bins to skip between consecutive sequences
///
/// # Returns
///
/// Vec of sequences, each containing `window_size` Arc-cloned FeatureVecs.
/// Sequence count: `max(0, (bins.len() - window_size) / stride + 1)`
///
/// # Alignment
///
/// The ending index of sequence `i` is `i * stride + window_size - 1`.
/// Labels are computed at this ending index.
pub fn build_all_from_slice(
    bins: &[FeatureVec],
    window_size: usize,
    stride: usize,
) -> Vec<Vec<FeatureVec>> {
    if bins.is_empty() || window_size == 0 || stride == 0 || bins.len() < window_size {
        return vec![];
    }
    let n_sequences = (bins.len() - window_size) / stride + 1;
    let mut sequences = Vec::with_capacity(n_sequences);
    for i in 0..n_sequences {
        let start = i * stride;
        let end = start + window_size;
        let seq: Vec<FeatureVec> = bins[start..end].iter().map(Arc::clone).collect();
        sequences.push(seq);
    }
    sequences
}

/// Compute the ending index for sequence `seq_idx`.
///
/// Labels and forward prices are computed at this bin index.
/// `ending_index = seq_idx * stride + window_size - 1`
#[inline]
pub fn ending_index(seq_idx: usize, stride: usize, window_size: usize) -> usize {
    seq_idx * stride + window_size - 1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a dummy FeatureVec filled with a recognizable value.
    fn make_fv(id: f64) -> FeatureVec {
        let mut v = vec![0.0; TOTAL_FEATURES];
        v[0] = id; // first element identifies the bin
        Arc::new(v)
    }

    fn make_bins(n: usize) -> Vec<FeatureVec> {
        (0..n).map(|i| make_fv(i as f64)).collect()
    }

    #[test]
    fn test_build_empty_input() {
        let bins: Vec<FeatureVec> = vec![];
        let seqs = build_all_from_slice(&bins, 5, 1);
        assert!(seqs.is_empty());
    }

    #[test]
    fn test_build_exact_window() {
        let bins = make_bins(5);
        let seqs = build_all_from_slice(&bins, 5, 1);
        assert_eq!(seqs.len(), 1, "Exactly window_size bins → 1 sequence");
        assert_eq!(seqs[0].len(), 5);
    }

    #[test]
    fn test_build_stride_1() {
        let bins = make_bins(10);
        let seqs = build_all_from_slice(&bins, 3, 1);
        assert_eq!(seqs.len(), 8, "(10 - 3) / 1 + 1 = 8");
        // First sequence starts at bin 0
        assert_eq!(seqs[0][0][0], 0.0);
        // Last sequence starts at bin 7
        assert_eq!(seqs[7][0][0], 7.0);
    }

    #[test]
    fn test_build_stride_5() {
        let bins = make_bins(25);
        let seqs = build_all_from_slice(&bins, 5, 5);
        assert_eq!(seqs.len(), 5, "(25 - 5) / 5 + 1 = 5");
        // Verify starts: 0, 5, 10, 15, 20
        for (i, seq) in seqs.iter().enumerate() {
            assert_eq!(seq[0][0], (i * 5) as f64, "Sequence {} starts at bin {}", i, i * 5);
        }
    }

    #[test]
    fn test_build_insufficient_bins() {
        let bins = make_bins(3);
        let seqs = build_all_from_slice(&bins, 5, 1);
        assert!(seqs.is_empty(), "3 bins < window_size 5 → empty");
    }

    #[test]
    fn test_sequence_count_formula() {
        // Formula: (bins - window) / stride + 1
        for (n_bins, window, stride, expected) in [
            (100, 20, 1, 81),
            (100, 20, 5, 17),
            (387, 20, 1, 368),
            (327, 20, 1, 308),  // typical day: 387 bins - 60 label truncation = 327
            (20, 20, 1, 1),
            (19, 20, 1, 0),     // insufficient
        ] {
            let bins = make_bins(n_bins);
            let seqs = build_all_from_slice(&bins, window, stride);
            assert_eq!(
                seqs.len(), expected,
                "bins={}, window={}, stride={}: expected {}, got {}",
                n_bins, window, stride, expected, seqs.len()
            );
        }
    }

    #[test]
    fn test_ending_index_alignment() {
        let bins = make_bins(30);
        let seqs = build_all_from_slice(&bins, 10, 1);
        for i in 0..seqs.len() {
            let end_idx = ending_index(i, 1, 10);
            assert_eq!(end_idx, i + 9);
            // The last element of the sequence should be the bin at end_idx
            assert_eq!(seqs[i][9][0], end_idx as f64);
        }
    }

    #[test]
    fn test_zero_copy_arc_sharing() {
        let bins = make_bins(5);
        let seqs = build_all_from_slice(&bins, 3, 1);
        // seqs[0] = [bin0, bin1, bin2]
        // seqs[1] = [bin1, bin2, bin3]
        // bin1 should be the same Arc in both sequences
        assert!(
            Arc::ptr_eq(&seqs[0][1], &seqs[1][0]),
            "Overlapping bins should share the same Arc pointer"
        );
        assert!(
            Arc::ptr_eq(&seqs[0][2], &seqs[1][1]),
            "Overlapping bins should share the same Arc pointer"
        );
    }

    #[test]
    fn test_deterministic_output() {
        let bins = make_bins(20);
        let seqs1 = build_all_from_slice(&bins, 5, 1);
        let seqs2 = build_all_from_slice(&bins, 5, 1);
        assert_eq!(seqs1.len(), seqs2.len());
        for (s1, s2) in seqs1.iter().zip(seqs2.iter()) {
            for (f1, f2) in s1.iter().zip(s2.iter()) {
                assert_eq!(f1.as_slice(), f2.as_slice());
            }
        }
    }

    #[test]
    fn test_feature_vec_length_preserved() {
        let bins = make_bins(10);
        let seqs = build_all_from_slice(&bins, 3, 1);
        for seq in &seqs {
            for fv in seq {
                assert_eq!(
                    fv.len(), TOTAL_FEATURES,
                    "Every FeatureVec must have {} elements", TOTAL_FEATURES
                );
            }
        }
    }

    #[test]
    fn test_window_zero_stride_zero() {
        let bins = make_bins(10);
        assert!(build_all_from_slice(&bins, 0, 1).is_empty());
        assert!(build_all_from_slice(&bins, 5, 0).is_empty());
    }
}
