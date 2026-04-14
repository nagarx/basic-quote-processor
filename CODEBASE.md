# CODEBASE.md — basic-quote-processor

Off-exchange trade processing for XNAS.BASIC CMBP-1 data. Standalone Rust crate.

**Status**: Phases 1-5 complete + Phase 9 Experimentation Foundation complete (471 tests: 408 lib + 63 integration)
**Schema**: off_exchange 1.0 (independent of MBO schema 2.2)
**Features**: 34 (indices 0-33), 10 groups
**Labels**: Point-return only (no smoothed), 8 horizons [1,2,3,5,10,20,30,60]

---

## Build & Test

```bash
cargo build --release          # Build lib + 3 CLI binaries
cargo test                     # Run all 471 tests
cargo test --lib               # Run 408 lib tests only
cargo clippy --all-targets     # Lint check
```

## CLI Tools

```bash
# Multi-day export with train/val/test splits
export_dataset --config configs/nvda_60s.toml

# Coverage validation against EQUS_SUMMARY
validate_coverage --config configs/nvda_60s.toml

# Per-day diagnostic statistics
profile_data --config configs/nvda_60s.toml --date 2025-02-03
```

---

## Architecture (13 Modules)

```
.dbn.zst (CMBP-1) ──► reader/ ──► CmbpRecord
                                      │
                         bbo_state/ ◄─┤──► BboState (L1 BBO tracking)
                                      │
                    trade_classifier/ ◄┘──► ClassifiedTrade
                         │                 (midpoint signing + BJZZ retail + BVC)
                         v
                    accumulator/ ──► BinAccumulator (per-bin state)
                         │           6 sub-accumulators:
                         │           flow, count, stats, burst, forward_fill, BVC
                         v
                    features/ ──► FeatureExtractor ──► Vec<f64> (34 features)
                         │
                         v
                    sequence_builder/ ──► FeatureVec = Arc<Vec<f64>>
                    labeling/ ──► LabelComputer (point-return bps)
                    export/ ──► DayExporter (NPY + metadata + normalization)
                         │
                         v
                    pipeline.rs ──► DayPipeline (init → stream → finalize)
                    context.rs ──► DailyContextLoader (EQUS_SUMMARY)
                    dates.rs ──► weekday enumeration, split assignment
```

---

## Module Summary

| Module | Files | Purpose |
|--------|-------|---------|
| `reader/` | 4 | Read .dbn.zst, convert CbboMsg → CmbpRecord, classify publishers |
| `bbo_state/` | 3 | Track Nasdaq L1 BBO (bid/ask/mid/spread), validate crossed books |
| `trade_classifier/` | 5 | Midpoint signing (Barber 2024), BJZZ retail (Boehmer 2021), BVC (Easley 2012) |
| `config.rs` | 1 | TOML config: ProcessorConfig + DatasetConfig + sub-configs |
| `sampling/` | 2 | Grid-aligned time bins, DST-aware, gap detection |
| `accumulator/` | 6 | Per-bin state: volumes, counts, TWAP, burst, forward-fill, BVC, VPIN |
| `features/` | 2 | 34-feature extraction with 3-level empty bin policy |
| `sequence_builder/` | 1 | Sliding window, FeatureVec = Arc<Vec<f64>> |
| `labeling/` | 3 | Point-return labels + forward price trajectories |
| `export/` | 5 | NPY writing (f32/f64), normalization stats, metadata JSON, manifest |
| `pipeline.rs` | 1 | DayPipeline orchestrator: init_day → stream_file → finalize |
| `context.rs` | 1 | EQUS_SUMMARY daily context (consolidated volume, OHLCV) |
| `dates.rs` | 1 | Weekday enumeration, train/val/test split, date format helpers |

---

## Key Types

| Type | Module | Description |
|------|--------|-------------|
| `CmbpRecord` | reader | Internal CMBP-1 record (i64 nanodollar prices) |
| `BboState` | bbo_state | L1 BBO with midpoint, spread, microprice |
| `ClassifiedTrade` | trade_classifier | Trade with direction (Buy/Sell/Unsigned) + retail status |
| `PublisherClass` | reader | Enum: Trf, Lit, MinorLit, QuoteOnly, Unknown |
| `BinAccumulator` | accumulator | Per-bin state orchestrator (6 sub-accumulators) |
| `DaySummary` | accumulator | Day-level diagnostics (cumulative trades, volumes, timestamps) |
| `FeatureExtractor` | features | Stateless: reads accumulator → produces Vec<f64> |
| `FeatureVec` | sequence_builder | `Arc<Vec<f64>>` for zero-copy window sharing |
| `DayPipeline` | pipeline | Per-day orchestrator: 15 fields, split lifecycle |
| `DayExport` | export | Bundle: sequences + labels + forward_prices + metadata |
| `DayExporter` | export | Atomic file writer with rollback on failure |
| `DatasetManifest` | export/manifest | Multi-day export manifest with completion tracking |
| `DailyContext` | context | EQUS_SUMMARY per-day data (volume, OHLCV) |
| `NormalizationComputer` | export/normalization | Per-feature Welford z-score with disabled-group exclusion |
| `ExportMetadata` | export/metadata | Complete per-day metadata JSON (all spec fields) |
| `LabelComputer` | labeling | Point-return labels with valid_mask filtering |
| `DatasetConfig` | config | CLI-level config wrapping ProcessorConfig + dates + export |

---

## Feature Groups (34 total, indices 0-33)

| Group | Indices | Count | Toggleable | Default |
|-------|---------|-------|------------|---------|
| signed_flow | 0-3 | 4 | Yes | On |
| venue_metrics | 4-7 | 4 | Yes | On |
| retail_metrics | 8-11 | 4 | Yes | On |
| bbo_dynamics | 12-17 | 6 | Yes | On |
| vpin | 18-19 | 2 | Yes | **Off** |
| trade_size | 20-23 | 4 | Yes | On |
| cross_venue | 24-26 | 3 | Yes | On |
| activity | 27-28 | 2 | Always | On |
| safety_gates | 29-30 | 2 | Always | On |
| context | 31-33 | 3 | Always | On |

Categorical (non-normalizable): [29, 30, 32, 33]

---

## Export Format

| File | Shape | Dtype | Unit |
|------|-------|-------|------|
| `{day}_sequences.npy` | [N, 20, 34] | float32 | raw or normalized |
| `{day}_labels.npy` | [N, 8] | float64 | basis points |
| `{day}_forward_prices.npy` | [N, 61] | float64 | USD |
| `{day}_metadata.json` | — | JSON | all spec fields |
| `{day}_normalization.json` | — | JSON | per-feature stats |

---

## Processing Loop (Canonical Pattern)

```
for record in dbn_file:
    1. CHECK BIN BOUNDARY first → extract previous bin
    2. BBO UPDATE always → pre-market warm-start
    3. TRADE CLASSIFICATION gated by is_in_session()
```

At each post-warmup bin: store FeatureVec + mid_price + update normalization.
Half-day detection: 10 consecutive empty bins → break + set_session_end().

---

## Contract Constants (`src/contract.rs`)

| Constant | Value |
|----------|-------|
| EPS | 1e-8 |
| NANO_TO_USD | 1e-9 |
| SCHEMA_VERSION | 1.0 |
| CONTRACT_VERSION | "off_exchange_1.0" |
| TOTAL_FEATURES | 34 |
| DEFAULT_HORIZONS | [1, 2, 3, 5, 10, 20, 30, 60] |
| DEFAULT_WINDOW_SIZE | 20 |
| DEFAULT_STRIDE | 1 |
| FEATURE_NAMES | 34-element array (validated against pipeline_contract.toml) |

---

## Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| hft-statistics | git (github.com/nagarx/hft-statistics) | Welford, VPIN, time regime, DST offset, phi() |
| dbn | v0.20.0 (git) | CbboMsg, OhlcvMsg decode |
| ndarray + ndarray-npy | 0.15 / 0.8 | NPY file writing |
| clap | 4 | CLI argument parsing |
| chrono | 0.4 | Date handling |
| serde + serde_json + toml | 1.0 | Config + metadata serialization |
| thiserror | 1.0 | Error type derivation |
| log + env_logger | 0.4 / 0.11 | Logging facade + output driver |
| tempfile (dev) | 3.8 | Test temporary directories |

---

## Test Inventory (412 total)

| File | Tests | Coverage |
|------|-------|---------|
| contract.rs | 9 | Constants, names, horizons |
| error.rs | 8 | All error variants |
| reader/*.rs | 15 | Record types, publisher IDs, file discovery |
| bbo_state/*.rs | 37 | BBO tracking, midpoint, validation, edge cases |
| trade_classifier/*.rs | 55 | Signing, BJZZ, BVC golden values |
| config.rs | 33 | All config types, validation, TOML roundtrip |
| sampling/*.rs | 14 | EST/EDT, gap detection, session boundaries |
| accumulator/*.rs | 73 | Sub-accumulators, reset, diagnostics, volumes |
| features/*.rs | 18 | All 34 formulas, indices, classification |
| sequence_builder/ | 11 | Sliding window, Arc sharing, stride |
| labeling/*.rs | 22 | Point-return, forward prices, golden tests |
| export/*.rs | 36 | NPY shapes, normalization, metadata, manifest |
| pipeline.rs | 11 | Finalize, alignment, determinism |
| context.rs | 11 | EQUS loading, fallback, date lookup |
| dates.rs | 12 | Weekday enum, split, date parsing |
| **Integration** (tests/) | 47 | Real NVIDIA data, full pipeline, shapes |

---

## Known Limitations / Deferred Items

### Accepted Deviations

- `[publishers]` config section deferred — using hardcoded PublisherClass::from_id()
- Half-day detection pre-detection bins have ~2% session_progress error (accepted AD1)
- `equs_summary_path` optional (spec says required, code overrides for library usability — AD2)
- Phase 6 (Python analysis scripts) not yet implemented — all analysis performed via `lob-dataset-analyzer` on the exported NPY files
- Phase 6 (Python analysis scripts) not yet implemented
- Single-feed XNAS.BASIC coverage = 61.2% (not 81-85% which requires multi-feed fusion)

### Validated Design Items (Post-Push Improvements)

These were identified during a 3-agent deep audit of all 41 source files. None are bugs — they are extensibility and quality items for future commits.

- **TRADE_COUNT (idx 22) and BIN_TRADE_COUNT (idx 27) produce identical values** — both compute `total_trades`. One is in the toggleable trade_size group, the other in the always-on activity group. Removing requires schema version bump.
- **`finalize()` does not dispatch on `LabelStrategy` enum** — always uses `LabelComputer` (PointReturn). Latent: only one variant exists. Fix when adding second label strategy.
- **Config structs lack `Serialize` derive** — ~~`ProcessorConfig` has `Deserialize` but not `Serialize`~~. **RESOLVED in Phase 9.3** — all 14 config structs/enums now derive `Serialize`. `ProcessorConfig::to_canonical_toml()` is the single canonical serialization path, and `ProcessorConfig::config_hash_hex()` provides a 64-char SHA-256 for provenance.
- **`SamplingConfig.strategy` and `ValidationConfig.empty_bin_policy` are String, not enum** — validated at runtime against hardcoded lists. Should be proper Rust enums for type-level safety.
- **No `Sampler` trait** — `TimeBinSampler` is hardwired in `pipeline.rs`. Extract trait when adding volume-based or composite sampling.
- **BVC uses sample variance (n-1), BurstTracker uses population variance (n)** — both defensible for their contexts (BVC estimates population parameter from sample, BurstTracker is descriptive statistic of bin data).
- **`ExportMetadata.normalization.strategy` is hardcoded to `"per_day_zscore"`** — **RESOLVED in Phase 9.5** — strategy is now configurable via `ExportMetadataBuilder::normalization_strategy(&str)` and defaults to `"none"`. The CLI (`export_dataset.rs`) threads `config.export.normalization` into the builder via `DayPipeline::set_normalization_strategy(String)`.
- **`ExportMetadata.feature_groups_enabled` and `.classification_config` emitted as empty `{}`** — **RESOLVED in Phase 9.4 / F2** — `pipeline.rs::finalize()` now serializes the active `FeatureConfig` and `ClassificationConfig` via `serde_json::to_value(&...)` and threads them into the builder, so metadata carries the actual config values alongside `config_hash`.
- **`ExportMetadata.provenance.source_file` emitted as empty string** — **RESOLVED in Phase 9.4 / F1** — `DayPipeline::set_source_file(String)` is called by the CLI per-day with the `.dbn.zst` basename (cleared by `reset()`).
- **No `forward_prices` metadata block** — **RESOLVED in Phase 9.1** — `ExportMetadata::forward_prices: Option<ForwardPricesMeta>` with 6 fields matching `contracts/pipeline_contract.toml [forward_prices.metadata]` and `hft-contracts.ForwardPriceContract.from_metadata()`. Unblocks T9 LabelFactory pathway for BASIC-only training.
- **H2: final partial bin drop** — **RESOLVED in Phase 9.2** — `pipeline.rs` and `tests/phase3_test.rs` final-flush condition now `has_trades() || bbo_update_count() > 0`. BBO-only bins are emitted into `feature_bins` but filtered out of sequences via `valid_mask` (all-NaN labels since no `t+h` bins exist beyond).
- **BUG-X2: `is_empty = trf_trades == 0` at `pipeline.rs:307` excludes lit-only bins** — DEFERRED (LOW severity, diagnostic counters only; does not affect labels/sequences).

### Test Coverage Roadmap (Future Work)

A 5-agent audit identified P1 coverage gaps. Adding these tests strengthens the safety net without requiring code changes:

1. **Half-day auto-detect unit test** — inject 10 synthetic empty bins; assert `set_session_end()` is called. Currently relies on Christmas Eve real-data file.
2. **DST transition tests** — `TimeBinSampler::init_day(2025, 3, 9)` (spring forward) and `(2025, 11, 2)` (fall back); verify offset switches.
3. **File with only trades, no quotes** — verify `TradeClassifier` correctly returns `Unsigned + Unknown` for all trades when no BBO updates exist.
4. **Truly empty `.dbn.zst` file** — current `test_edge_empty_iterator` uses `.take(0)`; need a test on a zero-record file.
5. **Convert `debug_assert!` → `assert!` in `src/features/mod.rs:164-179`** for safety_gates, schema_version, session_progress range — currently invariants are stripped in release builds.
6. **Sign convention contract test** — explicit per-feature: `buy_vol > sell_vol` ⇒ `trf_signed_imbalance > 0`; same for mroib, bvc_imbalance, quote_imbalance.
7. **VPIN below bucket_volume fallback** — feed one trade; verify `trf_vpin = 0.0` (not NaN).
8. **Gap-bin-at-end-of-day** — synthetic stream where the last emitted bin is a gap; verify `last_bin_end_ns` reflects the gap.
9. **`set_session_end()` impact** — verify session_progress clamping respects the auto-detected end.
10. **Integration test gating** — currently silently SKIP without data; add `CI=true` panic to prevent silent zero-coverage CI runs.
11. **Missing golden tests** for 10 features: `retail_volume_fraction`, `quote_imbalance`, `spread_change_rate`, `mean_trade_size`, `block_trade_ratio`, `trf_lit_volume_ratio`, `odd_lot_ratio`, `retail_trade_rate`, `time_bucket` regimes 4/5, VPIN fallback.
12. **`git_commit` / `git_dirty` provenance via `build.rs`** — currently metadata emits `processor_version` (from `env!("CARGO_PKG_VERSION")`) but no git commit hash. Phase 10 will add a `build.rs` script that shells to `git rev-parse HEAD` at compile time and exposes via `env!("GIT_COMMIT")`.
13. **Frozen golden-hash regression test** — assert that a fixed-config input produces a known 64-char hex hash (Phase 10, detects accidental drift from serde_derive or toml minor version bumps).
14. **`reset_bin` implicitly clears stats** — load-bearing invariant for the H2 half-day safety argument. Add a named test asserting the invariant (Phase 10+).
15. **`validate_off_exchange_export_contract`** (Python consumer) does not yet validate the `forward_prices` block presence/shape — add in `hft-contracts` alongside Phase 10.

### Phase 10+ Roadmap (surfaced by Round 8 agent validation)

**Architectural (worth planning for):**
16. **Runtime-derived `TOTAL_FEATURES`** — currently a compile-time `usize` with a fixed `[&str; 34]` `FEATURE_NAMES` array. Adding a new feature group forces a schema-breaking change. Phase 11+ should make this runtime-derived from `FeatureConfig.enabled_feature_count()` so feature additions are additive under a bumped `contract_version`.
17. **Extract `process_record()` from `stream_file()`** — the 100-line streaming for-loop body is the streaming hot path. Factoring it out as `fn process_record(&mut self, record: &CmbpRecord) -> Option<FeatureEmission>` would preserve streaming optionality for Phase 13+ without forking 100 lines of code later.
18. **Refactor provenance setters into a single `Provenance` struct** — the setter count on `DayPipeline` has reached 5 (`set_config_hash`, `set_source_file`, `set_normalization_strategy`, `set_normalization_applied`, `set_experiment`, `set_symbol` planned). Before adding a 6th, collapse the per-run subset (`config_hash`, `normalization_strategy`, `normalization_applied`, `experiment`) into `set_run_provenance(RunProvenance)`.
19. **Extract `Sampler` and `Labeler` traits** — DO IT at the moment a second implementation lands (triple-barrier, volume sampling), NOT before. Accumulating two more concrete implementations before the trait makes the eventual refactor harder.

**Observability (forensic / operational):**
20. **`--skip-existing` idempotent resume** — currently `--force` re-runs ALL 233 days from scratch; a mid-run failure costs a full re-run. Read existing `dataset_manifest.json.splits.*.days[]` and skip dates already present. Saves ~12 min per config in sweep runs.
21. **Per-day timing breakdown in metadata** — currently only wall-clock elapsed seconds in stderr. Adding `read_time_ms`, `extract_time_ms`, `write_time_ms` to `ExportMetadata` enables performance regression detection across sweep runs.
22. **Config-drift detection on `--force`** — read existing manifest's `config_hash`, warn (or refuse without `--clean`) if it differs from the new hash. Prevents silent inconsistency when mixing partial re-runs of different configs into the same `output_dir`.
23. **Surface silent fallbacks** — `source_basename.unwrap_or("")` and `let _ = fs::write(config_copy_path, ...)` currently swallow errors. Add `log::warn!` on empty basename or failed config copy.
24. **Promote `Ok(0)` sequences to an explicit status** — currently `export.sequences.is_empty()` is recorded as a successful day. Consider either (a) demoting it to `record_failure` with a specific reason, or (b) adding `n_days_zero_seq` to manifest so sweep consumers can differentiate broken days from legitimately empty days.

**Schema (Schema 2.0 consolidation):**
25. **De-duplicate metadata ↔ manifest** — ~12 fields overlap with 3 naming drifts (`n_features` vs `feature_count`, `window_size` vs `sequence_length`, `label_strategy` vs `labeling_strategy`). Schema 2.0 should consolidate to a single naming convention across both files and remove redundancy (e.g., manifest keeps dataset-level facts, metadata keeps per-day facts only, manifest_ref pointer in metadata).
26. **Remove duplicate `export_timestamp`** — metadata has both top-level `export_timestamp` and nested `provenance.export_timestamp_utc`, set to identical values. Pick one (recommend `provenance.export_timestamp_utc`).
27. **Convert string-valued enums to Rust enums** — `schema`, `data_source`, `signing_method`, `label_encoding`, `normalization.strategy` are de-facto enums but typed as `String`. Converting to Rust enums with `#[serde(rename_all)]` gives parse-time validation.
28. **Add `data_file_sha256` to provenance** — input file content hash enables detection of "same source_file path but different content" (databento re-issue). ~10 LOC, high forensic value. Round 8 M7.1.
29. **Document n_bins accounting invariant** — DONE IN ROUND 8 (doc comment added). Consider adding a `debug_assert!` in `finalize()` enforcing `n_bins_total == n_bins_valid + n_bins_label_truncated` to catch regressions.

**H10 VPIN fix forward-compatibility:**
30. **VPIN integration tests** — H10 regression test (Round 8) only asserts the `bucket_volume_override` field is SET correctly. When `vpin = true` is ever enabled in a production config, add an end-to-end test that confirms the first day's VPIN bucket reflects the first day's consolidated_volume (not the default 5000 or day N−1's volume).

---

## Design Specification

See `docs/design/` (7 documents, ~7,000 lines):
- 01_THEORETICAL_FOUNDATION.md — 47 papers, trade classification theory
- 02_MODULE_ARCHITECTURE.md — Repository structure, design decisions
- 03_DATA_FLOW.md — End-to-end data flow, BBO ordering
- 04_FEATURE_SPECIFICATION.md — All 34 features with exact formulas
- 05_CONFIGURATION_SCHEMA.md — TOML config reference
- 06_INTEGRATION_POINTS.md — MBO fusion, EQUS integration, export format
- 07_TESTING_STRATEGY.md — 6-phase plan, decision gates, E9 targets
