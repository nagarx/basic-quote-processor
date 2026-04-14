//! Per-bin trade count accumulation by category.
//!
//! Tracks total, TRF, lit, retail, subpenny, odd-lot, and block trade counts.
//! Subpenny, odd-lot, and retail counts are TRF-only. Block counts are all-trades.
//! All counters reset at each bin boundary.
//!
//! Source: docs/design/04_FEATURE_SPECIFICATION.md §5.3, §5.6

/// Per-bin trade count accumulator.
///
/// Categorization scopes:
/// - `subpenny_trades`, `odd_lot_trades`, `retail_trades`: TRF trades only
/// - `block_trades`: ALL trades (per spec index 21)
/// - `total_trades`, `trf_trades`, `lit_trades`: by venue
#[derive(Debug, Clone)]
pub(crate) struct CountAccumulator {
    pub(crate) total_trades: u64,
    pub(crate) trf_trades: u64,
    pub(crate) lit_trades: u64,
    pub(crate) retail_trades: u64,
    pub(crate) subpenny_trades: u64,
    pub(crate) odd_lot_trades: u64,
    pub(crate) block_trades: u64,
    block_threshold: u32,
}

impl CountAccumulator {
    /// Create with configurable block detection threshold.
    ///
    /// # Arguments
    /// * `block_threshold` — Trade size >= this counts as a block trade. Default: 10,000 shares.
    pub(crate) fn new(block_threshold: u32) -> Self {
        Self {
            total_trades: 0,
            trf_trades: 0,
            lit_trades: 0,
            retail_trades: 0,
            subpenny_trades: 0,
            odd_lot_trades: 0,
            block_trades: 0,
            block_threshold,
        }
    }

    /// Accumulate a single trade into the appropriate counters.
    ///
    /// `is_subpenny` should only be `true` when `is_trf` is also `true`.
    /// The caller computes `is_subpenny` via `bjzz::is_subpenny(price)` for TRF trades.
    pub(crate) fn accumulate(
        &mut self,
        size: u32,
        is_trf: bool,
        is_lit: bool,
        is_retail: bool,
        is_subpenny: bool,
    ) {
        self.total_trades += 1;

        if is_trf {
            self.trf_trades += 1;
            if is_retail {
                self.retail_trades += 1;
            }
            if is_subpenny {
                self.subpenny_trades += 1;
            }
            if size < 100 {
                self.odd_lot_trades += 1;
            }
        }

        if is_lit {
            self.lit_trades += 1;
        }

        // Block detection: ALL trades, not just TRF
        if size >= self.block_threshold {
            self.block_trades += 1;
        }
    }

    /// Reset all counters for new bin. Preserves block_threshold configuration.
    pub(crate) fn reset(&mut self) {
        self.total_trades = 0;
        self.trf_trades = 0;
        self.lit_trades = 0;
        self.retail_trades = 0;
        self.subpenny_trades = 0;
        self.odd_lot_trades = 0;
        self.block_trades = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_all_zeros() {
        let acc = CountAccumulator::new(10_000);
        assert_eq!(acc.total_trades, 0);
        assert_eq!(acc.trf_trades, 0);
        assert_eq!(acc.lit_trades, 0);
        assert_eq!(acc.retail_trades, 0);
        assert_eq!(acc.subpenny_trades, 0);
        assert_eq!(acc.odd_lot_trades, 0);
        assert_eq!(acc.block_trades, 0);
    }

    #[test]
    fn test_trf_trade_increments_total_and_trf() {
        let mut acc = CountAccumulator::new(10_000);
        acc.accumulate(100, true, false, false, false);
        assert_eq!(acc.total_trades, 1);
        assert_eq!(acc.trf_trades, 1);
        assert_eq!(acc.lit_trades, 0);
    }

    #[test]
    fn test_lit_trade_increments_total_and_lit() {
        let mut acc = CountAccumulator::new(10_000);
        acc.accumulate(100, false, true, false, false);
        assert_eq!(acc.total_trades, 1);
        assert_eq!(acc.lit_trades, 1);
        assert_eq!(acc.trf_trades, 0);
    }

    #[test]
    fn test_trf_retail_increments_retail() {
        let mut acc = CountAccumulator::new(10_000);
        acc.accumulate(100, true, false, true, false);
        assert_eq!(acc.trf_trades, 1);
        assert_eq!(acc.retail_trades, 1);
    }

    #[test]
    fn test_trf_subpenny_increments_subpenny() {
        let mut acc = CountAccumulator::new(10_000);
        acc.accumulate(100, true, false, false, true);
        assert_eq!(acc.subpenny_trades, 1);
    }

    #[test]
    fn test_trf_odd_lot() {
        let mut acc = CountAccumulator::new(10_000);
        // size < 100 is odd lot for TRF
        acc.accumulate(50, true, false, false, false);
        assert_eq!(acc.odd_lot_trades, 1);

        // size = 100 is NOT odd lot
        acc.accumulate(100, true, false, false, false);
        assert_eq!(acc.odd_lot_trades, 1, "size=100 should not be odd lot");
    }

    #[test]
    fn test_block_detection_all_trades() {
        let mut acc = CountAccumulator::new(10_000);
        // Block from TRF trade
        acc.accumulate(15_000, true, false, false, false);
        assert_eq!(acc.block_trades, 1);

        // Block from lit trade (block counts ALL trades)
        acc.accumulate(20_000, false, true, false, false);
        assert_eq!(acc.block_trades, 2, "Block detection must count ALL trades, not just TRF");
    }

    #[test]
    fn test_reset_clears_all_preserves_threshold() {
        let mut acc = CountAccumulator::new(5_000);
        acc.accumulate(100, true, false, true, true);
        acc.accumulate(6_000, false, true, false, false);
        assert!(acc.total_trades > 0);

        acc.reset();
        assert_eq!(acc.total_trades, 0);
        assert_eq!(acc.trf_trades, 0);
        assert_eq!(acc.retail_trades, 0);
        assert_eq!(acc.subpenny_trades, 0);
        assert_eq!(acc.block_trades, 0);

        // Verify threshold preserved by adding a block trade after reset
        acc.accumulate(5_000, false, true, false, false);
        assert_eq!(acc.block_trades, 1, "block_threshold must be preserved after reset");
    }
}
