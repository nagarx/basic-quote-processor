//! Point-to-point return label computation.
//!
//! Formula: `point_return(t, H) = (mid[t+H] - mid[t]) / mid[t] * 10000`  [bps]
//!
//! E8 showed smoothed-average labels produce DA=48.3% on point returns
//! (below random). This pipeline uses point-to-point returns ONLY.
//!
//! Source: docs/design/04_FEATURE_SPECIFICATION.md §6
//! Reference: Barber et al. (2024), §4.3

use crate::contract::EPS;
use crate::error::{ProcessorError, Result};

/// Computes point-return labels at multiple horizons.
pub struct LabelComputer {
    horizons: Vec<usize>,
}

/// Result of label computation for a day's bins.
#[derive(Debug, Clone)]
pub struct LabelResult {
    /// Labels `[N_bins, H]` in basis points.
    /// NaN where the label cannot be computed (end-of-day truncation,
    /// zero mid-price at either t or t+h).
    pub labels: Vec<Vec<f64>>,

    /// `true` if ALL horizons have finite labels for this bin.
    /// Used by the pipeline orchestrator to filter sequence ending indices.
    pub valid_mask: Vec<bool>,

    /// Number of bins with at least one NaN horizon.
    pub n_truncated: usize,
}

impl LabelComputer {
    /// Create a new label computer with the given horizons.
    ///
    /// Horizons must be non-empty, sorted ascending, all > 0.
    /// These invariants are enforced by `LabelConfig::validate()`.
    pub fn new(horizons: &[usize]) -> Result<Self> {
        if horizons.is_empty() {
            return Err(ProcessorError::label("horizons must be non-empty"));
        }
        Ok(Self {
            horizons: horizons.to_vec(),
        })
    }

    /// Maximum horizon value.
    pub fn max_horizon(&self) -> usize {
        *self.horizons.last().unwrap_or(&0)
    }

    /// Number of horizons.
    pub fn n_horizons(&self) -> usize {
        self.horizons.len()
    }

    /// Compute point-return labels for all bins.
    ///
    /// Formula: `point_return(t, H) = (mid[t+H] - mid[t]) / mid[t] * 10000` [bps]
    ///
    /// Guards (FIX #5):
    /// - `mid[t].abs() < EPS` → NaN for all horizons (zero denominator)
    /// - `mid[t+h].abs() < EPS` → NaN (zero numerator = invalid BBO)
    /// - `t + h >= n_bins` → NaN (end-of-day truncation)
    /// - `!result.is_finite()` → NaN (defensive)
    ///
    /// # Arguments
    ///
    /// * `mid_prices` — BBO midpoints at END of each post-warmup bin (f64 USD)
    ///
    /// # Returns
    ///
    /// `LabelResult` with labels, valid_mask, and truncation count.
    pub fn compute_labels(&self, mid_prices: &[f64]) -> LabelResult {
        let n_bins = mid_prices.len();
        let n_horizons = self.horizons.len();
        let mut labels = Vec::with_capacity(n_bins);
        let mut valid_mask = Vec::with_capacity(n_bins);
        let mut n_truncated = 0usize;

        for t in 0..n_bins {
            let mut row = Vec::with_capacity(n_horizons);
            let mut all_valid = true;

            if mid_prices[t].abs() < EPS || !mid_prices[t].is_finite() {
                // Base price invalid — all horizons NaN
                row.resize(n_horizons, f64::NAN);
                all_valid = false;
            } else {
                for &h in &self.horizons {
                    if t + h < n_bins
                        && mid_prices[t + h].abs() > EPS
                        && mid_prices[t + h].is_finite()
                    {
                        let ret =
                            (mid_prices[t + h] - mid_prices[t]) / mid_prices[t] * 10_000.0;
                        if ret.is_finite() {
                            row.push(ret);
                        } else {
                            row.push(f64::NAN);
                            all_valid = false;
                        }
                    } else {
                        row.push(f64::NAN);
                        all_valid = false;
                    }
                }
            }

            if !all_valid {
                n_truncated += 1;
            }
            labels.push(row);
            valid_mask.push(all_valid);
        }

        LabelResult {
            labels,
            valid_mask,
            n_truncated,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_label_formula_hand_calculated() {
        // mid = [100, 101, 102], H=1
        // ret(0,1) = (101-100)/100 * 10000 = 100 bps
        // ret(1,1) = (102-101)/101 * 10000 ≈ 99.0099 bps
        let lc = LabelComputer::new(&[1]).unwrap();
        let result = lc.compute_labels(&[100.0, 101.0, 102.0]);
        assert_eq!(result.labels.len(), 3);
        assert!((result.labels[0][0] - 100.0).abs() < 0.001);
        assert!((result.labels[1][0] - 99.0099).abs() < 0.01);
        // Last bin: t+1 = 3 >= 3 → NaN
        assert!(result.labels[2][0].is_nan());
    }

    #[test]
    fn test_label_formula_negative_return() {
        // mid = [100, 99], H=1
        // ret(0,1) = (99-100)/100 * 10000 = -100 bps
        let lc = LabelComputer::new(&[1]).unwrap();
        let result = lc.compute_labels(&[100.0, 99.0]);
        assert!((result.labels[0][0] - (-100.0)).abs() < 0.001);
    }

    #[test]
    fn test_label_nan_at_end_of_day() {
        let lc = LabelComputer::new(&[1, 5]).unwrap();
        let prices: Vec<f64> = (0..10).map(|i| 100.0 + i as f64).collect();
        let result = lc.compute_labels(&prices);

        // Last 5 bins should have invalid mask (can't compute H=5)
        for t in 5..10 {
            assert!(!result.valid_mask[t], "Bin {} should be invalid for H=5", t);
        }
        // First 5 bins should be valid
        for t in 0..5 {
            assert!(result.valid_mask[t], "Bin {} should be valid", t);
        }
    }

    #[test]
    fn test_valid_mask_excludes_nan() {
        let lc = LabelComputer::new(&[3]).unwrap();
        let prices = vec![100.0, 101.0, 102.0, 103.0, 104.0];
        let result = lc.compute_labels(&prices);

        // Bins 0, 1 can compute H=3 (t+3 < 5)
        assert!(result.valid_mask[0]);
        assert!(result.valid_mask[1]);
        // Bins 2, 3, 4 cannot (t+3 >= 5)
        assert!(!result.valid_mask[2]);
        assert!(!result.valid_mask[3]);
        assert!(!result.valid_mask[4]);
    }

    #[test]
    fn test_truncation_count() {
        let lc = LabelComputer::new(&[2]).unwrap();
        let prices = vec![100.0; 10];
        let result = lc.compute_labels(&prices);
        // Last 2 bins are truncated
        assert_eq!(result.n_truncated, 2);
    }

    #[test]
    fn test_label_multiple_horizons() {
        let lc = LabelComputer::new(&[1, 2, 3, 5, 10, 20, 30, 60]).unwrap();
        let prices: Vec<f64> = (0..100).map(|i| 100.0 + 0.01 * i as f64).collect();
        let result = lc.compute_labels(&prices);
        assert_eq!(result.labels[0].len(), 8, "8 horizons per bin");

        // First bin should have valid labels for H=1..30 but not H=60 (100 bins - 60 = 40)
        assert!(result.valid_mask[0]);
        // Bin 39 should still be valid (t+60 = 99 < 100)
        assert!(result.valid_mask[39]);
        // Bin 40 should be invalid (t+60 = 100 >= 100)
        assert!(!result.valid_mask[40]);
    }

    #[test]
    fn test_label_empty_input() {
        let lc = LabelComputer::new(&[1]).unwrap();
        let result = lc.compute_labels(&[]);
        assert!(result.labels.is_empty());
        assert!(result.valid_mask.is_empty());
        assert_eq!(result.n_truncated, 0);
    }

    #[test]
    fn test_label_single_bin() {
        let lc = LabelComputer::new(&[1]).unwrap();
        let result = lc.compute_labels(&[100.0]);
        assert_eq!(result.labels.len(), 1);
        assert!(result.labels[0][0].is_nan(), "Single bin cannot look ahead");
        assert!(!result.valid_mask[0]);
    }

    #[test]
    fn test_label_mid_price_zero_guard() {
        let lc = LabelComputer::new(&[1]).unwrap();
        // mid[0] is near-zero → NaN
        let result = lc.compute_labels(&[0.0, 100.0, 101.0]);
        assert!(result.labels[0][0].is_nan());
        assert!(!result.valid_mask[0]);
        // mid[1] is valid
        assert!(result.labels[1][0].is_finite());
    }

    #[test]
    fn test_label_future_mid_price_zero_guard() {
        let lc = LabelComputer::new(&[1]).unwrap();
        // mid[1] is near-zero → label at t=0 is NaN (FIX #5)
        let result = lc.compute_labels(&[100.0, 0.0, 101.0]);
        assert!(result.labels[0][0].is_nan(), "Future mid=0 should produce NaN");
        assert!(!result.valid_mask[0]);
    }

    #[test]
    fn test_label_mid_price_inf() {
        let lc = LabelComputer::new(&[1]).unwrap();
        let result = lc.compute_labels(&[f64::INFINITY, 100.0]);
        assert!(result.labels[0][0].is_nan());
        assert!(!result.valid_mask[0]);
    }

    #[test]
    fn test_label_mid_price_nan() {
        let lc = LabelComputer::new(&[1]).unwrap();
        let result = lc.compute_labels(&[f64::NAN, 100.0]);
        assert!(result.labels[0][0].is_nan());
        assert!(!result.valid_mask[0]);
    }

    #[test]
    fn test_label_sign_convention() {
        // Positive return = price increase = bullish (Rule 10)
        let lc = LabelComputer::new(&[1]).unwrap();
        let result = lc.compute_labels(&[100.0, 105.0]);
        assert!(result.labels[0][0] > 0.0, "Price increase → positive label (bullish)");

        let result = lc.compute_labels(&[100.0, 95.0]);
        assert!(result.labels[0][0] < 0.0, "Price decrease → negative label (bearish)");
    }

    #[test]
    fn test_golden_50_midprices() {
        // Frozen golden test: 50 mid-prices with known pattern
        let prices: Vec<f64> = (0..50).map(|i| 130.0 + 0.01 * (i as f64)).collect();
        let lc = LabelComputer::new(&[1, 5, 10]).unwrap();
        let result = lc.compute_labels(&prices);

        // Label at t=0, H=1: (130.01 - 130.00) / 130.00 * 10000
        let expected_h1 = (130.01 - 130.00) / 130.00 * 10_000.0;
        assert!(
            (result.labels[0][0] - expected_h1).abs() < 1e-8,
            "Golden H=1: expected {}, got {}",
            expected_h1, result.labels[0][0]
        );

        // Label at t=0, H=5: (130.05 - 130.00) / 130.00 * 10000
        let expected_h5 = (130.05 - 130.00) / 130.00 * 10_000.0;
        assert!(
            (result.labels[0][1] - expected_h5).abs() < 1e-8,
            "Golden H=5: expected {}, got {}",
            expected_h5, result.labels[0][1]
        );

        // Label at t=0, H=10: (130.10 - 130.00) / 130.00 * 10000
        let expected_h10 = (130.10 - 130.00) / 130.00 * 10_000.0;
        assert!(
            (result.labels[0][2] - expected_h10).abs() < 1e-8,
            "Golden H=10: expected {}, got {}",
            expected_h10, result.labels[0][2]
        );

        // Valid bins: 0..40 (50 - 10 = 40 valid for all horizons)
        assert_eq!(
            result.valid_mask.iter().filter(|&&v| v).count(),
            40,
            "40 bins should have all horizons valid"
        );
        assert_eq!(result.n_truncated, 10);
    }

    #[test]
    fn test_empty_horizons_rejected() {
        assert!(LabelComputer::new(&[]).is_err());
    }
}
