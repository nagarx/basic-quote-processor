//! Export metadata JSON for off-exchange feature exports.
//!
//! Contains ALL spec-required fields from 03_DATA_FLOW.md §2.5
//! and 06_INTEGRATION_POINTS.md §5.2.
//!
//! Source: docs/design/06_INTEGRATION_POINTS.md §5.2

use std::path::Path;

use serde::{Deserialize, Serialize};

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
    //
    // Bin accounting invariants (Round 8 doc clarification):
    //   `n_bins_total`          = post-warmup emitted bins (ALL — includes those
    //                              with valid labels AND those with NaN labels
    //                              beyond max-horizon tail).
    //   `n_bins_valid`          = bins whose labels are all finite across every
    //                              configured horizon (becomes sequence endpoints).
    //   `n_bins_label_truncated` = bins where at least one horizon yields NaN
    //                              (typically the last max_horizon bins of the day).
    //   `n_bins_warmup_discarded` = bins dropped during warmup (NOT counted in
    //                              `n_bins_total` — they are pre-warmup).
    //
    // Invariant: `n_bins_total == n_bins_valid + n_bins_label_truncated`.
    //            `n_bins_warmup_discarded` is ORTHOGONAL (counts pre-warmup bins
    //            that never contribute to n_bins_total).
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
    /// Phase 9.4 / D13: Experiment identifier from
    /// `DatasetExportConfig::experiment`. Makes metadata self-identifying
    /// without requiring a sibling `dataset_manifest.json` lookup. Omitted
    /// (skipped on serialize) for pre-Phase-9 compat — new exports always
    /// emit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experiment: Option<String>,

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

    // ── Forward-price trajectory export (Phase 9.1) ──────────────────
    /// Metadata for the `{day}_forward_prices.npy` file. Required by
    /// `hft-contracts.ForwardPriceContract.from_metadata()` for the
    /// T9 `LabelFactory` pathway (on-the-fly label recomputation).
    ///
    /// Emitted when present; absent in legacy v0 metadata produced before
    /// Phase 9 (handled by `#[serde(skip_serializing_if)]` and `default`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forward_prices: Option<ForwardPricesMeta>,
}

/// Metadata describing the `{day}_forward_prices.npy` file.
///
/// Contract: `contracts/pipeline_contract.toml [forward_prices.metadata]`
/// Consumer: `hft-contracts/label_factory.py::ForwardPriceContract.from_metadata()`
///
/// `basic-quote-processor` exports forward prices with layout
/// `forward_prices[t][k] = mid_price[t + k]` (column 0 = base price at t,
/// column h = price at t+h). There is **no** past smoothing window — hence
/// `smoothing_window_offset = 0` and `n_columns = max_horizon + 1`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForwardPricesMeta {
    /// Always `true` for new exports (file is always written alongside metadata).
    /// Consumers check this before attempting to load the `.npy` file.
    pub exported: bool,
    /// The `k` parameter (past-smoothing column offset). **Always 0** for
    /// basic-quote-processor — column 0 IS the base price at t.
    pub smoothing_window_offset: usize,
    /// The `H` parameter: maximum forward horizon in bins. Equals
    /// `LabelConfig::max_horizon()` at export time.
    pub max_horizon: usize,
    /// Total columns in the `.npy` file: `k + H + 1` (invariant enforced by
    /// `hft-contracts.ForwardPriceContract.__post_init__`).
    pub n_columns: usize,
    /// Unit of the prices. Always `"USD"` for basic-quote-processor
    /// (converted from raw i64 nanodollars at Phase 3 boundary).
    pub units: String,
    /// Human-readable layout description. Documentation-only — the Python
    /// consumer does NOT parse this field.
    pub column_layout: String,
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
    /// F5 → 9.5: Strategy string emitted in metadata.normalization.strategy.
    /// Default "none" (matches current production config).
    normalization_strategy: String,
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
    forward_prices: Option<ForwardPricesMeta>,
    experiment: Option<String>,
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
            normalization_strategy: "none".to_string(),
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
            forward_prices: None,
            experiment: None,
        }
    }

    pub fn day(mut self, day: &str) -> Self { self.day = Some(day.to_string()); self }
    pub fn n_sequences(mut self, n: usize) -> Self { self.n_sequences = Some(n); self }
    pub fn window_size(mut self, w: usize) -> Self { self.window_size = Some(w); self }
    pub fn horizons(mut self, h: Vec<usize>) -> Self { self.horizons = Some(h); self }
    pub fn bin_size_seconds(mut self, b: u32) -> Self { self.bin_size_seconds = Some(b); self }
    pub fn market_open_et(mut self, m: &str) -> Self { self.market_open_et = Some(m.to_string()); self }
    pub fn normalization_applied(mut self, a: bool) -> Self { self.normalization_applied = a; self }
    /// Phase 9.5: set the normalization strategy string emitted in metadata.
    ///
    /// Valid values match `DatasetExportConfig::normalization` TOML validation:
    /// `"none"` (default) or `"per_day_zscore"`. The builder does NOT validate
    /// the string — validation is upstream in config parsing.
    pub fn normalization_strategy(mut self, s: &str) -> Self {
        self.normalization_strategy = s.to_string();
        self
    }
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
    /// Phase 9.4 / D13: Attach experiment identifier.
    pub fn experiment(mut self, e: &str) -> Self { self.experiment = Some(e.to_string()); self }

    /// Phase 9.1: Attach forward-prices metadata block (unlocks T9 LabelFactory).
    ///
    /// `max_horizon` must match `LabelConfig::max_horizon()` at export time.
    /// `smoothing_window_offset` is hardcoded to 0 because basic-quote-processor
    /// exports `forward_prices[t][k] = mid_price[t + k]` with column 0 = base
    /// price at t (no past smoothing). `n_columns = k + H + 1 = 0 + H + 1 = H + 1`.
    pub fn forward_prices(mut self, max_horizon: usize) -> Self {
        self.forward_prices = Some(ForwardPricesMeta {
            exported: true,
            smoothing_window_offset: 0,
            max_horizon,
            n_columns: max_horizon + 1,
            units: "USD".to_string(),
            column_layout: "column_0_is_base_price_at_t; column_h_is_price_at_t_plus_h".to_string(),
        });
        self
    }

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
                // Phase 9.5: strategy is now configured, not hardcoded. Upstream
                // validation (DatasetExportConfig::validate) enforces it is one
                // of {"none", "per_day_zscore"} when loaded from the CLI config.
                strategy: self.normalization_strategy,
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
            forward_prices: self.forward_prices,
            experiment: self.experiment,
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

    // ── Phase 9.1: forward_prices metadata block tests ─────────────────

    fn metadata_with_fp(max_h: usize) -> ExportMetadata {
        ExportMetadata::builder()
            .day("2025-02-03")
            .n_sequences(100)
            .window_size(20)
            .horizons(vec![1, max_h])
            .bin_size_seconds(60)
            .forward_prices(max_h)
            .build()
            .unwrap()
    }

    #[test]
    fn test_metadata_includes_forward_prices_block() {
        // When builder method is called, metadata must include all 6
        // fields required by `hft-contracts.ForwardPriceContract.from_metadata()`.
        let meta = metadata_with_fp(60);
        let json = meta.to_json().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert!(
            parsed.get("forward_prices").is_some(),
            "Metadata must include forward_prices block when builder.forward_prices() is called"
        );
        let fp = &parsed["forward_prices"];
        assert_eq!(fp["exported"], true);
        assert_eq!(fp["smoothing_window_offset"], 0);
        assert_eq!(fp["max_horizon"], 60);
        assert_eq!(fp["n_columns"], 61);
        assert_eq!(fp["units"], "USD");
        assert!(fp["column_layout"].is_string());
    }

    #[test]
    fn test_forward_prices_n_columns_is_k_plus_h_plus_1() {
        // Load-bearing invariant enforced by
        // `hft-contracts.ForwardPriceContract.__post_init__` (label_factory.py:100-108).
        // n_columns must equal smoothing_window_offset + max_horizon + 1.
        for max_h in [1_usize, 5, 10, 30, 60, 100] {
            let meta = metadata_with_fp(max_h);
            let fp = meta.forward_prices.as_ref().expect("forward_prices set");
            assert_eq!(
                fp.n_columns,
                fp.smoothing_window_offset + fp.max_horizon + 1,
                "n_columns invariant violated for max_h={max_h}: \
                 got n_columns={}, expected {}",
                fp.n_columns,
                fp.smoothing_window_offset + fp.max_horizon + 1
            );
        }
    }

    #[test]
    fn test_forward_prices_units_is_usd() {
        // basic-quote-processor converts i64 nanodollars → f64 USD at the
        // Phase 3 boundary. The `units` field must always be "USD".
        let meta = metadata_with_fp(60);
        assert_eq!(meta.forward_prices.unwrap().units, "USD");
    }

    #[test]
    fn test_forward_prices_smoothing_offset_is_zero() {
        // basic-quote-processor does NOT apply past-smoothing to forward
        // prices. Column 0 IS the base price at t (not a smoothed average).
        // If a future variant ever smooths, it must update both the storage
        // layout AND this metadata field in lockstep.
        let meta = metadata_with_fp(60);
        assert_eq!(meta.forward_prices.unwrap().smoothing_window_offset, 0);
    }

    #[test]
    fn test_metadata_without_forward_prices_skips_field() {
        // Backward compat — if `forward_prices(...)` builder method is NOT
        // called, the field is `None` and omitted from JSON output via
        // `#[serde(skip_serializing_if = "Option::is_none")]`. This preserves
        // compatibility with pre-Phase-9 metadata files.
        let meta = minimal_metadata();
        assert!(
            meta.forward_prices.is_none(),
            "Unset forward_prices must be Option::None"
        );
        let json = meta.to_json().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(
            parsed.get("forward_prices").is_none(),
            "Unset forward_prices must be skipped in JSON output"
        );
    }

    // ── Phase 9.5: normalization strategy honesty tests ────────────────

    #[test]
    fn test_metadata_strategy_default_is_none() {
        // When no builder method is called, strategy defaults to "none" —
        // matches the current production configs (all 47 use normalization = "none").
        // Previously this was hardcoded to "per_day_zscore" regardless of config.
        let meta = minimal_metadata();
        assert_eq!(meta.normalization.strategy, "none");
    }

    #[test]
    fn test_metadata_strategy_reflects_config_none() {
        // Explicitly setting "none" via the builder produces "none" in JSON.
        let meta = ExportMetadata::builder()
            .day("2025-02-03")
            .n_sequences(100)
            .window_size(20)
            .horizons(vec![1])
            .bin_size_seconds(60)
            .normalization_strategy("none")
            .build()
            .unwrap();
        assert_eq!(meta.normalization.strategy, "none");
        assert!(!meta.normalization.applied);

        // Also verify JSON emission
        let json = meta.to_json().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["normalization"]["strategy"], "none");
    }

    #[test]
    fn test_metadata_experiment_field_when_set() {
        // Round 7 / D13: experiment field emitted in JSON when builder called.
        let meta = ExportMetadata::builder()
            .day("2025-02-03")
            .n_sequences(100)
            .window_size(20)
            .horizons(vec![1])
            .bin_size_seconds(60)
            .experiment("basic_nvda_60s_phase9")
            .build()
            .unwrap();
        assert_eq!(meta.experiment.as_deref(), Some("basic_nvda_60s_phase9"));
        let json = meta.to_json().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["experiment"], "basic_nvda_60s_phase9");
    }

    #[test]
    fn test_metadata_experiment_field_skipped_when_unset() {
        // Round 7 / D13: backward compat — without setter, field is skipped.
        let meta = minimal_metadata();
        assert!(meta.experiment.is_none());
        let json = meta.to_json().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(
            parsed.get("experiment").is_none(),
            "Unset experiment must be omitted from JSON (serde skip)"
        );
    }

    #[test]
    fn test_metadata_strategy_reflects_config_per_day_zscore() {
        // Setting "per_day_zscore" via builder produces that exact string.
        // (Used if/when Rust-side normalization is re-enabled; currently all
        // production configs use "none" per T15.)
        let meta = ExportMetadata::builder()
            .day("2025-02-03")
            .n_sequences(100)
            .window_size(20)
            .horizons(vec![1])
            .bin_size_seconds(60)
            .normalization_strategy("per_day_zscore")
            .normalization_applied(true)
            .build()
            .unwrap();
        assert_eq!(meta.normalization.strategy, "per_day_zscore");
        assert!(meta.normalization.applied);
    }
}
