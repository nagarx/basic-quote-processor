# Integration Points: basic-quote-processor with HFT Pipeline

**Status**: Design Specification — **Implementation Status**: Phases 1-5 complete (412 tests, NPY export + EQUS integration working)
**Date**: 2026-03-22 (spec), 2026-03-23 (implementation)
**Scope**: How basic-quote-processor connects to the MBO pipeline, EQUS_SUMMARY data, hft-statistics, pipeline_contract.toml, and downstream consumers (trainer, backtester)

---

## Table of Contents

1. [Integration with MBO Pipeline](#1-integration-with-mbo-pipeline)
2. [Integration with EQUS_SUMMARY](#2-integration-with-equs_summary)
3. [Integration with hft-statistics](#3-integration-with-hft-statistics)
4. [Integration with pipeline_contract.toml](#4-integration-with-pipeline_contracttoml)
5. [Export Compatibility](#5-export-compatibility)
6. [Sign Convention](#6-sign-convention)

---

## 1. Integration with MBO Pipeline

### 1.1 Independence Principle

The basic-quote-processor produces exports that are **structurally independent** from MBO pipeline exports. The two producers share no code, no state, and no runtime coupling. Fusion happens entirely downstream (in `lob-model-trainer` or a dedicated fusion script), never inside either producer.

This separation exists because the two data sources have fundamentally different structures:
- MBO pipeline: order lifecycle events (Add/Cancel/Modify/Trade) with full LOB reconstruction
- basic-quote-processor: L1 consolidated quotes (CMBP-1) with trade prints, no order lifecycle

### 1.2 Export Directory Structure

Both producers emit to sibling directories under `data/exports/`:

```
data/exports/
    basic_nvda_60s/                          # basic-quote-processor output
        train/
            2025-02-03_sequences.npy         # [N, T=20, F=34]  float32
            2025-02-03_labels.npy            # [N, H=8]         float64  (bps)
            2025-02-03_forward_prices.npy    # [N, max_H+1]     float64  (USD)
            2025-02-03_metadata.json
            2025-02-03_normalization.json
            ...
        val/
            ...
        test/
            ...
        dataset_manifest.json

    e5_timebased_60s/                        # MBO pipeline output
        train/
            2025-02-03_sequences.npy         # [N, T=20, F=98]  float32
            2025-02-03_labels.npy            # [N, H]           float64  (bps)
            2025-02-03_forward_prices.npy    # [N, max_H+1]     float64  (USD)
            2025-02-03_metadata.json
            2025-02-03_normalization.json
            ...
        val/
            ...
        test/
            ...
        dataset_manifest.json
```

### 1.3 Alignment Contract

For downstream fusion to work, both producers must agree on temporal alignment. The alignment contract defines the metadata fields that must match between paired exports.

**Required agreement (exact match)**:

| Field | Location | Example Value | Rationale |
|-------|----------|---------------|-----------|
| `bin_size_seconds` | `metadata.json` | `60` | Different bin sizes produce incompatible sequences |
| `market_open_et` | `metadata.json` | `"09:30"` | Grid must start at same wall-clock time |
| `date` | `metadata.json` | `"2025-02-03"` | Day-level pairing |
| `normalization.strategy` | `metadata.json` | `"per_day_zscore"` | Normalization must be compatible (or both raw) |
| `sequence.window_size` | `manifest.json` | `20` | Sequences must cover same number of bins |
| `sequence.stride` | `manifest.json` | `1` | Same stride ensures 1:1 sequence alignment |

**Required agreement (compatible, not necessarily identical)**:

| Field | Constraint | Rationale |
|-------|-----------|-----------|
| `split_dates` | Same train/val/test boundaries | Prevents leakage across splits |
| `horizons` | Must share at least one horizon | Label comparison requires common horizon |

**Allowed to differ**:

| Field | Why |
|-------|-----|
| `n_features` | Different feature spaces (34 vs 98) |
| `n_sequences` per day | Bin counts may differ due to data availability |
| `label_strategy` | MBO may use smoothed; off-exchange uses point-return only |
| `schema_version` | Independent version spaces |

### 1.4 Fusion Specification

Downstream fusion merges MBO and off-exchange features into a single tensor for model training. The fusion logic lives in the consumer (trainer), not in either producer.

**Fusion algorithm (per day)**:

```python
# Pseudocode — lives in lob-model-trainer or a fusion utility

def fuse_day(mbo_dir: Path, offex_dir: Path, day: str) -> tuple[np.ndarray, np.ndarray]:
    """
    Merge MBO and off-exchange features for a single day.

    Returns:
        sequences: [N, T, F_mbo + F_offex] float32
        labels:    [N, H] float64  (from off-exchange, point returns)
    """
    # 1. Load both
    mbo_seq = np.load(mbo_dir / f"{day}_sequences.npy")      # [N_mbo, T, F_mbo]
    offex_seq = np.load(offex_dir / f"{day}_sequences.npy")   # [N_offex, T, F_offex]

    # 2. Validate alignment metadata
    mbo_meta = json.load(open(mbo_dir / f"{day}_metadata.json"))
    offex_meta = json.load(open(offex_dir / f"{day}_metadata.json"))
    assert mbo_meta["bin_size_seconds"] == offex_meta["bin_size_seconds"]
    assert mbo_meta["market_open_et"] == offex_meta["market_open_et"]
    assert mbo_meta["date"] == offex_meta["date"]

    # 3. Align sequence counts (take minimum)
    N = min(mbo_seq.shape[0], offex_seq.shape[0])
    assert mbo_seq.shape[1] == offex_seq.shape[1], "Window size mismatch"

    # 4. Concatenate along feature axis
    fused = np.concatenate([mbo_seq[:N], offex_seq[:N]], axis=2)  # [N, T, F_mbo + F_offex]

    # 5. Labels from off-exchange (point returns) — or from MBO, depending on experiment
    labels = np.load(offex_dir / f"{day}_labels.npy")[:N]  # [N, H]

    return fused, labels
```

**Feature dimension after fusion**: `F_mbo + F_offex`. With MBO stable features (98) and off-exchange features (30 active + 4 gates/context = 34 total), the fused tensor is `[N, T=20, F=132]`.

**Important**: The fusion layer must track which feature indices map to which source. The fused tensor layout is:

```
[0, F_mbo)         = MBO features (indexed by FeatureIndex enum)
[F_mbo, F_mbo+F_offex) = Off-exchange features (indexed by OffExchangeFeatureIndex enum)
```

### 1.5 Handling Mismatched Bin Counts

The MBO pipeline and basic-quote-processor may produce different numbers of bins per day because:
- MBO data may start/end at slightly different times
- Off-exchange data may have more early/late-close detection sensitivity
- Warmup periods differ (MBO: 100+ events for `mbo_ready`; off-exchange: 3 bins)

**Resolution rules**:

| Scenario | Resolution |
|----------|-----------|
| `N_mbo > N_offex` | Truncate MBO sequences to `N_offex` (drop trailing MBO bins) |
| `N_mbo < N_offex` | Truncate off-exchange sequences to `N_mbo` (drop trailing off-exchange bins) |
| `N_mbo == 0` or `N_offex == 0` | Skip day entirely; log warning |
| Difference > 10% of max(N_mbo, N_offex) | Skip day; log error (likely alignment issue) |

**Alignment is temporal, not index-based**: Both producers grid-align to `market_open_et` (09:30 ET). Bin `i` in both exports corresponds to the same wall-clock interval `[09:30 + i*bin_size, 09:30 + (i+1)*bin_size)`. This is what makes truncation correct -- we are dropping bins at the end of the day, not introducing misalignment.

Each metadata JSON includes `first_bin_start_ns` (UTC nanosecond timestamp of the first bin start) and `last_bin_end_ns` (UTC nanosecond timestamp of the last bin end). The fusion layer validates that these overlap within one bin_size tolerance.

### 1.6 Label Compatibility

The MBO pipeline and basic-quote-processor use different label strategies:

| Property | MBO Pipeline | basic-quote-processor |
|----------|-------------|----------------------|
| Label type | Smoothed-average return (TLOB), classification, or regression | Point-to-point return ONLY |
| Horizon unit | Events (H=10 events) or time bins (H=10 bins at 60s) | Time bins (H=1..60 bins) |
| Label file | `{day}_labels.npy` or `{day}_regression_labels.npy` | `{day}_labels.npy` |
| Shape | `[N]` int8 or `[N, H]` float64 | `[N, H]` float64 |
| Unit | Class index or bps | Basis points |
| Forward prices | `{day}_forward_prices.npy` [N, k+H+1] float64 USD | `{day}_forward_prices.npy` [N, max_H+1] float64 USD |

**For fused training, use off-exchange labels (point returns)**. This is a deliberate design decision based on E8 findings: smoothed labels produce models that track the smoothing artifact rather than tradeable returns (DA=48.3% on point returns vs 74.9% on smoothed labels). Point-return labels ensure the model optimizes for the actual execution target.

**When using MBO features with off-exchange labels**: The MBO metadata may indicate `label_strategy = "tlob"` while the labels actually used come from the off-exchange export with `label_strategy = "point_return"`. The fusion layer must record which label source was used in the fused metadata, not inherit blindly from either producer.

**Forward price compatibility**: Both producers export forward mid-prices in USD. The off-exchange forward prices use the Nasdaq BBO midpoint. The MBO forward prices use the reconstructed LOB midpoint. These differ slightly due to:
- BBO vs 10-level LOB midpoint calculation
- Update frequency differences

For label computation, use a single forward price source consistently. The off-exchange forward prices are recommended because they reflect the BBO midpoint that determines actual execution prices.

---

## 2. Integration with EQUS_SUMMARY

### 2.1 Purpose

EQUS_SUMMARY provides daily consolidated statistics for NVDA across ALL venues (not just XNAS). It serves as the ground truth denominator for venue-share features and as a data completeness check.

### 2.2 Data Source

```
data/EQUS_SUMMARY/NVDA/ohlcv1d_2025-02-03_to_2026-03-05/
    equs-summary-*.ohlcv-1d.dbn.zst
```

File format: Databento `.dbn.zst` containing OHLCV-1D records with schema `ohlcv-1d`. One record per symbol per day.

### 2.3 Read-Only Loading Pattern

EQUS_SUMMARY is loaded once per day during pipeline initialization. It is never modified, never cached across days, and never used as an intraday feature.

```rust
// Pseudocode — lives in src/pipeline.rs

impl Pipeline {
    fn init_day(&mut self, date: NaiveDate) -> Result<(), PipelineError> {
        // Attempt to load EQUS_SUMMARY for this date
        let daily_context = match self.context_loader.load(date) {
            Ok(ctx) => ctx,
            Err(ContextError::DateNotFound(d)) => {
                warn!("EQUS_SUMMARY missing for {d}, using fallback");
                DailyContext::fallback(date)
            }
            Err(e) => return Err(PipelineError::ContextLoad(e)),
        };

        self.daily_context = daily_context;
        Ok(())
    }
}
```

### 2.4 Error Handling When Data is Unavailable

| Scenario | Behavior | Impact |
|----------|----------|--------|
| EQUS_SUMMARY file missing entirely | Pipeline proceeds with `DailyContext::fallback()` | `true_dark_share` unavailable; `dark_share` uses intraday TRF/lit ratio instead |
| Date not found in file | Same as above | Same |
| Volume is zero or NaN | Use `EPS` (1e-8) as denominator guard | `true_dark_share` will be very large; `bin_valid` should catch this |
| EQUS_SUMMARY path not configured | Pipeline proceeds without daily context | All EQUS-dependent features use fallback values |

**Fallback `DailyContext`**:

```rust
impl DailyContext {
    fn fallback(date: NaiveDate) -> Self {
        DailyContext {
            date,
            consolidated_volume: None,    // None = not available
            daily_open: None,
            daily_high: None,
            daily_low: None,
            daily_close: None,
            daily_vwap: None,
        }
    }
}
```

When `consolidated_volume` is `None`:
- `true_dark_share` is not computed (set to `NaN`, masked by `bin_valid`)
- `dark_share` falls back to intraday computation: `trf_volume / (trf_volume + lit_volume + EPS)`
- The metadata JSON records `"equs_summary_available": false` for downstream awareness

### 2.5 Daily Features from EQUS_SUMMARY

| Feature | Formula | Unit | Used For |
|---------|---------|------|----------|
| `true_dark_share` | `trf_daily_volume / consolidated_volume` | ratio [0, 1] | Ground-truth off-exchange fraction (daily level, NOT per-bin) |
| `daily_range_bps` | `(daily_high - daily_low) / daily_close * 10000` | bps | Regime context (high-vol vs low-vol day) |
| `daily_volume` | `consolidated_volume` | shares | Normalization denominator for VPIN bucket sizing |

These are **daily constants** embedded into every bin's feature vector for the day. They do not change intraday.

---

## 3. Integration with hft-statistics

### 3.1 Path Dependency Configuration

```toml
# basic-quote-processor/Cargo.toml
[dependencies]
hft-statistics = { git = "https://github.com/nagarx/hft-statistics.git" }
```

```toml
# basic-quote-processor/.cargo/config.toml
[patch.crates-io]
# No patches needed — hft-statistics is a local path dependency
```

The `hft-statistics` crate is a leaf crate with zero domain dependencies (no LOB, MBO, or options types). It provides bounded-memory streaming accumulators and DST-aware time utilities.

### 3.2 Reused Primitives

| Primitive | Module | Description | Usage in basic-quote-processor |
|-----------|--------|-------------|-------------------------------|
| `WelfordAccumulator` | `statistics::welford` | Numerically stable single-pass running mean/variance (Welford 1962). Supports `update(x)`, `mean()`, `variance()`, `count()`, `reset()`. | Per-day normalization statistics: compute mean/std per feature across all bins in a day for z-score normalization. Also used for running trade size statistics within a bin. |
| `StreamingDistribution` | `statistics::streaming_dist` | Streaming quantile estimation (P2 algorithm) with configurable quantile targets. Provides p25/p50/p75/p90, skewness, kurtosis. | Trade size distribution features: `size_concentration`, percentile-based block detection thresholds. |
| `RegimeClassifier` | `time::regime` | 7-regime intraday classification (pre-market, open-auction, morning, midday, afternoon, close-auction, post-market) with exact DST boundaries (2nd Sunday March, 1st Sunday November). | Optional `time_regime` feature if needed for downstream regime-conditioned analysis. Also used to validate `session_progress` computation. |
| `utc_offset_for_date(y, m, d) -> i32` | `time::regime` | Returns -4 (EDT) or -5 (EST) for a given date using exact DST rules (2nd Sunday March, 1st Sunday November). | Market hours boundary computation: converting `market_open_et`/`market_close_et` from ET to UTC for bin grid alignment. Computed once per day at pipeline init. |
| `day_epoch_ns(y, m, d, offset) -> i64` | `time::regime` | UTC nanosecond timestamp of midnight ET for a given date and UTC offset. | Bin grid start computation: `market_open_ns = day_epoch_ns + 9*3600*1e9 + 30*60*1e9`. |
| `time_regime(ts_ns, offset) -> u8` | `time::regime` | 7-regime intraday classifier. Takes UTC nanoseconds + offset, returns regime 0-6. | Used for `time_bucket` feature (index 32). Also validates `session_progress` computation. |
| `AcfComputer` | `statistics::acf` | Streaming autocorrelation function estimation. Supports lag-k ACF for diagnostic purposes. | Not used in feature computation. Used in Python analysis scripts for signal persistence validation (reproducing E9 ACF results from Rust exports). |
| `Vpin` | `statistics::vpin` | Volume-synchronized probability of informed trading (Easley, Lopez de Prado & O'Hara 2012). Volume-bar BVC-based computation. | `trf_vpin` and `lit_vpin` features. The existing implementation handles volume-bar construction and BVC signing internally. |

### 3.3 New Primitives to Add to hft-statistics

One new primitive is needed in `hft-statistics` before basic-quote-processor development begins:

**Kyle's Lambda (Kyle 1985)**

```rust
// Proposed addition to hft-statistics/src/statistics/mod.rs

/// Rolling Kyle's lambda estimator.
///
/// Lambda = Cov(delta_price, signed_volume) / Var(signed_volume)
///
/// Measures permanent price impact per unit of signed order flow.
/// Higher lambda = less liquid, more information in order flow.
///
/// Reference: Kyle (1985) "Continuous Auctions and Insider Trading",
///            Econometrica 53(6), pp. 1315-1335.
///
/// # Parameters
/// - `window`: number of observations for rolling computation
///
/// # Properties
/// - Numerically stable: uses Welford-based covariance
/// - Bounded memory: O(window) storage
/// - Unit: USD per share (price impact per signed volume unit)
pub struct KyleLambda {
    window: usize,
    delta_prices: VecDeque<f64>,
    signed_volumes: VecDeque<f64>,
    cov_accumulator: WelfordCovariance,  // or rolling covariance
}

impl KyleLambda {
    pub fn new(window: usize) -> Self { ... }

    /// Update with a new (delta_price, signed_volume) observation.
    pub fn update(&mut self, delta_price: f64, signed_volume: f64) { ... }

    /// Current lambda estimate, or None if insufficient data.
    pub fn lambda(&self) -> Option<f64> { ... }

    /// Reset all state.
    pub fn reset(&mut self) { ... }
}
```

**Why in hft-statistics rather than basic-quote-processor**: Kyle's lambda is a general microstructure primitive applicable to any venue or instrument. It depends only on (price_change, signed_volume) pairs, with no CMBP-1 or LOB specifics. Placing it in `hft-statistics` makes it available for future use in `mbo-statistical-profiler` or other consumers.

**Implementation note**: The rolling covariance computation should reuse `WelfordAccumulator` internals or implement an equivalent two-variable Welford recurrence to avoid catastrophic cancellation in the `Cov(X,Y) = E[XY] - E[X]E[Y]` naive formula.

---

## 4. Integration with pipeline_contract.toml

> **Standalone repo note**: In the standalone `basic-quote-processor` repository, `src/contract.rs` is the authoritative constant source. The `pipeline_contract.toml` and verification scripts are maintained in the parent HFT-pipeline-v2 repository for cross-pipeline consistency.

### 4.1 New TOML Section: `[features.off_exchange]`

The following section is added to `contracts/pipeline_contract.toml`. It defines the off-exchange feature index space, which is **independent** from the MBO feature indices (0-147). The off-exchange features occupy their own index space starting at 0.

```toml
# =============================================================================
# Off-Exchange Features (34) — Indices 0-33 (independent index space)
# =============================================================================
# Source: basic-quote-processor/src/contract.rs
# Data: XNAS.BASIC CMBP-1 (consolidated L1 quotes + trade prints)
# These features are NOT extensions of the MBO 148-feature space.
# Fusion (concatenation) happens downstream in the trainer.

[features.off_exchange]
schema_version = "1.0"
total_count = 34
active_feature_count = 30    # Excluding 2 safety gates + 2 context features for model input

[features.off_exchange.signed_flow]
start = 0
count = 4
features = ["trf_signed_imbalance", "mroib", "inv_inst_direction", "bvc_imbalance"]

[features.off_exchange.venue_metrics]
start = 4
count = 4
features = ["dark_share", "trf_volume", "lit_volume", "total_volume"]

[features.off_exchange.retail_metrics]
start = 8
count = 4
features = ["subpenny_intensity", "odd_lot_ratio", "retail_trade_rate", "retail_volume_fraction"]

[features.off_exchange.bbo_dynamics]
start = 12
count = 6
features = ["spread_bps", "bid_pressure", "ask_pressure", "bbo_update_rate", "quote_imbalance", "spread_change_rate"]

[features.off_exchange.vpin]
start = 18
count = 2
features = ["trf_vpin", "lit_vpin"]

[features.off_exchange.trade_size]
start = 20
count = 4
features = ["mean_trade_size", "block_trade_ratio", "trade_count", "size_concentration"]

[features.off_exchange.cross_venue]
start = 24
count = 3
features = ["trf_burst_intensity", "time_since_burst", "trf_lit_volume_ratio"]

[features.off_exchange.activity]
start = 27
count = 2
features = ["bin_trade_count", "bin_trf_trade_count"]

[features.off_exchange.safety_gates]
start = 29
count = 2
features = ["bin_valid", "bbo_valid"]

[features.off_exchange.context]
start = 31
count = 3
features = ["session_progress", "time_bucket", "schema_version"]

[features.off_exchange.categorical]
indices = [29, 30, 32, 33]
names = ["bin_valid", "bbo_valid", "time_bucket", "schema_version"]

[features.off_exchange.non_normalizable]
indices = [29, 30, 32, 33]
names = ["bin_valid", "bbo_valid", "time_bucket", "schema_version"]

[features.off_exchange.unsigned]
indices = [4, 5, 6, 7, 8, 9, 10, 11, 18, 19, 20, 22, 23, 24, 25, 26, 27, 28]
names = [
    "dark_share", "trf_volume", "lit_volume", "total_volume",
    "subpenny_intensity", "odd_lot_ratio", "retail_trade_rate", "retail_volume_fraction",
    "trf_vpin", "lit_vpin",
    "mean_trade_size", "trade_count", "size_concentration",
    "trf_burst_intensity", "time_since_burst", "trf_lit_volume_ratio",
    "bin_trade_count", "bin_trf_trade_count",
]

[features.off_exchange.sign_convention]
bullish_positive = true
```

### 4.2 Extension of verify_rust_constants.py

The existing `verify_rust_constants.py` validates MBO pipeline Rust constants against the TOML. It must be extended to also validate basic-quote-processor constants.

**Changes required**:

1. **Add a new Rust source path**:

```python
# In verify_rust_constants.py

BASIC_QUOTE_SRC = REPO_ROOT / "basic-quote-processor" / "src"
BASIC_QUOTE_CONTRACT_RS = BASIC_QUOTE_SRC / "contract.rs"
```

2. **Add off-exchange verification function**:

```python
def verify_off_exchange(contract: dict) -> None:
    """Verify basic-quote-processor/src/contract.rs against [features.off_exchange]."""
    offex = contract.get("features", {}).get("off_exchange")
    if offex is None:
        print("SKIP: [features.off_exchange] not in contract, skipping basic-quote-processor")
        return

    if not BASIC_QUOTE_CONTRACT_RS.exists():
        print(f"SKIP: {BASIC_QUOTE_CONTRACT_RS} not found (basic-quote-processor not built yet)")
        return

    text = read_file(BASIC_QUOTE_CONTRACT_RS)

    # Verify schema version
    check_string_const(text, "SCHEMA_VERSION", offex["schema_version"], "contract.rs")

    # Verify total count
    check_int_const(text, "TOTAL_FEATURE_COUNT", offex["total_count"], "contract.rs")

    # Verify per-group start indices and counts
    for group_name in ["signed_flow", "venue_metrics", "retail_metrics", "bbo_dynamics",
                       "vpin", "trade_size", "cross_venue", "activity",
                       "safety_gates", "context"]:
        group = offex.get(group_name)
        if group is None:
            continue
        upper = group_name.upper()
        check_int_const(text, f"{upper}_START", group["start"], "contract.rs")
        check_int_const(text, f"{upper}_COUNT", group["count"], "contract.rs")
```

3. **Call the new function from main**:

```python
def main():
    contract = load_contract()
    verify_mbo(contract)          # Existing
    verify_off_exchange(contract)  # New
    report_results()
```

**Behavior when basic-quote-processor does not exist yet**: The script skips off-exchange verification with a `SKIP` message if `basic-quote-processor/src/contract.rs` is not found. This allows the contract TOML section to be added before the Rust crate exists.

### 4.3 Extension of generate_python_contract.py

The existing `generate_python_contract.py` produces `hft-contracts/_generated.py` with `FeatureIndex`, `ExperimentalFeatureIndex`, and `SignalIndex` enums. It must be extended to also produce `OffExchangeFeatureIndex`.

**Changes required**:

1. **Add a new collection function**:

```python
def _collect_off_exchange(contract: dict) -> list[tuple[str, int, str]] | None:
    """Collect off-exchange features as (UPPER_NAME, index, group_name).

    Returns None if [features.off_exchange] is not present in the contract.
    """
    offex = contract.get("features", {}).get("off_exchange")
    if offex is None:
        return None

    entries: list[tuple[str, int, str]] = []
    for group_name in ["signed_flow", "venue_metrics", "retail_metrics", "bbo_dynamics",
                       "vpin", "trade_size", "cross_venue", "activity",
                       "safety_gates", "context"]:
        group = offex.get(group_name)
        if group is None or "features" not in group:
            continue
        start = group["start"]
        for i, feat_name in enumerate(group["features"]):
            entries.append((_upper(feat_name), start + i, group_name))

    entries.sort(key=lambda x: x[1])
    return entries
```

2. **Generate the `OffExchangeFeatureIndex` enum in the output**:

```python
# In generate() function, after ExperimentalFeatureIndex generation:

offex = _collect_off_exchange(contract)
if offex is not None:
    offex_section = contract["features"]["off_exchange"]
    offex_schema = offex_section.get("schema_version", "1.0")
    offex_total = offex_section.get("total_count", len(offex))

    W("# " + "=" * 69)
    W(f"# OffExchangeFeatureIndex Enum ({offex_total} features)")
    W("# " + "=" * 69)
    W("")
    W("")
    W("class OffExchangeFeatureIndex(IntEnum):")
    W('    """')
    W(f"    Off-exchange feature indices (0-{offex_total - 1}).")
    W("")
    W("    These indices are in an INDEPENDENT index space from FeatureIndex.")
    W("    After fusion, off-exchange features are appended after MBO features.")
    W("    Source: basic-quote-processor (XNAS.BASIC CMBP-1 data).")
    W(f"    Schema version: {offex_schema}")
    W('    """')
    W("")
    for name, idx, _ in offex:
        W(f"    {name} = {idx}")
    W("")
    W("")

    # Off-exchange feature counts
    W(f"OFF_EXCHANGE_FEATURE_COUNT: Final[int] = {offex_total}")
    W('"""Total off-exchange features (all groups)."""')
    W("")
    W(f'OFF_EXCHANGE_SCHEMA_VERSION: Final[str] = "{offex_schema}"')
    W('"""Schema version for the off-exchange feature export format."""')
    W("")

    # Off-exchange group slices
    for group_name in ["signed_flow", "venue_metrics", "retail_metrics", "bbo_dynamics",
                       "vpin", "trade_size", "cross_venue", "activity",
                       "safety_gates", "context"]:
        group = offex_section.get(group_name)
        if group is None:
            continue
        upper = _upper(group_name)
        s = group["start"]
        c = group["count"]
        W(f"OFF_EXCHANGE_{upper}_SLICE = slice({s}, {s + c})")
    W("")

    # Off-exchange non-normalizable and categorical sets
    offex_cat = offex_section.get("categorical", {})
    offex_non_norm = offex_section.get("non_normalizable", {})
    if offex_cat.get("indices"):
        W(f"OFF_EXCHANGE_CATEGORICAL_INDICES: Final[frozenset[int]] = frozenset({{{', '.join(str(i) for i in sorted(offex_cat['indices']))}}})")
        W('"""Off-exchange categorical indices. Must NOT be normalized."""')
        W("")
    if offex_non_norm.get("indices"):
        W(f"OFF_EXCHANGE_NON_NORMALIZABLE_INDICES: Final[frozenset[int]] = frozenset({{{', '.join(str(i) for i in sorted(offex_non_norm['indices']))}}})")
        W('"""Off-exchange non-normalizable indices."""')
        W("")
```

### 4.4 Contract Registration Checklist

When adding off-exchange features to the pipeline contract, follow this checklist:

- [ ] Add `[features.off_exchange]` section to `contracts/pipeline_contract.toml`
- [ ] Add a `[[changelog]]` entry with version description
- [ ] Run `python contracts/generate_python_contract.py` to regenerate `_generated.py`
- [ ] Run `python contracts/generate_python_contract.py --check` to verify
- [ ] Verify `hft-contracts/_generated.py` contains `OffExchangeFeatureIndex` enum
- [ ] Create `basic-quote-processor/src/contract.rs` with matching constants
- [ ] Run `python contracts/verify_rust_constants.py` to validate Rust constants
- [ ] Update `hft-contracts` tests to cover `OffExchangeFeatureIndex`
- [ ] Update root `CLAUDE.md` feature index layout section with off-exchange reference (parent HFT-pipeline-v2 repo)
- [ ] Verify `pip install -e .` in `hft-contracts/` still passes all tests

**Contract version note**: The off-exchange feature space has its own `schema_version` (starting at `"1.0"`), independent of the MBO `schema_version` (`"2.2"`). This allows the two feature spaces to evolve independently. The pipeline-level `contract.schema_version` (`"2.2"`) does NOT need to bump for off-exchange changes -- only for MBO stable feature changes.

---

## 5. Export Compatibility

### 5.1 NPY File Format Table

| File | Shape | Dtype | Unit | Description |
|------|-------|-------|------|-------------|
| `{day}_sequences.npy` | `[N, T, F]` | `float32` | normalized | Feature sequences. T = `window_size` (default 20). F = enabled features (up to 34). Normalized per-day z-score by default. |
| `{day}_labels.npy` | `[N, H]` | `float64` | basis points | Point-to-point returns at each configured horizon. H = number of horizons (default 8: [1,2,3,5,10,20,30,60] bins). |
| `{day}_forward_prices.npy` | `[N, max_H+1]` | `float64` | USD | Mid-price trajectory. Column 0 = base mid-price at time t. Columns 1..max_H = mid-price at t+1..t+max_H bins. |
| `{day}_metadata.json` | -- | JSON | -- | Schema version, provenance, feature count, alignment metadata. |
| `{day}_normalization.json` | -- | JSON | -- | Per-day per-feature mean/std for z-score normalization. |
| `dataset_manifest.json` | -- | JSON | -- | Split information, configuration, date lists, experiment metadata. |

**Downcast boundary**: All feature computation occurs in `f64`. Downcast to `f32` happens exactly once, at the NPY export boundary. Before downcast, every value is checked with `is_finite()`. Any non-finite value triggers an error (not silent clamping).

### 5.2 Metadata JSON Required Fields

Each `{day}_metadata.json` must contain the following fields. These mirror the MBO pipeline metadata contract but include off-exchange-specific fields.

```json
{
    "day": "2025-02-03",
    "n_sequences": 370,
    "window_size": 20,
    "n_features": 34,
    "schema_version": "1.0",
    "contract_version": "1.0",
    "label_strategy": "point_return",
    "label_encoding": "continuous_bps",
    "normalization": {
        "strategy": "per_day_zscore",
        "applied": false,
        "params_file": "2025-02-03_normalization.json"
    },
    "provenance": {
        "extractor": "basic-quote-processor",
        "extractor_version": "0.1.0",
        "git_commit": "abc1234",
        "git_dirty": false,
        "config_hash": "sha256:...",
        "contract_version": "1.0",
        "export_timestamp_utc": "2026-03-22T15:30:00Z"
    },
    "export_timestamp": "2026-03-22T15:30:00Z",
    "horizons": [1, 2, 3, 5, 10, 20, 30, 60],
    "bin_size_seconds": 60,
    "market_open_et": "09:30",
    "first_bin_start_ns": 1738594200000000000,
    "last_bin_end_ns": 1738617600000000000,
    "total_bins": 390,
    "valid_bins": 387,
    "n_trf_trades": 12543,
    "n_lit_trades": 28901,
    "equs_summary_available": true,
    "consolidated_volume": 45000000,
    "feature_groups_enabled": {
        "signed_flow": true,
        "venue_metrics": true,
        "retail_metrics": true,
        "bbo_dynamics": true,
        "vpin": false,
        "trade_size": true,
        "cross_venue": true,
        "activity": true,
        "safety_gates": true,
        "context": true
    },
    "classification_config": {
        "signing_method": "midpoint",
        "exclusion_band": 0.10,
        "bjzz_enabled": true
    },
    "data_source": "XNAS.BASIC",
    "schema": "cmbp-1",
    "symbol": "NVDA"
}
```

**Required fields** (validation fails if any missing):

| Field | Type | Description |
|-------|------|-------------|
| `day` | string | ISO date `YYYY-MM-DD` |
| `n_sequences` | int | Number of sequences in the file |
| `window_size` | int | Bins per sequence |
| `n_features` | int | Features per bin |
| `schema_version` | string | Off-exchange schema version |
| `contract_version` | string | Contract version |
| `label_strategy` | string | Always `"point_return"` for this module |
| `label_encoding` | string | Always `"continuous_bps"` |
| `normalization` | object | Strategy, applied flag, params file path |
| `provenance` | object | Extractor, version, git, config hash, timestamps |
| `export_timestamp` | string | ISO 8601 UTC timestamp |
| `bin_size_seconds` | int | Time bin size for alignment validation |
| `market_open_et` | string | Market open time (ET) for alignment validation |

**Additional fields** (informational, not validated at load):

| Field | Type | Description |
|-------|------|-------------|
| `first_bin_start_ns` | int | UTC nanosecond timestamp of first bin start |
| `last_bin_end_ns` | int | UTC nanosecond timestamp of last bin end |
| `total_bins` | int | Total bins produced for the day |
| `valid_bins` | int | Bins with `bin_valid == 1.0` |
| `n_trf_trades` | int | Total TRF trades processed |
| `n_lit_trades` | int | Total lit trades processed |
| `equs_summary_available` | bool | Whether EQUS_SUMMARY was loaded |
| `consolidated_volume` | int or null | Daily consolidated volume from EQUS_SUMMARY |
| `feature_groups_enabled` | object | Which feature groups are active |
| `classification_config` | object | Trade signing parameters |
| `horizons` | list[int] | Horizon values in bins |
| `data_source` | string | Always `"XNAS.BASIC"` |
| `schema` | string | Always `"cmbp-1"` |
| `symbol` | string | Instrument symbol |

### 5.3 Normalization JSON Format

Each `{day}_normalization.json` contains per-feature statistics for z-score normalization.

```json
{
    "strategy": "per_day_zscore",
    "day": "2025-02-03",
    "sample_count": 387,
    "features": [
        {
            "index": 0,
            "name": "trf_signed_imbalance",
            "mean": -0.0234,
            "std": 0.4521,
            "min": -1.0,
            "max": 1.0,
            "n_finite": 387,
            "n_nan": 0,
            "normalizable": true
        },
        {
            "index": 1,
            "name": "mroib",
            "mean": 0.0012,
            "std": 0.0893,
            "min": -0.312,
            "max": 0.289,
            "n_finite": 387,
            "n_nan": 0,
            "normalizable": true
        },
        ...
        {
            "index": 29,
            "name": "bin_valid",
            "mean": 0.992,
            "std": 0.089,
            "min": 0.0,
            "max": 1.0,
            "n_finite": 387,
            "n_nan": 0,
            "normalizable": false
        },
        ...
    ]
}
```

**Key properties**:
- One entry per feature, ordered by index
- `normalizable` is `false` for indices in `non_normalizable` set: [29, 30, 32, 33] (`bin_valid`, `bbo_valid`, `time_bucket`, `schema_version`)
- When `std < EPS` (1e-8), the feature is normalized to 1.0 (not 0.0) to preserve the signal that "this feature is constant"
- Statistics are computed from ALL bins in the day, including invalid bins (the safety gates themselves need stats for normalization)
- `n_nan` should always be 0 for a valid export (NaN guard is applied before stats computation)

**Normalization application**: By default, the exporter writes **raw** (unnormalized) NPY files and saves the normalization statistics separately. The training pipeline applies normalization at load time using the saved statistics. This is consistent with the MBO pipeline behavior (`raw_export_default = true` in the pipeline contract).

### 5.4 Dataset Manifest Format

The `dataset_manifest.json` at the root of the export directory describes the full dataset.

```json
{
    "experiment": "basic_nvda_60s",
    "symbol": "NVDA",
    "data_source": "XNAS.BASIC",
    "schema": "cmbp-1",
    "feature_count": 34,
    "days_processed": 233,
    "export_timestamp": "2026-03-22T15:30:00Z",
    "config_hash": "sha256:...",
    "schema_version": "1.0",
    "sequence_length": 20,
    "stride": 1,
    "labeling_strategy": "point_return",
    "horizons": [1, 2, 3, 5, 10, 20, 30, 60],
    "bin_size_seconds": 60,
    "market_open_et": "09:30",
    "splits": {
        "train": {
            "days": ["2025-02-03", "2025-02-04", "..."],
            "n_days": 163,
            "n_sequences": 60000,
            "date_range": ["2025-02-03", "2025-09-30"]
        },
        "val": {
            "days": ["2025-10-01", "..."],
            "n_days": 35,
            "n_sequences": 13000,
            "date_range": ["2025-10-01", "2025-11-13"]
        },
        "test": {
            "days": ["2025-11-14", "..."],
            "n_days": 35,
            "n_sequences": 13000,
            "date_range": ["2025-11-14", "2026-01-08"]
        }
    },
    "feature_groups_enabled": {
        "signed_flow": true,
        "venue_metrics": true,
        "retail_metrics": true,
        "bbo_dynamics": true,
        "vpin": false,
        "trade_size": true,
        "cross_venue": true,
        "activity": true,
        "safety_gates": true,
        "context": true
    },
    "normalization": {
        "strategy": "per_day_zscore",
        "applied": false
    },
    "classification_config": {
        "signing_method": "midpoint",
        "exclusion_band": 0.10,
        "bjzz_enabled": true
    },
    "total_trf_trades": 2900000,
    "total_lit_trades": 6700000,
    "mean_bins_per_day": 387,
    "mean_valid_bins_per_day": 383
}
```

**Required fields** (subset that must always be present, matching MBO manifest contract plus off-exchange additions):

| Field | Type | Description |
|-------|------|-------------|
| `experiment` | string | Experiment name (directory name) |
| `symbol` | string | Instrument symbol |
| `feature_count` | int | Features per bin |
| `days_processed` | int | Total days in all splits |
| `export_timestamp` | string | ISO 8601 UTC |
| `config_hash` | string | SHA-256 of config TOML |
| `schema_version` | string | Off-exchange schema version |
| `sequence_length` | int | Window size (bins per sequence) |
| `stride` | int | Sliding window stride |
| `labeling_strategy` | string | Always `"point_return"` |
| `horizons` | list[int] | Horizon values in bins |
| `splits` | object | Train/val/test day lists and sequence counts |

---

## 6. Sign Convention

All directional features follow the pipeline-wide sign convention defined in `src/contract.rs` (mirrored in the parent pipeline's `contracts/pipeline_contract.toml`):

```
> 0 = Bullish / Buy pressure
< 0 = Bearish / Sell pressure
= 0 = Neutral / No signal
```

### 6.1 Signed Features (Follow Convention)

These features carry directional information. Positive values indicate buy-side pressure or bullish conditions.

| Index | Feature | Positive Means | Negative Means | Range |
|-------|---------|---------------|----------------|-------|
| 0 | `trf_signed_imbalance` | Net buying in TRF trades | Net selling in TRF trades | [-1, +1] |
| 1 | `mroib` | Off-exchange retail buy pressure exceeds sell | Off-exchange retail sell pressure exceeds buy | [-1, +1] |
| 2 | `inv_inst_direction` | Institutional flow is net buying (inverted from raw) | Institutional flow is net selling | [-1, +1] |
| 3 | `bvc_imbalance` | BVC classifies net buying | BVC classifies net selling | [-1, +1] |
| 10 | `retail_trade_rate` | Higher retail activity (bullish lean in NVDA retail) | Lower retail activity | [0, 1] |
| 11 | `retail_volume_fraction` | Higher retail fraction | Lower retail fraction | [0, 1] |
| 13 | `bid_pressure` | More aggressive bid-side quoting | Less aggressive bid-side quoting | [0, +inf) |
| 14 | `ask_pressure` | More aggressive ask-side quoting | Less aggressive ask-side quoting | [0, +inf) |
| 16 | `quote_imbalance` | Bid side stronger than ask | Ask side stronger than bid | [-1, +1] |
| 17 | `spread_change_rate` | Spread widening (bearish) | Spread narrowing (bullish) | (-inf, +inf) |

**Note on `spread_change_rate`**: Positive spread change = widening = typically bearish (liquidity withdrawal). This follows the convention that "positive = bullish" when interpreted as: "the market is getting tighter" (negative change rate). The raw feature is `(spread_now - spread_prev) / spread_prev`, so a positive value indicates deteriorating conditions. Downstream consumers should be aware of this inversion.

**Note on `retail_trade_rate` and `retail_volume_fraction`**: These are strictly non-negative [0, 1] ratios. They carry a weak bullish lean because NVDA retail flow is empirically buy-biased, but they are not strongly directional. They are included here because higher retail activity has a statistical association with positive NVDA returns (E9 finding).

### 6.2 Unsigned Features (No Directional Convention)

These features measure magnitude, rate, or proportion without directional meaning. They are unsigned by construction.

| Index | Feature | Unit | Range | Description |
|-------|---------|------|-------|-------------|
| 4 | `dark_share` | ratio | [0, 1] | Fraction of volume executed off-exchange |
| 5 | `trf_volume` | shares | [0, +inf) | Total TRF-reported volume in bin |
| 6 | `lit_volume` | shares | [0, +inf) | Total lit (XNAS+minor) volume in bin |
| 7 | `total_volume` | shares | [0, +inf) | trf_volume + lit_volume |
| 8 | `subpenny_intensity` | ratio | [0, 1] | Fraction of trades at subpenny increments |
| 9 | `odd_lot_ratio` | ratio | [0, 1] | Fraction of trades with odd lot sizes |
| 12 | `spread_bps` | bps | [0, +inf) | Current bid-ask spread |
| 15 | `bbo_update_rate` | updates/sec | [0, +inf) | Rate of BBO quote updates |
| 18 | `trf_vpin` | probability | [0, 1] | VPIN for TRF flow |
| 19 | `lit_vpin` | probability | [0, 1] | VPIN for lit flow |
| 20 | `mean_trade_size` | shares | [0, +inf) | Average trade size in bin |
| 21 | `block_trade_ratio` | ratio | [0, 1] | Fraction of volume in block-sized trades |
| 22 | `trade_count` | count | [0, +inf) | Total trades (all venues) in bin |
| 23 | `size_concentration` | ratio | [0, 1] | Herfindahl-type concentration of trade sizes |
| 24 | `trf_burst_intensity` | ratio | [0, +inf) | Intensity of clustered TRF activity |
| 25 | `time_since_burst` | seconds | [0, bin_size] | Time since last TRF burst |
| 26 | `trf_lit_volume_ratio` | ratio | [0, +inf) | TRF volume / lit volume |
| 27 | `bin_trade_count` | count | [0, +inf) | Total trades in bin (all venues) |
| 28 | `bin_trf_trade_count` | count | [0, +inf) | TRF trades in bin |

### 6.3 Safety Gates and Context Features

These features are categorical or metadata. They are never normalized and carry no directional meaning.

| Index | Feature | Type | Values | Description |
|-------|---------|------|--------|-------------|
| 29 | `bin_valid` | binary | {0.0, 1.0} | 1.0 if `n_trf_trades >= min_trades_per_bin`, else 0.0 |
| 30 | `bbo_valid` | binary | {0.0, 1.0} | 1.0 if BBO was updated within staleness threshold, else 0.0 |
| 31 | `session_progress` | ratio | [0.0, 1.0] | Fraction of trading session elapsed (0.0 at open, 1.0 at close) |
| 32 | `time_bucket` | categorical | {0, 1, 2, ...} | Intraday time bucket (e.g., 15-minute bins mapped to integers) |
| 33 | `schema_version` | constant | 1.0 | Off-exchange schema version, emitted in every feature vector |

### 6.4 Sign Convention Enforcement

Sign convention is enforced through:

1. **Contract definition**: `[features.off_exchange.sign_convention] bullish_positive = true` in pipeline_contract.toml
2. **Rust contract constants**: `pub const SIGN_CONVENTION_BULLISH_POSITIVE: bool = true;` in `contract.rs`
3. **Formula tests**: Each signed feature has a hand-calculated test verifying the sign matches the convention (e.g., net buy flow produces positive `trf_signed_imbalance`)
4. **Integration tests**: End-to-end tests with known input data verify that directional features have the expected sign
5. **Python validation**: The `OffExchangeFeatureIndex` enum docstring documents the sign convention, and downstream consumers reference it

---

## Appendix A: Alignment Validation Pseudocode

This pseudocode shows how a downstream fusion layer validates alignment between MBO and off-exchange exports before merging.

```python
def validate_alignment(mbo_meta: dict, offex_meta: dict) -> list[str]:
    """
    Validate that MBO and off-exchange metadata are compatible for fusion.

    Returns list of error strings (empty = valid).
    """
    errors = []

    # Exact match required
    if mbo_meta["bin_size_seconds"] != offex_meta["bin_size_seconds"]:
        errors.append(
            f"bin_size mismatch: MBO={mbo_meta['bin_size_seconds']}, "
            f"offex={offex_meta['bin_size_seconds']}"
        )

    if mbo_meta["market_open_et"] != offex_meta["market_open_et"]:
        errors.append(
            f"market_open mismatch: MBO={mbo_meta['market_open_et']}, "
            f"offex={offex_meta['market_open_et']}"
        )

    if mbo_meta["date"] != offex_meta["date"]:
        errors.append(
            f"date mismatch: MBO={mbo_meta['date']}, "
            f"offex={offex_meta['date']}"
        )

    if mbo_meta.get("window_size") != offex_meta.get("window_size"):
        errors.append(
            f"window_size mismatch: MBO={mbo_meta.get('window_size')}, "
            f"offex={offex_meta.get('window_size')}"
        )

    # Temporal overlap check
    mbo_start = mbo_meta.get("first_bin_start_ns", 0)
    offex_start = offex_meta.get("first_bin_start_ns", 0)
    bin_ns = mbo_meta["bin_size_seconds"] * 1_000_000_000

    if abs(mbo_start - offex_start) > bin_ns:
        errors.append(
            f"first_bin_start_ns differs by more than one bin: "
            f"MBO={mbo_start}, offex={offex_start}, bin_ns={bin_ns}"
        )

    # Sequence count divergence check
    n_mbo = mbo_meta.get("n_sequences", 0)
    n_offex = offex_meta.get("n_sequences", 0)
    if n_mbo > 0 and n_offex > 0:
        max_n = max(n_mbo, n_offex)
        divergence = abs(n_mbo - n_offex) / max_n
        if divergence > 0.10:
            errors.append(
                f"sequence count divergence {divergence:.1%} exceeds 10%: "
                f"MBO={n_mbo}, offex={n_offex}"
            )

    return errors
```

## Appendix B: Rust contract.rs Template

The Rust contract file (`basic-quote-processor/src/contract.rs`) mirrors the TOML definitions and is verified by `verify_rust_constants.py`.

```rust
//! Off-exchange feature contract constants.
//!
//! These values MUST match contracts/pipeline_contract.toml [features.off_exchange].
//! Verified by: python contracts/verify_rust_constants.py

/// Schema version for off-exchange feature exports.
pub const SCHEMA_VERSION: &str = "1.0";

/// Numerical precision guard for division denominators.
pub const EPS: f64 = 1e-8;

/// Total number of off-exchange features.
pub const TOTAL_FEATURE_COUNT: usize = 34;

// -- Feature Group Boundaries --

pub const SIGNED_FLOW_START: usize = 0;
pub const SIGNED_FLOW_COUNT: usize = 4;

pub const VENUE_METRICS_START: usize = 4;
pub const VENUE_METRICS_COUNT: usize = 4;

pub const RETAIL_METRICS_START: usize = 8;
pub const RETAIL_METRICS_COUNT: usize = 4;

pub const BBO_DYNAMICS_START: usize = 12;
pub const BBO_DYNAMICS_COUNT: usize = 6;

pub const VPIN_START: usize = 18;
pub const VPIN_COUNT: usize = 2;

pub const TRADE_SIZE_START: usize = 20;
pub const TRADE_SIZE_COUNT: usize = 4;

pub const CROSS_VENUE_START: usize = 24;
pub const CROSS_VENUE_COUNT: usize = 3;

pub const ACTIVITY_START: usize = 27;
pub const ACTIVITY_COUNT: usize = 2;

pub const SAFETY_GATES_START: usize = 29;
pub const SAFETY_GATES_COUNT: usize = 2;

pub const CONTEXT_START: usize = 31;
pub const CONTEXT_COUNT: usize = 3;

// -- Named Feature Indices --

pub const TRF_SIGNED_IMBALANCE: usize = 0;
pub const MROIB: usize = 1;
pub const INV_INST_DIRECTION: usize = 2;
pub const BVC_IMBALANCE: usize = 3;

pub const DARK_SHARE: usize = 4;
pub const TRF_VOLUME: usize = 5;
pub const LIT_VOLUME: usize = 6;
pub const TOTAL_VOLUME: usize = 7;

pub const SUBPENNY_INTENSITY: usize = 8;
pub const ODD_LOT_RATIO: usize = 9;
pub const RETAIL_TRADE_RATE: usize = 10;
pub const RETAIL_VOLUME_FRACTION: usize = 11;

pub const SPREAD_BPS: usize = 12;
pub const BID_PRESSURE: usize = 13;
pub const ASK_PRESSURE: usize = 14;
pub const BBO_UPDATE_RATE: usize = 15;
pub const QUOTE_IMBALANCE: usize = 16;
pub const SPREAD_CHANGE_RATE: usize = 17;

pub const TRF_VPIN: usize = 18;
pub const LIT_VPIN: usize = 19;

pub const MEAN_TRADE_SIZE: usize = 20;
pub const BLOCK_TRADE_RATIO: usize = 21;
pub const TRADE_COUNT: usize = 22;
pub const SIZE_CONCENTRATION: usize = 23;

pub const TRF_BURST_INTENSITY: usize = 24;
pub const TIME_SINCE_BURST: usize = 25;
pub const TRF_LIT_VOLUME_RATIO: usize = 26;

pub const BIN_TRADE_COUNT: usize = 27;
pub const BIN_TRF_TRADE_COUNT: usize = 28;

pub const BIN_VALID: usize = 29;
pub const BBO_VALID: usize = 30;

pub const SESSION_PROGRESS: usize = 31;
pub const TIME_BUCKET: usize = 32;
pub const SCHEMA_VERSION_INDEX: usize = 33;

// -- Feature Classification --

/// Categorical feature indices (never normalized).
pub const CATEGORICAL_INDICES: &[usize] = &[29, 30, 32, 33];

/// Non-normalizable feature indices (superset of categorical).
pub const NON_NORMALIZABLE_INDICES: &[usize] = &[29, 30, 32, 33];

/// Sign convention: > 0 = Bullish, < 0 = Bearish, = 0 = Neutral.
pub const SIGN_CONVENTION_BULLISH_POSITIVE: bool = true;
```
