# 03 Data Flow Specification

**Date**: 2026-03-22
**Status**: APPROVED — **Implementation Status**: Phases 1-5 complete (412 tests)
**Repo**: `basic-quote-processor`
**Contract Schema Version**: 1.0 (off-exchange feature space, independent of MBO 0-147)

---

## Table of Contents

1. [End-to-End Pipeline Diagram](#1-end-to-end-pipeline-diagram)
2. [Processing Lifecycle (Per-Day)](#2-processing-lifecycle-per-day)
3. [Key Types](#3-key-types)
4. [Price Precision Chain](#4-price-precision-chain)
5. [Empty Bin Handling Policy](#5-empty-bin-handling-policy)
6. [Half-Day Auto-Detection](#6-half-day-auto-detection)
7. [Data Validation Rules](#7-data-validation-rules)
8. [Reset Semantics](#8-reset-semantics)

---

## 1. End-to-End Pipeline Diagram

Two independent input streams converge at feature extraction. EQUS_SUMMARY provides daily context (consolidated volume for true dark share computation). XNAS.BASIC CMBP-1 provides the intraday record stream.

```
XNAS.BASIC (.dbn.zst)                     EQUS_SUMMARY (.dbn.zst)
       |                                         |
       v                                         v
  DbnReader                               DailyContextLoader
  (per-day file iteration)                (load consolidated vol)
       |                                         |
       v                                         |
  CmbpRecord (internal type)                     |
  (ts_recv, action, side, price,                 |
   size, bid/ask_px/sz/ct, pub_id)               |
       |                                         |
       |--- BboState.update()                    |
       |    (track Nasdaq L1 BBO)                |
       |                                         |
       |--- TradeClassifier.classify()           |
       |    (midpoint signing + BJZZ)            |
       |    -> ClassifiedTrade                   |
       |      { direction, retail_status,        |
       |        price, size, publisher_id }       |
       |                                         |
       v                                         |
  BinAccumulator.accumulate()                    |
  (aggregate per time bin)                       |
       |                                         |
       v                                         |
  FeatureExtractor.extract()  <------------------+
  (compute features from bin)     (daily context: consolidated_vol)
       |
       v
  Arc<Vec<f64>> (feature vector per bin)
       |
       |--- SequenceBuilder (sliding window -> [T, F] sequences)
       |
       |--- LabelComputer (point returns at H=1..60)
       |
       +--- ForwardPriceExporter (mid-price trajectory)
               |
               v
          NPY Export
          |-- {day}_sequences.npy      [N, T, F] float32
          |-- {day}_labels.npy         [N, H] float64 (point returns, bps)
          |-- {day}_forward_prices.npy [N, max_H+1] float64 (USD)
          |-- {day}_metadata.json      (schema, provenance, feature count)
          +-- {day}_normalization.json  (per-day stats for normalization)
```

### Data Source Properties

| Source | Schema | Record Type | Content |
|--------|--------|-------------|---------|
| XNAS.BASIC | CMBP-1 | Consolidated BBO + Trade | L1 quote updates and trade prints from Nasdaq UTP feeds |
| EQUS_SUMMARY | OHLCV-1D | Daily bar | Consolidated daily volume, OHLCV per symbol |

### Publisher IDs in XNAS.BASIC

| Publisher ID | Name | Classification | Description |
|---|---|---|---|
| 81 | XNAS | Lit | Nasdaq Stock Market (primary lit venue) |
| 82 | FINN | TRF | FINRA/Nasdaq TRF Carteret (off-exchange) |
| 83 | FINC | TRF | FINRA/Nasdaq TRF Chicago (off-exchange) |
| 88 | XBOS | Minor Lit | Nasdaq BX (secondary lit, configurable) |
| 89 | XPSX | Minor Lit | Nasdaq PSX (secondary lit, configurable) |

TRF publishers (82, 83) report off-exchange executions (dark pools, internalizers, wholesalers). All off-exchange flow features derive from trades with these publisher IDs. Minor lit venues (88, 89) are included in lit volume by default (`include_minor_lit_in_lit = true`).

---

## 2. Processing Lifecycle (Per-Day)

### 2.1 Phase 1: INIT

```
1. INIT
   +-- Load EQUS_SUMMARY for this day -> consolidated_volume
   +-- Open XNAS.BASIC .dbn.zst file -> DbnReader iterator
   +-- Initialize BboState         (all fields zeroed, bbo_valid = false)
   +-- Initialize TradeClassifier  (signing method from config)
   +-- Initialize BinAccumulator   (all counters zeroed)
   +-- Initialize FeatureExtractor (feature config loaded)
   +-- Set first_bin_start = market_open_et aligned to bin_size_seconds
   +-- Set warmup_counter = 0
```

If EQUS_SUMMARY is unavailable for a given day, `consolidated_volume` is set to NaN and `true_dark_share` will be NaN for the day. The `bin_valid` gate is NOT affected -- daily context is optional.

### 2.2 Phase 2: STREAM (Per-Record)

This is the hot loop. Every record from `DbnReader` is dispatched by its `action` field.

```
2. STREAM (per record from DbnReader)
   |
   |-- If action == 'A' (BBO update / quote):
   |   +-- BboState.update_from_record(record)
   |       Updates: bid_px, bid_sz, ask_px, ask_sz
   |       Recomputes: mid_price, spread, microprice
   |       Sets: last_bbo_update_ts = record.ts_recv
   |
   |-- If action == 'T' (trade):
   |   |
   |   |  *** CRITICAL: BBO MUST BE UPDATED BEFORE TRADE CLASSIFICATION ***
   |   |
   |   |-- Step 1: BboState.update_from_record(record)
   |   |   The CMBP-1 trade record carries the contemporaneous BBO in its
   |   |   bid/ask_px/sz/ct fields. These MUST be applied to BboState BEFORE
   |   |   classification. Using a stale BBO degrades Barber (2024) midpoint
   |   |   signing accuracy because the trade's midpoint reference would be
   |   |   computed from an outdated spread.
   |   |
   |   |-- Step 2: TradeClassifier.classify(record, &bbo_state) -> ClassifiedTrade
   |   |   Uses the UPDATED BBO midpoint for signing.
   |   |   Applies BJZZ subpenny test on record.price.
   |   |   Produces: direction (Buy/Sell/Unsigned), retail_status (Retail/Institutional/Unknown)
   |   |
   |   +-- Step 3: BinAccumulator.accumulate(classified_trade, &bbo_state)
   |       Adds to running counters: signed volume, trade count, retail counts, etc.
   |
   |-- If action is other ('R', 'N', etc.):
   |   +-- Skip with counter increment (diagnostic only, not an error)
   |       Log at DEBUG level if unexpected action count > 0 at day end.
   |
   +-- If ts_recv crosses bin boundary:
       |-- FeatureExtractor.extract(accumulator, bbo_state, daily_context) -> feature_vec
       |-- Apply empty bin policy if bin has zero TRF trades (see Section 5)
       |-- Apply warmup discard if warmup_counter < config.warmup_bins
       |   +-- warmup_counter += 1
       |   +-- Do NOT push feature_vec to SequenceBuilder
       |   +-- Do NOT record mid-price for label computation
       |   +-- Continue to next bin
       |-- SequenceBuilder.push(feature_vec)
       |-- LabelComputer.record_midprice(bbo_state.mid_price, bin_index)
       +-- BinAccumulator.reset()  (counters zeroed for next bin)
```

### 2.3 BBO Update Order -- Rationale and Specification

**This is the most safety-critical ordering constraint in the pipeline.**

The CMBP-1 schema embeds the contemporaneous BBO inside every trade record. When a trade arrives, its `bid_px`, `bid_sz`, `ask_px`, `ask_sz` fields (from `CbboMsg.levels[0]`) reflect the BBO at the time of trade execution on that venue. The processing order MUST be:

```
record arrives (action == 'T')
    |
    v
BboState.update_from_record(record)   // Step 1: apply embedded BBO
    |
    v
BboState now reflects the trade's     // Midpoint is current, not stale
contemporaneous quote
    |
    v
TradeClassifier.classify(record, &bbo_state)  // Step 2: sign using current midpoint
    |
    v
ClassifiedTrade.direction is correct   // Barber (2024) accuracy preserved
```

**Why this matters**: Midpoint signing compares `trade_price` to `(bid + ask) / 2`. If the BBO has not been updated from the trade record's embedded quote, the midpoint may be stale by one or more quote updates, shifting the signing boundary. In E9, stale-BBO signing reduced directional accuracy by ~3-5% on average, with worst-case degradation during fast moves.

**Implementation constraint**: The `classify()` method takes `&bbo_state` (immutable reference), NOT `&mut bbo_state`. The update happens BEFORE the borrow. This is enforced by Rust's borrow checker -- attempting to mutate `bbo_state` while `classify()` holds a reference would be a compile error.

### 2.4 Phase 3: FINALIZE

```
3. FINALIZE
   +-- Flush any partial final bin (if records exist past the last boundary)
   |   If partial bin has >= min_trades_per_bin, extract and push.
   |   Otherwise discard (partial bins at market close are common).
   |
   +-- LabelComputer.compute_labels(horizons) -> labels [N_bins, H]
   |   Bins within max_H of end-of-day have NaN labels for
   |   horizons they cannot compute. These bins are EXCLUDED from
   |   exported sequences (labels must be fully defined).
   |
   +-- SequenceBuilder.build_sequences(window_size, stride) -> sequences
   |   Sliding window over valid (post-warmup, valid-labeled) feature bins.
   |   Output shape: [N_sequences, T=window_size, F=n_features]
   |
   +-- ForwardPriceExporter.export(mid_prices, max_H) -> forward_prices
   |   forward_prices[t, h] = mid_price at bin (t + h), for h = 0..max_H
   |   Shape: [N_sequences, max_H + 1] float64 (USD)
   |   Column 0 is the base price at sequence end.
   |
   +-- Normalization stats computed from this day's valid feature bins:
   |   Per-feature mean and std (Welford accumulator).
   |   Categorical features excluded: [bin_valid(29), bbo_valid(30), time_bucket(32), schema_version(33)]
   |
   +-- NPY Export:
   |   |-- {day}_sequences.npy      [N, T, F] float32  (after normalization)
   |   |-- {day}_labels.npy         [N, H] float64     (point returns, bps, unnormalized)
   |   |-- {day}_forward_prices.npy [N, max_H+1] float64 (USD, unnormalized)
   |   |-- {day}_metadata.json      (see Section 2.5)
   |   +-- {day}_normalization.json  (means, stds, sample_count, excluded_indices)
   |
   +-- Pipeline.reset()  (clear ALL state for next day -- see Section 8)
```

### 2.5 Metadata JSON Specification

Every exported day includes a metadata JSON file with these required fields:

```json
{
  "day": "2025-06-15",
  "n_sequences": 342,
  "window_size": 20,
  "n_features": 34,
  "schema_version": "1.0",
  "contract_version": "off_exchange_1.0",
  "label_strategy": "point_return",
  "label_encoding": "continuous_bps",
  "horizons": [1, 2, 3, 5, 10, 20, 30, 60],
  "bin_size_seconds": 60,
  "market_open_et": "09:30",
  "normalization": {
    "strategy": "per_day_zscore",
    "applied": false,
    "params_file": "2025-06-15_normalization.json"
  },
  "provenance": {
    "source_file": "xnas-basic-20250615.cmbp-1.dbn.zst",
    "processor_version": "0.1.0",
    "export_timestamp_utc": "2026-03-22T14:30:00Z"
  },
  "export_timestamp": "2026-03-22T14:30:00Z",
  "first_bin_start_ns": 1718456400000000000,
  "last_bin_end_ns": 1718479800000000000,
  "n_bins_total": 390,
  "n_bins_valid": 367,
  "n_bins_warmup_discarded": 3,
  "n_bins_label_truncated": 20,
  "n_total_records": 285642,
  "n_trade_records": 42381,
  "n_trf_trades": 22105,
  "n_lit_trades": 12500,
  "data_source": "XNAS.BASIC",
  "schema": "cmbp-1",
  "symbol": "NVDA",
  "equs_summary_available": true,
  "consolidated_volume": 45230000,
  "trf_volume_fraction": 0.489,
  "feature_groups_enabled": { "signed_flow": true, "venue_metrics": true, "retail_metrics": true, "bbo_dynamics": true, "vpin": false, "trade_size": true, "cross_venue": true },
  "classification_config": { "exclusion_band": 0.10, "bjzz_lower": 0.001 },
  "signing_method": "midpoint",
  "exclusion_band": 0.10
}
```

> **Schema notes**: Field names match `src/export/metadata.rs` (`ExportMetadata` struct). Notably:
> - `label_strategy` (NOT `label_type`) — matches the JSON serialization
> - `normalization` is a nested object `{strategy, applied, params_file}` — not a bare string
> - `provenance` contains `source_file`, `processor_version`, `export_timestamp_utc`, `config_hash` (optional)
> - `consolidated_volume` and `trf_volume_fraction` are `Option<>` and are omitted (not `null`) when EQUS_SUMMARY is unavailable, via `#[serde(skip_serializing_if = "Option::is_none")]`

### 2.6 Bin Boundary Alignment

Time bins are grid-aligned to `market_open_et`:

```
bin_start(i) = market_open_et + i * bin_size_seconds
bin_end(i)   = market_open_et + (i + 1) * bin_size_seconds

Example (60s bins, 09:30 ET open):
  Bin 0: 09:30:00 -- 09:30:59.999...
  Bin 1: 09:31:00 -- 09:31:59.999...
  ...
  Bin 389: 16:00:00 -- 16:00:59.999...  (may be partial or empty)
```

A record with `ts_recv` belongs to bin `floor((ts_recv - market_open_ns) / bin_size_ns)`.

All timestamps are converted from UTC (wire format) to Eastern Time using DST-aware conversion (`hft_statistics::time::regime::utc_offset_for_date()` returns -4 EDT or -5 EST; `day_epoch_ns()` computes midnight UTC for the trading day). The UTC offset is computed once per day at pipeline init, not per-record.

---

## 3. Key Types

| Type | Size | Description |
|------|------|-------------|
| `CmbpRecord` | ~80B | Internal representation of a CMBP-1 record. Fields: `ts_event` (u64 ns UTC, from header), `ts_recv` (u64 ns UTC), `action` (u8: `b'T'` trade, `b'A'` quote), `side` (u8: `b'A'`/`b'B'`/`b'N'`), `flags` (u8 bitfield from `CbboMsg.flags.raw()` — used in Phase 2+ for TRF indicator detection), `price` (i64 nanodollars), `size` (u32 shares), `bid_px` (i64), `bid_sz` (u32), `ask_px` (i64), `ask_sz` (u32), `publisher_id` (u16, from header). Converted from `dbn::CbboMsg` at the reader boundary via `CmbpRecord::from_cbbo()` (field-by-field copy). **Note**: The Rust `dbn::CbboMsg` uses `ConsolidatedBidAskPair` which has `bid_pb`/`ask_pb` (publisher IDs at the BBO level) but NOT `bid_ct`/`ask_ct` (order counts). Order counts are available in the Python Databento SDK but not exposed by the Rust dbn crate v0.20.0. No feature formulas depend on order counts. |
| `BboState` | ~56B | Current Nasdaq L1 BBO snapshot. Fields: `bid_price` (f64 USD), `bid_size` (u32 shares), `ask_price` (f64 USD), `ask_size` (u32 shares), `mid_price` (f64 USD, derived), `spread` (f64 USD, derived), `microprice` (f64 USD, derived), `last_update_ts` (u64 ns), `is_valid` (bool). Updated from every record's embedded BBO fields. Validity requires `spread > 0` and `is_finite(bid_price)` and `is_finite(ask_price)`. |
| `ClassifiedTrade` | ~40B | A trade with signing and retail classification applied. Fields: `direction` (enum: Buy/Sell/Unsigned), `retail_status` (enum: Retail/Institutional/Unknown), `price` (f64 USD), `size` (u32 shares), `publisher_id` (u16), `ts_recv` (u64 ns). **Note**: `midpoint_at_trade` was removed — the accumulator reads midpoint directly from `BboState` (which is always updated before classification), avoiding redundant storage. |
| `BinAccumulator` | ~512B | Per-bin accumulation state. Contains running sums and counters for: signed volume (buy/sell/unsigned), trade counts (total, TRF, lit, retail, institutional), size statistics (Welford accumulators for mean/variance), BBO snapshot at bin end, quote update count, spread sum for averaging, subpenny trade count, odd-lot count, block trade count (size >= threshold). Reset at every bin boundary. |
| `FeatureVec` | `Arc<Vec<f64>>` | Feature vector for one time bin. Length = `n_features` (34 with all groups enabled). `Arc` wrapper enables zero-copy sharing between `SequenceBuilder` sliding window slots. |
| `DailyContext` | ~32B | Per-day context from EQUS_SUMMARY. Fields: `consolidated_volume` (u64 shares), `daily_open` (f64 USD), `daily_high` (f64 USD), `daily_low` (f64 USD), `daily_close` (f64 USD). Loaded once at day init, read-only during processing. |

### Type Relationships

```
DbnReader --yields--> CmbpRecord
CmbpRecord --updates--> BboState
CmbpRecord + BboState --produces--> ClassifiedTrade
ClassifiedTrade + BboState --fed into--> BinAccumulator
BinAccumulator + BboState + DailyContext --produces--> FeatureVec
FeatureVec --pushed to--> SequenceBuilder
SequenceBuilder --emits--> [T, F] sequences (Vec<Arc<Vec<f64>>>)
```

### Enum Types

```rust
enum TradeDirection {
    Buy,       // Trade price > midpoint + exclusion_band * spread
    Sell,      // Trade price < midpoint - exclusion_band * spread
    Unsigned,  // Trade price within exclusion band of midpoint
}

enum RetailStatus {
    Retail,        // BJZZ subpenny test positive (fractional cent in retail range)
    Institutional, // BJZZ subpenny test negative (at round cent or wrong fractional)
    Unknown,       // BBO invalid at time of trade, classification impossible
}

enum PublisherClass {
    Trf,       // FINN (82), FINC (83)
    Lit,       // XNAS (81)
    MinorLit,  // XBOS (88), XPSX (89)
}
```

---

## 4. Price Precision Chain

Every price transformation in the pipeline is explicit and occurs at a defined boundary. There are exactly four representations, and each conversion happens exactly once per price value.

```
Stage 1: Databento Wire Format
  Type:   i64 (fixed-point)
  Unit:   nanodollars (1 USD = 1,000,000,000 nanodollars)
  Scale:  FIXED_PRICE_SCALE = 1e-9
  Range:  i64::MIN..i64::MAX (sufficient for all equity prices)
  Where:  .dbn.zst file on disk

          |
          | Conversion: price_usd = price_nanodollars as f64 * 1e-9
          | Location:   DbnReader, at record construction
          | Precision:  f64 has 15-16 significant digits;
          |             nanodollar prices for equities use ~12 digits max.
          |             No precision loss for any realistic equity price.
          |
          v

Stage 2: CmbpRecord Internal Storage
  Type:   i64 (preserved from wire)
  Unit:   nanodollars
  Where:  CmbpRecord.price, CmbpRecord.bid_px, CmbpRecord.ask_px
  Note:   Stored as i64 to avoid premature conversion. The i64->f64
          conversion happens at the BboState boundary, not here.
          CmbpRecord is a thin wrapper over the DBN record.

          |
          | Conversion: f64_price = i64_price as f64 * 1e-9
          | Location:   BboState.update_from_record()
          | Frequency:  Once per BBO update (every record with BBO fields)
          |
          v

Stage 3: BboState and All Feature Computation
  Type:   f64
  Unit:   USD (for prices), shares (for sizes), various (for derived)
  Where:  BboState.bid_price, BboState.ask_price, BboState.mid_price,
          BboState.spread, ClassifiedTrade.price, all feature computations
  Note:   ALL intermediate computation is f64. No f32 until export.
          Midpoint: (bid_price + ask_price) / 2.0
          Spread:   ask_price - bid_price
          Microprice: (bid_price * ask_size + ask_price * bid_size)
                      / (bid_size + ask_size)  [with EPS guard on denom]

          |
          | Conversion: f32_value = f64_value as f32
          | Location:   NPY export boundary (npy_export.rs)
          | Guard:      assert!(value.is_finite()) BEFORE downcast
          | Impact:     f32 has ~7 significant digits.
          |             For normalized features (z-score, mean ~0, std ~1),
          |             this provides ~7 digits of precision -- sufficient.
          |             For unnormalized prices, f32 is NOT used (labels
          |             and forward_prices remain f64).
          |
          v

Stage 4: NPY Export
  Type:   f32 (sequences ONLY) or f64 (labels, forward_prices)
  Unit:   normalized (sequences) or bps (labels) or USD (forward_prices)
  Where:  {day}_sequences.npy -> f32
          {day}_labels.npy -> f64  (point returns in bps, NOT downcast)
          {day}_forward_prices.npy -> f64  (mid-prices in USD, NOT downcast)
```

### Precision Validation

| Conversion | Source | Target | Max Error | Acceptable |
|---|---|---|---|---|
| i64 nanodollars -> f64 USD | 12 digits | 15 digits | 0 (exact for prices < $10M) | Yes |
| f64 USD -> f64 midpoint | 15 digits | 15 digits | < 1e-15 USD | Yes |
| f64 normalized -> f32 | 15 digits | 7 digits | ~1e-7 (relative) | Yes for z-scored features |
| f64 bps -> f64 (no conversion) | 15 digits | 15 digits | 0 | Yes, labels stay f64 |

### Invariants

1. **No f32 in computation**: All math uses f64. f32 appears only in the NPY export of feature sequences.
2. **Labels never downcast**: Point-return labels (bps) and forward prices (USD) are exported as f64.
3. **One conversion point**: Each price is converted from i64 to f64 exactly once, at `BboState.update_from_record()`.
4. **Finite guard before downcast**: `assert!(value.is_finite())` before every `f64 as f32` cast. Non-finite values indicate an upstream bug, not a data condition.

---

## 5. Empty Bin Handling Policy

Bins with zero TRF trades are expected during low-activity periods (pre-market, lunch, post-hours). The pipeline handles these deterministically.

### 5.1 Feature Categories and Empty-Bin Behavior

| Category | Features (Indices) | Empty Bin Value | Rationale |
|---|---|---|---|
| **Flow features** | trf_signed_imbalance (0), mroib (1), inv_inst_direction (2), bvc_imbalance (3), trf_volume (5), retail_trade_rate (10), trade_count (22), trf_burst_intensity (24), bin_trade_count (27), bin_trf_trade_count (28) | **0.0** | No trades = no flow. Zero is the neutral value for imbalance/count/rate metrics. |
| **State features** | subpenny_intensity (8), odd_lot_ratio (9), retail_volume_fraction (11), dark_share (4), mean_trade_size (20), block_trade_ratio (21), size_concentration (23), time_since_burst (25), trf_lit_volume_ratio (26) | **Forward-fill** from previous bin | These describe market state, not instantaneous flow. The last observed state persists until updated. |
| **BBO features** | spread_bps (12), bid_pressure (13), ask_pressure (14), bbo_update_rate (15), quote_imbalance (16), spread_change_rate (17) | **From BboState** (live quote data) | BBO may update even without TRF trades. Features computed from BboState directly, not from trade flow. |
| **VPIN** | trf_vpin (18), lit_vpin (19) | **Forward-fill** | VPIN is a rolling estimate; persists until new volume buckets form. |
| **Volume features** | lit_volume (6), total_volume (7) | **From BboState / lit trades** | Lit trades may exist even when TRF trades are zero. |
| **Safety gates** | bin_valid (29), bbo_valid (30) | **Computed** (see below) | Always computed from actual conditions. |
| **Context** | session_progress (31), time_bucket (32), schema_version (33) | **Computed** from clock | Always available; no dependence on trade flow. |

### 5.2 Safety Gate Computation

**`bin_valid` (index 29)**:
```
bin_valid = 1.0  if n_trf_trades >= config.min_trades_per_bin  (default: 10)
bin_valid = 0.0  otherwise
```
This gate indicates whether the bin has enough TRF trade data for flow features to be statistically meaningful. Downstream models should mask or down-weight bins with `bin_valid = 0.0`.

**`bbo_valid` (index 30)**:
```
bbo_valid = 1.0  if BboState.is_valid == true
                 AND (current_ts - BboState.last_update_ts) < config.bbo_staleness_max_ns
bbo_valid = 0.0  otherwise
```
Default staleness threshold: 5 seconds (5,000,000,000 ns). If the BBO has not been updated for more than this duration, quotes are considered stale and BBO-derived features are unreliable.

### 5.3 Forward-Fill Mechanics

For state features that forward-fill:

1. At day start, all forward-fill state is initialized to `0.0` (no prior state).
2. When a bin with `n_trf_trades >= 1` completes, its state feature values are stored as the forward-fill source.
3. When a subsequent bin has `n_trf_trades == 0`, state features use the stored forward-fill values.
4. Forward-fill state resets at day boundary (see Section 8). There is NO cross-day forward-fill.

**Implementation**: A `ForwardFillState` struct holds the last valid value for each state feature. Updated at every non-empty bin. Read at every empty bin.

### 5.4 Warmup Period

The first `config.warmup_bins` bins (default: 3) of each day are discarded entirely:

- Feature vectors are computed (accumulators need the data) but NOT pushed to `SequenceBuilder`.
- Mid-prices are NOT recorded for label computation.
- Forward-fill state IS updated during warmup (so the first post-warmup bin has valid forward-fill if needed).

Warmup ensures accumulators (running statistics, VPIN buckets) reach a stable state before feature emission. The warmup count is recorded in metadata as `n_bins_warmup_discarded`.

### 5.5 NaN Guard Policy

Every division in the feature computation pipeline is guarded:

```rust
const EPS: f64 = 1e-8;

// Pattern for all ratio computations:
let ratio = if denominator.abs() < EPS {
    0.0  // or forward-fill value, depending on feature category
} else {
    numerator / denominator
};
```

At the NPY export boundary, every value in the feature vector is checked:

```rust
for (i, &value) in feature_vec.iter().enumerate() {
    assert!(
        value.is_finite(),
        "Non-finite value at feature index {}: {} (day={}, bin={})",
        i, value, day, bin_index
    );
}
```

Non-finite values at export are treated as bugs, not data conditions. The pipeline must never produce NaN or Inf in a feature vector. If a denominator is near-zero, the empty bin policy or EPS guard handles it before NaN can propagate.

---

## 6. Half-Day Auto-Detection

NYSE early-close days (approximately 3 per year: July 3, day after Thanksgiving, Christmas Eve) close at 13:00 ET instead of 16:00 ET. The pipeline does NOT use a hardcoded calendar. Instead, it detects end-of-day from the data stream.

### 6.1 Detection Algorithm

```
consecutive_empty_bins = 0

for each bin boundary:
    if bin had zero records (no quotes AND no trades):
        consecutive_empty_bins += 1
    else:
        consecutive_empty_bins = 0

    if consecutive_empty_bins >= config.close_detection_gap_bins:  // default: 5
        detected_close_time = current_bin_start - (close_detection_gap_bins * bin_size)
        break  // day is complete
```

### 6.2 Consequences of Early Close Detection

1. **Session progress adjustment**: The `session_progress` feature (index 31) is computed as:
   ```
   session_progress = elapsed_time / detected_session_duration
   ```
   On a half-day, `detected_session_duration` = 13:00 - 09:30 = 3.5 hours (instead of 6.5 hours). This means `session_progress = 1.0` at 13:00, not at 16:00.

2. **Label truncation**: Bins within `max_H` bins of the detected close have NaN labels for horizons that extend past close. These bins are excluded from exported sequences.

3. **Metadata**: The metadata JSON includes the detected close time and actual session duration.

### 6.3 Edge Cases

| Condition | Handling |
|---|---|
| Trading halt (not early close) | Halts typically last < 5 bins at 60s. Increase `close_detection_gap_bins` if halts cause false positives. At 10s bins, use a larger gap (e.g., 15). |
| No records at all for a day | Day skipped entirely. Logged as warning. Not exported. |
| Records only in pre-market (before 09:30 ET) | No bins emitted (all records fall before `market_open_et`). Day skipped. |
| Very low activity day | May trigger false early-close. The `close_detection_gap_bins` default of 10 (= 10 minutes at 60s bins) is tuned to avoid this for NVDA and to absorb LULD halts (typically <5 min). |

### 6.4 Configuration

```toml
[validation]
auto_detect_close = true          # Enable/disable auto-detection
close_detection_gap_bins = 10     # Consecutive empty bins to trigger close (default: 10)
```

If `auto_detect_close = false`, the pipeline always uses `market_close_et` (default 16:00 ET) as the session end.

---

## 7. Data Validation Rules

Validation occurs at three pipeline stages: record ingestion, feature extraction, and export.

### 7.1 Record-Level Validation (DbnReader)

| Check | Condition | Action |
|---|---|---|
| Timestamp monotonicity | `ts_recv >= prev_ts_recv` | Warn and accept (out-of-order by < 1ms is a known feed artifact) |
| Price sanity | `price > 0` for trades | Skip record, increment `n_invalid_prices` counter |
| Size sanity | `size > 0` for trades | Skip record, increment `n_zero_size_trades` counter |
| Publisher ID valid | `publisher_id in {81, 82, 83, 88, 89}` | Skip record, increment `n_unknown_publishers` counter |
| Action recognized | `action in {'A', 'T', 'R', 'N', ...}` | Non-A/T actions: skip with counter. All actions accepted by parser. |

### 7.2 BBO-Level Validation (BboState)

| Check | Condition | On Failure |
|---|---|---|
| Spread positive | `ask_price - bid_price > 0` | `BboState.is_valid = false`. Features that require valid BBO use forward-fill or zero. `bbo_valid` gate = 0.0. |
| Prices finite | `bid_price.is_finite() && ask_price.is_finite()` | `BboState.is_valid = false`. Same consequences as above. |
| Spread reasonable | `spread_bps < 1000` (10% of price) | Log warning. Accept value (extreme spreads happen during halts and open/close auctions). Do NOT clamp. |
| Crossed book | `bid_price >= ask_price` | `BboState.is_valid = false`. This can occur briefly during fast market transitions in consolidated feeds. |

### 7.3 Trade Classification Validation

| Check | Condition | On Failure |
|---|---|---|
| BBO valid at signing | `BboState.is_valid == true` | Trade direction = `Unsigned`, retail status = `Unknown`. Increment `n_unsigned_invalid_bbo` counter. |
| Trade price within bounds | `bid_price <= trade_price <= ask_price` (within tolerance) | Accept anyway (trades can execute outside NBBO due to latency). Log if outside by > 10 bps. |
| Subpenny extraction | fractional cent part extracted correctly | If price has no subpenny component (exact cent), classify as `Institutional` per BJZZ. |

### 7.4 Feature-Level Validation (FeatureExtractor)

| Check | Condition | On Failure |
|---|---|---|
| All values finite | `feature_vec.iter().all(\|v\| v.is_finite())` | Bug. Panic with diagnostic (feature index, day, bin). |
| Feature count | `feature_vec.len() == expected_n_features` | Bug. Panic. |
| Safety gates in {0.0, 1.0} | `bin_valid` and `bbo_valid` are exactly 0.0 or 1.0 | Bug. Panic. |
| Schema version correct | `feature_vec[33] == SCHEMA_VERSION` | Bug. Panic. |
| Session progress in [0.0, 1.0] | bounds check | Clamp to [0.0, 1.0] with warning (can exceed 1.0 briefly if auto-close detection is slow). |

### 7.5 Export-Level Validation (Exporter)

| Check | Condition | On Failure |
|---|---|---|
| Sequences shape | `sequences.shape == [N, T, F]` where T = `window_size`, F = `n_features` | Bug. Panic. |
| Labels shape | `labels.shape == [N, H]` where H = `len(horizons)` | Bug. Panic. |
| Forward prices shape | `forward_prices.shape == [N, max_H + 1]` | Bug. Panic. |
| N consistency | `sequences.shape[0] == labels.shape[0] == forward_prices.shape[0]` | Bug. Panic. |
| All sequences finite | `sequences.iter().all(\|v\| v.is_finite())` | Bug. Panic. |
| All labels finite | `labels.iter().all(\|v\| v.is_finite())` | Bug. Panic (NaN labels should have been excluded). |
| N > 0 | At least one sequence exported | Warn and skip day (no output files created). |
| Normalization stats valid | `std > 0` for all non-categorical features | If `std == 0`, normalize to 0.0 (constant feature). Log which features are constant. |

---

## 8. Reset Semantics

Two levels of reset occur in the pipeline: per-bin (accumulator reset) and per-day (full pipeline reset). No state carries across day boundaries.

### 8.1 Per-Bin Reset (BinAccumulator)

**Trigger**: Time crosses a bin boundary.

**What resets**:
| Component | Reset Action |
|---|---|
| Signed volume counters | Set to 0 (buy_volume, sell_volume, unsigned_volume) |
| Trade counts | Set to 0 (n_trades, n_trf_trades, n_lit_trades, n_retail_trades, n_institutional_trades) |
| Size statistics | Welford accumulators reset (count=0, mean=0, M2=0) |
| Subpenny counter | Set to 0 |
| Odd-lot counter | Set to 0 |
| Block trade counter | Set to 0 |
| BBO snapshot for bin | Overwritten from live BboState at extraction time |
| Quote update counter | Set to 0 |
| Spread accumulator | Set to 0 (for average spread computation) |

**What persists across bins**:
| Component | Persistence Reason |
|---|---|
| BboState | BBO is a running state, not per-bin. Quote updates are continuous. |
| Forward-fill state | Carries last valid state feature values across empty bins. |
| VPIN buckets | VPIN is computed over a rolling volume window, not per-bin. |
| Warmup counter | Counts bins since day start, monotonically increasing. |
| SequenceBuilder buffer | Accumulates feature vectors for windowed sequence construction. |
| LabelComputer mid-prices | Accumulates bin-end mid-prices for forward-return computation. |
| TRF burst tracker | Tracks inter-TRF-burst timing across bins for `trf_burst_intensity` and `time_since_burst`. |

### 8.2 Per-Day Reset (Pipeline)

**Trigger**: All records for a day have been processed and exported (FINALIZE completes).

**Everything resets**. No state carries from day D to day D+1.

| Component | Reset Action |
|---|---|
| BboState | All fields zeroed. `is_valid = false`. `last_update_ts = 0`. |
| TradeClassifier | No persistent state (stateless classification). |
| BinAccumulator | Full reset (same as per-bin reset). |
| Forward-fill state | All forward-fill values set to 0.0. |
| FeatureExtractor | No persistent state (stateless extraction from accumulator). |
| SequenceBuilder | Buffer cleared. Index reset to 0. |
| LabelComputer | Mid-price buffer cleared. |
| ForwardPriceExporter | Buffer cleared. |
| VPIN estimator | All volume buckets cleared. Running buy/sell volume reset. |
| TRF burst tracker | Burst history cleared. Timing state reset. |
| Warmup counter | Reset to 0. |
| DailyContext | Replaced with next day's EQUS_SUMMARY data (or NaN if unavailable). |
| Diagnostic counters | Logged/recorded, then reset. |

### 8.3 Reset Ordering

The per-day reset MUST occur after FINALIZE exports are written and BEFORE the first record of the next day is processed:

```
Day N: INIT -> STREAM -> FINALIZE -> [export written to disk]
                                          |
                                    Pipeline.reset()
                                          |
Day N+1: INIT -> STREAM -> FINALIZE -> [export written to disk]
                                          |
                                    Pipeline.reset()
                                          ...
```

### 8.4 Reset Testing Requirements

Each component's reset must be tested with the following pattern:

```rust
#[test]
fn test_component_reset_produces_clean_state() {
    let mut component = Component::new(config);

    // Process some data to dirty the state
    component.process(sample_data_1);
    assert_ne!(component, Component::new(config));  // State has changed

    // Reset
    component.reset();

    // Process same data again
    component.process(sample_data_1);

    // Must produce identical results as a fresh component
    let mut fresh = Component::new(config);
    fresh.process(sample_data_1);
    assert_eq!(component.output(), fresh.output());
}
```

This test verifies that `reset()` truly returns the component to its initial state, with no ghost state leaking between days.

---

## Appendix A: Data Volume Estimates

For NVDA at 60-second bins across a full trading day (09:30 - 16:00 ET):

| Metric | Estimate | Notes |
|---|---|---|
| Trading minutes per day | 390 | 6.5 hours |
| Bins per day (60s) | 390 | Grid-aligned |
| Valid bins per day (post-warmup, pre-label-truncation) | ~327 | 390 - 3 warmup - 60 label-truncated (at max_H=60) |
| Sequences per day (window=20, stride=1) | ~307 | 327 - 20 + 1 |
| CMBP-1 records per day (typical) | ~250K-400K | Quotes + trades |
| TRF trades per day (typical) | ~15K-30K | ~45-55% of all trades |
| Feature vector size (34 features, f64) | 272 bytes | 34 * 8 bytes |
| Sequence size (20 bins, 34 features, f32) | 2,720 bytes | 20 * 34 * 4 bytes |
| Daily sequences file (f32) | ~835 KB | 307 * 2,720 bytes |
| Daily labels file (8 horizons, f64) | ~19 KB | 307 * 8 * 8 bytes |
| 233-day dataset total | ~220 MB | Sequences + labels + metadata |

## Appendix B: Output File Layout

```
data/exports/{experiment_name}/
    train/
        {day}_sequences.npy          [N, T=20, F=34]  float32
        {day}_labels.npy             [N, H=8]         float64  (bps)
        {day}_forward_prices.npy     [N, 61]          float64  (USD, max_H=60)
        {day}_metadata.json
        {day}_normalization.json
    val/
        ...  (same structure)
    test/
        ...  (same structure)
    dataset_manifest.json
```

The `dataset_manifest.json` at the top level contains:

```json
{
  "experiment_name": "basic_nvda_60s",
  "schema_version": "1.0",
  "bin_size_seconds": 60,
  "window_size": 20,
  "n_features": 34,
  "horizons": [1, 2, 3, 5, 10, 20, 30, 60],
  "label_strategy": "point_return",
  "normalization": "none",
  "train_days": ["2025-02-03", "2025-02-04", "..."],
  "val_days": ["2025-10-01", "2025-10-02", "..."],
  "test_days": ["2025-11-14", "2025-11-17", "..."],
  "train_sequences": 50123,
  "val_sequences": 10456,
  "test_sequences": 10234,
  "config": { ... },
  "created_at": "2026-03-22T14:30:00Z"
}
```
