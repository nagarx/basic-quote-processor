//! Day-level pipeline orchestrator for off-exchange feature processing.
//!
//! Composes Phases 1-3 (reader, BBO, trade classifier, sampling, accumulation,
//! feature extraction) with Phase 4 (sequence building, label computation,
//! normalization, export assembly) into a single `DayPipeline` with a split
//! lifecycle: `init_day()` → `stream_file()` → `finalize()`.
//!
//! Source: docs/design/03_DATA_FLOW.md §2

use std::path::Path;
use std::sync::Arc;

use crate::accumulator::{BinAccumulator, DaySummary};
use crate::bbo_state::BboState;
use crate::config::ProcessorConfig;
use crate::contract::TOTAL_FEATURES;
use crate::error::{ProcessorError, Result};
use crate::export::metadata::ExportMetadata;
use crate::export::normalization::NormalizationComputer;
use crate::export::DayExport;
use crate::features::indices::{SPREAD_BPS, QUOTE_IMBALANCE};
use crate::features::FeatureExtractor;
use crate::labeling::{ForwardPriceComputer, LabelComputer};
use crate::reader::DbnReader;
use crate::sampling::{BinBoundary, TimeBinSampler};
use crate::sequence_builder::FeatureVec;
use crate::trade_classifier::TradeClassifier;

/// Day-level pipeline orchestrator.
///
/// Owns all Phase 1-3 components and Phase 4 state. Processes one .dbn.zst
/// file per day through the canonical INIT → STREAM → FINALIZE lifecycle.
///
/// # Usage
///
/// ```ignore
/// let mut pipeline = DayPipeline::new(&config)?;
/// let export = pipeline.process_day(file, 2025, 2, 3)?;
/// // Or split lifecycle:
/// pipeline.init_day(2025, 2, 3);
/// pipeline.stream_file(file)?;
/// let export = pipeline.finalize()?;
/// ```
pub struct DayPipeline {
    config: ProcessorConfig,
    // Phase 1-3 components
    bbo: BboState,
    classifier: TradeClassifier,
    sampler: TimeBinSampler,
    accumulator: BinAccumulator,
    extractor: FeatureExtractor,
    // Phase 4 components
    normalizer: NormalizationComputer,
    // Per-day state
    feature_bins: Vec<FeatureVec>,
    mid_prices: Vec<f64>,
    feature_buffer: Vec<f64>,
    // Day metadata
    day_str: String,
    // Phase 5: daily context + half-day detection
    daily_context: Option<crate::context::DailyContext>,
    bucket_volume_override: Option<u64>,
    consecutive_empty_bins: u32,
    detected_close_ns: Option<u64>,
    // Phase 9.4: provenance
    /// SHA-256 of canonical TOML of `ProcessorConfig`. Set once at startup via
    /// `set_config_hash`; preserved across `reset()` (same config across all days).
    config_hash: Option<String>,
    /// Basename of the input .dbn.zst for the current day (F1). Set per-day
    /// via `set_source_file`; cleared by `reset()`.
    source_file: Option<String>,
    /// Phase 9.5: normalization strategy string to emit in metadata. Set
    /// once via `set_normalization_strategy`; preserved across `reset()`.
    /// `None` → builder default ("none") is used.
    normalization_strategy: Option<String>,
    /// Phase 9.4: whether Rust-side normalization was actually applied to the
    /// NPY. Set once via `set_normalization_applied`; preserved across
    /// `reset()`. Paired with `normalization_strategy` — both come from the
    /// same CLI config (`apply_norm = config.export.normalization != "none"`).
    normalization_applied: Option<bool>,
    /// Phase 9.4 / D13: experiment identifier (from `DatasetExportConfig.experiment`).
    /// Set once at startup; preserved across `reset()`.
    experiment: Option<String>,
}

impl DayPipeline {
    /// Create a new pipeline from configuration.
    pub fn new(config: &ProcessorConfig) -> Result<Self> {
        let classifier = TradeClassifier::new(config.classification.clone())
            .map_err(|e| ProcessorError::config(format!("TradeClassifier: {e}")))?;

        Ok(Self {
            config: config.clone(),
            bbo: BboState::new(),
            classifier,
            sampler: TimeBinSampler::new(config.sampling.bin_size_seconds),
            accumulator: BinAccumulator::new(
                &config.validation,
                &config.vpin,
                &config.features,
            ),
            extractor: FeatureExtractor::new(
                &config.features,
                &config.validation,
                config.sampling.bin_size_seconds,
            ),
            normalizer: NormalizationComputer::new(TOTAL_FEATURES, &config.features),
            feature_bins: Vec::new(),
            mid_prices: Vec::new(),
            feature_buffer: Vec::with_capacity(TOTAL_FEATURES),
            day_str: String::new(),
            daily_context: None,
            bucket_volume_override: None,
            consecutive_empty_bins: 0,
            detected_close_ns: None,
            config_hash: None,
            source_file: None,
            normalization_strategy: None,
            normalization_applied: None,
            experiment: None,
        })
    }

    /// Set the canonical config hash for metadata provenance (Phase 9.4).
    ///
    /// Call once immediately after `DayPipeline::new()` with the result of
    /// `ProcessorConfig::config_hash_hex()`. The hash is preserved across
    /// `reset()` — two consecutive days processed by the same pipeline
    /// instance emit identical `provenance.config_hash` in their metadata.
    pub fn set_config_hash(&mut self, hash: String) {
        self.config_hash = Some(hash);
    }

    /// Set the input-file basename for metadata provenance (F1).
    ///
    /// Call **before** `stream_file()` each day. Cleared by `reset()` so
    /// per-day state does not leak across days. Pass the basename (not the
    /// full path) — absolute paths embed user-specific filesystem layout and
    /// make metadata non-portable across machines.
    pub fn set_source_file(&mut self, basename: String) {
        self.source_file = Some(basename);
    }

    /// Set the normalization strategy string emitted in metadata (Phase 9.5).
    ///
    /// Valid values: `"none"` (default) or `"per_day_zscore"`. Preserved
    /// across `reset()` since the strategy is constant for a whole run.
    /// If not called, metadata emits `"none"` (the current production default).
    pub fn set_normalization_strategy(&mut self, strategy: String) {
        self.normalization_strategy = Some(strategy);
    }

    /// Set the `normalization.applied` boolean emitted in metadata (Phase 9.4).
    ///
    /// `true` if Rust-side normalization was actually applied to the sequences
    /// before writing NPY; `false` if raw f64 values were exported. Pairs
    /// with `set_normalization_strategy` — both are constant per run and
    /// preserved across `reset()`. Under T15 "Raw Rust, Variable Python",
    /// production configs set this to `false`.
    pub fn set_normalization_applied(&mut self, applied: bool) {
        self.normalization_applied = Some(applied);
    }

    /// Set the experiment identifier emitted in metadata (Phase 9.4 / D13).
    ///
    /// Typically comes from `DatasetExportConfig::experiment`. Makes each
    /// metadata.json self-identifying (no directory-path dependence). Set
    /// once at startup; preserved across `reset()`.
    pub fn set_experiment(&mut self, experiment: String) {
        self.experiment = Some(experiment);
    }

    /// Phase 1: Initialize for a new day.
    ///
    /// Sets up the sampler and extractor with the day's market hours
    /// (DST-aware via `utc_offset_for_date`).
    pub fn init_day(&mut self, year: i32, month: u32, day: u32) {
        self.day_str = format!("{:04}-{:02}-{:02}", year, month, day);
        self.sampler.init_day(year, month, day);
        self.extractor.init_day(
            self.sampler.utc_offset_hours(),
            self.sampler.market_open_ns(),
            self.sampler.market_close_ns(),
        );
        self.accumulator.set_bin_size_ns(self.sampler.bin_size_ns());
    }

    /// Initialize for a new day with EQUS_SUMMARY daily context.
    ///
    /// Computes VPIN bucket volume override from daily context if configured,
    /// then delegates to `init_day()`.
    pub fn init_day_with_context(
        &mut self,
        year: i32,
        month: u32,
        day: u32,
        context: Option<crate::context::DailyContext>,
    ) {
        // Compute VPIN override from daily context BEFORE init_day.
        //
        // H10 fix (Round 8 validation): previously, `bucket_volume_override`
        // was set here but the accumulator was NOT rebuilt until the next
        // `reset()` — meaning day N's VPIN computation used day (N-1)'s
        // consolidated volume (or the static default on day 0). Under
        // `vpin = false` (current production configs) this was dead code,
        // but would silently produce wrong VPIN values the moment anyone
        // enabled the feature group. The fix: rebuild the accumulator with
        // the new bucket volume NOW, so the current day's VPIN uses the
        // current day's volume.
        if let (Some(ref ctx), Some(frac)) = (&context, self.config.vpin.bucket_volume_fraction) {
            if let Some(vol) = ctx.consolidated_volume {
                let dynamic_bucket = (vol as f64 * frac) as u64;
                if dynamic_bucket > 0 {
                    self.bucket_volume_override = Some(dynamic_bucket);
                    // Rebuild accumulator with the dynamic bucket volume so
                    // the CURRENT day (not the next one) uses it.
                    let mut vpin_config = self.config.vpin.clone();
                    vpin_config.bucket_volume = dynamic_bucket;
                    self.accumulator = BinAccumulator::new(
                        &self.config.validation,
                        &vpin_config,
                        &self.config.features,
                    );
                }
            }
        }
        self.daily_context = context;
        self.init_day(year, month, day);
    }

    /// Phase 2: Stream all records from a .dbn.zst file.
    ///
    /// Contains the canonical processing loop:
    /// 1. Check bin boundary FIRST (extract previous bin)
    /// 2. BBO update ALWAYS (enables pre-market warm-start)
    /// 3. Trade accumulation gated by `is_in_session()`
    ///
    /// At each post-warmup bin emission, records the feature vector and
    /// mid-price for Phase 4 label computation and sequence building.
    pub fn stream_file(&mut self, file_path: &Path) -> Result<()> {
        // FIX #8: Guard against stale state
        assert!(
            self.feature_bins.is_empty(),
            "stream_file() called without reset() — stale state detected. \
             Call reset() between days."
        );

        let reader = DbnReader::new(file_path)?;
        let (_metadata, records) = reader.open()?;
        let warmup_bins = self.config.validation.warmup_bins as u64;

        for record in records {
            // 1. Check bin boundary FIRST
            if let Some(boundary) = self.sampler.check_boundary(record.ts_recv) {
                self.accumulator.prepare_for_extraction(boundary.bin_end_ts);
                self.extractor.extract(
                    &self.accumulator, &self.bbo,
                    &boundary, &mut self.feature_buffer,
                );

                // Forward-fill update (even during warmup per R2)
                if self.accumulator.trf_trades() > 0 {
                    self.accumulator.update_forward_fill(
                        &self.feature_buffer, self.config.features.vpin,
                    );
                }
                if self.accumulator.bbo_update_count() > 0 {
                    self.accumulator.update_forward_fill_bbo(
                        self.feature_buffer[SPREAD_BPS],
                        self.feature_buffer[QUOTE_IMBALANCE],
                    );
                }

                // Track activity for half-day detection BEFORE warmup gate
                let bin_had_activity = self.accumulator.total_trades() > 0
                    || self.accumulator.bbo_update_count() > 0;

                // Warmup gate
                if self.accumulator.bin_index() >= warmup_bins {
                    self.emit_bin(boundary.bin_end_ts);
                } else {
                    self.accumulator.record_warmup_discard();
                }

                self.accumulator.reset_bin();

                // Half-day auto-detection (Phase 5)
                if bin_had_activity {
                    self.consecutive_empty_bins = 0;
                } else {
                    self.consecutive_empty_bins += 1;
                }
                if self.config.validation.auto_detect_close
                    && self.consecutive_empty_bins
                        >= self.config.validation.close_detection_gap_bins
                {
                    let detected_close = boundary.bin_end_ts.saturating_sub(
                        self.consecutive_empty_bins as u64 * self.sampler.bin_size_ns(),
                    );
                    self.extractor.set_session_end(detected_close);
                    self.detected_close_ns = Some(detected_close);
                    break; // Exit processing loop — day is complete
                }

                // Emit gap bins individually (FIX #1)
                // Each gap bin is empty by definition → counts toward consecutive
                for gap_i in 0..boundary.gap_bins {
                    let gap_end = boundary.bin_end_ts
                        + (gap_i + 1) * self.sampler.bin_size_ns();
                    let gap_boundary = BinBoundary {
                        bin_end_ts: gap_end,
                        bin_midpoint_ts: gap_end - self.sampler.bin_size_ns() / 2,
                        bin_index: boundary.bin_index + 1 + gap_i,
                        gap_bins: 0,
                    };
                    self.extractor.extract(
                        &self.accumulator, &self.bbo,
                        &gap_boundary, &mut self.feature_buffer,
                    );
                    if self.accumulator.bin_index() >= warmup_bins {
                        self.emit_gap_bin(gap_end);
                    }
                    self.accumulator.reset_bin();
                    self.consecutive_empty_bins += 1;

                    // Check half-day during gap emission too
                    if self.config.validation.auto_detect_close
                        && self.consecutive_empty_bins
                            >= self.config.validation.close_detection_gap_bins
                    {
                        let detected_close = boundary.bin_end_ts; // Gap started here
                        self.extractor.set_session_end(detected_close);
                        self.detected_close_ns = Some(detected_close);
                        break; // Break out of gap loop
                    }
                }
                // If we detected close during gap emission, break the main loop
                if self.detected_close_ns.is_some() {
                    break;
                }
            }

            // 2. BBO update (always — enables pre-market warm-start)
            if self.bbo.update_from_record(&record) {
                if self.sampler.is_in_session(record.ts_recv) {
                    self.accumulator.accumulate_bbo_update(&self.bbo, record.ts_recv);
                }
            }

            // 3. Trade classification + accumulation (session-gated)
            if record.is_trade() && self.sampler.is_in_session(record.ts_recv) {
                let classified = self.classifier.classify(&record, &self.bbo);
                self.accumulator.accumulate(&classified);
            }
        }

        // Flush last partial bin (FIX #13; Phase 9.2 / H2 extended to BBO-only)
        //
        // H2 fix rationale (F4):
        //   Original condition `has_trades()` dropped the final partial bin when
        //   the last ~30s of a session had only BBO updates and zero trades — a
        //   common pattern near the close. The BBO-only bin's features are
        //   valid (forward-filled flow, live BBO dynamics, context gates). We
        //   emit it here.
        //
        //   Safety: a BBO-only final bin has ALL labels = NaN (no `t+h` bins
        //   exist beyond it — see `src/labeling/point_return.rs:93`). The
        //   sequence builder below (in `finalize()`) filters such bins via
        //   the `valid_mask[ending_idx]` check — so the new bin is added to
        //   `feature_bins` / `mid_prices` / `n_bins_total` but does NOT
        //   inflate `n_sequences`.
        //   See tests at `src/labeling/point_return.rs::test_label_single_bin`
        //   and `::test_valid_mask_excludes_nan` for the invariant.
        //
        //   On half-days (auto-detect-close), `break` exits the record loop
        //   AFTER `reset_bin()` — so at this point the accumulator is empty
        //   (`false || false == false`) and this block is safely skipped.
        self.accumulator.prepare_for_extraction(self.sampler.market_close_ns());
        if self.accumulator.has_trades() || self.accumulator.bbo_update_count() > 0 {
            let final_boundary = BinBoundary {
                bin_end_ts: self.sampler.market_close_ns(),
                bin_midpoint_ts: self.sampler.market_close_ns()
                    - self.sampler.bin_size_ns() / 2,
                bin_index: self.accumulator.bin_index(),
                gap_bins: 0,
            };
            self.extractor.extract(
                &self.accumulator, &self.bbo,
                &final_boundary, &mut self.feature_buffer,
            );
            if self.accumulator.trf_trades() > 0 {
                self.accumulator.update_forward_fill(
                    &self.feature_buffer, self.config.features.vpin,
                );
            }
            // FIX CRITICAL-2: Must also update BBO forward-fill on final flush,
            // matching the main loop pattern. Without this, the final bin's
            // spread_bps and quote_imbalance forward-fill values may be stale.
            if self.accumulator.bbo_update_count() > 0 {
                self.accumulator.update_forward_fill_bbo(
                    self.feature_buffer[SPREAD_BPS],
                    self.feature_buffer[QUOTE_IMBALANCE],
                );
            }
            if self.accumulator.bin_index() >= warmup_bins {
                self.emit_bin(self.sampler.market_close_ns());
            }
        }

        Ok(())
    }

    /// Emit a normal bin (post-warmup): store features, mid-price, update normalization.
    fn emit_bin(&mut self, bin_end_ts: u64) {
        let fv = Arc::new(self.feature_buffer.clone());
        self.feature_bins.push(fv);
        self.mid_prices.push(self.bbo.mid_price);
        self.normalizer.update(&self.feature_buffer);
        let is_empty = self.accumulator.trf_trades() == 0;
        self.accumulator.record_bin_emitted_with_ts(is_empty, bin_end_ts);
    }

    /// Emit a gap bin: same as normal but uses gap-bin diagnostic counter.
    /// FIX CRITICAL-1: Uses `record_gap_bin_with_ts` to track timestamps.
    fn emit_gap_bin(&mut self, bin_end_ts: u64) {
        let fv = Arc::new(self.feature_buffer.clone());
        self.feature_bins.push(fv);
        self.mid_prices.push(self.bbo.mid_price);
        self.normalizer.update(&self.feature_buffer);
        self.accumulator.record_gap_bin_with_ts(bin_end_ts);
    }

    /// Phase 3: Finalize — compute labels, build sequences, assemble export.
    ///
    /// Can be tested independently with synthetic `feature_bins` and `mid_prices`
    /// by calling `set_test_data()` before `finalize()`.
    pub fn finalize(&mut self) -> Result<DayExport> {
        let horizons = &self.config.labeling.horizons;
        let max_horizon = self.config.labeling.max_horizon();
        let window_size = self.config.sequence.window_size;
        let stride = self.config.sequence.stride;
        let n_bins = self.feature_bins.len();

        // 1. Compute labels from mid-prices
        let label_computer = LabelComputer::new(horizons)?;
        let label_result = label_computer.compute_labels(&self.mid_prices);

        // 2. Compute forward prices
        let fwd_computer = ForwardPriceComputer::new(max_horizon);
        let all_fwd = fwd_computer.compute(&self.mid_prices);

        // 3. FIX #9: Determine valid sequence ending indices using valid_mask
        //    A sequence ending at bin `e` requires:
        //    - e >= window_size - 1 (enough preceding bins for the window)
        //    - label_result.valid_mask[e] == true (all horizons finite)
        let mut sequences: Vec<Vec<FeatureVec>> = Vec::new();
        let mut aligned_labels: Vec<Vec<f64>> = Vec::new();
        let mut aligned_fwd: Vec<Vec<f64>> = Vec::new();

        if n_bins >= window_size {
            for ending_idx in (window_size - 1)..n_bins {
                if !label_result.valid_mask[ending_idx] {
                    continue;
                }
                // Check stride alignment
                let seq_start = ending_idx + 1 - window_size;
                if seq_start % stride != 0 {
                    continue;
                }

                let seq: Vec<FeatureVec> = self.feature_bins[seq_start..=ending_idx]
                    .iter()
                    .map(Arc::clone)
                    .collect();
                sequences.push(seq);
                aligned_labels.push(label_result.labels[ending_idx].clone());
                aligned_fwd.push(all_fwd[ending_idx].clone());
            }
        }

        // 4. Build metadata
        let summary = self.accumulator.day_summary();
        let norm_json = self.normalizer.to_json(&self.day_str)?;

        // F2 (Phase 9.4) — serialize the active config structs into metadata so
        // the export carries both the `config_hash` (identity) and the actual
        // values (forensic inspection). Requires 9.3 Serialize derives on
        // `FeatureConfig` and `ClassificationConfig`.
        let features_json = serde_json::to_value(&self.config.features)
            .map_err(|e| ProcessorError::export(format!("serialize features config: {e}")))?;
        let classification_json = serde_json::to_value(&self.config.classification)
            .map_err(|e| ProcessorError::export(format!("serialize classification config: {e}")))?;

        let mut builder = ExportMetadata::builder()
            .day(&self.day_str)
            .n_sequences(sequences.len())
            .window_size(window_size)
            .horizons(horizons.clone())
            .bin_size_seconds(self.config.sampling.bin_size_seconds)
            .market_open_et(&self.config.sampling.market_open_et)
            .first_bin_start_ns(summary.first_bin_start_ns)
            .last_bin_end_ns(summary.last_bin_end_ns)
            .n_bins_total(summary.total_bins_emitted as usize)
            .n_bins_valid(
                self.feature_bins.iter()
                    .enumerate()
                    .filter(|(i, _)| label_result.valid_mask.get(*i).copied().unwrap_or(false))
                    .count()
            )
            .n_bins_warmup_discarded(summary.warmup_bins_discarded as usize)
            .n_bins_label_truncated(label_result.n_truncated)
            .n_total_records(summary.total_records_processed)
            .n_trade_records(summary.total_trade_records)
            .n_trf_trades(summary.total_trf_trades)
            .n_lit_trades(summary.total_lit_trades)
            .symbol(&self.config.input.symbol)
            .signing_method("midpoint")
            .exclusion_band(self.config.classification.exclusion_band)
            // Phase 5: EQUS context in metadata
            .equs_summary_available(
                self.daily_context.as_ref().map_or(false, |c| c.has_volume())
            )
            .consolidated_volume(
                self.daily_context.as_ref().and_then(|c| c.consolidated_volume)
            )
            .trf_volume_fraction(
                if let (Some(ctx), true) = (
                    &self.daily_context,
                    summary.total_trf_volume > 0.0,
                ) {
                    ctx.consolidated_volume.map(|cv| {
                        if cv > 0 { summary.total_trf_volume / cv as f64 } else { 0.0 }
                    })
                } else {
                    None
                }
            )
            // F2 — populate previously-empty `feature_groups_enabled` and
            //      `classification_config` with the active config snapshot.
            .feature_groups_enabled(features_json)
            .classification_config(classification_json)
            // Phase 9.1 — forward_prices metadata block. Unlocks T9 LabelFactory
            // for BASIC-only training. `smoothing_window_offset = 0` is hardcoded
            // in the builder (basic-quote-processor does NOT smooth forward prices).
            .forward_prices(max_horizon);

        // Phase 9.4 provenance — conditional setters. `config_hash` is set once
        // at startup (preserved across `reset()`), `source_file` is set per-day
        // (cleared by `reset()`). Unset → Option::None → metadata omits the
        // field via `#[serde(skip_serializing_if = "Option::is_none")]`.
        if let Some(hash) = self.config_hash.as_deref() {
            builder = builder.config_hash(hash);
        }
        if let Some(src) = self.source_file.as_deref() {
            builder = builder.provenance_source_file(src);
        }
        // Phase 9.5 — emit the configured strategy, not the hardcoded
        // "per_day_zscore". If unset, metadata defaults to "none".
        if let Some(strategy) = self.normalization_strategy.as_deref() {
            builder = builder.normalization_strategy(strategy);
        }
        // Phase 9.4 — honest `applied` field. Defaults to `false` in builder
        // if never set (matches pre-Phase-9 behavior).
        if let Some(applied) = self.normalization_applied {
            builder = builder.normalization_applied(applied);
        }
        // Phase 9.4 / D13 — experiment identifier (absent in pre-Phase-9 metadata)
        if let Some(exp) = self.experiment.as_deref() {
            builder = builder.experiment(exp);
        }

        let metadata = builder.build()?;

        Ok(DayExport {
            day: self.day_str.clone(),
            sequences,
            labels: aligned_labels,
            forward_prices: aligned_fwd,
            metadata,
            normalizer: self.normalizer.clone(),
            normalization_json: norm_json,
        })
    }

    /// Convenience: init_day + stream_file + finalize in one call.
    pub fn process_day(
        &mut self,
        file_path: &Path,
        year: i32,
        month: u32,
        day: u32,
    ) -> Result<DayExport> {
        self.init_day(year, month, day);
        self.stream_file(file_path)?;
        self.finalize()
    }

    /// Reset all state for the next day.
    ///
    /// Uses per-component reset where possible (FIX #19).
    /// Config-derived parameters are preserved.
    pub fn reset(&mut self) {
        self.bbo = BboState::new();
        // FIX #25: TradeClassifier::new takes by value
        self.classifier = TradeClassifier::new(self.config.classification.clone())
            .expect("config already validated in DayPipeline::new()");
        self.sampler = TimeBinSampler::new(self.config.sampling.bin_size_seconds);
        // Phase 5: Use VPIN override if set (dynamic bucket volume from EQUS)
        if let Some(override_vol) = self.bucket_volume_override {
            let mut vpin_config = self.config.vpin.clone();
            vpin_config.bucket_volume = override_vol;
            self.accumulator = BinAccumulator::new(
                &self.config.validation, &vpin_config, &self.config.features,
            );
        } else {
            self.accumulator = BinAccumulator::new(
                &self.config.validation, &self.config.vpin, &self.config.features,
            );
        }
        self.extractor = FeatureExtractor::new(
            &self.config.features,
            &self.config.validation,
            self.config.sampling.bin_size_seconds,
        );
        self.normalizer = NormalizationComputer::new(TOTAL_FEATURES, &self.config.features);
        self.feature_bins.clear();
        self.mid_prices.clear();
        self.feature_buffer.clear();
        self.day_str.clear();
        // Phase 5: clear half-day detection state
        self.daily_context = None;
        self.consecutive_empty_bins = 0;
        self.detected_close_ns = None;
        self.bucket_volume_override = None;
        // Phase 9.4: source_file is per-day (clear); config_hash /
        // normalization_strategy / normalization_applied / experiment are
        // per-run (preserve — constant across all days in a single run).
        self.source_file = None;
    }

    /// Day-level diagnostic summary.
    pub fn day_summary(&self) -> DaySummary {
        self.accumulator.day_summary()
    }

    /// Number of post-warmup bins collected so far.
    pub fn n_bins(&self) -> usize {
        self.feature_bins.len()
    }

    /// Number of mid-prices recorded (should equal n_bins).
    pub fn n_mid_prices(&self) -> usize {
        self.mid_prices.len()
    }

    // ── Test support ─────────────────────────────────────────────────

    /// Inject synthetic feature bins and mid-prices for testing finalize()
    /// independently of stream_file().
    #[cfg(test)]
    pub(crate) fn set_test_data(
        &mut self,
        feature_bins: Vec<FeatureVec>,
        mid_prices: Vec<f64>,
    ) {
        self.feature_bins = feature_bins;
        self.mid_prices = mid_prices;
        // Update normalizer with the test bins
        for fv in &self.feature_bins {
            self.normalizer.update(fv);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        FeatureConfig, InputConfig, LabelConfig, LabelStrategy, SamplingConfig,
        SequenceConfig, ValidationConfig, VpinConfig,
    };
    use crate::trade_classifier::ClassificationConfig;

    fn test_config() -> ProcessorConfig {
        ProcessorConfig {
            input: InputConfig {
                data_dir: ".".to_string(),
                filename_pattern: "*.dbn.zst".to_string(),
                symbol: "NVDA".to_string(),
                equs_summary_path: None,
            },
            sampling: SamplingConfig::default(),
            classification: ClassificationConfig::default(),
            features: FeatureConfig::default(),
            vpin: VpinConfig::default(),
            validation: ValidationConfig::default(),
            sequence: SequenceConfig::default(),
            labeling: LabelConfig::default(),
        }
    }

    fn make_fv(id: f64) -> FeatureVec {
        let mut v = vec![0.0; TOTAL_FEATURES];
        v[0] = id;
        Arc::new(v)
    }

    #[test]
    fn test_pipeline_new_from_config() {
        let config = test_config();
        let pipeline = DayPipeline::new(&config);
        assert!(pipeline.is_ok());
        let p = pipeline.unwrap();
        assert_eq!(p.n_bins(), 0);
        assert_eq!(p.n_mid_prices(), 0);
    }

    #[test]
    fn test_pipeline_finalize_with_synthetic_data() {
        let config = test_config();
        let mut pipeline = DayPipeline::new(&config).unwrap();
        pipeline.init_day(2025, 2, 3);

        // Create 50 bins with linearly increasing mid-prices
        let bins: Vec<FeatureVec> = (0..50).map(|i| make_fv(i as f64)).collect();
        let mid_prices: Vec<f64> = (0..50).map(|i| 130.0 + 0.01 * i as f64).collect();

        pipeline.set_test_data(bins, mid_prices);

        let export = pipeline.finalize().unwrap();

        // With default config: max_horizon=60, but only 50 bins.
        // No bin has t + 60 < 50, so ALL bins are label-truncated.
        // But some bins have shorter horizons valid:
        // Actually, ALL horizons [1,2,3,5,10,20,30,60] need to be valid.
        // For bin 0: H=60 needs mid_prices[60] which doesn't exist → NaN.
        // So with only 50 bins and max_H=60, zero sequences have all horizons valid.
        // This is expected for a very short day.

        // Use smaller horizons for a meaningful test
        drop(export);
    }

    #[test]
    fn test_pipeline_finalize_small_horizons() {
        let mut config = test_config();
        config.labeling = LabelConfig {
            label_type: LabelStrategy::PointReturn,
            horizons: vec![1, 2, 3],
        };
        config.sequence = SequenceConfig { window_size: 5, stride: 1 };

        let mut pipeline = DayPipeline::new(&config).unwrap();
        pipeline.init_day(2025, 2, 3);

        // 20 bins, max_H=3, window=5
        let bins: Vec<FeatureVec> = (0..20).map(|i| make_fv(i as f64)).collect();
        let mid_prices: Vec<f64> = (0..20).map(|i| 130.0 + 0.01 * i as f64).collect();

        pipeline.set_test_data(bins, mid_prices.clone());
        let export = pipeline.finalize().unwrap();

        // Valid labels: bins 0..17 (20 - 3 = 17 valid bins)
        // Sequences with window=5, stride=1: ending_idx in [4, 16]
        // So 13 sequences: ending_idx = 4, 5, ..., 16
        assert_eq!(export.sequences.len(), 13, "Expected 13 sequences");
        assert_eq!(export.labels.len(), 13);
        assert_eq!(export.forward_prices.len(), 13);

        // Verify label alignment: label[0] corresponds to ending_idx=4
        // point_return(4, 1) = (mid[5] - mid[4]) / mid[4] * 10000
        let expected = (mid_prices[5] - mid_prices[4]) / mid_prices[4] * 10_000.0;
        assert!(
            (export.labels[0][0] - expected).abs() < 1e-8,
            "Label[0][0] should be {}, got {}",
            expected, export.labels[0][0]
        );

        // Verify forward price: column 0 = base price at ending_idx=4
        assert!(
            (export.forward_prices[0][0] - mid_prices[4]).abs() < 1e-10,
            "Forward price col 0 should be mid_prices[4]"
        );
    }

    #[test]
    fn test_pipeline_mid_price_count_equals_bin_count() {
        let config = test_config();
        let mut pipeline = DayPipeline::new(&config).unwrap();
        pipeline.init_day(2025, 2, 3);

        let bins: Vec<FeatureVec> = (0..30).map(|i| make_fv(i as f64)).collect();
        let mid_prices: Vec<f64> = (0..30).map(|_| 130.0).collect();

        pipeline.set_test_data(bins, mid_prices);
        assert_eq!(pipeline.n_bins(), pipeline.n_mid_prices());
    }

    #[test]
    fn test_pipeline_valid_mask_filters_zero_midprice() {
        let mut config = test_config();
        config.labeling.horizons = vec![1];
        config.sequence = SequenceConfig { window_size: 2, stride: 1 };

        let mut pipeline = DayPipeline::new(&config).unwrap();
        pipeline.init_day(2025, 2, 3);

        // 5 bins, but mid_prices[2] = 0.0 (BBO invalid)
        let bins: Vec<FeatureVec> = (0..5).map(|i| make_fv(i as f64)).collect();
        let mid_prices = vec![130.0, 131.0, 0.0, 133.0, 134.0];

        pipeline.set_test_data(bins, mid_prices);
        let export = pipeline.finalize().unwrap();

        // Labels: bin 0 valid (mid[1] > EPS), bin 1 invalid (mid[2] = 0),
        //         bin 2 invalid (mid[2] = 0 base), bin 3 valid (mid[4] > EPS),
        //         bin 4 invalid (end-of-day)
        // Valid endings for sequences (window=2): ending_idx in {1, 3}
        // But bin 1's label needs mid[2] = 0.0 → NaN → invalid
        // So only ending_idx = 3 is valid (mid[3]=133, mid[4]=134, both > EPS)
        // Sequence: bins [2, 3], label at ending_idx=3
        // But wait: ending_idx=3, seq_start=2, seq_start%1==0 ✓
        // And ending_idx=0? seq_start would be negative → not in range
        // ending_idx=1: label valid? mid[1]=131 > EPS, mid[2]=0 → NaN → invalid
        // ending_idx=3: label valid? mid[3]=133, mid[4]=134 → valid ✓
        // So 1 sequence

        assert!(
            !export.sequences.is_empty(),
            "Should have at least one valid sequence"
        );
        // All exported labels should be finite
        for (i, label) in export.labels.iter().enumerate() {
            for (j, &val) in label.iter().enumerate() {
                assert!(
                    val.is_finite(),
                    "Label[{}][{}] = {} is not finite",
                    i, j, val
                );
            }
        }
    }

    #[test]
    fn test_pipeline_no_feature_lookahead() {
        // Verify features in sequence[i] use only data from bins ≤ ending_idx
        let mut config = test_config();
        config.labeling.horizons = vec![1, 2];
        config.sequence = SequenceConfig { window_size: 3, stride: 1 };

        let mut pipeline = DayPipeline::new(&config).unwrap();
        pipeline.init_day(2025, 2, 3);

        // Each bin's feature[0] is its index → we can verify ordering
        let bins: Vec<FeatureVec> = (0..10).map(|i| make_fv(i as f64)).collect();
        let mid_prices: Vec<f64> = (0..10).map(|i| 130.0 + i as f64).collect();

        pipeline.set_test_data(bins, mid_prices);
        let export = pipeline.finalize().unwrap();

        // For each sequence, the last bin's feature[0] should be <= ending_idx
        for (seq_i, seq) in export.sequences.iter().enumerate() {
            let last_bin_id = seq.last().unwrap()[0];
            let ending_idx = seq_i * 1 + 3 - 1; // stride=1, window=3
            assert!(
                last_bin_id as usize <= ending_idx,
                "Sequence {} has last bin id {} > ending_idx {}",
                seq_i, last_bin_id, ending_idx
            );
            // Also verify sequence is in order
            for j in 1..seq.len() {
                assert!(
                    seq[j][0] > seq[j - 1][0],
                    "Sequence {} bins not in order at position {}",
                    seq_i, j
                );
            }
        }
    }

    #[test]
    fn test_pipeline_reset_clears_state() {
        let config = test_config();
        let mut pipeline = DayPipeline::new(&config).unwrap();
        pipeline.init_day(2025, 2, 3);

        let bins: Vec<FeatureVec> = (0..5).map(|i| make_fv(i as f64)).collect();
        let mid_prices: Vec<f64> = (0..5).map(|_| 130.0).collect();
        pipeline.set_test_data(bins, mid_prices);
        assert_eq!(pipeline.n_bins(), 5);

        pipeline.reset();
        assert_eq!(pipeline.n_bins(), 0);
        assert_eq!(pipeline.n_mid_prices(), 0);
        assert!(pipeline.day_str.is_empty());
    }

    #[test]
    fn test_pipeline_deterministic() {
        let mut config = test_config();
        config.labeling.horizons = vec![1, 2];
        config.sequence = SequenceConfig { window_size: 3, stride: 1 };

        let bins: Vec<FeatureVec> = (0..15).map(|i| make_fv(i as f64)).collect();
        let mid_prices: Vec<f64> = (0..15).map(|i| 130.0 + 0.01 * i as f64).collect();

        let mut p1 = DayPipeline::new(&config).unwrap();
        p1.init_day(2025, 2, 3);
        p1.set_test_data(bins.clone(), mid_prices.clone());
        let e1 = p1.finalize().unwrap();

        let mut p2 = DayPipeline::new(&config).unwrap();
        p2.init_day(2025, 2, 3);
        p2.set_test_data(bins, mid_prices);
        let e2 = p2.finalize().unwrap();

        assert_eq!(e1.sequences.len(), e2.sequences.len());
        for (i, (l1, l2)) in e1.labels.iter().zip(e2.labels.iter()).enumerate() {
            for (j, (&v1, &v2)) in l1.iter().zip(l2.iter()).enumerate() {
                assert_eq!(v1, v2, "Label mismatch at [{i}][{j}]");
            }
        }
    }

    #[test]
    fn test_pipeline_zero_valid_bins() {
        let config = test_config(); // max_horizon=60
        let mut pipeline = DayPipeline::new(&config).unwrap();
        pipeline.init_day(2025, 2, 3);

        // Only 10 bins, max_H=60 → zero valid sequences
        let bins: Vec<FeatureVec> = (0..10).map(|i| make_fv(i as f64)).collect();
        let mid_prices: Vec<f64> = (0..10).map(|i| 130.0 + i as f64).collect();

        pipeline.set_test_data(bins, mid_prices);
        let export = pipeline.finalize().unwrap();
        assert_eq!(export.sequences.len(), 0, "Very short day → 0 sequences");
    }

    #[test]
    fn test_pipeline_normalization_broader_than_export() {
        let mut config = test_config();
        config.labeling.horizons = vec![1, 2];
        config.sequence = SequenceConfig { window_size: 3, stride: 1 };

        let mut pipeline = DayPipeline::new(&config).unwrap();
        pipeline.init_day(2025, 2, 3);

        // 10 bins: all contribute to normalization stats
        // But only bins 0..8 have valid labels (max_H=2, so last 2 truncated)
        // Sequences: ending_idx in [2, 7] → 6 sequences
        let bins: Vec<FeatureVec> = (0..10).map(|i| make_fv(i as f64 + 1.0)).collect();
        let mid_prices: Vec<f64> = (0..10).map(|i| 130.0 + i as f64).collect();

        pipeline.set_test_data(bins, mid_prices);
        let export = pipeline.finalize().unwrap();

        // Normalization used all 10 bins
        let result = export.normalizer.finalize("test");
        assert_eq!(result.sample_count, 10, "Normalization should use all 10 bins");
        // But only 6 sequences exported (bins 0-7 valid, window=3, stride=1)
        assert_eq!(export.sequences.len(), 6, "Only 6 sequences exported");
    }

    #[test]
    fn test_pipeline_sequence_label_alignment() {
        let mut config = test_config();
        config.labeling.horizons = vec![1];
        config.sequence = SequenceConfig { window_size: 3, stride: 2 };

        let mut pipeline = DayPipeline::new(&config).unwrap();
        pipeline.init_day(2025, 2, 3);

        // 10 bins with distinct mid-prices
        let bins: Vec<FeatureVec> = (0..10).map(|i| make_fv(i as f64)).collect();
        let mid_prices: Vec<f64> = (0..10).map(|i| 100.0 + i as f64 * 10.0).collect();

        pipeline.set_test_data(bins, mid_prices.clone());
        let export = pipeline.finalize().unwrap();

        // Stride=2: ending indices at 2, 4, 6, 8 (seq_start=0,2,4,6)
        // But valid labels: bins 0..8 (max_H=1, 10-1=9 valid)
        // ending_idx=8: mid[9]/mid[8] → valid
        for (seq_i, label_row) in export.labels.iter().enumerate() {
            let ending_idx = seq_i * 2 + 3 - 1; // stride=2, window=3
            let expected = (mid_prices[ending_idx + 1] - mid_prices[ending_idx])
                / mid_prices[ending_idx] * 10_000.0;
            assert!(
                (label_row[0] - expected).abs() < 1e-6,
                "Seq {} label mismatch: expected {}, got {}",
                seq_i, expected, label_row[0]
            );
        }
    }

    // ── Phase 9.4: provenance setter + finalize wiring tests ───────────

    #[test]
    fn test_pipeline_set_config_hash_preserved_across_reset() {
        // Phase 9.4 — `config_hash` is set once at startup and preserved across
        // `reset()` because the config does not change across days in a single run.
        let config = test_config();
        let mut pipeline = DayPipeline::new(&config).unwrap();
        pipeline.set_config_hash("deadbeef".to_string());
        assert_eq!(pipeline.config_hash.as_deref(), Some("deadbeef"));
        pipeline.reset();
        assert_eq!(
            pipeline.config_hash.as_deref(),
            Some("deadbeef"),
            "config_hash must be preserved across reset() (same config all days)"
        );
    }

    #[test]
    fn test_pipeline_set_source_file_cleared_on_reset() {
        // F1 — `source_file` changes per day and must be cleared by `reset()` so
        // the next day's metadata does not leak the previous day's filename.
        let config = test_config();
        let mut pipeline = DayPipeline::new(&config).unwrap();
        pipeline.set_source_file("2025-02-03.dbn.zst".to_string());
        assert_eq!(pipeline.source_file.as_deref(), Some("2025-02-03.dbn.zst"));
        pipeline.reset();
        assert_eq!(
            pipeline.source_file, None,
            "source_file must be cleared by reset() (per-day state)"
        );
    }

    #[test]
    fn test_pipeline_finalize_emits_provenance_fields() {
        // F1, F2, 9.4 — when setters are called, `finalize()` emits those fields
        // in the metadata. Also verifies F2 populates the previously-empty
        // `feature_groups_enabled` and `classification_config` objects.
        let mut config = test_config();
        config.labeling.horizons = vec![1, 2];
        config.sequence = SequenceConfig { window_size: 3, stride: 1 };
        config.validation.warmup_bins = 0;

        let mut pipeline = DayPipeline::new(&config).unwrap();
        pipeline.init_day(2025, 2, 3);
        let fake_hash: String = "c".repeat(64);
        pipeline.set_config_hash(fake_hash.clone());
        pipeline.set_source_file("xnas-basic-20250203.cmbp-1.dbn.zst".to_string());

        let bins: Vec<FeatureVec> = (0..6).map(|i| make_fv(i as f64)).collect();
        let mid_prices: Vec<f64> = (0..6).map(|i| 100.0 + i as f64).collect();
        pipeline.set_test_data(bins, mid_prices);

        let export = pipeline.finalize().unwrap();

        // 9.4 — config_hash threaded through
        assert_eq!(
            export.metadata.provenance.config_hash.as_ref(),
            Some(&fake_hash),
            "config_hash must appear in metadata.provenance"
        );
        // F1 — source_file threaded through (basename, not empty string)
        assert_eq!(
            export.metadata.provenance.source_file,
            "xnas-basic-20250203.cmbp-1.dbn.zst",
            "source_file must be threaded through finalize"
        );
        // F2 — populated (no longer empty `{}`)
        assert_ne!(
            export.metadata.feature_groups_enabled,
            serde_json::json!({}),
            "feature_groups_enabled must be populated (F2)"
        );
        assert_ne!(
            export.metadata.classification_config,
            serde_json::json!({}),
            "classification_config must be populated (F2)"
        );
        // F2 — feature_groups_enabled must contain the actual field structure
        assert!(
            export.metadata.feature_groups_enabled
                .get("signed_flow").is_some(),
            "feature_groups_enabled must reflect FeatureConfig fields"
        );
    }

    #[test]
    fn test_pipeline_set_normalization_strategy_threads_through_finalize() {
        // Round 7 (Agent 4 recommendation): catches the bug where
        // `set_normalization_strategy` and `set_normalization_applied` are
        // silently disconnected from the builder chain in finalize().
        let mut config = test_config();
        config.labeling.horizons = vec![1];
        config.sequence = SequenceConfig { window_size: 3, stride: 1 };
        config.validation.warmup_bins = 0;

        let mut pipeline = DayPipeline::new(&config).unwrap();
        pipeline.init_day(2025, 2, 3);
        pipeline.set_normalization_strategy("per_day_zscore".to_string());
        pipeline.set_normalization_applied(true);

        let bins: Vec<FeatureVec> = (0..5).map(|i| make_fv(i as f64)).collect();
        let mid_prices: Vec<f64> = (0..5).map(|i| 100.0 + i as f64).collect();
        pipeline.set_test_data(bins, mid_prices);

        let export = pipeline.finalize().unwrap();

        assert_eq!(
            export.metadata.normalization.strategy, "per_day_zscore",
            "Strategy must be threaded from setter to metadata"
        );
        assert!(
            export.metadata.normalization.applied,
            "Applied must be threaded from setter to metadata"
        );
    }

    #[test]
    fn test_pipeline_set_experiment_preserved_across_reset() {
        // Round 7 / D13: experiment identifier is per-run state (constant
        // across days). Must survive reset() for the same reason config_hash does.
        let config = test_config();
        let mut pipeline = DayPipeline::new(&config).unwrap();
        pipeline.set_experiment("basic_nvda_60s_phase9".to_string());
        assert_eq!(pipeline.experiment.as_deref(), Some("basic_nvda_60s_phase9"));
        pipeline.reset();
        assert_eq!(
            pipeline.experiment.as_deref(),
            Some("basic_nvda_60s_phase9"),
            "experiment must be preserved across reset (per-run state)"
        );
    }

    #[test]
    fn test_pipeline_set_normalization_applied_preserved_across_reset() {
        // Round 7: normalization_applied is per-run state (same across all days).
        let config = test_config();
        let mut pipeline = DayPipeline::new(&config).unwrap();
        pipeline.set_normalization_applied(true);
        assert_eq!(pipeline.normalization_applied, Some(true));
        pipeline.reset();
        assert_eq!(
            pipeline.normalization_applied,
            Some(true),
            "normalization_applied must be preserved across reset (per-run state)"
        );
    }

    #[test]
    fn test_h10_bucket_volume_override_set_during_init_with_context() {
        // Round 8 / H10 regression: bucket_volume_override must be SET during
        // `init_day_with_context` (not deferred to the next reset()). Before
        // the fix, day N used day (N-1)'s volume for VPIN bucket sizing;
        // day 0 used the static default. After the fix, day N uses day N's
        // volume IMMEDIATELY. This test asserts the override FIELD is set
        // correctly — the accumulator rebuild is tested indirectly by the
        // non-panic of this call path with vpin_config cloned from the
        // dynamic bucket.
        use chrono::NaiveDate;
        let mut config = test_config();
        config.features.vpin = true;
        config.vpin.bucket_volume_fraction = Some(0.02); // 2% of daily vol

        let mut pipeline = DayPipeline::new(&config).unwrap();
        let context = crate::context::DailyContext {
            date: NaiveDate::from_ymd_opt(2025, 2, 3).unwrap(),
            consolidated_volume: Some(100_000_000), // 100M shares
            daily_open: None,
            daily_high: None,
            daily_low: None,
            daily_close: None,
        };
        pipeline.init_day_with_context(2025, 2, 3, Some(context));

        // Override = 100_000_000 * 0.02 = 2_000_000
        assert_eq!(
            pipeline.bucket_volume_override,
            Some(2_000_000),
            "bucket_volume_override = consolidated_volume * fraction"
        );
        // Also verify daily_context is stored
        assert!(pipeline.daily_context.is_some());
        assert_eq!(
            pipeline.daily_context.as_ref().unwrap().consolidated_volume,
            Some(100_000_000)
        );
    }

    #[test]
    fn test_h10_no_override_when_context_missing() {
        // If daily context lacks consolidated_volume, override stays None
        // and accumulator uses the static default bucket_volume.
        let mut config = test_config();
        config.features.vpin = true;
        config.vpin.bucket_volume_fraction = Some(0.02);

        let mut pipeline = DayPipeline::new(&config).unwrap();
        let context = crate::context::DailyContext::fallback(
            chrono::NaiveDate::from_ymd_opt(2025, 2, 3).unwrap(),
        );
        pipeline.init_day_with_context(2025, 2, 3, Some(context));
        assert_eq!(pipeline.bucket_volume_override, None);
    }

    #[test]
    fn test_h10_no_override_when_fraction_unset() {
        // If config has no bucket_volume_fraction, the EQUS context is not
        // consulted for override — static bucket_volume is used.
        let mut config = test_config();
        config.features.vpin = true;
        // bucket_volume_fraction left at default (None)

        let mut pipeline = DayPipeline::new(&config).unwrap();
        let context = crate::context::DailyContext {
            date: chrono::NaiveDate::from_ymd_opt(2025, 2, 3).unwrap(),
            consolidated_volume: Some(100_000_000),
            daily_open: None,
            daily_high: None,
            daily_low: None,
            daily_close: None,
        };
        pipeline.init_day_with_context(2025, 2, 3, Some(context));
        assert_eq!(
            pipeline.bucket_volume_override, None,
            "No fraction configured → no override computed"
        );
    }

    #[test]
    fn test_pipeline_finalize_emits_experiment_when_set() {
        // Round 7 / D13: when set_experiment is called, metadata.experiment is emitted.
        let mut config = test_config();
        config.labeling.horizons = vec![1];
        config.sequence = SequenceConfig { window_size: 3, stride: 1 };
        config.validation.warmup_bins = 0;

        let mut pipeline = DayPipeline::new(&config).unwrap();
        pipeline.init_day(2025, 2, 3);
        pipeline.set_experiment("basic_nvda_60s_phase9".to_string());

        let bins: Vec<FeatureVec> = (0..5).map(|i| make_fv(i as f64)).collect();
        let mid_prices: Vec<f64> = (0..5).map(|i| 100.0 + i as f64).collect();
        pipeline.set_test_data(bins, mid_prices);

        let export = pipeline.finalize().unwrap();
        assert_eq!(
            export.metadata.experiment.as_deref(),
            Some("basic_nvda_60s_phase9"),
            "experiment field must be threaded into metadata"
        );
    }

    #[test]
    fn test_pipeline_finalize_without_provenance_setters_omits_fields() {
        // Backward compat: if `set_config_hash` / `set_source_file` are never
        // called, the metadata's `config_hash` is None (skipped on serialize)
        // and `source_file` is "" (existing default behavior).
        let mut config = test_config();
        config.labeling.horizons = vec![1];
        config.sequence = SequenceConfig { window_size: 3, stride: 1 };
        config.validation.warmup_bins = 0;

        let mut pipeline = DayPipeline::new(&config).unwrap();
        pipeline.init_day(2025, 2, 3);

        let bins: Vec<FeatureVec> = (0..5).map(|i| make_fv(i as f64)).collect();
        let mid_prices: Vec<f64> = (0..5).map(|i| 100.0 + i as f64).collect();
        pipeline.set_test_data(bins, mid_prices);

        let export = pipeline.finalize().unwrap();

        assert!(
            export.metadata.provenance.config_hash.is_none(),
            "Unset config_hash must remain None in metadata"
        );
        assert_eq!(
            export.metadata.provenance.source_file, "",
            "Unset source_file defaults to empty string (preserves v0 behavior)"
        );
    }
}
