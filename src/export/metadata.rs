//! Export metadata JSON for off-exchange feature exports.
//!
//! Contains ALL spec-required fields from 03_DATA_FLOW.md §2.5
//! and 06_INTEGRATION_POINTS.md §5.2.
//!
//! Source: docs/design/06_INTEGRATION_POINTS.md §5.2

use std::path::Path;

use serde::Serialize;

use crate::contract::{CONTRACT_VERSION, SCHEMA_VERSION, TOTAL_FEATURES};
use crate::error::{ProcessorError, Result};

/// Complete export metadata for one day.
///
/// Every field listed in 03_DATA_FLOW.md §2.5 and 06_INTEGRATION_POINTS.md §5.2.
#[derive(Debug, Clone, Serialize)]
pub struct ExportMetadata {
    // ── Core identifiers ─────────────────────────────────────────────
    pub day: String,
    pub n_sequences: usize,
    pub window_size: usize,
    pub n_features: usize,
    pub schema_version: String,
    pub contract_version: String,
    pub label_strategy: String,
    pub label_encoding: String,
    pub horizons: Vec<usize>,
    pub bin_size_seconds: u32,
    pub market_open_et: String,

    // ── Normalization ────────────────────────────────────────────────
    pub normalization: NormalizationMeta,

    // ── Provenance ───────────────────────────────────────────────────
    pub provenance: ProvenanceMeta,
    pub export_timestamp: String,

    // ── Bin statistics ───────────────────────────────────────────────
    /// UTC nanoseconds since epoch. First emitted bin's start time.
    pub first_bin_start_ns: u64,
    /// UTC nanoseconds since epoch. Last emitted bin's end time.
    pub last_bin_end_ns: u64,
    pub n_bins_total: usize,
    pub n_bins_valid: usize,
    pub n_bins_warmup_discarded: usize,
    pub n_bins_label_truncated: usize,

    // ── Trade statistics ─────────────────────────────────────────────
    pub n_total_records: u64,
    pub n_trade_records: u64,
    pub n_trf_trades: u64,
    pub n_lit_trades: u64,

    // ── Data source ──────────────────────────────────────────────────
    pub data_source: String,
    pub schema: String,
    pub symbol: String,

    // ── EQUS context ─────────────────────────────────────────────────
    pub equs_summary_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consolidated_volume: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trf_volume_fraction: Option<f64>,

    // ── Config snapshot ──────────────────────────────────────────────
    pub feature_groups_enabled: serde_json::Value,
    pub classification_config: serde_json::Value,
    pub signing_method: String,
    pub exclusion_band: f64,
}

/// Normalization metadata embedded in export metadata.
#[derive(Debug, Clone, Serialize)]
pub struct NormalizationMeta {
    pub strategy: String,
    pub applied: bool,
    pub params_file: String,
}

/// Provenance metadata for reproducibility.
#[derive(Debug, Clone, Serialize)]
pub struct ProvenanceMeta {
    pub source_file: String,
    pub processor_version: String,
    pub export_timestamp_utc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_hash: Option<String>,
}

impl ExportMetadata {
    /// Serialize to pretty-printed JSON string.
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| ProcessorError::export(format!("metadata JSON: {e}")))
    }

    /// Write metadata JSON to a file.
    pub fn write_to_file(&self, path: &Path) -> Result<()> {
        let json = self.to_json()?;
        std::fs::write(path, json)
            .map_err(|e| ProcessorError::export(format!("Failed to write {}: {e}", path.display())))
    }
}

/// Builder for ExportMetadata with defaults for constant fields.
pub struct ExportMetadataBuilder {
    day: Option<String>,
    n_sequences: Option<usize>,
    window_size: Option<usize>,
    horizons: Option<Vec<usize>>,
    bin_size_seconds: Option<u32>,
    market_open_et: Option<String>,
    normalization_applied: bool,
    normalization_params_file: Option<String>,
    provenance_source_file: Option<String>,
    export_timestamp: Option<String>,
    first_bin_start_ns: u64,
    last_bin_end_ns: u64,
    n_bins_total: usize,
    n_bins_valid: usize,
    n_bins_warmup_discarded: usize,
    n_bins_label_truncated: usize,
    n_total_records: u64,
    n_trade_records: u64,
    n_trf_trades: u64,
    n_lit_trades: u64,
    symbol: Option<String>,
    equs_summary_available: bool,
    consolidated_volume: Option<u64>,
    trf_volume_fraction: Option<f64>,
    feature_groups_enabled: Option<serde_json::Value>,
    classification_config: Option<serde_json::Value>,
    signing_method: Option<String>,
    exclusion_band: f64,
    config_hash: Option<String>,
}

impl ExportMetadataBuilder {
    pub fn new() -> Self {
        Self {
            day: None,
            n_sequences: None,
            window_size: None,
            horizons: None,
            bin_size_seconds: None,
            market_open_et: None,
            normalization_applied: false,
            normalization_params_file: None,
            provenance_source_file: None,
            export_timestamp: None,
            first_bin_start_ns: 0,
            last_bin_end_ns: 0,
            n_bins_total: 0,
            n_bins_valid: 0,
            n_bins_warmup_discarded: 0,
            n_bins_label_truncated: 0,
            n_total_records: 0,
            n_trade_records: 0,
            n_trf_trades: 0,
            n_lit_trades: 0,
            symbol: None,
            equs_summary_available: false,
            consolidated_volume: None,
            trf_volume_fraction: None,
            feature_groups_enabled: None,
            classification_config: None,
            signing_method: None,
            exclusion_band: 0.10,
            config_hash: None,
        }
    }

    pub fn day(mut self, day: &str) -> Self { self.day = Some(day.to_string()); self }
    pub fn n_sequences(mut self, n: usize) -> Self { self.n_sequences = Some(n); self }
    pub fn window_size(mut self, w: usize) -> Self { self.window_size = Some(w); self }
    pub fn horizons(mut self, h: Vec<usize>) -> Self { self.horizons = Some(h); self }
    pub fn bin_size_seconds(mut self, b: u32) -> Self { self.bin_size_seconds = Some(b); self }
    pub fn market_open_et(mut self, m: &str) -> Self { self.market_open_et = Some(m.to_string()); self }
    pub fn normalization_applied(mut self, a: bool) -> Self { self.normalization_applied = a; self }
    pub fn normalization_params_file(mut self, f: &str) -> Self { self.normalization_params_file = Some(f.to_string()); self }
    pub fn provenance_source_file(mut self, f: &str) -> Self { self.provenance_source_file = Some(f.to_string()); self }
    pub fn export_timestamp(mut self, t: &str) -> Self { self.export_timestamp = Some(t.to_string()); self }
    pub fn first_bin_start_ns(mut self, ns: u64) -> Self { self.first_bin_start_ns = ns; self }
    pub fn last_bin_end_ns(mut self, ns: u64) -> Self { self.last_bin_end_ns = ns; self }
    pub fn n_bins_total(mut self, n: usize) -> Self { self.n_bins_total = n; self }
    pub fn n_bins_valid(mut self, n: usize) -> Self { self.n_bins_valid = n; self }
    pub fn n_bins_warmup_discarded(mut self, n: usize) -> Self { self.n_bins_warmup_discarded = n; self }
    pub fn n_bins_label_truncated(mut self, n: usize) -> Self { self.n_bins_label_truncated = n; self }
    pub fn n_total_records(mut self, n: u64) -> Self { self.n_total_records = n; self }
    pub fn n_trade_records(mut self, n: u64) -> Self { self.n_trade_records = n; self }
    pub fn n_trf_trades(mut self, n: u64) -> Self { self.n_trf_trades = n; self }
    pub fn n_lit_trades(mut self, n: u64) -> Self { self.n_lit_trades = n; self }
    pub fn symbol(mut self, s: &str) -> Self { self.symbol = Some(s.to_string()); self }
    pub fn equs_summary_available(mut self, a: bool) -> Self { self.equs_summary_available = a; self }
    pub fn consolidated_volume(mut self, v: Option<u64>) -> Self { self.consolidated_volume = v; self }
    pub fn trf_volume_fraction(mut self, f: Option<f64>) -> Self { self.trf_volume_fraction = f; self }
    pub fn feature_groups_enabled(mut self, v: serde_json::Value) -> Self { self.feature_groups_enabled = Some(v); self }
    pub fn classification_config(mut self, v: serde_json::Value) -> Self { self.classification_config = Some(v); self }
    pub fn signing_method(mut self, m: &str) -> Self { self.signing_method = Some(m.to_string()); self }
    pub fn exclusion_band(mut self, b: f64) -> Self { self.exclusion_band = b; self }
    pub fn config_hash(mut self, h: &str) -> Self { self.config_hash = Some(h.to_string()); self }

    /// Build the ExportMetadata. Validates that required fields are set.
    pub fn build(self) -> Result<ExportMetadata> {
        let day = self.day.ok_or_else(|| ProcessorError::export("day is required"))?;
        let n_sequences = self.n_sequences.ok_or_else(|| ProcessorError::export("n_sequences is required"))?;
        let window_size = self.window_size.ok_or_else(|| ProcessorError::export("window_size is required"))?;
        let horizons = self.horizons.ok_or_else(|| ProcessorError::export("horizons is required"))?;
        let bin_size_seconds = self.bin_size_seconds.ok_or_else(|| ProcessorError::export("bin_size_seconds is required"))?;

        let now = chrono::Utc::now().to_rfc3339();
        let export_timestamp = self.export_timestamp.unwrap_or_else(|| now.clone());
        let norm_params_file = self.normalization_params_file
            .unwrap_or_else(|| format!("{}_normalization.json", day));

        Ok(ExportMetadata {
            day: day.clone(),
            n_sequences,
            window_size,
            n_features: TOTAL_FEATURES,
            schema_version: format!("{:.1}", SCHEMA_VERSION),
            contract_version: CONTRACT_VERSION.to_string(),
            label_strategy: "point_return".to_string(),
            label_encoding: "continuous_bps".to_string(),
            horizons,
            bin_size_seconds,
            market_open_et: self.market_open_et.unwrap_or_else(|| "09:30".to_string()),
            normalization: NormalizationMeta {
                strategy: "per_day_zscore".to_string(),
                applied: self.normalization_applied,
                params_file: norm_params_file,
            },
            provenance: ProvenanceMeta {
                source_file: self.provenance_source_file.unwrap_or_default(),
                processor_version: env!("CARGO_PKG_VERSION").to_string(),
                export_timestamp_utc: export_timestamp.clone(),
                config_hash: self.config_hash,
            },
            export_timestamp,
            first_bin_start_ns: self.first_bin_start_ns,
            last_bin_end_ns: self.last_bin_end_ns,
            n_bins_total: self.n_bins_total,
            n_bins_valid: self.n_bins_valid,
            n_bins_warmup_discarded: self.n_bins_warmup_discarded,
            n_bins_label_truncated: self.n_bins_label_truncated,
            n_total_records: self.n_total_records,
            n_trade_records: self.n_trade_records,
            n_trf_trades: self.n_trf_trades,
            n_lit_trades: self.n_lit_trades,
            data_source: "XNAS.BASIC".to_string(),
            schema: "cmbp-1".to_string(),
            symbol: self.symbol.unwrap_or_else(|| "NVDA".to_string()),
            equs_summary_available: self.equs_summary_available,
            consolidated_volume: self.consolidated_volume,
            trf_volume_fraction: self.trf_volume_fraction,
            feature_groups_enabled: self.feature_groups_enabled
                .unwrap_or(serde_json::json!({})),
            classification_config: self.classification_config
                .unwrap_or(serde_json::json!({})),
            signing_method: self.signing_method.unwrap_or_else(|| "midpoint".to_string()),
            exclusion_band: self.exclusion_band,
        })
    }
}

impl ExportMetadata {
    /// Create a builder for ExportMetadata.
    pub fn builder() -> ExportMetadataBuilder {
        ExportMetadataBuilder::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_metadata() -> ExportMetadata {
        ExportMetadata::builder()
            .day("2025-02-03")
            .n_sequences(100)
            .window_size(20)
            .horizons(vec![1, 5, 10])
            .bin_size_seconds(60)
            .build()
            .unwrap()
    }

    #[test]
    fn test_metadata_to_json_roundtrip() {
        let meta = minimal_metadata();
        let json = meta.to_json().unwrap();

        // Parse back and check key fields
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["day"], "2025-02-03");
        assert_eq!(parsed["n_sequences"], 100);
        assert_eq!(parsed["n_features"], TOTAL_FEATURES);
        assert_eq!(parsed["label_strategy"], "point_return");
        assert_eq!(parsed["label_encoding"], "continuous_bps");
    }

    #[test]
    fn test_metadata_builder_missing_required_fails() {
        // Missing day
        let result = ExportMetadata::builder()
            .n_sequences(100)
            .window_size(20)
            .horizons(vec![1])
            .bin_size_seconds(60)
            .build();
        assert!(result.is_err());

        // Missing n_sequences
        let result = ExportMetadata::builder()
            .day("2025-02-03")
            .window_size(20)
            .horizons(vec![1])
            .bin_size_seconds(60)
            .build();
        assert!(result.is_err());
    }

    #[test]
    fn test_metadata_label_strategy_always_point_return() {
        let meta = minimal_metadata();
        assert_eq!(meta.label_strategy, "point_return");
        assert_eq!(meta.label_encoding, "continuous_bps");
    }

    #[test]
    fn test_metadata_schema_version_matches_contract() {
        let meta = minimal_metadata();
        assert_eq!(meta.schema_version, format!("{:.1}", SCHEMA_VERSION));
        assert_eq!(meta.contract_version, CONTRACT_VERSION);
    }

    #[test]
    fn test_metadata_all_required_fields_present() {
        let meta = minimal_metadata();
        let json = meta.to_json().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Check every required field exists
        let required = [
            "day", "n_sequences", "window_size", "n_features",
            "schema_version", "contract_version", "label_strategy",
            "label_encoding", "horizons", "bin_size_seconds",
            "market_open_et", "normalization", "provenance",
            "export_timestamp", "first_bin_start_ns", "last_bin_end_ns",
            "n_bins_total", "n_bins_valid", "n_bins_warmup_discarded",
            "n_bins_label_truncated", "n_total_records", "n_trade_records",
            "n_trf_trades", "n_lit_trades", "data_source", "schema",
            "symbol", "equs_summary_available", "feature_groups_enabled",
            "classification_config", "signing_method", "exclusion_band",
        ];
        for field in &required {
            assert!(
                parsed.get(field).is_some(),
                "Missing required field: {}",
                field
            );
        }
    }

    #[test]
    fn test_metadata_write_to_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_meta.json");
        let meta = minimal_metadata();
        meta.write_to_file(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed["day"], "2025-02-03");
    }
}
