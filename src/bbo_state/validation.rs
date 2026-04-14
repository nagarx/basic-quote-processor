//! BBO validity checks and staleness measurement.
//!
//! Pure functions for validating BBO state. No side effects, no state.
//!
//! Source: docs/design/02_MODULE_ARCHITECTURE.md §4.2

/// Check whether a bid/ask pair forms a valid BBO.
///
/// Valid BBO requires:
/// - Both prices finite (not NaN, not Inf)
/// - Both prices positive (> 0.0)
/// - Positive spread (ask > bid, strictly — locked BBOs are rejected)
///
/// Source: docs/design/03_DATA_FLOW.md §7
#[inline]
pub fn is_valid_bbo(bid: f64, ask: f64) -> bool {
    bid.is_finite() && ask.is_finite() && bid > 0.0 && ask > 0.0 && ask > bid
}

/// Compute staleness in nanoseconds between last BBO update and current time.
///
/// Uses saturating subtraction to avoid underflow if timestamps are misordered.
#[inline]
pub fn staleness_ns(last_update_ts: u64, current_ts: u64) -> u64 {
    current_ts.saturating_sub(last_update_ts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_bbo() {
        assert!(is_valid_bbo(100.0, 100.01));
    }

    #[test]
    fn test_crossed_bbo_invalid() {
        // ask < bid (crossed)
        assert!(!is_valid_bbo(100.01, 100.0));
    }

    #[test]
    fn test_locked_bbo_invalid() {
        // ask == bid (zero spread)
        assert!(!is_valid_bbo(100.0, 100.0));
    }

    #[test]
    fn test_zero_bid_invalid() {
        assert!(!is_valid_bbo(0.0, 100.0));
    }

    #[test]
    fn test_zero_ask_invalid() {
        assert!(!is_valid_bbo(100.0, 0.0));
    }

    #[test]
    fn test_nan_bid_invalid() {
        assert!(!is_valid_bbo(f64::NAN, 100.0));
    }

    #[test]
    fn test_nan_ask_invalid() {
        assert!(!is_valid_bbo(100.0, f64::NAN));
    }

    #[test]
    fn test_inf_bid_invalid() {
        assert!(!is_valid_bbo(f64::INFINITY, 100.0));
    }

    #[test]
    fn test_inf_ask_invalid() {
        assert!(!is_valid_bbo(100.0, f64::INFINITY));
    }

    #[test]
    fn test_neg_inf_invalid() {
        assert!(!is_valid_bbo(f64::NEG_INFINITY, 100.0));
    }

    #[test]
    fn test_negative_bid_invalid() {
        assert!(!is_valid_bbo(-1.0, 100.0));
    }

    #[test]
    fn test_minimal_valid_spread() {
        // 1 nanodollar spread in USD
        let bid = 100.0;
        let ask = 100.0 + 1e-9;
        assert!(is_valid_bbo(bid, ask));
    }

    #[test]
    fn test_staleness_normal() {
        assert_eq!(staleness_ns(1000, 2000), 1000);
    }

    #[test]
    fn test_staleness_zero() {
        assert_eq!(staleness_ns(1000, 1000), 0);
    }

    #[test]
    fn test_staleness_saturating() {
        // current < last (misordered): saturates to 0, no underflow
        assert_eq!(staleness_ns(2000, 1000), 0);
    }
}
