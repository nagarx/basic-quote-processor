//! Trade classification: midpoint signing + BJZZ retail identification.
//!
//! `TradeClassifier` orchestrates the classification of TRF trades:
//! 1. **Direction** (Buy/Sell/Unsigned) via midpoint signing (Barber et al. 2024)
//! 2. **Retail status** (Retail/Institutional/Unknown) via BJZZ subpenny (Boehmer et al. 2021)
//!
//! # Critical Ordering Constraint
//!
//! `BboState::update_from_record()` MUST be called BEFORE `classify()`.
//! The CMBP-1 trade record carries the contemporaneous BBO; using stale BBO
//! degrades midpoint signing accuracy by 3-5% (E9 validation).
//! Rust's borrow checker enforces this: `classify()` takes `&bbo_state` (immutable).
//!
//! # BVC Separation
//!
//! BVC (Bulk Volume Classification) is defined in `bvc.rs` but NOT owned by
//! `TradeClassifier`. BVC is for aggregate volume features (bvc_imbalance, VPIN),
//! not per-trade direction. BvcState is owned by the Phase 3 accumulator.
//!
//! Source: docs/design/02_MODULE_ARCHITECTURE.md §4.3

pub mod types;
pub mod midpoint_signer;
pub mod bjzz;
pub mod bvc;

pub use types::{
    TradeDirection, RetailStatus, ClassifiedTrade,
    ClassificationConfig, SigningMethod,
};
pub use bvc::BvcState;

use crate::bbo_state::BboState;
use crate::contract::{NANO_TO_USD, UNDEF_PRICE};
use crate::error::Result;
use crate::reader::CmbpRecord;

/// Trade classifier orchestrating midpoint signing and BJZZ retail identification.
///
/// Classifies each trade record into a `ClassifiedTrade` with direction and
/// retail status. Only TRF trades (publisher_id 82/83) go through signing
/// and BJZZ; non-TRF trades receive Unsigned + Institutional.
///
/// # Usage
///
/// ```no_run
/// use basic_quote_processor::trade_classifier::{TradeClassifier, ClassificationConfig};
/// use basic_quote_processor::bbo_state::BboState;
///
/// let classifier = TradeClassifier::new(ClassificationConfig::default()).unwrap();
/// // For each trade record:
/// //   1. bbo_state.update_from_record(&record);  // mutable
/// //   2. classifier.classify(&record, &bbo_state); // immutable borrow
/// ```
pub struct TradeClassifier {
    config: ClassificationConfig,
    // Diagnostic counters (private, accessed via methods)
    total_trades: u64,
    trf_trades: u64,
    signed_buy: u64,
    signed_sell: u64,
    unsigned: u64,
    retail_count: u64,
    institutional_count: u64,
    unknown_count: u64,
    invalid_price_count: u64,
}

impl TradeClassifier {
    /// Create a new classifier with the given configuration.
    ///
    /// Validates the configuration and fails fast for unsupported signing methods.
    pub fn new(config: ClassificationConfig) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            config,
            total_trades: 0,
            trf_trades: 0,
            signed_buy: 0,
            signed_sell: 0,
            unsigned: 0,
            retail_count: 0,
            institutional_count: 0,
            unknown_count: 0,
            invalid_price_count: 0,
        })
    }

    /// Create a classifier with E9-validated default parameters.
    pub fn with_defaults() -> Result<Self> {
        Self::new(ClassificationConfig::default())
    }

    /// Classify a single trade record.
    ///
    /// # Precondition
    ///
    /// `bbo_state.update_from_record(&record)` MUST have been called before this method.
    /// The borrow checker enforces this: this method takes `&bbo_state` (immutable),
    /// so the mutable update must have completed.
    ///
    /// # Classification Logic
    ///
    /// 1. Guard: reject sentinel/invalid trade prices → Unsigned + Unknown
    /// 2. Convert trade price: `record.price as f64 * NANO_TO_USD`
    /// 3. Non-TRF trades → Unsigned + Institutional
    /// 4. TRF trades with invalid BBO → Unsigned + Unknown
    /// 5. TRF trades with valid BBO → midpoint sign + BJZZ identify
    pub fn classify(&mut self, record: &CmbpRecord, bbo_state: &BboState) -> ClassifiedTrade {
        self.total_trades += 1;

        // 0. Guard: reject sentinel/invalid trade prices
        if record.price == UNDEF_PRICE || record.price <= 0 {
            self.invalid_price_count += 1;
            self.unsigned += 1;
            self.unknown_count += 1;
            return ClassifiedTrade {
                direction: TradeDirection::Unsigned,
                retail_status: RetailStatus::Unknown,
                price: 0.0,
                size: record.size,
                publisher_id: record.publisher_id,
                ts_recv: record.ts_recv,
            };
        }

        // 1. Convert trade price: i64 nanodollars → f64 USD
        let price_usd = record.price as f64 * NANO_TO_USD;

        // 2. Non-TRF trades → Unsigned + Institutional
        if !record.publisher_class().is_trf() {
            self.unsigned += 1;
            self.institutional_count += 1;
            return ClassifiedTrade {
                direction: TradeDirection::Unsigned,
                retail_status: RetailStatus::Institutional,
                price: price_usd,
                size: record.size,
                publisher_id: record.publisher_id,
                ts_recv: record.ts_recv,
            };
        }

        // From here: TRF trade (publisher 82 or 83)
        self.trf_trades += 1;

        // 3. TRF trade with invalid BBO → Unsigned + Unknown
        if !bbo_state.is_valid {
            self.unsigned += 1;
            self.unknown_count += 1;
            return ClassifiedTrade {
                direction: TradeDirection::Unsigned,
                retail_status: RetailStatus::Unknown,
                price: price_usd,
                size: record.size,
                publisher_id: record.publisher_id,
                ts_recv: record.ts_recv,
            };
        }

        // 4. TRF trade with valid BBO → midpoint sign + BJZZ identify
        let direction = midpoint_signer::sign_midpoint(
            price_usd,
            bbo_state.mid_price,
            bbo_state.spread,
            self.config.exclusion_band,
            true, // bbo_valid already checked above
        );

        let retail_status = bjzz::identify_retail(
            price_usd,
            self.config.bjzz_lower,
            self.config.bjzz_upper_sell,
            self.config.bjzz_lower_buy,
            self.config.bjzz_upper,
        );

        // 5. Update counters
        match direction {
            TradeDirection::Buy => self.signed_buy += 1,
            TradeDirection::Sell => self.signed_sell += 1,
            TradeDirection::Unsigned => self.unsigned += 1,
        }
        match retail_status {
            RetailStatus::Retail => self.retail_count += 1,
            RetailStatus::Institutional => self.institutional_count += 1,
            RetailStatus::Unknown => self.unknown_count += 1,
        }

        ClassifiedTrade {
            direction,
            retail_status,
            price: price_usd,
            size: record.size,
            publisher_id: record.publisher_id,
            ts_recv: record.ts_recv,
        }
    }

    /// Reset classifier state for a new trading day.
    pub fn reset(&mut self) {
        self.total_trades = 0;
        self.trf_trades = 0;
        self.signed_buy = 0;
        self.signed_sell = 0;
        self.unsigned = 0;
        self.retail_count = 0;
        self.institutional_count = 0;
        self.unknown_count = 0;
        self.invalid_price_count = 0;
    }

    // ── Diagnostic accessors ──

    /// Total trades processed (all publishers).
    pub fn total_trades(&self) -> u64 { self.total_trades }
    /// TRF trades processed (publisher 82/83 only).
    pub fn trf_trades(&self) -> u64 { self.trf_trades }
    /// Trades signed as Buy.
    pub fn signed_buy(&self) -> u64 { self.signed_buy }
    /// Trades signed as Sell.
    pub fn signed_sell(&self) -> u64 { self.signed_sell }
    /// Unsigned trades (within exclusion band, invalid BBO, or non-TRF).
    pub fn unsigned(&self) -> u64 { self.unsigned }
    /// Trades identified as retail by BJZZ.
    pub fn retail_count(&self) -> u64 { self.retail_count }
    /// Trades identified as institutional.
    pub fn institutional_count(&self) -> u64 { self.institutional_count }
    /// Trades with unknown retail status (invalid BBO).
    pub fn unknown_count(&self) -> u64 { self.unknown_count }
    /// Trades with invalid prices (sentinel or non-positive).
    pub fn invalid_price_count(&self) -> u64 { self.invalid_price_count }

    /// Retail rate: retail / trf_trades. Returns 0.0 if no TRF trades.
    /// Phase 2 gate target: 45.3% +/-2% (E9 validation).
    pub fn retail_rate(&self) -> f64 {
        if self.trf_trades == 0 { 0.0 }
        else { self.retail_count as f64 / self.trf_trades as f64 }
    }

    /// Unsigned rate among TRF trades: unsigned_trf / trf_trades.
    /// Phase 2 gate target: 15.4% +/-2% (E9 validation).
    ///
    /// NOTE: This counts TRF-only unsigned trades. Non-TRF trades are always
    /// unsigned but are excluded from this rate.
    pub fn trf_unsigned_rate(&self) -> f64 {
        if self.trf_trades == 0 { return 0.0; }
        // TRF unsigned = total unsigned - non-TRF trades (which are all unsigned)
        let non_trf_trades = self.total_trades - self.trf_trades;
        let trf_unsigned = self.unsigned.saturating_sub(non_trf_trades);
        trf_unsigned as f64 / self.trf_trades as f64
    }

    /// Reference to the current configuration.
    pub fn config(&self) -> &ClassificationConfig { &self.config }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::record::tests::make_record;

    fn setup() -> (TradeClassifier, BboState) {
        let classifier = TradeClassifier::with_defaults().unwrap();
        let mut bbo = BboState::new();
        // Set up valid BBO: bid=$100.00, ask=$100.10
        let quote = make_record(b'A', 0, 0, 100_000_000_000, 100_100_000_000, 500, 300, 81);
        bbo.update_from_record(&quote);
        (classifier, bbo)
    }

    #[test]
    fn test_trf_buy() {
        let (mut classifier, bbo) = setup();
        // TRF trade above buy threshold: $100.08 > $100.06
        let trade = make_record(b'T', 100_080_000_000, 100, 100_000_000_000, 100_100_000_000, 500, 300, 82);
        let classified = classifier.classify(&trade, &bbo);
        assert_eq!(classified.direction, TradeDirection::Buy);
        assert_eq!(classified.publisher_id, 82);
    }

    #[test]
    fn test_trf_sell() {
        let (mut classifier, bbo) = setup();
        // TRF trade below sell threshold: $100.02 < $100.04
        let trade = make_record(b'T', 100_020_000_000, 100, 100_000_000_000, 100_100_000_000, 500, 300, 82);
        let classified = classifier.classify(&trade, &bbo);
        assert_eq!(classified.direction, TradeDirection::Sell);
    }

    #[test]
    fn test_trf_unsigned() {
        let (mut classifier, bbo) = setup();
        // TRF trade at midpoint: $100.05 → within exclusion band → Unsigned
        let trade = make_record(b'T', 100_050_000_000, 100, 100_000_000_000, 100_100_000_000, 500, 300, 82);
        let classified = classifier.classify(&trade, &bbo);
        assert_eq!(classified.direction, TradeDirection::Unsigned);
    }

    #[test]
    fn test_non_trf_unsigned_institutional() {
        let (mut classifier, bbo) = setup();
        // Lit trade (publisher 81) → Unsigned + Institutional
        let trade = make_record(b'T', 100_080_000_000, 100, 100_000_000_000, 100_100_000_000, 500, 300, 81);
        let classified = classifier.classify(&trade, &bbo);
        assert_eq!(classified.direction, TradeDirection::Unsigned);
        assert_eq!(classified.retail_status, RetailStatus::Institutional);
    }

    #[test]
    fn test_invalid_bbo_unknown() {
        let mut classifier = TradeClassifier::with_defaults().unwrap();
        let bbo = BboState::new(); // Not updated → is_valid = false
        let trade = make_record(b'T', 100_080_000_000, 100, 100_000_000_000, 100_100_000_000, 500, 300, 82);
        let classified = classifier.classify(&trade, &bbo);
        assert_eq!(classified.direction, TradeDirection::Unsigned);
        assert_eq!(classified.retail_status, RetailStatus::Unknown);
    }

    #[test]
    fn test_sentinel_price_rejected() {
        let (mut classifier, bbo) = setup();
        let trade = make_record(b'T', i64::MAX, 100, 100_000_000_000, 100_100_000_000, 500, 300, 82);
        let classified = classifier.classify(&trade, &bbo);
        assert_eq!(classified.direction, TradeDirection::Unsigned);
        assert_eq!(classified.retail_status, RetailStatus::Unknown);
        assert_eq!(classifier.invalid_price_count(), 1);
    }

    #[test]
    fn test_negative_price_rejected() {
        let (mut classifier, bbo) = setup();
        let trade = make_record(b'T', -100_000_000_000, 100, 100_000_000_000, 100_100_000_000, 500, 300, 82);
        let classified = classifier.classify(&trade, &bbo);
        assert_eq!(classified.direction, TradeDirection::Unsigned);
        assert_eq!(classified.retail_status, RetailStatus::Unknown);
    }

    #[test]
    fn test_retail_identification() {
        let (mut classifier, bbo) = setup();
        // TRF trade with subpenny: $100.0035 → frac_cent = 0.35 → Retail
        let trade = make_record(b'T', 100_003_500_000, 100, 100_000_000_000, 100_100_000_000, 500, 300, 82);
        let classified = classifier.classify(&trade, &bbo);
        assert_eq!(classified.retail_status, RetailStatus::Retail);
    }

    #[test]
    fn test_round_penny_institutional() {
        let (mut classifier, bbo) = setup();
        // TRF trade at round penny: $100.08 → frac_cent = 0.0 → Institutional
        let trade = make_record(b'T', 100_080_000_000, 100, 100_000_000_000, 100_100_000_000, 500, 300, 82);
        let classified = classifier.classify(&trade, &bbo);
        assert_eq!(classified.retail_status, RetailStatus::Institutional);
    }

    #[test]
    fn test_counter_tracking() {
        let (mut classifier, bbo) = setup();

        // 3 TRF trades: 1 buy, 1 sell, 1 unsigned
        classifier.classify(&make_record(b'T', 100_080_000_000, 100, 100_000_000_000, 100_100_000_000, 500, 300, 82), &bbo);
        classifier.classify(&make_record(b'T', 100_020_000_000, 100, 100_000_000_000, 100_100_000_000, 500, 300, 83), &bbo);
        classifier.classify(&make_record(b'T', 100_050_000_000, 100, 100_000_000_000, 100_100_000_000, 500, 300, 82), &bbo);

        // 1 lit trade
        classifier.classify(&make_record(b'T', 100_050_000_000, 100, 100_000_000_000, 100_100_000_000, 500, 300, 81), &bbo);

        assert_eq!(classifier.total_trades(), 4);
        assert_eq!(classifier.trf_trades(), 3);
        assert_eq!(classifier.signed_buy(), 1);
        assert_eq!(classifier.signed_sell(), 1);
    }

    #[test]
    fn test_reset() {
        let (mut classifier, bbo) = setup();
        classifier.classify(&make_record(b'T', 100_080_000_000, 100, 100_000_000_000, 100_100_000_000, 500, 300, 82), &bbo);
        assert_eq!(classifier.total_trades(), 1);

        classifier.reset();
        assert_eq!(classifier.total_trades(), 0);
        assert_eq!(classifier.trf_trades(), 0);
        assert_eq!(classifier.retail_count(), 0);
    }

    #[test]
    fn test_price_conversion_in_classified_trade() {
        let (mut classifier, bbo) = setup();
        let trade = make_record(b'T', 100_080_000_000, 200, 100_000_000_000, 100_100_000_000, 500, 300, 82);
        let classified = classifier.classify(&trade, &bbo);
        assert!((classified.price - 100.08).abs() < 1e-10, "Price should be $100.08, got {}", classified.price);
        assert_eq!(classified.size, 200);
    }

    // ── trf_unsigned_rate() arithmetic tests ──

    #[test]
    fn test_trf_unsigned_rate_normal_mix() {
        // 7 TRF trades (3 buy, 2 sell, 2 unsigned) + 3 non-TRF (all unsigned)
        // Total unsigned = 5, non_trf = 3, trf_unsigned = 2, rate = 2/7
        let (mut classifier, bbo) = setup();
        // 3 TRF buys
        for _ in 0..3 {
            classifier.classify(&make_record(b'T', 100_080_000_000, 100, 100_000_000_000, 100_100_000_000, 500, 300, 82), &bbo);
        }
        // 2 TRF sells
        for _ in 0..2 {
            classifier.classify(&make_record(b'T', 100_020_000_000, 100, 100_000_000_000, 100_100_000_000, 500, 300, 82), &bbo);
        }
        // 2 TRF unsigned (at midpoint)
        for _ in 0..2 {
            classifier.classify(&make_record(b'T', 100_050_000_000, 100, 100_000_000_000, 100_100_000_000, 500, 300, 82), &bbo);
        }
        // 3 non-TRF (lit, always unsigned)
        for _ in 0..3 {
            classifier.classify(&make_record(b'T', 100_080_000_000, 100, 100_000_000_000, 100_100_000_000, 500, 300, 81), &bbo);
        }

        assert_eq!(classifier.total_trades(), 10);
        assert_eq!(classifier.trf_trades(), 7);
        let rate = classifier.trf_unsigned_rate();
        assert!((rate - 2.0 / 7.0).abs() < 1e-10, "Expected 2/7 = {:.6}, got {:.6}", 2.0/7.0, rate);
    }

    #[test]
    fn test_trf_unsigned_rate_all_trf() {
        // 5 TRF trades (2 buy, 2 sell, 1 unsigned), 0 non-TRF → rate = 1/5
        let (mut classifier, bbo) = setup();
        classifier.classify(&make_record(b'T', 100_080_000_000, 100, 100_000_000_000, 100_100_000_000, 500, 300, 82), &bbo);
        classifier.classify(&make_record(b'T', 100_080_000_000, 100, 100_000_000_000, 100_100_000_000, 500, 300, 82), &bbo);
        classifier.classify(&make_record(b'T', 100_020_000_000, 100, 100_000_000_000, 100_100_000_000, 500, 300, 83), &bbo);
        classifier.classify(&make_record(b'T', 100_020_000_000, 100, 100_000_000_000, 100_100_000_000, 500, 300, 83), &bbo);
        classifier.classify(&make_record(b'T', 100_050_000_000, 100, 100_000_000_000, 100_100_000_000, 500, 300, 82), &bbo);

        assert_eq!(classifier.trf_trades(), 5);
        assert!((classifier.trf_unsigned_rate() - 0.2).abs() < 1e-10, "Expected 1/5 = 0.2");
    }

    #[test]
    fn test_trf_unsigned_rate_no_trf() {
        // 5 non-TRF trades → trf_trades = 0 → rate = 0.0 (guard)
        let (mut classifier, bbo) = setup();
        for _ in 0..5 {
            classifier.classify(&make_record(b'T', 100_080_000_000, 100, 100_000_000_000, 100_100_000_000, 500, 300, 81), &bbo);
        }
        assert_eq!(classifier.trf_trades(), 0);
        assert_eq!(classifier.trf_unsigned_rate(), 0.0);
    }

    #[test]
    fn test_trf_unsigned_rate_zero_trf_unsigned() {
        // 4 TRF (all signed) + 3 non-TRF → trf_unsigned = 0, rate = 0.0
        let (mut classifier, bbo) = setup();
        classifier.classify(&make_record(b'T', 100_080_000_000, 100, 100_000_000_000, 100_100_000_000, 500, 300, 82), &bbo);
        classifier.classify(&make_record(b'T', 100_080_000_000, 100, 100_000_000_000, 100_100_000_000, 500, 300, 82), &bbo);
        classifier.classify(&make_record(b'T', 100_020_000_000, 100, 100_000_000_000, 100_100_000_000, 500, 300, 83), &bbo);
        classifier.classify(&make_record(b'T', 100_020_000_000, 100, 100_000_000_000, 100_100_000_000, 500, 300, 83), &bbo);
        for _ in 0..3 {
            classifier.classify(&make_record(b'T', 100_080_000_000, 100, 100_000_000_000, 100_100_000_000, 500, 300, 81), &bbo);
        }
        assert_eq!(classifier.trf_trades(), 4);
        assert_eq!(classifier.trf_unsigned_rate(), 0.0);
    }

    #[test]
    fn test_trf_unsigned_rate_no_trades() {
        let classifier = TradeClassifier::with_defaults().unwrap();
        assert_eq!(classifier.trf_unsigned_rate(), 0.0);
    }
}
