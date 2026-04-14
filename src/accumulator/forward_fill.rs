//! Explicit forward-fill state for state-type features across empty bins.
//!
//! When a bin has zero TRF trades, state-type features are forward-filled
//! from the most recent non-empty bin. This preserves persistent market state
//! information rather than producing misleading zeros.
//!
//! # 3-Level Empty Bin Policy
//!
//! ```text
//! Level 1: trf_trades > 0         → compute all features fresh (no forward-fill)
//! Level 2: trf_trades == 0, BBO > 0 → forward-fill TRF state, keep live BBO
//! Level 3: trf_trades == 0, BBO == 0 → forward-fill ALL state (incl BBO)
//! ```
//!
//! # Exclusions
//!
//! - `time_since_burst` (25): Always recomputed from persistent burst tracker,
//!   NOT forward-filled (it increments over time).
//! - `session_progress` (31): Always computed from clock.
//! - Flow features: Always zero on empty bins (no activity).
//! - Safety gates, context: Always computed.
//!
//! Source: docs/design/04_FEATURE_SPECIFICATION.md §8
//!         docs/design/03_DATA_FLOW.md §5

use crate::features::indices;

/// Forward-fill slot assignments.
/// Maps each forward-fill feature index to an internal array slot.
const SLOT_DARK_SHARE: usize = 0;
const SLOT_SUBPENNY_INTENSITY: usize = 1;
const SLOT_ODD_LOT_RATIO: usize = 2;
const SLOT_RETAIL_VOLUME_FRACTION: usize = 3;
const SLOT_SPREAD_BPS: usize = 4;        // BBO state (3-level)
const SLOT_QUOTE_IMBALANCE: usize = 5;   // BBO state (3-level)
const SLOT_MEAN_TRADE_SIZE: usize = 6;
const SLOT_BLOCK_TRADE_RATIO: usize = 7;
const SLOT_SIZE_CONCENTRATION: usize = 8;
const SLOT_TRF_LIT_VOLUME_RATIO: usize = 9;
const SLOT_TRF_VPIN: usize = 10;
const SLOT_LIT_VPIN: usize = 11;
const NUM_SLOTS: usize = 12; // 10 base + 2 VPIN

/// TRF-derived state feature slots (NOT BBO-derived).
/// These are always overwritten from forward-fill on empty bins.
const TRF_STATE_SLOTS: &[(usize, usize)] = &[
    (SLOT_DARK_SHARE, indices::DARK_SHARE),
    (SLOT_SUBPENNY_INTENSITY, indices::SUBPENNY_INTENSITY),
    (SLOT_ODD_LOT_RATIO, indices::ODD_LOT_RATIO),
    (SLOT_RETAIL_VOLUME_FRACTION, indices::RETAIL_VOLUME_FRACTION),
    (SLOT_MEAN_TRADE_SIZE, indices::MEAN_TRADE_SIZE),
    (SLOT_BLOCK_TRADE_RATIO, indices::BLOCK_TRADE_RATIO),
    (SLOT_SIZE_CONCENTRATION, indices::SIZE_CONCENTRATION),
    (SLOT_TRF_LIT_VOLUME_RATIO, indices::TRF_LIT_VOLUME_RATIO),
];

/// BBO-derived state feature slots.
/// Overwritten from forward-fill ONLY when the bin has zero BBO updates (Level 3).
const BBO_STATE_SLOTS: &[(usize, usize)] = &[
    (SLOT_SPREAD_BPS, indices::SPREAD_BPS),
    (SLOT_QUOTE_IMBALANCE, indices::QUOTE_IMBALANCE),
];

/// VPIN state feature slots (conditional on VPIN being enabled).
const VPIN_STATE_SLOTS: &[(usize, usize)] = &[
    (SLOT_TRF_VPIN, indices::TRF_VPIN),
    (SLOT_LIT_VPIN, indices::LIT_VPIN),
];

/// Persistent forward-fill state across bins.
///
/// At day start, all values are 0.0. Updated after each non-empty bin.
/// Applied to empty bins to prevent state features from being zero.
#[derive(Debug, Clone)]
pub(crate) struct ForwardFillState {
    /// Stored values for state-type features.
    values: [f64; NUM_SLOTS],
    /// True once at least one non-empty bin has been processed.
    pub(crate) initialized: bool,
    /// True once BBO-derived state features have been stored.
    has_bbo_ff: bool,
}

impl Default for ForwardFillState {
    fn default() -> Self {
        Self {
            values: [0.0; NUM_SLOTS],
            initialized: false,
            has_bbo_ff: false,
        }
    }
}

impl ForwardFillState {
    /// Update forward-fill state from a just-extracted feature vector.
    ///
    /// Called after every non-empty bin (trf_trades > 0). Copies state-type
    /// feature values into internal storage for future empty bins.
    pub(crate) fn update_from_features(&mut self, features: &[f64], vpin_enabled: bool) {
        // TRF-derived state features
        for &(slot, feat_idx) in TRF_STATE_SLOTS {
            self.values[slot] = features[feat_idx];
        }

        // BBO-derived state features (also updated here since TRF bin has BBO data)
        for &(slot, feat_idx) in BBO_STATE_SLOTS {
            self.values[slot] = features[feat_idx];
        }
        self.has_bbo_ff = true;

        // VPIN (if enabled)
        if vpin_enabled {
            for &(slot, feat_idx) in VPIN_STATE_SLOTS {
                self.values[slot] = features[feat_idx];
            }
        }

        self.initialized = true;
    }

    /// Update BBO-derived state features separately.
    ///
    /// Called when a bin has BBO updates but zero TRF trades (Level 2).
    /// This ensures BBO forward-fill state stays current even when
    /// there are no TRF trades to trigger `update_from_features()`.
    pub(crate) fn update_bbo_features(&mut self, spread_bps: f64, quote_imbalance: f64) {
        self.values[SLOT_SPREAD_BPS] = spread_bps;
        self.values[SLOT_QUOTE_IMBALANCE] = quote_imbalance;
        self.has_bbo_ff = true;
    }

    /// Apply forward-fill values to a feature vector for an empty bin.
    ///
    /// # 3-Level Policy
    ///
    /// - TRF-derived state features: ALWAYS overwritten from forward-fill
    /// - BBO-derived state features: overwritten ONLY if `!had_bbo_updates`
    ///   (Level 3). If the bin HAD BBO updates, the live-computed values are kept.
    /// - Flow features: untouched (remain at 0.0)
    /// - Safety gates, context: untouched (always computed)
    /// - VPIN: overwritten if `vpin_enabled`
    pub(crate) fn apply_to(
        &self,
        features: &mut [f64],
        vpin_enabled: bool,
        had_bbo_updates: bool,
    ) {
        // TRF-derived state features: always forward-fill on empty bins
        for &(slot, feat_idx) in TRF_STATE_SLOTS {
            features[feat_idx] = self.values[slot];
        }

        // BBO-derived state features: forward-fill ONLY if no BBO updates (Level 3)
        if !had_bbo_updates {
            for &(slot, feat_idx) in BBO_STATE_SLOTS {
                features[feat_idx] = self.values[slot];
            }
        }
        // If had_bbo_updates = true (Level 2): keep live-computed BBO values

        // VPIN forward-fill (if enabled)
        if vpin_enabled {
            for &(slot, feat_idx) in VPIN_STATE_SLOTS {
                features[feat_idx] = self.values[slot];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::TOTAL_FEATURES;

    fn make_feature_vec() -> Vec<f64> {
        let mut v = vec![0.0; TOTAL_FEATURES];
        // Set known values for state features
        v[indices::DARK_SHARE] = 0.75;
        v[indices::SUBPENNY_INTENSITY] = 0.45;
        v[indices::ODD_LOT_RATIO] = 0.30;
        v[indices::RETAIL_VOLUME_FRACTION] = 0.50;
        v[indices::SPREAD_BPS] = 7.5;
        v[indices::QUOTE_IMBALANCE] = 0.20;
        v[indices::MEAN_TRADE_SIZE] = 150.0;
        v[indices::BLOCK_TRADE_RATIO] = 0.05;
        v[indices::SIZE_CONCENTRATION] = 0.35;
        v[indices::TRF_LIT_VOLUME_RATIO] = 3.0;
        // Flow features
        v[indices::TRF_SIGNED_IMBALANCE] = 0.40;
        v[indices::MROIB] = 0.25;
        // Safety/context
        v[indices::BIN_VALID] = 1.0;
        v[indices::SCHEMA_VERSION_IDX] = 1.0;
        v
    }

    #[test]
    fn test_default_state() {
        let ff = ForwardFillState::default();
        assert!(!ff.initialized);
        assert!(!ff.has_bbo_ff);
        for &val in &ff.values {
            assert_eq!(val, 0.0);
        }
    }

    #[test]
    fn test_update_copies_correct_indices() {
        let mut ff = ForwardFillState::default();
        let features = make_feature_vec();
        ff.update_from_features(&features, false);

        assert!(ff.initialized);
        assert!(ff.has_bbo_ff);
        assert_eq!(ff.values[SLOT_DARK_SHARE], 0.75);
        assert_eq!(ff.values[SLOT_SUBPENNY_INTENSITY], 0.45);
        assert_eq!(ff.values[SLOT_SPREAD_BPS], 7.5);
        assert_eq!(ff.values[SLOT_QUOTE_IMBALANCE], 0.20);
        assert_eq!(ff.values[SLOT_MEAN_TRADE_SIZE], 150.0);
    }

    #[test]
    fn test_apply_with_bbo_updates_keeps_live_bbo() {
        let mut ff = ForwardFillState::default();
        let features = make_feature_vec();
        ff.update_from_features(&features, false);

        // Create empty bin with different BBO values (live-computed)
        let mut empty_bin = vec![0.0; TOTAL_FEATURES];
        empty_bin[indices::SPREAD_BPS] = 12.0;       // live BBO value
        empty_bin[indices::QUOTE_IMBALANCE] = -0.30;  // live BBO value
        empty_bin[indices::SCHEMA_VERSION_IDX] = 1.0;

        // Apply with had_bbo_updates=true (Level 2)
        ff.apply_to(&mut empty_bin, false, true);

        // TRF state features: overwritten from forward-fill
        assert_eq!(empty_bin[indices::DARK_SHARE], 0.75, "TRF state must be forward-filled");
        assert_eq!(empty_bin[indices::MEAN_TRADE_SIZE], 150.0, "TRF state must be forward-filled");

        // BBO state features: KEPT as live-computed (NOT overwritten)
        assert_eq!(
            empty_bin[indices::SPREAD_BPS], 12.0,
            "BBO with updates: spread_bps should keep live value, not forward-fill"
        );
        assert_eq!(
            empty_bin[indices::QUOTE_IMBALANCE], -0.30,
            "BBO with updates: quote_imbalance should keep live value"
        );
    }

    #[test]
    fn test_apply_without_bbo_updates_forward_fills_bbo() {
        let mut ff = ForwardFillState::default();
        let features = make_feature_vec();
        ff.update_from_features(&features, false);

        // Create empty bin (no BBO updates → Level 3)
        let mut empty_bin = vec![0.0; TOTAL_FEATURES];
        empty_bin[indices::SCHEMA_VERSION_IDX] = 1.0;

        // Apply with had_bbo_updates=false (Level 3)
        ff.apply_to(&mut empty_bin, false, false);

        // BBO state features: overwritten from forward-fill
        assert_eq!(
            empty_bin[indices::SPREAD_BPS], 7.5,
            "BBO without updates: spread_bps should be forward-filled"
        );
        assert_eq!(
            empty_bin[indices::QUOTE_IMBALANCE], 0.20,
            "BBO without updates: quote_imbalance should be forward-filled"
        );
    }

    #[test]
    fn test_vpin_conditional() {
        let mut ff = ForwardFillState::default();
        let mut features = make_feature_vec();
        features[indices::TRF_VPIN] = 0.30;
        features[indices::LIT_VPIN] = 0.15;

        // Update with VPIN enabled
        ff.update_from_features(&features, true);
        assert_eq!(ff.values[SLOT_TRF_VPIN], 0.30);

        // Apply without VPIN enabled → VPIN indices untouched
        let mut empty = vec![0.0; TOTAL_FEATURES];
        ff.apply_to(&mut empty, false, false);
        assert_eq!(empty[indices::TRF_VPIN], 0.0, "VPIN disabled: should NOT forward-fill");

        // Apply with VPIN enabled → VPIN indices forward-filled
        let mut empty2 = vec![0.0; TOTAL_FEATURES];
        ff.apply_to(&mut empty2, true, false);
        assert_eq!(empty2[indices::TRF_VPIN], 0.30, "VPIN enabled: should forward-fill");
    }

    #[test]
    fn test_update_sets_initialized() {
        let mut ff = ForwardFillState::default();
        assert!(!ff.initialized);
        ff.update_from_features(&make_feature_vec(), false);
        assert!(ff.initialized);
    }

    #[test]
    fn test_flow_features_never_overwritten() {
        let mut ff = ForwardFillState::default();
        let features = make_feature_vec();
        ff.update_from_features(&features, false);

        // Create empty bin with flow features at 0.0
        let mut empty = vec![0.0; TOTAL_FEATURES];
        ff.apply_to(&mut empty, false, false);

        // Flow features must remain at 0.0 (not touched by forward-fill)
        assert_eq!(
            empty[indices::TRF_SIGNED_IMBALANCE], 0.0,
            "Flow feature should NOT be forward-filled"
        );
        assert_eq!(
            empty[indices::MROIB], 0.0,
            "Flow feature should NOT be forward-filled"
        );
        assert_eq!(
            empty[indices::BIN_TRADE_COUNT], 0.0,
            "Activity flow should NOT be forward-filled"
        );
    }

    #[test]
    fn test_update_bbo_features_separately() {
        let mut ff = ForwardFillState::default();
        assert!(!ff.has_bbo_ff);

        ff.update_bbo_features(8.5, -0.15);
        assert!(ff.has_bbo_ff);
        assert_eq!(ff.values[SLOT_SPREAD_BPS], 8.5);
        assert_eq!(ff.values[SLOT_QUOTE_IMBALANCE], -0.15);
    }
}
