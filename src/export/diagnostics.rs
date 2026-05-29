//! Per-day diagnostics sidecar for off-exchange exports.
//!
//! Persists the `BinAccumulator`'s `DaySummary` health counters as
//! `{day}_diagnostics.json`, mirroring the MBO pipeline's `_diagnostics.json`
//! producer-side health surface. Written inside the `DayExporter` temp-dir +
//! rename envelope, so it inherits atomic-write semantics for free.
//!
//! Rationale: the per-bin health counters (records processed, empty/warmup/gap
//! bins, TRF/lit trade + volume splits, first/last bin timestamps) are computed
//! during processing but were previously discarded at export time — recoverable
//! only by re-running the `profile_data` CLI over the full source file.
//! Persisting them makes each export self-describing and offline-auditable
//! (hft-rules §8: never silently drop diagnostics).
//!
//! Source: docs/design/06_INTEGRATION_POINTS.md §5 (parity with MBO diagnostics)

use serde::Serialize;

use crate::accumulator::DaySummary;
use crate::error::{ProcessorError, Result};

/// Schema version for the off-exchange `{day}_diagnostics.json` sidecar.
///
/// SemVer over the sidecar's JSON shape: MINOR bump on additive fields, MAJOR
/// on field rename/removal. Independent of `SCHEMA_VERSION` (the feature
/// contract) and the MBO `PRODUCER_DIAGNOSTICS_SCHEMA_VERSION` (a different
/// counter set).
pub const BASIC_DIAGNOSTICS_SCHEMA_VERSION: &str = "1.0.0";

/// Self-describing per-day diagnostics sidecar.
///
/// Wraps the `DaySummary` counters with a schema version + day id so the
/// artifact is interpretable offline without re-running the pipeline. Borrows
/// its payload (zero-copy) — constructed transiently at write time.
#[derive(Debug, Serialize)]
pub struct DiagnosticsSidecar<'a> {
    /// Sidecar JSON schema version (`BASIC_DIAGNOSTICS_SCHEMA_VERSION`).
    pub schema_version: &'static str,
    /// ISO date (YYYY-MM-DD) this sidecar describes.
    pub day: &'a str,
    /// Per-bin health counters for the day.
    pub summary: &'a DaySummary,
}

impl<'a> DiagnosticsSidecar<'a> {
    /// Build a sidecar view over a day's summary.
    pub fn new(day: &'a str, summary: &'a DaySummary) -> Self {
        Self {
            schema_version: BASIC_DIAGNOSTICS_SCHEMA_VERSION,
            day,
            summary,
        }
    }

    /// Serialize to pretty-printed JSON.
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| ProcessorError::export(format!("diagnostics JSON: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_summary() -> DaySummary {
        DaySummary {
            total_records_processed: 1_000,
            total_bins_emitted: 380,
            total_empty_bins: 12,
            warmup_bins_discarded: 3,
            gap_bins_emitted: 1,
            total_trf_trades: 200,
            total_lit_trades: 800,
            total_trade_records: 1_000,
            first_bin_start_ns: 1_700_000_000_000_000_000,
            first_bin_end_ns: 1_700_000_060_000_000_000,
            last_bin_end_ns: 1_700_023_400_000_000_000,
            total_trf_volume: 12_345.0,
            total_lit_volume: 54_321.0,
        }
    }

    #[test]
    fn test_sidecar_serializes_with_schema_and_day() {
        let summary = sample_summary();
        let sidecar = DiagnosticsSidecar::new("2025-02-03", &summary);
        let json = sidecar.to_json().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["schema_version"], BASIC_DIAGNOSTICS_SCHEMA_VERSION);
        assert_eq!(parsed["day"], "2025-02-03");
        // Representative counters round-trip through the nested summary block.
        assert_eq!(parsed["summary"]["total_bins_emitted"], 380);
        assert_eq!(parsed["summary"]["total_trf_trades"], 200);
    }

    #[test]
    fn test_sidecar_summary_exposes_all_counters() {
        let summary = sample_summary();
        let sidecar = DiagnosticsSidecar::new("2025-06-15", &summary);
        let json = sidecar.to_json().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        for field in &[
            "total_records_processed", "total_bins_emitted", "total_empty_bins",
            "warmup_bins_discarded", "gap_bins_emitted", "total_trf_trades",
            "total_lit_trades", "total_trade_records", "first_bin_start_ns",
            "first_bin_end_ns", "last_bin_end_ns", "total_trf_volume",
            "total_lit_volume",
        ] {
            assert!(
                parsed["summary"].get(field).is_some(),
                "diagnostics summary missing counter: {}", field
            );
        }
    }
}
