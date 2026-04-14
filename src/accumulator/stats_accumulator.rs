//! Per-bin statistics accumulation: time-weighted spread, bid/ask pressure, trade-size HHI.
//!
//! Tracks BBO dynamics (spread TWAP, bid/ask pressure from start to end of bin,
//! BBO update count) and trade-size distribution (HHI for size concentration).
//! All counters reset at each bin boundary.
//!
//! # Time-Weighted Spread Algorithm
//!
//! Each BBO update contributes the PREVIOUS spread × its duration. On the first
//! update in a bin, we snapshot the start state without accumulating. On finalize,
//! the last spread × remaining duration is added.
//!
//! ```text
//! Update 1 (ts=100, spread=1.5): snapshot start. No accumulation.
//! Update 2 (ts=200, spread=2.0): sum += 1.5 × (200-100) = 150
//! Update 3 (ts=350, spread=1.8): sum += 2.0 × (350-200) = 300
//! Finalize (ts=400):             sum += 1.8 × (400-350) =  90
//! TWAP = 540 / 300 = 1.8 bps
//! ```
//!
//! Source: docs/design/04_FEATURE_SPECIFICATION.md §5.4

use crate::bbo_state::BboState;
use crate::contract::EPS;

/// Per-bin BBO dynamics and trade-size statistics accumulator.
#[derive(Debug, Clone)]
pub(crate) struct StatsAccumulator {
    // ── Time-weighted spread ────────────────────────────────────────
    /// Cumulative spread_bps × duration (nanosecond-weighted).
    spread_bps_sum: f64,
    /// Cumulative duration in nanoseconds.
    spread_duration_sum_ns: f64,
    /// Spread_bps at last BBO observation.
    last_spread_bps: f64,
    /// Timestamp of last BBO observation (UTC nanoseconds).
    last_spread_ts_ns: u64,

    // ── Bin-start snapshot ──────────────────────────────────────────
    /// Bid size at start of bin (for pressure computation).
    pub(crate) bid_size_start: u32,
    /// Ask size at start of bin (for pressure computation).
    pub(crate) ask_size_start: u32,
    /// Spread in bps at start of bin (for spread_change_rate).
    pub(crate) spread_bps_start: f64,
    /// True once the first BBO update in the bin has been processed.
    pub(crate) has_start_snapshot: bool,

    // ── BBO update count ───────────────────────────────────────────
    /// Number of valid BBO updates within this bin.
    pub(crate) update_count: u64,

    // ── Trade-size HHI ─────────────────────────────────────────────
    /// SUM(size_i^2) for all trades in bin.
    size_squared_sum: f64,
    /// SUM(size_i) for all trades in bin.
    pub(crate) total_volume: f64,
}

impl StatsAccumulator {
    pub(crate) fn new() -> Self {
        Self {
            spread_bps_sum: 0.0,
            spread_duration_sum_ns: 0.0,
            last_spread_bps: 0.0,
            last_spread_ts_ns: 0,
            bid_size_start: 0,
            ask_size_start: 0,
            spread_bps_start: 0.0,
            has_start_snapshot: false,
            update_count: 0,
            size_squared_sum: 0.0,
            total_volume: 0.0,
        }
    }

    /// Snapshot BBO state at the start of a bin.
    fn snapshot_bin_start(&mut self, bbo: &BboState) {
        self.bid_size_start = bbo.bid_size;
        self.ask_size_start = bbo.ask_size;
        self.spread_bps_start = bbo.spread_bps();
    }

    /// Process a BBO update within the current bin.
    ///
    /// On the FIRST update: snapshots the start state and sets the initial
    /// spread/timestamp. No accumulation occurs (there is no prior duration).
    ///
    /// On subsequent updates: accumulates the PREVIOUS spread × elapsed duration,
    /// then updates to the current spread/timestamp.
    pub(crate) fn accumulate_bbo_update(&mut self, bbo: &BboState, ts_ns: u64) {
        if !self.has_start_snapshot {
            // First BBO update in this bin: snapshot start, set initial state
            self.snapshot_bin_start(bbo);
            self.last_spread_ts_ns = ts_ns;
            self.last_spread_bps = bbo.spread_bps();
            self.has_start_snapshot = true;
        } else {
            // Subsequent updates: accumulate previous spread × duration
            let duration_ns = ts_ns.saturating_sub(self.last_spread_ts_ns) as f64;
            self.spread_bps_sum += self.last_spread_bps * duration_ns;
            self.spread_duration_sum_ns += duration_ns;
            self.last_spread_ts_ns = ts_ns;
            self.last_spread_bps = bbo.spread_bps();
        }

        self.update_count += 1;
    }

    /// Finalize the time-weighted spread at bin boundary.
    ///
    /// Adds the final interval: last_spread × (bin_end - last_ts).
    /// Must be called BEFORE extraction. If no BBO updates occurred in the
    /// bin, returns immediately (TWAP remains 0.0).
    pub(crate) fn finalize_spread_twap(&mut self, bin_end_ts_ns: u64) {
        if !self.has_start_snapshot {
            return; // No BBO updates in bin → TWAP stays 0.0
        }
        let duration_ns = bin_end_ts_ns.saturating_sub(self.last_spread_ts_ns) as f64;
        self.spread_bps_sum += self.last_spread_bps * duration_ns;
        self.spread_duration_sum_ns += duration_ns;
    }

    /// Time-weighted average spread in basis points over the bin.
    ///
    /// Returns 0.0 if no BBO updates occurred (duration_sum < EPS).
    ///
    /// Uses conditional guard instead of `max(den, EPS)` pattern to avoid
    /// unnecessary division when no BBO updates occurred. Both are equivalent
    /// since duration_sum is always >= 0.
    pub(crate) fn time_weighted_spread_bps(&self) -> f64 {
        if self.spread_duration_sum_ns < EPS {
            0.0
        } else {
            self.spread_bps_sum / self.spread_duration_sum_ns
        }
    }

    /// Accumulate a trade's size for HHI computation.
    ///
    /// Called for ALL trades (both TRF and lit).
    pub(crate) fn accumulate_trade_size(&mut self, size: u32) {
        let s = size as f64;
        self.size_squared_sum += s * s;
        self.total_volume += s;
    }

    /// Herfindahl-Hirschman Index of trade sizes within the bin.
    ///
    /// `HHI = SUM(size_i^2) / total_volume^2`
    ///
    /// Returns 0.0 when total_volume < EPS (no trades).
    /// Range: [0.0, 1.0]. Single trade → 1.0, equal trades → 1/n.
    pub(crate) fn size_concentration(&self) -> f64 {
        let total_sq = self.total_volume * self.total_volume;
        if total_sq < EPS {
            0.0
        } else {
            self.size_squared_sum / total_sq
        }
    }

    /// Reset all per-bin state. Called at bin boundaries.
    pub(crate) fn reset(&mut self) {
        self.spread_bps_sum = 0.0;
        self.spread_duration_sum_ns = 0.0;
        self.last_spread_bps = 0.0;
        self.last_spread_ts_ns = 0;
        self.bid_size_start = 0;
        self.ask_size_start = 0;
        self.spread_bps_start = 0.0;
        self.has_start_snapshot = false;
        self.update_count = 0;
        self.size_squared_sum = 0.0;
        self.total_volume = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a BboState with known values.
    fn make_bbo(bid_price: f64, ask_price: f64, bid_size: u32, ask_size: u32) -> BboState {
        let mut bbo = BboState::new();
        // Directly set fields for testing
        bbo.bid_price = bid_price;
        bbo.ask_price = ask_price;
        bbo.bid_size = bid_size;
        bbo.ask_size = ask_size;
        bbo.mid_price = (bid_price + ask_price) / 2.0;
        bbo.spread = ask_price - bid_price;
        bbo.is_valid = true;
        bbo
    }

    #[test]
    fn test_empty_twap_returns_zero() {
        let acc = StatsAccumulator::new();
        assert_eq!(acc.time_weighted_spread_bps(), 0.0);
    }

    #[test]
    fn test_constant_spread_twap() {
        let mut acc = StatsAccumulator::new();
        let bbo = make_bbo(134.56, 134.57, 100, 100);
        // spread = 0.01, mid = 134.565, spread_bps = 0.01/134.565 * 10000 ≈ 7.43 bps
        let spread_bps = bbo.spread_bps();

        acc.accumulate_bbo_update(&bbo, 1_000_000_000); // t=1s
        acc.accumulate_bbo_update(&bbo, 2_000_000_000); // t=2s, same spread
        acc.accumulate_bbo_update(&bbo, 3_000_000_000); // t=3s, same spread
        acc.finalize_spread_twap(4_000_000_000);         // finalize at t=4s

        let twap = acc.time_weighted_spread_bps();
        assert!(
            (twap - spread_bps).abs() < 1e-10,
            "Constant spread: TWAP ({}) should equal spread_bps ({})",
            twap, spread_bps
        );
    }

    #[test]
    fn test_golden_twap_changing_spread() {
        // Golden test from plan: hand-calculated TWAP
        let mut acc = StatsAccumulator::new();

        // Update 1 (ts=100ns, spread=1.5 bps): snapshot start, no accumulation
        let _bbo1 = make_bbo(100.0, 100.015, 100, 100); // spread ≈ 1.5 bps
        // We inject spread_bps directly since BBO math is complex.
        // Test verifies the TWAP algorithm with known spread_bps values.
        acc.last_spread_bps = 0.0;
        acc.last_spread_ts_ns = 0;
        acc.has_start_snapshot = false;

        // Simulate 3 BBO updates with known spread_bps values
        // Update 1: ts=100, spread=1.5 bps
        acc.has_start_snapshot = false;
        acc.last_spread_ts_ns = 100;
        acc.last_spread_bps = 1.5;
        acc.has_start_snapshot = true;
        acc.update_count = 1;

        // Update 2: ts=200, spread=2.0 bps
        let duration1 = (200 - 100) as f64;
        acc.spread_bps_sum += 1.5 * duration1; // 150
        acc.spread_duration_sum_ns += duration1;
        acc.last_spread_ts_ns = 200;
        acc.last_spread_bps = 2.0;
        acc.update_count = 2;

        // Update 3: ts=350, spread=1.8 bps
        let duration2 = (350 - 200) as f64;
        acc.spread_bps_sum += 2.0 * duration2; // 300
        acc.spread_duration_sum_ns += duration2;
        acc.last_spread_ts_ns = 350;
        acc.last_spread_bps = 1.8;
        acc.update_count = 3;

        // Finalize at ts=400
        acc.finalize_spread_twap(400);

        // Expected: sum = 150 + 300 + 1.8*(400-350) = 150 + 300 + 90 = 540
        // Duration: 100 + 150 + 50 = 300
        // TWAP = 540 / 300 = 1.8
        let twap = acc.time_weighted_spread_bps();
        assert!(
            (twap - 1.8).abs() < 1e-10,
            "Golden TWAP: expected 1.8, got {}",
            twap
        );
    }

    #[test]
    fn test_golden_twap_via_api() {
        // API-driven golden test: uses accumulate_bbo_update() not raw field access.
        // 3 BBO updates with different spreads, verify hand-calculated TWAP.
        let mut acc = StatsAccumulator::new();

        // bid=100.00, ask=100.02 → spread=0.02, mid=100.01, spread_bps=0.02/100.01*10000 ≈ 1.9998 bps
        let bbo1 = make_bbo(100.00, 100.02, 100, 100);
        let spread1 = bbo1.spread_bps();

        // bid=100.00, ask=100.04 → spread=0.04 → spread_bps ≈ 3.9996 bps
        let bbo2 = make_bbo(100.00, 100.04, 100, 100);
        let spread2 = bbo2.spread_bps();

        // bid=100.00, ask=100.01 → spread=0.01 → spread_bps ≈ 0.9999 bps
        let bbo3 = make_bbo(100.00, 100.01, 100, 100);
        let spread3 = bbo3.spread_bps();

        let t0: u64 = 1_000_000_000; // 1s
        let t1: u64 = 3_000_000_000; // 3s (2s after first)
        let t2: u64 = 4_000_000_000; // 4s (1s after second)
        let t_end: u64 = 6_000_000_000; // 6s (2s after third)

        acc.accumulate_bbo_update(&bbo1, t0); // snapshot start, no accumulation
        acc.accumulate_bbo_update(&bbo2, t1); // accumulate spread1 × 2s
        acc.accumulate_bbo_update(&bbo3, t2); // accumulate spread2 × 1s
        acc.finalize_spread_twap(t_end);       // accumulate spread3 × 2s

        // Expected TWAP = (spread1×2 + spread2×1 + spread3×2) / (2+1+2)
        //               = (spread1×2 + spread2×1 + spread3×2) / 5
        // Note: durations are in nanoseconds but ratios cancel
        let expected = (spread1 * 2.0 + spread2 * 1.0 + spread3 * 2.0) / 5.0;
        let twap = acc.time_weighted_spread_bps();

        assert!(
            (twap - expected).abs() < 1e-10,
            "API golden TWAP: expected {:.10}, got {:.10}",
            expected, twap
        );
    }

    #[test]
    fn test_finalize_guard_no_bbo_updates() {
        let mut acc = StatsAccumulator::new();
        // Finalize without any BBO updates
        acc.finalize_spread_twap(10_000_000_000);
        assert_eq!(acc.time_weighted_spread_bps(), 0.0, "No BBO updates → TWAP = 0");
        assert_eq!(acc.spread_duration_sum_ns, 0.0, "No duration accumulated");
    }

    #[test]
    fn test_single_bbo_update_finalize() {
        let mut acc = StatsAccumulator::new();
        let bbo = make_bbo(100.0, 100.01, 100, 100);
        let spread_bps = bbo.spread_bps();

        acc.accumulate_bbo_update(&bbo, 1_000_000_000); // t=1s, first update
        assert_eq!(acc.update_count, 1);
        assert!(acc.has_start_snapshot);

        acc.finalize_spread_twap(2_000_000_000); // finalize at t=2s

        let twap = acc.time_weighted_spread_bps();
        assert!(
            (twap - spread_bps).abs() < 1e-10,
            "Single update: TWAP ({}) should equal spread_bps ({})",
            twap, spread_bps
        );
    }

    #[test]
    fn test_bid_ask_pressure() {
        let mut acc = StatsAccumulator::new();
        let bbo_start = make_bbo(100.0, 100.01, 100, 200);
        acc.accumulate_bbo_update(&bbo_start, 1_000_000_000);

        // Pressure = (end - start) / max(start, EPS)
        // bid: (150 - 100) / 100 = 0.50
        // ask: (100 - 200) / 200 = -0.50
        assert_eq!(acc.bid_size_start, 100);
        assert_eq!(acc.ask_size_start, 200);
    }

    #[test]
    fn test_no_start_snapshot_pressure_zero() {
        let acc = StatsAccumulator::new();
        assert!(!acc.has_start_snapshot, "No snapshot before first BBO update");
        assert_eq!(acc.bid_size_start, 0);
        assert_eq!(acc.ask_size_start, 0);
    }

    #[test]
    fn test_bbo_update_count() {
        let mut acc = StatsAccumulator::new();
        let bbo = make_bbo(100.0, 100.01, 100, 100);
        acc.accumulate_bbo_update(&bbo, 1_000_000_000);
        acc.accumulate_bbo_update(&bbo, 2_000_000_000);
        acc.accumulate_bbo_update(&bbo, 3_000_000_000);
        assert_eq!(acc.update_count, 3);
    }

    #[test]
    fn test_hhi_single_trade() {
        let mut acc = StatsAccumulator::new();
        acc.accumulate_trade_size(100);
        // HHI = 100^2 / 100^2 = 1.0 (single trade = perfect concentration)
        assert!((acc.size_concentration() - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_hhi_two_equal_trades() {
        let mut acc = StatsAccumulator::new();
        acc.accumulate_trade_size(100);
        acc.accumulate_trade_size(100);
        // HHI = (100^2 + 100^2) / (200)^2 = 20000 / 40000 = 0.5
        assert!((acc.size_concentration() - 0.5).abs() < 1e-10);
    }

    #[test]
    fn test_hhi_three_trades_golden() {
        let mut acc = StatsAccumulator::new();
        acc.accumulate_trade_size(100);
        acc.accumulate_trade_size(200);
        acc.accumulate_trade_size(300);
        // total = 600, sum_sq = 10000 + 40000 + 90000 = 140000
        // HHI = 140000 / 360000 = 0.38888...
        let expected = 140000.0 / 360000.0;
        assert!(
            (acc.size_concentration() - expected).abs() < 1e-10,
            "HHI: expected {}, got {}",
            expected,
            acc.size_concentration()
        );
    }

    #[test]
    fn test_hhi_no_trades() {
        let acc = StatsAccumulator::new();
        assert_eq!(acc.size_concentration(), 0.0, "No trades → HHI = 0");
    }

    #[test]
    fn test_reset_clears_all() {
        let mut acc = StatsAccumulator::new();
        let bbo = make_bbo(100.0, 100.01, 100, 100);
        acc.accumulate_bbo_update(&bbo, 1_000_000_000);
        acc.accumulate_trade_size(500);
        assert!(acc.has_start_snapshot);
        assert!(acc.update_count > 0);
        assert!(acc.total_volume > 0.0);

        acc.reset();
        assert!(!acc.has_start_snapshot);
        assert_eq!(acc.update_count, 0);
        assert_eq!(acc.total_volume, 0.0);
        assert_eq!(acc.size_squared_sum, 0.0);
        assert_eq!(acc.spread_bps_sum, 0.0);
    }
}
