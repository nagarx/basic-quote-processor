//! Stateless BBO computation helpers: midpoint, spread, microprice.
//!
//! Pure functions with no side effects. Usable by both `BboState` and
//! `TradeClassifier` (Phase 2).
//!
//! Source: docs/design/04_FEATURE_SPECIFICATION.md §4

use crate::contract::EPS;

/// Compute BBO midpoint: `(bid + ask) / 2.0`.
///
/// Returns 0.0 if either price is non-finite.
#[inline]
pub fn midpoint(bid: f64, ask: f64) -> f64 {
    if !bid.is_finite() || !ask.is_finite() {
        return 0.0;
    }
    (bid + ask) / 2.0
}

/// Spread in basis points: `(ask - bid) / mid * 10000`.
///
/// Returns 0.0 if midpoint is near zero (below EPS).
///
/// Formula: standard market microstructure definition.
#[inline]
pub fn spread_bps(bid: f64, ask: f64) -> f64 {
    let mid = (bid + ask) / 2.0;
    if mid < EPS {
        return 0.0;
    }
    (ask - bid) / mid * 10_000.0
}

/// Microprice: size-weighted midpoint.
///
/// `microprice = (bid * ask_size + ask * bid_size) / (bid_size + ask_size)`
///
/// When both sizes are zero, falls back to simple midpoint.
/// Uses `EPS` guard on denominator to prevent division by zero.
///
/// Source: docs/design/04_FEATURE_SPECIFICATION.md §5.4
#[inline]
pub fn microprice(bid: f64, ask: f64, bid_sz: u32, ask_sz: u32) -> f64 {
    let denom = (bid_sz as f64) + (ask_sz as f64);
    if denom < EPS {
        return (bid + ask) / 2.0;
    }
    (bid * ask_sz as f64 + ask * bid_sz as f64) / denom
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_midpoint_exact() {
        // bid = $100.00, ask = $100.10 → mid = $100.05
        let mid = midpoint(100.0, 100.10);
        assert!(
            (mid - 100.05).abs() < 1e-15,
            "Midpoint should be exactly 100.05, got {}",
            mid
        );
    }

    #[test]
    fn test_midpoint_nan_input() {
        assert_eq!(midpoint(f64::NAN, 100.0), 0.0);
        assert_eq!(midpoint(100.0, f64::NAN), 0.0);
    }

    #[test]
    fn test_spread_bps_exact() {
        // bid = $100.00, ask = $100.10
        // spread = $0.10, mid = $100.05
        // spread_bps = 0.10 / 100.05 * 10000 = 9.99500249875...
        let bps = spread_bps(100.0, 100.10);
        assert!(
            (bps - 9.995002498750625).abs() < 1e-10,
            "Spread BPS should be ~9.995, got {}",
            bps
        );
    }

    #[test]
    fn test_spread_bps_one_cent() {
        // bid = $100.00, ask = $100.01 (1 tick = $0.01)
        // spread_bps = 0.01 / 100.005 * 10000 = 0.99995...
        let bps = spread_bps(100.0, 100.01);
        assert!(
            (bps - 0.999950002499875).abs() < 1e-10,
            "1-tick spread should be ~1.0 bps, got {}",
            bps
        );
    }

    #[test]
    fn test_spread_bps_zero_mid() {
        assert_eq!(spread_bps(0.0, 0.0), 0.0);
    }

    #[test]
    fn test_microprice_equal_sizes() {
        // Equal sizes → microprice = midpoint
        let mp = microprice(100.0, 100.10, 500, 500);
        let mid = midpoint(100.0, 100.10);
        assert!(
            (mp - mid).abs() < 1e-15,
            "Equal sizes: microprice should equal midpoint"
        );
    }

    #[test]
    fn test_microprice_asymmetric_sizes() {
        // bid_sz=100, ask_sz=900 → microprice weighted toward bid
        // mp = (100*900 + 100.10*100) / (100+900) = (90000 + 10010) / 1000 = 100.01
        let mp = microprice(100.0, 100.10, 100, 900);
        assert!(
            (mp - 100.01).abs() < 1e-12,
            "Asymmetric sizes: microprice should be 100.01, got {}",
            mp
        );
    }

    #[test]
    fn test_microprice_zero_sizes() {
        // Both sizes 0 → falls back to simple midpoint
        let mp = microprice(100.0, 100.10, 0, 0);
        let mid = midpoint(100.0, 100.10);
        assert!(
            (mp - mid).abs() < 1e-15,
            "Zero sizes: microprice should fall back to midpoint"
        );
    }

    #[test]
    fn test_microprice_one_side_zero() {
        // bid_sz=0, ask_sz=1000 → mp = (100*1000 + 100.10*0) / 1000 = 100.0
        let mp = microprice(100.0, 100.10, 0, 1000);
        assert!(
            (mp - 100.0).abs() < 1e-15,
            "Only ask size: microprice should equal bid price, got {}",
            mp
        );
    }
}
