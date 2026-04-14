//! EQUS_SUMMARY daily context loading for off-exchange processing.
//!
//! Reads Databento EQUS_SUMMARY OHLCV-1D data to provide per-day consolidated
//! volume and OHLCV prices. Used for coverage validation (`true_dark_share`)
//! and VPIN bucket volume sizing.
//!
//! Source: docs/design/06_INTEGRATION_POINTS.md §2

use std::collections::HashMap;
use std::io::BufReader;
use std::path::Path;

use chrono::NaiveDate;
use dbn::decode::{DecodeRecord, DynDecoder};
use dbn::enums::VersionUpgradePolicy;
use dbn::OhlcvMsg;

use crate::contract::NANO_TO_USD;
use crate::error::{ProcessorError, Result};

/// Per-day context from EQUS_SUMMARY (consolidated volume + OHLCV).
///
/// Loaded once per day during pipeline initialization. Read-only during processing.
/// When EQUS_SUMMARY is unavailable, `DailyContext::fallback()` provides
/// all-`None` values so the pipeline can proceed gracefully.
///
/// Source: docs/design/06_INTEGRATION_POINTS.md §2
#[derive(Debug, Clone)]
pub struct DailyContext {
    /// Trading date.
    pub date: NaiveDate,
    /// Total consolidated volume across ALL US venues (shares).
    /// `None` when EQUS_SUMMARY is unavailable for this date.
    pub consolidated_volume: Option<u64>,
    /// Daily open price (USD). Converted from i64 nanodollars via NANO_TO_USD.
    pub daily_open: Option<f64>,
    /// Daily high price (USD).
    pub daily_high: Option<f64>,
    /// Daily low price (USD).
    pub daily_low: Option<f64>,
    /// Daily close price (USD).
    pub daily_close: Option<f64>,
    // NOTE: daily_vwap omitted — not available from OHLCV-1D schema (AD3).
}

impl DailyContext {
    /// Create a fallback context when EQUS_SUMMARY is unavailable.
    ///
    /// All `Option` fields are `None`. The pipeline proceeds without
    /// EQUS-dependent features (true_dark_share, daily_range_bps).
    pub fn fallback(date: NaiveDate) -> Self {
        Self {
            date,
            consolidated_volume: None,
            daily_open: None,
            daily_high: None,
            daily_low: None,
            daily_close: None,
        }
    }

    /// Whether this context has consolidated volume data.
    pub fn has_volume(&self) -> bool {
        self.consolidated_volume.is_some()
    }
}

/// Loads EQUS_SUMMARY OHLCV-1D data and provides per-date context.
///
/// Reads the entire `.dbn.zst` file once at construction, builds a
/// `HashMap<NaiveDate, DailyContext>` for O(1) lookups per date.
///
/// # Usage
///
/// ```ignore
/// let loader = DailyContextLoader::from_file(equs_path)?;
/// let context = loader.get(NaiveDate::from_ymd_opt(2025, 2, 3).unwrap());
/// ```
pub struct DailyContextLoader {
    contexts: HashMap<NaiveDate, DailyContext>,
}

impl DailyContextLoader {
    /// Load all OHLCV-1D records from an EQUS_SUMMARY `.dbn.zst` file.
    ///
    /// Reads the entire file into memory (typically < 100 records for ~1 year).
    /// Prices are converted from i64 nanodollars to f64 USD via `NANO_TO_USD`.
    ///
    /// # Errors
    ///
    /// Returns `ProcessorError::Data` if the file cannot be read or decoded.
    pub fn from_file(path: &Path) -> Result<Self> {
        let file = std::fs::File::open(path)
            .map_err(|e| ProcessorError::data(format!(
                "Failed to open EQUS_SUMMARY file {}: {e}", path.display()
            )))?;
        let reader = BufReader::with_capacity(64 * 1024, file);
        let mut decoder = DynDecoder::inferred_with_buffer(reader, VersionUpgradePolicy::AsIs)
            .map_err(|e| ProcessorError::data(format!(
                "Failed to create EQUS_SUMMARY decoder: {e}"
            )))?;

        let mut contexts = HashMap::new();

        while let Some(record) = decoder.decode_record::<OhlcvMsg>()
            .map_err(|e| ProcessorError::data(format!(
                "Failed to decode EQUS_SUMMARY OhlcvMsg: {e}"
            )))? {
            let ts_secs = record.hd.ts_event / 1_000_000_000;
            let days_since_epoch = ts_secs / 86400;
            // Convert Unix days to NaiveDate: Unix epoch is 1970-01-01
            // which is day 719_163 in the proleptic Gregorian calendar (from CE)
            let date = NaiveDate::from_num_days_from_ce_opt(
                (days_since_epoch as i32) + 719_163,
            );
            let Some(date) = date else {
                log::warn!("EQUS_SUMMARY: invalid date from ts_event={}", record.hd.ts_event);
                continue;
            };

            let ctx = DailyContext {
                date,
                consolidated_volume: Some(record.volume),
                daily_open: Some(record.open as f64 * NANO_TO_USD),
                daily_high: Some(record.high as f64 * NANO_TO_USD),
                daily_low: Some(record.low as f64 * NANO_TO_USD),
                daily_close: Some(record.close as f64 * NANO_TO_USD),
            };
            contexts.insert(date, ctx);
        }

        log::info!("EQUS_SUMMARY: loaded {} dates from {}", contexts.len(), path.display());
        Ok(Self { contexts })
    }

    /// Create an empty loader (no EQUS data available).
    ///
    /// All calls to `get()` will return `DailyContext::fallback()`.
    pub fn empty() -> Self {
        Self {
            contexts: HashMap::new(),
        }
    }

    /// Get the daily context for a specific date.
    ///
    /// Returns the loaded context if available, or `DailyContext::fallback(date)`
    /// if the date is not in the EQUS_SUMMARY data.
    pub fn get(&self, date: NaiveDate) -> DailyContext {
        self.contexts
            .get(&date)
            .cloned()
            .unwrap_or_else(|| DailyContext::fallback(date))
    }

    /// Number of dates loaded from the EQUS_SUMMARY file.
    pub fn n_dates(&self) -> usize {
        self.contexts.len()
    }

    /// Whether any data was loaded.
    pub fn is_empty(&self) -> bool {
        self.contexts.is_empty()
    }

    /// Iterator over all loaded dates (sorted).
    pub fn dates(&self) -> Vec<NaiveDate> {
        let mut dates: Vec<_> = self.contexts.keys().copied().collect();
        dates.sort();
        dates
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2025, 2, 3).unwrap()
    }

    #[test]
    fn test_fallback_context_all_none() {
        let ctx = DailyContext::fallback(test_date());
        assert_eq!(ctx.date, test_date());
        assert!(ctx.consolidated_volume.is_none());
        assert!(ctx.daily_open.is_none());
        assert!(ctx.daily_high.is_none());
        assert!(ctx.daily_low.is_none());
        assert!(ctx.daily_close.is_none());
        assert!(!ctx.has_volume());
    }

    #[test]
    fn test_context_has_volume() {
        let mut ctx = DailyContext::fallback(test_date());
        assert!(!ctx.has_volume());
        ctx.consolidated_volume = Some(100_000_000);
        assert!(ctx.has_volume());
    }

    #[test]
    fn test_empty_loader() {
        let loader = DailyContextLoader::empty();
        assert!(loader.is_empty());
        assert_eq!(loader.n_dates(), 0);
    }

    #[test]
    fn test_empty_loader_returns_fallback() {
        let loader = DailyContextLoader::empty();
        let ctx = loader.get(test_date());
        assert!(ctx.consolidated_volume.is_none());
        assert_eq!(ctx.date, test_date());
    }

    #[test]
    fn test_get_is_idempotent() {
        let loader = DailyContextLoader::empty();
        let ctx1 = loader.get(test_date());
        let ctx2 = loader.get(test_date());
        assert_eq!(ctx1.date, ctx2.date);
        assert_eq!(ctx1.consolidated_volume, ctx2.consolidated_volume);
    }

    // ── Data-gated tests ────────────────────────────────────────────

    const EQUS_PATH: &str = "../data/EQUS_SUMMARY/NVDA/ohlcv1d_2025-02-03_to_2026-03-05/equs-summary-20250203-20260305.ohlcv-1d.dbn.zst";

    fn equs_available() -> bool {
        std::path::Path::new(EQUS_PATH).exists()
    }

    #[test]
    fn test_load_real_equs_file() {
        if !equs_available() {
            eprintln!("SKIP: EQUS_SUMMARY data not available");
            return;
        }
        let loader = DailyContextLoader::from_file(std::path::Path::new(EQUS_PATH)).unwrap();
        assert!(loader.n_dates() > 200, "Expected 200+ dates, got {}", loader.n_dates());
        assert!(!loader.is_empty());
    }

    #[test]
    fn test_known_date_has_context() {
        if !equs_available() {
            eprintln!("SKIP: EQUS_SUMMARY data not available");
            return;
        }
        let loader = DailyContextLoader::from_file(std::path::Path::new(EQUS_PATH)).unwrap();
        let ctx = loader.get(test_date());
        assert!(ctx.has_volume(), "2025-02-03 should have EQUS data");
    }

    #[test]
    fn test_volume_reasonable_range() {
        if !equs_available() {
            eprintln!("SKIP: EQUS_SUMMARY data not available");
            return;
        }
        let loader = DailyContextLoader::from_file(std::path::Path::new(EQUS_PATH)).unwrap();
        let ctx = loader.get(test_date());
        let vol = ctx.consolidated_volume.unwrap();
        assert!(
            vol > 10_000_000 && vol < 1_000_000_000,
            "NVDA daily volume {} should be 10M-1B range",
            vol
        );
    }

    #[test]
    fn test_close_price_reasonable_range() {
        if !equs_available() {
            eprintln!("SKIP: EQUS_SUMMARY data not available");
            return;
        }
        let loader = DailyContextLoader::from_file(std::path::Path::new(EQUS_PATH)).unwrap();
        let ctx = loader.get(test_date());
        let close = ctx.daily_close.unwrap();
        assert!(
            close > 50.0 && close < 500.0,
            "NVDA close price {} should be $50-$500 range",
            close
        );
    }

    #[test]
    fn test_missing_date_returns_fallback() {
        if !equs_available() {
            eprintln!("SKIP: EQUS_SUMMARY data not available");
            return;
        }
        let loader = DailyContextLoader::from_file(std::path::Path::new(EQUS_PATH)).unwrap();
        // A Saturday should not exist
        let saturday = NaiveDate::from_ymd_opt(2025, 2, 1).unwrap();
        let ctx = loader.get(saturday);
        assert!(ctx.consolidated_volume.is_none(), "Saturday should have no data");
    }

    #[test]
    fn test_dates_are_sorted() {
        if !equs_available() {
            eprintln!("SKIP: EQUS_SUMMARY data not available");
            return;
        }
        let loader = DailyContextLoader::from_file(std::path::Path::new(EQUS_PATH)).unwrap();
        let dates = loader.dates();
        for i in 1..dates.len() {
            assert!(dates[i] > dates[i - 1], "Dates should be sorted");
        }
    }
}
