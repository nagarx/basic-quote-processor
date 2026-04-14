# Module Architecture: basic-quote-processor

**Status**: Design Specification — **Implementation Status**: Phases 1-5 complete (412 tests, 41 source files, 3 CLI binaries)
**Date**: 2026-03-22 (spec), 2026-03-23 (implementation)
**Scope**: Repository structure, module boundaries, key design decisions, and dependency map for the `basic-quote-processor` crate
**Prerequisite**: [01_THEORETICAL_FOUNDATION.md](01_THEORETICAL_FOUNDATION.md) -- mathematical and statistical foundations
**Plan source**: Approved architecture plan (golden-questing-moore, reviewed 2026-03-22)

---

## Table of Contents

1. [Repository Identity](#1-repository-identity)
2. [Directory Structure](#2-directory-structure)
3. [Key Design Decisions](#3-key-design-decisions)
4. [Module Boundaries](#4-module-boundaries)
5. [Pattern Justification](#5-pattern-justification)
6. [Key Types](#6-key-types)
7. [Dependencies](#7-dependencies)

---

## 1. Repository Identity

**Name**: `basic-quote-processor`
**Location**: `basic-quote-processor/`
**Language**: Rust (core processing) + Python (analysis scripts, exploration)
**Scope**: Process XNAS.BASIC CMBP-1 data -> extract off-exchange features -> export NPY datasets

### What It IS

- A standalone Rust crate that reads Databento XNAS.BASIC `.dbn.zst` files
- Classifies TRF trades (midpoint signing, BJZZ retail identification)
- Computes off-exchange features at configurable time bins
- Exports NPY sequences + labels + metadata for downstream consumption
- Includes Python analysis scripts for exploration and signal validation

### What It Is NOT

- NOT an order book reconstructor (CMBP-1 has no order lifecycle)
- NOT a replacement for the MBO feature extractor (different data, different features)
- NOT a fusion module (fusion happens downstream in `lob-model-trainer` or a dedicated fusion script, not here)

### Why Standalone

After 15 experiments (E1-E9, F1, P0, R1-R8), we established that MBO-only features are contemporaneous -- they cannot predict point-to-point returns (0/67 non-price features IC > 0.05). The E9 cross-validation revealed the first genuine predictive signals from off-exchange data: `trf_signed_imbalance` IC=+0.103 at H=1, `subpenny_intensity` IC=+0.104 at H=60. The CMBP-1 data model (L1 snapshots + trade prints) is fundamentally different from MBO (individual order events), so the two extractors share no core types or processing logic. They share only utility primitives via `hft-statistics`.

---

## 2. Directory Structure (Actual Implementation, Phase 5)

This tree shows the ACTUAL repository layout as of Phase 5. The original spec described a more granular per-feature-group file decomposition; the implementation consolidated feature extraction into `src/features/mod.rs` (extractor) + `src/features/indices.rs` (constants and classification) — see §5 "Pattern Justification" for rationale.

```
basic-quote-processor/
├── .gitignore                      # excludes target/, .DS_Store, .cargo/config.toml
├── .cargo/                         # (gitignored) optional local path patches for dev
├── Cargo.toml                      # Rust crate manifest; deps via git for hft-statistics + dbn
├── Cargo.lock                      # committed for binary crates (3 binaries)
├── LICENSE                         # proprietary
├── README.md                       # Quick start, prerequisites, output format
├── CODEBASE.md                     # Operational technical reference (REQUIRED reading)
│
├── configs/
│   └── nvda_60s.toml               # production 60s-bin config (the only shipped config)
│
├── docs/
│   └── design/                     # 7-document design specification (this file is 02)
│       ├── 01_THEORETICAL_FOUNDATION.md
│       ├── 02_MODULE_ARCHITECTURE.md
│       ├── 03_DATA_FLOW.md
│       ├── 04_FEATURE_SPECIFICATION.md
│       ├── 05_CONFIGURATION_SCHEMA.md
│       ├── 06_INTEGRATION_POINTS.md
│       └── 07_TESTING_STRATEGY.md
│
├── src/
│   ├── lib.rs                      # Crate root: 13 public modules + Phase 1-5 re-exports
│   ├── error.rs                    # ProcessorError enum (8 variants) + Result alias
│   ├── contract.rs                 # EPS, NANO_TO_USD, SCHEMA_VERSION, CONTRACT_VERSION,
│   │                               #   DEFAULT_HORIZONS, FEATURE_NAMES[34], TOTAL_FEATURES
│   ├── config.rs                   # ProcessorConfig, DatasetConfig + 10 sub-config structs
│   ├── pipeline.rs                 # DayPipeline orchestrator: init -> stream -> finalize
│   ├── context.rs                  # DailyContextLoader (EQUS_SUMMARY OHLCV-1D)
│   ├── dates.rs                    # Weekday/Split enums, date parsing, file-name formatting
│   │
│   ├── reader/                     # Data ingestion
│   │   ├── mod.rs                  # Re-exports
│   │   ├── dbn_reader.rs           # DbnReader, RecordIterator, discover_files()
│   │   ├── record.rs               # CmbpRecord (12 fields: ts_event, ts_recv, action, side,
│   │   │                           #   flags, price, size, bid_px, bid_sz, ask_px, ask_sz,
│   │   │                           #   publisher_id) — preserves i64 nanodollars
│   │   └── publisher.rs            # PublisherClass enum + 6 named ID constants
│   │
│   ├── bbo_state/                  # L1 BBO tracking
│   │   ├── mod.rs                  # BboState (14 fields, single i64→f64 conversion point)
│   │   ├── midpoint.rs             # midpoint(), spread_bps(), microprice() pure functions
│   │   └── validation.rs           # is_valid_bbo(), staleness_ns()
│   │
│   ├── trade_classifier/           # Trade signing + retail ID + BVC
│   │   ├── mod.rs                  # TradeClassifier orchestrator
│   │   ├── midpoint_signer.rs      # sign_midpoint() — Barber (2024)
│   │   ├── bjzz.rs                 # fractional_cent(), identify_retail() — Boehmer (2021)
│   │   ├── bvc.rs                  # BvcState — Easley (2012) Eq. 7
│   │   └── types.rs                # TradeDirection, RetailStatus, ClassifiedTrade,
│   │                               #   ClassificationConfig, SigningMethod
│   │
│   ├── sampling/                   # Time-bin sampling
│   │   ├── mod.rs                  # Re-exports
│   │   └── time_bin_sampler.rs     # TimeBinSampler, BinBoundary (grid-aligned, DST-aware)
│   │
│   ├── accumulator/                # Per-bin state (6 sub-accumulators)
│   │   ├── mod.rs                  # BinAccumulator orchestrator + DaySummary
│   │   ├── flow_accumulator.rs     # FlowAccumulator: TRF buy/sell/retail volumes per venue
│   │   ├── count_accumulator.rs    # CountAccumulator: total/trf/lit/retail/subpenny/odd_lot/block
│   │   ├── stats_accumulator.rs    # StatsAccumulator: TWAP spread, BBO snapshots, HHI
│   │   ├── burst_tracker.rs        # BurstTracker: inter-arrival CV, time_since_burst
│   │   └── forward_fill.rs         # ForwardFillState: 3-level empty bin policy
│   │
│   ├── features/                   # Feature extraction (single-file extractor + indices)
│   │   ├── mod.rs                  # FeatureExtractor (stateless reader of accumulator state).
│   │   │                           #   Contains all 10 group extraction methods inline.
│   │   └── indices.rs              # 34 named constants, group ranges, classification arrays
│   │
│   ├── sequence_builder/           # Sliding window
│   │   └── mod.rs                  # FeatureVec = Arc<Vec<f64>>, build_all_from_slice()
│   │
│   ├── labeling/                   # Point-return labels + forward prices
│   │   ├── mod.rs                  # Re-exports
│   │   ├── point_return.rs         # LabelComputer, LabelResult (multi-horizon bps)
│   │   └── forward_prices.rs       # ForwardPriceComputer ([N, max_H+1] f64 USD)
│   │
│   ├── export/                     # NPY + JSON export
│   │   ├── mod.rs                  # DayExport, DayExporter (atomic write + rollback)
│   │   ├── npy_writer.rs           # write_sequences (f32), write_labels (f64), write_forward_prices (f64)
│   │   ├── normalization.rs        # NormalizationComputer (per-feature Welford streaming)
│   │   ├── metadata.rs             # ExportMetadata + builder pattern
│   │   └── manifest.rs             # DatasetManifest (multi-day completion tracking)
│   │
│   └── bin/                        # 3 CLI binaries
│       ├── export_dataset.rs       # multi-day NPY export with train/val/test splits
│       ├── validate_coverage.rs    # TRF + lit volume vs EQUS_SUMMARY coverage check
│       └── profile_data.rs         # per-day diagnostic statistics
│
└── tests/                          # 47 integration tests (data-gated)
    ├── integration_test.rs         # Phase 1: DbnReader + BBO tracking on real data
    ├── classifier_test.rs          # Phase 2: classification + golden vectors
    ├── phase3_test.rs              # Phase 3: full feature extraction pipeline
    ├── phase4_test.rs              # Phase 4: pipeline + export + NPY shape contract
    └── phase5_test.rs              # Phase 5: EQUS context + multi-day pipeline
```

### Files NOT in This Repo (originally proposed in earlier spec drafts)

The following files were proposed in pre-implementation spec drafts but were never created. They are listed here so spec readers don't expect them:

| Spec mentioned | Status | Replacement / Notes |
|----------------|--------|---------------------|
| `src/builder.rs` (PipelineBuilder fluent API) | NOT created | `DayPipeline::new(...)` is the only constructor |
| `src/features/{config,signed_flow,venue_metrics,retail_metrics,vpin,kyle_lambda,bbo_dynamics,trade_size,cross_venue,experimental}.rs` | NOT created | Consolidated into `src/features/mod.rs` (one method per group, all in one file). Decomposition is deferred until `mod.rs` exceeds maintainability thresholds. |
| `src/sampling/{time_bin,volume_bin}.rs` | NOT created | Single file `time_bin_sampler.rs`. `volume_bin.rs` is DEFERRED with the volume-based sampling strategy. |
| `src/sequence_builder/window.rs` | NOT created | Logic lives in `sequence_builder/mod.rs` |
| `src/export/npy_export.rs` | NOT created | Renamed to `export/npy_writer.rs` |
| `tests/{contract,formula,edge,signing,golden}_test.rs` | NOT created | Tests are organized by Phase: `phase3/4/5_test.rs` cover formula+edge+integration; `classifier_test.rs` covers signing+golden vectors; `integration_test.rs` covers Phase 1 reader edge cases |
| `analysis/` (Python scripts: requirements.txt, explore_cmbp1.py, signal_validation.py, coverage_analysis.py, feature_correlation.py) | DEFERRED | Phase 6 (Python analysis scripts) is not yet implemented. Equivalent analysis happens in the parent monorepo (`MBO-LOB-analyzer`, `lob-dataset-analyzer`). |
| `docs/FEATURE_REFERENCE.md`, `docs/CONFIG_REFERENCE.md` | NOT created | Replaced by `docs/design/04_FEATURE_SPECIFICATION.md` and `docs/design/05_CONFIGURATION_SCHEMA.md` |
| `CHANGELOG.md` | NOT created | Will be added at v0.2.0; v0.1.0 is the initial release |
| `CLAUDE.md` | NOT created (in this repo) | LLM coding context lives in the parent monorepo's `.claude/`. The standalone repo relies on `CODEBASE.md` + `docs/design/` for context. |

### File Roles

| File / Directory | Purpose |
|---|---|
| `lib.rs` | Public API surface -- re-exports `DayPipeline`, `ProcessorConfig`, `DatasetConfig`, key types |
| `error.rs` | Centralized error enum using `thiserror`. All modules return `Result<T, ProcessorError>` |
| `contract.rs` | Pipeline constants: `EPS`, `NANO_TO_USD`, `SCHEMA_VERSION`, `CONTRACT_VERSION`, `DEFAULT_HORIZONS`, `FEATURE_NAMES[34]`, `TOTAL_FEATURES`. Authoritative for the standalone repo. |
| `config.rs` | `ProcessorConfig` (per-day, library) and `DatasetConfig` (multi-day, CLI) deserialized from TOML via `serde`. All behavior configurable. |
| `pipeline.rs` | `DayPipeline` orchestrator: per-day processing lifecycle (`init_day` → `stream_file` → `finalize`). Owns all sub-modules. |
| `context.rs` | `DailyContextLoader` reads EQUS_SUMMARY OHLCV-1D for per-day consolidated volume context. |
| `dates.rs` | Weekday enumeration, train/val/test split assignment, date format helpers. |
| `bin/export_dataset.rs` | Production CLI: iterates days, calls `DayPipeline`, writes NPY/JSON via `DayExporter`. |
| `bin/profile_data.rs` | Diagnostic CLI: prints per-day statistics (trade counts, volumes, BBO update rates). |
| `bin/validate_coverage.rs` | Validation CLI: cross-checks TRF/lit volumes against EQUS_SUMMARY consolidated volume. |

---

## 3. Key Design Decisions

### D1: Standalone Repo, Shared Primitives via Path Dependencies

The `basic-quote-processor` is a standalone Rust crate. It shares no core types with the MBO pipeline.

**Dependencies**:
- `hft-statistics` (git = `"https://github.com/nagarx/hft-statistics.git"`) for Welford accumulators, regime classification, DST handling
- No dependency on `mbo-lob-reconstructor` (CMBP-1 has no order lifecycle)
- No dependency on `feature-extractor-MBO-LOB` (different data model entirely)

**Contract integration**:
- Off-exchange features registered in `contracts/pipeline_contract.toml` under `[features.off_exchange]` section (Single Source of Truth)
- Local `contract.rs` mirrors the TOML definitions (verified by `verify_rust_constants.py`)
- Python constants auto-generated via `generate_python_contract.py` for downstream consumers under `OffExchangeFeatureIndex`

**Rationale**: CMBP-1 data (L1 snapshots + trade prints with publisher attribution) has no structural overlap with MBO data (individual order events with order lifecycle). Forcing them into the same processing pipeline would create artificial coupling. Shared statistical primitives (Welford, DST handling, regime classification) are cleanly isolated in `hft-statistics`.

### D2: Accumulator-Based Architecture (NOT Event-Driven Tracker Pattern)

The MBO pipeline uses a tracker pattern: each order event mutates state in one or more trackers (OfiComputer, VolatilityEstimator, etc.), and features are read from tracker state at sample points. This fits MBO because each event carries semantic meaning (add, cancel, modify, fill).

CMBP-1 data is fundamentally different: it consists of L1 BBO snapshots and trade prints. There is no order lifecycle. The natural processing unit is a time bin, not an event.

**Architecture**:
- Each time bin accumulates trades and BBO updates via `BinAccumulator`
- At bin boundary, `FeatureExtractor.extract()` computes all features from accumulated state
- Accumulator resets per bin; pipeline resets per day

**Rationale**: The accumulator pattern matches the data model. Trade prints are aggregated (signed volume, trade counts, retail fraction) over a bin, then features are computed from the aggregates. There is no order-level state to track. See [Section 5](#5-pattern-justification) for the full comparison.

### D3: Point-Return Labels ONLY

No smoothed-average labels are computed or exported. This is a direct consequence of the E8 finding: the model trained on smoothed labels learned the smoothing residual (R-squared=45.0%) rather than the tradeable point component (R-squared=0.02%). DA on point-to-point returns was 48.3% (below random). All 8 backtests were negative.

**Label formula**:
```
point_return(t, H) = (mid_price[t + H] - mid_price[t]) / mid_price[t] * 10000  [bps]
```

Where `mid_price[t]` = Nasdaq BBO midpoint at the end of bin t.

**Forward prices**: `forward_prices[t, h]` = mid_price at bin (t+h), for h = 0..max_H. Exported for any downstream label computation.

**Rationale**: The entire purpose of this module is to find signals that predict tradeable returns. Smoothed labels are structurally orthogonal to execution. Point-to-point returns are the only labels aligned with how trades are actually entered and exited.

### D4: Multi-Horizon by Design

The three top signals from E9 have different optimal horizons:

| Signal | Best Horizon | IC |
|---|---|---|
| `trf_signed_imbalance` | H=1 (1 min at 60s bins) | +0.103 |
| `subpenny_intensity` | H=60 (60 min at 60s bins) | +0.104 |
| `dark_share` | H=10 (10 min at 60s bins) | +0.035 |

The architecture must support multiple horizons natively, not as an afterthought.

**Implementation**:
- Export features at configurable bin sizes (10s, 30s, 60s, 120s)
- Compute returns at multiple horizons simultaneously: H = {1, 2, 3, 5, 10, 20, 30, 60} bins
- Label shape is `[N, H]` where H = number of horizons, allowing models to train on any subset

**Rationale**: Single-horizon design would require re-exporting data for every horizon experiment. Multi-horizon export is cheap (just additional label columns) and eliminates this friction.

### D5: Empty Bin Policy (CRITICAL for Data Integrity)

Bins with zero TRF trades are a real occurrence. At 60s bins during regular hours, approximately 2-5% of bins have no TRF prints. Ratio features (e.g., `odd_lot_ratio = odd_lot_trades / total_trades`) produce NaN when the denominator is zero. This must be handled explicitly.

**Policy by feature type**:

| Feature Type | Empty Bin Behavior | Rationale |
|---|---|---|
| **State features** (subpenny_intensity, odd_lot_ratio, dark_share) | Forward-fill from previous bin | These represent persistent market state; absence of trades does not mean absence of state |
| **Flow features** (trf_signed_imbalance, volumes, mroib) | Set to 0.0 | No flow occurred; 0.0 is the true value, not a fill |

**Safety gates**:
- `bin_valid` = 1.0 if `n_trf_trades >= min_trades_per_bin` (configurable, default 10), 0.0 otherwise
- `bbo_valid` = 1.0 if BBO was updated within the bin, 0.0 otherwise

**Warmup**: Discard first N bins per day (configurable, default 3) before accumulators are stable. This prevents forward-fill from producing arbitrary values at day start.

**NaN guard**: Every division guarded with `EPS` (1e-8). `is_finite()` check before NPY export. Any non-finite value is a hard error.

### D6: Half-Day Auto-Detection

NYSE early-close days (~3/year: July 3, day after Thanksgiving, Christmas Eve) close at 13:00 ET instead of 16:00 ET.

**Approach**: Data-driven detection, not hardcoded calendar.
- If no records arrive for N consecutive bins (configurable, default 10), treat day as complete
- Session progress feature adjusted to reflect actual trading duration (1.0 at detected close, not at 16:00)

**Rationale**: A hardcoded holiday calendar is fragile (exchange schedule changes, unscheduled closures). Data-driven detection is robust to any closure pattern and requires zero maintenance.

### D7: Price Precision Chain (Explicit, Documented)

Every conversion in the price pipeline is explicit and occurs at a defined boundary:

```
Databento wire: i64 nanodollars (FIXED_PRICE_SCALE = 1e-9)
  -> dbn crate decode: i64 (preserved)
  -> CmbpRecord: i64 prices (nanodollars), u32 sizes (shares)
  -> BboState: f64 prices (USD, converted once at update boundary)
  -> Midpoint computation: f64 USD (from BboState)
  -> Feature vectors: f64 (all computations)
  -> NPY export: f32 (downcast at export boundary, with finite check)
```

**Key constraints**:
- Integer nanodollars are preserved through `CmbpRecord` to avoid floating-point drift before conversion
- Conversion to f64 USD happens exactly once, at the `BboState.update()` boundary
- All feature computations use f64 exclusively
- Downcast to f32 happens only at NPY export, with `is_finite()` validation

**Rationale**: Silent precision loss in price handling is a category of bug that compounds into incorrect signals. The explicit chain ensures every conversion is deliberate, tested, and documented.

### D8: EQUS_SUMMARY as Daily Context, Not Feature Source

EQUS_SUMMARY provides daily consolidated OHLCV data for NVDA across all exchanges.

**Usage**:
- Loaded once per day at pipeline initialization
- Provides consolidated volume (ground truth denominator for `true_dark_share = TRF_volume / consolidated_volume`)
- Provides daily OHLCV for regime context (optional)

**Restrictions**:
- NOT used as an intraday feature (daily resolution only -- using it intra-bin would be lookahead)
- NOT required for core feature computation (pipeline functions without it, just lacks `true_dark_share`)

**Rationale**: EQUS_SUMMARY covers all exchanges (XNAS, ARCX, BATS, IEX, MEMX, etc.) and gives the true denominator for dark pool share computation. Without it, `dark_share` is computed relative to XNAS.BASIC volume only, which understates the denominator. However, it is daily data -- injecting it at finer granularity would create lookahead bias.

---

## 4. Module Boundaries

Each directory module has a single responsibility, well-defined inputs and outputs, and explicit reset semantics. Modules communicate through types, not through shared mutable state.

### 4.1 reader/ -- Data Ingestion

**Responsibility**: Read Databento XNAS.BASIC `.dbn.zst` files and emit `CmbpRecord` values.

**Files**:

| File | Purpose |
|---|---|
| `dbn_reader.rs` | Opens `.dbn.zst` files via the `dbn` crate, iterates records, filters by symbol |
| `record.rs` | `CmbpRecord` struct -- our internal representation of a CMBP-1 record. Converts from `dbn::CbboMsg`. Prices stored as i64 nanodollars, sizes as u32 |
| `publisher.rs` | `PublisherId` enum with variants for each venue, plus `is_trf()` / `is_lit()` classification methods |

**Input**: File path to `.dbn.zst` file
**Output**: Iterator of `CmbpRecord`
**State**: Stateless (file handle only)
**Reset**: N/A -- creates a new reader per file

**Publisher ID mapping**:

| ID | Venue | Classification |
|----|-------|---------------|
| 81 | XNAS (Nasdaq) | Lit |
| 82 | FINN (FINRA TRF Carteret) | TRF (off-exchange) |
| 83 | FINC (FINRA TRF Chicago) | TRF (off-exchange) |
| 88 | XBOS (Nasdaq BX) | Lit (minor) |
| 89 | XPSX (Nasdaq PSX) | Lit (minor) |

Whether minor lit venues (XBOS, XPSX) are counted as lit is configurable via `[publishers].include_minor_lit_in_lit`.

### 4.2 bbo_state/ -- L1 Book State Tracking

**Responsibility**: Track the current Nasdaq best bid and offer from quote updates and trade records.

**Files**:

| File | Purpose |
|---|---|
| `mod.rs` | `BboState` struct: bid/ask price (f64 USD), bid/ask size (u32), midpoint (f64), spread (f64), last update timestamp (u64 ns). `update_from_record()` method. **Note**: Rust dbn `ConsolidatedBidAskPair` has `bid_pb`/`ask_pb` (publisher IDs) not `bid_ct`/`ask_ct` (order counts); no features depend on counts |
| `midpoint.rs` | `midpoint()`, `spread_bps()`, `microprice()` computation. Microprice = weighted midpoint using bid/ask sizes |
| `validation.rs` | `is_valid()` check: spread > 0, both prices finite, both prices > 0. `staleness_ns()` method for `bbo_valid` safety gate |

**Input**: `CmbpRecord` (both trade and quote records carry BBO fields)
**Output**: Current `BboState` (queried by trade classifier and feature extractor)
**State**: Current BBO snapshot (overwritten on each update)
**Reset**: Reset to invalid state (zero prices) at day boundary

**CRITICAL ordering constraint**: For trade records (`action == 'T'`), BBO must be updated BEFORE the trade is classified. The CMBP-1 trade record carries the contemporaneous BBO; using a stale BBO degrades Barber (2024) midpoint signing accuracy. The pipeline orchestrator enforces this ordering.

### 4.3 trade_classifier/ -- Trade Signing and Retail Identification

**Responsibility**: Classify each TRF trade by direction (buy/sell/unsigned) and retail status (retail/institutional/unknown).

**Files**:

| File | Purpose |
|---|---|
| `mod.rs` | `TradeClassifier` orchestrator. Calls midpoint signer, then BJZZ, produces `ClassifiedTrade` |
| `midpoint_signer.rs` | Barber (2024) midpoint signing. Trade above midpoint + exclusion band = buy, below = sell, within band = unsigned. Configurable `exclusion_band` (default 0.10 = 10% of spread) |
| `bjzz.rs` | BJZZ retail identification (Boehmer et al. 2021). Uses subpenny fractional cent to classify: `frac_cent in (0.001, 0.40)` = retail sell, `frac_cent in (0.60, 0.999)` = retail buy. All comparisons use open intervals (strict `>` and `<`). Thresholds configurable |
| `bvc.rs` | Bulk Volume Classification (Easley et al. 2012, Eq. 7). Probabilistic volume splitting: `buy_vol = size * Phi((price_i - price_{i-1}) / sigma)`. Used by VPIN computation and bvc_imbalance feature, not for per-trade direction assignment |
| `types.rs` | `TradeDirection` enum {Buy, Sell, Unsigned}, `RetailStatus` enum {Retail, Institutional, Unknown}, `ClassifiedTrade` struct |

**Input**: `CmbpRecord` (trade records only) + current `BboState`
**Output**: `ClassifiedTrade` with direction, retail status, price, size, publisher_id
**State**: Previous trade price for tick-test fallback (optional). BVC maintains rolling sigma window.
**Reset**: Reset per day (clear previous-price state)

**Classification pipeline per trade**:
1. Check publisher_id -- only TRF trades (FINN=82, FINC=83) go through midpoint signing
2. Midpoint sign using Barber (2024) exclusion band relative to current BBO spread
3. BJZZ retail classification using subpenny fractional price
4. If both signing and BJZZ agree on direction, high confidence; if they disagree, direction from midpoint signer takes precedence, retail status from BJZZ takes precedence

### 4.4 features/ -- Feature Computation

**Responsibility**: Compute all 34 off-exchange features from accumulated bin data.

**Files** (2):

| File | Purpose |
|---|---|
| `mod.rs` | `FeatureExtractor` orchestrator. Delegates to inline helper functions per feature group (NOT separate sub-files). Consumes `BinAccumulator` + `BboState` + `DailyContext`, produces the 34-element `Vec<f64>` in one pass. Empty-bin forward-fill logic lives here. |
| `indices.rs` | Named constants for all 34 feature indices (`TRF_SIGNED_IMBALANCE = 0`, `SUBPENNY_INTENSITY = 8`, etc.) + per-group `Range` constants (`SIGNED_FLOW_RANGE`, `VPIN_RANGE`, etc.). Consumed by `FeatureConfig::enabled_feature_count()` and the contract module. |

Feature groups are organized as contiguous index ranges (§4.4 below) rather than separate files. `FeatureConfig` (`src/config.rs`) toggles which groups are computed; disabled groups produce zeros at their indices but still occupy feature-vector space.

**Input**: `BinAccumulator` (accumulated state for current bin) + `BboState` + `DailyContext` (consolidated volume from EQUS_SUMMARY)
**Output**: `Vec<f64>` (34 elements) — populated in-place via a mutable `&mut Vec<f64>` buffer to avoid per-bin allocation. Wrapped in `Arc<Vec<f64>>` by `DayPipeline::emit_bin` for zero-copy sharing with the sequence builder and label computer.
**State**: Stateless per extraction call. All state lives in the accumulator.
**Reset**: N/A -- stateless

**Feature index assignment** (34 features total):

| Index Range | Group | Count | Features |
|---|---|---|---|
| 0-3 | signed_flow | 4 | trf_signed_imbalance, mroib, inv_inst_direction, bvc_imbalance |
| 4-7 | venue_metrics | 4 | dark_share, trf_volume, lit_volume, total_volume |
| 8-11 | retail_metrics | 4 | subpenny_intensity, odd_lot_ratio, retail_trade_rate, retail_volume_fraction |
| 12-17 | bbo_dynamics | 6 | spread_bps, bid_pressure, ask_pressure, bbo_update_rate, quote_imbalance, spread_change_rate |
| 18-19 | vpin | 2 | trf_vpin, lit_vpin |
| 20-23 | trade_size | 4 | mean_trade_size, block_trade_ratio, trade_count, size_concentration |
| 24-26 | cross_venue | 3 | trf_burst_intensity, time_since_burst, trf_lit_volume_ratio |
| 27-28 | activity | 2 | bin_trade_count, bin_trf_trade_count |
| 29-30 | safety_gates | 2 | bin_valid, bbo_valid |
| 31-33 | context | 3 | session_progress, time_bucket, schema_version |

These indices are independent of the MBO pipeline's 0-147 feature space. The downstream fusion layer handles alignment.

**Note on bbo_dynamics overlap**: The MBO pipeline computes `spread_bps` (index 42) and related features from the full 20-level order book. This module computes BBO dynamics from L1 quotes only. The two are NOT redundant -- they capture different information (full depth vs. top-of-book dynamics from a consolidated feed). The fusion layer must document whether both are included or one is dropped.

### 4.5 sampling/ -- Time-Bin Sampling

**Responsibility**: Determine bin boundaries and signal when a bin is complete.

**Files** (2):

| File | Purpose |
|---|---|
| `mod.rs` | Module entry. Re-exports `TimeBinSampler` + `BinBoundary`. No trait abstraction today — `DayPipeline` hardwires `TimeBinSampler` as the sole implementation (see "Validated Design Items" in `CODEBASE.md` for the deferred `Sampler` trait). |
| `time_bin_sampler.rs` | `TimeBinSampler`: fixed-interval time bins grid-aligned to market open (09:30 ET). Configurable bin size (5s, 10s, 15s, 30s, 60s, 120s, 300s, 600s). Uses `hft_statistics::time::regime::{utc_offset_for_date, day_epoch_ns}` for exact EST/EDT handling. |

Volume-based sampling (VPIN computation) lives inside `accumulator/` (bucket-volume tracking) rather than as a separate sampler — VPIN runs in parallel with the time-bin sampling, not as an alternative to it.

**Input**: Timestamp (u64 nanoseconds UTC) for time bins; cumulative volume for volume bins
**Output**: Boolean indicating bin boundary crossed; bin identifier
**State**: Current bin start/end timestamps, cumulative volume for volume bins
**Reset**: Reset per day

**Grid alignment**: Time bins are aligned to market open, not to midnight. At 60s bins with 09:30 open, bin 0 covers [09:30:00, 09:31:00), bin 1 covers [09:31:00, 09:32:00), etc. This ensures consistency across days regardless of the first record's timestamp.

### 4.6 accumulator/ -- Per-Bin Feature Accumulation

**Responsibility**: Aggregate classified trades and BBO updates within a single time bin.

**Files** (6):

| File | Purpose |
|---|---|
| `mod.rs` | `BinAccumulator` orchestrator. Dispatches to sub-accumulators. Provides `accumulate(trade)`, `accumulate_bbo_update(bbo, ts)`, `prepare_for_extraction(bin_end_ts)`, `reset_bin()` methods. Carries the VPIN bucket-volume tracker (volume-bar BVC) alongside the time-bin logic. |
| `flow_accumulator.rs` | Accumulates signed volumes: buy_volume, sell_volume, unsigned_volume, retail_buy_volume, retail_sell_volume, trf_volume, lit_volume. All as f64 shares. |
| `count_accumulator.rs` | Accumulates trade counts: total_trades, trf_trades, lit_trades, retail_trades, subpenny_trades, odd_lot_trades, block_trades. All as u64. |
| `stats_accumulator.rs` | Running statistics per bin using `Welford` from `hft-statistics`: trade-size mean/variance, BBO-update TWAP (time-weighted average spread), update count, start/last-spread snapshots. |
| `burst_tracker.rs` | Tracks TRF "burst" events — consecutive TRF prints within a short window, used by `trf_burst_intensity` and `time_since_burst` features (cross-venue group). |
| `forward_fill.rs` | Per-bin forward-fill state (Level 2 empty-bin policy): when a bin has no TRF trades, the feature extractor carries forward the previous bin's flow-derived values so downstream models see continuous series rather than spurious zeros. |

**Input**: `ClassifiedTrade` + current `BboState`
**Output**: Accumulated state (queried by `FeatureExtractor` at bin boundary)
**State**: All counters and running statistics for the current bin
**Reset**: Full reset at each bin boundary (all counters to zero, Welford accumulators reset). The `reset()` method is called by the pipeline orchestrator after feature extraction.

**Size estimate**: ~512 bytes per `BinAccumulator` instance. Only one instance exists at a time (current bin).

### 4.7 sequence_builder/ -- Sequence Construction for ML

**Responsibility**: Define the `FeatureVec` type alias and provide sliding-window sequence construction helpers.

**Files** (1):

| File | Purpose |
|---|---|
| `mod.rs` | Defines `pub type FeatureVec = Arc<Vec<f64>>` (zero-copy 34-element feature vector shared between the producer and all consumers). Sliding-window sequence assembly lives inline in `pipeline.rs::finalize()` (`feature_bins[seq_start..=ending_idx]`) rather than as a standalone type — the window is consumed once per day, not streamed, so a reusable `SequenceBuilder` struct would be premature abstraction. |

**Input**: `Arc<Vec<f64>>` feature vectors, one per bin
**Output**: Sequences of shape `[T, F]` where T = `window_size` (configurable, default 20) and F = number of enabled features
**State**: Ring buffer of `window_size` feature vectors
**Reset**: Clear buffer at day boundary. First `window_size - 1` bins of each day produce no sequences (warmup).

**Stride**: Configurable (default 1). Stride=1 means every bin produces a sequence (after warmup). Stride=5 means every 5th bin produces a sequence.

### 4.8 labeling/ -- Return Labels

**Responsibility**: Compute point-to-point return labels at multiple horizons.

**Files**:

| File | Purpose |
|---|---|
| `mod.rs` | `LabelComputer`: tracks mid-prices per bin, computes labels after all bins are processed (requires lookahead into future bins) |
| `point_return.rs` | `point_return(t, H) = (mid[t+H] - mid[t]) / mid[t] * 10000` in basis points. Returns `NaN` for bins where `t+H` exceeds day boundary |
| `forward_prices.rs` | Exports forward mid-price trajectories: `forward_prices[t, h] = mid_price` at bin `(t+h)` for `h = 0..max_H`. Shape `[N, max_H+1]` |

**Input**: Vector of mid-prices (one per bin, collected during streaming phase)
**Output**: Label matrix `[N, H]` float64 (point returns in bps) + forward prices `[N, max_H+1]` float64 (USD)
**State**: Stores all mid-prices for the day during streaming. Labels computed in finalization phase.
**Reset**: Clear mid-price buffer at day boundary.

**End-of-day truncation**: Sequences within `max_H` bins of day end cannot have complete labels for all horizons. These sequences have `NaN` for the affected horizon columns. The downstream consumer must handle this (either drop rows with NaN or train only on non-NaN horizons).

### 4.9 export/ -- NPY/JSON Export

**Responsibility**: Write sequences, labels, forward prices, metadata, manifest, and normalization stats to disk in the pipeline's standard export format.

**Files** (5):

| File | Purpose |
|---|---|
| `mod.rs` | `DayExporter` orchestrator + `DayExport` struct. Wires the per-day assembly (sequences + labels + forward_prices + metadata + normalization) and delegates file writes to the sub-modules. |
| `metadata.rs` | `ExportMetadata` struct + `ExportMetadataBuilder`. Writes `{day}_metadata.json` with all spec-required fields including `provenance.config_hash`, `provenance.source_file`, `forward_prices` (6-field block), `normalization` (honest strategy), `feature_groups_enabled` (active `FeatureConfig` snapshot), `classification_config`. |
| `normalization.rs` | `NormalizationComputer` using `Welford` (from `hft-statistics`) for per-feature mean/std across all post-warmup bins. Excludes categorical indices (29, 30, 32, 33). Writes `{day}_normalization.json` with finalized statistics. |
| `npy_writer.rs` | Writes NPY files via `ndarray-npy`. Handles f32 downcast for sequences (with `is_finite()` guard) and f64 for labels + forward prices. Emits three files per day: `{day}_sequences.npy`, `{day}_labels.npy`, `{day}_forward_prices.npy`. |
| `manifest.rs` | `DatasetManifest` — the top-level `dataset_manifest.json` written to the export root. Tracks all processed days across train/val/test splits, aggregate statistics, and per-day status (success/failure with error message). |

**Input**: Sequences `[N, T, F]`, labels `[N, H]`, forward prices `[N, max_H+1]`, metadata struct
**Output**: Files on disk in the export directory
**State**: Stateless (writes are atomic per day)
**Reset**: N/A

**Export directory structure**:
```
data/exports/{experiment_name}/
    train/  {day}_sequences.npy, {day}_labels.npy, {day}_forward_prices.npy,
            {day}_metadata.json, {day}_normalization.json
    val/    ...
    test/   ...
    dataset_manifest.json
```

**Export file contract**:

| File | Shape | Dtype | Unit | Description |
|---|---|---|---|---|
| `{day}_sequences.npy` | `[N, T, F]` | float32 | normalized | Feature sequences |
| `{day}_labels.npy` | `[N, H]` | float64 | basis points | Point returns per horizon |
| `{day}_forward_prices.npy` | `[N, max_H+1]` | float64 | USD | Mid-price trajectory |
| `{day}_metadata.json` | -- | JSON | -- | Schema, provenance, feature count |
| `{day}_normalization.json` | -- | JSON | -- | Per-day mean/std per feature |
| `dataset_manifest.json` | -- | JSON | -- | Split info, config, date lists |

---

## 5. Pattern Justification

### Why Accumulator, Not Tracker

The MBO feature extractor (`feature-extractor-MBO-LOB`) uses a tracker pattern:

```
MBO Event (Add/Cancel/Modify/Fill)
    -> OfiComputer.on_event()    (mutates internal state)
    -> VolatilityEstimator.on_event()
    -> ...
    -> sample_point: read state from all trackers -> feature vector
```

This works for MBO because:
1. Each event has rich semantic meaning (order add vs. cancel vs. fill)
2. Trackers maintain complex state (order maps, running OFI, queue positions)
3. Features are instantaneous snapshots of tracker state at sample points

CMBP-1 data is fundamentally different:

```
CMBP-1 Record (BBO update OR trade print)
    -> BboState.update()         (overwrite L1 state)
    -> TradeClassifier.classify() (one-shot classification)
    -> BinAccumulator.accumulate() (aggregate into bin)
    -> bin_boundary: compute features from aggregates -> feature vector
```

This favors the accumulator pattern because:
1. Trade prints have limited per-event semantics (price, size, publisher, subpenny -- no order lifecycle)
2. Features are aggregates over time windows (signed volume, trade counts, ratios), not instantaneous snapshots
3. The natural unit of computation is the bin, not the event
4. Accumulator state is simple (counters, running sums) and resets cleanly per bin

### Comparison Table

| Aspect | Tracker (MBO) | Accumulator (CMBP-1) |
|---|---|---|
| Event semantics | Rich (5 action types, order lifecycle) | Simple (trade print + BBO update) |
| State complexity | High (order maps, queues, OFI history) | Low (counters, running sums) |
| Feature timing | Instantaneous snapshot at sample point | Aggregate over bin window |
| Reset granularity | Per-day (trackers carry inter-event state) | Per-bin (accumulator resets each bin) |
| Memory footprint | ~10KB+ per tracker | ~512B per accumulator |
| Natural sample unit | Event count or time trigger | Time bin boundary |

---

## 6. Key Types

| Type | Approx. Size | Description | Lifetime |
|---|---|---|---|
| `CmbpRecord` | ~72B | Internal representation of CMBP-1 record. i64 nanodollar prices, u32 sizes, u16 publisher_id, u64 timestamps, u8 flags. Converted from `dbn::CbboMsg` | Per-record (transient) |
| `BboState` | ~56B | Current Nasdaq BBO: bid/ask price (f64 USD), bid/ask size (u32), midpoint (f64), spread (f64), microprice (f64), last_update_ts (u64), is_valid (bool). **Note**: Rust dbn `ConsolidatedBidAskPair` has `bid_pb`/`ask_pb` (publisher IDs) not `bid_ct`/`ask_ct` (order counts); no features depend on counts | Per-day (mutated on each update) |
| `ClassifiedTrade` | ~40B | Trade with direction (Buy/Sell/Unsigned), retail status (Retail/Institutional/Unknown), price (f64 USD), size (u32), publisher_id (u16), ts_recv (u64). **Note**: midpoint_at_trade removed — accumulator reads BboState directly | Per-record (transient) |
| `BinAccumulator` | ~512B | Per-bin accumulation state: volumes by direction/venue, trade counts by type, Welford running stats for size/spread | Per-bin (reset at boundary) |
| `FeatureVec` | `Arc<Vec<f64>>` | Feature vector for one time bin. Arc-wrapped for zero-copy sharing between sequence builder and label computer | Per-bin, shared across consumers |
| `DailyContext` | ~32B | Consolidated volume (f64), daily OHLCV from EQUS_SUMMARY | Per-day (loaded at init) |
| `PipelineConfig` | variable | Full configuration deserialized from TOML. Owns all sub-configs | Pipeline lifetime |

---

## 7. Dependencies

### Cargo.toml

```toml
[dependencies]
# Shared statistics primitives
hft-statistics = { git = "https://github.com/nagarx/hft-statistics.git" }

# Core
thiserror = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"

# Data I/O
dbn = { git = "https://github.com/databento/dbn.git", tag = "v0.20.0" }  # Databento DBN format (same version as other crates)
ndarray = "0.15"
ndarray-npy = "0.8"

# Time handling
chrono = { version = "0.4", features = ["serde"] }
chrono-tz = "0.9"

# Logging
log = "0.4"
env_logger = "0.11"

[features]
parallel = ["rayon"]              # Optional parallel processing

[dev-dependencies]
tempfile = "3"
approx = "0.5"
```

### Dependency Rationale

| Dependency | Purpose | Why This Version |
|---|---|---|
| `hft-statistics` | Welford accumulators, StreamingDistribution, VpinComputer, `time::regime::{time_regime, utc_offset_for_date, day_epoch_ns}`, AcfComputer | Shared crate; git dependency. **Note**: `KyleLambda` not yet implemented — needed before Phase 3 |
| `dbn` | Read `.dbn.zst` files (Databento's native format). Wire type: `CbboMsg` (consolidated BBO) for CMBP-1 schema | Same version (v0.20.0 git tag) as `MBO-LOB-reconstructor` and `feature-extractor-MBO-LOB` for wire-format consistency |
| `ndarray` + `ndarray-npy` | Write NPY files for Python consumption | Same versions as MBO extractor for format compatibility |
| `chrono` + `chrono-tz` | Timezone-aware time handling (EST/EDT market hours) | Standard Rust time library |
| `thiserror` | Derive-based error types | Idiomatic Rust error handling |
| `serde` + `serde_json` + `toml` | Config deserialization and metadata serialization | Standard serialization ecosystem |
| `log` + `env_logger` | Structured logging | Lightweight, no runtime overhead when disabled |
| `rayon` (optional) | Parallel day processing | Behind `parallel` feature flag; not used in hot path |
| `tempfile` (dev) | Temporary directories for integration tests | Test isolation |
| `approx` (dev) | Floating-point comparison in tests | `assert_relative_eq!` for numerical validation |

### Primitives Reused from hft-statistics

| Primitive | Usage in basic-quote-processor |
|---|---|
| `WelfordAccumulator` | Running mean/variance for per-bin trade size stats, per-day normalization |
| `StreamingDistribution` | Quantiles and skewness for trade size analysis |
| `RegimeClassifier` | 7-regime time classification for `time_bucket` feature |
| `time::regime::utc_offset_for_date()` | Returns -4 (EDT) or -5 (EST) for a given date. Exact DST transitions (2nd Sunday March, 1st Sunday November) |
| `time::regime::day_epoch_ns()` | UTC nanosecond timestamp of midnight ET for a given date |
| `time::regime::time_regime()` | 7-regime intraday classifier (takes UTC ns + offset, returns 0-6) |
| `AcfComputer` | Autocorrelation for signal diagnostics (Python analysis scripts) |
| `KyleLambda` | Rolling Kyle's lambda estimation for price impact feature |

### What Is NOT Depended On

| Crate | Why Not |
|---|---|
| `mbo-lob-reconstructor` | CMBP-1 has no order lifecycle; no `LobState`, no `MboMessage` |
| `feature-extractor-MBO-LOB` | Different data model, different feature space, different sampling strategy |
| `lob-models` | This module produces data; model architectures are downstream |
| `lob-model-trainer` | Downstream consumer of this module's exports |

---

## Appendix: Contract Registration

Off-exchange features are registered in `contracts/pipeline_contract.toml` under a new `[features.off_exchange]` section:

```toml
[features.off_exchange]
schema_version = "1.0"
total_count = 34
signed_flow = { start = 0, count = 4 }
venue_metrics = { start = 4, count = 4 }
retail_metrics = { start = 8, count = 4 }
bbo_dynamics = { start = 12, count = 6 }
vpin = { start = 18, count = 2 }
trade_size = { start = 20, count = 4 }
cross_venue = { start = 24, count = 3 }
activity = { start = 27, count = 2 }
safety_gates = { start = 29, count = 2 }
context = { start = 31, count = 3 }
```

The local `src/contract.rs` mirrors these values as Rust constants. `verify_rust_constants.py` is extended to validate `basic-quote-processor` constants against the TOML. Python constants are auto-generated via `generate_python_contract.py` under `OffExchangeFeatureIndex` for downstream consumers.

This preserves the Single Source of Truth principle while keeping the off-exchange index space independent from MBO features (0-147). The fusion layer (downstream) is responsible for mapping both index spaces into a combined feature tensor.
