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

## 2. Directory Structure

```
basic-quote-processor/
├── Cargo.toml                    # Rust crate, depends on hft-statistics
├── .cargo/config.toml            # Local path patches
├── CODEBASE.md                   # Technical reference (REQUIRED)
├── CLAUDE.md                     # LLM coding guide
├── README.md                     # Quick start
├── CHANGELOG.md                  # Version history
│
├── src/
│   ├── lib.rs                    # Public API re-exports
│   ├── error.rs                  # Error types (thiserror)
│   ├── contract.rs               # Schema version, feature indices, EPS constants
│   ├── config.rs                 # PipelineConfig (serde::Deserialize from TOML)
│   ├── pipeline.rs               # Main Pipeline orchestrator
│   ├── builder.rs                # PipelineBuilder fluent API
│   │
│   ├── reader/                   # Data ingestion (directory module)
│   │   ├── mod.rs                # Re-exports
│   │   ├── dbn_reader.rs         # Databento .dbn.zst file reader
│   │   ├── record.rs             # CmbpRecord type (our internal repr of CMBP-1)
│   │   └── publisher.rs          # PublisherId enum (XNAS=81, FINN=82, FINC=83, XBOS=88, XPSX=89)
│   │
│   ├── bbo_state/                # L1 book state tracking (directory module)
│   │   ├── mod.rs                # BboState type + update logic
│   │   ├── midpoint.rs           # Midpoint, spread, microprice computation
│   │   └── validation.rs         # BBO validity checks (spread > 0, finite prices)
│   │
│   ├── trade_classifier/         # Trade signing + retail identification (directory module)
│   │   ├── mod.rs                # TradeClassifier orchestrator
│   │   ├── midpoint_signer.rs    # Barber (2024) midpoint signing with configurable exclusion band
│   │   ├── bjzz.rs               # BJZZ retail identification (Boehmer 2021)
│   │   ├── bvc.rs                # Bulk Volume Classification (Easley 2012)
│   │   └── types.rs              # ClassifiedTrade, TradeDirection, RetailStatus enums
│   │
│   ├── features/                 # Feature computation (directory module)
│   │   ├── mod.rs                # FeatureExtractor orchestrator
│   │   ├── config.rs             # FeatureConfig (which groups enabled)
│   │   ├── signed_flow.rs        # trf_signed_imbalance, mroib, inv_inst_direction
│   │   ├── venue_metrics.rs      # dark_share, trf_volume, lit_volume, total_volume
│   │   ├── retail_metrics.rs     # subpenny_intensity, odd_lot_ratio, retail_trade_rate
│   │   ├── vpin.rs               # Volume-synchronized VPIN (Easley 2012, volume-bar BVC)
│   │   ├── kyle_lambda.rs        # FUTURE: Rolling Kyle's lambda (NOT in 34-feature index; requires KyleLambda in hft-statistics, not yet implemented)
│   │   ├── bbo_dynamics.rs       # L1 spread dynamics, bid/ask pressure from quote updates
│   │   ├── trade_size.rs         # Block detection, trade size distribution, round-lot ratio
│   │   ├── cross_venue.rs        # TRF burst detection, quote-trade timing
│   │   └── experimental.rs       # Future experimental features (behind feature flag)
│   │
│   ├── sampling/                 # Time-bin sampling (directory module)
│   │   ├── mod.rs                # Sampler trait + TimeBinSampler
│   │   ├── time_bin.rs           # Fixed-interval time bins (configurable: 10s, 30s, 60s, etc.)
│   │   └── volume_bin.rs         # Dollar-volume bins for VPIN (Easley 2021)
│   │
│   ├── accumulator/              # Per-bin feature accumulation (directory module)
│   │   ├── mod.rs                # BinAccumulator orchestrator
│   │   ├── flow_accumulator.rs   # Accumulates signed volume per bin
│   │   ├── count_accumulator.rs  # Accumulates trade counts per bin
│   │   └── stats_accumulator.rs  # Running statistics per bin (Welford via hft-statistics)
│   │
│   ├── sequence_builder/         # Sequence construction for ML (directory module)
│   │   ├── mod.rs                # SequenceBuilder
│   │   └── window.rs             # Sliding window over feature bins
│   │
│   ├── labeling/                 # Return labels (directory module)
│   │   ├── mod.rs                # LabelComputer
│   │   ├── point_return.rs       # Point-to-point return at H bins ahead (bps)
│   │   └── forward_prices.rs     # Forward mid-price trajectory export
│   │
│   └── export/                   # NPY/JSON export (directory module)
│       ├── mod.rs                # Exporter orchestrator
│       ├── npy_export.rs         # NPY file writer
│       ├── metadata.rs           # Metadata JSON (schema_version, provenance, etc.)
│       └── normalization.rs      # Per-day normalization stats export
│
├── bin/
│   ├── export_dataset.rs         # CLI: process days -> NPY exports
│   ├── profile_data.rs           # CLI: compute per-day statistics (trade counts, volumes, etc.)
│   └── validate_coverage.rs      # CLI: cross-check against EQUS_SUMMARY
│
├── configs/
│   ├── nvda_60s.toml             # Default: 60s bins, all features, H=1..60
│   ├── nvda_10s.toml             # Fine-grained: 10s bins
│   └── nvda_vpin.toml            # VPIN-focused: volume bins
│
├── tests/
│   ├── contract_test.rs          # Feature indices match contract
│   ├── formula_test.rs           # Math verification with hand-calculated values
│   ├── edge_test.rs              # 0, NaN, Inf, near-zero, spread<=0
│   ├── signing_test.rs           # Trade classification accuracy
│   ├── integration_test.rs       # End-to-end: raw .dbn.zst -> NPY
│   └── golden_test.rs            # Deterministic output for fixed input
│
├── analysis/                     # Python analysis scripts
│   ├── requirements.txt          # databento, numpy, scipy, pandas
│   ├── explore_cmbp1.py          # Data exploration and statistics
│   ├── signal_validation.py      # IC computation and horizon sweep
│   ├── coverage_analysis.py      # EQUS_SUMMARY cross-check
│   └── feature_correlation.py    # Feature correlation matrix
│
└── docs/
    ├── FEATURE_REFERENCE.md      # All features with formulas, indices, units
    └── CONFIG_REFERENCE.md       # All TOML config parameters
```

### File Roles

| File / Directory | Purpose |
|---|---|
| `lib.rs` | Public API surface -- re-exports `Pipeline`, `PipelineBuilder`, `PipelineConfig`, key types |
| `error.rs` | Centralized error enum using `thiserror`. All modules return `Result<T, Error>` |
| `contract.rs` | Schema version, feature index constants, `EPS` (1e-8). Mirrors `pipeline_contract.toml` values. Validated by `verify_rust_constants.py` |
| `config.rs` | `PipelineConfig` struct deserialized from TOML via `serde`. All behavior configurable |
| `pipeline.rs` | Orchestrates the per-day processing lifecycle (init -> stream -> finalize). Owns all sub-modules |
| `builder.rs` | `PipelineBuilder` fluent API for programmatic construction (tests, CLI) |
| `bin/export_dataset.rs` | Production CLI entry point. Iterates days, calls `Pipeline`, writes NPY/JSON |
| `bin/profile_data.rs` | Diagnostic CLI. Prints per-day statistics (trade counts, volumes, BBO update rates) |
| `bin/validate_coverage.rs` | Validation CLI. Cross-checks TRF/lit volumes against EQUS_SUMMARY consolidated volume |

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
- If no records arrive for N consecutive bins (configurable, default 5), treat day as complete
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

**Responsibility**: Compute all off-exchange features from accumulated bin data.

**Files**:

| File | Purpose |
|---|---|
| `mod.rs` | `FeatureExtractor` orchestrator. Calls each enabled feature group, assembles `Vec<f64>` |
| `config.rs` | `FeatureConfig` -- which groups are enabled. Derived from `[features]` section of TOML |
| `signed_flow.rs` | `trf_signed_imbalance` (net signed volume / total volume), `mroib` (retail OIB), `inv_inst_direction` (inverse institutional), `bvc_imbalance` |
| `venue_metrics.rs` | `dark_share` (TRF volume / total volume), `trf_volume`, `lit_volume`, `total_volume` |
| `retail_metrics.rs` | `subpenny_intensity` (subpenny trades / total trades), `odd_lot_ratio`, `retail_trade_rate`, `retail_volume_fraction` |
| `vpin.rs` | Volume-synchronized VPIN (Easley et al. 2012). Uses volume bars (not time bars) with BVC for trade signing. Separate VPIN for TRF and lit venues |
| `kyle_lambda.rs` | Rolling Kyle's lambda estimate. Delegates to `KyleLambda` from `hft-statistics`. Measures price impact per unit volume |
| `bbo_dynamics.rs` | L1 spread dynamics from quote updates: `spread_bps`, `bid_pressure` (bid size change rate), `ask_pressure`, `bbo_update_rate`, `quote_imbalance` (bid_size - ask_size normalized), `spread_change_rate` |
| `trade_size.rs` | `mean_trade_size`, `block_trade_ratio` (trades > threshold / total), `trade_count`, `size_concentration` (Herfindahl on size buckets) |
| `cross_venue.rs` | `trf_burst_intensity` (clustered TRF prints within short window), `time_since_burst`, `trf_lit_volume_ratio` |
| `experimental.rs` | Placeholder for future features. Behind `#[cfg(feature = "experimental")]` gate |

**Input**: `BinAccumulator` (accumulated state for current bin) + `DailyContext` (consolidated volume from EQUS_SUMMARY)
**Output**: `Arc<Vec<f64>>` -- feature vector for one time bin. Zero-copy shared between sequence builder and label computer.
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

**Files**:

| File | Purpose |
|---|---|
| `mod.rs` | `Sampler` trait with `is_bin_complete(timestamp) -> bool` and `current_bin_id() -> u64`. `TimeBinSampler` as primary implementation |
| `time_bin.rs` | `TimeBinSampler`: fixed-interval time bins grid-aligned to market open (09:30 ET). Configurable bin size (10s, 30s, 60s, 120s). Uses `hft_statistics::time::regime::{utc_offset_for_date, day_epoch_ns}` for EST/EDT handling |
| `volume_bin.rs` | `VolumeBinSampler`: dollar-volume bins for VPIN computation (Easley 2012). Bucket volume = daily volume * `bucket_volume_fraction`. Separate from the main time-bin sampling |

**Input**: Timestamp (u64 nanoseconds UTC) for time bins; cumulative volume for volume bins
**Output**: Boolean indicating bin boundary crossed; bin identifier
**State**: Current bin start/end timestamps, cumulative volume for volume bins
**Reset**: Reset per day

**Grid alignment**: Time bins are aligned to market open, not to midnight. At 60s bins with 09:30 open, bin 0 covers [09:30:00, 09:31:00), bin 1 covers [09:31:00, 09:32:00), etc. This ensures consistency across days regardless of the first record's timestamp.

### 4.6 accumulator/ -- Per-Bin Feature Accumulation

**Responsibility**: Aggregate classified trades and BBO updates within a single time bin.

**Files**:

| File | Purpose |
|---|---|
| `mod.rs` | `BinAccumulator` orchestrator. Dispatches to sub-accumulators. Provides `accumulate(trade, bbo)` and `reset()` methods |
| `flow_accumulator.rs` | Accumulates signed volumes: buy_volume, sell_volume, unsigned_volume, retail_buy_volume, retail_sell_volume, trf_volume, lit_volume. All as f64 shares |
| `count_accumulator.rs` | Accumulates trade counts: total_trades, trf_trades, lit_trades, retail_trades, subpenny_trades, odd_lot_trades, block_trades. All as u64 |
| `stats_accumulator.rs` | Running statistics per bin using `WelfordAccumulator` from `hft-statistics`: trade size mean/variance, spread mean/variance, BBO update count |

**Input**: `ClassifiedTrade` + current `BboState`
**Output**: Accumulated state (queried by `FeatureExtractor` at bin boundary)
**State**: All counters and running statistics for the current bin
**Reset**: Full reset at each bin boundary (all counters to zero, Welford accumulators reset). The `reset()` method is called by the pipeline orchestrator after feature extraction.

**Size estimate**: ~512 bytes per `BinAccumulator` instance. Only one instance exists at a time (current bin).

### 4.7 sequence_builder/ -- Sequence Construction for ML

**Responsibility**: Construct fixed-length sequences from a sliding window of feature vectors.

**Files**:

| File | Purpose |
|---|---|
| `mod.rs` | `SequenceBuilder`: maintains a ring buffer of `window_size` feature vectors. `push(feature_vec)` adds a new vector; `try_build_sequence()` returns `Some([T, F])` if the buffer is full |
| `window.rs` | Sliding window implementation with configurable stride. Ring buffer of `Arc<Vec<f64>>` (zero-copy from feature extractor) |

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

**Responsibility**: Write sequences, labels, and metadata to disk in the pipeline's standard export format.

**Files**:

| File | Purpose |
|---|---|
| `mod.rs` | `Exporter` orchestrator. Manages output directory structure, calls sub-exporters |
| `npy_export.rs` | Writes NPY files using `ndarray-npy`. Handles f32 downcast for sequences (with `is_finite()` guard) and f64 for labels/forward prices |
| `metadata.rs` | Writes `{day}_metadata.json` with: schema_version, n_sequences, n_features, window_size, bin_size_seconds, feature_groups_enabled, label_horizons, market_open_et, date, export_timestamp, provenance |
| `normalization.rs` | Computes and writes `{day}_normalization.json` with per-feature mean and std (excluding categorical features at indices 29-30, 32-33) |

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
