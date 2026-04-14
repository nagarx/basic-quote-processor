//! Midpoint signing for TRF trades using the Barber et al. (2024) method.
//!
//! Compares trade price to the BBO midpoint with a configurable exclusion band.
//! Trades above/below the exclusion zone are signed Buy/Sell; trades within
//! the zone are Unsigned. Pure function — no state, no side effects.
//!
//! # Formula
//!
//! ```text
//! buy_threshold  = mid + exclusion_band * spread
//! sell_threshold = mid - exclusion_band * spread
//! direction = Buy       if price > buy_threshold    (strict >)
//!           = Sell      if price < sell_threshold    (strict <)
//!           = Unsigned  otherwise
//! ```
//!
//! Uses FULL spread (not half-spread) — a documented conservative deviation
//! from the BJZZ half-spread convention, producing ~15.4% unsigned trades.
//!
//! Published accuracy: 94.8% equal-weighted, uniform across spread levels.
//!
//! # Source
//!
//! Barber, B.M., X. Huang, P. Jorion, T. Odean, and C. Schwarz (2024).
//! "A (Sub)penny for Your Thoughts." *J. Finance*, 79(4), 2403-2427. Section III.

use super::types::TradeDirection;

/// Sign a TRF trade using the midpoint method.
///
/// Returns `Unsigned` when `bbo_valid` is false (cannot sign without valid reference).
/// Uses strict comparison (`>` and `<`, not `>=`/`<=`) — exact-midpoint trades are Unsigned.
///
/// # Arguments
///
/// * `trade_price` — Trade execution price in USD
/// * `mid` — BBO midpoint `(bid + ask) / 2.0` in USD
/// * `spread` — BBO spread `ask - bid` in USD (must be > 0 when valid)
/// * `exclusion_band` — Fraction of spread for unsigned zone [0.0, 0.50]
/// * `bbo_valid` — Whether the BBO is valid (spread > 0, prices finite/positive)
#[inline]
pub fn sign_midpoint(
    trade_price: f64,
    mid: f64,
    spread: f64,
    exclusion_band: f64,
    bbo_valid: bool,
) -> TradeDirection {
    if !bbo_valid {
        return TradeDirection::Unsigned;
    }

    let band = exclusion_band * spread;
    let buy_threshold = mid + band;
    let sell_threshold = mid - band;

    if trade_price > buy_threshold {
        TradeDirection::Buy
    } else if trade_price < sell_threshold {
        TradeDirection::Sell
    } else {
        TradeDirection::Unsigned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Standard test BBO: bid=$100.00, ask=$100.10, mid=$100.05, spread=$0.10
    const MID: f64 = 100.05;
    const SPREAD: f64 = 0.10;
    const EXCL: f64 = 0.10; // exclusion_band
    // buy_threshold = 100.05 + 0.10 * 0.10 = 100.06
    // sell_threshold = 100.05 - 0.01 = 100.04

    #[test]
    fn test_buy_above_threshold() {
        // 100.08 > 100.06 → Buy
        assert_eq!(sign_midpoint(100.08, MID, SPREAD, EXCL, true), TradeDirection::Buy);
    }

    #[test]
    fn test_sell_below_threshold() {
        // 100.02 < 100.04 → Sell
        assert_eq!(sign_midpoint(100.02, MID, SPREAD, EXCL, true), TradeDirection::Sell);
    }

    #[test]
    fn test_unsigned_within_band() {
        // 100.05 is between 100.04 and 100.06 → Unsigned
        assert_eq!(sign_midpoint(100.05, MID, SPREAD, EXCL, true), TradeDirection::Unsigned);
    }

    #[test]
    fn test_exactly_at_midpoint() {
        // Exact midpoint → Unsigned (within band for any excl >= 0)
        assert_eq!(sign_midpoint(MID, MID, SPREAD, EXCL, true), TradeDirection::Unsigned);
    }

    #[test]
    fn test_exactly_at_buy_threshold() {
        // 100.06 == buy_threshold → Unsigned (strict >, not >=)
        let buy_threshold = MID + EXCL * SPREAD;
        assert_eq!(
            sign_midpoint(buy_threshold, MID, SPREAD, EXCL, true),
            TradeDirection::Unsigned,
            "Exact buy threshold should be Unsigned (strict >)"
        );
    }

    #[test]
    fn test_exactly_at_sell_threshold() {
        // 100.04 == sell_threshold → Unsigned (strict <, not <=)
        let sell_threshold = MID - EXCL * SPREAD;
        assert_eq!(
            sign_midpoint(sell_threshold, MID, SPREAD, EXCL, true),
            TradeDirection::Unsigned,
            "Exact sell threshold should be Unsigned (strict <)"
        );
    }

    #[test]
    fn test_invalid_bbo_returns_unsigned() {
        assert_eq!(
            sign_midpoint(100.08, MID, SPREAD, EXCL, false),
            TradeDirection::Unsigned,
            "Invalid BBO must produce Unsigned"
        );
    }

    #[test]
    fn test_zero_exclusion_band() {
        // exclusion_band = 0.0: only exact midpoint is Unsigned
        assert_eq!(sign_midpoint(MID, MID, SPREAD, 0.0, true), TradeDirection::Unsigned);
        // Even 1 nanodollar above mid → Buy
        assert_eq!(sign_midpoint(MID + 1e-9, MID, SPREAD, 0.0, true), TradeDirection::Buy);
        assert_eq!(sign_midpoint(MID - 1e-9, MID, SPREAD, 0.0, true), TradeDirection::Sell);
    }

    #[test]
    fn test_realistic_nvda_price() {
        // Real NVDA scenario: bid=$134.56, ask=$134.57
        let mid = 134.565;
        let spread = 0.01;
        let excl = 0.10;
        // buy_threshold = 134.565 + 0.001 = 134.566
        // sell_threshold = 134.565 - 0.001 = 134.564

        // Wholesaler buy: $134.5675 > 134.566 → Buy
        assert_eq!(sign_midpoint(134.5675, mid, spread, excl, true), TradeDirection::Buy);
        // Wholesaler sell: $134.5625 < 134.564 → Sell
        assert_eq!(sign_midpoint(134.5625, mid, spread, excl, true), TradeDirection::Sell);
        // At midpoint: $134.565 → Unsigned
        assert_eq!(sign_midpoint(134.565, mid, spread, excl, true), TradeDirection::Unsigned);
    }
}
