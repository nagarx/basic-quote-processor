//! Per-bin volume accumulation by venue, direction, and retail status.
//!
//! Tracks signed/unsigned TRF volume, retail volume (including unsigned retail),
//! lit volume, and BVC-classified probabilistic volume. All counters reset at
//! each bin boundary.
//!
//! Source: docs/design/04_FEATURE_SPECIFICATION.md §5.1-5.3

use crate::trade_classifier::{TradeDirection, RetailStatus};

/// Per-bin volume accumulator.
///
/// Accumulates trade volumes by venue, direction, and retail status.
/// All fields are in shares (f64 for fractional BVC volumes).
#[derive(Debug, Clone, Default)]
pub(crate) struct FlowAccumulator {
    // ── TRF signed volumes ──────────────────────────────────────────
    /// Volume of TRF trades signed Buy.
    pub(crate) trf_buy_vol: f64,
    /// Volume of TRF trades signed Sell.
    pub(crate) trf_sell_vol: f64,
    /// Volume of TRF trades classified as Unsigned (within exclusion band or invalid BBO).
    pub(crate) trf_unsigned_vol: f64,

    // ── TRF retail volumes ──────────────────────────────────────────
    /// Retail TRF Buy volume.
    pub(crate) retail_buy_vol: f64,
    /// Retail TRF Sell volume.
    pub(crate) retail_sell_vol: f64,
    /// Total retail TRF volume regardless of direction (Buy + Sell + Unsigned).
    /// Used for `retail_volume_fraction` which counts ALL retail TRF volume.
    pub(crate) retail_total_vol: f64,

    // ── Lit volume ──────────────────────────────────────────────────
    /// Volume from lit venues (XNAS, XBOS, XPSX). Direction irrelevant for L1 tape.
    pub(crate) lit_vol: f64,

    // ── BVC volumes ─────────────────────────────────────────────────
    /// Probabilistic buy volume from BVC. Accumulated from ALL trades (not TRF-only).
    /// Source: Easley et al. (2012) Eq. 7.
    pub(crate) bvc_buy_vol: f64,
    /// Probabilistic sell volume from BVC. bvc_buy + bvc_sell ≈ total volume.
    pub(crate) bvc_sell_vol: f64,
}

impl FlowAccumulator {
    /// Accumulate a TRF trade's volume by direction and retail status.
    ///
    /// Dispatches to trf_buy/sell/unsigned based on direction.
    /// If retail: ALWAYS adds to `retail_total_vol` (regardless of direction),
    /// plus `retail_buy_vol` or `retail_sell_vol` if direction is Buy/Sell.
    pub(crate) fn accumulate_trf(
        &mut self,
        direction: TradeDirection,
        retail_status: RetailStatus,
        size: u32,
    ) {
        let size_f = size as f64;

        // TRF signed volume
        match direction {
            TradeDirection::Buy => self.trf_buy_vol += size_f,
            TradeDirection::Sell => self.trf_sell_vol += size_f,
            TradeDirection::Unsigned => self.trf_unsigned_vol += size_f,
        }

        // Retail volume (if retail, track total AND signed)
        if retail_status == RetailStatus::Retail {
            self.retail_total_vol += size_f;
            match direction {
                TradeDirection::Buy => self.retail_buy_vol += size_f,
                TradeDirection::Sell => self.retail_sell_vol += size_f,
                TradeDirection::Unsigned => {} // only retail_total_vol incremented
            }
        }
    }

    /// Accumulate a lit venue trade's volume.
    pub(crate) fn accumulate_lit(&mut self, size: u32) {
        self.lit_vol += size as f64;
    }

    /// Accumulate BVC-classified probabilistic volumes.
    ///
    /// Called with output from `BvcState::classify_trade()`. Processes ALL trades.
    pub(crate) fn accumulate_bvc(&mut self, buy_vol: f64, sell_vol: f64) {
        self.bvc_buy_vol += buy_vol;
        self.bvc_sell_vol += sell_vol;
    }

    /// Total TRF volume (buy + sell + unsigned).
    pub(crate) fn trf_total_vol(&self) -> f64 {
        self.trf_buy_vol + self.trf_sell_vol + self.trf_unsigned_vol
    }

    /// Reset all counters for new bin.
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_all_zeros() {
        let acc = FlowAccumulator::default();
        assert_eq!(acc.trf_buy_vol, 0.0);
        assert_eq!(acc.trf_sell_vol, 0.0);
        assert_eq!(acc.trf_unsigned_vol, 0.0);
        assert_eq!(acc.retail_buy_vol, 0.0);
        assert_eq!(acc.retail_sell_vol, 0.0);
        assert_eq!(acc.retail_total_vol, 0.0);
        assert_eq!(acc.lit_vol, 0.0);
        assert_eq!(acc.bvc_buy_vol, 0.0);
        assert_eq!(acc.bvc_sell_vol, 0.0);
    }

    #[test]
    fn test_trf_buy() {
        let mut acc = FlowAccumulator::default();
        acc.accumulate_trf(TradeDirection::Buy, RetailStatus::Institutional, 100);
        assert_eq!(acc.trf_buy_vol, 100.0);
        assert_eq!(acc.trf_sell_vol, 0.0);
        assert_eq!(acc.trf_unsigned_vol, 0.0);
        assert_eq!(acc.trf_total_vol(), 100.0);
        assert_eq!(acc.retail_total_vol, 0.0);
    }

    #[test]
    fn test_trf_sell() {
        let mut acc = FlowAccumulator::default();
        acc.accumulate_trf(TradeDirection::Sell, RetailStatus::Institutional, 200);
        assert_eq!(acc.trf_sell_vol, 200.0);
        assert_eq!(acc.trf_total_vol(), 200.0);
    }

    #[test]
    fn test_trf_unsigned() {
        let mut acc = FlowAccumulator::default();
        acc.accumulate_trf(TradeDirection::Unsigned, RetailStatus::Institutional, 50);
        assert_eq!(acc.trf_unsigned_vol, 50.0);
        assert_eq!(acc.trf_total_vol(), 50.0);
    }

    #[test]
    fn test_retail_buy_increments_total_and_signed() {
        let mut acc = FlowAccumulator::default();
        acc.accumulate_trf(TradeDirection::Buy, RetailStatus::Retail, 100);
        assert_eq!(acc.trf_buy_vol, 100.0);
        assert_eq!(acc.retail_buy_vol, 100.0);
        assert_eq!(acc.retail_sell_vol, 0.0);
        assert_eq!(acc.retail_total_vol, 100.0, "retail_total_vol must track all retail");
    }

    #[test]
    fn test_retail_unsigned_only_increments_total() {
        let mut acc = FlowAccumulator::default();
        acc.accumulate_trf(TradeDirection::Unsigned, RetailStatus::Retail, 75);
        assert_eq!(acc.trf_unsigned_vol, 75.0);
        assert_eq!(acc.retail_buy_vol, 0.0, "unsigned retail should NOT go to retail_buy");
        assert_eq!(acc.retail_sell_vol, 0.0, "unsigned retail should NOT go to retail_sell");
        assert_eq!(acc.retail_total_vol, 75.0, "unsigned retail MUST go to retail_total");
    }

    #[test]
    fn test_lit_volume() {
        let mut acc = FlowAccumulator::default();
        acc.accumulate_lit(300);
        assert_eq!(acc.lit_vol, 300.0);
        assert_eq!(acc.trf_total_vol(), 0.0, "lit should not affect TRF");
    }

    #[test]
    fn test_bvc_independent() {
        let mut acc = FlowAccumulator::default();
        acc.accumulate_bvc(691.5, 308.5);
        assert_eq!(acc.bvc_buy_vol, 691.5);
        assert_eq!(acc.bvc_sell_vol, 308.5);
        assert_eq!(acc.trf_total_vol(), 0.0, "BVC should not affect TRF counters");
        assert_eq!(acc.lit_vol, 0.0, "BVC should not affect lit volume");
    }

    #[test]
    fn test_reset_clears_all() {
        let mut acc = FlowAccumulator::default();
        acc.accumulate_trf(TradeDirection::Buy, RetailStatus::Retail, 100);
        acc.accumulate_lit(200);
        acc.accumulate_bvc(50.0, 50.0);
        assert!(acc.trf_total_vol() > 0.0);

        acc.reset();
        assert_eq!(acc.trf_buy_vol, 0.0);
        assert_eq!(acc.retail_total_vol, 0.0);
        assert_eq!(acc.lit_vol, 0.0);
        assert_eq!(acc.bvc_buy_vol, 0.0);
    }
}
