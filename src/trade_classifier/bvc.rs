//! Bulk Volume Classification (BVC) for probabilistic trade signing.
//!
//! BVC uses price changes and their rolling standard deviation to estimate
//! buy/sell volume fractions via the standard normal CDF Phi(z). Unlike
//! midpoint signing (per-trade direction), BVC produces probabilistic
//! volume splits used for aggregate features (bvc_imbalance, VPIN).
//!
//! # Formula (Easley et al. 2012, Eq. 7)
//!
//! ```text
//! For each trade i:
//!     delta_p_i = price_i - price_{i-1}
//!     sigma = std(delta_p) over rolling window
//!     z_i = delta_p_i / max(sigma, EPS)
//!     buy_vol_i  = size_i * Phi(z_i)
//!     sell_vol_i = size_i * (1.0 - Phi(z_i))
//! ```
//!
//! When sigma = 0, Phi(0) = 0.5, producing equal buy/sell split (neutral).
//!
//! # Ownership
//!
//! BvcState is DEFINED here but NOT owned by TradeClassifier. It is owned
//! by the Phase 3 pipeline orchestrator / BinAccumulator, which feeds it
//! each trade's (price, size, timestamp) directly. This separation follows
//! single responsibility: TradeClassifier = signing + retail ID,
//! BvcState = probabilistic volume classification.
//!
//! BVC processes ALL trades (not TRF-only) per 04_FEATURE_SPECIFICATION.md line 308.
//!
//! # Source
//!
//! Easley, D., M. Lopez de Prado, and M. O'Hara (2012). "Flow Toxicity and
//! Liquidity in a High-Frequency World." *Rev. Financial Studies*, 25(5),
//! 1457-1493. Eq. 7.

use std::collections::VecDeque;
use hft_statistics::statistics::phi;
use crate::contract::EPS;

/// BVC state maintaining rolling price change statistics.
///
/// Tracks price changes within a configurable time window for computing
/// the standard deviation (sigma) used in BVC classification.
#[derive(Debug)]
pub struct BvcState {
    /// Previous trade price (USD). None until first trade is seen.
    prev_price: Option<f64>,
    /// Rolling window of price changes (delta_p = price_i - price_{i-1}).
    price_changes: VecDeque<f64>,
    /// Timestamps corresponding to entries in price_changes.
    timestamps: VecDeque<u64>,
    /// Sigma window duration in nanoseconds.
    window_ns: u64,
}

impl BvcState {
    /// Create a new BVC state with the given sigma window.
    ///
    /// # Arguments
    ///
    /// * `sigma_window_minutes` — Rolling window for sigma computation.
    ///   Default: 1 minute. Source: 05_CONFIGURATION_SCHEMA.md [vpin] section.
    pub fn new(sigma_window_minutes: u32) -> Self {
        Self {
            prev_price: None,
            price_changes: VecDeque::with_capacity(1024),
            timestamps: VecDeque::with_capacity(1024),
            window_ns: sigma_window_minutes as u64 * 60 * 1_000_000_000,
        }
    }

    /// Classify a single trade, returning (buy_volume, sell_volume).
    ///
    /// Updates the rolling sigma window and produces BVC-classified
    /// volume fractions for this trade.
    ///
    /// # Returns
    ///
    /// `(buy_vol, sell_vol)` where `buy_vol + sell_vol = size` (approximately,
    /// subject to f64 rounding).
    ///
    /// # Arguments
    ///
    /// * `price` — Trade price in USD
    /// * `size` — Trade size in shares
    /// * `ts_ns` — Receipt timestamp in UTC nanoseconds (for window eviction)
    pub fn classify_trade(&mut self, price: f64, size: u32, ts_ns: u64) -> (f64, f64) {
        let size_f = size as f64;

        // Compute price change
        let delta_p = match self.prev_price {
            Some(prev) => price - prev,
            None => 0.0, // First trade: no price change → phi(0) = 0.5 → neutral
        };
        self.prev_price = Some(price);

        // Add to rolling window
        self.price_changes.push_back(delta_p);
        self.timestamps.push_back(ts_ns);

        // Evict old entries outside the sigma window
        while let Some(&oldest_ts) = self.timestamps.front() {
            if ts_ns.saturating_sub(oldest_ts) > self.window_ns {
                self.price_changes.pop_front();
                self.timestamps.pop_front();
            } else {
                break;
            }
        }

        // Compute sigma (std of price changes in window)
        let sigma = self.compute_sigma();

        // BVC classification: buy_vol = size * phi(delta_p / sigma)
        let z = if sigma > EPS {
            delta_p / sigma
        } else {
            0.0 // sigma ≈ 0 → no variation → phi(0) = 0.5 → neutral split
        };

        let phi_z = phi(z);
        let buy_vol = size_f * phi_z;
        let sell_vol = size_f * (1.0 - phi_z);

        (buy_vol, sell_vol)
    }

    /// Compute the standard deviation of price changes in the rolling window.
    ///
    /// Uses the two-pass algorithm (acceptable for the small window sizes used here).
    /// Returns 0.0 when fewer than 2 observations are available.
    fn compute_sigma(&self) -> f64 {
        let n = self.price_changes.len();
        if n < 2 {
            return 0.0;
        }

        let n_f = n as f64;
        let mean = self.price_changes.iter().sum::<f64>() / n_f;
        let variance = self.price_changes.iter()
            .map(|&dp| (dp - mean).powi(2))
            .sum::<f64>() / (n_f - 1.0); // Sample variance (Bessel's correction)

        variance.sqrt()
    }

    /// Reset state for a new trading day.
    pub fn reset(&mut self) {
        self.prev_price = None;
        self.price_changes.clear();
        self.timestamps.clear();
    }

    /// Number of price changes in the current rolling window.
    pub fn window_size(&self) -> usize {
        self.price_changes.len()
    }

    /// Current sigma (std of price changes). 0.0 if < 2 observations.
    pub fn current_sigma(&self) -> f64 {
        self.compute_sigma()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NS_PER_SECOND: u64 = 1_000_000_000;

    #[test]
    fn test_first_trade_neutral() {
        let mut bvc = BvcState::new(1); // 1 minute window
        let (buy, sell) = bvc.classify_trade(100.0, 1000, 0);
        // First trade: delta_p = 0, phi(0) = 0.5
        assert!((buy - 500.0).abs() < 1e-6, "First trade: buy should be 500, got {}", buy);
        assert!((sell - 500.0).abs() < 1e-6, "First trade: sell should be 500, got {}", sell);
    }

    #[test]
    fn test_positive_price_move() {
        let mut bvc = BvcState::new(1);
        // Two trades to establish sigma
        bvc.classify_trade(100.0, 100, 0);
        bvc.classify_trade(100.01, 100, NS_PER_SECOND);
        bvc.classify_trade(100.02, 100, 2 * NS_PER_SECOND);

        // Third trade: positive price move
        let (buy, sell) = bvc.classify_trade(100.05, 1000, 3 * NS_PER_SECOND);
        assert!(
            buy > sell,
            "Positive price move: buy_vol ({}) should exceed sell_vol ({})",
            buy, sell
        );
    }

    #[test]
    fn test_negative_price_move() {
        let mut bvc = BvcState::new(1);
        bvc.classify_trade(100.0, 100, 0);
        bvc.classify_trade(100.01, 100, NS_PER_SECOND);
        bvc.classify_trade(100.02, 100, 2 * NS_PER_SECOND);

        // Negative price move
        let (buy, sell) = bvc.classify_trade(99.95, 1000, 3 * NS_PER_SECOND);
        assert!(
            sell > buy,
            "Negative price move: sell_vol ({}) should exceed buy_vol ({})",
            sell, buy
        );
    }

    #[test]
    fn test_sigma_zero_produces_neutral() {
        let mut bvc = BvcState::new(1);
        // All trades at same price → sigma = 0
        bvc.classify_trade(100.0, 100, 0);
        let (buy, sell) = bvc.classify_trade(100.0, 1000, NS_PER_SECOND);
        assert!(
            (buy - 500.0).abs() < 1e-6,
            "Same price: should be neutral, buy={}",
            buy
        );
        assert!((sell - 500.0).abs() < 1e-6, "Same price: sell={}", sell);
    }

    #[test]
    fn test_reset_clears_state() {
        let mut bvc = BvcState::new(1);
        bvc.classify_trade(100.0, 100, 0);
        bvc.classify_trade(100.01, 100, NS_PER_SECOND);
        assert!(bvc.window_size() > 0);

        bvc.reset();
        assert_eq!(bvc.window_size(), 0);
        assert!(bvc.prev_price.is_none());

        // After reset, first trade is neutral again
        let (buy, sell) = bvc.classify_trade(100.0, 1000, 0);
        assert!((buy - 500.0).abs() < 1e-6);
        assert!((sell - 500.0).abs() < 1e-6);
    }

    #[test]
    fn test_golden_bvc_phi() {
        // Golden test: verify phi(0.5) = 0.6914624612740131 (scipy reference)
        let phi_half = phi(0.5);
        assert!(
            (phi_half - 0.6914624612740131).abs() < 1.5e-7,
            "phi(0.5) = {}, expected 0.6914624612740131",
            phi_half
        );

        // BVC with known values: delta_p = 0.05, sigma = 0.10
        // z = 0.05 / 0.10 = 0.5, phi(0.5) = 0.6915
        // buy_vol = 1000 * 0.6915 = 691.5, sell_vol = 308.5
        // We can't directly set sigma, but we can verify the math is correct
        let buy = 1000.0 * phi(0.5);
        assert!(
            (buy - 691.46).abs() < 0.1,
            "BVC golden: buy should be ~691.5, got {}",
            buy
        );
    }

    #[test]
    fn test_golden_bvc_determinism_sequence() {
        // Golden determinism test: 5 trades with known prices, verify exact (buy, sell)
        // at each step. Required by 07_TESTING_STRATEGY.md Section 4.6.
        //
        // sigma_window = 1 minute, all trades within window (1s apart).
        let mut bvc = BvcState::new(1);

        // Trade 1: price=100.00, size=1000, first trade → delta_p=0, sigma=0, phi≈0.5
        // Note: phi(0.0) is not EXACTLY 0.5 due to the erf approximation (~1e-9 error)
        let (buy, sell) = bvc.classify_trade(100.00, 1000, 0);
        assert!(
            (buy - 500.0).abs() < 1000.0 * 1.5e-7,
            "Trade 1: buy={:.10}, expected ~500.0", buy
        );
        assert!(
            (sell - 500.0).abs() < 1000.0 * 1.5e-7,
            "Trade 1: sell={:.10}, expected ~500.0", sell
        );

        // Trade 2: price=100.01, size=500
        // delta_p = +0.01, window = [0.0, +0.01]
        // sigma = sqrt(0.00005) = 0.00707106781186548 (sample std, n-1=1)
        // z = 0.01 / 0.00707... = sqrt(2) = 1.41421356...
        // phi(sqrt(2)) = 0.5*(1 + erf(1.0)) = 0.92135...
        let (buy2, sell2) = bvc.classify_trade(100.01, 500, NS_PER_SECOND);
        let phi_sqrt2 = phi(std::f64::consts::SQRT_2);
        let expected_buy2 = 500.0 * phi_sqrt2;
        let expected_sell2 = 500.0 * (1.0 - phi_sqrt2);
        assert!(
            (buy2 - expected_buy2).abs() < 500.0 * 1.5e-7,
            "Trade 2: buy={:.10}, expected={:.10}",
            buy2, expected_buy2
        );
        assert!(
            (sell2 - expected_sell2).abs() < 500.0 * 1.5e-7,
            "Trade 2: sell={:.10}, expected={:.10}",
            sell2, expected_sell2
        );
        assert!(buy2 > sell2, "Trade 2: positive move → buy > sell");

        // Trade 3: price=100.03, size=800
        // delta_p = +0.02, window = [0.0, +0.01, +0.02]
        // mean=0.01, sample_var=0.0001, sigma=0.01, z=2.0
        // phi(2.0) = 0.9772498680518208 (scipy reference)
        let (buy3, sell3) = bvc.classify_trade(100.03, 800, 2 * NS_PER_SECOND);
        let phi_2 = phi(2.0);
        let expected_buy3 = 800.0 * phi_2;
        let expected_sell3 = 800.0 * (1.0 - phi_2);
        assert!(
            (buy3 - expected_buy3).abs() < 800.0 * 1.5e-7,
            "Trade 3: buy={:.10}, expected={:.10}",
            buy3, expected_buy3
        );
        assert!(
            (sell3 - expected_sell3).abs() < 800.0 * 1.5e-7,
            "Trade 3: sell={:.10}, expected={:.10}",
            sell3, expected_sell3
        );

        // Trade 4: price=100.01, size=600
        // delta_p = -0.02 (negative move → sell should dominate)
        let (buy4, sell4) = bvc.classify_trade(100.01, 600, 3 * NS_PER_SECOND);
        assert!(
            sell4 > buy4,
            "Trade 4: negative move → sell ({:.4}) should exceed buy ({:.4})",
            sell4, buy4
        );
        // Verify conservation: buy + sell = size
        assert!(
            (buy4 + sell4 - 600.0).abs() < 1e-10,
            "Trade 4: buy+sell should equal size 600, got {}",
            buy4 + sell4
        );

        // Trade 5: price=100.01, size=400
        // delta_p = 0.0 (same price) → z = 0.0/sigma = 0.0 → phi ≈ 0.5
        // Note: phi(0.0) has ~1e-9 error from erf approximation
        let (buy5, sell5) = bvc.classify_trade(100.01, 400, 4 * NS_PER_SECOND);
        assert!(
            (buy5 - 200.0).abs() < 400.0 * 1.5e-7,
            "Trade 5: zero delta → buy≈200.0, got {:.10}", buy5
        );
        assert!(
            (sell5 - 200.0).abs() < 400.0 * 1.5e-7,
            "Trade 5: zero delta → sell≈200.0, got {:.10}", sell5
        );

        // Verify window size grew correctly
        assert_eq!(bvc.window_size(), 5, "Should have 5 price changes in window");
    }
}
