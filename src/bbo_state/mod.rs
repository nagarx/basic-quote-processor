//! L1 book state tracking: Nasdaq best bid/offer from CMBP-1 records.
//!
//! `BboState` maintains the current Nasdaq BBO, updated from every record
//! (both quote updates and trade records carry embedded BBO fields).
//!
//! # Critical Ordering Constraint
//!
//! For trade records (`action == 'T'`), `update_from_record()` MUST be called
//! BEFORE `TradeClassifier.classify()` (Phase 2). The CMBP-1 trade record
//! carries the contemporaneous BBO; using a stale BBO degrades midpoint
//! signing accuracy by 3-5% (E9 validation).
//!
//! Rust's borrow checker enforces this: `classify()` takes `&bbo_state`
//! (immutable), so the mutable update must complete first.
//!
//! Source: docs/design/02_MODULE_ARCHITECTURE.md §4.2

pub mod midpoint;
pub mod validation;

use crate::contract::{EPS, NANO_TO_USD, UNDEF_PRICE};
use crate::reader::CmbpRecord;

/// Nasdaq L1 BBO state tracker.
///
/// Updated from every `CmbpRecord` (both quote updates and trade records).
/// Rejects crossed BBOs (ask <= bid) and sentinel/invalid prices.
///
/// # Fields
///
/// Public fields provide read access for `TradeClassifier` and `FeatureExtractor`.
/// Diagnostic counters are private with accessor methods.
///
/// Source: docs/design/03_DATA_FLOW.md §3
#[derive(Debug, Clone)]
pub struct BboState {
    // ── Prices (f64 USD, converted from i64 nanodollars at update boundary) ──
    /// Current best bid price (USD).
    pub bid_price: f64,
    /// Current best bid size (shares).
    pub bid_size: u32,
    /// Current best ask price (USD).
    pub ask_price: f64,
    /// Current best ask size (shares).
    pub ask_size: u32,

    // ── Derived values (recomputed on each valid update) ──
    /// Midpoint: `(bid + ask) / 2.0` (USD).
    pub mid_price: f64,
    /// Spread: `ask - bid` (USD). Always > 0 when `is_valid`.
    pub spread: f64,
    /// Microprice: size-weighted midpoint (USD).
    pub microprice: f64,

    // ── Metadata ──
    /// Timestamp of last valid BBO update (UTC nanoseconds).
    pub last_update_ts: u64,
    /// True if current BBO is valid: spread > 0, both prices finite and positive.
    pub is_valid: bool,

    // ── Diagnostic counters (private, accessed via methods) ──
    crossed_count: u64,
    invalid_count: u64,
    update_count: u64,
}

impl BboState {
    /// Create a new BboState with all fields zeroed and `is_valid = false`.
    pub fn new() -> Self {
        Self {
            bid_price: 0.0,
            bid_size: 0,
            ask_price: 0.0,
            ask_size: 0,
            mid_price: 0.0,
            spread: 0.0,
            microprice: 0.0,
            last_update_ts: 0,
            is_valid: false,
            crossed_count: 0,
            invalid_count: 0,
            update_count: 0,
        }
    }

    /// Update BBO state from a CMBP-1 record's embedded BBO fields.
    ///
    /// Returns `true` if the update was applied, `false` if rejected.
    ///
    /// Rejection causes (with counter increment):
    /// - Sentinel prices (`i64::MAX`) or non-positive prices → `invalid_count`
    /// - Non-finite f64 after conversion → `invalid_count`
    /// - Crossed or locked BBO (`ask <= bid`) → `crossed_count`
    ///
    /// # Price Conversion
    ///
    /// i64 nanodollars → f64 USD happens here (ONCE, at this boundary).
    /// `price_usd = price_nanodollars as f64 * NANO_TO_USD`
    ///
    /// Source: docs/design/03_DATA_FLOW.md §4
    pub fn update_from_record(&mut self, record: &CmbpRecord) -> bool {
        let bid_px = record.bid_px;
        let ask_px = record.ask_px;

        // Reject sentinel/invalid prices (Stage 2 → Stage 3 boundary guard)
        if bid_px == UNDEF_PRICE || bid_px <= 0 || ask_px == UNDEF_PRICE || ask_px <= 0 {
            self.invalid_count += 1;
            return false;
        }

        // Convert i64 nanodollars → f64 USD (ONCE, at this boundary)
        let bid_f64 = bid_px as f64 * NANO_TO_USD;
        let ask_f64 = ask_px as f64 * NANO_TO_USD;

        // Paranoia guard: reject non-finite (shouldn't happen from valid i64)
        if !bid_f64.is_finite() || !ask_f64.is_finite() {
            self.invalid_count += 1;
            return false;
        }

        // Reject crossed/locked BBO (ask must be strictly greater than bid)
        if ask_f64 <= bid_f64 {
            self.crossed_count += 1;
            return false;
        }

        // ── Apply update ──
        self.bid_price = bid_f64;
        self.bid_size = record.bid_sz;
        self.ask_price = ask_f64;
        self.ask_size = record.ask_sz;
        self.last_update_ts = record.ts_recv;
        self.update_count += 1;

        // ── Recompute derived values ──
        self.mid_price = (bid_f64 + ask_f64) / 2.0;
        self.spread = ask_f64 - bid_f64;

        let size_sum = (self.bid_size as f64) + (self.ask_size as f64);
        self.microprice = if size_sum > EPS {
            (bid_f64 * self.ask_size as f64 + ask_f64 * self.bid_size as f64) / size_sum
        } else {
            self.mid_price
        };

        self.is_valid = true;
        true
    }

    /// Spread in basis points.
    ///
    /// `(ask - bid) / mid * 10000`
    pub fn spread_bps(&self) -> f64 {
        midpoint::spread_bps(self.bid_price, self.ask_price)
    }

    /// Reset BBO state for a new trading day.
    ///
    /// Zeroes all price/size fields and sets `is_valid = false`.
    /// Diagnostic counters are **preserved** for day-end logging.
    /// Call `reset_counters()` separately at the start of a new day.
    pub fn reset(&mut self) {
        self.bid_price = 0.0;
        self.bid_size = 0;
        self.ask_price = 0.0;
        self.ask_size = 0;
        self.mid_price = 0.0;
        self.spread = 0.0;
        self.microprice = 0.0;
        self.last_update_ts = 0;
        self.is_valid = false;
    }

    // ── Diagnostic counter accessors ──

    /// Number of crossed/locked BBO rejections.
    pub fn crossed_count(&self) -> u64 {
        self.crossed_count
    }

    /// Number of invalid price rejections (sentinel, non-positive, non-finite).
    pub fn invalid_count(&self) -> u64 {
        self.invalid_count
    }

    /// Number of valid BBO updates applied.
    pub fn update_count(&self) -> u64 {
        self.update_count
    }

    /// Reset diagnostic counters. Call at the start of each new trading day.
    pub fn reset_counters(&mut self) {
        self.crossed_count = 0;
        self.invalid_count = 0;
        self.update_count = 0;
    }
}

impl Default for BboState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::record::tests::make_record;

    #[test]
    fn test_new_state() {
        let bbo = BboState::new();
        assert!(!bbo.is_valid, "New BboState should be invalid");
        assert_eq!(bbo.bid_price, 0.0);
        assert_eq!(bbo.ask_price, 0.0);
        assert_eq!(bbo.mid_price, 0.0);
        assert_eq!(bbo.spread, 0.0);
        assert_eq!(bbo.update_count(), 0);
    }

    #[test]
    fn test_single_update() {
        let mut bbo = BboState::new();
        // bid = $100.00, ask = $100.10
        let record = make_record(
            b'A',
            0,
            0,
            100_000_000_000, // bid = $100.00
            100_100_000_000, // ask = $100.10
            500,
            300,
            81,
        );
        assert!(bbo.update_from_record(&record));
        assert!(bbo.is_valid);
        assert!((bbo.bid_price - 100.0).abs() < 1e-12);
        assert!((bbo.ask_price - 100.10).abs() < 1e-12);
        assert!((bbo.mid_price - 100.05).abs() < 1e-12);
        assert!((bbo.spread - 0.10).abs() < 1e-12);
        assert_eq!(bbo.bid_size, 500);
        assert_eq!(bbo.ask_size, 300);
        assert_eq!(bbo.update_count(), 1);
    }

    #[test]
    fn test_sequential_updates() {
        let mut bbo = BboState::new();

        // First update: bid=$100, ask=$100.10
        let r1 = make_record(b'A', 0, 0, 100_000_000_000, 100_100_000_000, 500, 300, 81);
        assert!(bbo.update_from_record(&r1));

        // Second update: bid=$100.05, ask=$100.15
        let r2 = make_record(b'A', 0, 0, 100_050_000_000, 100_150_000_000, 600, 400, 81);
        assert!(bbo.update_from_record(&r2));

        assert!((bbo.bid_price - 100.05).abs() < 1e-12);
        assert!((bbo.ask_price - 100.15).abs() < 1e-12);
        assert!((bbo.mid_price - 100.10).abs() < 1e-12);
        assert_eq!(bbo.bid_size, 600);
        assert_eq!(bbo.ask_size, 400);
        assert_eq!(bbo.update_count(), 2);
    }

    #[test]
    fn test_crossed_bbo_rejected() {
        let mut bbo = BboState::new();
        // First: establish valid BBO
        let valid = make_record(b'A', 0, 0, 100_000_000_000, 100_100_000_000, 500, 300, 81);
        assert!(bbo.update_from_record(&valid));

        // Crossed BBO: ask ($100.00) < bid ($100.10)
        let crossed = make_record(b'A', 0, 0, 100_100_000_000, 100_000_000_000, 500, 300, 81);
        assert!(!bbo.update_from_record(&crossed));
        assert_eq!(bbo.crossed_count(), 1);

        // Previous valid state preserved
        assert!(bbo.is_valid);
        assert!((bbo.bid_price - 100.0).abs() < 1e-12, "Previous bid should be preserved");
    }

    #[test]
    fn test_locked_bbo_rejected() {
        let mut bbo = BboState::new();
        // Locked: ask == bid (zero spread)
        let locked = make_record(b'A', 0, 0, 100_000_000_000, 100_000_000_000, 500, 300, 81);
        assert!(!bbo.update_from_record(&locked));
        assert_eq!(bbo.crossed_count(), 1);
        assert!(!bbo.is_valid);
    }

    #[test]
    fn test_sentinel_price_rejected() {
        let mut bbo = BboState::new();
        let sentinel = make_record(b'A', 0, 0, i64::MAX, 100_100_000_000, 500, 300, 81);
        assert!(!bbo.update_from_record(&sentinel));
        assert_eq!(bbo.invalid_count(), 1);
    }

    #[test]
    fn test_zero_price_rejected() {
        let mut bbo = BboState::new();
        let zero_bid = make_record(b'A', 0, 0, 0, 100_100_000_000, 500, 300, 81);
        assert!(!bbo.update_from_record(&zero_bid));
        assert_eq!(bbo.invalid_count(), 1);
    }

    #[test]
    fn test_negative_price_rejected() {
        let mut bbo = BboState::new();
        let neg = make_record(b'A', 0, 0, -100_000_000_000, 100_100_000_000, 500, 300, 81);
        assert!(!bbo.update_from_record(&neg));
        assert_eq!(bbo.invalid_count(), 1);
    }

    #[test]
    fn test_reset() {
        let mut bbo = BboState::new();
        let record = make_record(b'A', 0, 0, 100_000_000_000, 100_100_000_000, 500, 300, 81);
        bbo.update_from_record(&record);
        assert!(bbo.is_valid);

        bbo.reset();
        assert!(!bbo.is_valid);
        assert_eq!(bbo.bid_price, 0.0);
        assert_eq!(bbo.mid_price, 0.0);
        // Counters preserved after reset
        assert_eq!(bbo.update_count(), 1, "Counters should persist after reset");
    }

    #[test]
    fn test_reset_counters() {
        let mut bbo = BboState::new();
        let record = make_record(b'A', 0, 0, 100_000_000_000, 100_100_000_000, 500, 300, 81);
        bbo.update_from_record(&record);
        assert_eq!(bbo.update_count(), 1);

        bbo.reset_counters();
        assert_eq!(bbo.update_count(), 0);
        assert_eq!(bbo.crossed_count(), 0);
        assert_eq!(bbo.invalid_count(), 0);
    }

    #[test]
    fn test_microprice_computed() {
        let mut bbo = BboState::new();
        // bid=$100, ask=$100.10, bid_sz=100, ask_sz=900
        // microprice = (100*900 + 100.10*100) / (100+900) = 100010/1000 = 100.01
        let record = make_record(b'A', 0, 0, 100_000_000_000, 100_100_000_000, 100, 900, 81);
        bbo.update_from_record(&record);
        assert!(
            (bbo.microprice - 100.01).abs() < 1e-12,
            "Microprice should be 100.01, got {}",
            bbo.microprice
        );
    }

    #[test]
    fn test_spread_bps_accessor() {
        let mut bbo = BboState::new();
        // bid=$100, ask=$100.01 → ~1.0 bps
        let record = make_record(b'A', 0, 0, 100_000_000_000, 100_010_000_000, 500, 500, 81);
        bbo.update_from_record(&record);
        let bps = bbo.spread_bps();
        assert!(
            (bps - 0.999950002499875).abs() < 1e-10,
            "Spread should be ~1.0 bps, got {}",
            bps
        );
    }

    #[test]
    fn test_golden_bbo_values() {
        // Golden test: pinned to 10 decimal places
        let mut bbo = BboState::new();
        // bid = $134.560000000 (134_560_000_000 nanodollars)
        // ask = $134.570000000 (134_570_000_000 nanodollars)
        let record = make_record(
            b'A', 0, 0,
            134_560_000_000,
            134_570_000_000,
            1200,
            800,
            81,
        );
        bbo.update_from_record(&record);

        // mid = (134.56 + 134.57) / 2 = 134.565
        assert!(
            (bbo.mid_price - 134.565).abs() < 1e-10,
            "Golden mid: expected 134.565, got {:.10}",
            bbo.mid_price
        );

        // spread = 134.57 - 134.56 = 0.01
        assert!(
            (bbo.spread - 0.01).abs() < 1e-10,
            "Golden spread: expected 0.01, got {:.10}",
            bbo.spread
        );

        // microprice = (134.56 * 800 + 134.57 * 1200) / (1200 + 800)
        //            = (107648 + 161484) / 2000 = 269132 / 2000 = 134.566
        assert!(
            (bbo.microprice - 134.566).abs() < 1e-10,
            "Golden microprice: expected 134.566, got {:.10}",
            bbo.microprice
        );

        // spread_bps = 0.01 / 134.565 * 10000 = 0.7431...
        let expected_bps = 0.01 / 134.565 * 10_000.0;
        assert!(
            (bbo.spread_bps() - expected_bps).abs() < 1e-8,
            "Golden spread_bps: expected {:.10}, got {:.10}",
            expected_bps,
            bbo.spread_bps()
        );
    }
}
