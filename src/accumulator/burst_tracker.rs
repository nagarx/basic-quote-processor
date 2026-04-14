//! TRF burst detection and inter-arrival time analysis.
//!
//! Produces two independent outputs:
//! 1. `trf_burst_intensity` (index 24): Per-bin coefficient of variation of TRF
//!    inter-arrival times. FLOW feature — resets each bin.
//! 2. `time_since_burst` (index 25): Cross-bin seconds since last detected burst.
//!    ALWAYS-COMPUTED from persistent state — never forward-filled.
//!
//! A burst is defined as `burst_threshold` or more TRF trades within a 1-second
//! window (configurable via `burst_window_ns`).
//!
//! # Burst Detection Algorithm (O(n) Two-Pointer)
//!
//! Since `bin_arrivals` is sorted (timestamps arrive in order), we use a two-pointer
//! sliding window for O(n) burst detection instead of O(n²) nested loops.
//!
//! Source: docs/design/04_FEATURE_SPECIFICATION.md §5.7

use crate::contract::EPS;

/// TRF burst tracker with per-bin CV and cross-bin burst detection.
///
/// `bin_arrivals` resets each bin. `last_burst_ts` persists across bins within
/// a trading day but resets at day boundary.
#[derive(Debug, Clone)]
pub(crate) struct BurstTracker {
    /// TRF trade arrival timestamps within the current bin (UTC nanoseconds).
    /// Sorted by arrival order (monotonic within a bin).
    bin_arrivals: Vec<u64>,

    /// Timestamp of last detected burst (UTC nanoseconds). 0 = no burst ever.
    /// Persists across bins, resets at day boundary.
    last_burst_ts: u64,

    /// Minimum TRF trades within `burst_window_ns` to constitute a burst.
    burst_threshold: u32,

    /// Burst detection window in nanoseconds (default: 1 second = 1_000_000_000).
    burst_window_ns: u64,
}

impl BurstTracker {
    /// Create with configurable burst threshold.
    ///
    /// # Arguments
    /// * `burst_threshold` — Minimum TRF trades per 1-second window for burst. Default: 20.
    pub(crate) fn new(burst_threshold: u32) -> Self {
        Self {
            bin_arrivals: Vec::with_capacity(128),
            last_burst_ts: 0,
            burst_threshold,
            burst_window_ns: 1_000_000_000, // 1 second
        }
    }

    /// Record a TRF trade arrival timestamp.
    pub(crate) fn record_arrival(&mut self, ts_ns: u64) {
        self.bin_arrivals.push(ts_ns);
    }

    /// Coefficient of variation of TRF inter-arrival times within the current bin.
    ///
    /// Returns 0.0 if fewer than 2 TRF trades (need at least 1 inter-arrival interval).
    /// CV > 1.0 indicates bursty arrivals; CV < 1.0 indicates regular arrivals.
    /// Poisson process has CV = 1.0.
    ///
    /// Formula: CV = std(deltas) / max(mean(deltas), EPS)
    /// Source: 04_FEATURE_SPECIFICATION.md §5.7 (index 24)
    pub(crate) fn compute_burst_intensity(&self) -> f64 {
        let n = self.bin_arrivals.len();
        if n < 2 {
            return 0.0;
        }

        // Compute inter-arrival deltas
        let n_deltas = n - 1;
        let mut sum: f64 = 0.0;
        let mut sum_sq: f64 = 0.0;

        for i in 0..n_deltas {
            let delta = self.bin_arrivals[i + 1].saturating_sub(self.bin_arrivals[i]) as f64;
            sum += delta;
            sum_sq += delta * delta;
        }

        let mean = sum / n_deltas as f64;
        if mean < EPS {
            return 0.0; // All trades at same timestamp → CV = 0
        }

        // Variance = E[X^2] - E[X]^2 (population variance for the bin's deltas)
        let variance = (sum_sq / n_deltas as f64) - (mean * mean);
        let std = variance.max(0.0).sqrt(); // max(0.0) guards against floating-point underflow

        std / mean.max(EPS)
    }

    /// Detect bursts using O(n) two-pointer sliding window.
    ///
    /// Scans `bin_arrivals` (sorted) for any 1-second window containing
    /// `>= burst_threshold` trades. If found, updates `last_burst_ts`.
    /// Must be called at bin boundary BEFORE `reset_bin()`.
    pub(crate) fn check_and_update_burst(&mut self) {
        let n = self.bin_arrivals.len();
        if n < self.burst_threshold as usize {
            return; // Not enough trades for any burst
        }

        let mut left: usize = 0;
        for right in 0..n {
            // Advance left pointer until window fits within burst_window_ns
            while self.bin_arrivals[right].saturating_sub(self.bin_arrivals[left]) > self.burst_window_ns {
                left += 1;
            }
            // Check if window contains enough trades
            if (right - left + 1) >= self.burst_threshold as usize {
                self.last_burst_ts = self.bin_arrivals[right];
            }
        }
    }

    /// Seconds since last detected burst.
    ///
    /// If no burst has ever been detected (`last_burst_ts == 0`), returns
    /// `warmup_bins * bin_size_secs` (capped at warmup period, per spec).
    /// Otherwise returns `(current_ts - last_burst_ts)` in seconds.
    ///
    /// This method always recomputes from persistent state — it is NOT
    /// forward-filled. Time naturally increments between bins.
    ///
    /// Source: 04_FEATURE_SPECIFICATION.md §5.7 (index 25)
    pub(crate) fn time_since_burst_secs(
        &self,
        current_ts: u64,
        warmup_bins: u32,
        bin_size_secs: u32,
    ) -> f64 {
        if self.last_burst_ts == 0 {
            // No burst ever detected → cap at warmup period
            (warmup_bins as f64) * (bin_size_secs as f64)
        } else {
            current_ts.saturating_sub(self.last_burst_ts) as f64 / 1e9
        }
    }

    /// Reset per-bin state. Preserves cross-bin burst detection state.
    pub(crate) fn reset_bin(&mut self) {
        self.bin_arrivals.clear();
        // last_burst_ts, burst_threshold, burst_window_ns PERSIST
    }

    /// Reset all state for a new trading day.
    pub(crate) fn reset_day(&mut self) {
        self.bin_arrivals.clear();
        self.last_burst_ts = 0;
    }

    /// Number of TRF arrivals in the current bin.
    #[cfg(test)]
    pub(crate) fn bin_arrival_count(&self) -> usize {
        self.bin_arrivals.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NS_PER_MS: u64 = 1_000_000;
    const NS_PER_SEC: u64 = 1_000_000_000;

    #[test]
    fn test_empty_intensity_zero() {
        let tracker = BurstTracker::new(20);
        assert_eq!(tracker.compute_burst_intensity(), 0.0);
    }

    #[test]
    fn test_single_arrival_intensity_zero() {
        let mut tracker = BurstTracker::new(20);
        tracker.record_arrival(1_000_000_000);
        assert_eq!(
            tracker.compute_burst_intensity(),
            0.0,
            "Need >= 2 arrivals for inter-arrival times"
        );
    }

    #[test]
    fn test_regular_arrivals_low_cv() {
        let mut tracker = BurstTracker::new(20);
        // 10 trades every 100ms → perfectly regular
        for i in 0..10 {
            tracker.record_arrival(i * 100 * NS_PER_MS);
        }
        let cv = tracker.compute_burst_intensity();
        assert!(
            cv < 0.01,
            "Regular arrivals (100ms apart): CV should be ~0, got {}",
            cv
        );
    }

    #[test]
    fn test_bursty_arrivals_high_cv() {
        let mut tracker = BurstTracker::new(20);
        // Alternating: 10ms then 500ms → bursty pattern
        let mut ts: u64 = 0;
        for i in 0..20 {
            tracker.record_arrival(ts);
            ts += if i % 2 == 0 { 10 * NS_PER_MS } else { 500 * NS_PER_MS };
        }
        let cv = tracker.compute_burst_intensity();
        assert!(
            cv > 1.0,
            "Bursty arrivals (10ms/500ms alternating): CV should be > 1.0, got {}",
            cv
        );
    }

    #[test]
    fn test_all_same_timestamp_cv_zero() {
        let mut tracker = BurstTracker::new(20);
        // All trades at the same timestamp → mean_iat = 0, CV = 0
        for _ in 0..5 {
            tracker.record_arrival(1_000_000_000);
        }
        assert_eq!(
            tracker.compute_burst_intensity(),
            0.0,
            "All same timestamp: CV should be 0.0 (guarded by EPS)"
        );
    }

    #[test]
    fn test_burst_detected() {
        let mut tracker = BurstTracker::new(20);
        // 25 trades in 500ms → burst (>= 20 in 1s window)
        for i in 0..25 {
            tracker.record_arrival(1_000_000_000 + i * 20 * NS_PER_MS); // 20ms apart
        }
        assert_eq!(tracker.last_burst_ts, 0, "No burst before check");
        tracker.check_and_update_burst();
        assert_ne!(
            tracker.last_burst_ts, 0,
            "25 trades in 500ms should trigger burst"
        );
    }

    #[test]
    fn test_no_burst_below_threshold() {
        let mut tracker = BurstTracker::new(20);
        // 5 trades in 500ms → no burst (< 20 in 1s window)
        for i in 0..5 {
            tracker.record_arrival(1_000_000_000 + i * 100 * NS_PER_MS);
        }
        tracker.check_and_update_burst();
        assert_eq!(
            tracker.last_burst_ts, 0,
            "5 trades should NOT trigger burst (threshold=20)"
        );
    }

    #[test]
    fn test_time_since_burst_warmup_cap() {
        let tracker = BurstTracker::new(20);
        // No burst ever → returns warmup_bins * bin_size_secs
        let time = tracker.time_since_burst_secs(5_000_000_000, 3, 60);
        assert_eq!(
            time, 180.0,
            "No burst ever: should return warmup_cap = 3 * 60 = 180s, got {}",
            time
        );
    }

    #[test]
    fn test_time_since_burst_after_detection() {
        let mut tracker = BurstTracker::new(20);
        // Create a burst at t=2s
        for i in 0..25 {
            tracker.record_arrival(2 * NS_PER_SEC + i * 10 * NS_PER_MS);
        }
        tracker.check_and_update_burst();
        assert_ne!(tracker.last_burst_ts, 0);

        // Check time_since_burst at t=5s → should be ~3s
        let time = tracker.time_since_burst_secs(5 * NS_PER_SEC, 3, 60);
        assert!(
            (time - 3.0).abs() < 0.5,
            "3s after burst: time_since_burst should be ~3.0, got {}",
            time
        );
    }

    #[test]
    fn test_time_since_burst_increments_across_bins() {
        let mut tracker = BurstTracker::new(20);
        // Create burst at t=10s
        for i in 0..25 {
            tracker.record_arrival(10 * NS_PER_SEC + i * 10 * NS_PER_MS);
        }
        tracker.check_and_update_burst();
        tracker.reset_bin(); // End of bin — burst state persists

        // Bin+1: t=70s → 60s later
        let time1 = tracker.time_since_burst_secs(70 * NS_PER_SEC, 3, 60);

        // Bin+2: t=130s → 120s later (empty/gap bin)
        let time2 = tracker.time_since_burst_secs(130 * NS_PER_SEC, 3, 60);

        assert!(
            time2 > time1,
            "time_since_burst must INCREMENT across bins: t1={}, t2={}",
            time1, time2
        );
        assert!(
            (time1 - 60.0).abs() < 1.0,
            "1 bin later: expected ~60s, got {}",
            time1
        );
        assert!(
            (time2 - 120.0).abs() < 1.0,
            "2 bins later: expected ~120s, got {}",
            time2
        );
    }

    #[test]
    fn test_reset_bin_preserves_burst_state() {
        let mut tracker = BurstTracker::new(20);
        for i in 0..25 {
            tracker.record_arrival(1 * NS_PER_SEC + i * 10 * NS_PER_MS);
        }
        tracker.check_and_update_burst();
        let burst_ts = tracker.last_burst_ts;
        assert_ne!(burst_ts, 0);

        tracker.reset_bin();
        assert_eq!(tracker.bin_arrival_count(), 0, "bin_arrivals should be cleared");
        assert_eq!(
            tracker.last_burst_ts, burst_ts,
            "last_burst_ts must persist across bins"
        );
    }

    #[test]
    fn test_reset_day_clears_everything() {
        let mut tracker = BurstTracker::new(20);
        for i in 0..25 {
            tracker.record_arrival(1 * NS_PER_SEC + i * 10 * NS_PER_MS);
        }
        tracker.check_and_update_burst();
        assert_ne!(tracker.last_burst_ts, 0);

        tracker.reset_day();
        assert_eq!(tracker.bin_arrival_count(), 0);
        assert_eq!(tracker.last_burst_ts, 0, "last_burst_ts must be cleared on day reset");
    }
}
