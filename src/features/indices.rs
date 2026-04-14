//! Named constants for all 34 off-exchange feature indices.
//!
//! Every feature computation downstream uses these constants instead of raw
//! integer indices. This prevents hardcoded-index bugs and makes feature
//! references self-documenting.
//!
//! # Index Space
//!
//! Off-exchange features use an INDEPENDENT index space (0-33), separate from
//! the MBO pipeline's 0-147 space. Fusion happens downstream at integration.
//!
//! Source: docs/design/04_FEATURE_SPECIFICATION.md

use std::ops::Range;

// ── Signed Flow (0-3) ─────────────────────────────────────────────────
/// TRF signed imbalance: (buy - sell) / (buy + sell). Primary directional signal.
pub const TRF_SIGNED_IMBALANCE: usize = 0;
/// Market Retail Order Imbalance. Retail-only signed flow.
pub const MROIB: usize = 1;
/// Inverse institutional direction: -MROIB.
pub const INV_INST_DIRECTION: usize = 2;
/// BVC imbalance: probabilistic buy/sell from Easley et al. (2012) Eq. 7.
pub const BVC_IMBALANCE: usize = 3;

// ── Venue Metrics (4-7) ───────────────────────────────────────────────
/// TRF share of volume: trf / (trf + lit). NOT consolidated dark share.
pub const DARK_SHARE: usize = 4;
/// Total TRF volume in shares.
pub const TRF_VOLUME: usize = 5;
/// Total lit (XNAS/XBOS/XPSX) volume in shares.
pub const LIT_VOLUME: usize = 6;
/// Total volume (TRF + lit) in shares.
pub const TOTAL_VOLUME: usize = 7;

// ── Retail Metrics (8-11) ─────────────────────────────────────────────
/// Fraction of TRF trades with subpenny execution. COUNT ratio.
pub const SUBPENNY_INTENSITY: usize = 8;
/// Fraction of TRF trades with size < 100. COUNT ratio.
pub const ODD_LOT_RATIO: usize = 9;
/// Fraction of TRF trades classified as retail. COUNT ratio.
pub const RETAIL_TRADE_RATE: usize = 10;
/// Retail volume / total TRF volume. Volume ratio.
pub const RETAIL_VOLUME_FRACTION: usize = 11;

// ── BBO Dynamics (12-17) ──────────────────────────────────────────────
/// Time-weighted average spread in basis points over the bin.
pub const SPREAD_BPS: usize = 12;
/// Bid size fractional change from bin start to end.
pub const BID_PRESSURE: usize = 13;
/// Ask size fractional change from bin start to end.
pub const ASK_PRESSURE: usize = 14;
/// Count of BBO updates within the bin.
pub const BBO_UPDATE_RATE: usize = 15;
/// (bid_sz - ask_sz) / (bid_sz + ask_sz) at end of bin.
pub const QUOTE_IMBALANCE: usize = 16;
/// spread_end_bps - spread_start_bps within the bin.
pub const SPREAD_CHANGE_RATE: usize = 17;

// ── VPIN (18-19, disabled by default) ─────────────────────────────────
/// Volume-synchronized PIN for TRF trades. Easley et al. (2012).
pub const TRF_VPIN: usize = 18;
/// Volume-synchronized PIN for lit trades.
pub const LIT_VPIN: usize = 19;

// ── Trade Size (20-23) ────────────────────────────────────────────────
/// Mean trade size across all trades in bin (shares).
pub const MEAN_TRADE_SIZE: usize = 20;
/// Fraction of trades with size >= block_threshold. COUNT ratio.
pub const BLOCK_TRADE_RATIO: usize = 21;
/// Total trade count (all venues) in bin.
pub const TRADE_COUNT: usize = 22;
/// Herfindahl-Hirschman Index of trade sizes. SUM((size_i / total_vol)^2).
pub const SIZE_CONCENTRATION: usize = 23;

// ── Cross-Venue (24-26) ───────────────────────────────────────────────
/// Coefficient of variation of TRF inter-arrival times within bin.
pub const TRF_BURST_INTENSITY: usize = 24;
/// Seconds since last detected TRF burst. Capped at warmup period if no burst.
pub const TIME_SINCE_BURST: usize = 25;
/// TRF volume / lit volume ratio.
pub const TRF_LIT_VOLUME_RATIO: usize = 26;

// ── Activity (27-28, always enabled) ──────────────────────────────────
/// Total trade count (all venues, all types) in bin.
pub const BIN_TRADE_COUNT: usize = 27;
/// TRF trade count in bin.
pub const BIN_TRF_TRADE_COUNT: usize = 28;

// ── Safety Gates (29-30, always enabled) ──────────────────────────────
/// 1.0 if trf_trades >= min_trades_per_bin, else 0.0. Categorical.
pub const BIN_VALID: usize = 29;
/// 1.0 if BBO valid and not stale, else 0.0. Categorical.
pub const BBO_VALID: usize = 30;

// ── Context (31-33, always enabled) ───────────────────────────────────
/// Fraction of trading session elapsed. Clamped [0.0, 1.0].
pub const SESSION_PROGRESS: usize = 31;
/// Intraday regime bucket (0-5). Categorical.
pub const TIME_BUCKET: usize = 32;
/// Off-exchange schema version. Always 1.0. Categorical.
pub const SCHEMA_VERSION_IDX: usize = 33;

// ── Group Ranges ──────────────────────────────────────────────────────
pub const SIGNED_FLOW_RANGE: Range<usize> = 0..4;
pub const VENUE_METRICS_RANGE: Range<usize> = 4..8;
pub const RETAIL_METRICS_RANGE: Range<usize> = 8..12;
pub const BBO_DYNAMICS_RANGE: Range<usize> = 12..18;
pub const VPIN_RANGE: Range<usize> = 18..20;
pub const TRADE_SIZE_RANGE: Range<usize> = 20..24;
pub const CROSS_VENUE_RANGE: Range<usize> = 24..27;
pub const ACTIVITY_RANGE: Range<usize> = 27..29;
pub const SAFETY_GATES_RANGE: Range<usize> = 29..31;
pub const CONTEXT_RANGE: Range<usize> = 31..34;

// ── Feature Classifications ──────────────────────────────────────────

/// Categorical features: never normalized. Values are discrete labels.
/// Source: docs/design/04_FEATURE_SPECIFICATION.md [features.off_exchange.categorical]
pub const CATEGORICAL_INDICES: &[usize] = &[BIN_VALID, BBO_VALID, TIME_BUCKET, SCHEMA_VERSION_IDX];

/// State-type features that forward-fill on empty bins (trf_trades == 0).
/// These describe persistent market state; their last known value is more
/// informative than zero when no TRF trades occur.
///
/// NOTE: `SESSION_PROGRESS` (31) is listed as "state" in spec §8.1 but is
/// always computed from the clock, so forward-fill is unnecessary. It is
/// classified under "context (always computed)" instead.
pub const FORWARD_FILL_INDICES: &[usize] = &[
    DARK_SHARE,
    SUBPENNY_INTENSITY,
    ODD_LOT_RATIO,
    RETAIL_VOLUME_FRACTION,
    SPREAD_BPS,
    QUOTE_IMBALANCE,
    MEAN_TRADE_SIZE,
    BLOCK_TRADE_RATIO,
    SIZE_CONCENTRATION,
    // NOTE: TIME_SINCE_BURST is intentionally EXCLUDED — see ALWAYS_COMPUTED_STATE below.
    TRF_LIT_VOLUME_RATIO,
];

/// Features that use persistent cross-bin state and are always recomputed,
/// never forward-filled. Their values change over time even without new trades.
///
/// `time_since_burst` (25): Recomputed from `(current_ts - last_burst_ts)` using
/// the burst tracker's persistent state. Forward-filling would produce stale values
/// because time increments between bins. The burst tracker persists across bins,
/// so this method works correctly for normal bins, empty bins, and gap bins.
pub const ALWAYS_COMPUTED_STATE: &[usize] = &[TIME_SINCE_BURST];

/// VPIN forward-fill indices (only active when VPIN feature group is enabled).
pub const FORWARD_FILL_VPIN_INDICES: &[usize] = &[TRF_VPIN, LIT_VPIN];

/// TRF-derived flow features that are zero on empty bins (no TRF trades).
///
/// NOTE: BBO-derived flow features (bid_pressure=13, ask_pressure=14,
/// bbo_update_rate=15, spread_change_rate=17) are intentionally EXCLUDED.
/// They have different empty-bin semantics: computed from live BBO state
/// when available, not from TRF trade flow. They are handled separately
/// by the 3-level empty bin policy in forward_fill.rs.
pub const FLOW_INDICES: &[usize] = &[
    TRF_SIGNED_IMBALANCE,
    MROIB,
    INV_INST_DIRECTION,
    BVC_IMBALANCE,
    TRF_VOLUME,
    LIT_VOLUME,
    TOTAL_VOLUME,
    RETAIL_TRADE_RATE,
    TRADE_COUNT,
    TRF_BURST_INTENSITY,
    BIN_TRADE_COUNT,
    BIN_TRF_TRADE_COUNT,
];

/// BBO-derived state features: a SUBSET of `FORWARD_FILL_INDICES`.
///
/// These use live BBO values when BBO updates exist in the bin, and
/// forward-fill ONLY when the bin has zero BBO updates. This is the
/// distinction between Level 2 and Level 3 of the 3-level empty bin policy.
///
/// See: 04_FEATURE_SPECIFICATION.md Section 8, 03_DATA_FLOW.md Section 5.
pub const BBO_STATE_INDICES: &[usize] = &[SPREAD_BPS, QUOTE_IMBALANCE];

/// Number of feature groups that are always enabled (activity + safety + context).
pub const ALWAYS_ENABLED_COUNT: usize = 2 + 2 + 3; // 7

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::TOTAL_FEATURES;
    use std::collections::HashSet;

    #[test]
    fn test_all_indices_in_valid_range() {
        let all_indices = [
            TRF_SIGNED_IMBALANCE, MROIB, INV_INST_DIRECTION, BVC_IMBALANCE,
            DARK_SHARE, TRF_VOLUME, LIT_VOLUME, TOTAL_VOLUME,
            SUBPENNY_INTENSITY, ODD_LOT_RATIO, RETAIL_TRADE_RATE, RETAIL_VOLUME_FRACTION,
            SPREAD_BPS, BID_PRESSURE, ASK_PRESSURE, BBO_UPDATE_RATE,
            QUOTE_IMBALANCE, SPREAD_CHANGE_RATE,
            TRF_VPIN, LIT_VPIN,
            MEAN_TRADE_SIZE, BLOCK_TRADE_RATIO, TRADE_COUNT, SIZE_CONCENTRATION,
            TRF_BURST_INTENSITY, TIME_SINCE_BURST, TRF_LIT_VOLUME_RATIO,
            BIN_TRADE_COUNT, BIN_TRF_TRADE_COUNT,
            BIN_VALID, BBO_VALID,
            SESSION_PROGRESS, TIME_BUCKET, SCHEMA_VERSION_IDX,
        ];
        assert_eq!(all_indices.len(), TOTAL_FEATURES, "Must have exactly 34 indices");
        for &idx in &all_indices {
            assert!(idx < TOTAL_FEATURES, "Index {} out of range [0, {})", idx, TOTAL_FEATURES);
        }
    }

    #[test]
    fn test_no_duplicate_indices() {
        let all_indices = [
            TRF_SIGNED_IMBALANCE, MROIB, INV_INST_DIRECTION, BVC_IMBALANCE,
            DARK_SHARE, TRF_VOLUME, LIT_VOLUME, TOTAL_VOLUME,
            SUBPENNY_INTENSITY, ODD_LOT_RATIO, RETAIL_TRADE_RATE, RETAIL_VOLUME_FRACTION,
            SPREAD_BPS, BID_PRESSURE, ASK_PRESSURE, BBO_UPDATE_RATE,
            QUOTE_IMBALANCE, SPREAD_CHANGE_RATE,
            TRF_VPIN, LIT_VPIN,
            MEAN_TRADE_SIZE, BLOCK_TRADE_RATIO, TRADE_COUNT, SIZE_CONCENTRATION,
            TRF_BURST_INTENSITY, TIME_SINCE_BURST, TRF_LIT_VOLUME_RATIO,
            BIN_TRADE_COUNT, BIN_TRF_TRADE_COUNT,
            BIN_VALID, BBO_VALID,
            SESSION_PROGRESS, TIME_BUCKET, SCHEMA_VERSION_IDX,
        ];
        let unique: HashSet<usize> = all_indices.iter().copied().collect();
        assert_eq!(
            unique.len(),
            all_indices.len(),
            "Duplicate index detected: {} unique vs {} total",
            unique.len(),
            all_indices.len()
        );
    }

    #[test]
    fn test_group_ranges_contiguous_and_non_overlapping() {
        let ranges: &[(&str, Range<usize>)] = &[
            ("signed_flow", SIGNED_FLOW_RANGE),
            ("venue_metrics", VENUE_METRICS_RANGE),
            ("retail_metrics", RETAIL_METRICS_RANGE),
            ("bbo_dynamics", BBO_DYNAMICS_RANGE),
            ("vpin", VPIN_RANGE),
            ("trade_size", TRADE_SIZE_RANGE),
            ("cross_venue", CROSS_VENUE_RANGE),
            ("activity", ACTIVITY_RANGE),
            ("safety_gates", SAFETY_GATES_RANGE),
            ("context", CONTEXT_RANGE),
        ];
        // Verify contiguous (each range starts where the previous ended)
        for window in ranges.windows(2) {
            let (name_a, range_a) = &window[0];
            let (name_b, range_b) = &window[1];
            assert_eq!(
                range_a.end, range_b.start,
                "Gap between {} ({:?}) and {} ({:?})",
                name_a, range_a, name_b, range_b
            );
        }
        // First starts at 0, last ends at TOTAL_FEATURES
        assert_eq!(ranges[0].1.start, 0, "First range must start at 0");
        assert_eq!(
            ranges.last().unwrap().1.end,
            TOTAL_FEATURES,
            "Last range must end at TOTAL_FEATURES ({})",
            TOTAL_FEATURES
        );
    }

    #[test]
    fn test_categorical_indices_match_spec() {
        // Source: docs/design/04_FEATURE_SPECIFICATION.md [features.off_exchange.categorical]
        assert_eq!(
            CATEGORICAL_INDICES,
            &[29, 30, 32, 33],
            "Categorical indices must be [bin_valid(29), bbo_valid(30), time_bucket(32), schema_version(33)]"
        );
    }

    #[test]
    fn test_all_indices_classified() {
        // Every index 0..34 must be in exactly one classification:
        // FORWARD_FILL_INDICES, FLOW_INDICES, BBO_STATE_INDICES (subset of FF),
        // BID_PRESSURE, ASK_PRESSURE, BBO_UPDATE_RATE, SPREAD_CHANGE_RATE (BBO flow),
        // SAFETY_GATES, CONTEXT
        let mut classified: HashSet<usize> = HashSet::new();

        // Forward-fill state features (includes BBO state features)
        for &idx in FORWARD_FILL_INDICES {
            assert!(classified.insert(idx), "Duplicate classification for index {}", idx);
        }
        for &idx in FORWARD_FILL_VPIN_INDICES {
            assert!(classified.insert(idx), "Duplicate classification for index {}", idx);
        }

        // Flow features
        for &idx in FLOW_INDICES {
            assert!(classified.insert(idx), "Duplicate classification for index {}", idx);
        }

        // Always-computed state (persistent cross-bin state, recomputed each bin)
        for &idx in ALWAYS_COMPUTED_STATE {
            assert!(classified.insert(idx), "Duplicate classification for index {}", idx);
        }

        // BBO flow features (not forward-filled, not in FLOW_INDICES — BBO-derived flow)
        let bbo_flow = [BID_PRESSURE, ASK_PRESSURE, BBO_UPDATE_RATE, SPREAD_CHANGE_RATE];
        for &idx in &bbo_flow {
            assert!(classified.insert(idx), "Duplicate classification for index {}", idx);
        }

        // Safety gates (always computed)
        classified.insert(BIN_VALID);
        classified.insert(BBO_VALID);

        // Context (always computed)
        classified.insert(SESSION_PROGRESS);
        classified.insert(TIME_BUCKET);
        classified.insert(SCHEMA_VERSION_IDX);

        assert_eq!(
            classified.len(),
            TOTAL_FEATURES,
            "Not all {} features classified: missing {:?}",
            TOTAL_FEATURES,
            (0..TOTAL_FEATURES).filter(|i| !classified.contains(i)).collect::<Vec<_>>()
        );
    }
}
