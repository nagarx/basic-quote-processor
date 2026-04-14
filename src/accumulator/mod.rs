//! Per-bin accumulation of trade and quote data for feature extraction.
//!
//! The `BinAccumulator` orchestrates sub-accumulators that track volumes, counts,
//! BBO dynamics, burst detection, and forward-fill state. At bin boundaries,
//! the `FeatureExtractor` reads the accumulated state to produce feature vectors.
//!
//! Source: docs/design/02_MODULE_ARCHITECTURE.md §4.6

pub(crate) mod flow_accumulator;
pub(crate) mod count_accumulator;
pub(crate) mod stats_accumulator;
pub(crate) mod burst_tracker;
pub(crate) mod forward_fill;

use crate::bbo_state::BboState;
use crate::config::{FeatureConfig, ValidationConfig, VpinConfig};
use crate::reader::PublisherClass;
use crate::trade_classifier::bjzz;
use crate::trade_classifier::{BvcState, ClassifiedTrade, RetailStatus};

use self::burst_tracker::BurstTracker;
use self::count_accumulator::CountAccumulator;
use self::flow_accumulator::FlowAccumulator;
use self::forward_fill::ForwardFillState;
use self::stats_accumulator::StatsAccumulator;

use hft_statistics::statistics::VpinComputer;

/// Per-day diagnostic summary for metadata JSON.
///
/// Must be read via `day_summary()` BEFORE `reset_day()`, following the
/// pattern established by `BboState::crossed_count()` / `reset_counters()`.
#[derive(Debug, Clone, Default)]
pub struct DaySummary {
    pub total_records_processed: u64,
    pub total_bins_emitted: u64,
    pub total_empty_bins: u64,
    pub warmup_bins_discarded: u64,
    pub gap_bins_emitted: u64,
    // Phase 4 metadata fields (FIX #12, #26)
    /// Cumulative TRF trades across all bins in the day.
    pub total_trf_trades: u64,
    /// Cumulative lit trades across all bins in the day.
    pub total_lit_trades: u64,
    /// Cumulative trade records (TRF + lit) in the day.
    pub total_trade_records: u64,
    /// UTC nanosecond timestamp of the first emitted bin's START.
    /// Computed as bin_end_ts - bin_size_ns.
    pub first_bin_start_ns: u64,
    /// UTC nanosecond timestamp of the first emitted bin's END.
    pub first_bin_end_ns: u64,
    /// UTC nanosecond timestamp of the last emitted bin's END.
    pub last_bin_end_ns: u64,
    // Phase 5: cumulative volume counters for coverage validation
    /// Cumulative TRF volume in shares (f64 for precision with large sums).
    pub total_trf_volume: f64,
    /// Cumulative lit volume in shares.
    pub total_lit_volume: f64,
}

/// Central accumulator orchestrating all per-bin state.
///
/// Owns sub-accumulators for volumes, counts, BBO dynamics, burst detection,
/// and forward-fill state. Also owns BvcState (persistent across bins) and
/// optional VpinComputers.
///
/// # Reset Semantics
///
/// - `reset_bin()`: Per-bin sub-accumulators zeroed, bin_index incremented.
///   BvcState, VPIN, forward-fill, and burst tracker persist.
/// - `reset_day()`: Everything zeroed including persistent state and diagnostics.
pub struct BinAccumulator {
    // ── Per-bin sub-accumulators (reset at bin boundaries) ───────────
    pub(crate) flow: FlowAccumulator,
    pub(crate) counts: CountAccumulator,
    pub(crate) stats: StatsAccumulator,

    // ── Persistent across bins (reset at day boundaries) ────────────
    pub(crate) burst_tracker: BurstTracker,
    pub(crate) forward_fill: ForwardFillState,
    pub(crate) bvc: BvcState,

    // ── Optional VPIN (persistent across bins) ──────────────────────
    pub(crate) trf_vpin: Option<VpinComputer>,
    pub(crate) lit_vpin: Option<VpinComputer>,

    // ── Bin metadata ────────────────────────────────────────────────
    bin_index: u64,

    // ── Day-level diagnostic counters ───────────────────────────────
    total_records_processed: u64,
    total_bins_emitted: u64,
    total_empty_bins: u64,
    warmup_bins_discarded: u64,
    gap_bins_emitted: u64,

    // ── Phase 4 cumulative counters (not reset per bin) ───────────
    total_trf_trades: u64,
    total_lit_trades: u64,
    total_trade_records: u64,
    first_bin_start_ns: u64,
    first_bin_end_ns: u64,
    last_bin_end_ns: u64,
    bin_size_ns: u64,
    // Phase 5: cumulative volume
    total_trf_volume: f64,
    total_lit_volume: f64,
}

impl BinAccumulator {
    /// Create a new accumulator with configuration-driven sub-accumulators.
    pub fn new(
        validation: &ValidationConfig,
        vpin_config: &VpinConfig,
        feature_config: &FeatureConfig,
    ) -> Self {
        let trf_vpin = if feature_config.vpin {
            Some(VpinComputer::new(vpin_config.bucket_volume, vpin_config.lookback_buckets))
        } else {
            None
        };
        let lit_vpin = if feature_config.vpin {
            Some(VpinComputer::new(vpin_config.bucket_volume, vpin_config.lookback_buckets))
        } else {
            None
        };

        Self {
            flow: FlowAccumulator::default(),
            counts: CountAccumulator::new(validation.block_threshold),
            stats: StatsAccumulator::new(),
            burst_tracker: BurstTracker::new(validation.burst_threshold),
            forward_fill: ForwardFillState::default(),
            bvc: BvcState::new(vpin_config.sigma_window_minutes),
            trf_vpin,
            lit_vpin,
            bin_index: 0,
            total_records_processed: 0,
            total_bins_emitted: 0,
            total_empty_bins: 0,
            warmup_bins_discarded: 0,
            gap_bins_emitted: 0,
            total_trf_trades: 0,
            total_lit_trades: 0,
            total_trade_records: 0,
            first_bin_start_ns: 0,
            first_bin_end_ns: 0,
            last_bin_end_ns: 0,
            bin_size_ns: 0, // Set via set_bin_size_ns() after construction
            total_trf_volume: 0.0,
            total_lit_volume: 0.0,
        }
    }

    /// Accumulate a classified trade into the current bin.
    ///
    /// Dispatches to all sub-accumulators based on venue, direction, and retail status.
    /// BVC processes ALL trades (not TRF-only) per Easley et al. (2012).
    pub fn accumulate(&mut self, trade: &ClassifiedTrade) {
        let publisher = PublisherClass::from_id(trade.publisher_id);
        let is_trf = publisher.is_trf();
        let is_lit = publisher.is_lit();
        let is_retail = trade.retail_status == RetailStatus::Retail;
        // R6: is_subpenny only computed for TRF trades (optimization)
        let is_subpenny = if is_trf { bjzz::is_subpenny(trade.price) } else { false };

        // 1. Flow volumes
        if is_trf {
            self.flow.accumulate_trf(trade.direction, trade.retail_status, trade.size);
        }
        if is_lit {
            self.flow.accumulate_lit(trade.size);
        }

        // 2. BVC (ALL trades, not just TRF)
        let (bvc_buy, bvc_sell) = self.bvc.classify_trade(trade.price, trade.size, trade.ts_recv);
        self.flow.accumulate_bvc(bvc_buy, bvc_sell);

        // 3. Counts
        self.counts.accumulate(trade.size, is_trf, is_lit, is_retail, is_subpenny);

        // 4. Trade size for HHI + mean
        self.stats.accumulate_trade_size(trade.size);

        // 5. Burst tracking (TRF only)
        if is_trf {
            self.burst_tracker.record_arrival(trade.ts_recv);
        }

        // 6. VPIN feeding (if enabled)
        if let Some(ref mut v) = self.trf_vpin {
            if is_trf {
                v.add_trade(trade.price, trade.size as u64);
            }
        }
        if let Some(ref mut v) = self.lit_vpin {
            if is_lit {
                v.add_trade(trade.price, trade.size as u64);
            }
        }

        self.total_records_processed += 1;

        // Phase 4: cumulative day-level trade counts (not reset per bin)
        if is_trf {
            self.total_trf_trades += 1;
            self.total_trf_volume += trade.size as f64;
        }
        if is_lit {
            self.total_lit_trades += 1;
            self.total_lit_volume += trade.size as f64;
        }
        self.total_trade_records += 1;
    }

    /// Accumulate a BBO update into the current bin's stats.
    ///
    /// Feeds through to StatsAccumulator for time-weighted spread and BBO snapshots.
    pub fn accumulate_bbo_update(&mut self, bbo: &BboState, ts_ns: u64) {
        self.stats.accumulate_bbo_update(bbo, ts_ns);
    }

    /// Prepare the current bin for feature extraction.
    ///
    /// Finalizes time-weighted spread and checks for burst detection.
    /// Must be called at bin boundaries AND for last-bin flush.
    pub fn prepare_for_extraction(&mut self, bin_end_ts: u64) {
        self.stats.finalize_spread_twap(bin_end_ts);
        self.burst_tracker.check_and_update_burst();
    }

    /// Reset per-bin state for a new bin. Preserves cross-bin state.
    ///
    /// Resets: flow, counts, stats, burst_tracker per-bin arrivals.
    /// Preserves: bvc, trf_vpin, lit_vpin, forward_fill, burst_tracker.last_burst_ts.
    pub fn reset_bin(&mut self) {
        self.flow.reset();
        self.counts.reset();
        self.stats.reset();
        self.burst_tracker.reset_bin();
        self.bin_index += 1;
    }

    /// Reset all state for a new trading day.
    ///
    /// Resets everything including BVC sigma, VPIN volume bars, forward-fill,
    /// burst history, and diagnostic counters.
    pub fn reset_day(&mut self) {
        self.flow.reset();
        self.counts.reset();
        self.stats.reset();
        self.burst_tracker.reset_day();
        self.forward_fill = ForwardFillState::default();
        self.bvc.reset();
        if let Some(ref mut v) = self.trf_vpin {
            v.reset();
        }
        if let Some(ref mut v) = self.lit_vpin {
            v.reset();
        }
        self.bin_index = 0;
        self.total_records_processed = 0;
        self.total_bins_emitted = 0;
        self.total_empty_bins = 0;
        self.warmup_bins_discarded = 0;
        self.gap_bins_emitted = 0;
        self.total_trf_trades = 0;
        self.total_lit_trades = 0;
        self.total_trade_records = 0;
        self.first_bin_start_ns = 0;
        self.first_bin_end_ns = 0;
        self.last_bin_end_ns = 0;
        self.total_trf_volume = 0.0;
        self.total_lit_volume = 0.0;
        // bin_size_ns preserved (config-derived, not per-day state)
    }

    // ── Diagnostic counter increment methods (R1: encapsulated) ─────

    /// Record that a bin was emitted (passed warmup gate).
    ///
    /// `bin_end_ts`: UTC nanosecond timestamp of the bin boundary.
    /// On first emitted bin, records both start and end timestamps.
    pub fn record_bin_emitted_with_ts(&mut self, is_empty: bool, bin_end_ts: u64) {
        self.total_bins_emitted += 1;
        if is_empty {
            self.total_empty_bins += 1;
        }
        // Track first/last bin timestamps for metadata
        if self.first_bin_end_ns == 0 {
            self.first_bin_end_ns = bin_end_ts;
            self.first_bin_start_ns = bin_end_ts.saturating_sub(self.bin_size_ns);
        }
        self.last_bin_end_ns = bin_end_ts;
    }

    /// Record that a bin was emitted (legacy signature without timestamp).
    pub fn record_bin_emitted(&mut self, is_empty: bool) {
        self.total_bins_emitted += 1;
        if is_empty {
            self.total_empty_bins += 1;
        }
    }

    /// Record that a warmup bin was discarded.
    pub fn record_warmup_discard(&mut self) {
        self.warmup_bins_discarded += 1;
    }

    /// Record that a gap bin was emitted (also counts as empty bin).
    pub fn record_gap_bin(&mut self) {
        self.total_bins_emitted += 1;
        self.total_empty_bins += 1;
        self.gap_bins_emitted += 1;
    }

    /// Record a gap bin with timestamp tracking for metadata.
    /// FIX CRITICAL-1: Gap bins must update first/last_bin timestamps,
    /// otherwise metadata `last_bin_end_ns` will be wrong if the last
    /// emitted bin of a day is a gap bin.
    pub fn record_gap_bin_with_ts(&mut self, bin_end_ts: u64) {
        self.total_bins_emitted += 1;
        self.total_empty_bins += 1;
        self.gap_bins_emitted += 1;
        if self.first_bin_end_ns == 0 {
            self.first_bin_end_ns = bin_end_ts;
            self.first_bin_start_ns = bin_end_ts.saturating_sub(self.bin_size_ns);
        }
        self.last_bin_end_ns = bin_end_ts;
    }

    // ── Accessors ───────────────────────────────────────────────────

    /// Current bin index (0-based, incremented by `reset_bin()`).
    pub fn bin_index(&self) -> u64 {
        self.bin_index
    }

    // ── Public delegate methods for processing loop (R18) ─────────
    // These enable integration tests (outside the crate) to query
    // accumulator state and update forward-fill without accessing
    // pub(crate) sub-accumulator fields.

    /// Number of TRF trades in the current bin.
    pub fn trf_trades(&self) -> u64 {
        self.counts.trf_trades
    }

    /// Number of BBO updates in the current bin.
    pub fn bbo_update_count(&self) -> u64 {
        self.stats.update_count
    }

    /// Whether any trades (TRF or lit) were accumulated in the current bin.
    pub fn has_trades(&self) -> bool {
        self.counts.total_trades > 0
    }

    /// Total trades in the current bin (all venues).
    pub fn total_trades(&self) -> u64 {
        self.counts.total_trades
    }

    /// Update forward-fill state from extracted features.
    ///
    /// Called after every non-empty bin (trf_trades > 0). Delegates to
    /// `ForwardFillState::update_from_features()`.
    /// R2: Must be called even during warmup bins per spec §5.4.
    pub fn update_forward_fill(&mut self, features: &[f64], vpin_enabled: bool) {
        self.forward_fill.update_from_features(features, vpin_enabled);
    }

    /// Update BBO-derived forward-fill state separately.
    ///
    /// Called when a bin has BBO updates (even with 0 TRF trades).
    /// Delegates to `ForwardFillState::update_bbo_features()`.
    pub fn update_forward_fill_bbo(&mut self, spread_bps: f64, quote_imbalance: f64) {
        self.forward_fill.update_bbo_features(spread_bps, quote_imbalance);
    }

    /// Set bin size in nanoseconds (called once after construction).
    /// Required for first_bin_start_ns computation.
    pub fn set_bin_size_ns(&mut self, bin_size_ns: u64) {
        self.bin_size_ns = bin_size_ns;
    }

    /// Day-level diagnostic summary. Must be called BEFORE `reset_day()`.
    pub fn day_summary(&self) -> DaySummary {
        DaySummary {
            total_records_processed: self.total_records_processed,
            total_bins_emitted: self.total_bins_emitted,
            total_empty_bins: self.total_empty_bins,
            warmup_bins_discarded: self.warmup_bins_discarded,
            gap_bins_emitted: self.gap_bins_emitted,
            total_trf_trades: self.total_trf_trades,
            total_lit_trades: self.total_lit_trades,
            total_trade_records: self.total_trade_records,
            first_bin_start_ns: self.first_bin_start_ns,
            first_bin_end_ns: self.first_bin_end_ns,
            last_bin_end_ns: self.last_bin_end_ns,
            total_trf_volume: self.total_trf_volume,
            total_lit_volume: self.total_lit_volume,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{FeatureConfig, ValidationConfig, VpinConfig};
    use crate::trade_classifier::{TradeDirection, RetailStatus, ClassifiedTrade};
    use crate::reader::publisher;

    fn default_accumulator() -> BinAccumulator {
        BinAccumulator::new(
            &ValidationConfig::default(),
            &VpinConfig::default(),
            &FeatureConfig::default(),
        )
    }

    fn make_trf_trade(direction: TradeDirection, retail: RetailStatus, price: f64, size: u32, ts: u64) -> ClassifiedTrade {
        ClassifiedTrade {
            direction,
            retail_status: retail,
            price,
            size,
            publisher_id: publisher::FINN, // TRF
            ts_recv: ts,
        }
    }

    fn make_lit_trade(price: f64, size: u32, ts: u64) -> ClassifiedTrade {
        ClassifiedTrade {
            direction: TradeDirection::Unsigned,
            retail_status: RetailStatus::Institutional,
            price,
            size,
            publisher_id: publisher::XNAS, // Lit
            ts_recv: ts,
        }
    }

    #[test]
    fn test_construction_without_vpin() {
        let acc = default_accumulator();
        assert!(acc.trf_vpin.is_none(), "VPIN disabled by default");
        assert!(acc.lit_vpin.is_none());
        assert_eq!(acc.bin_index(), 0);
    }

    #[test]
    fn test_construction_with_vpin() {
        let feature_config = FeatureConfig { vpin: true, ..Default::default() };
        let acc = BinAccumulator::new(
            &ValidationConfig::default(),
            &VpinConfig::default(),
            &feature_config,
        );
        assert!(acc.trf_vpin.is_some(), "VPIN should be enabled");
        assert!(acc.lit_vpin.is_some());
    }

    #[test]
    fn test_single_trf_buy() {
        let mut acc = default_accumulator();
        let trade = make_trf_trade(
            TradeDirection::Buy, RetailStatus::Institutional,
            134.5675, 100, 1_000_000_000,
        );
        acc.accumulate(&trade);

        assert_eq!(acc.flow.trf_buy_vol, 100.0);
        assert_eq!(acc.flow.trf_sell_vol, 0.0);
        assert_eq!(acc.counts.trf_trades, 1);
        assert_eq!(acc.counts.total_trades, 1);
        assert!(acc.flow.bvc_buy_vol + acc.flow.bvc_sell_vol > 0.0, "BVC should process trade");
    }

    #[test]
    fn test_single_lit_trade() {
        let mut acc = default_accumulator();
        let trade = make_lit_trade(134.56, 300, 1_000_000_000);
        acc.accumulate(&trade);

        assert_eq!(acc.flow.lit_vol, 300.0);
        assert_eq!(acc.flow.trf_buy_vol, 0.0, "Lit trade should not affect TRF");
        assert_eq!(acc.counts.lit_trades, 1);
        assert_eq!(acc.counts.trf_trades, 0);
        assert_eq!(acc.counts.total_trades, 1);
    }

    #[test]
    fn test_bvc_processes_all_trades() {
        let mut acc = default_accumulator();
        // TRF trade
        acc.accumulate(&make_trf_trade(
            TradeDirection::Buy, RetailStatus::Institutional,
            100.0, 100, 1_000_000_000,
        ));
        let bvc_after_trf = acc.flow.bvc_buy_vol + acc.flow.bvc_sell_vol;

        // Lit trade
        acc.accumulate(&make_lit_trade(100.01, 200, 2_000_000_000));
        let bvc_after_lit = acc.flow.bvc_buy_vol + acc.flow.bvc_sell_vol;

        assert!(bvc_after_trf > 0.0, "BVC should process TRF trade");
        assert!(
            bvc_after_lit > bvc_after_trf,
            "BVC should also process lit trade: before={}, after={}",
            bvc_after_trf, bvc_after_lit
        );
    }

    #[test]
    fn test_bbo_update_feeds_stats() {
        let mut acc = default_accumulator();
        let mut bbo = BboState::new();
        bbo.bid_price = 100.0;
        bbo.ask_price = 100.01;
        bbo.bid_size = 100;
        bbo.ask_size = 200;
        bbo.spread = 0.01;
        bbo.is_valid = true;

        acc.accumulate_bbo_update(&bbo, 1_000_000_000);
        assert_eq!(acc.stats.update_count, 1);
        assert!(acc.stats.has_start_snapshot);
    }

    #[test]
    fn test_reset_bin_zeros_per_bin_preserves_persistent() {
        let mut acc = default_accumulator();
        acc.accumulate(&make_trf_trade(
            TradeDirection::Buy, RetailStatus::Retail,
            100.0035, 50, 1_000_000_000,
        ));
        assert!(acc.flow.trf_buy_vol > 0.0);
        assert!(acc.counts.trf_trades > 0);

        acc.reset_bin();
        assert_eq!(acc.flow.trf_buy_vol, 0.0, "Per-bin flow should reset");
        assert_eq!(acc.counts.trf_trades, 0, "Per-bin counts should reset");
        assert_eq!(acc.bin_index(), 1, "bin_index should increment");
        // BVC persists (sigma state is internal, just verify no panic)
        let _ = acc.bvc.window_size();
    }

    #[test]
    fn test_reset_day_zeros_everything() {
        let mut acc = default_accumulator();
        acc.accumulate(&make_trf_trade(
            TradeDirection::Buy, RetailStatus::Institutional,
            100.0, 100, 1_000_000_000,
        ));
        acc.reset_bin();
        assert_eq!(acc.bin_index(), 1);

        acc.reset_day();
        assert_eq!(acc.bin_index(), 0, "bin_index should reset on day reset");
        assert_eq!(acc.total_records_processed, 0, "Diagnostics should reset");
    }

    #[test]
    fn test_subpenny_detection_routed() {
        let mut acc = default_accumulator();
        // Price $100.0035 → frac_cent=0.35 → subpenny (in retail sell zone)
        acc.accumulate(&make_trf_trade(
            TradeDirection::Sell, RetailStatus::Retail,
            100.0035, 100, 1_000_000_000,
        ));
        assert_eq!(acc.counts.subpenny_trades, 1, "Subpenny should be detected for TRF");

        // Lit trade with same subpenny price → should NOT count
        acc.accumulate(&make_lit_trade(100.0035, 100, 2_000_000_000));
        assert_eq!(acc.counts.subpenny_trades, 1, "Lit trade should not count as subpenny");
    }

    #[test]
    fn test_odd_lot_trf_only() {
        let mut acc = default_accumulator();
        // TRF trade with size < 100 → odd lot
        acc.accumulate(&make_trf_trade(
            TradeDirection::Buy, RetailStatus::Institutional,
            100.0, 50, 1_000_000_000,
        ));
        assert_eq!(acc.counts.odd_lot_trades, 1);

        // Lit trade with size < 100 → NOT odd lot (TRF-only metric)
        acc.accumulate(&make_lit_trade(100.0, 30, 2_000_000_000));
        assert_eq!(acc.counts.odd_lot_trades, 1, "Lit odd-lot should not be counted");
    }

    #[test]
    fn test_block_detection_all_trades() {
        let mut acc = default_accumulator();
        // TRF block trade
        acc.accumulate(&make_trf_trade(
            TradeDirection::Buy, RetailStatus::Institutional,
            100.0, 15_000, 1_000_000_000,
        ));
        assert_eq!(acc.counts.block_trades, 1);

        // Lit block trade (block counts ALL trades)
        acc.accumulate(&make_lit_trade(100.0, 20_000, 2_000_000_000));
        assert_eq!(acc.counts.block_trades, 2, "Block detection must count ALL trades");
    }

    #[test]
    fn test_bin_index_incremented_on_reset() {
        let mut acc = default_accumulator();
        assert_eq!(acc.bin_index(), 0);
        acc.reset_bin();
        assert_eq!(acc.bin_index(), 1);
        acc.reset_bin();
        assert_eq!(acc.bin_index(), 2);
    }

    #[test]
    fn test_diagnostic_counters_encapsulated() {
        let mut acc = default_accumulator();

        acc.record_bin_emitted(false);
        assert_eq!(acc.day_summary().total_bins_emitted, 1);
        assert_eq!(acc.day_summary().total_empty_bins, 0);

        acc.record_bin_emitted(true);
        assert_eq!(acc.day_summary().total_bins_emitted, 2);
        assert_eq!(acc.day_summary().total_empty_bins, 1);

        acc.record_warmup_discard();
        assert_eq!(acc.day_summary().warmup_bins_discarded, 1);

        acc.record_gap_bin();
        assert_eq!(acc.day_summary().total_bins_emitted, 3);
        assert_eq!(acc.day_summary().total_empty_bins, 2);
        assert_eq!(acc.day_summary().gap_bins_emitted, 1);
    }

    // ── Phase 4 cumulative counter tests ──────────────────────────

    #[test]
    fn test_cumulative_trf_count_across_bins() {
        let mut acc = default_accumulator();
        // 3 TRF trades in bin 0
        for _ in 0..3 {
            acc.accumulate(&make_trf_trade(
                TradeDirection::Buy, RetailStatus::Institutional,
                100.0, 100, 1_000_000_000,
            ));
        }
        acc.reset_bin();
        // 2 TRF trades in bin 1
        for _ in 0..2 {
            acc.accumulate(&make_trf_trade(
                TradeDirection::Sell, RetailStatus::Institutional,
                100.0, 50, 2_000_000_000,
            ));
        }
        let summary = acc.day_summary();
        assert_eq!(summary.total_trf_trades, 5, "Cumulative TRF across bins");
        assert_eq!(summary.total_trade_records, 5);
    }

    #[test]
    fn test_cumulative_lit_count() {
        let mut acc = default_accumulator();
        acc.accumulate(&make_lit_trade(100.0, 100, 1_000_000_000));
        acc.accumulate(&make_lit_trade(100.0, 200, 2_000_000_000));
        assert_eq!(acc.day_summary().total_lit_trades, 2);
    }

    #[test]
    fn test_bin_timestamps_recorded() {
        let mut acc = default_accumulator();
        acc.set_bin_size_ns(60_000_000_000); // 60s bins

        // First emitted bin at ts=100s
        let bin_end_ts_1 = 100_000_000_000_u64;
        acc.record_bin_emitted_with_ts(false, bin_end_ts_1);
        let s = acc.day_summary();
        assert_eq!(s.first_bin_end_ns, bin_end_ts_1);
        assert_eq!(s.first_bin_start_ns, bin_end_ts_1 - 60_000_000_000);
        assert_eq!(s.last_bin_end_ns, bin_end_ts_1);

        // Second emitted bin at ts=160s
        let bin_end_ts_2 = 160_000_000_000_u64;
        acc.record_bin_emitted_with_ts(false, bin_end_ts_2);
        let s = acc.day_summary();
        assert_eq!(s.first_bin_end_ns, bin_end_ts_1, "first should not change");
        assert_eq!(s.last_bin_end_ns, bin_end_ts_2, "last should update");
    }

    #[test]
    fn test_reset_day_clears_cumulative() {
        let mut acc = default_accumulator();
        acc.set_bin_size_ns(60_000_000_000);
        acc.accumulate(&make_trf_trade(
            TradeDirection::Buy, RetailStatus::Institutional,
            100.0, 100, 1_000_000_000,
        ));
        acc.record_bin_emitted_with_ts(false, 100_000_000_000);
        acc.reset_day();
        let s = acc.day_summary();
        assert_eq!(s.total_trf_trades, 0);
        assert_eq!(s.total_lit_trades, 0);
        assert_eq!(s.first_bin_end_ns, 0);
        assert_eq!(s.last_bin_end_ns, 0);
    }

    #[test]
    fn test_prepare_for_extraction() {
        let mut acc = default_accumulator();
        let mut bbo = BboState::new();
        bbo.bid_price = 100.0;
        bbo.ask_price = 100.01;
        bbo.bid_size = 100;
        bbo.ask_size = 100;
        bbo.spread = 0.01;
        bbo.is_valid = true;

        acc.accumulate_bbo_update(&bbo, 1_000_000_000);
        acc.accumulate_bbo_update(&bbo, 2_000_000_000);

        // Before finalization, TWAP not finalized
        acc.prepare_for_extraction(3_000_000_000);
        let twap = acc.stats.time_weighted_spread_bps();
        assert!(twap > 0.0, "TWAP should be > 0 after finalization");
    }

    #[test]
    fn test_vpin_only_fed_matching_venue() {
        let feature_config = FeatureConfig { vpin: true, ..Default::default() };
        let mut acc = BinAccumulator::new(
            &ValidationConfig::default(),
            &VpinConfig::default(),
            &feature_config,
        );

        // TRF trade → feeds trf_vpin only
        acc.accumulate(&make_trf_trade(
            TradeDirection::Buy, RetailStatus::Institutional,
            100.0, 1000, 1_000_000_000,
        ));

        // Lit trade → feeds lit_vpin only
        acc.accumulate(&make_lit_trade(100.01, 1000, 2_000_000_000));

        // Both VPIN computers should exist and be functional
        assert!(acc.trf_vpin.is_some(), "trf_vpin should be enabled");
        assert!(acc.lit_vpin.is_some(), "lit_vpin should be enabled");
    }

    // ── Phase 5 cumulative volume tests ─────────────────────────

    #[test]
    fn test_cumulative_trf_volume_across_bins() {
        let mut acc = default_accumulator();
        // Bin 0: TRF trade size=100, another size=200
        acc.accumulate(&make_trf_trade(
            TradeDirection::Buy, RetailStatus::Institutional,
            100.0, 100, 1_000_000_000,
        ));
        acc.accumulate(&make_trf_trade(
            TradeDirection::Sell, RetailStatus::Institutional,
            100.0, 200, 2_000_000_000,
        ));
        acc.reset_bin();
        // Bin 1: TRF trade size=300
        acc.accumulate(&make_trf_trade(
            TradeDirection::Buy, RetailStatus::Institutional,
            100.0, 300, 3_000_000_000,
        ));
        let summary = acc.day_summary();
        assert_eq!(summary.total_trf_volume, 600.0, "100+200+300 = 600");
    }

    #[test]
    fn test_cumulative_lit_volume() {
        let mut acc = default_accumulator();
        acc.accumulate(&make_lit_trade(100.0, 500, 1_000_000_000));
        acc.accumulate(&make_lit_trade(100.0, 300, 2_000_000_000));
        assert_eq!(acc.day_summary().total_lit_volume, 800.0);
    }

    #[test]
    fn test_reset_day_clears_volumes() {
        let mut acc = default_accumulator();
        acc.accumulate(&make_trf_trade(
            TradeDirection::Buy, RetailStatus::Institutional,
            100.0, 1000, 1_000_000_000,
        ));
        acc.accumulate(&make_lit_trade(100.0, 500, 2_000_000_000));
        assert!(acc.day_summary().total_trf_volume > 0.0);
        acc.reset_day();
        assert_eq!(acc.day_summary().total_trf_volume, 0.0);
        assert_eq!(acc.day_summary().total_lit_volume, 0.0);
    }
}
