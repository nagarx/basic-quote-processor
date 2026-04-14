//! Dataset manifest for multi-day exports.
//!
//! Written to `dataset_manifest.json` at the export root directory.
//! Tracks per-split statistics, failed days, and completion status.
//!
//! Source: docs/design/06_INTEGRATION_POINTS.md §5

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::contract::{CONTRACT_VERSION, SCHEMA_VERSION, TOTAL_FEATURES};
use crate::error::{ProcessorError, Result};

/// Complete manifest for a multi-day dataset export.
///
/// Written incrementally during export (`complete: false`) and finalized
/// on successful completion (`complete: true`). Enables detection of
/// partial exports.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetManifest {
    pub experiment: String,
    pub symbol: String,
    pub data_source: String,
    pub schema: String,
    pub feature_count: usize,
    pub schema_version: String,
    pub contract_version: String,
    pub days_processed: usize,
    pub total_sequences: usize,
    pub sequence_length: usize,
    pub stride: usize,
    pub bin_size_seconds: u32,
    pub labeling_strategy: String,
    pub horizons: Vec<usize>,
    pub normalization: String,
    pub export_timestamp: String,
    pub config_hash: String,
    pub processor_version: String,
    /// False during export, true on successful completion.
    pub complete: bool,
    pub splits: SplitsInfo,
}

/// Per-split statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplitsInfo {
    pub train: SplitDetail,
    pub val: SplitDetail,
    pub test: SplitDetail,
}

/// Detail for one split (train, val, or test).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplitDetail {
    pub days: Vec<String>,
    pub n_days: usize,
    pub n_sequences: usize,
    pub date_range: [String; 2],
    pub failed_days: Vec<FailedDay>,
}

/// Record of a day that failed during export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailedDay {
    pub date: String,
    pub error: String,
}

impl SplitDetail {
    /// Create an empty split detail.
    pub fn empty() -> Self {
        Self {
            days: Vec::new(),
            n_days: 0,
            n_sequences: 0,
            date_range: [String::new(), String::new()],
            failed_days: Vec::new(),
        }
    }

    /// Record a successfully exported day.
    pub fn record_day(&mut self, date: &str, n_sequences: usize) {
        self.days.push(date.to_string());
        self.n_days += 1;
        self.n_sequences += n_sequences;
        if self.date_range[0].is_empty() || date < self.date_range[0].as_str() {
            self.date_range[0] = date.to_string();
        }
        if self.date_range[1].is_empty() || date > self.date_range[1].as_str() {
            self.date_range[1] = date.to_string();
        }
    }

    /// Record a failed day.
    pub fn record_failure(&mut self, date: &str, error: &str) {
        self.failed_days.push(FailedDay {
            date: date.to_string(),
            error: error.to_string(),
        });
    }
}

impl DatasetManifest {
    /// Create a new manifest with initial values from config.
    pub fn new(
        experiment: &str,
        symbol: &str,
        sequence_length: usize,
        stride: usize,
        bin_size_seconds: u32,
        horizons: Vec<usize>,
        normalization: &str,
    ) -> Self {
        Self {
            experiment: experiment.to_string(),
            symbol: symbol.to_string(),
            data_source: "XNAS.BASIC".to_string(),
            schema: "cmbp-1".to_string(),
            feature_count: TOTAL_FEATURES,
            schema_version: format!("{:.1}", SCHEMA_VERSION),
            contract_version: CONTRACT_VERSION.to_string(),
            days_processed: 0,
            total_sequences: 0,
            sequence_length,
            stride,
            bin_size_seconds,
            labeling_strategy: "point_return".to_string(),
            horizons,
            normalization: normalization.to_string(),
            export_timestamp: chrono::Utc::now().to_rfc3339(),
            config_hash: String::new(),
            processor_version: env!("CARGO_PKG_VERSION").to_string(),
            complete: false,
            splits: SplitsInfo {
                train: SplitDetail::empty(),
                val: SplitDetail::empty(),
                test: SplitDetail::empty(),
            },
        }
    }

    /// Set the config hash (SHA-256 or similar of the serialized config).
    pub fn set_config_hash(&mut self, hash: &str) {
        self.config_hash = hash.to_string();
    }

    /// Update total counts from split details.
    pub fn update_totals(&mut self) {
        self.days_processed = self.splits.train.n_days
            + self.splits.val.n_days
            + self.splits.test.n_days;
        self.total_sequences = self.splits.train.n_sequences
            + self.splits.val.n_sequences
            + self.splits.test.n_sequences;
    }

    /// Mark the manifest as complete.
    pub fn mark_complete(&mut self) {
        self.complete = true;
        self.update_totals();
        self.export_timestamp = chrono::Utc::now().to_rfc3339();
    }

    /// Serialize to pretty-printed JSON.
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| ProcessorError::export(format!("manifest JSON: {e}")))
    }

    /// Write manifest to a file.
    pub fn write_to_file(&self, path: &Path) -> Result<()> {
        let json = self.to_json()?;
        std::fs::write(path, json)
            .map_err(|e| ProcessorError::export(format!(
                "Failed to write manifest {}: {e}", path.display()
            )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_manifest() -> DatasetManifest {
        DatasetManifest::new(
            "test_experiment",
            "NVDA",
            20,
            1,
            60,
            vec![1, 5, 10],
            "none",
        )
    }

    #[test]
    fn test_manifest_serializes_to_json() {
        let manifest = test_manifest();
        let json = manifest.to_json().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["experiment"], "test_experiment");
        assert_eq!(parsed["symbol"], "NVDA");
        assert_eq!(parsed["complete"], false);
        assert_eq!(parsed["feature_count"], TOTAL_FEATURES);
        assert_eq!(parsed["labeling_strategy"], "point_return");
    }

    #[test]
    fn test_manifest_required_fields() {
        let manifest = test_manifest();
        let json = manifest.to_json().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        for field in &[
            "experiment", "symbol", "data_source", "schema",
            "feature_count", "schema_version", "contract_version",
            "days_processed", "total_sequences", "sequence_length",
            "stride", "bin_size_seconds", "labeling_strategy",
            "horizons", "normalization", "export_timestamp",
            "processor_version", "complete", "splits",
        ] {
            assert!(parsed.get(field).is_some(), "Missing field: {}", field);
        }
    }

    #[test]
    fn test_split_detail_records() {
        let mut split = SplitDetail::empty();
        assert_eq!(split.n_days, 0);

        split.record_day("2025-02-03", 308);
        split.record_day("2025-02-04", 305);
        assert_eq!(split.n_days, 2);
        assert_eq!(split.n_sequences, 613);
        assert_eq!(split.date_range[0], "2025-02-03");
        assert_eq!(split.date_range[1], "2025-02-04");
    }

    #[test]
    fn test_split_detail_failures() {
        let mut split = SplitDetail::empty();
        split.record_failure("2025-02-05", "file not found");
        assert_eq!(split.failed_days.len(), 1);
        assert_eq!(split.failed_days[0].date, "2025-02-05");
    }

    #[test]
    fn test_manifest_mark_complete() {
        let mut manifest = test_manifest();
        manifest.splits.train.record_day("2025-02-03", 100);
        manifest.splits.val.record_day("2025-10-01", 50);
        manifest.splits.test.record_day("2025-11-14", 30);
        manifest.mark_complete();

        assert!(manifest.complete);
        assert_eq!(manifest.days_processed, 3);
        assert_eq!(manifest.total_sequences, 180);
    }

    #[test]
    fn test_manifest_roundtrip() {
        let manifest = test_manifest();
        let json = manifest.to_json().unwrap();
        let parsed: DatasetManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.experiment, manifest.experiment);
        assert_eq!(parsed.feature_count, manifest.feature_count);
    }
}
