//! Schema version, pipeline constants, and feature index definitions.
//!
//! Authoritative contract constants for the off-exchange feature schema.
//! Cross-validated against the parent pipeline contract (see docs/design/06_INTEGRATION_POINTS.md).
//!
//! # Price Precision Chain
//!
//! ```text
//! Databento wire:    i64 nanodollars (1 USD = 1,000,000,000 nanodollars)
//!   → CmbpRecord:   i64 nanodollars (preserved, no conversion)
//!   → BboState:     f64 USD (converted ONCE via NANO_TO_USD at update boundary)
//!   → Features:     f64 (all computation)
//!   → NPY export:   f32 (downcast at export boundary, with is_finite() guard)
//! ```

/// Division guard for all denominators across the pipeline.
/// Pipeline-wide constant, same value in all HFT pipeline modules.
/// Pipeline-wide constant, consistent across all HFT pipeline modules.
pub const EPS: f64 = 1e-8;

/// Nanodollar-to-USD conversion multiplier.
///
/// `price_usd = price_nanodollars as f64 * NANO_TO_USD`
///
/// Named `NANO_TO_USD` (not `FIXED_PRICE_SCALE`) to avoid collision with
/// `dbn::FIXED_PRICE_SCALE` which is `i64 = 1_000_000_000` (the inverse).
pub const NANO_TO_USD: f64 = 1e-9;

/// Sentinel value for undefined/missing prices in the dbn crate.
///
/// Equal to `i64::MAX = 9_223_372_036_854_775_807` nanodollars.
/// Records with this price value must be rejected (not converted to f64).
pub const UNDEF_PRICE: i64 = i64::MAX;

/// Off-exchange feature schema version.
/// Emitted at feature index 33 in every feature vector.
/// Independent of MBO pipeline schema version (2.2).
///
/// Source: docs/design/04_FEATURE_SPECIFICATION.md
pub const SCHEMA_VERSION: f64 = 1.0;

/// Off-exchange contract version string.
/// Independent of MBO pipeline contract version (2.2).
///
/// Source: docs/design/04_FEATURE_SPECIFICATION.md
pub const CONTRACT_VERSION: &str = "off_exchange_1.0";

/// Default label horizons in bins.
/// At 60s bins: H=1 (1 min), H=10 (10 min), H=60 (1 hr).
///
/// Source: docs/design/04_FEATURE_SPECIFICATION.md
pub const DEFAULT_HORIZONS: &[usize] = &[1, 2, 3, 5, 10, 20, 30, 60];

/// Default sequence window size (bins per sequence).
///
/// Source: docs/design/06_INTEGRATION_POINTS.md §1.3
pub const DEFAULT_WINDOW_SIZE: usize = 20;

/// Default stride for sliding window.
pub const DEFAULT_STRIDE: usize = 1;

/// Feature names ordered by index, for normalization JSON and metadata.
/// Validated against the pipeline contract at integration time.
///
/// Source: docs/design/04_FEATURE_SPECIFICATION.md §2
pub const FEATURE_NAMES: [&str; 34] = [
    "trf_signed_imbalance",   // 0
    "mroib",                   // 1
    "inv_inst_direction",      // 2
    "bvc_imbalance",           // 3
    "dark_share",              // 4
    "trf_volume",              // 5
    "lit_volume",              // 6
    "total_volume",            // 7
    "subpenny_intensity",      // 8
    "odd_lot_ratio",           // 9
    "retail_trade_rate",       // 10
    "retail_volume_fraction",  // 11
    "spread_bps",              // 12
    "bid_pressure",            // 13
    "ask_pressure",            // 14
    "bbo_update_rate",         // 15
    "quote_imbalance",         // 16
    "spread_change_rate",      // 17
    "trf_vpin",                // 18
    "lit_vpin",                // 19
    "mean_trade_size",         // 20
    "block_trade_ratio",       // 21
    "trade_count",             // 22
    "size_concentration",      // 23
    "trf_burst_intensity",     // 24
    "time_since_burst",        // 25
    "trf_lit_volume_ratio",    // 26
    "bin_trade_count",         // 27
    "bin_trf_trade_count",     // 28
    "bin_valid",               // 29
    "bbo_valid",               // 30
    "session_progress",        // 31
    "time_bucket",             // 32
    "schema_version",          // 33
];

/// Total number of off-exchange features (indices 0-33).
/// Sum: signed_flow(4) + venue_metrics(4) + retail_metrics(4) + bbo_dynamics(6)
///    + vpin(2) + trade_size(4) + cross_venue(3) + activity(2) + safety_gates(2)
///    + context(3) = 34
///
/// Source: docs/design/04_FEATURE_SPECIFICATION.md §1
pub const TOTAL_FEATURES: usize = 34;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_eps_is_pipeline_standard() {
        assert_eq!(EPS, 1e-8, "EPS must match pipeline-wide constant");
    }

    #[test]
    fn test_nano_to_usd_conversion() {
        // $100.00 = 100_000_000_000 nanodollars
        let price_nano: i64 = 100_000_000_000;
        let price_usd = price_nano as f64 * NANO_TO_USD;
        assert!(
            (price_usd - 100.0).abs() < 1e-15,
            "100B nanodollars should convert to exactly $100.00, got {}",
            price_usd
        );
    }

    #[test]
    fn test_undef_price_is_i64_max() {
        assert_eq!(UNDEF_PRICE, i64::MAX);
    }

    #[test]
    fn test_schema_version() {
        assert_eq!(SCHEMA_VERSION, 1.0);
    }

    #[test]
    fn test_feature_names_length_matches_total_features() {
        assert_eq!(
            FEATURE_NAMES.len(),
            TOTAL_FEATURES,
            "FEATURE_NAMES length must match TOTAL_FEATURES"
        );
    }

    #[test]
    fn test_feature_names_no_duplicates() {
        let mut seen = std::collections::HashSet::new();
        for name in &FEATURE_NAMES {
            assert!(
                seen.insert(name),
                "Duplicate feature name: {}",
                name
            );
        }
    }

    #[test]
    fn test_default_horizons_sorted_ascending() {
        for i in 1..DEFAULT_HORIZONS.len() {
            assert!(
                DEFAULT_HORIZONS[i] > DEFAULT_HORIZONS[i - 1],
                "DEFAULT_HORIZONS not sorted: {} <= {}",
                DEFAULT_HORIZONS[i],
                DEFAULT_HORIZONS[i - 1]
            );
        }
    }

    #[test]
    fn test_default_horizons_max_is_60() {
        assert_eq!(
            *DEFAULT_HORIZONS.last().unwrap(),
            60,
            "Default max horizon should be 60 bins (1 hour at 60s)"
        );
    }

    #[test]
    fn test_total_features() {
        assert_eq!(TOTAL_FEATURES, 34);
        // Verify group sum: 4+4+4+6+2+4+3+2+2+3 = 34
        let group_sizes = [4, 4, 4, 6, 2, 4, 3, 2, 2, 3];
        assert_eq!(
            group_sizes.iter().sum::<usize>(),
            TOTAL_FEATURES,
            "Group sizes must sum to TOTAL_FEATURES"
        );
    }
}
