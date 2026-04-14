//! Pipeline configuration for off-exchange processing.
//!
//! All config structs deserialize from TOML with `#[serde(default)]` for
//! optional sections. Validation is fail-fast per pipeline convention (fail-fast configuration validation).
//!
//! Source: docs/design/05_CONFIGURATION_SCHEMA.md

use serde::{Deserialize, Serialize};
use crate::error::{ProcessorError, Result};
use crate::features::indices;

/// Top-level processor configuration.
///
/// Deserialized from a TOML file. All sections except `[input]` have defaults.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessorConfig {
    pub input: InputConfig,
    #[serde(default)]
    pub sampling: SamplingConfig,
    #[serde(default)]
    pub classification: crate::trade_classifier::ClassificationConfig,
    #[serde(default)]
    pub features: FeatureConfig,
    #[serde(default)]
    pub vpin: VpinConfig,
    #[serde(default)]
    pub validation: ValidationConfig,
    /// Sequence building configuration (Phase 4).
    #[serde(default)]
    pub sequence: SequenceConfig,
    /// Label configuration (Phase 4).
    #[serde(default)]
    pub labeling: LabelConfig,
}

impl ProcessorConfig {
    /// Load and validate from a TOML file path.
    pub fn from_toml(path: &std::path::Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| ProcessorError::config(format!("Failed to read config: {}", e)))?;
        let config: Self = toml::from_str(&content)
            .map_err(|e| ProcessorError::config(format!("TOML parse error: {}", e)))?;
        config.validate()?;
        Ok(config)
    }

    /// Validate all sub-configs.
    pub fn validate(&self) -> Result<()> {
        self.sampling.validate()?;
        self.classification.validate()?;
        self.validation.validate()?;
        self.sequence.validate()?;
        self.labeling.validate()?;
        self.vpin.validate()?;
        Ok(())
    }

    /// Canonical TOML serialization — the input to `config_hash_hex` (Phase 9.4).
    ///
    /// Uses `toml::to_string` (compact form) rather than `to_string_pretty` because
    /// the compact format is less likely to drift across `toml` 0.8.x minor version
    /// bumps (pretty-printing has more formatting freedom the crate may refine over
    /// time). Field order is determined by struct declaration order — a documented
    /// invariant of `serde_derive` + `toml` — so two runs with the same
    /// `ProcessorConfig` always produce byte-identical output, even if the user's
    /// original TOML text differed in comments, whitespace, or field order.
    ///
    /// Callers that need a human-readable view of the config should use
    /// `toml::to_string_pretty(self)` directly; this helper is intentionally the
    /// single canonical input to the SHA-256 hash.
    pub fn to_canonical_toml(&self) -> Result<String> {
        toml::to_string(self)
            .map_err(|e| ProcessorError::config(format!("config serialize: {e}")))
    }

    /// SHA-256 hash of the canonical TOML serialization — 64-character lowercase hex.
    ///
    /// This is the **config provenance** field emitted in metadata.json at
    /// `provenance.config_hash`. It uniquely identifies the processing configuration
    /// that produced an export — two runs with structurally identical
    /// `ProcessorConfig` instances produce identical hashes, even if the source TOML
    /// files differ in comments, whitespace, or field order.
    ///
    /// # Scope
    ///
    /// Only `ProcessorConfig` contents are hashed — **not** the multi-day
    /// orchestration fields (`DateRangeConfig`, `DatasetExportConfig`). This is
    /// intentional: `config_hash` represents "processing identity" (what features,
    /// what sampling, what labels), not "run identity" (when, where). Two runs
    /// producing the same processed data to different `output_dir`s share a hash.
    ///
    /// # Canonicalization input
    ///
    /// The hash input is `toml::to_string(&self)` — the serialized form of the
    /// derived `ProcessorConfig`, NOT the user's original TOML text. Two users
    /// who write TOML files with different comments, whitespace, or field
    /// order but which deserialize into structurally identical
    /// `ProcessorConfig` instances produce byte-identical hashes.
    ///
    /// # Errors
    ///
    /// Returns an error if `toml::to_string` fails. For the current
    /// `ProcessorConfig` layout (primitives + String + Option + Vec + enum),
    /// this is effectively unreachable. Callers are still expected to have
    /// called `validate()` beforehand to reject NaN/Inf floats — if NaN
    /// reaches this helper, TOML emits `nan` as a literal and the hash is
    /// deterministic-but-meaningless.
    pub fn config_hash_hex(&self) -> Result<String> {
        use sha2::{Digest, Sha256};
        let canonical = self.to_canonical_toml()?;
        let mut hasher = Sha256::new();
        hasher.update(canonical.as_bytes());
        Ok(format!("{:x}", hasher.finalize()))
    }
}

/// Input data configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InputConfig {
    /// Directory containing .dbn.zst files.
    pub data_dir: String,
    /// Filename pattern with `{date}` placeholder.
    pub filename_pattern: String,
    /// Trading symbol (default: "NVDA").
    #[serde(default = "default_symbol")]
    pub symbol: String,
    /// Path to EQUS_SUMMARY .dbn.zst file (optional).
    /// When None, pipeline proceeds without consolidated volume context.
    /// AD2: spec says required but we make optional for library usability.
    #[serde(default)]
    pub equs_summary_path: Option<String>,
}

fn default_symbol() -> String {
    "NVDA".to_string()
}

/// Time-bin sampling configuration.
///
/// Source: docs/design/05_CONFIGURATION_SCHEMA.md [sampling]
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SamplingConfig {
    /// Sampling strategy. Currently only "time_based" is supported.
    #[serde(default = "default_strategy")]
    pub strategy: String,
    /// Bin size in seconds. Must be one of {5, 10, 15, 30, 60, 120, 300, 600}.
    #[serde(default = "default_bin_size")]
    pub bin_size_seconds: u32,
    /// Market open time in ET (HH:MM format).
    #[serde(default = "default_market_open")]
    pub market_open_et: String,
    /// Market close time in ET (HH:MM format).
    #[serde(default = "default_market_close")]
    pub market_close_et: String,
}

impl Default for SamplingConfig {
    fn default() -> Self {
        Self {
            strategy: default_strategy(),
            bin_size_seconds: default_bin_size(),
            market_open_et: default_market_open(),
            market_close_et: default_market_close(),
        }
    }
}

impl SamplingConfig {
    /// Validate sampling configuration.
    pub fn validate(&self) -> Result<()> {
        if self.strategy != "time_based" {
            return Err(ProcessorError::config(format!(
                "Unknown sampling strategy '{}'; only 'time_based' is supported",
                self.strategy
            )));
        }
        const VALID_BIN_SIZES: &[u32] = &[5, 10, 15, 30, 60, 120, 300, 600];
        if !VALID_BIN_SIZES.contains(&self.bin_size_seconds) {
            return Err(ProcessorError::config(format!(
                "bin_size_seconds ({}) must be one of {:?}",
                self.bin_size_seconds, VALID_BIN_SIZES
            )));
        }
        Ok(())
    }
}

fn default_strategy() -> String { "time_based".to_string() }
fn default_bin_size() -> u32 { 60 }
fn default_market_open() -> String { "09:30".to_string() }
fn default_market_close() -> String { "16:00".to_string() }

/// Feature group enable/disable configuration.
///
/// Activity (27-28), safety gates (29-30), and context (31-33) are ALWAYS enabled
/// per spec. Only the optional groups are toggleable here.
///
/// Source: docs/design/05_CONFIGURATION_SCHEMA.md [features]
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureConfig {
    /// Signed flow features (indices 0-3). Default: enabled.
    #[serde(default = "default_true")]
    pub signed_flow: bool,
    /// Venue metrics (indices 4-7). Default: enabled.
    #[serde(default = "default_true")]
    pub venue_metrics: bool,
    /// Retail metrics (indices 8-11). Default: enabled.
    #[serde(default = "default_true")]
    pub retail_metrics: bool,
    /// BBO dynamics (indices 12-17). Default: enabled.
    #[serde(default = "default_true")]
    pub bbo_dynamics: bool,
    /// VPIN (indices 18-19). Default: DISABLED (requires daily volume context).
    #[serde(default)]
    pub vpin: bool,
    /// Trade size features (indices 20-23). Default: enabled.
    #[serde(default = "default_true")]
    pub trade_size: bool,
    /// Cross-venue features (indices 24-26). Default: enabled.
    #[serde(default = "default_true")]
    pub cross_venue: bool,
    // NOTE: activity (27-28), safety_gates (29-30), context (31-33) always enabled.
    // Per spec: "Groups activity, safety_gates, and context are always emitted."
}

impl Default for FeatureConfig {
    fn default() -> Self {
        Self {
            signed_flow: true,
            venue_metrics: true,
            retail_metrics: true,
            bbo_dynamics: true,
            vpin: false,
            trade_size: true,
            cross_venue: true,
        }
    }
}

impl FeatureConfig {
    /// Count of ENABLED features. For metadata only — the feature vector is always 34 elements.
    ///
    /// Disabled groups produce zeros at their indices but still occupy space.
    pub fn enabled_feature_count(&self) -> usize {
        let mut count = indices::ALWAYS_ENABLED_COUNT; // activity(2) + safety(2) + context(3) = 7
        if self.signed_flow { count += indices::SIGNED_FLOW_RANGE.len(); }
        if self.venue_metrics { count += indices::VENUE_METRICS_RANGE.len(); }
        if self.retail_metrics { count += indices::RETAIL_METRICS_RANGE.len(); }
        if self.bbo_dynamics { count += indices::BBO_DYNAMICS_RANGE.len(); }
        if self.vpin { count += indices::VPIN_RANGE.len(); }
        if self.trade_size { count += indices::TRADE_SIZE_RANGE.len(); }
        if self.cross_venue { count += indices::CROSS_VENUE_RANGE.len(); }
        count
    }
}

fn default_true() -> bool { true }

/// VPIN computation configuration.
///
/// Source: docs/design/05_CONFIGURATION_SCHEMA.md [vpin]
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VpinConfig {
    /// Shares per volume bar. Default: 5000.
    ///
    /// NOTE: The spec defines `bucket_volume_fraction = 0.02` (fraction of daily
    /// average volume). The fraction-based approach is deferred to Phase 5 when
    /// EQUS_SUMMARY daily context is available. For Phase 3, a fixed absolute
    /// volume is used.
    #[serde(default = "default_bucket_volume")]
    pub bucket_volume: u64,
    /// Number of volume bars in VPIN rolling window. Default: 50.
    #[serde(default = "default_lookback")]
    pub lookback_buckets: usize,
    /// BVC sigma window in minutes. Default: 1.
    #[serde(default = "default_sigma_window")]
    pub sigma_window_minutes: u32,
    /// Fraction of daily volume for bucket sizing (e.g., 0.02 = 2%).
    /// When Some and daily volume available (from EQUS_SUMMARY),
    /// overrides `bucket_volume` with `(daily_volume * fraction) as u64`.
    #[serde(default)]
    pub bucket_volume_fraction: Option<f64>,
}

impl Default for VpinConfig {
    fn default() -> Self {
        Self {
            bucket_volume: default_bucket_volume(),
            lookback_buckets: default_lookback(),
            sigma_window_minutes: default_sigma_window(),
            bucket_volume_fraction: None,
        }
    }
}

impl VpinConfig {
    /// Validate VPIN configuration parameters.
    ///
    /// F6 (Phase 9.4): `bucket_volume_fraction: Option<f64>` is explicitly
    /// guarded against NaN/±Inf. Without this guard, `toml::to_string(&config)`
    /// emits `bucket_volume_fraction = nan` (or `inf`) as literal TOML —
    /// technically parseable but semantically meaningless. The resulting
    /// `config_hash` would be deterministic but would describe an invalid
    /// configuration. The guard enforces that only semantically valid
    /// fractions (finite, in [0.0, 1.0]) ever reach the hash.
    pub fn validate(&self) -> Result<()> {
        if let Some(frac) = self.bucket_volume_fraction {
            if !frac.is_finite() {
                return Err(ProcessorError::config(format!(
                    "vpin.bucket_volume_fraction ({}) must be a finite value (not NaN/Inf)",
                    frac
                )));
            }
            if !(0.0..=1.0).contains(&frac) {
                return Err(ProcessorError::config(format!(
                    "vpin.bucket_volume_fraction ({}) must be in [0.0, 1.0]",
                    frac
                )));
            }
        }
        Ok(())
    }
}

fn default_bucket_volume() -> u64 { 5000 }
fn default_lookback() -> usize { 50 }
fn default_sigma_window() -> u32 { 1 }

/// Validation and gating configuration.
///
/// Source: docs/design/05_CONFIGURATION_SCHEMA.md [validation]
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationConfig {
    /// Minimum TRF trades per bin for bin_valid gate. Default: 10.
    #[serde(default = "default_min_trades")]
    pub min_trades_per_bin: u64,
    /// Maximum BBO staleness in nanoseconds for bbo_valid gate. Default: 5s.
    #[serde(default = "default_staleness")]
    pub bbo_staleness_max_ns: u64,
    /// Number of initial bins to discard as warmup. Default: 3.
    #[serde(default = "default_warmup")]
    pub warmup_bins: u32,
    /// Trade size threshold for block detection. Default: 10,000 shares.
    #[serde(default = "default_block_threshold")]
    pub block_threshold: u32,
    /// TRF trades per 1-second window to trigger burst. Default: 20.
    #[serde(default = "default_burst_threshold")]
    pub burst_threshold: u32,
    /// Empty bin policy. Default: "forward_fill_state".
    /// Valid: "forward_fill_state", "zero_all", "nan_all".
    #[serde(default = "default_empty_bin_policy")]
    pub empty_bin_policy: String,
    /// Enable half-day auto-detection. Default: true.
    /// When enabled, consecutive empty bins trigger early session close.
    #[serde(default = "default_true")]
    pub auto_detect_close: bool,
    /// Consecutive empty bins to trigger close detection. Default: 10.
    /// At 60s bins, 10 = 10 minutes. Avoids LULD halt false positives.
    #[serde(default = "default_close_gap")]
    pub close_detection_gap_bins: u32,
    // NOTE: [publishers] config deferred — using PublisherClass::from_id() for now.
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            min_trades_per_bin: default_min_trades(),
            bbo_staleness_max_ns: default_staleness(),
            warmup_bins: default_warmup(),
            block_threshold: default_block_threshold(),
            burst_threshold: default_burst_threshold(),
            empty_bin_policy: default_empty_bin_policy(),
            auto_detect_close: true,
            close_detection_gap_bins: default_close_gap(),
        }
    }
}

impl ValidationConfig {
    /// Validate configuration parameters.
    pub fn validate(&self) -> Result<()> {
        if self.min_trades_per_bin == 0 {
            return Err(ProcessorError::config(
                "min_trades_per_bin must be > 0",
            ));
        }
        if self.bbo_staleness_max_ns == 0 {
            return Err(ProcessorError::config(
                "bbo_staleness_max_ns must be > 0",
            ));
        }
        if self.block_threshold == 0 {
            return Err(ProcessorError::config(
                "block_threshold must be > 0",
            ));
        }
        let valid_policies = ["forward_fill_state", "zero_all", "nan_all"];
        if !valid_policies.contains(&self.empty_bin_policy.as_str()) {
            return Err(ProcessorError::config(format!(
                "empty_bin_policy '{}' must be one of {:?}",
                self.empty_bin_policy, valid_policies
            )));
        }
        if self.close_detection_gap_bins == 0 {
            return Err(ProcessorError::config(
                "close_detection_gap_bins must be >= 1",
            ));
        }
        Ok(())
    }
}

// ── Phase 4 Config Types ──────────────────────────────────────────────

/// Sequence building configuration.
///
/// Source: docs/design/06_INTEGRATION_POINTS.md §1.3
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SequenceConfig {
    /// Number of bins per sequence (sliding window length). Default: 20.
    #[serde(default = "default_window_size")]
    pub window_size: usize,
    /// Stride between consecutive sequences. Default: 1.
    #[serde(default = "default_stride")]
    pub stride: usize,
}

impl Default for SequenceConfig {
    fn default() -> Self {
        Self {
            window_size: default_window_size(),
            stride: default_stride(),
        }
    }
}

impl SequenceConfig {
    /// Validate sequence configuration.
    pub fn validate(&self) -> Result<()> {
        if self.window_size == 0 {
            return Err(ProcessorError::config("window_size must be > 0"));
        }
        if self.stride == 0 {
            return Err(ProcessorError::config("stride must be > 0"));
        }
        if self.stride > self.window_size {
            return Err(ProcessorError::config(format!(
                "stride ({}) must be <= window_size ({})",
                self.stride, self.window_size
            )));
        }
        Ok(())
    }
}

fn default_window_size() -> usize { crate::contract::DEFAULT_WINDOW_SIZE }
fn default_stride() -> usize { crate::contract::DEFAULT_STRIDE }

/// Label strategy enum. Only point-return is supported for the off-exchange pipeline.
///
/// Source: docs/design/04_FEATURE_SPECIFICATION.md §6
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LabelStrategy {
    /// Point-to-point forward return: (mid[t+H] - mid[t]) / mid[t] * 10000 bps
    PointReturn,
}

/// Label configuration.
///
/// Source: docs/design/04_FEATURE_SPECIFICATION.md §6
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LabelConfig {
    /// Label strategy. Only "point_return" supported.
    #[serde(default = "default_label_strategy")]
    pub label_type: LabelStrategy,
    /// Horizons in bins for multi-horizon labels. Default: [1,2,3,5,10,20,30,60].
    /// Each element must be in [1, 200], sorted ascending, no duplicates.
    #[serde(default = "default_horizons")]
    pub horizons: Vec<usize>,
}

impl Default for LabelConfig {
    fn default() -> Self {
        Self {
            label_type: default_label_strategy(),
            horizons: default_horizons(),
        }
    }
}

impl LabelConfig {
    /// Validate label configuration.
    pub fn validate(&self) -> Result<()> {
        if self.horizons.is_empty() {
            return Err(ProcessorError::config("horizons must be non-empty"));
        }
        for (i, &h) in self.horizons.iter().enumerate() {
            if h == 0 || h > 200 {
                return Err(ProcessorError::config(format!(
                    "horizon[{}] = {} must be in [1, 200]", i, h
                )));
            }
            if i > 0 && h <= self.horizons[i - 1] {
                return Err(ProcessorError::config(format!(
                    "horizons must be sorted ascending with no duplicates: \
                     horizon[{}] = {} <= horizon[{}] = {}",
                    i, h, i - 1, self.horizons[i - 1]
                )));
            }
        }
        Ok(())
    }

    /// Maximum horizon value.
    pub fn max_horizon(&self) -> usize {
        *self.horizons.last().unwrap_or(&0)
    }
}

fn default_label_strategy() -> LabelStrategy { LabelStrategy::PointReturn }
fn default_horizons() -> Vec<usize> { crate::contract::DEFAULT_HORIZONS.to_vec() }

/// Export destination configuration.
///
/// NOT part of ProcessorConfig — passed directly to DayExporter.
/// Processing parameters (window_size, horizons) are in ProcessorConfig;
/// export parameters (output_dir) are external to processing logic.
#[derive(Debug, Clone)]
pub struct ExportConfig {
    /// Output directory for exported files.
    pub output_dir: std::path::PathBuf,
    /// Whether to apply z-score normalization to exported sequences.
    /// Default: false (raw export, stats saved separately per spec).
    pub apply_normalization: bool,
    /// Experiment name for metadata.
    pub experiment: String,
}

impl Default for ExportConfig {
    fn default() -> Self {
        Self {
            output_dir: std::path::PathBuf::from("output"),
            apply_normalization: false,
            experiment: "basic_nvda".to_string(),
        }
    }
}

// ── Phase 5 CLI-level config types ────────────────────────────────

/// Complete config for the `export_dataset` CLI binary.
///
/// Contains all `ProcessorConfig` fields (inlined) plus multi-day
/// orchestration (dates, splits, export destination).
///
/// Use `to_processor_config()` to extract a `ProcessorConfig` for `DayPipeline`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetConfig {
    pub input: InputConfig,
    #[serde(default)]
    pub sampling: SamplingConfig,
    #[serde(default)]
    pub classification: crate::trade_classifier::ClassificationConfig,
    #[serde(default)]
    pub features: FeatureConfig,
    #[serde(default)]
    pub vpin: VpinConfig,
    #[serde(default)]
    pub validation: ValidationConfig,
    #[serde(default)]
    pub sequence: SequenceConfig,
    #[serde(default)]
    pub labeling: LabelConfig,
    /// Date range for multi-day processing.
    pub dates: DateRangeConfig,
    /// Export destination and split configuration.
    pub export: DatasetExportConfig,
}

impl DatasetConfig {
    /// Extract a ProcessorConfig for the DayPipeline.
    pub fn to_processor_config(&self) -> ProcessorConfig {
        ProcessorConfig {
            input: self.input.clone(),
            sampling: self.sampling.clone(),
            classification: self.classification.clone(),
            features: self.features.clone(),
            vpin: self.vpin.clone(),
            validation: self.validation.clone(),
            sequence: self.sequence.clone(),
            labeling: self.labeling.clone(),
        }
    }

    /// Load and validate from a TOML file.
    pub fn from_toml(path: &std::path::Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| ProcessorError::config(format!("Failed to read config: {}", e)))?;
        let config: Self = toml::from_str(&content)
            .map_err(|e| ProcessorError::config(format!("TOML parse error: {}", e)))?;
        config.validate()?;
        Ok(config)
    }

    /// Validate all sub-configs.
    pub fn validate(&self) -> Result<()> {
        let pc = self.to_processor_config();
        pc.validate()?;
        self.dates.validate()?;
        self.export.validate(&self.dates)?;
        Ok(())
    }
}

/// Date range for multi-day processing.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DateRangeConfig {
    /// Start date (inclusive), YYYY-MM-DD.
    pub start_date: String,
    /// End date (inclusive), YYYY-MM-DD.
    pub end_date: String,
    /// Dates to exclude (holidays), YYYY-MM-DD.
    #[serde(default)]
    pub exclude_dates: Vec<String>,
}

impl DateRangeConfig {
    /// Validate date range.
    pub fn validate(&self) -> Result<()> {
        let start = crate::dates::parse_iso_date(&self.start_date)?;
        let end = crate::dates::parse_iso_date(&self.end_date)?;
        if start > end {
            return Err(ProcessorError::config(format!(
                "start_date ({}) must be <= end_date ({})",
                self.start_date, self.end_date
            )));
        }
        for (i, date_str) in self.exclude_dates.iter().enumerate() {
            crate::dates::parse_iso_date(date_str)
                .map_err(|e| ProcessorError::config(format!(
                    "exclude_dates[{}] '{}': {}", i, date_str, e
                )))?;
        }
        Ok(())
    }
}

/// Export destination and split configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetExportConfig {
    /// Output directory for exports.
    pub output_dir: String,
    /// Train/val/test split date boundaries.
    pub split_dates: SplitDatesConfig,
    /// Normalization strategy. "per_day_zscore" or "none".
    #[serde(default = "default_normalization_strategy")]
    pub normalization: String,
    /// Experiment name for metadata.
    #[serde(default = "default_experiment")]
    pub experiment: String,
    /// Continue processing on per-day errors. Default: true.
    #[serde(default = "default_true")]
    pub continue_on_error: bool,
}

impl DatasetExportConfig {
    /// Validate export config against date range.
    pub fn validate(&self, dates: &DateRangeConfig) -> Result<()> {
        let start = crate::dates::parse_iso_date(&dates.start_date)?;
        let train_end = crate::dates::parse_iso_date(&self.split_dates.train_end)?;
        let val_end = crate::dates::parse_iso_date(&self.split_dates.val_end)?;
        let end = crate::dates::parse_iso_date(&dates.end_date)?;

        if train_end < start {
            return Err(ProcessorError::config(format!(
                "train_end ({}) must be >= start_date ({})",
                self.split_dates.train_end, dates.start_date
            )));
        }
        if val_end <= train_end {
            return Err(ProcessorError::config(format!(
                "val_end ({}) must be > train_end ({})",
                self.split_dates.val_end, self.split_dates.train_end
            )));
        }
        if val_end > end {
            return Err(ProcessorError::config(format!(
                "val_end ({}) must be <= end_date ({})",
                self.split_dates.val_end, dates.end_date
            )));
        }
        let valid_norms = ["per_day_zscore", "none"];
        if !valid_norms.contains(&self.normalization.as_str()) {
            return Err(ProcessorError::config(format!(
                "normalization '{}' must be one of {:?}",
                self.normalization, valid_norms
            )));
        }
        Ok(())
    }
}

/// Train/val/test split date boundaries.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SplitDatesConfig {
    /// Last date of training set (inclusive), YYYY-MM-DD.
    pub train_end: String,
    /// Last date of validation set (inclusive), YYYY-MM-DD.
    pub val_end: String,
}

fn default_normalization_strategy() -> String { "none".to_string() }
fn default_experiment() -> String { "basic_nvda".to_string() }

fn default_min_trades() -> u64 { 10 }
fn default_staleness() -> u64 { 5_000_000_000 } // 5 seconds
fn default_warmup() -> u32 { 3 }
fn default_block_threshold() -> u32 { 10_000 }
fn default_burst_threshold() -> u32 { 20 }
fn default_empty_bin_policy() -> String { "forward_fill_state".to_string() }
fn default_close_gap() -> u32 { 10 }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_sampling_validates() {
        let config = SamplingConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_invalid_bin_size_rejected() {
        let config = SamplingConfig {
            bin_size_seconds: 7,
            ..Default::default()
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("bin_size_seconds"), "Error: {}", err);
    }

    #[test]
    fn test_zero_bin_size_rejected() {
        let config = SamplingConfig {
            bin_size_seconds: 0,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_large_bin_size_rejected() {
        let config = SamplingConfig {
            bin_size_seconds: 1000,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_unknown_strategy_rejected() {
        let config = SamplingConfig {
            strategy: "volume_based".to_string(),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_feature_count_defaults() {
        let config = FeatureConfig::default();
        // All enabled except VPIN: 4+4+4+6+0+4+3 + 7(always) = 32
        assert_eq!(config.enabled_feature_count(), 32);
    }

    #[test]
    fn test_feature_count_all_enabled() {
        let config = FeatureConfig {
            vpin: true,
            ..Default::default()
        };
        // 4+4+4+6+2+4+3 + 7 = 34
        assert_eq!(config.enabled_feature_count(), 34);
    }

    #[test]
    fn test_feature_count_minimal() {
        let config = FeatureConfig {
            signed_flow: false,
            venue_metrics: false,
            retail_metrics: false,
            bbo_dynamics: false,
            vpin: false,
            trade_size: false,
            cross_venue: false,
        };
        // Only always-enabled: 7
        assert_eq!(config.enabled_feature_count(), 7);
    }

    #[test]
    fn test_vpin_disabled_by_default() {
        let config = FeatureConfig::default();
        assert!(!config.vpin);
    }

    #[test]
    fn test_default_validation_validates() {
        let config = ValidationConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_invalid_empty_bin_policy() {
        let config = ValidationConfig {
            empty_bin_policy: "invalid".to_string(),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_toml_deserialization_feature_config() {
        let toml_str = r#"
            signed_flow = true
            venue_metrics = false
            vpin = true
        "#;
        let config: FeatureConfig = toml::from_str(toml_str).unwrap();
        assert!(config.signed_flow);
        assert!(!config.venue_metrics);
        assert!(config.vpin);
        assert!(config.retail_metrics); // default true
    }

    #[test]
    fn test_toml_deserialization_sampling_config() {
        let toml_str = r#"
            bin_size_seconds = 30
        "#;
        let config: SamplingConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.bin_size_seconds, 30);
        assert_eq!(config.strategy, "time_based"); // default
        assert!(config.validate().is_ok());
    }

    // ── Phase 4 config tests ──────────────────────────────────────────

    #[test]
    fn test_default_sequence_config_validates() {
        let config = SequenceConfig::default();
        assert!(config.validate().is_ok());
        assert_eq!(config.window_size, 20);
        assert_eq!(config.stride, 1);
    }

    #[test]
    fn test_invalid_window_size_zero_rejected() {
        let config = SequenceConfig { window_size: 0, stride: 1 };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_invalid_stride_greater_than_window_rejected() {
        let config = SequenceConfig { window_size: 10, stride: 15 };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("stride"), "Error: {}", err);
    }

    #[test]
    fn test_invalid_stride_zero_rejected() {
        let config = SequenceConfig { window_size: 10, stride: 0 };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_default_label_config_validates() {
        let config = LabelConfig::default();
        assert!(config.validate().is_ok());
        assert_eq!(config.label_type, LabelStrategy::PointReturn);
        assert_eq!(config.horizons, vec![1, 2, 3, 5, 10, 20, 30, 60]);
        assert_eq!(config.max_horizon(), 60);
    }

    #[test]
    fn test_empty_horizons_rejected() {
        let config = LabelConfig {
            label_type: LabelStrategy::PointReturn,
            horizons: vec![],
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_horizons_exceeding_200_rejected() {
        let config = LabelConfig {
            label_type: LabelStrategy::PointReturn,
            horizons: vec![1, 10, 201],
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("200"), "Error: {}", err);
    }

    #[test]
    fn test_unsorted_horizons_rejected() {
        let config = LabelConfig {
            label_type: LabelStrategy::PointReturn,
            horizons: vec![10, 5, 20],
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_duplicate_horizons_rejected() {
        let config = LabelConfig {
            label_type: LabelStrategy::PointReturn,
            horizons: vec![1, 5, 5, 10],
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_toml_deserialization_label_config() {
        let toml_str = r#"
            label_type = "point_return"
            horizons = [1, 5, 10]
        "#;
        let config: LabelConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.label_type, LabelStrategy::PointReturn);
        assert_eq!(config.horizons, vec![1, 5, 10]);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_toml_deserialization_sequence_config() {
        let toml_str = r#"
            window_size = 50
            stride = 5
        "#;
        let config: SequenceConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.window_size, 50);
        assert_eq!(config.stride, 5);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_default_values_match_spec() {
        let s = SamplingConfig::default();
        assert_eq!(s.bin_size_seconds, 60);
        assert_eq!(s.market_open_et, "09:30");
        assert_eq!(s.market_close_et, "16:00");

        let v = ValidationConfig::default();
        assert_eq!(v.min_trades_per_bin, 10);
        assert_eq!(v.bbo_staleness_max_ns, 5_000_000_000);
        assert_eq!(v.warmup_bins, 3);
        assert_eq!(v.block_threshold, 10_000);
        assert_eq!(v.burst_threshold, 20);
        assert_eq!(v.empty_bin_policy, "forward_fill_state");

        let vp = VpinConfig::default();
        assert_eq!(vp.bucket_volume, 5000);
        assert_eq!(vp.lookback_buckets, 50);
        assert_eq!(vp.sigma_window_minutes, 1);
        assert!(vp.bucket_volume_fraction.is_none());
    }

    // ── Phase 5 config tests ──────────────────────────────────────

    #[test]
    fn test_auto_detect_close_defaults_true() {
        let v = ValidationConfig::default();
        assert!(v.auto_detect_close);
        assert_eq!(v.close_detection_gap_bins, 10);
    }

    #[test]
    fn test_close_detection_gap_zero_rejected() {
        let v = ValidationConfig {
            close_detection_gap_bins: 0,
            ..Default::default()
        };
        assert!(v.validate().is_err());
    }

    #[test]
    fn test_equs_summary_path_optional() {
        let toml_str = r#"
            data_dir = "."
            filename_pattern = "*.dbn.zst"
        "#;
        let config: InputConfig = toml::from_str(toml_str).unwrap();
        assert!(config.equs_summary_path.is_none());
    }

    #[test]
    fn test_bucket_volume_fraction_parsing() {
        let toml_str = r#"
            bucket_volume_fraction = 0.02
        "#;
        let config: VpinConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.bucket_volume_fraction, Some(0.02));
    }

    #[test]
    fn test_dataset_config_deserializes() {
        let toml_str = r#"
            [input]
            data_dir = "../data"
            filename_pattern = "xnas-basic-{date}.cmbp-1.dbn.zst"

            [dates]
            start_date = "2025-02-03"
            end_date = "2026-01-06"

            [export]
            output_dir = "../output"
            experiment = "test"

            [export.split_dates]
            train_end = "2025-09-30"
            val_end = "2025-11-13"
        "#;
        let config: DatasetConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.input.data_dir, "../data");
        assert_eq!(config.dates.start_date, "2025-02-03");
        assert_eq!(config.export.split_dates.train_end, "2025-09-30");
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_dataset_config_split_dates_validation() {
        let toml_str = r#"
            [input]
            data_dir = "."
            filename_pattern = "*.dbn.zst"

            [dates]
            start_date = "2025-02-03"
            end_date = "2026-01-06"

            [export]
            output_dir = "."

            [export.split_dates]
            train_end = "2025-11-13"
            val_end = "2025-09-30"
        "#;
        let config: DatasetConfig = toml::from_str(toml_str).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("val_end"), "Error: {}", err);
    }

    #[test]
    fn test_dataset_config_to_processor_config() {
        let toml_str = r#"
            [input]
            data_dir = "../data"
            filename_pattern = "*.dbn.zst"

            [dates]
            start_date = "2025-02-03"
            end_date = "2026-01-06"

            [export]
            output_dir = "."

            [export.split_dates]
            train_end = "2025-09-30"
            val_end = "2025-11-13"

            [sampling]
            bin_size_seconds = 30
        "#;
        let ds: DatasetConfig = toml::from_str(toml_str).unwrap();
        let pc = ds.to_processor_config();
        assert_eq!(pc.sampling.bin_size_seconds, 30);
        assert_eq!(pc.input.data_dir, "../data");
    }

    #[test]
    fn test_dataset_config_invalid_normalization() {
        let toml_str = r#"
            [input]
            data_dir = "."
            filename_pattern = "*.dbn.zst"

            [dates]
            start_date = "2025-02-03"
            end_date = "2026-01-06"

            [export]
            output_dir = "."
            normalization = "invalid"

            [export.split_dates]
            train_end = "2025-09-30"
            val_end = "2025-11-13"
        "#;
        let config: DatasetConfig = toml::from_str(toml_str).unwrap();
        assert!(config.validate().is_err());
    }

    // ── Phase 9.3 Serialize derive tests ──────────────────────────────

    /// Sample processor config used by roundtrip tests.
    fn sample_processor_config() -> ProcessorConfig {
        ProcessorConfig {
            input: InputConfig {
                data_dir: "../data".to_string(),
                filename_pattern: "test-{date}.dbn.zst".to_string(),
                symbol: "NVDA".to_string(),
                equs_summary_path: None,
            },
            sampling: SamplingConfig::default(),
            classification: crate::trade_classifier::ClassificationConfig::default(),
            features: FeatureConfig::default(),
            vpin: VpinConfig::default(),
            validation: ValidationConfig::default(),
            sequence: SequenceConfig::default(),
            labeling: LabelConfig::default(),
        }
    }

    /// Sample dataset config used by roundtrip tests.
    fn sample_dataset_config() -> DatasetConfig {
        DatasetConfig {
            input: InputConfig {
                data_dir: "../data".to_string(),
                filename_pattern: "test-{date}.dbn.zst".to_string(),
                symbol: "NVDA".to_string(),
                equs_summary_path: None,
            },
            sampling: SamplingConfig::default(),
            classification: crate::trade_classifier::ClassificationConfig::default(),
            features: FeatureConfig::default(),
            vpin: VpinConfig::default(),
            validation: ValidationConfig::default(),
            sequence: SequenceConfig::default(),
            labeling: LabelConfig::default(),
            dates: DateRangeConfig {
                start_date: "2025-02-03".to_string(),
                end_date: "2026-01-06".to_string(),
                exclude_dates: vec![],
            },
            export: DatasetExportConfig {
                output_dir: "/tmp/output".to_string(),
                split_dates: SplitDatesConfig {
                    train_end: "2025-09-30".to_string(),
                    val_end: "2025-11-13".to_string(),
                },
                normalization: "none".to_string(),
                experiment: "test".to_string(),
                continue_on_error: true,
            },
        }
    }

    #[test]
    fn test_processor_config_serialize_roundtrip() {
        // Round-trip the canonical TOML. A second serialization of the
        // re-parsed config must byte-match the first — proves Serialize and
        // Deserialize are inverses for the current struct layout.
        let c1 = sample_processor_config();
        let s1 = c1.to_canonical_toml().expect("serialize 1");
        let c2: ProcessorConfig = toml::from_str(&s1).expect("deserialize");
        let s2 = c2.to_canonical_toml().expect("serialize 2");
        assert_eq!(s1, s2, "ProcessorConfig roundtrip must be byte-stable");
    }

    #[test]
    fn test_dataset_config_serialize_roundtrip() {
        // Same invariant for the multi-day CLI config (includes [dates] and
        // [export] sections that ProcessorConfig does not have).
        let c1 = sample_dataset_config();
        let s1 = toml::to_string(&c1).expect("serialize 1");
        let c2: DatasetConfig = toml::from_str(&s1).expect("deserialize");
        let s2 = toml::to_string(&c2).expect("serialize 2");
        assert_eq!(s1, s2, "DatasetConfig roundtrip must be byte-stable");
    }

    #[test]
    fn test_label_strategy_enum_serializes_as_snake_case() {
        // LabelStrategy has #[serde(rename_all = "snake_case")]. Verify
        // Serialize honors the same rename as Deserialize: `PointReturn`
        // variant serializes to "point_return" (exactly matches the
        // `label_strategy` field already emitted in metadata.json).
        let config = sample_processor_config();
        let s = config.to_canonical_toml().expect("serialize");
        assert!(
            s.contains("label_type = \"point_return\""),
            "Expected LabelStrategy::PointReturn → \"point_return\" snake_case, got:\n{s}"
        );
    }

    #[test]
    fn test_to_canonical_toml_uses_compact_form() {
        // F3 invariant: helper uses toml::to_string (not pretty). The
        // compact form lacks leading whitespace before section headers
        // (pretty form would insert blank lines between sections).
        let config = sample_processor_config();
        let s = config.to_canonical_toml().expect("serialize");
        // Compact form still emits [section] headers — we're verifying the
        // absence of extra prettification by checking the output doesn't
        // contain double blank lines.
        assert!(
            !s.contains("\n\n\n"),
            "Compact form must not contain triple-newlines, got:\n{s}"
        );
    }

    // ── Phase 9.4: config_hash_hex tests ───────────────────────────────

    #[test]
    fn test_config_hash_deterministic() {
        // Two structurally identical configs must produce identical SHA-256 hashes.
        // This is the load-bearing invariant for reproducibility.
        let c1 = sample_processor_config();
        let c2 = sample_processor_config();
        let h1 = c1.config_hash_hex().expect("hash 1");
        let h2 = c2.config_hash_hex().expect("hash 2");
        assert_eq!(h1, h2, "Structurally identical configs must hash identically");
    }

    #[test]
    fn test_config_hash_changes_on_param_change() {
        // Changing a single processing parameter must change the hash —
        // otherwise drift-detection is useless.
        let c1 = sample_processor_config();
        let mut c2 = sample_processor_config();
        c2.sampling.bin_size_seconds = 30; // was 60 by default
        let h1 = c1.config_hash_hex().expect("hash 1");
        let h2 = c2.config_hash_hex().expect("hash 2");
        assert_ne!(h1, h2, "bin_size_seconds change must change hash");
    }

    #[test]
    fn test_config_hash_is_64_char_hex() {
        // SHA-256 → 32 bytes → 64 lowercase hex chars. Load-bearing shape
        // contract: consumers (EXPERIMENT_INDEX.md, trainer) will assume this.
        let c = sample_processor_config();
        let h = c.config_hash_hex().expect("hash");
        assert_eq!(h.len(), 64, "SHA-256 hex must be 64 chars, got {}", h.len());
        assert!(
            h.chars().all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase()),
            "Hash must be lowercase hex, got: {h}"
        );
    }

    #[test]
    fn test_config_hash_reflects_feature_toggle() {
        // Toggling a feature group is a config-identity change → hash must differ.
        let c1 = sample_processor_config();
        let mut c2 = sample_processor_config();
        c2.features.vpin = !c1.features.vpin;
        assert_ne!(
            c1.config_hash_hex().unwrap(),
            c2.config_hash_hex().unwrap(),
            "Feature toggle must change hash"
        );
    }

    #[test]
    fn test_config_hash_cross_struct_field_independence() {
        // Round 7 (Agent 4 recommendation): each sub-config field contributes
        // distinctly to the hash. Changing `SamplingConfig.bin_size_seconds`
        // MUST produce a different hash from changing `SequenceConfig.window_size`
        // by the same value. Rules out the bug where two fields in different
        // sub-structs accidentally canonicalize to the same TOML bytes.
        let baseline = sample_processor_config();
        let mut c_sampling = sample_processor_config();
        c_sampling.sampling.bin_size_seconds = 30;
        let mut c_sequence = sample_processor_config();
        c_sequence.sequence.window_size = 30;

        let h0 = baseline.config_hash_hex().unwrap();
        let h_samp = c_sampling.config_hash_hex().unwrap();
        let h_seq = c_sequence.config_hash_hex().unwrap();

        assert_ne!(h0, h_samp, "bin_size_seconds change must change hash");
        assert_ne!(h0, h_seq, "window_size change must change hash");
        assert_ne!(
            h_samp, h_seq,
            "Changes in sibling sub-structs must produce DISTINCT hashes — \
             rules out cross-struct canonicalization collisions"
        );
    }

    #[test]
    fn test_to_canonical_toml_equals_compact_form_byte_for_byte() {
        // Round 7 (Agent 4 recommendation) — stronger than the previous
        // "no triple-newlines" check. Asserts the helper delegates to
        // `toml::to_string` exactly (compact form), so a future accidental
        // switch back to `to_string_pretty` would fail this test loudly
        // even before `config_hash` consumers notice drift.
        let config = sample_processor_config();
        let via_helper = config.to_canonical_toml().expect("helper");
        let via_direct = toml::to_string(&config).expect("direct to_string");
        assert_eq!(
            via_helper, via_direct,
            "to_canonical_toml must be byte-identical to toml::to_string output"
        );
    }

    // ── F6 (Phase 9.4): VpinConfig NaN/Inf guard tests ─────────────────

    #[test]
    fn test_vpin_bucket_volume_fraction_nan_rejected() {
        // Without this guard, `toml::to_string(&config)` panics when
        // computing `config_hash_hex`. Validation must reject NaN at the
        // system boundary.
        let config = VpinConfig {
            bucket_volume_fraction: Some(f64::NAN),
            ..Default::default()
        };
        let err = config.validate().unwrap_err();
        assert!(
            err.to_string().contains("bucket_volume_fraction"),
            "Error should mention field: {err}"
        );
    }

    #[test]
    fn test_vpin_bucket_volume_fraction_inf_rejected() {
        let config = VpinConfig {
            bucket_volume_fraction: Some(f64::INFINITY),
            ..Default::default()
        };
        assert!(config.validate().is_err(), "Inf fraction must be rejected");
    }

    #[test]
    fn test_vpin_bucket_volume_fraction_neg_inf_rejected() {
        let config = VpinConfig {
            bucket_volume_fraction: Some(f64::NEG_INFINITY),
            ..Default::default()
        };
        assert!(config.validate().is_err(), "-Inf fraction must be rejected");
    }

    #[test]
    fn test_vpin_bucket_volume_fraction_out_of_range_rejected() {
        let config = VpinConfig {
            bucket_volume_fraction: Some(1.5),
            ..Default::default()
        };
        let err = config.validate().unwrap_err();
        assert!(
            err.to_string().contains("[0.0, 1.0]"),
            "Error should mention range: {err}"
        );
    }

    #[test]
    fn test_vpin_bucket_volume_fraction_valid_accepted() {
        // Legitimate fraction (2%) must validate cleanly.
        let config = VpinConfig {
            bucket_volume_fraction: Some(0.02),
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_vpin_none_fraction_accepted() {
        // None is the default; must not trip the NaN guard.
        let config = VpinConfig::default();
        assert!(config.bucket_volume_fraction.is_none());
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_vpin_validate_propagates_through_processor_config() {
        // NaN in a nested config must be caught by ProcessorConfig::validate,
        // not just direct VpinConfig::validate.
        let mut pc = sample_processor_config();
        pc.vpin.bucket_volume_fraction = Some(f64::NAN);
        assert!(
            pc.validate().is_err(),
            "Nested NaN must propagate through ProcessorConfig::validate"
        );
    }
}
