//! Internal representation of a CMBP-1 record.
//!
//! `CmbpRecord` is the pipeline's internal type for XNAS.BASIC records.
//! It is converted from `dbn::CbboMsg` at the reader boundary, preserving
//! prices as i64 nanodollars. Conversion to f64 USD happens at the
//! `BboState.update_from_record()` boundary (see `bbo_state/mod.rs`).
//!
//! # Price Precision Chain (Stage 2)
//!
//! ```text
//! dbn::CbboMsg (wire format, i64 nanodollars)
//!   → CmbpRecord (i64 preserved, field-by-field copy)
//!   → BboState.update_from_record() converts to f64 USD
//! ```
//!
//! Source: docs/design/03_DATA_FLOW.md §3

use super::publisher::PublisherClass;

/// Internal representation of a CMBP-1 record.
///
/// Converted from `dbn::CbboMsg` at the reader boundary.
/// Prices stored as i64 nanodollars — conversion to f64 USD happens
/// at the `BboState.update_from_record()` boundary.
///
/// # Field Mapping from `dbn::CbboMsg`
///
/// | CmbpRecord field | CbboMsg access path |
/// |------------------|---------------------|
/// | `ts_event` | `msg.hd.ts_event` |
/// | `ts_recv` | `msg.ts_recv` |
/// | `action` | `msg.action as u8` |
/// | `side` | `msg.side as u8` |
/// | `flags` | `msg.flags.raw()` |
/// | `price` | `msg.price` |
/// | `size` | `msg.size` |
/// | `bid_px` | `msg.levels[0].bid_px` |
/// | `bid_sz` | `msg.levels[0].bid_sz` |
/// | `ask_px` | `msg.levels[0].ask_px` |
/// | `ask_sz` | `msg.levels[0].ask_sz` |
/// | `publisher_id` | `msg.hd.publisher_id` |
#[derive(Debug, Clone)]
pub struct CmbpRecord {
    /// Exchange timestamp (UTC nanoseconds). From `RecordHeader.ts_event`.
    pub ts_event: u64,
    /// Capture-server-received timestamp (UTC nanoseconds). From `CbboMsg.ts_recv`.
    pub ts_recv: u64,
    /// Event action: `b'T'` = trade, `b'A'` = BBO quote update.
    pub action: u8,
    /// Event side: `b'A'` = ask, `b'B'` = bid, `b'N'` = none (TRF trades).
    pub side: u8,
    /// Event flags bitfield. Used in Phase 2+ for TRF indicator detection.
    pub flags: u8,
    /// Trade price in nanodollars (i64 fixed-point, multiply by 1e-9 for USD).
    pub price: i64,
    /// Trade/order size in shares.
    pub size: u32,
    /// Best bid price in nanodollars. From `ConsolidatedBidAskPair.bid_px`.
    pub bid_px: i64,
    /// Best bid size in shares. From `ConsolidatedBidAskPair.bid_sz`.
    pub bid_sz: u32,
    /// Best ask price in nanodollars. From `ConsolidatedBidAskPair.ask_px`.
    pub ask_px: i64,
    /// Best ask size in shares. From `ConsolidatedBidAskPair.ask_sz`.
    pub ask_sz: u32,
    /// Venue publisher ID. From `RecordHeader.publisher_id`.
    /// See `PublisherClass::from_id()` for classification.
    pub publisher_id: u16,
}

impl CmbpRecord {
    /// Convert from `dbn::CbboMsg`.
    ///
    /// Field-by-field copy is more efficient than `.clone()` because it
    /// skips the reserved bytes in `CbboMsg` and `ConsolidatedBidAskPair`.
    /// No price conversion — i64 nanodollars are preserved.
    pub fn from_cbbo(msg: &dbn::CbboMsg) -> Self {
        Self {
            ts_event: msg.hd.ts_event,
            ts_recv: msg.ts_recv,
            action: msg.action as u8,
            side: msg.side as u8,
            flags: msg.flags.raw(),
            price: msg.price,
            size: msg.size,
            bid_px: msg.levels[0].bid_px,
            bid_sz: msg.levels[0].bid_sz,
            ask_px: msg.levels[0].ask_px,
            ask_sz: msg.levels[0].ask_sz,
            publisher_id: msg.hd.publisher_id,
        }
    }

    /// True if this record is a trade (`action == 'T'`).
    #[inline]
    pub fn is_trade(&self) -> bool {
        self.action == b'T'
    }

    /// True if this record is a BBO quote update (`action == 'A'`).
    #[inline]
    pub fn is_quote(&self) -> bool {
        self.action == b'A'
    }

    /// Classify the publisher ID into a venue category.
    #[inline]
    pub fn publisher_class(&self) -> PublisherClass {
        PublisherClass::from_id(self.publisher_id)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Helper: create a CmbpRecord with specified fields for testing.
    pub fn make_record(
        action: u8,
        price: i64,
        size: u32,
        bid_px: i64,
        ask_px: i64,
        bid_sz: u32,
        ask_sz: u32,
        publisher_id: u16,
    ) -> CmbpRecord {
        CmbpRecord {
            ts_event: 1_700_000_000_000_000_000, // arbitrary UTC ns
            ts_recv: 1_700_000_000_000_100_000,   // 100µs after ts_event
            action,
            side: b'N',
            flags: 0,
            price,
            size,
            bid_px,
            bid_sz,
            ask_px,
            ask_sz,
            publisher_id,
        }
    }

    #[test]
    fn test_is_trade() {
        let record = make_record(b'T', 100_000_000_000, 100, 0, 0, 0, 0, 82);
        assert!(record.is_trade());
        assert!(!record.is_quote());
    }

    #[test]
    fn test_is_quote() {
        let record = make_record(b'A', 0, 0, 100_000_000_000, 100_010_000_000, 500, 300, 81);
        assert!(record.is_quote());
        assert!(!record.is_trade());
    }

    #[test]
    fn test_publisher_class() {
        assert_eq!(
            make_record(b'T', 0, 0, 0, 0, 0, 0, 82).publisher_class(),
            PublisherClass::Trf
        );
        assert_eq!(
            make_record(b'A', 0, 0, 0, 0, 0, 0, 81).publisher_class(),
            PublisherClass::Lit
        );
        assert_eq!(
            make_record(b'A', 0, 0, 0, 0, 0, 0, 93).publisher_class(),
            PublisherClass::QuoteOnly
        );
    }

    #[test]
    fn test_timestamps_distinct() {
        let record = make_record(b'T', 100_000_000_000, 100, 0, 0, 0, 0, 82);
        assert_ne!(
            record.ts_event, record.ts_recv,
            "ts_event and ts_recv should be distinct timestamps"
        );
    }

    #[test]
    fn test_price_preserved_as_nanodollars() {
        let price_nano: i64 = 100_235_000_000; // $100.235
        let record = make_record(b'T', price_nano, 100, 0, 0, 0, 0, 82);
        assert_eq!(
            record.price, price_nano,
            "Price must be preserved as i64 nanodollars"
        );
    }
}
