# CODEBASE.md — basic-quote-processor

Off-exchange trade processing for XNAS.BASIC CMBP-1 data. Standalone Rust crate.

> **Pipeline scope (2026-06-02).** This module is part of an **intraday trading research pipeline** — an experiment-first platform for discovering and validating *any* profitable **intraday** trading edge (no overnight positions), across approach classes (microstructure/HFT, scalping, intraday momentum, intraday statistical arbitrage, …) and instruments (equities, futures, same-day options). The pipeline *originated* as a high-frequency NVDA MBO/LOB microstructure system — that origin explains the "HFT" / "LOB" / "MBO" naming here — and that microstructure-direction program is now one (largely-closed) track among many. **Names are historical; the mission is general.** This module's role: a Rust processor turning XNAS.BASIC CMBP-1 (L1 NBBO + off-exchange TRF trades) into 34 features + point-return labels at configurable time-bins — a cheaper, quote-level data on-ramp (no full order book required). For the full mission + approach taxonomy + capability-readiness boundary, see root `CLAUDE.md` §Research Scope & Charter (+ `CROSS_ASSET_OFI_FINDINGS_AND_ISSUES_2026_06_01.md` §9).

**Status**: Phases 1-5 complete + Phase 9 Experimentation Foundation complete (test counts are deliberately not hand-typed here per hft-rules §11 — run `cargo test` for the live count)
**Schema**: off_exchange 1.0 (independent of MBO schema 3.0)
**Features**: 34 (indices 0-33), 10 groups
**Labels**: Point-return only (no smoothed), 8 horizons [1,2,3,5,10,20,30,60]

---

## Build & Test

```bash
cargo build --release          # Build lib + 3 CLI binaries
cargo test                     # Run all tests (lib + integration)
cargo test --lib               # Run lib tests only
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
| `export/` | 6 | NPY writing (f32/f64), normalization stats, metadata JSON, manifest, diagnostics sidecar |
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
| `DayPipeline` | pipeline | Per-day orchestrator: 23 fields (incl. Phase-9 provenance/diagnostics), split lifecycle |
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
| `{day}_diagnostics.json` | — | JSON | per-day health counters (`DaySummary`); `schema_version` 1.2.0 (1.1.0 added `dropped_invalid_price_trades`; 1.2.0 added `decode_errors` + `decode_truncated`) |

The multi-day `dataset_manifest.json` additionally carries `diagnostics_files` — a list of `<split>/<day>_diagnostics.json` relative paths aggregating the per-day sidecars. Per-day `{day}_metadata.json` also includes `provenance.git_commit` / `provenance.git_dirty` (captured at build time via `build.rs`) and `provenance.data_file_sha256` (streaming SHA-256 of the raw input `.dbn.zst`, for Databento-re-issue detection; omitted if hashing fails). The manifest also carries `zero_sequence_days` (ISO dates that streamed OK but produced 0 sequences; also present in `splits.*.days[]`, an explicit empty-day annotation).

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

## Test Inventory

> Per-file test counts are deliberately NOT hand-maintained here (hft-rules §11 — they drift; a 2026-07-07 audit found the previous counts stale by exactly the Phase-9 additions). For live counts run `cargo test` (totals) or `grep -rc '#\[test\]' src/ tests/` (per-file).

| File | Coverage |
|------|---------|
| contract.rs | Constants, names, horizons |
| error.rs | All error variants |
| hash.rs | Streaming SHA-256 of a file (empty / known-vector / multichunk / error) |
| reader/*.rs | Record types, publisher IDs, file discovery |
| bbo_state/*.rs | BBO tracking, midpoint, validation, edge cases |
| trade_classifier/*.rs | Signing, BJZZ, BVC golden values, BVC sigma-window eviction (mutation-proven: shrink + strict-`>` boundary) |
| config.rs | All config types, validation, TOML roundtrip, canonical serialization + golden config-hash regression (Phase 9.3) |
| sampling/*.rs | EST/EDT, DST transition days, gap detection, session boundaries |
| accumulator/*.rs | Sub-accumulators, reset, diagnostics, volumes, invalid-price guard (①), forward-fill index SSoT lock (slot tables ↔ `indices::*`) |
| features/*.rs | All 34 formulas, indices, classification, sign-convention contract (§10), golden value tests (#11) |
| sequence_builder/ | Sliding window, Arc sharing, stride |
| labeling/*.rs | Point-return, forward prices, golden tests |
| export/*.rs | NPY shapes, normalization (strategy-threading ③), metadata, manifest, diagnostics sidecar, data_file_sha256, zero_sequence_days |
| pipeline.rs | Finalize, alignment, determinism, per-day provenance reset, decode-diagnostics surfacing (②) |
| context.rs | EQUS loading, fallback, date lookup |
| dates.rs | Weekday enum, split, date parsing |
| **Integration** (tests/, 6 files) | 5 data-gated files (real NVIDIA data, full pipeline, shapes) + `contract_conformance_test.rs` (data-less: validates `ExportMetadata` JSON conformance against a hand-mirrored copy of the Python validator's required-field list — it does NOT read `pipeline_contract.toml` or reference `FEATURE_NAMES`) |

---

## Known Limitations / Deferred Items

### Accepted Deviations

- `[publishers]` config section deferred — using hardcoded PublisherClass::from_id()
- Half-day detection pre-detection bins have ~2% session_progress error (accepted AD1)
- `equs_summary_path` optional (spec says required, code overrides for library usability — AD2)
- Phase 6 (Python analysis scripts) not yet implemented — all analysis performed via `lob-dataset-analyzer` on the exported NPY files
- Single-feed XNAS.BASIC coverage = 61.2% (not 81-85% which requires multi-feed fusion)

### Validated Design Items (Post-Push Improvements)

These were identified during a 3-agent deep audit of all 41 source files. None are bugs — they are extensibility and quality items for future commits.

- **TRADE_COUNT (idx 22) and BIN_TRADE_COUNT (idx 27) produce identical values** — both compute `total_trades`. One is in the toggleable trade_size group, the other in the always-on activity group. Removing requires schema version bump.
- **`finalize()` does not dispatch on `LabelStrategy` enum** — always uses `LabelComputer` (PointReturn). Latent: only one variant exists. Fix when adding second label strategy.
- **Config structs lack `Serialize` derive** — ~~`ProcessorConfig` has `Deserialize` but not `Serialize`~~. **RESOLVED in Phase 9.3** — all 14 config structs/enums now derive `Serialize`. `ProcessorConfig::to_canonical_toml()` is the single canonical serialization path, and `ProcessorConfig::config_hash_hex()` provides a 64-char SHA-256 for provenance.
- **`SamplingConfig.strategy` and `ValidationConfig.empty_bin_policy` are String, not enum** — **RESOLVED in commit 8e46608 (2026-05-28)** — both are now typed Rust enums (`SamplingStrategy::TimeBased`, `EmptyBinPolicy::{ForwardFillState, ZeroAll, NanAll}`) with `#[serde(rename_all = "snake_case")]`. TOML wire-format is byte-identical (config_hash preserved; golden hash regression test pins `c142f466...`). Unknown variants fail at serde deserialization. **Follow-up (post-commit)**: `EmptyBinPolicy::{ZeroAll, NanAll}` are not implemented in `features/mod.rs`; `ValidationConfig::validate()` now fail-fast rejects them per hft-rules §5.
- **No `Sampler` trait** — `TimeBinSampler` is hardwired in `pipeline.rs`. Extract trait when adding volume-based or composite sampling.
- **BVC uses sample variance (n-1), BurstTracker uses population variance (n)** — both defensible for their contexts (BVC estimates population parameter from sample, BurstTracker is descriptive statistic of bin data).
- **`ExportMetadata.normalization.strategy` is hardcoded to `"per_day_zscore"`** — **RESOLVED in Phase 9.5** — strategy is now configurable via `ExportMetadataBuilder::normalization_strategy(&str)` and defaults to `"none"`. The CLI (`export_dataset.rs`) threads `config.export.normalization` into the builder via `DayPipeline::set_normalization_strategy(String)`.

<!-- 2026-05-30 adversarial validation audit (5 fresh-eyes module auditors + 2 REFUTE agents, ground-truth code over docs): the module is SOUND — all 34 feature formulas, the BVC/BJZZ/midpoint math, temporal alignment (leakage-free), DST offsets (hand-verified), determinism, reset semantics, and the producer↔Python-consumer contract verified correct. Three actionable items were found and fixed (below). Deferred/blocked: the `hft-statistics` `branch="main"` pin → tag migration is cross-repo blocked (changes the compiled DST path; the committed Cargo.lock pins rev `e976ff7` so a given build IS reproducible — see M-2 note); the idx-2 inverted-polarity is a downstream contract-doc note (below); `git_dirty` is tracked-files-only (honestly documented at `metadata.rs`). -->
- **Invalid/sentinel-price trades were accumulated unguarded (BVC σ-window poisoning)** — **RESOLVED (2026-05-30)** — `BinAccumulator::accumulate()` now skips trades with `price <= 0` or non-finite (the `0.0` sentinel `TradeClassifier::classify` emits for UNDEF/non-positive prices), per `docs/design/03_DATA_FLOW.md §"Price sanity"`. Previously such a trade poisoned the BVC sigma window (and the next trade's delta) and its real `size` inflated volume/count/size features, while the sibling VPIN consumer already guarded the identical input. The drop is counted in `DaySummary.dropped_invalid_price_trades` and surfaced in the diagnostics sidecar (hft-rules §8). Tests: `accumulator::tests::{test_accumulate_skips_invalid_sentinel_price, test_accumulate_processes_valid_price_unaffected_by_guard}`.
- **Reader decode errors were never persisted (silent-truncation blind spot)** — **RESOLVED (2026-05-30)** — `RecordIterator` now exposes `aborted()`; `DayPipeline::stream_file` captures `decode_errors` + the abort/truncation flag, and `day_summary()` surfaces them as `DaySummary.{decode_errors, decode_truncated}` → diagnostics sidecar (§8). A partially-corrupt or 1000-consecutive-error-truncated day is now visible offline instead of looking clean (`total_records_processed` counts only successes, so it was not a proxy). Test: `pipeline::tests::test_pipeline_day_summary_surfaces_decode_diagnostics`.
- **`{day}_normalization.json` sidecar `strategy` was hardcoded `"per_day_zscore"`** (the SIDECAR analog of the metadata fix above) — **RESOLVED (2026-05-30)** — `NormalizationComputer::{finalize,to_json}` now take `strategy: &str` threaded from `DayPipeline.normalization_strategy` (→ `"none"` for raw v3p0-style exports), so the sidecar's reported strategy matches `metadata.json` + the actual on-disk data (hft-rules §11). Previously the sidecar self-contradicted metadata; no consumer was misled (the Python validator reads the metadata strategy, not the sidecar's), so this is an artifact-honesty fix. Test: `normalization::tests::test_finalize_strategy_is_threaded_not_hardcoded`. NOTE: the live stale `basic_nvda_60s/` export keeps its old `"per_day_zscore"` sidecar until re-exported (Roadmap #33); root `CLAUDE.md` §Cross-Module Data Contracts still describes that stale on-disk value.

<!-- 2026-05-30 coverage-hardening cycle (test-only; 2 Wave-1 audit agents + 2 Wave-2 adversarial validators, ground-truth-over-docs). Two REAL gaps surfaced beyond the 3 fixes above; both closed with TEST-ONLY additions (ZERO production-code change → config_hash / M-2 dormant-patch / determinism all untouched; lib 446→449). Two further candidates were adversarially REJECTED — see Roadmap #36/#37 for the deferred/dropped dispositions. -->
- **BVC sigma-window time-eviction was the one correctness-critical UNTESTED path** — **RESOLVED (2026-05-30, test-only)** — the rolling-window eviction loop in `BvcState::classify_trade` (`bvc.rs` `while let Some(&oldest_ts)`) had ZERO direct coverage: every prior BVC test kept trades inside the 60s window (`test_golden_bvc_determinism_sequence` asserts `window_size()==5` = nothing evicted), so a broken eviction would SILENTLY corrupt sigma → the exported `bvc_imbalance` feature (idx 3). Added `bvc::tests::{test_eviction_shrinks_window_and_changes_sigma, test_eviction_boundary_is_strict_greater_than}` (drive the already-public `window_size()`/`current_sigma()`; no new accessor). **Mutation-proven non-tautological**: flipping the predicate `>`→`>=` fails the boundary test (`window_size` 2≠3); swapping the `saturating_sub` operands (disabling eviction) fails BOTH (4≠1 / 4≠3). The eviction predicate (`bvc.rs:104`) remains byte-identical to HEAD.
- **Forward-fill index set had two UNLOCKED sources of truth** — **RESOLVED (2026-05-30, test-only)** — `forward_fill.rs` (`TRF_STATE_SLOTS`/`BBO_STATE_SLOTS`/`VPIN_STATE_SLOTS`) and `indices.rs` (`FORWARD_FILL_INDICES`/`BBO_STATE_INDICES`/`FORWARD_FILL_VPIN_INDICES`) hand-encode the same fact; `indices::tests::test_all_indices_classified` never compared them, so adding a forward-fill index to one module but not the other would silently disable forward-fill for that feature — a live hazard the moment the 34-feature layout is extended for new feature-set configs. Added `forward_fill::tests::test_slot_tables_match_indices_ssot` asserting the union invariant (`TRF ∪ BBO == FORWARD_FILL_INDICES`), `BBO == BBO_STATE_INDICES`, `VPIN == FORWARD_FILL_VPIN_INDICES`, TRF/BBO disjoint, + cardinality (8/2/2). hft-rules §0/§1.
- **idx-2 `inv_inst_direction` has the INVERTED sign convention vs all other signed features** — by design (`= -MROIB`, documented at `features/indices.rs`), correct and tested. Downstream contract-clarity note: a fusion/normalization layer applying a uniform "positive=bullish" polarity across the signed-flow block (idx 0-3) would invert this one signal; consult the per-feature polarity, do not assume uniformity.
- **`ExportMetadata.feature_groups_enabled` and `.classification_config` emitted as empty `{}`** — **RESOLVED in Phase 9.4 / F2** — `pipeline.rs::finalize()` now serializes the active `FeatureConfig` and `ClassificationConfig` via `serde_json::to_value(&...)` and threads them into the builder, so metadata carries the actual config values alongside `config_hash`.
- **`ExportMetadata.provenance.source_file` emitted as empty string** — **RESOLVED in Phase 9.4 / F1** — `DayPipeline::set_source_file(String)` is called by the CLI per-day with the `.dbn.zst` basename (cleared by `reset()`).
- **No `forward_prices` metadata block** — **RESOLVED in Phase 9.1** — `ExportMetadata::forward_prices: Option<ForwardPricesMeta>` with 6 fields matching `contracts/pipeline_contract.toml [forward_prices.metadata]` and `hft-contracts.ForwardPriceContract.from_metadata()`. Unblocks T9 LabelFactory pathway for BASIC-only training.
- **H2: final partial bin drop** — **RESOLVED in Phase 9.2** — `pipeline.rs` and `tests/phase3_test.rs` final-flush condition now `has_trades() || bbo_update_count() > 0`. BBO-only bins are emitted into `feature_bins` but filtered out of sequences via `valid_mask` (all-NaN labels since no `t+h` bins exist beyond).
- **BUG-X2: `is_empty = trf_trades == 0` in `emit_bin()` (`src/pipeline.rs` — line anchors drift; grep for `trf_trades() == 0`) excludes lit-only bins** — DEFERRED (LOW severity, diagnostic counters only; does not affect labels/sequences).

### Audit Findings — Ground-Truth Verdicts (2026-05-29 re-validation)

A 4-agent ground-truth re-validation (depend-on-code-not-docs) reclassified the open audit findings. **Do not re-investigate these without new evidence:**

- **M-1 (BVC window grows unbounded on non-monotonic timestamps)** — **defensive-only, NOT a live bug.** `saturating_sub` skips eviction only on a backward `ts_ns`, but the real XNAS.BASIC feed produces only sub-millisecond out-of-order artifacts ("warn and accept" policy); one late trade is re-evicted by the next in-order trade (≈1 ms extra retention on a 60s window). Same "phantom boundary cluster" pattern as the MBO-LOB-reconstructor audit.
- **L-3 (`midpoint_signer` no zero-spread guard)** — **PHANTOM.** Correctly delegated to upstream `bbo_valid` (spread > 0, finite prices); the signer returns `Unsigned` before the band math when invalid.
- **L-4 (gap-bin warmup uses accumulator `bin_index`)** — **PHANTOM.** Sampler and accumulator `bin_index` stay in lockstep (one `reset_bin()` per gap iteration). Readability nit at most.
- **L-8 (no `dataset_manifest.json`)** — **PHANTOM / stale finding.** `src/export/manifest.rs` already writes a full manifest (now also `diagnostics_files[]`).
- **`EmptyBinPolicy::{ZeroAll, NanAll}` unimplemented** — **correct permanent design**, not a gap. Fail-loud-rejected per §5; implementing them is speculative. Do NOT implement absent a real experiment need.
- **Whole-crate incompleteness sweep** — **N = 0 incomplete implementations** (no `todo!`/`unimplemented!`/unwired-config/scaffolding; the only "not yet implemented" strings are fail-loud rejections). The crate is cleanly closeable.

**⚠️ Engineering trap (future atomic-write / SSoT work):** reusing `hft_statistics::io::atomic_write_json` from this crate RE-TRIGGERS the M-2 incident — it flips the local `.cargo/config.toml` `[[patch.unused]]` block active, swaps `hft-statistics` git-`0.1.0` → local `0.3.0-dev`, and drags `tempfile` into the production dep graph (Cargo.lock churn). The 2026-05-29 diagnostics sidecar therefore rides the EXISTING temp-dir+rename envelope. If true single-file manifest atomicity is ever needed, add a small LOCAL `tempfile` helper — do NOT pull `hft_statistics::io` while the local patch override exists.

**Operational debt:** the live `data/exports/basic_nvda_60s/` is STALE (pre-Phase-9.4: empty `config_hash`, `processor_version` `0.1.0`). The wired traceability (`config_hash`, `git_commit`/`git_dirty`, diagnostics sidecar) only reaches the DATA after a re-export; deferred (0 configs reference it today — see Roadmap #33).

### Test Coverage Roadmap (Future Work)

A 5-agent audit identified P1 coverage gaps. Adding these tests strengthens the safety net without requiring code changes:

1. **Half-day auto-detect unit test** — inject 10 synthetic empty bins; assert `set_session_end()` is called. Currently relies on Christmas Eve real-data file.
2. **DST transition tests** — `TimeBinSampler::init_day(2025, 3, 9)` (spring forward) and `(2025, 11, 2)` (fall back); verify offset switches. **RESOLVED (2026-05-30)**: `test_init_day_dst_transition_days` asserts the spring-forward day (2025-03-09 → EDT -4) + fall-back day (2025-11-02 → EST -5) + the Saturday-before each (opposite regime), bracketing both transition boundaries.
3. **File with only trades, no quotes** — verify `TradeClassifier` correctly returns `Unsigned + Unknown` for all trades when no BBO updates exist.
4. **Truly empty `.dbn.zst` file** — current `test_edge_empty_iterator` uses `.take(0)`; need a test on a zero-record file.
5. **Convert `debug_assert!` → `assert!` in `src/features/mod.rs:164-179`** for safety_gates, schema_version, session_progress range — **RESOLVED in commit 8e46608 (2026-05-28)** — 6 `debug_assert!` calls promoted to `assert!`, enforcing `bin_valid`, `bbo_valid`, `schema_version`, `session_progress` invariants in release builds. One `debug_assert!` for `extract_context` regime sanity remains intentional (covered by `match` exhaustiveness).
6. **Sign convention contract test** — explicit per-feature: `buy_vol > sell_vol` ⇒ `trf_signed_imbalance > 0`; same for mroib, bvc_imbalance, quote_imbalance. **RESOLVED (2026-05-30)**: 4 `test_sign_convention_*` tests in `features/mod.rs` lock all 5 signed features (incl. the `INV_INST_DIRECTION = -MROIB` inversion) across buy-pressure>0 / sell-pressure<0 / balanced==0; BVC driven via `accumulate_bvc`, QUOTE via the BBO snapshot.
7. **VPIN below bucket_volume fallback** — feed one trade; verify `trf_vpin = 0.0` (not NaN).
8. **Gap-bin-at-end-of-day** — synthetic stream where the last emitted bin is a gap; verify `last_bin_end_ns` reflects the gap.
9. **`set_session_end()` impact** — verify session_progress clamping respects the auto-detected end.
10. **Integration test gating** — **RESOLVED in this commit (PARTIAL)** — all 5 integration test files (`classifier_test`, `integration_test`, `phase3_test`, `phase4_test`, `phase5_test`) now panic if data is missing AND `CI` env var is set, preserving local-dev silent-skip behavior otherwise. Same pattern applied to `equs_available()` in phase5_test. Note: a few orphan path-checks (e.g., `test_discover_files`, second-day path in `classifier_test::test_no_state_leakage_between_days`) bypass `data_available()` and remain silent-skip — fix in a follow-up.
11. **Missing golden tests** for 10 features: `retail_volume_fraction`, `quote_imbalance`, `spread_change_rate`, `mean_trade_size`, `block_trade_ratio`, `trf_lit_volume_ratio`, `odd_lot_ratio`, `retail_trade_rate`, `time_bucket` regimes 4/5, VPIN fallback. **RESOLVED (2026-05-30)**: 10 golden/value tests in `features/mod.rs` (hand-derived from §5 formulas) — shared `extract_trf_trades` helper for the count/volume ratios; inline setup for `spread_change_rate` (`accumulate_bbo_update`), `time_bucket` regimes 4/5 (afternoon/close-auction boundaries), and the VPIN fallback (vpin-enabled config → `current_vpin()` None → exactly 0.0).
12. **`git_commit` / `git_dirty` provenance via `build.rs`** — **RESOLVED (2026-05-29)** — `build.rs` shells to `git rev-parse HEAD` + `git diff --quiet HEAD` at compile time, exposing `GIT_COMMIT_HASH`/`GIT_DIRTY` rustc-env vars consumed via `option_env!` in `ProvenanceMeta` (`src/export/metadata.rs`), which now emits `git_commit` + `git_dirty` ("unknown"/false fallback). Mirrors the MBO extractor `build.rs`. Crate version also bumped `0.1.0` → `0.9.0` so `processor_version` is a meaningful staleness signal.
13. **Frozen golden-hash regression test** — **RESOLVED in commit 8e46608 (2026-05-28)** — `test_config_hash_golden_regression` in `src/config.rs` pins SHA-256 `c142f46663ae401bd9ae3250b3f7e9d3047b09db425d19050aecfdbb22ea11fa` for the `sample_processor_config()` fixture; detects drift from serde_derive, toml minor version bumps, or accidental struct-field reordering.
14. **`reset_bin` implicitly clears stats** — load-bearing invariant for the H2 half-day safety argument. Add a named test asserting the invariant (Phase 10+). **RESOLVED (2026-05-30)**: `test_reset_bin_zeros_per_bin_preserves_persistent` (already covered flow/counts/bin_index/bvc) strengthened with a `stats.total_volume` clearing assertion — the per-bin StatsAccumulator was the one sub-accumulator the named test had not covered.
15. **`validate_off_exchange_export_contract`** (Python consumer) does not yet validate the `forward_prices` block presence/shape — add in `hft-contracts` alongside Phase 10.

### Phase 10+ Roadmap (surfaced by Round 8 agent validation)

> **⚠️ NOT the same as the cross-repo "Phase 10/11" in `docs/plan/EXPERIMENT_ORCHESTRATION_LAYER.md`.** That design (SQLite+Parquet Ledger v2 + Experiment Envelope) is a SEPARATE, **cross-repo** effort whose implementation lands in `hft-ops` + `hft-contracts` + root `contracts/pipeline_contract.toml` + trainer + backtester. The doc explicitly states *"BQP is NOT touched in Phase 11"* (§2.5.1) and even the Phase-11.5 BQP-envelope is contract-first-blocked on the upstream schema/canonical-JSON/fingerprint. **It is NOT a bqp-internal task and is BLOCKED while those sibling repos are sister-active** (verified 2026-05-30, 3-agent recon — do not start it from this repo). The numbered items below (#16–33) are the actual bqp roadmap.

**Architectural (worth planning for):**
16. **Runtime-derived `TOTAL_FEATURES`** — currently a compile-time `usize` with a fixed `[&str; 34]` `FEATURE_NAMES` array. Adding a new feature group forces a schema-breaking change. Phase 11+ should make this runtime-derived from `FeatureConfig.enabled_feature_count()` so feature additions are additive under a bumped `contract_version`.
17. **Extract `process_record()` from `stream_file()`** — the 100-line streaming for-loop body is the streaming hot path. Factoring it out as `fn process_record(&mut self, record: &CmbpRecord) -> Option<FeatureEmission>` would preserve streaming optionality for Phase 13+ without forking 100 lines of code later.
18. **Refactor provenance setters into a single `Provenance` struct** — the setter count on `DayPipeline` has reached 5 (`set_config_hash`, `set_source_file`, `set_normalization_strategy`, `set_normalization_applied`, `set_experiment`, `set_symbol` planned). Before adding a 6th, collapse the per-run subset (`config_hash`, `normalization_strategy`, `normalization_applied`, `experiment`) into `set_run_provenance(RunProvenance)`.
19. **Extract `Sampler` and `Labeler` traits** — DO IT at the moment a second implementation lands (triple-barrier, volume sampling), NOT before. Accumulating two more concrete implementations before the trait makes the eventual refactor harder.

**Observability (forensic / operational):**
20. **`--skip-existing` idempotent resume** — currently `--force` re-runs ALL 233 days from scratch; a mid-run failure costs a full re-run. Read existing `dataset_manifest.json.splits.*.days[]` and skip dates already present. Saves ~12 min per config in sweep runs. **CUT (2026-05-30 adversarial review).** Speculative for the CLOSED BASIC dataset (both directional + magnitude axes closed per E17 2026-05-29; 1 config, 0 sweep manifests, SSD unmounted) — building sweep-resume now is an hft-rules §0 "don't build for hypothetical futures" miss. **If ever built** (a real BASIC sweep is actually scheduled): `record_day`/`diagnostics_files`/`zero_sequence_days` are APPEND-ONLY, so loading + resuming a manifest DOUBLE-COUNTS `total_sequences`/`days_processed` and duplicates `diagnostics_files[]` → fails the C1/C2 `validate_export_dir` and `phase5_test.rs` exact-count asserts. Requirements: idempotent-by-date recording; atomic manifest write (tmp+rename, not bare `fs::write`); refresh stale header fields (`processor_version`/`schema_version`/`contract_version`) on load; reconcile stale `failed_days` for retried dates. Treat #20+#22 as ONE coupled feature, loaded only when `!--force`.
21. **Per-day timing breakdown in metadata** — currently only wall-clock elapsed seconds in stderr. Adding `read_time_ms`, `extract_time_ms`, `write_time_ms` to `ExportMetadata` enables performance regression detection across sweep runs. (Deferred 2026-05-30: a clean `read_time_ms`/`extract_time_ms` split requires instrumenting the `stream_file` hot loop, which fuses decode+extract; a coarse `process`/`finalize`/`write` split is the cheap honest version. Secondary to the forensic bundle — revisit with a real sweep.)
22. **Config-drift detection on `--force`** — read existing manifest's `config_hash`, warn (or refuse without `--clean`) if it differs from the new hash. Prevents silent inconsistency when mixing partial re-runs of different configs into the same `output_dir`. **CUT (2026-05-30 adversarial review)** — same rationale as #20 (speculative; coupled to the manifest-load there). Disposition when built: 3-state (no manifest = fresh; config matches + resume = resume; config differs = refuse with message; `--force` = overwrite from scratch, NOT load). Do NOT overload `--force` to also mean "ignore drift."
23. **Surface silent fallbacks** — `source_basename.unwrap_or("")` and `let _ = fs::write(config_copy_path, ...)` currently swallow errors. Add `log::warn!` on empty basename or failed config copy. **RESOLVED (2026-05-30)**: `export_dataset.rs` now warns on a non-UTF8/empty `source_basename` and on BOTH the config-copy re-read AND write failures; warn-and-continue (never aborts the export).
24. **Promote `Ok(0)` sequences to an explicit status** — currently `export.sequences.is_empty()` is recorded as a successful day. Consider either (a) demoting it to `record_failure` with a specific reason, or (b) adding `n_days_zero_seq` to manifest so sweep consumers can differentiate broken days from legitimately empty days. **RESOLVED (2026-05-30)**: `dataset_manifest.json.zero_sequence_days: Vec<String>` (`#[serde(default)]`) records 0-sequence days IN ADDITION to `splits.*.days[]` (counts unchanged; does NOT flip `complete` — observation, not failure).

**Schema (Schema 2.0 consolidation):**
25. **De-duplicate metadata ↔ manifest** — ~12 fields overlap with 3 naming drifts (`n_features` vs `feature_count`, `window_size` vs `sequence_length`, `label_strategy` vs `labeling_strategy`). Schema 2.0 should consolidate to a single naming convention across both files and remove redundancy (e.g., manifest keeps dataset-level facts, metadata keeps per-day facts only, manifest_ref pointer in metadata).
26. **Remove duplicate `export_timestamp`** — metadata has both top-level `export_timestamp` and nested `provenance.export_timestamp_utc`, set to identical values. Pick one (recommend `provenance.export_timestamp_utc`).
27. **Convert string-valued enums to Rust enums** — `schema`, `data_source`, `signing_method`, `label_encoding`, `normalization.strategy` are de-facto enums but typed as `String`. Converting to Rust enums with `#[serde(rename_all)]` gives parse-time validation.
28. **Add `data_file_sha256` to provenance** — input file content hash enables detection of "same source_file path but different content" (databento re-issue). ~10 LOC, high forensic value. Round 8 M7.1. **RESOLVED (2026-05-30)**: per-day `provenance.data_file_sha256` via `crate::hash::sha256_file` (streaming SHA-256 of the raw compressed `.dbn.zst`, reusing the existing `sha2` dep — NOT `hft_statistics::io`, per the M-2 trap); cleared by `reset()`; warn-and-continue if hashing fails.
29. **Document n_bins accounting invariant** — DONE IN ROUND 8 (doc comment added). Consider adding a `debug_assert!` in `finalize()` enforcing `n_bins_total == n_bins_valid + n_bins_label_truncated` to catch regressions.

**H10 VPIN fix forward-compatibility:**
30. **VPIN integration tests** — H10 regression test (Round 8) only asserts the `bucket_volume_override` field is SET correctly. When `vpin = true` is ever enabled in a production config, add an end-to-end test that confirms the first day's VPIN bucket reflects the first day's consolidated_volume (not the default 5000 or day N−1's volume).

**Cross-repo (next traceability cycle — needs sibling repos free):**
31. **Consumer-side `validate_export_dir` gate (hft-contracts)** — the higher-leverage "monitorable" win: assert manifest↔disk count parity + uniform schema/commit per export dir, reusing the per-day `validate_off_exchange_export_contract`. Would auto-catch a stale/corrupt export (e.g. the current `basic_nvda_60s`). Approved design in monorepo `FOUNDATION_INTEGRITY_PLAN_2026_05.md`; the producer-side traceability shipped 2026-05-29 is its prerequisite. Blocked while hft-contracts is sister-active.
32. **hft-ops BASIC extraction stage (M-3)** — a `BasicExtractionRunner` parallel to the MBO `ExtractionRunner` so BASIC datasets get orchestrated/cached/ledgered runs (currently CLI-only via `export_dataset`). Harvest the new per-day diagnostics sidecar as the dataset-health surface.
33. **Re-export `basic_nvda_60s`** — refresh the stale live export so the DATA carries `config_hash` + git provenance + diagnostics sidecars (needs the external SSD + confirmation BASIC is on-roadmap). NOTE: this also retires root `CLAUDE.md` §Cross-Module-Data-Contracts L335 ("BASIC sidecar `strategy` = per_day_zscore") — accurate for the STALE on-disk export, but post-③ a fresh export reports `"none"`; update that shared doc in a coordinated edit when re-exporting.
34. **(optional) Quantify ① invalid-price-trade frequency** — the 2026-05-30 invalid/sentinel-price guard in `accumulate()` is correct regardless of frequency, but the real-XNAS.BASIC *trade-record* frequency of `price<=0`/UNDEF is UNVERIFIED (Databento sentinels usually ride quotes/status, not trades). The new `DaySummary.dropped_invalid_price_trades` counter makes it measurable on the next export. LOW priority — confirms materiality only; does not change the fix.
35. **Migrate `hft-statistics` pin `branch="main"` → `tag`/`rev`** — the single §7 reproducibility item (audit-surfaced). The committed `Cargo.lock` pins rev `e976ff7` and the local `[[patch.unused]]` is dormant BY DESIGN (M-2), so a given build IS reproducible; the residual is only `cargo update`-drift off the moving branch. CROSS-REPO BLOCKED: bumping the pin changes the compiled crate (incl. the DST-offset path used by the sampler) → requires export re-validation, gated on a tagged `hft-statistics` release matching what bqp was validated against. Mirror the `dbn` `tag="v0.20.0"` pattern. (The stale `deps/hft-statistics-v0.2.1` branch is NOT a usable fix — it predates Phase 9.)
36. **(deferred, design-validated) Diagnostics sidecar for 0-sequence days that have a health anomaly** — `DayExporter::export_day` returns early for 0-seq days, so a day that streamed with `decode_errors>0 || decode_truncated || dropped_invalid_price_trades>0` but produced 0 sequences writes NO `{day}_diagnostics.json` (only the date lands in `dataset_manifest.json.zero_sequence_days[]`) — this partially defeats fix ② for the corrupt-0-seq-day corner. **DEFERRED 2026-05-30** (adversarially validated): this intersection has ZERO observed occurrences on the 233-day NVDA corpus (a few corrupt records don't zero a full day; the truncation abort is itself near-unreachable per #37; an all-invalid-price NVDA day is absurd), so building it now is speculative completeness, not a fix for an observed problem — and it adds a `diagnostics_files[]` / `zero_sequence_days[]` partition nuance. BUILD ONLY IF a 0-seq-anomaly day is ever observed. **Validated SAFE design (turnkey):** add `pub fn day_summary_is_diagnostic_worthy(&DaySummary) -> bool` in `export/mod.rs` as the SINGLE source for the predicate; have `export_day` (still returning `Ok(0)`) write a sidecar-only file via the same temp-dir + atomic-rename envelope when the predicate holds; have `export_dataset.rs` append that path to `manifest.diagnostics_files[]` using the SAME predicate (no orphan, no omission). `test_day_exporter_zero_sequences_skips` (all-zero summary) stays green as the no-spam guard.
37. **(dropped, evaluated) `decode_truncated` does not catch silent-EOF truncation — in-tree docs already honest** — fix ②'s `decode_truncated` only trips on the 1000-CONSECUTIVE-decode-error abort. The dominant real truncation (a `.dbn.zst` cut mid-record / mid-zstd-frame) surfaces as a clean `Ok(None)` EOF via dbn's `silence_eof_error`, so it is NOT detected (`decode_errors=0, decode_truncated=false`). **DROPPED 2026-05-30** (nothing to fix in-tree): the field doc (`accumulator/mod.rs`) and the ② bullet above already scope it precisely to the consecutive-error abort + disclaim `total_records_processed` as a proxy — only the immutable commit-message prose of `8d091ee` oversold it. A `metadata.end`-based detector was EVALUATED and REJECTED: dbn v0.20.0 `Metadata` has no record-count field, and `end` is the query boundary (end-of-day) for date-range pulls, not the last-record ts → a clean quiet-tail day has `last_ts ≪ end`, making the check false-positive-prone (and `end` is `Option`). No sound in-band truncation detector exists; do not re-litigate.

---

### Next-Session Entry Point (post-`8d091ee` + 2026-05-30 coverage-hardening cycle `419cad0`)

**bqp is validated SOUND.** A from-scratch adversarial audit (5 fresh-eyes module auditors + 2 REFUTE agents, ground-truth code over docs, converged findings) verified correct: all 34 feature formulas, the BVC/BJZZ/midpoint math, leakage-free feature→label alignment, DST offsets (hand-verified at all 4 transition dates), determinism, reset semantics, and the producer↔Python-consumer contract; §0 reuse-first is clean. The 3 actionable findings it surfaced are FIXED (① invalid-price accumulation guard, ② decode-error observability, ③ normalization-sidecar honesty — see the Validated Design Items section + commit `8d091ee`). A follow-on 2026-05-30 coverage-hardening cycle (2 audit + 2 adversarial agents) then added TWO TEST-ONLY guards (lib 446→449, ZERO production change → config_hash / M-2 patch / determinism untouched; shipped as `419cad0`): the BVC sigma-window eviction tests (mutation-proven to bite `>`→`>=` and eviction-disabled) + the forward-fill index SSoT lock — closing the one correctness-critical untested path (idx-3 `bvc_imbalance`) + the one unlocked single-source-of-truth seam (relevant for future feature-set extensions). Two further candidates were adversarially **DEFERRED/DROPPED**: Roadmap #36 (0-seq-anomaly sidecar — safe design recorded, but unmotivated by any observed case) and #37 (silent-EOF truncation — in-tree docs already honest; no sound in-band detector exists).

- **bqp-internal work is now LOW-VALUE**: the remaining test gaps (#1 half-day auto-detect, #8 gap-bin-at-end, #9 `set_session_end`, #3 no-quotes, #4 empty-file — #3/#4 likely need a synthetic `.dbn` fixture) + the optional #34. Producer formula/sign/DST/reset/contract correctness is LOCKED by tests (the lib suite + the data-less `contract_conformance_test.rs` suite; run `cargo test` for live counts).
- **The HIGH-VALUE next work is CROSS-REPO and currently BLOCKED**: #31 `validate_export_dir` consumer gate in hft-contracts (the "monitorable" win), #32 hft-ops BASIC stage, #33 re-export — all need a sister-free hft-contracts/hft-ops and/or the external SSD. Do NOT start them from bqp. #35 (dep-pin) is a latent reproducibility item.
- **Do NOT re-flag the audit's REFUTED non-bugs**: idx-2 `inv_inst_direction`'s inverted sign is INTENDED (`= -MROIB`; a downstream polarity-doc note only); the dep-pin "reproducibility risk" is OVER-STATED (the committed lock pins `e976ff7`; the patch is dormant by design — see #35).
- **Stale pointer**: root `SESSION_HANDOFF_2026_05_29.md` (archived 2026-07-06 to root `.archive/2026-07-06-root-md-cleanup/SESSION_HANDOFF_2026_05_29.md`) predates this cycle; the current state is THIS section + commit `8d091ee` + the git log (+ agent memory `project_2026_05_27_bqp_audit.md` Follow-on 5 for full detail).

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
