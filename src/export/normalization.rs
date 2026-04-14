//! Per-day normalization statistics for off-exchange features.
//!
//! Uses `WelfordAccumulator` from `hft_statistics` for numerically stable
//! per-feature mean/std computation. Produces JSON for export alongside NPY files.
//!
//! Source: docs/design/06_INTEGRATION_POINTS.md §5.3

use std::collections::HashSet;

use serde::Serialize;

use crate::config::FeatureConfig;
use crate::contract::{EPS, FEATURE_NAMES};
#[cfg(test)]
use crate::contract::TOTAL_FEATURES;
use crate::error::{ProcessorError, Result};
use crate::features::indices::{
    CATEGORICAL_INDICES, BBO_DYNAMICS_RANGE, CROSS_VENUE_RANGE, RETAIL_METRICS_RANGE,
    SIGNED_FLOW_RANGE, TRADE_SIZE_RANGE, VENUE_METRICS_RANGE, VPIN_RANGE,
};
use hft_statistics::statistics::WelfordAccumulator;

/// Per-feature normalization statistics.
#[derive(Debug, Clone, Serialize)]
pub struct FeatureStats {
    pub index: usize,
    pub name: String,
    pub mean: f64,
    pub std: f64,
    pub min: f64,
    pub max: f64,
    pub n_finite: u64,
    pub n_nan: u64,
    pub normalizable: bool,
}

/// Complete normalization result for JSON export.
#[derive(Debug, Clone, Serialize)]
pub struct NormalizationResult {
    pub strategy: String,
    pub day: String,
    pub sample_count: u64,
    pub features: Vec<FeatureStats>,
}

/// Computes per-feature normalization statistics across a day's bins.
///
/// Stats are computed from ALL post-warmup bins (broader than exported sequences)
/// for better mean/std estimates.
///
/// # Non-Normalizable Features
///
/// Two categories of features are excluded from normalization:
/// 1. **Categorical**: [29, 30, 32, 33] (bin_valid, bbo_valid, time_bucket, schema_version)
/// 2. **Disabled groups**: Feature indices from toggled-off groups (e.g., VPIN indices 18-19
///    when VPIN is disabled). These contain constant 0.0 and would create phantom signals
///    if z-score normalized (FIX #10).
#[derive(Debug, Clone)]
pub struct NormalizationComputer {
    accumulators: Vec<WelfordAccumulator>,
    n_features: usize,
    non_normalizable: HashSet<usize>,
    /// Total number of update() calls (bins fed).
    total_updates: u64,
}

impl NormalizationComputer {
    /// Create a new normalization computer.
    ///
    /// # Arguments
    ///
    /// * `n_features` — Number of features (must be TOTAL_FEATURES = 34)
    /// * `feature_config` — Used to determine which groups are disabled (FIX #10)
    pub fn new(n_features: usize, feature_config: &FeatureConfig) -> Self {
        let mut non_normalizable: HashSet<usize> =
            CATEGORICAL_INDICES.iter().copied().collect();

        // FIX #10: Mark disabled-group indices as non-normalizable
        if !feature_config.signed_flow {
            non_normalizable.extend(SIGNED_FLOW_RANGE);
        }
        if !feature_config.venue_metrics {
            non_normalizable.extend(VENUE_METRICS_RANGE);
        }
        if !feature_config.retail_metrics {
            non_normalizable.extend(RETAIL_METRICS_RANGE);
        }
        if !feature_config.bbo_dynamics {
            non_normalizable.extend(BBO_DYNAMICS_RANGE);
        }
        if !feature_config.vpin {
            non_normalizable.extend(VPIN_RANGE);
        }
        if !feature_config.trade_size {
            non_normalizable.extend(TRADE_SIZE_RANGE);
        }
        if !feature_config.cross_venue {
            non_normalizable.extend(CROSS_VENUE_RANGE);
        }

        Self {
            accumulators: vec![WelfordAccumulator::new(); n_features],
            n_features,
            non_normalizable,
            total_updates: 0,
        }
    }

    /// Update statistics with one bin's feature values.
    ///
    /// WelfordAccumulator silently skips NaN/Inf values (not counted).
    pub fn update(&mut self, feature_vec: &[f64]) {
        debug_assert_eq!(feature_vec.len(), self.n_features);
        for (i, &val) in feature_vec.iter().enumerate() {
            self.accumulators[i].update(val);
        }
        self.total_updates += 1;
    }

    /// Normalize a single feature value using z-score.
    ///
    /// - Non-normalizable features (categorical + disabled groups): pass through unchanged.
    /// - When `std < EPS`: `((value - mean) / EPS).clamp(-100.0, 100.0)` (FIX #16, #28).
    /// - Normal case: `(value - mean) / std`.
    pub fn normalize_value(&self, feature_idx: usize, value: f64) -> f64 {
        if self.non_normalizable.contains(&feature_idx) {
            return value;
        }
        let mean = self.accumulators[feature_idx].mean();
        let std = self.accumulators[feature_idx].std();
        if std < EPS {
            // FIX #16 + #28: avoid extreme values from near-zero variance
            ((value - mean) / EPS).clamp(-100.0, 100.0)
        } else {
            (value - mean) / std
        }
    }

    /// Normalize an entire feature vector.
    pub fn normalize_vec(&self, feature_vec: &[f64]) -> Vec<f64> {
        feature_vec
            .iter()
            .enumerate()
            .map(|(i, &val)| self.normalize_value(i, val))
            .collect()
    }

    /// Build the normalization result for JSON export.
    pub fn finalize(&self, day: &str) -> NormalizationResult {
        let sample_count = self.total_updates;

        let features: Vec<FeatureStats> = (0..self.n_features)
            .map(|i| {
                let acc = &self.accumulators[i];
                FeatureStats {
                    index: i,
                    name: FEATURE_NAMES[i].to_string(),
                    mean: acc.mean(),
                    std: acc.std(),
                    min: if acc.count() > 0 { acc.min() } else { 0.0 },
                    max: if acc.count() > 0 { acc.max() } else { 0.0 },
                    n_finite: acc.count(),
                    n_nan: sample_count.saturating_sub(acc.count()),
                    normalizable: !self.non_normalizable.contains(&i),
                }
            })
            .collect();

        NormalizationResult {
            strategy: "per_day_zscore".to_string(),
            day: day.to_string(),
            sample_count,
            features,
        }
    }

    /// Serialize normalization result to JSON string.
    pub fn to_json(&self, day: &str) -> Result<String> {
        let result = self.finalize(day);
        serde_json::to_string_pretty(&result)
            .map_err(|e| ProcessorError::export(format!("normalization JSON: {e}")))
    }

    /// Reset all accumulators for a new day.
    pub fn reset(&mut self) {
        for acc in &mut self.accumulators {
            acc.reset();
        }
        self.total_updates = 0;
    }

    /// Whether a feature index is normalizable.
    pub fn is_normalizable(&self, idx: usize) -> bool {
        !self.non_normalizable.contains(&idx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_normalizer() -> NormalizationComputer {
        NormalizationComputer::new(TOTAL_FEATURES, &FeatureConfig::default())
    }

    #[test]
    fn test_normalization_known_values() {
        let mut nc = default_normalizer();
        // Feed 4 samples: feature[0] = {1, 2, 3, 4}
        for val in [1.0, 2.0, 3.0, 4.0] {
            let mut fv = vec![0.0; TOTAL_FEATURES];
            fv[0] = val;
            nc.update(&fv);
        }
        let result = nc.finalize("2025-02-03");
        assert_eq!(result.sample_count, 4);
        // mean = 2.5, population std = sqrt(1.25) ≈ 1.118
        let stats = &result.features[0];
        assert!((stats.mean - 2.5).abs() < 1e-10);
        assert!((stats.std - 1.25_f64.sqrt()).abs() < 1e-10);
        assert_eq!(stats.min, 1.0);
        assert_eq!(stats.max, 4.0);
        assert!(stats.normalizable);
    }

    #[test]
    fn test_categorical_never_normalized() {
        let nc = default_normalizer();
        // Categorical indices: [29, 30, 32, 33]
        for &idx in &[29_usize, 30, 32, 33] {
            assert!(
                !nc.is_normalizable(idx),
                "Feature {} should be non-normalizable (categorical)",
                idx
            );
            // Pass-through: 42.0 → 42.0
            assert_eq!(nc.normalize_value(idx, 42.0), 42.0);
        }
    }

    #[test]
    fn test_disabled_feature_not_normalized() {
        // Default config has VPIN disabled → indices 18, 19 non-normalizable
        let nc = default_normalizer();
        assert!(!nc.is_normalizable(18), "trf_vpin should be non-normalizable (VPIN disabled)");
        assert!(!nc.is_normalizable(19), "lit_vpin should be non-normalizable (VPIN disabled)");
        // Pass-through: 0.0 → 0.0 (no phantom signal)
        assert_eq!(nc.normalize_value(18, 0.0), 0.0);
    }

    #[test]
    fn test_enabled_vpin_is_normalizable() {
        let config = FeatureConfig { vpin: true, ..Default::default() };
        let nc = NormalizationComputer::new(TOTAL_FEATURES, &config);
        assert!(nc.is_normalizable(18), "trf_vpin should be normalizable when VPIN enabled");
        assert!(nc.is_normalizable(19), "lit_vpin should be normalizable when VPIN enabled");
    }

    #[test]
    fn test_zero_std_constant_feature() {
        let mut nc = default_normalizer();
        // Feed constant value 5.0 for feature[0]
        for _ in 0..10 {
            let mut fv = vec![0.0; TOTAL_FEATURES];
            fv[0] = 5.0;
            nc.update(&fv);
        }
        // std = 0 < EPS, value = mean = 5.0
        // FIX #16: (5.0 - 5.0) / EPS = 0.0
        assert_eq!(nc.normalize_value(0, 5.0), 0.0, "Constant feature → 0.0");
    }

    #[test]
    fn test_zero_std_disabled_feature_no_phantom() {
        let nc = default_normalizer();
        // VPIN disabled: feature 18 is always 0.0, non-normalizable
        // Without FIX #10, this would normalize to 1.0 (phantom signal)
        assert_eq!(nc.normalize_value(18, 0.0), 0.0, "Disabled feature → 0.0 (no phantom)");
    }

    #[test]
    fn test_nan_skipped_by_welford() {
        let mut nc = default_normalizer();
        let mut fv = vec![0.0; TOTAL_FEATURES];
        fv[0] = 10.0;
        nc.update(&fv);
        fv[0] = f64::NAN;
        nc.update(&fv);
        fv[0] = 20.0;
        nc.update(&fv);
        // NaN should be skipped: mean = (10 + 20) / 2 = 15
        let result = nc.finalize("test");
        assert!((result.features[0].mean - 15.0).abs() < 1e-10);
        assert_eq!(result.features[0].n_finite, 2);
        assert_eq!(result.features[0].n_nan, 1); // 3 samples - 2 finite = 1 NaN
    }

    #[test]
    fn test_normalize_value_zscore() {
        let mut nc = default_normalizer();
        // Feed: [0, 10] → mean=5, std=5
        let mut fv1 = vec![0.0; TOTAL_FEATURES];
        fv1[0] = 0.0;
        nc.update(&fv1);
        let mut fv2 = vec![0.0; TOTAL_FEATURES];
        fv2[0] = 10.0;
        nc.update(&fv2);
        // z-score of 10.0: (10 - 5) / 5 = 1.0
        let z = nc.normalize_value(0, 10.0);
        assert!((z - 1.0).abs() < 1e-10, "z-score of max should be 1.0, got {}", z);
    }

    #[test]
    fn test_normalization_json_34_features() {
        let nc = default_normalizer();
        let result = nc.finalize("2025-02-03");
        assert_eq!(result.features.len(), TOTAL_FEATURES);
        for (i, stats) in result.features.iter().enumerate() {
            assert_eq!(stats.index, i);
            assert_eq!(stats.name, FEATURE_NAMES[i]);
        }
    }

    #[test]
    fn test_normalization_json_n_nan_field() {
        let mut nc = default_normalizer();
        let fv = vec![0.0; TOTAL_FEATURES];
        nc.update(&fv);
        let json = nc.to_json("test").unwrap();
        assert!(json.contains("\"n_nan\""), "JSON must contain n_nan field");
        assert!(json.contains("\"normalizable\""), "JSON must contain normalizable field");
    }

    #[test]
    fn test_reset() {
        let mut nc = default_normalizer();
        let fv = vec![42.0; TOTAL_FEATURES];
        nc.update(&fv);
        nc.reset();
        let result = nc.finalize("test");
        assert_eq!(result.sample_count, 0);
        assert_eq!(result.features[0].n_finite, 0);
    }

    #[test]
    fn test_zscore_clamping() {
        let mut nc = default_normalizer();
        // Feed constant=0 for feature[0] (std will be 0)
        for _ in 0..5 {
            let fv = vec![0.0; TOTAL_FEATURES];
            nc.update(&fv);
        }
        // Normalize a value far from mean when std < EPS
        // (1000.0 - 0.0) / EPS = 1e11, but clamped to 100.0
        let z = nc.normalize_value(0, 1000.0);
        assert_eq!(z, 100.0, "Should be clamped at 100.0");

        let z_neg = nc.normalize_value(0, -1000.0);
        assert_eq!(z_neg, -100.0, "Should be clamped at -100.0");
    }
}
