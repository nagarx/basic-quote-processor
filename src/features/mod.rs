//! Off-exchange feature computation from accumulated bin state.
//!
//! The `FeatureExtractor` reads from `BinAccumulator` state at bin boundaries
//! and produces a 34-element `Vec<f64>` feature vector. It is stateless —
//! all mutable state lives in the accumulator.
//!
//! # Feature Groups
//!
//! | Group | Indices | Toggleable | Default |
//! |-------|---------|------------|---------|
//! | signed_flow | 0-3 | Yes | On |
//! | venue_metrics | 4-7 | Yes | On |
//! | retail_metrics | 8-11 | Yes | On |
//! | bbo_dynamics | 12-17 | Yes | On |
//! | vpin | 18-19 | Yes | Off |
//! | trade_size | 20-23 | Yes | On |
//! | cross_venue | 24-26 | Yes | On |
//! | activity | 27-28 | No (always) | On |
//! | safety_gates | 29-30 | No (always) | On |
//! | context | 31-33 | No (always) | On |
//!
//! Source: docs/design/04_FEATURE_SPECIFICATION.md

pub mod indices;

use crate::accumulator::BinAccumulator;
use crate::bbo_state::{self, BboState};
use crate::config::{FeatureConfig, ValidationConfig};
use crate::contract::{EPS, SCHEMA_VERSION, TOTAL_FEATURES};
use crate::sampling::BinBoundary;

use self::indices::*;

/// Stateless feature extractor that reads accumulated bin state.
///
/// Initialized per-day via `init_day()` to receive market timing parameters
/// from the `TimeBinSampler`. All per-extraction state comes from the
/// `BinAccumulator` and `BboState` references passed to `extract()`.
pub struct FeatureExtractor {
    feature_config: FeatureConfig,
    validation_config: ValidationConfig,
    utc_offset_hours: i32,
    market_open_ns: u64,
    session_duration_ns: u64,
    warmup_bins: u32,
    bin_size_secs: u32,
}

impl FeatureExtractor {
    /// Create a new extractor with feature and validation configuration.
    ///
    /// Must call `init_day()` before `extract()`.
    pub fn new(
        feature_config: &FeatureConfig,
        validation_config: &ValidationConfig,
        bin_size_secs: u32,
    ) -> Self {
        Self {
            feature_config: feature_config.clone(),
            validation_config: validation_config.clone(),
            utc_offset_hours: 0,
            market_open_ns: 0,
            session_duration_ns: 0,
            warmup_bins: validation_config.warmup_bins,
            bin_size_secs,
        }
    }

    /// Initialize for a specific trading day.
    ///
    /// Receives market timing from the `TimeBinSampler::init_day()` results.
    /// Must be called before `extract()`.
    pub fn init_day(&mut self, utc_offset_hours: i32, market_open_ns: u64, session_end_ns: u64) {
        self.utc_offset_hours = utc_offset_hours;
        self.market_open_ns = market_open_ns;
        self.session_duration_ns = session_end_ns.saturating_sub(market_open_ns);
    }

    /// Update session end for half-day detection (Phase 5 integration).
    pub fn set_session_end(&mut self, end_ns: u64) {
        self.session_duration_ns = end_ns.saturating_sub(self.market_open_ns);
    }

    /// Extract a 34-element feature vector from accumulated bin state.
    ///
    /// The output vector is always `TOTAL_FEATURES` (34) elements. Disabled groups
    /// produce zeros at their indices. After group extraction, the 3-level empty
    /// bin policy applies forward-fill for state features when no TRF trades exist.
    ///
    /// # Post-Extraction Invariants (FIX #12)
    ///
    /// - All values are finite (NaN/Inf = panic)
    /// - Safety gates are exactly 0.0 or 1.0
    /// - Schema version equals `SCHEMA_VERSION`
    /// - Session progress is in [0.0, 1.0]
    pub fn extract(
        &self,
        acc: &BinAccumulator,
        bbo: &BboState,
        boundary: &BinBoundary,
        output: &mut Vec<f64>,
    ) {
        debug_assert!(
            self.session_duration_ns > 0,
            "FeatureExtractor::init_day() must be called before extract()"
        );

        output.clear();
        output.resize(TOTAL_FEATURES, 0.0);

        let has_trf_trades = acc.counts.trf_trades > 0;
        let had_bbo_updates = acc.stats.update_count > 0;

        // ── Optional groups (produce zeros when disabled) ───────────
        if self.feature_config.signed_flow {
            self.extract_signed_flow(acc, output);
        }
        if self.feature_config.venue_metrics {
            self.extract_venue_metrics(acc, output);
        }
        if self.feature_config.retail_metrics {
            self.extract_retail_metrics(acc, output);
        }
        if self.feature_config.bbo_dynamics {
            self.extract_bbo_dynamics(acc, bbo, output);
        }
        if self.feature_config.vpin {
            self.extract_vpin(acc, output);
        }
        if self.feature_config.trade_size {
            self.extract_trade_size(acc, output);
        }
        if self.feature_config.cross_venue {
            self.extract_cross_venue(acc, output);
        }

        // ── Always-enabled groups ───────────────────────────────────
        self.extract_activity(acc, output);
        self.extract_safety_gates(acc, bbo, boundary.bin_end_ts, output);
        self.extract_context(boundary.bin_midpoint_ts, output);

        // ── Always-computed state (not forward-filled) ──────────────
        // time_since_burst uses persistent burst tracker, recomputes every bin.
        // time_since_burst uses bin_end_ts per spec: "(bin_end_time - last_burst_time)"
        if self.feature_config.cross_venue {
            output[TIME_SINCE_BURST] = acc.burst_tracker.time_since_burst_secs(
                boundary.bin_end_ts,
                self.warmup_bins,
                self.bin_size_secs,
            );
        }

        // ── 3-level empty bin policy (FIX #2) ──────────────────────
        if !has_trf_trades {
            acc.forward_fill.apply_to(output, self.feature_config.vpin, had_bbo_updates);
        }

        // ── Post-extraction invariant checks (FIX #12, R20) ────────
        assert!(
            output.iter().all(|v| v.is_finite()),
            "Non-finite feature value detected at bin {}",
            boundary.bin_index
        );
        debug_assert!(
            output[BIN_VALID] == 0.0 || output[BIN_VALID] == 1.0,
            "bin_valid must be 0.0 or 1.0, got {}",
            output[BIN_VALID]
        );
        debug_assert!(
            output[BBO_VALID] == 0.0 || output[BBO_VALID] == 1.0,
            "bbo_valid must be 0.0 or 1.0, got {}",
            output[BBO_VALID]
        );
        debug_assert_eq!(output[SCHEMA_VERSION_IDX], SCHEMA_VERSION);
        debug_assert!(
            output[SESSION_PROGRESS] >= 0.0 && output[SESSION_PROGRESS] <= 1.0,
            "session_progress out of [0,1]: {}",
            output[SESSION_PROGRESS]
        );
    }

    // ── Per-group extraction methods (private) ──────────────────────

    /// Signed Flow (indices 0-3).
    /// Source: 04_FEATURE_SPECIFICATION.md §5.1
    fn extract_signed_flow(&self, acc: &BinAccumulator, out: &mut [f64]) {
        let buy = acc.flow.trf_buy_vol;
        let sell = acc.flow.trf_sell_vol;
        out[TRF_SIGNED_IMBALANCE] = (buy - sell) / (buy + sell).max(EPS);

        let r_buy = acc.flow.retail_buy_vol;
        let r_sell = acc.flow.retail_sell_vol;
        out[MROIB] = (r_buy - r_sell) / (r_buy + r_sell).max(EPS);

        out[INV_INST_DIRECTION] = -out[MROIB];

        let bvc_buy = acc.flow.bvc_buy_vol;
        let bvc_sell = acc.flow.bvc_sell_vol;
        out[BVC_IMBALANCE] = (bvc_buy - bvc_sell) / (bvc_buy + bvc_sell).max(EPS);
    }

    /// Venue Metrics (indices 4-7).
    /// Source: 04_FEATURE_SPECIFICATION.md §5.2
    fn extract_venue_metrics(&self, acc: &BinAccumulator, out: &mut [f64]) {
        let trf_vol = acc.flow.trf_total_vol();
        let lit_vol = acc.flow.lit_vol;
        out[DARK_SHARE] = trf_vol / (trf_vol + lit_vol).max(EPS);
        out[TRF_VOLUME] = trf_vol;
        out[LIT_VOLUME] = lit_vol;
        out[TOTAL_VOLUME] = trf_vol + lit_vol;
    }

    /// Retail Metrics (indices 8-11). COUNT ratios use max(n, 1) per R14.
    /// Source: 04_FEATURE_SPECIFICATION.md §5.3
    fn extract_retail_metrics(&self, acc: &BinAccumulator, out: &mut [f64]) {
        let n_trf = acc.counts.trf_trades.max(1) as f64;

        // COUNT ratios: max(trf_trades, 1) denominator guard (R14)
        out[SUBPENNY_INTENSITY] = acc.counts.subpenny_trades as f64 / n_trf;
        out[ODD_LOT_RATIO] = acc.counts.odd_lot_trades as f64 / n_trf;
        out[RETAIL_TRADE_RATE] = acc.counts.retail_trades as f64 / n_trf;

        // VOLUME ratio: max(trf_vol, EPS) denominator guard
        let trf_vol = acc.flow.trf_total_vol();
        out[RETAIL_VOLUME_FRACTION] = acc.flow.retail_total_vol / trf_vol.max(EPS);
    }

    /// BBO Dynamics (indices 12-17).
    /// Source: 04_FEATURE_SPECIFICATION.md §5.4
    fn extract_bbo_dynamics(&self, acc: &BinAccumulator, bbo: &BboState, out: &mut [f64]) {
        // spread_bps: TWAP with fallback to current BBO (R12)
        let twap = acc.stats.time_weighted_spread_bps();
        out[SPREAD_BPS] = if twap > EPS {
            twap
        } else if bbo.is_valid {
            bbo.spread_bps()
        } else {
            0.0
        };

        // bid/ask pressure: fractional change from bin start (R11: zero-start guard)
        if acc.stats.has_start_snapshot {
            let bid_start = acc.stats.bid_size_start as f64;
            let ask_start = acc.stats.ask_size_start as f64;
            out[BID_PRESSURE] = if acc.stats.bid_size_start == 0 {
                0.0
            } else {
                // bid_start >= 1.0 here (u32 != 0 implies >= 1)
                (bbo.bid_size as f64 - bid_start) / bid_start
            };
            out[ASK_PRESSURE] = if acc.stats.ask_size_start == 0 {
                0.0
            } else {
                // ask_start >= 1.0 here (u32 != 0 implies >= 1)
                (bbo.ask_size as f64 - ask_start) / ask_start
            };
        }
        // else: 0.0 (default from resize)

        out[BBO_UPDATE_RATE] = acc.stats.update_count as f64;

        // quote_imbalance at end of bin
        let bid_sz = bbo.bid_size as f64;
        let ask_sz = bbo.ask_size as f64;
        out[QUOTE_IMBALANCE] = (bid_sz - ask_sz) / (bid_sz + ask_sz).max(EPS);

        // spread_change_rate
        if acc.stats.has_start_snapshot {
            out[SPREAD_CHANGE_RATE] = bbo.spread_bps() - acc.stats.spread_bps_start;
        }
    }

    /// VPIN (indices 18-19, disabled by default).
    /// Source: 04_FEATURE_SPECIFICATION.md §5.5
    fn extract_vpin(&self, acc: &BinAccumulator, out: &mut [f64]) {
        if let Some(ref v) = acc.trf_vpin {
            out[TRF_VPIN] = v.current_vpin().unwrap_or(0.0);
        }
        if let Some(ref v) = acc.lit_vpin {
            out[LIT_VPIN] = v.current_vpin().unwrap_or(0.0);
        }
    }

    /// Trade Size (indices 20-23).
    /// Source: 04_FEATURE_SPECIFICATION.md §5.6
    fn extract_trade_size(&self, acc: &BinAccumulator, out: &mut [f64]) {
        let n_total = acc.counts.total_trades.max(1) as f64;
        out[MEAN_TRADE_SIZE] = acc.stats.total_volume / n_total;
        out[BLOCK_TRADE_RATIO] = acc.counts.block_trades as f64 / n_total;
        out[TRADE_COUNT] = acc.counts.total_trades as f64;
        out[SIZE_CONCENTRATION] = acc.stats.size_concentration();
    }

    /// Cross-Venue (indices 24, 26 only — time_since_burst at index 25 handled separately).
    /// Source: 04_FEATURE_SPECIFICATION.md §5.7
    fn extract_cross_venue(&self, acc: &BinAccumulator, out: &mut [f64]) {
        out[TRF_BURST_INTENSITY] = acc.burst_tracker.compute_burst_intensity();
        // time_since_burst (25) is always-computed in extract() — NOT here.

        // trf_lit_volume_ratio: state variable, forward-filled on empty bins (R21)
        let trf_vol = acc.flow.trf_total_vol();
        let lit_vol = acc.flow.lit_vol;
        out[TRF_LIT_VOLUME_RATIO] = trf_vol / lit_vol.max(EPS);
    }

    /// Activity (indices 27-28, always enabled).
    fn extract_activity(&self, acc: &BinAccumulator, out: &mut [f64]) {
        out[BIN_TRADE_COUNT] = acc.counts.total_trades as f64;
        out[BIN_TRF_TRADE_COUNT] = acc.counts.trf_trades as f64;
    }

    /// Safety Gates (indices 29-30, always enabled, categorical).
    /// Uses bin_end_ts for staleness per R13.
    /// Source: 04_FEATURE_SPECIFICATION.md §5.9
    fn extract_safety_gates(
        &self,
        acc: &BinAccumulator,
        bbo: &BboState,
        bin_end_ts: u64,
        out: &mut [f64],
    ) {
        out[BIN_VALID] = if acc.counts.trf_trades >= self.validation_config.min_trades_per_bin {
            1.0
        } else {
            0.0
        };

        let staleness = bbo_state::validation::staleness_ns(bbo.last_update_ts, bin_end_ts);
        out[BBO_VALID] = if bbo.is_valid && staleness <= self.validation_config.bbo_staleness_max_ns
        {
            1.0
        } else {
            0.0
        };
    }

    /// Context (indices 31-33, always enabled).
    /// Source: 04_FEATURE_SPECIFICATION.md §5.10
    fn extract_context(&self, bin_midpoint_ts: u64, out: &mut [f64]) {
        // session_progress: clamped [0.0, 1.0]
        let elapsed = bin_midpoint_ts.saturating_sub(self.market_open_ns) as f64;
        out[SESSION_PROGRESS] = (elapsed / self.session_duration_ns as f64).clamp(0.0, 1.0);

        // time_bucket: map 7-regime to 6 buckets
        // R3: time_regime takes i64, our timestamps are u64 — safe cast for 2025-2026
        let regime = hft_statistics::time::regime::time_regime(
            bin_midpoint_ts as i64,
            self.utc_offset_hours,
        );
        // R20: pre-market regime should never occur during session
        debug_assert!(
            (1..=6).contains(&regime),
            "Unexpected pre-market regime ({}) at bin midpoint {}",
            regime,
            bin_midpoint_ts
        );
        out[TIME_BUCKET] = match regime {
            0 => 5.0, // pre-market → post-market bucket (defensive fallback)
            1 => 0.0, // open-auction
            2 => 1.0, // morning
            3 => 2.0, // midday
            4 => 3.0, // afternoon
            5 => 4.0, // close-auction
            6 => 5.0, // post-market
            _ => 5.0, // unreachable
        };

        out[SCHEMA_VERSION_IDX] = SCHEMA_VERSION;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accumulator::BinAccumulator;
    use crate::bbo_state::BboState;
    use crate::config::{FeatureConfig, SamplingConfig, ValidationConfig, VpinConfig};
    use crate::sampling::BinBoundary;
    use crate::trade_classifier::{ClassifiedTrade, RetailStatus, TradeDirection};
    use crate::reader::publisher;

    fn default_extractor() -> FeatureExtractor {
        let mut ext = FeatureExtractor::new(
            &FeatureConfig::default(),
            &ValidationConfig::default(),
            SamplingConfig::default().bin_size_seconds,
        );
        // Init for EST date 2025-02-03 (09:30 EST = 14:30 UTC)
        let date = chrono::NaiveDate::from_ymd_opt(2025, 2, 3).unwrap();
        let midnight = date.and_hms_opt(0, 0, 0).unwrap().and_utc()
            .timestamp_nanos_opt().unwrap() as u64;
        let open = midnight + (14 * 3600 + 30 * 60) * 1_000_000_000;
        let close = open + (6 * 3600 + 30 * 60) * 1_000_000_000;
        ext.init_day(-5, open, close);
        ext
    }

    fn default_accumulator() -> BinAccumulator {
        BinAccumulator::new(
            &ValidationConfig::default(),
            &VpinConfig::default(),
            &FeatureConfig::default(),
        )
    }

    fn make_boundary(bin_end_ts: u64, bin_index: u64) -> BinBoundary {
        BinBoundary {
            bin_end_ts,
            bin_midpoint_ts: bin_end_ts - 30 * 1_000_000_000, // 30s before end (60s bins)
            bin_index,
            gap_bins: 0,
        }
    }

    fn make_bbo(bid: f64, ask: f64, bid_sz: u32, ask_sz: u32) -> BboState {
        let mut bbo = BboState::new();
        bbo.bid_price = bid;
        bbo.ask_price = ask;
        bbo.bid_size = bid_sz;
        bbo.ask_size = ask_sz;
        bbo.mid_price = (bid + ask) / 2.0;
        bbo.spread = ask - bid;
        bbo.is_valid = true;
        bbo.last_update_ts = 1_738_600_000_000_000_000; // within session
        bbo
    }

    #[test]
    fn test_all_zeros_accumulator() {
        let ext = default_extractor();
        let acc = default_accumulator();
        let bbo = make_bbo(100.0, 100.01, 100, 100);
        let boundary = make_boundary(ext.market_open_ns + 60 * 1_000_000_000, 0);
        let mut output = Vec::new();

        ext.extract(&acc, &bbo, &boundary, &mut output);

        assert_eq!(output.len(), TOTAL_FEATURES);
        // Most features should be 0.0 (no trades accumulated)
        assert_eq!(output[TRF_SIGNED_IMBALANCE], 0.0);
        assert_eq!(output[MROIB], 0.0);
        assert_eq!(output[TRF_VOLUME], 0.0);
        assert_eq!(output[TRADE_COUNT], 0.0);
        // Schema version always set
        assert_eq!(output[SCHEMA_VERSION_IDX], 1.0);
        // Session progress should be > 0 (we're past market open)
        assert!(output[SESSION_PROGRESS] > 0.0);
    }

    #[test]
    fn test_signed_flow_golden() {
        let ext = default_extractor();
        let mut acc = default_accumulator();
        let bbo = make_bbo(134.56, 134.57, 100, 100);

        // Buy 500 shares (TRF)
        acc.accumulate(&ClassifiedTrade {
            direction: TradeDirection::Buy,
            retail_status: RetailStatus::Institutional,
            price: 134.5675,
            size: 500,
            publisher_id: publisher::FINN,
            ts_recv: ext.market_open_ns + 10_000_000_000,
        });
        // Sell 300 shares (TRF)
        acc.accumulate(&ClassifiedTrade {
            direction: TradeDirection::Sell,
            retail_status: RetailStatus::Institutional,
            price: 134.5625,
            size: 300,
            publisher_id: publisher::FINN,
            ts_recv: ext.market_open_ns + 20_000_000_000,
        });

        acc.prepare_for_extraction(ext.market_open_ns + 60_000_000_000);
        let boundary = make_boundary(ext.market_open_ns + 60_000_000_000, 0);
        let mut output = Vec::new();
        ext.extract(&acc, &bbo, &boundary, &mut output);

        // trf_signed_imbalance = (500 - 300) / (500 + 300) = 200 / 800 = 0.25
        assert!(
            (output[TRF_SIGNED_IMBALANCE] - 0.25).abs() < 1e-10,
            "Expected 0.25, got {}",
            output[TRF_SIGNED_IMBALANCE]
        );
    }

    #[test]
    fn test_mroib_golden() {
        let ext = default_extractor();
        let mut acc = default_accumulator();
        let bbo = make_bbo(134.56, 134.57, 100, 100);

        // Retail Buy 200 shares
        acc.accumulate(&ClassifiedTrade {
            direction: TradeDirection::Buy,
            retail_status: RetailStatus::Retail,
            price: 134.5675,
            size: 200,
            publisher_id: publisher::FINN,
            ts_recv: ext.market_open_ns + 10_000_000_000,
        });
        // Retail Sell 100 shares
        acc.accumulate(&ClassifiedTrade {
            direction: TradeDirection::Sell,
            retail_status: RetailStatus::Retail,
            price: 134.5625,
            size: 100,
            publisher_id: publisher::FINN,
            ts_recv: ext.market_open_ns + 20_000_000_000,
        });

        acc.prepare_for_extraction(ext.market_open_ns + 60_000_000_000);
        let boundary = make_boundary(ext.market_open_ns + 60_000_000_000, 0);
        let mut output = Vec::new();
        ext.extract(&acc, &bbo, &boundary, &mut output);

        // mroib = (200 - 100) / (200 + 100) = 100/300 = 0.333...
        assert!(
            (output[MROIB] - 1.0 / 3.0).abs() < 1e-10,
            "Expected 0.333, got {}",
            output[MROIB]
        );
        // inv_inst_direction = -mroib
        assert!(
            (output[INV_INST_DIRECTION] + 1.0 / 3.0).abs() < 1e-10,
            "Expected -0.333, got {}",
            output[INV_INST_DIRECTION]
        );
    }

    #[test]
    fn test_dark_share_golden() {
        let ext = default_extractor();
        let mut acc = default_accumulator();
        let bbo = make_bbo(100.0, 100.01, 100, 100);

        // TRF trades: 600 shares total
        for _ in 0..6 {
            acc.accumulate(&ClassifiedTrade {
                direction: TradeDirection::Buy,
                retail_status: RetailStatus::Institutional,
                price: 100.005, size: 100,
                publisher_id: publisher::FINN,
                ts_recv: ext.market_open_ns + 10_000_000_000,
            });
        }
        // Lit trades: 400 shares
        for _ in 0..4 {
            acc.accumulate(&ClassifiedTrade {
                direction: TradeDirection::Unsigned,
                retail_status: RetailStatus::Institutional,
                price: 100.005, size: 100,
                publisher_id: publisher::XNAS,
                ts_recv: ext.market_open_ns + 20_000_000_000,
            });
        }

        acc.prepare_for_extraction(ext.market_open_ns + 60_000_000_000);
        let boundary = make_boundary(ext.market_open_ns + 60_000_000_000, 0);
        let mut output = Vec::new();
        ext.extract(&acc, &bbo, &boundary, &mut output);

        // dark_share = 600 / (600 + 400) = 0.6
        assert!(
            (output[DARK_SHARE] - 0.6).abs() < 1e-10,
            "Expected 0.6, got {}",
            output[DARK_SHARE]
        );
    }

    #[test]
    fn test_subpenny_intensity_count_ratio() {
        let ext = default_extractor();
        let mut acc = default_accumulator();
        let bbo = make_bbo(100.0, 100.01, 100, 100);

        // 3 subpenny TRF trades (frac_cent in (0.001, 0.999))
        for price in [100.0035, 100.0075, 100.0015] {
            acc.accumulate(&ClassifiedTrade {
                direction: TradeDirection::Buy,
                retail_status: RetailStatus::Retail,
                price, size: 100,
                publisher_id: publisher::FINN,
                ts_recv: ext.market_open_ns + 10_000_000_000,
            });
        }
        // 1 non-subpenny TRF trade (round penny)
        acc.accumulate(&ClassifiedTrade {
            direction: TradeDirection::Sell,
            retail_status: RetailStatus::Institutional,
            price: 100.00, size: 100,
            publisher_id: publisher::FINN,
            ts_recv: ext.market_open_ns + 20_000_000_000,
        });

        acc.prepare_for_extraction(ext.market_open_ns + 60_000_000_000);
        let boundary = make_boundary(ext.market_open_ns + 60_000_000_000, 0);
        let mut output = Vec::new();
        ext.extract(&acc, &bbo, &boundary, &mut output);

        // subpenny_intensity = 3/4 = 0.75 (COUNT ratio)
        assert!(
            (output[SUBPENNY_INTENSITY] - 0.75).abs() < 1e-10,
            "Expected 0.75, got {}",
            output[SUBPENNY_INTENSITY]
        );
    }

    #[test]
    fn test_safety_gates_threshold() {
        let ext = default_extractor();
        let mut acc = default_accumulator();
        let bbo = make_bbo(100.0, 100.01, 100, 100);

        // Add exactly min_trades_per_bin (10) TRF trades
        for i in 0..10 {
            acc.accumulate(&ClassifiedTrade {
                direction: TradeDirection::Buy,
                retail_status: RetailStatus::Institutional,
                price: 100.005, size: 100,
                publisher_id: publisher::FINN,
                ts_recv: ext.market_open_ns + i * 1_000_000_000,
            });
        }

        acc.prepare_for_extraction(ext.market_open_ns + 60_000_000_000);
        let boundary = make_boundary(ext.market_open_ns + 60_000_000_000, 0);
        let mut output = Vec::new();
        ext.extract(&acc, &bbo, &boundary, &mut output);

        assert_eq!(output[BIN_VALID], 1.0, "10 TRF trades >= min_trades(10)");

        // Now test with 9 trades (below threshold)
        acc.reset_bin();
        for i in 0..9 {
            acc.accumulate(&ClassifiedTrade {
                direction: TradeDirection::Buy,
                retail_status: RetailStatus::Institutional,
                price: 100.005, size: 100,
                publisher_id: publisher::FINN,
                ts_recv: ext.market_open_ns + 60_000_000_000 + i * 1_000_000_000,
            });
        }
        acc.prepare_for_extraction(ext.market_open_ns + 120_000_000_000);
        let boundary2 = make_boundary(ext.market_open_ns + 120_000_000_000, 1);
        ext.extract(&acc, &bbo, &boundary2, &mut output);

        assert_eq!(output[BIN_VALID], 0.0, "9 TRF trades < min_trades(10)");
    }

    #[test]
    fn test_session_progress_at_open_and_close() {
        let ext = default_extractor();
        let acc = default_accumulator();
        let bbo = make_bbo(100.0, 100.01, 100, 100);

        // At market open: midpoint = open + 30s → progress ≈ 0.0013 (not exactly 0)
        let boundary_open = BinBoundary {
            bin_end_ts: ext.market_open_ns + 60_000_000_000,
            bin_midpoint_ts: ext.market_open_ns + 30_000_000_000,
            bin_index: 0,
            gap_bins: 0,
        };
        let mut output = Vec::new();
        ext.extract(&acc, &bbo, &boundary_open, &mut output);
        assert!(output[SESSION_PROGRESS] < 0.01, "Near open: progress ≈ 0, got {}", output[SESSION_PROGRESS]);

        // At market close: session_duration = 6.5h = 23400s
        let close_ns = ext.market_open_ns + ext.session_duration_ns;
        let boundary_close = BinBoundary {
            bin_end_ts: close_ns,
            bin_midpoint_ts: close_ns - 30_000_000_000,
            bin_index: 389,
            gap_bins: 0,
        };
        ext.extract(&acc, &bbo, &boundary_close, &mut output);
        assert!(output[SESSION_PROGRESS] > 0.99, "Near close: progress ≈ 1.0, got {}", output[SESSION_PROGRESS]);
    }

    #[test]
    fn test_time_bucket_mapping() {
        let ext = default_extractor();
        let acc = default_accumulator();
        let bbo = make_bbo(100.0, 100.01, 100, 100);
        let mut output = Vec::new();

        // Open auction: 09:32 ET = midpoint in regime 1
        let open_bin = BinBoundary {
            bin_end_ts: ext.market_open_ns + 2 * 60 * 1_000_000_000,
            bin_midpoint_ts: ext.market_open_ns + 90 * 1_000_000_000, // 09:31:30
            bin_index: 1,
            gap_bins: 0,
        };
        ext.extract(&acc, &bbo, &open_bin, &mut output);
        assert_eq!(output[TIME_BUCKET], 0.0, "09:31:30 should be open-auction bucket 0");

        // Morning: 10:00 ET = regime 2
        let morning_offset = (30 * 60) * 1_000_000_000u64; // 30 min after open
        let morning_bin = BinBoundary {
            bin_end_ts: ext.market_open_ns + morning_offset + 60_000_000_000,
            bin_midpoint_ts: ext.market_open_ns + morning_offset + 30_000_000_000,
            bin_index: 30,
            gap_bins: 0,
        };
        ext.extract(&acc, &bbo, &morning_bin, &mut output);
        assert_eq!(output[TIME_BUCKET], 1.0, "10:00 should be morning bucket 1");
    }

    #[test]
    fn test_schema_version_always_set() {
        let ext = default_extractor();
        let acc = default_accumulator();
        let bbo = BboState::new(); // invalid BBO
        let boundary = make_boundary(ext.market_open_ns + 60_000_000_000, 0);
        let mut output = Vec::new();

        ext.extract(&acc, &bbo, &boundary, &mut output);
        assert_eq!(output[SCHEMA_VERSION_IDX], 1.0);
    }

    #[test]
    fn test_disabled_group_produces_zeros() {
        let config = FeatureConfig {
            signed_flow: false,
            venue_metrics: false,
            ..Default::default()
        };
        let mut ext = FeatureExtractor::new(
            &config,
            &ValidationConfig::default(),
            60,
        );
        let date = chrono::NaiveDate::from_ymd_opt(2025, 2, 3).unwrap();
        let midnight = date.and_hms_opt(0, 0, 0).unwrap().and_utc()
            .timestamp_nanos_opt().unwrap() as u64;
        let open = midnight + (14 * 3600 + 30 * 60) * 1_000_000_000;
        let close = open + (6 * 3600 + 30 * 60) * 1_000_000_000;
        ext.init_day(-5, open, close);

        let mut acc = default_accumulator();
        let bbo = make_bbo(100.0, 100.01, 100, 100);
        // Add some trades that WOULD produce non-zero signed flow
        acc.accumulate(&ClassifiedTrade {
            direction: TradeDirection::Buy,
            retail_status: RetailStatus::Institutional,
            price: 100.005, size: 500,
            publisher_id: publisher::FINN,
            ts_recv: open + 10_000_000_000,
        });

        acc.prepare_for_extraction(open + 60_000_000_000);
        let boundary = make_boundary(open + 60_000_000_000, 0);
        let mut output = Vec::new();
        ext.extract(&acc, &bbo, &boundary, &mut output);

        // Disabled groups should be zero
        assert_eq!(output[TRF_SIGNED_IMBALANCE], 0.0, "Signed flow disabled");
        assert_eq!(output[DARK_SHARE], 0.0, "Venue metrics disabled");
        // Enabled groups should be non-zero
        assert!(output[TRADE_COUNT] > 0.0, "Trade size should still work");
    }

    #[test]
    fn test_empty_bin_forward_fill() {
        let ext = default_extractor();
        let mut acc = default_accumulator();
        let bbo = make_bbo(100.0, 100.01, 100, 100);

        // Bin 0: accumulate trades to establish forward-fill state
        for i in 0..15 {
            acc.accumulate(&ClassifiedTrade {
                direction: TradeDirection::Buy,
                retail_status: RetailStatus::Institutional,
                price: 100.005, size: 100,
                publisher_id: publisher::FINN,
                ts_recv: ext.market_open_ns + i * 1_000_000_000,
            });
        }
        acc.accumulate_bbo_update(&bbo, ext.market_open_ns + 5_000_000_000);

        acc.prepare_for_extraction(ext.market_open_ns + 60_000_000_000);
        let boundary0 = make_boundary(ext.market_open_ns + 60_000_000_000, 0);
        let mut output0 = Vec::new();
        ext.extract(&acc, &bbo, &boundary0, &mut output0);

        let dark_share_bin0 = output0[DARK_SHARE];
        assert!(dark_share_bin0 > 0.0, "Bin 0 dark_share should be > 0");

        // Update forward-fill from bin 0
        acc.update_forward_fill(&output0, false);
        if acc.bbo_update_count() > 0 {
            acc.update_forward_fill_bbo(output0[SPREAD_BPS], output0[QUOTE_IMBALANCE]);
        }
        acc.reset_bin();

        // Bin 1: empty (no trades, no BBO updates)
        acc.prepare_for_extraction(ext.market_open_ns + 120_000_000_000);
        let boundary1 = make_boundary(ext.market_open_ns + 120_000_000_000, 1);
        let mut output1 = Vec::new();
        ext.extract(&acc, &bbo, &boundary1, &mut output1);

        // Flow features should be 0
        assert_eq!(output1[TRF_SIGNED_IMBALANCE], 0.0, "Empty bin: flow = 0");
        assert_eq!(output1[TRADE_COUNT], 0.0, "Empty bin: no trades");

        // State features should be forward-filled from bin 0
        assert_eq!(
            output1[DARK_SHARE], dark_share_bin0,
            "Empty bin: dark_share should be forward-filled from bin 0"
        );
    }

    #[test]
    fn test_bid_pressure_zero_start_guard() {
        let ext = default_extractor();
        let mut acc = default_accumulator();

        // BBO with bid_size = 0
        let mut bbo_zero = BboState::new();
        bbo_zero.bid_price = 100.0;
        bbo_zero.ask_price = 100.01;
        bbo_zero.bid_size = 0; // zero start!
        bbo_zero.ask_size = 100;
        bbo_zero.is_valid = true;
        bbo_zero.spread = 0.01;
        bbo_zero.last_update_ts = ext.market_open_ns + 1_000_000_000;

        acc.accumulate_bbo_update(&bbo_zero, ext.market_open_ns + 1_000_000_000);

        // Update BBO to non-zero bid
        let bbo_end = make_bbo(100.0, 100.01, 500, 100);
        acc.accumulate_bbo_update(&bbo_end, ext.market_open_ns + 30_000_000_000);

        // Add a trade so this isn't an empty bin
        acc.accumulate(&ClassifiedTrade {
            direction: TradeDirection::Buy,
            retail_status: RetailStatus::Institutional,
            price: 100.005, size: 100,
            publisher_id: publisher::FINN,
            ts_recv: ext.market_open_ns + 10_000_000_000,
        });

        acc.prepare_for_extraction(ext.market_open_ns + 60_000_000_000);
        let boundary = make_boundary(ext.market_open_ns + 60_000_000_000, 0);
        let mut output = Vec::new();
        ext.extract(&acc, &bbo_end, &boundary, &mut output);

        // R11: bid_pressure should be 0.0 (zero start guard), NOT ~5e10
        assert_eq!(
            output[BID_PRESSURE], 0.0,
            "bid_pressure with zero start should be 0.0, got {}",
            output[BID_PRESSURE]
        );
        // All values must be finite
        assert!(output.iter().all(|v| v.is_finite()), "No NaN/Inf allowed");
    }

    #[test]
    fn test_feature_count() {
        let ext = default_extractor();
        let acc = default_accumulator();
        let bbo = make_bbo(100.0, 100.01, 100, 100);
        let boundary = make_boundary(ext.market_open_ns + 60_000_000_000, 0);
        let mut output = Vec::new();

        ext.extract(&acc, &bbo, &boundary, &mut output);
        assert_eq!(output.len(), TOTAL_FEATURES, "Always 34 features");
    }
}
