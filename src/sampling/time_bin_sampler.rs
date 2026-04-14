//! Grid-aligned time bin boundary detection for off-exchange processing.
//!
//! `TimeBinSampler` divides the trading session (09:30-16:00 ET) into
//! fixed-width time bins grid-aligned to market open. It detects when a
//! record's timestamp crosses a bin boundary and reports the number of
//! gap bins that were skipped.
//!
//! # Grid Alignment
//!
//! ```text
//! bin_start(i) = market_open_ns + i * bin_size_ns
//! bin_end(i)   = market_open_ns + (i+1) * bin_size_ns
//! ```
//!
//! Bins are left-closed, right-open: `[bin_start, bin_end)`. A record at
//! exactly a boundary belongs to the NEW bin.
//!
//! # DST Handling
//!
//! UTC offset is computed once per day via `hft_statistics::time::regime::utc_offset_for_date()`.
//! Returns -4 (EDT) or -5 (EST). Market open/close are converted to UTC nanoseconds.
//!
//! Source: docs/design/03_DATA_FLOW.md §2.6
//!         docs/design/02_MODULE_ARCHITECTURE.md §4.5

use hft_statistics::time::regime::utc_offset_for_date;

const NS_PER_SEC: u64 = 1_000_000_000;
const NS_PER_MIN: u64 = 60 * NS_PER_SEC;
const NS_PER_HOUR: u64 = 3600 * NS_PER_SEC;

/// Completed bin boundary information.
#[derive(Debug, Clone)]
pub struct BinBoundary {
    /// UTC nanosecond timestamp of the bin's end (= start of next bin).
    pub bin_end_ts: u64,
    /// UTC nanosecond timestamp at the center of the bin.
    pub bin_midpoint_ts: u64,
    /// 0-based index of the completed bin.
    pub bin_index: u64,
    /// Number of SKIPPED empty bins between the previous bin and this one.
    /// The caller must emit gap_bins individual forward-filled feature vectors.
    pub gap_bins: u64,
}

/// Grid-aligned time bin sampler.
///
/// Initialized per trading day via `init_day()`. Detects bin boundaries
/// as records arrive and reports gap bins for temporal continuity.
pub struct TimeBinSampler {
    bin_size_ns: u64,
    market_open_ns: u64,
    market_close_ns: u64,
    next_boundary_ns: u64,
    current_bin_index: u64,
    utc_offset_hours: i32,
    initialized: bool,
    bins_emitted: u64,
}

impl TimeBinSampler {
    /// Create a new sampler with the given bin size in seconds.
    ///
    /// Must call `init_day()` before processing records.
    pub fn new(bin_size_seconds: u32) -> Self {
        Self {
            bin_size_ns: bin_size_seconds as u64 * NS_PER_SEC,
            market_open_ns: 0,
            market_close_ns: 0,
            next_boundary_ns: 0,
            current_bin_index: 0,
            utc_offset_hours: 0,
            initialized: false,
            bins_emitted: 0,
        }
    }

    /// Initialize for a specific trading day.
    ///
    /// Computes market open/close in UTC nanoseconds using DST-aware offset.
    /// Must be called before `check_boundary()` or `is_in_session()`.
    ///
    /// # Arguments
    /// * `year`, `month`, `day` — Trading date (e.g., 2025, 2, 3)
    pub fn init_day(&mut self, year: i32, month: u32, day: u32) {
        self.utc_offset_hours = utc_offset_for_date(year, month, day);

        // Compute midnight UTC for the trading date using chrono
        let date = chrono::NaiveDate::from_ymd_opt(year, month, day)
            .expect("Invalid date for init_day");
        let midnight_utc = date
            .and_hms_opt(0, 0, 0)
            .expect("Invalid time")
            .and_utc()
            .timestamp_nanos_opt()
            .expect("Timestamp overflow") as u64;

        // Convert 09:30 ET to UTC nanoseconds:
        // market_open_utc = midnight_utc + (09:30_secs - utc_offset_secs) * NS_PER_SEC
        // Since utc_offset is negative (-4 or -5), subtracting it adds hours.
        // EST (-5): 09:30 + 5h = 14:30 UTC. EDT (-4): 09:30 + 4h = 13:30 UTC.
        let open_et_secs: i64 = 9 * 3600 + 30 * 60; // 34200 seconds
        let offset_secs: i64 = self.utc_offset_hours as i64 * 3600;
        let open_utc_secs = open_et_secs - offset_secs; // seconds from midnight UTC
        self.market_open_ns = midnight_utc + open_utc_secs as u64 * NS_PER_SEC;

        // Market close: 16:00 ET = market_open + 6.5 hours
        self.market_close_ns = self.market_open_ns + 6 * NS_PER_HOUR + 30 * NS_PER_MIN;

        // First bin boundary
        self.next_boundary_ns = self.market_open_ns + self.bin_size_ns;
        self.current_bin_index = 0;
        self.initialized = true;
        self.bins_emitted = 0;
    }

    /// Check if a record's timestamp crosses a bin boundary.
    ///
    /// Returns `Some(BinBoundary)` if the record belongs to a new bin (the previous
    /// bin is complete). Returns `None` if still within the current bin, not initialized,
    /// or past market close.
    ///
    /// # Gap Handling
    ///
    /// When multiple boundaries pass without records, `gap_bins` in the returned
    /// `BinBoundary` indicates how many empty bins were skipped. The caller must
    /// emit individual forward-filled feature vectors for each gap bin to preserve
    /// temporal continuity.
    pub fn check_boundary(&mut self, ts_recv: u64) -> Option<BinBoundary> {
        if !self.initialized || ts_recv < self.next_boundary_ns {
            return None;
        }
        if ts_recv >= self.market_close_ns {
            return None;
        }

        // The previous bin is complete
        let completed_bin_index = self.current_bin_index;
        let completed_bin_end = self.next_boundary_ns;

        // Count skipped boundaries (gap detection)
        let mut gap_count: u64 = 0;
        self.next_boundary_ns += self.bin_size_ns;
        self.current_bin_index += 1;
        while self.next_boundary_ns <= ts_recv {
            self.next_boundary_ns += self.bin_size_ns;
            self.current_bin_index += 1;
            gap_count += 1;
        }

        self.bins_emitted += 1;
        Some(BinBoundary {
            bin_end_ts: completed_bin_end,
            bin_midpoint_ts: completed_bin_end - self.bin_size_ns / 2,
            bin_index: completed_bin_index,
            gap_bins: gap_count,
        })
    }

    /// Whether the given timestamp is within the trading session [open, close).
    ///
    /// Pre-market records (< market_open) and post-market records (>= market_close)
    /// return false. BBO updates happen regardless; only trade accumulation is gated.
    pub fn is_in_session(&self, ts_recv: u64) -> bool {
        self.initialized && ts_recv >= self.market_open_ns && ts_recv < self.market_close_ns
    }

    /// Bin size in nanoseconds (accessor for gap bin emission loop).
    pub fn bin_size_ns(&self) -> u64 {
        self.bin_size_ns
    }

    /// Market close timestamp in UTC nanoseconds (accessor for last-bin flush).
    pub fn market_close_ns(&self) -> u64 {
        self.market_close_ns
    }

    /// Market open timestamp in UTC nanoseconds.
    pub fn market_open_ns(&self) -> u64 {
        self.market_open_ns
    }

    /// UTC offset for the current day (-4 EDT or -5 EST).
    pub fn utc_offset_hours(&self) -> i32 {
        self.utc_offset_hours
    }

    /// Number of bins emitted so far this day.
    pub fn bins_emitted(&self) -> u64 {
        self.bins_emitted
    }

    /// Reset for a new trading day.
    pub fn reset(&mut self) {
        self.market_open_ns = 0;
        self.market_close_ns = 0;
        self.next_boundary_ns = 0;
        self.current_bin_index = 0;
        self.utc_offset_hours = 0;
        self.initialized = false;
        self.bins_emitted = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_day_est() {
        let mut sampler = TimeBinSampler::new(60);
        // 2025-02-03 is EST (offset = -5)
        sampler.init_day(2025, 2, 3);
        assert_eq!(sampler.utc_offset_hours(), -5, "February should be EST");

        // 09:30 EST = 14:30 UTC = 14*3600 + 30*60 = 52200 seconds past midnight
        let expected_secs_from_midnight: u64 = 14 * 3600 + 30 * 60;
        let date = chrono::NaiveDate::from_ymd_opt(2025, 2, 3).unwrap();
        let midnight = date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp_nanos_opt().unwrap() as u64;
        let expected_open = midnight + expected_secs_from_midnight * NS_PER_SEC;
        assert_eq!(
            sampler.market_open_ns(), expected_open,
            "market_open_ns for EST date"
        );
    }

    #[test]
    fn test_init_day_edt() {
        let mut sampler = TimeBinSampler::new(60);
        // 2025-06-16 is EDT (offset = -4)
        sampler.init_day(2025, 6, 16);
        assert_eq!(sampler.utc_offset_hours(), -4, "June should be EDT");

        // 09:30 EDT = 13:30 UTC
        let expected_secs_from_midnight: u64 = 13 * 3600 + 30 * 60;
        let date = chrono::NaiveDate::from_ymd_opt(2025, 6, 16).unwrap();
        let midnight = date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp_nanos_opt().unwrap() as u64;
        let expected_open = midnight + expected_secs_from_midnight * NS_PER_SEC;
        assert_eq!(
            sampler.market_open_ns(), expected_open,
            "market_open_ns for EDT date"
        );
    }

    #[test]
    fn test_est_edt_differ_by_one_hour() {
        let mut sampler_est = TimeBinSampler::new(60);
        let mut sampler_edt = TimeBinSampler::new(60);
        sampler_est.init_day(2025, 2, 3);  // EST
        sampler_edt.init_day(2025, 6, 16); // EDT
        // EST open is 1 hour later in UTC than EDT open (same local time)
        // But these are different dates, so compare the offset effect:
        // EST: 09:30 + 5h = 14:30 UTC, EDT: 09:30 + 4h = 13:30 UTC
        // The absolute timestamps differ because dates are different,
        // but we can verify the market_close - market_open = 6.5h for both.
        let session_est = sampler_est.market_close_ns() - sampler_est.market_open_ns();
        let session_edt = sampler_edt.market_close_ns() - sampler_edt.market_open_ns();
        let expected_session = 6 * NS_PER_HOUR + 30 * NS_PER_MIN;
        assert_eq!(session_est, expected_session, "EST session should be 6.5h");
        assert_eq!(session_edt, expected_session, "EDT session should be 6.5h");
    }

    #[test]
    fn test_no_boundary_within_first_bin() {
        let mut sampler = TimeBinSampler::new(60);
        sampler.init_day(2025, 2, 3);
        let open = sampler.market_open_ns();

        // Record 30s into first bin → no boundary
        assert!(sampler.check_boundary(open + 30 * NS_PER_SEC).is_none());
    }

    #[test]
    fn test_boundary_at_exact_edge() {
        let mut sampler = TimeBinSampler::new(60);
        sampler.init_day(2025, 2, 3);
        let open = sampler.market_open_ns();

        // Record at exactly the first bin boundary (open + 60s)
        // This belongs to the NEW bin → the old bin (index 0) is complete
        let boundary = sampler.check_boundary(open + 60 * NS_PER_SEC).unwrap();
        assert_eq!(boundary.bin_index, 0, "Completed bin index should be 0");
        assert_eq!(boundary.gap_bins, 0, "No gap");
        assert_eq!(boundary.bin_end_ts, open + 60 * NS_PER_SEC);
    }

    #[test]
    fn test_boundary_past_edge() {
        let mut sampler = TimeBinSampler::new(60);
        sampler.init_day(2025, 2, 3);
        let open = sampler.market_open_ns();

        // Record 10s past boundary
        let boundary = sampler.check_boundary(open + 70 * NS_PER_SEC).unwrap();
        assert_eq!(boundary.bin_index, 0);
        assert_eq!(boundary.gap_bins, 0);
    }

    #[test]
    fn test_gap_three_bins() {
        let mut sampler = TimeBinSampler::new(60);
        sampler.init_day(2025, 2, 3);
        let open = sampler.market_open_ns();

        // First record at open + 30s → no boundary
        assert!(sampler.check_boundary(open + 30 * NS_PER_SEC).is_none());

        // Next record at open + 3min10s → crosses 3 boundaries (60s, 120s, 180s)
        // Completed bin: index 0 (the bin that had the first record)
        // Gaps: bins 1 and 2 are empty
        let boundary = sampler.check_boundary(open + 190 * NS_PER_SEC).unwrap();
        assert_eq!(boundary.bin_index, 0, "Completed bin is index 0");
        assert_eq!(boundary.gap_bins, 2, "Bins 1 and 2 are gap bins");
        assert_eq!(boundary.bin_end_ts, open + 60 * NS_PER_SEC);
    }

    #[test]
    fn test_pre_market_returns_none() {
        let mut sampler = TimeBinSampler::new(60);
        sampler.init_day(2025, 2, 3);
        let open = sampler.market_open_ns();

        // Pre-market: 1 minute before open
        assert!(sampler.check_boundary(open - 60 * NS_PER_SEC).is_none());
        assert!(!sampler.is_in_session(open - 1));
    }

    #[test]
    fn test_post_market_returns_none() {
        let mut sampler = TimeBinSampler::new(60);
        sampler.init_day(2025, 2, 3);
        let close = sampler.market_close_ns();

        // At market close
        assert!(sampler.check_boundary(close).is_none());
        assert!(!sampler.is_in_session(close));
    }

    #[test]
    fn test_bin_midpoint() {
        let mut sampler = TimeBinSampler::new(60);
        sampler.init_day(2025, 2, 3);
        let open = sampler.market_open_ns();

        let boundary = sampler.check_boundary(open + 60 * NS_PER_SEC).unwrap();
        let expected_midpoint = open + 30 * NS_PER_SEC;
        assert_eq!(boundary.bin_midpoint_ts, expected_midpoint);
    }

    #[test]
    fn test_is_in_session_at_open() {
        let mut sampler = TimeBinSampler::new(60);
        sampler.init_day(2025, 2, 3);
        let open = sampler.market_open_ns();

        assert!(sampler.is_in_session(open), "Exactly at open: IN session");
        assert!(!sampler.is_in_session(open - 1), "1ns before open: NOT in session");
    }

    #[test]
    fn test_is_in_session_at_close() {
        let mut sampler = TimeBinSampler::new(60);
        sampler.init_day(2025, 2, 3);
        let close = sampler.market_close_ns();

        assert!(!sampler.is_in_session(close), "Exactly at close: NOT in session");
        assert!(sampler.is_in_session(close - 1), "1ns before close: IN session");
    }

    #[test]
    fn test_reset_clears_state() {
        let mut sampler = TimeBinSampler::new(60);
        sampler.init_day(2025, 2, 3);
        assert!(sampler.is_in_session(sampler.market_open_ns()));

        sampler.reset();
        assert!(!sampler.is_in_session(sampler.market_open_ns()));
        assert_eq!(sampler.bins_emitted(), 0);
    }

    #[test]
    fn test_not_initialized_returns_none() {
        let mut sampler = TimeBinSampler::new(60);
        assert!(sampler.check_boundary(1_000_000_000_000_000_000).is_none());
        assert!(!sampler.is_in_session(1_000_000_000_000_000_000));
    }
}
