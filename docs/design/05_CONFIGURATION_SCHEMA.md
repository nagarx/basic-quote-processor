# Configuration Schema: basic-quote-processor TOML Reference

**Status**: Reference Document — **Implementation Status**: Phases 1-5 complete (412 tests)
**Date**: 2026-03-22 (spec), 2026-03-23 (implementation aligned)
**Scope**: Complete TOML configuration schema for the basic-quote-processor pipeline, including all parameters, validation rules, defaults, and rationale

---

## Implementation Status (Phase 5 Complete, 2026-04-13)

This schema documents the **complete intended design**. Several sections describe deferred or planned features. Always cross-reference against `src/config.rs` and `CODEBASE.md` "Known Limitations" for the operational reference of current capabilities.

| Section / Feature | Status | Notes |
|-------------------|--------|-------|
| `[input]` (data paths only — no dates) | **IMPLEMENTED** | `start_date`/`end_date` live in `[dates]`, not `[input]` |
| `[dates]` (`start_date`, `end_date`, `exclude_dates`) | **IMPLEMENTED** | Required for `DatasetConfig` (CLI); not in `ProcessorConfig` (library API) |
| `[sampling]` | **PARTIALLY IMPLEMENTED** | Only `strategy = "time_based"` is currently valid; `"volume_based"` is **PLANNED** |
| `[classification]` | **IMPLEMENTED** | `signing_method = "tick_test"` returns explicit error (reserved) |
| `[features]` | **IMPLEMENTED** | No `context` toggle — `activity`, `safety_gates`, `context` are always-on (not user-toggleable) |
| `[vpin]` | **PARTIALLY IMPLEMENTED** | `bucket_volume_fraction` (relative-to-daily) deferred; uses fixed `bucket_volume = 5000` |
| `[validation]` | **IMPLEMENTED** | Default `close_detection_gap_bins = 10` (spec text in §11 was wrong @ 5) |
| `[sequence]`, `[labeling]`, `[export.split_dates]` | **IMPLEMENTED** | -- |
| `[export]` | **PARTIALLY IMPLEMENTED** | Only `normalization = "per_day_zscore"` and `"none"` valid; `"global_zscore"` is **PLANNED**. Default is `"none"` (not `"per_day_zscore"`) |
| `[publishers]` | **DEFERRED** | Uses hardcoded `PublisherClass::from_id()` in `src/reader/publisher.rs` |
| Multi-config files (`nvda_10s.toml`, `nvda_vpin.toml`) | **EXAMPLE-ONLY** | Only `nvda_60s.toml` exists in `configs/`; the other examples illustrate the planned `[vpin]` and 10s schemas |

When this schema and the code disagree, **the code is authoritative**. Schema corrections are tracked here as drift is discovered.

---

## Table of Contents

1. [Design Principles](#1-design-principles)
2. [Complete TOML Structure](#2-complete-toml-structure)
3. [Section Reference: `[input]`](#3-section-reference-input)
4. [Section Reference: `[sampling]`](#4-section-reference-sampling)
5. [Section Reference: `[classification]`](#5-section-reference-classification)
6. [Section Reference: `[features]`](#6-section-reference-features)
7. [Section Reference: `[vpin]`](#7-section-reference-vpin)
8. [Section Reference: `[labeling]`](#8-section-reference-labeling)
9. [Section Reference: `[sequence]`](#9-section-reference-sequence)
10. [Section Reference: `[export]`](#10-section-reference-export)
11. [Section Reference: `[validation]`](#11-section-reference-validation)
12. [Section Reference: `[publishers]`](#12-section-reference-publishers)
13. [Non-Configurable Constants](#13-non-configurable-constants)
14. [Config Validation Rules](#14-config-validation-rules)
15. [Example Configs](#15-example-configs)
16. [Experiment Tracking Integration](#16-experiment-tracking-integration)

---

## 1. Design Principles

The configuration follows the pipeline's established design philosophy (see HFT pipeline convention (Rule 5: Configuration-Driven Design)):

- **Every behavioral parameter is configurable.** No magic numbers embedded in source code.
- **Sensible defaults with full override capability.** The default config (`nvda_60s.toml`) is production-ready without modification.
- **Fail-fast on invalid input.** Every parameter has defined valid ranges. The pipeline refuses to start with invalid config rather than silently degrading.
- **Serializable for experiment tracking.** Configs are TOML files that can be committed, diff'd, and referenced by experiment ID.
- **Citation-backed defaults.** Every default value either comes from a published paper (cited inline) or from validated empirical results (E9 cross-validation, 35 test days).

The config is parsed at pipeline startup by `src/config.rs` (implementing `serde::Deserialize`). All validation occurs at parse time before any data processing begins.

---

## 2. Complete TOML Structure

> **Note**: Paths in example configs reference the parent monorepo data layout (`../data/...`). Adjust `data_dir`, `equs_summary_path`, and `output_dir` to match your local data location.

```toml
# IMPLEMENTED sections (current code)

[input]
data_dir = "./data/XNAS_BASIC/NVDA/cmbp1"
equs_summary_path = "./data/EQUS_SUMMARY/NVDA/equs-summary.ohlcv-1d.dbn.zst"
filename_pattern = "xnas-basic-{date}.cmbp-1.dbn.zst"
symbol = "NVDA"

[dates]
start_date = "2025-02-03"
end_date = "2026-01-06"
exclude_dates = []                         # optional: holidays, halts to skip

[sampling]
strategy = "time_based"                    # only "time_based" currently valid
bin_size_seconds = 60                      # one of {5,10,15,30,60,120,300,600}
market_open_et = "09:30"
market_close_et = "16:00"

[classification]
signing_method = "midpoint"                # "tick_test" reserved (returns explicit error)
exclusion_band = 0.10
bjzz_lower = 0.001
bjzz_upper_sell = 0.40
bjzz_lower_buy = 0.60
bjzz_upper = 0.999

[features]
# Note: activity (27-28), safety_gates (29-30), context (31-33) are always-on
# (not toggleable). Only the seven optional groups appear here.
signed_flow = true
venue_metrics = true
retail_metrics = true
bbo_dynamics = true
vpin = false                               # default disabled (requires daily volume context)
trade_size = true
cross_venue = true

[vpin]
bucket_volume = 5000                       # absolute shares per bucket (default)
# bucket_volume_fraction = 0.02            # PLANNED: requires EQUS daily volume; not yet enforced
lookback_buckets = 50
sigma_window_minutes = 1

[labeling]
label_type = "point_return"                # only variant currently supported
horizons = [1, 2, 3, 5, 10, 20, 30, 60]

[sequence]
window_size = 20
stride = 1

[validation]
min_trades_per_bin = 10
bbo_staleness_max_ns = 5_000_000_000
warmup_bins = 3
block_threshold = 10_000
burst_threshold = 20
empty_bin_policy = "forward_fill_state"    # one of "forward_fill_state","zero_all","nan_all"
auto_detect_close = true
close_detection_gap_bins = 10              # default: 10 (10 minutes at 60s bins)

[export]
output_dir = "./data/exports/basic_nvda_60s"
experiment = "basic_nvda"
normalization = "none"                     # one of "per_day_zscore","none". Default: "none".
continue_on_error = true

[export.split_dates]
train_end = "2025-09-30"
val_end = "2025-11-13"

# DEFERRED sections (NOT in code yet — illustrative only)
#
# [publishers]                             # DEFERRED: uses hardcoded PublisherClass::from_id()
# trf = [82, 83]
# lit = [81]
# minor_lit = [88, 89]
# include_minor_lit_in_lit = true
```

---

## 3. Section Reference: `[input]`

Controls data source paths and ticker.

> **Note**: Date range parameters (`start_date`, `end_date`, `exclude_dates`) live in the separate `[dates]` section (see §3.1), NOT in `[input]`. `[dates]` is required by `DatasetConfig` (the multi-day CLI config used by `export_dataset`); the library-level `ProcessorConfig` operates on a single day at a time and does not need a date range.

| Parameter | Type | Default | Valid Range | Description |
|-----------|------|---------|-------------|-------------|
| `data_dir` | string | (required) | Valid directory path | Root directory containing XNAS.BASIC `.dbn.zst` files. One file per trading day. |
| `equs_summary_path` | Option<string> | `None` (omit field) | Path to a single `.dbn.zst` OHLCV-1D file | Optional EQUS_SUMMARY path. When omitted, the pipeline proceeds without consolidated volume context (AD2: spec says required, code makes optional for library usability). |
| `filename_pattern` | string | (required) | Must contain `{date}` placeholder | Filename template for per-day XNAS.BASIC files. The `{date}` token is replaced with `YYYYMMDD`. |
| `symbol` | string | `"NVDA"` | Non-empty string | Ticker symbol. Used for metadata and logging. |

### Rationale

- **`data_dir`**: Relative paths resolve from the working directory. Adjust to your local data layout.
- **`equs_summary_path`**: Optional. When present, provides per-day consolidated volume for `consolidated_volume` and `trf_volume_fraction` metadata fields. When absent, those fields are `null` in metadata; pipeline still produces all 34 features.
- **`filename_pattern`**: The `{date}` placeholder is replaced with `YYYYMMDD`. Alternative schemas (e.g., different naming for ARCX in the future) can be specified without code changes.

### Impact of Changes

| Change | Effect |
|--------|--------|
| Removing `equs_summary_path` | Pipeline proceeds without daily volume context (informational only — does not affect feature values). |
| Wrong `filename_pattern` | No files found. Pipeline exits with zero-day error at startup. |

### 3.1 Section Reference: `[dates]` (required for `DatasetConfig`)

| Parameter | Type | Default | Valid Range | Description |
|-----------|------|---------|-------------|-------------|
| `start_date` | string (date) | (required) | `YYYY-MM-DD`, must be a valid calendar date | First trading day to process (inclusive). |
| `end_date` | string (date) | (required) | `YYYY-MM-DD`, must be >= `start_date` | Last trading day to process (inclusive). |
| `exclude_dates` | array of string (date) | `[]` | Each `YYYY-MM-DD`, must parse as valid date | Dates to skip (holidays, halts). Optional. |

### Rationale

- **`start_date`/`end_date`**: The pipeline iterates calendar dates in this range. Missing files (weekends, holidays, gaps in data) produce a log entry and continue. This avoids requiring a separate trading calendar.
- **`exclude_dates`**: Explicit holiday handling. Listed dates are skipped without producing log entries.

---

## 4. Section Reference: `[sampling]`

Controls how raw records are aggregated into discrete bins for feature computation.

| Parameter | Type | Default | Valid Range | Description |
|-----------|------|---------|-------------|-------------|
| `strategy` | string (enum) | `"time_based"` | `"time_based"` (only) | Sampling strategy. **Currently only `"time_based"` is implemented**; the spec mentions `"volume_based"` for future use, but the code rejects it with `"Unknown sampling strategy"`. VPIN computation (when enabled) uses an internal volume-bar accumulator regardless of this setting. |
| `bin_size_seconds` | u32 | `60` | `{5, 10, 15, 30, 60, 120, 300, 600}` | Duration of each time bin in seconds. Only used when `strategy = "time_based"`. Bins are grid-aligned to `market_open_et`. |
| `market_open_et` | string (time) | `"09:30"` | `HH:MM` in ET, must be before `market_close_et` | Eastern Time market open. First bin starts at this time. Grid alignment ensures bins are comparable across days. |
| `market_close_et` | string (time) | `"16:00"` | `HH:MM` in ET, must be after `market_open_et` | Eastern Time market close. Records after this time are discarded (no post-market processing). The last bin may be shorter than `bin_size_seconds` if the session does not divide evenly. |

### Rationale

- **`strategy = "time_based"`**: The E5 experiment (IC=0.380 at 60s bins, the best tradeable result in the MBO pipeline) validated that time-based sampling preserves signal persistence (ACF), while the F1 experiment proved event-based sampling destroys it (ACF drops from 0.266 to 0.021). Time-based is the only supported strategy for feature export. `volume_based` is used internally for VPIN computation only.
- **`bin_size_seconds = 60`**: E5 validated 60s as the optimal bin size for NVDA. Larger bins (120s) reduce sample count without improving IC. Smaller bins (10s) increase noise. The 60s default balances signal quality with sample size (~390 bins per full trading day, ~370 usable after warmup).
- **`market_open_et = "09:30"`**: Nasdaq regular session open. Grid alignment means bins start at 09:30:00, 09:31:00, 09:32:00, etc. (at 60s bins). This ensures cross-day bin comparability.
- **`market_close_et = "16:00"`**: Nasdaq regular session close. Excludes pre-market and post-market sessions by default, consistent with the MBO pipeline's treatment of regular hours as the primary signal regime. See `auto_detect_close` in `[validation]` for early-close handling.

### Impact of Changes

| Change | Effect |
|--------|--------|
| `bin_size_seconds = 10` | ~2,340 bins/day (6x more). Higher temporal resolution but noisier features and more bins below `min_trades_per_bin`. Sequences represent 200s windows instead of 1200s. |
| `bin_size_seconds = 120` | ~195 bins/day (3.25x fewer). Smoother features but lower sample count. Sequences represent 2400s (40min) windows. |
| `strategy = "volume_based"` | **Not currently implemented.** Planned for future: bins would form when cumulative dollar volume reaches a threshold, producing variable-length time intervals. |
| Moving `market_open_et` to `"09:35"` | Skips first 5 minutes of trading. May avoid open-auction volatility but loses early-session signal. |

### Bin Count Formula

```
bins_per_day = floor((market_close - market_open) / bin_size_seconds)
usable_bins = bins_per_day - warmup_bins
sequences_per_day = usable_bins - window_size + 1    (with stride=1)

Example (defaults):
  bins_per_day = floor(23400 / 60) = 390
  usable_bins = 390 - 3 = 387
  sequences_per_day = 387 - 20 + 1 = 368
```

---

## 5. Section Reference: `[classification]`

Controls trade signing (buyer-initiated vs seller-initiated determination) and retail trade identification.

| Parameter | Type | Default | Valid Range | Description |
|-----------|------|---------|-------------|-------------|
| `signing_method` | string (enum) | `"midpoint"` | `"midpoint"`, `"tick_test"` | Algorithm for determining trade direction. `"midpoint"` = Barber et al. (2024), 94.8% accuracy (default). `"tick_test"` = Lee-Ready (1991), reserved — fails fast if selected. **Note**: BVC is NOT a signing method; it produces probabilistic volume splits for aggregate features (bvc_imbalance, VPIN) and is configured via `[vpin]`, not here. |
| `exclusion_band` | f64 | `0.10` | `[0.0, 0.50]` | Fraction of the BBO spread defining the unsigned zone around the midpoint. Trades within `mid +/- exclusion_band * spread` are classified as unsigned. Only used when `signing_method = "midpoint"`. |
| `bjzz_lower` | f64 | `0.001` | `(0.0, bjzz_upper_sell)` | Lower bound of the sub-penny fractional cent for retail identification. Trades with `frac_cent < bjzz_lower` are excluded (round-penny). |
| `bjzz_upper_sell` | f64 | `0.40` | `(bjzz_lower, 0.50)` | Upper bound of the sub-penny sell zone. Trades with `frac_cent in (bjzz_lower, bjzz_upper_sell)` are classified as retail sells (wholesaler provided price improvement above bid). |
| `bjzz_lower_buy` | f64 | `0.60` | `(0.50, bjzz_upper)` | Lower bound of the sub-penny buy zone. Trades with `frac_cent in (bjzz_lower_buy, bjzz_upper)` are classified as retail buys (wholesaler provided price improvement below ask). |
| `bjzz_upper` | f64 | `0.999` | `(bjzz_lower_buy, 1.0)` | Upper bound of the sub-penny buy zone. |

### Signing Methods

**`midpoint`** (default, recommended):
Barber, Huang, Jorion, Odean, and Schwarz (2024), "A (Sub)penny for Your Thoughts: Tracking Retail Investor Activity in TAQ," *The Journal of Finance*, 79(4), 2403-2427.

- Trade price > midpoint + exclusion_band * spread --> BUY
- Trade price < midpoint - exclusion_band * spread --> SELL
- Otherwise --> UNSIGNED
- Accuracy: 94.8% (validated on 85,000 known retail trades across 5 brokers)
- Key advantage: Uniform accuracy across all spread levels (unlike BJZZ sub-penny digit, which degrades from 93% at 1-penny spread to ~52% at 10+ cent spread)

**`tick_test`**:
Lee and Ready (1991), "Inferring Trade Direction from Intraday Data," *The Journal of Finance*, 46(2), 733-746.

- Trade price > last different price --> BUY (uptick)
- Trade price < last different price --> SELL (downtick)
- Equal price --> use previous classification
- Accuracy: ~68-79% on modern data (Chakrabarty, Moulton, and Shkilko 2012 report 31-32% misclassification on Nasdaq)
- Use case: Fallback when BBO is unavailable or stale

**`bvc`** (Bulk Volume Classification):
Easley, Lopez de Prado, and O'Hara (2012), "Flow Toxicity and Liquidity in a High-Frequency World," *Review of Financial Studies*, 25(5), 1457-1493.

- V_buy = volume * Phi((price_change) / sigma)
- V_sell = volume - V_buy
- Classifies volume in aggregate, not individual trades
- No timestamp alignment required
- Use case: VPIN computation (see Section 7)

### BJZZ Parameter Rationale

The BJZZ zones are defined by Boehmer, Jones, Zhang, and Zhang (2021), "Tracking Retail Investor Activity," *The Journal of Finance*, 76(5), 2249-2305:

- **`bjzz_lower = 0.001`**: Excludes round-penny trades (frac_cent < 0.001). Round-penny prints are ambiguous -- they could be lit exchange executions reported to TRF, institutional midpoint crosses at a round-penny midpoint, or genuine round-penny retail fills. Excluding them reduces noise in the retail classification.
- **`bjzz_upper_sell = 0.40` / `bjzz_lower_buy = 0.60`**: The gap `[0.40, 0.60]` around the half-penny excludes likely institutional midpoint crosses. Institutional participants commonly cross at the midpoint, which falls at the half-penny (frac_cent = 0.50) for stocks with a 1-cent spread. This exclusion zone removes the highest-contamination region.
- **`bjzz_upper = 0.999`**: Excludes the exact penny boundary from the buy zone (symmetric with `bjzz_lower`).

Battalio, Jennings, Saglam, and Wu (2022, University of Notre Dame working paper) found that 24.45% of known institutional trades print at sub-penny prices, so BJZZ identification is a noisy proxy. The midpoint signing method (separate from BJZZ identification) corrects the directional assignment.

### Impact of Changes

| Change | Effect |
|--------|--------|
| `exclusion_band = 0.0` | All trades signed (no unsigned zone). Increases signed volume by ~15.4% but introduces misclassified trades near the midpoint where accuracy is lowest. |
| `exclusion_band = 0.40` | Matches the Barber et al. (2024) half-spread convention. Excludes ~12.2% of trades (vs 15.4% at 0.10). Minor impact on IC values; the 0.10 default is conservative. |
| `signing_method = "tick_test"` | Lower accuracy (~70% vs 94.8%) but signs 100% of trades (no unsigned zone). Useful for comparison or when BBO quality is suspect. |
| Widening BJZZ exclusion `[0.30, 0.70]` | Fewer trades classified as retail (higher precision, lower recall). Reduces retail_trade_rate and subpenny_intensity values. |
| Narrowing BJZZ exclusion `[0.45, 0.55]` | More trades classified as retail (lower precision, higher recall). Includes more institutional midpoint crosses in the retail population. |

---

## 6. Section Reference: `[features]`

Toggles for independently enabling/disabling each feature group. Each group maps to a contiguous range of feature indices.

| Parameter | Type | Default | Feature Count | Index Range | Description |
|-----------|------|---------|---------------|-------------|-------------|
| `signed_flow` | bool | `true` | 4 | 0-3 | TRF signed imbalance, retail order imbalance (Mroib), inverse institutional direction, BVC imbalance |
| `venue_metrics` | bool | `true` | 4 | 4-7 | Dark share, TRF volume, lit volume, total volume |
| `retail_metrics` | bool | `true` | 4 | 8-11 | Subpenny intensity, odd-lot ratio, retail trade rate, retail volume fraction |
| `bbo_dynamics` | bool | `true` | 6 | 12-17 | Spread (bps), bid pressure, ask pressure, BBO update rate, quote imbalance, spread change rate |
| `vpin` | bool | `false` | 2 | 18-19 | TRF-specific VPIN, lit-specific VPIN |
| `trade_size` | bool | `true` | 4 | 20-23 | Mean trade size, block trade ratio, trade count, size concentration |
| `cross_venue` | bool | `true` | 3 | 24-26 | TRF burst intensity, time since burst, TRF/lit volume ratio |

**Always-on groups (NOT user-toggleable, no `[features]` field):**

| Group | Feature Count | Index Range | Description |
|-------|---------------|-------------|-------------|
| activity | 2 | 27-28 | `bin_trade_count`, `bin_trf_trade_count` (always emitted) |
| safety_gates | 2 | 29-30 | `bin_valid`, `bbo_valid` (always emitted; structural fields) |
| context | 3 | 31-33 | `session_progress`, `time_bucket`, `schema_version` (always emitted) |

These three groups are emitted regardless of any toggle. They are structural fields required for downstream data integrity. Attempting to set `context = true` (or any other always-on group) in `[features]` will cause TOML parse failure under `#[serde(deny_unknown_fields)]`.

### Feature Count Formula

The output vector is **always 34 elements** regardless of toggles. Disabled groups produce zeros at their indices but still occupy slots (consistent shape for downstream tensors). The formula below counts ENABLED features (for metadata `feature_groups_enabled` only):

```
enabled_count = (signed_flow ? 4 : 0)
              + (venue_metrics ? 4 : 0)
              + (retail_metrics ? 4 : 0)
              + (bbo_dynamics ? 6 : 0)
              + (vpin ? 2 : 0)
              + (trade_size ? 4 : 0)
              + (cross_venue ? 3 : 0)
              + 2                       # activity (always)
              + 2                       # safety_gates (always)
              + 3                       # context (always)

Default: 4 + 4 + 4 + 6 + 0 + 4 + 3 + 2 + 2 + 3 = 32 enabled (34 emitted, 2 zeros at VPIN slots)
All enabled: 4 + 4 + 4 + 6 + 2 + 4 + 3 + 2 + 2 + 3 = 34 enabled and emitted
```

### Rationale for Defaults

- **`signed_flow = true`**: Contains `trf_signed_imbalance` (IC=+0.103 at H=1, E9 cross-validation), the strongest off-exchange predictive signal found in 233 days of NVDA data.
- **`venue_metrics = true`**: Contains `dark_share` (IC=+0.035 at H=10). Required for understanding the lit/dark volume composition.
- **`retail_metrics = true`**: Contains `subpenny_intensity` (IC=+0.104 at H=60), the strongest signal for longer horizons.
- **`bbo_dynamics = true`**: L1 spread and quote pressure features. `spread_bps` is used for cost-aware modeling.
- **`vpin = false`**: VPIN requires volume-based binning (dollar-volume bars), which is architecturally separate from the time-based bin pipeline. Enabling VPIN adds overhead and is only recommended when specifically investigating toxicity signals. See the `nvda_vpin.toml` example config.
- **`trade_size = true`**: Block detection and trade size distribution. `block_trade_ratio` distinguishes uninformative large prints (Comerton-Forde and Putnins 2015) from informative small prints.
- **`cross_venue = true`**: TRF burst detection captures sudden off-exchange activity shifts that may precede lit-market price moves.
- **Context (always-on)**: `session_progress`, `time_bucket`, `schema_version` are required for intraday seasonality control and metadata. Not toggleable.

### Impact of Changes

| Change | Effect |
|--------|--------|
| Disabling `signed_flow` | Loses the strongest predictive feature. Not recommended for production exports. Valid for ablation experiments. |
| Enabling `vpin` | Activates 2 features (indices 18-19). Requires the `[vpin]` section to be configured. VPIN computation runs a parallel volume-bar accumulator inside the same time-binned pipeline. |
| Disabling all optional groups | The 7 always-on features (activity 27-28, safety_gates 29-30, context 31-33) still emit. Indices 0-26 produce zeros. Not useful for modeling but valid for data integrity testing. |

---

## 7. Section Reference: `[vpin]`

Configures Volume-Synchronized Probability of Informed Trading computation. Only relevant when `[features].vpin = true`.

| Parameter | Type | Default | Valid Range | Description |
|-----------|------|---------|-------------|-------------|
| `bucket_volume` | u64 | `5000` | `> 0` | **Currently used.** Absolute shares per VPIN bucket. Default 5000 is conservative; tune by data size. |
| `bucket_volume_fraction` | Option<f64> | `None` (omit) | `(0.0, 0.20]` | **PARTIALLY IMPLEMENTED.** When `Some(f)` AND daily volume from EQUS_SUMMARY is available, overrides `bucket_volume` with `(daily_volume * f) as u64`. When `None` (default) or EQUS unavailable, the absolute `bucket_volume` is used. The fully automatic fraction-based mode (Easley et al. 2012 §3) is not yet enforced via validation. |
| `lookback_buckets` | usize | `50` | (no validation) | Number of volume buckets in the VPIN rolling window. `50` ≈ one full trading day of volume. |
| `sigma_window_minutes` | u32 | `1` | (no validation) | Window size (in minutes) for computing the standard deviation of price changes used in BVC. |

> **Note**: `[vpin]` parameters currently have NO validation in code (no `impl VpinConfig::validate`). Out-of-range values would not cause startup errors. Validation rules listed in §14 are PLANNED.

### Rationale

All defaults follow Easley, Lopez de Prado, and O'Hara (2012), "Flow Toxicity and Liquidity in a High-Frequency World," *Review of Financial Studies*, 25(5), 1457-1493, Section 3:

- **`bucket_volume_fraction = 0.02`**: "We use time bars of 1 minute and volume buckets of 1/50 of the average daily volume" (Section 3). For NVDA with ~150M shares daily volume, each bucket is ~3M shares.
- **`lookback_buckets = 50`**: The VPIN rolling window of 50 buckets covers approximately one full trading day of volume, providing a daily-scale toxicity measure.
- **`sigma_window_minutes = 1`**: BVC uses 1-minute price returns for the sigma parameter in the CDF calculation: `V_buy = V * Phi((P_i - P_{i-1}) / sigma)`. Shorter windows increase noise; longer windows over-smooth.

Per the Andersen and Bondarenko (2014) critique ("VPIN and the Flash Crash," *Journal of Financial Markets*, 17, 1-46), time-rule VPIN has mechanical positive correlation with trading intensity (r = 0.50-0.71). Our implementation uses volume-bar BVC (not time-bar) to address this concern. The `bucket_volume_fraction` parameter controls the volume-bar size.

### VPIN Computation Pipeline

```
1. Accumulate TRF (or lit) trade volume in dollar-volume bars
2. When bar completes (cumulative_dollar_volume >= bucket_size):
   a. Compute BVC: V_buy = V * Phi(price_change / sigma)
   b. Store |V_sell - V_buy| / V for this bucket
3. VPIN = mean of last lookback_buckets ratios
```

Two separate VPINs are computed when enabled:
- `trf_vpin` (index 18): Uses only TRF trades (publisher IDs in `[publishers].trf`)
- `lit_vpin` (index 19): Uses only lit trades (publisher IDs in `[publishers].lit` + `[publishers].minor_lit` if `include_minor_lit_in_lit = true`)

### Impact of Changes

| Change | Effect |
|--------|--------|
| `bucket_volume_fraction = 0.01` | Smaller buckets (1/100 of daily vol). More granular VPIN but noisier per-bucket estimates. ~100 buckets per day. |
| `bucket_volume_fraction = 0.05` | Larger buckets (1/20 of daily vol). Smoother estimates but lower temporal resolution. ~20 buckets per day. |
| `lookback_buckets = 100` | ~2-day lookback. Smoother VPIN but slower to react to regime changes. |
| `lookback_buckets = 20` | ~0.4-day lookback. More responsive but noisier. |
| `sigma_window_minutes = 5` | Smoother BVC sigma. Reduces trade-level noise but may over-smooth rapid volatility changes. |

---

## 8. Section Reference: `[labeling]`

Controls return label computation. Only point-to-point returns are supported (no smoothed-average labels).

| Parameter | Type | Default | Valid Range | Description |
|-----------|------|---------|-------------|-------------|
| `horizons` | array of u32 | `[1, 2, 3, 5, 10, 20, 30, 60]` | Non-empty, each element in `[1, 200]`, ascending order, no duplicates | Forecast horizons in units of bins. At 60s bins: H=1 is 1 minute, H=10 is 10 minutes, H=60 is 1 hour. |
| `label_type` | string (enum) | `"point_return"` | `"point_return"` only | Label computation method. Only point-to-point returns are supported. |

### Label Formula

```
point_return(t, H) = (mid_price[t + H] - mid_price[t]) / mid_price[t] * 10000   [basis points]
```

Where `mid_price[t]` is the Nasdaq BBO midpoint at the end of bin `t`.

### Why Only Point Returns

The E8 experiment (2026-03-21) proved definitively that training on smoothed-average labels produces models that predict the smoothing residual, not the tradeable return. Key findings from `lob-model-trainer/reports/e8_model_execution_diagnostic_2026_03.md`:

- DA on point-to-point returns = 48.3% (below random)
- R-squared(model, smoothing residual) = 45.0%
- R-squared(model, point-to-point component) = 0.02%
- When smoothed and point labels disagree (19.5% of samples), the model follows the smoothing artifact 90.1% of the time

Smoothed labels are architecturally excluded from this pipeline to prevent repeating this validated failure.

### Impact of Changes

| Change | Effect |
|--------|--------|
| `horizons = [1, 5, 10]` | Fewer label columns. Reduces `{day}_labels.npy` shape from `[N, 8]` to `[N, 3]`. Fewer horizons = fewer future samples consumed for labels = more usable sequences per day. |
| `horizons = [10]` | Single-horizon. Label shape `[N, 1]`. Simplest downstream training setup. |
| Adding H=100 or H=200 | Very long horizon labels (100-200 minutes at 60s bins). Consumes many end-of-day samples. May have different signal characteristics. |
| Removing H=1 | Loses the shortest horizon where `trf_signed_imbalance` has strongest IC (+0.103). |
| Non-ascending order | Rejected at validation. Horizons must be strictly ascending. |

### Label Data Budget

Every horizon value H consumes H bins from the end of the trading day (labels at time T require mid_price at time T+H). The maximum horizon determines how many trailing bins cannot produce labels.

```
labelable_bins = usable_bins - max(horizons)

Example (defaults):
  usable_bins = 387
  max(horizons) = 60
  labelable_bins = 327
  sequences_per_day = 327 - 20 + 1 = 308
```

---

## 9. Section Reference: `[sequence]`

Controls sliding window construction for ML input sequences.

| Parameter | Type | Default | Valid Range | Description |
|-----------|------|---------|-------------|-------------|
| `window_size` | u32 | `20` | `[1, 200]` | Number of consecutive bins per sequence. Each sequence is a `[window_size, F]` matrix. |
| `stride` | u32 | `1` | `[1, window_size]` | Step size for the sliding window. Stride=1 means maximum overlap (every bin starts a new sequence). Stride=window_size means no overlap. |

### Rationale

- **`window_size = 20`**: At 60s bins, this captures 20 minutes of history per sequence. The MBO pipeline's signal analysis (T=100 events at ~0.001s/event = ~0.1s total) found that signal concentrates in the last 5 timesteps. For 60s bins, 20 minutes provides sufficient context while keeping sequences manageable. For comparison: E5 used T=20 with 60s bins (same effective window).
- **`stride = 1`**: Maximum overlap produces the most training samples. No overlap (`stride = window_size`) reduces sample count by a factor of `window_size` but eliminates autocorrelation between consecutive sequences.

### Sequence Shape

```
Output: [N, window_size, F] float32

N = (labelable_bins - window_size + 1) / stride   (floor division for stride > 1)

Example (defaults, 60s bins):
  labelable_bins = 327
  N = (327 - 20 + 1) / 1 = 308 sequences per day
  Shape: [308, 20, 32] float32
```

### Impact of Changes

| Change | Effect |
|--------|--------|
| `window_size = 10` | 10-minute history per sequence. More sequences per day (317 vs 308 at defaults). Less context for temporal models. |
| `window_size = 60` | 60-minute history per sequence. Fewer sequences (267 vs 308). Richer temporal context but larger tensors. |
| `stride = 5` | ~62 sequences per day (vs 308). Eliminates most overlap, reducing train-set autocorrelation. Useful for walk-forward validation without purging. |
| `stride = 20` | ~16 sequences per day. Non-overlapping windows. Minimal autocorrelation but very small sample counts. |

---

## 10. Section Reference: `[export]`

Controls NPY file output, train/val/test splitting, and normalization.

| Parameter | Type | Default | Valid Range | Description |
|-----------|------|---------|-------------|-------------|
| `output_dir` | string | (required) | Valid directory path (created if absent) | Root output directory. Subdirectories `train/`, `val/`, `test/` are created automatically. |
| `experiment` | string | `"basic_nvda"` | Non-empty | Experiment name for metadata. |
| `continue_on_error` | bool | `true` | -- | When `true`, per-day failures are logged and the run continues. When `false`, the first failure aborts the run. |
| `split_dates` | table | (required) | See below | Temporal split boundaries for train/val/test. |
| `split_dates.train_end` | string (date) | `"2025-09-30"` | `YYYY-MM-DD`, must be >= `start_date` and <= `val_end` | Last date in the training set (inclusive). |
| `split_dates.val_end` | string (date) | `"2025-11-13"` | `YYYY-MM-DD`, must be > `train_end` and <= `end_date` | Last date in the validation set (inclusive). Days after `val_end` through `end_date` are the test set. |
| `normalization` | string (enum) | `"none"` | `"per_day_zscore"`, `"none"` | Normalization strategy applied to feature sequences before NPY export. **Default is `"none"`** (raw export; the trainer applies its own normalization). `"global_zscore"` is **PLANNED** but not currently valid. |

### Split Logic

```
Training set:  start_date <= day <= train_end
Validation set: train_end < day <= val_end
Test set:       val_end < day <= end_date
```

This matches the temporal split used across all MBO pipeline experiments (163/35/35 days). Zero temporal overlap is enforced.

### Normalization Strategies

**`none`** (default):
No normalization. Features exported in raw units. The downstream Python trainer applies its own normalization using training-set statistics (consistent with the MBO pipeline's `market_structure_zscore` approach in `lob-model-trainer`). The current default lets the trainer remain authoritative for normalization policy.

**`per_day_zscore`**:
Per-day, per-feature z-score normalization computed in Rust at export time. For each day independently:
```
feature_normalized = (feature - mean_day) / (std_day + EPS)
```
Statistics (mean, std) are computed via streaming Welford on the day's bins. Categorical features (indices 29, 30, 32, 33: bin_valid, bbo_valid, time_bucket, schema_version) are excluded from normalization.

The `{day}_normalization.json` sidecar always carries the per-day Welford statistics regardless of whether normalization is applied; consumers can re-normalize or denormalize using these stats.

**`global_zscore`** (PLANNED, not yet implemented):
Would compute mean and std across all training days, apply uniformly. Currently rejected by `DatasetExportConfig::validate()` with `"normalization '{value}' must be one of [\"per_day_zscore\", \"none\"]"`.

### Output Files

Per day:

| File | Shape | Dtype | Description |
|------|-------|-------|-------------|
| `{day}_sequences.npy` | `[N, T, F]` | float32 | Feature sequences (normalized if configured) |
| `{day}_labels.npy` | `[N, H]` | float64 | Point returns in basis points per horizon |
| `{day}_forward_prices.npy` | `[N, max_H+1]` | float64 | Mid-price trajectory in USD (column 0 = base price at time t) |
| `{day}_metadata.json` | -- | JSON | Schema version, feature count, sample count, provenance |
| `{day}_normalization.json` | -- | JSON | Per-feature mean and std used for normalization |

Per dataset:

| File | Description |
|------|-------------|
| `dataset_manifest.json` | Split date lists, config snapshot, total sample counts, experiment metadata |

### Impact of Changes

| Change | Effect |
|--------|--------|
| `normalization = "none"` (default) | Raw feature values exported. The Python trainer applies its own normalization. |
| `normalization = "per_day_zscore"` | Rust-side per-day z-score applied at export time. The `{day}_normalization.json` sidecar carries the stats either way. |
| `normalization = "global_zscore"` | **Currently rejected.** Planned future strategy: single set of statistics across all training days. |
| Changing `split_dates` | Moves the boundary between train/val/test. Ensure sufficient days in each split (minimum ~20 for meaningful statistics). |

---

## 11. Section Reference: `[validation]`

Controls data quality gates, warmup, and empty-bin handling.

| Parameter | Type | Default | Valid Range | Description |
|-----------|------|---------|-------------|-------------|
| `min_trades_per_bin` | u64 | `10` | `> 0` | Minimum number of TRF trades in a bin for `bin_valid = 1.0`. Below this threshold, `bin_valid = 0.0`. |
| `bbo_staleness_max_ns` | u64 | `5_000_000_000` (5s) | `> 0` | Maximum nanoseconds since the last BBO update before `bbo_valid = 0.0`. |
| `warmup_bins` | u32 | `3` | (no validation) | Number of bins to discard at the start of each trading day. |
| `block_threshold` | u32 | `10_000` | `> 0` | Trade size threshold for block detection (used by `block_trade_ratio` feature). |
| `burst_threshold` | u32 | `20` | (no validation) | TRF trades per 1-second window to trigger burst detection. |
| `empty_bin_policy` | string (enum) | `"forward_fill_state"` | `"forward_fill_state"`, `"zero_all"`, `"nan_all"` | How to handle bins with zero TRF trades. |
| `auto_detect_close` | bool | `true` | -- | If true, the pipeline detects early market close (NYSE half-days: July 3, day after Thanksgiving, Christmas Eve) by monitoring for consecutive empty bins. |
| `close_detection_gap_bins` | u32 | `10` | `>= 1` | Number of consecutive bins with zero activity before declaring the trading day complete. **Default 10** at 60s bins = 10 minutes; chosen to avoid LULD halt false positives. Only used when `auto_detect_close = true`. |

### Empty Bin Policy Details

**`forward_fill_state`** (default, recommended):
Distinguishes between state features and flow features:

| Feature Type | Examples | Empty Bin Value | Rationale |
|--------------|----------|-----------------|-----------|
| State/ratio features | subpenny_intensity, odd_lot_ratio, dark_share, spread_bps | Forward-fill from previous bin | These measure persistent market structure that does not change because no trades occurred in one bin. |
| Flow features | trf_signed_imbalance, mroib, volumes, trade_count | 0.0 | No trades = no flow. Zero is the correct value, not a missing value. |
| Safety gates | bin_valid | 0.0 | Below `min_trades_per_bin` by definition. |
| Safety gates | bbo_valid | Computed from BBO staleness | BBO may still be valid even if no trades occurred. |

**`zero_all`**:
All features set to 0.0 for empty bins. Simpler but loses the distinction between "no trades = neutral flow" and "no trades = unknown state."

**`nan_all`**:
All features set to NaN for empty bins. Requires downstream NaN handling (imputation or masking). Useful for analysis but not recommended for model training without explicit NaN strategy.

### Rationale

- **`min_trades_per_bin = 10`**: At 60s bins with ~1.5M TRF trades per day across 390 bins, the average is ~3,800 TRF trades per bin. A threshold of 10 catches only severely anomalous bins (extended halts, pre/post-market edges). This is intentionally loose -- the `bin_valid` gate is a safety flag, not a quality filter.
- **`bbo_staleness_max_ns = 5_000_000_000`** (5 seconds): Nasdaq BBO updates arrive at millisecond frequency during active trading. A 5-second gap indicates a data feed issue or trading halt. Using a stale BBO for midpoint signing degrades accuracy.
- **`warmup_bins = 3`**: Three bins (3 minutes at 60s) allows accumulators to build initial state: forward-fill buffers are populated, running statistics have sufficient samples, and the first few noisy open-auction bins are excluded.
- **`auto_detect_close = true`**: NYSE early-close days (approximately 3 per year) end at 13:00 ET instead of 16:00 ET. Rather than maintaining a hardcoded calendar, the pipeline detects the end of trading from the data. This is more robust to calendar changes and handles unexpected halts.
- **`close_detection_gap_bins = 10`**: Ten consecutive empty bins (10 minutes at 60s) is long enough to distinguish a genuine close from intraday LULD halts (typically <5 min) or other transient lulls, but short enough to detect early closes promptly. The original spec proposed 5 bins; raised to 10 in code to reduce false positives during halts.

### Impact of Changes

| Change | Effect |
|--------|--------|
| `min_trades_per_bin = 1` | Almost all bins are valid. Only truly empty bins are flagged. |
| `min_trades_per_bin = 100` | More bins flagged as invalid. At 10s bins (~380 TRF trades per bin average), a threshold of 100 would flag ~25% of bins. |
| `bbo_staleness_max_ns = 1_000_000_000` (1s) | Stricter BBO freshness. May flag some bins during brief low-activity periods. |
| `warmup_bins = 0` | No warmup. First bin features may be unreliable (no forward-fill history, accumulators have zero samples). |
| `warmup_bins = 10` | 10 minutes of warmup. More conservative. Loses 10 samples per day. |
| `auto_detect_close = false` | Pipeline always processes through `market_close_et`. On early-close days, trailing bins will be empty and flagged via `bin_valid = 0.0`. |
| `close_detection_gap_bins = 2` | Aggressive close detection. May prematurely end processing during brief intraday lulls (e.g., midday low-activity periods). |

---

## 12. Section Reference: `[publishers]` — DEFERRED

> **DEFERRED**: This section is **not currently implemented** in code. There is no `PublishersConfig` struct in `src/config.rs`. Publisher classification uses the hardcoded `PublisherClass::from_id()` function in `src/reader/publisher.rs`. Adding this section to a TOML config will fail with `#[serde(deny_unknown_fields)]`.
>
> The intent of this section is preserved here for future implementation. When ready, it would map Databento publisher IDs to venue categories (TRF vs lit), allowing per-instrument or per-feed customization without code changes.

Maps Databento publisher IDs to venue categories (TRF vs lit). These IDs are fixed by the XNAS.BASIC CMBP-1 schema.

| Parameter | Type | Default | Valid Range | Description |
|-----------|------|---------|-------------|-------------|
| `trf` | array of u16 | `[82, 83]` | Non-empty, valid publisher IDs | Publisher IDs for FINRA TRF venues. Trades from these publishers are classified as off-exchange/dark. |
| `lit` | array of u16 | `[81]` | Non-empty, valid publisher IDs | Publisher IDs for primary lit exchanges. |
| `minor_lit` | array of u16 | `[88, 89]` | Valid publisher IDs (may be empty) | Publisher IDs for minor lit exchanges with negligible volume. |
| `include_minor_lit_in_lit` | bool | `true` | -- | If true, `minor_lit` publisher IDs are counted as lit volume in venue metrics. If false, they are excluded from both TRF and lit volume (counted only in total). |

### Publisher ID Reference

From Databento XNAS.BASIC CMBP-1 schema, validated across 35 test days of NVDA data:

| ID | Venue Code | Full Name | Type | Share of Trades |
|----|-----------|-----------|------|-----------------|
| 81 | XNAS | Nasdaq Stock Market | Primary lit | ~31% |
| 82 | FINN | FINRA/Nasdaq TRF Carteret | Off-exchange (TRF) | ~67% |
| 83 | FINC | FINRA/Nasdaq TRF Chicago | Off-exchange (TRF) | ~2% |
| 88 | XBOS | Nasdaq BX | Minor lit | ~0.2% |
| 89 | XPSX | Nasdaq PSX | Minor lit | ~0.2% |
| 93 | -- | Consolidated BBO | Quotes only | 0% (no trades) |

Publisher 93 emits only BBO quote updates (~5.6M records/day) and zero trade records. It is not included in any category because it has no trades to classify.

### Rationale

- **`trf = [82, 83]`**: FINRA TRF Carteret (82) is the dominant off-exchange venue (~67% of all trades). TRF Chicago (83) is much smaller (~2%) but is the same regulatory facility. Both are FINRA Trade Reporting Facilities where off-exchange transactions in NMS stocks must be reported (FINRA Rules 6282, 6380A, 6380B).
- **`lit = [81]`**: XNAS is the primary Nasdaq lit market. It is the only lit venue with significant volume in the XNAS.BASIC feed.
- **`minor_lit = [88, 89]`**: XBOS and XPSX are Nasdaq-affiliated lit exchanges with negligible volume (~0.2% combined). Including them in lit volume has minimal impact but is technically correct (they are lit exchange executions).
- **`include_minor_lit_in_lit = true`**: Default includes minor lit venues in the lit denominator for dark_share computation. Setting to false would marginally increase the computed dark_share ratio.

### Impact of Changes

| Change | Effect |
|--------|--------|
| `trf = [82]` only | Excludes TRF Chicago. ~2% of off-exchange volume is reclassified as "other" (neither TRF nor lit). Minimal impact on features. |
| `include_minor_lit_in_lit = false` | XBOS/XPSX volume excluded from lit denominator. Increases computed dark_share by ~0.2 percentage points. |
| Adding new publisher IDs | If Databento adds new venues to XNAS.BASIC, they can be mapped here without code changes. The pipeline processes any `publisher_id` present in the data; unmapped IDs are counted in total volume but not in TRF or lit. |

---

## 13. Non-Configurable Constants

The following values are hardcoded constants that must NOT be exposed as configuration parameters. They are defined in `src/contract.rs` (the authoritative source for the standalone repo). The parent HFT-pipeline-v2 monorepo cross-validates these against `contracts/pipeline_contract.toml`.

| Constant | Value | Type | Rationale |
|----------|-------|------|-----------|
| `EPS` | `1e-8` | f64 | Division guard for all denominators. Consistent across all pipeline modules. Changing this value would alter the numerical behavior of every feature computation. |
| `SCHEMA_VERSION` | `1.0` | f64 | Schema version for off-exchange features. Emitted at feature index 33. Changing requires a contract version bump and synchronized updates in all consumers. |
| `CONTRACT_VERSION` | `"off_exchange_1.0"` | &str | Off-exchange contract version string. Independent of MBO schema version (2.2). |
| `NANO_TO_USD` | `1e-9` | f64 | Multiplier for converting i64 nanodollar wire prices to f64 USD. Defined by the Databento CMBP-1 schema (`dbn::FIXED_PRICE_SCALE` is its reciprocal `i64 = 1_000_000_000`). Not a pipeline choice. |
| `UNDEF_PRICE` | `i64::MAX` | i64 | Sentinel value for undefined/missing prices in the dbn crate. Records with this value are rejected at the BBO update boundary. |
| Sign convention | `> 0 = Bullish` | -- | All signed features follow `> 0 = bullish, < 0 = bearish, = 0 = neutral`. Enforced in contract tests. Consistent with the MBO pipeline. |
| Feature index assignments | See `src/features/indices.rs` | usize | Feature-to-index mapping is a contract. Changing indices is a breaking change requiring a schema version bump. |
| Categorical feature indices | `[29, 30, 32, 33]` (`bin_valid`, `bbo_valid`, `time_bucket`, `schema_version`) | -- | These four indices are never normalized. Verified by `test_categorical_indices_match_spec` in `src/features/indices.rs`. |
| Non-normalizable indices | `[29, 30, 31, 32, 33]` | -- | Categorical indices PLUS `session_progress` (31), which is already in `[0, 1]`. Excluded from per-feature z-score. |
| BPS scale literal | `10_000.0` (inline) | f64 | Basis points per unit: `return_bps = (p1/p0 - 1) * 10000`. Standard financial convention. Inlined in formulas; not currently a named constant. |

**Tolerance constants used in tests** (not in `contract.rs`):
- Tests typically compare with tolerance `1e-10` for f64 equality (e.g., `assert!((a - b).abs() < 1e-10)`). Some tests use `1e-15` for chained-operation expected values. These are inlined per-test; not a public constant.

### Why These Are Not Configurable

- **EPS**: This is a numerical infrastructure constant, not a behavioral parameter. Making it configurable would create silent numerical divergence between experiments and between modules. Every division in the pipeline uses EPS. If one experiment uses 1e-8 and another uses 1e-10, their features are subtly different and non-comparable.
- **SCHEMA_VERSION**: The schema version is a contract identifier, not a preference. It must match between producer (this pipeline) and consumer (trainer, backtester). It changes only when the feature layout changes (breaking change protocol).
- **Sign convention**: Directional semantics must be consistent across all features in all modules. A sign flip in one feature would silently invert all downstream signal interpretations.
- **Feature indices**: Indices are a contract surface consumed by downstream Python code (the parent monorepo's `hft-contracts` package auto-generates Python constants from these). Changing them silently breaks all downstream consumers.

---

## 14. Config Validation Rules

All validation occurs at config parse time in `ProcessorConfig::from_toml()` (single-day) or `DatasetConfig::from_toml()` (multi-day CLI). Any validation failure produces a descriptive error message and prevents pipeline startup.

> **Note**: The rules below describe **only what the code currently enforces**. Rules listed under "PLANNED" sections are documented for future implementation. Out-of-range values for unvalidated parameters do not currently fail at startup.

### Date Validation (`DateRangeConfig::validate`, `DatasetExportConfig::validate`)

| Rule | Error Message (paraphrased) |
|------|---------------|
| `start_date` must parse as valid `YYYY-MM-DD` | parse error from `dates::parse_iso_date` |
| `end_date` must parse as valid `YYYY-MM-DD` | parse error from `dates::parse_iso_date` |
| `start_date <= end_date` | `"start_date ({}) must be <= end_date ({})"` |
| Each `exclude_dates[i]` must parse | `"exclude_dates[{i}] '{value}': {parse_error}"` |
| `train_end >= start_date` | `"train_end ({}) must be >= start_date ({})"` |
| `val_end > train_end` | `"val_end ({}) must be > train_end ({})"` |
| `val_end <= end_date` | `"val_end ({}) must be <= end_date ({})"` |

### Sampling Validation (`SamplingConfig::validate`)

| Rule | Error Message |
|------|---------------|
| `strategy` must equal `"time_based"` | `"Unknown sampling strategy '{}'; only 'time_based' is supported"` |
| `bin_size_seconds` must be in `{5, 10, 15, 30, 60, 120, 300, 600}` | `"bin_size_seconds ({}) must be one of [5, 10, ..., 600]"` |

### Classification Validation (`ClassificationConfig::validate`)

| Rule | Error Message |
|------|---------------|
| `signing_method = "tick_test"` is reserved | `"tick_test signing not yet implemented; use 'midpoint' (default)"` |
| Unknown enum variants | Rejected by serde at deserialization (typed enum) |
| `exclusion_band` in `[0.0, 0.50]` | configurable check (see `types.rs`) |
| BJZZ zone bounds (lower < upper, etc.) | configurable check (see `types.rs`) |

### Validation Section (`ValidationConfig::validate`)

| Rule | Error Message |
|------|---------------|
| `min_trades_per_bin > 0` | `"min_trades_per_bin must be > 0"` |
| `bbo_staleness_max_ns > 0` | `"bbo_staleness_max_ns must be > 0"` |
| `block_threshold > 0` | `"block_threshold must be > 0"` |
| `empty_bin_policy` in `{"forward_fill_state", "zero_all", "nan_all"}` | `"empty_bin_policy '{}' must be one of [\"forward_fill_state\", \"zero_all\", \"nan_all\"]"` |
| `close_detection_gap_bins >= 1` | `"close_detection_gap_bins must be >= 1"` |

### Sequence Validation (`SequenceConfig::validate`)

| Rule | Error Message |
|------|---------------|
| `window_size > 0` | `"window_size must be > 0"` |
| `stride > 0` | `"stride must be > 0"` |
| `stride <= window_size` | `"stride ({}) must be <= window_size ({})"` |

### Labeling Validation (`LabelConfig::validate`)

| Rule | Error Message |
|------|---------------|
| `horizons` must be non-empty | `"horizons must be non-empty"` |
| Each horizon in `[1, 200]` | `"horizon[{i}] = {value} must be in [1, 200]"` |
| `horizons` must be strictly ascending | `"horizons must be sorted ascending with no duplicates: ..."` |

### Export Validation (`DatasetExportConfig::validate`)

| Rule | Error Message |
|------|---------------|
| `normalization` in `{"per_day_zscore", "none"}` | `"normalization '{}' must be one of [\"per_day_zscore\", \"none\"]"` |

### PLANNED Validation (NOT currently enforced)

These are documented for future implementation. Code does NOT enforce them as of Phase 5:

| PLANNED Rule | Why Deferred |
|--------------|--------------|
| `bucket_volume_fraction` in `(0.0, 0.20]` (when `features.vpin = true`) | Awaits full daily-volume integration |
| `lookback_buckets` in `[10, 500]` | -- |
| `sigma_window_minutes` in `[1, 60]` | -- |
| Cross-section: `vpin = true` requires `[vpin]` section | Currently `[vpin]` always has defaults via `serde(default)` |
| Cross-section: at least one feature group enabled | Currently 7 always-on features make this trivially true |
| `[publishers]` validation (overlap between trf/lit/minor_lit) | `[publishers]` section deferred entirely |
| `start_date` parse-error template "Invalid start_date format: ..." | Code uses parser's native error message instead |

---

## 15. Example Configs

### 15.1 `nvda_60s.toml` -- Default Production Config (MIRRORS `configs/nvda_60s.toml`)

The standard configuration for 60-second bins. The shipped `configs/nvda_60s.toml` is a minimal version of this template (relies on `serde(default)` for unset values).

```toml
# nvda_60s.toml -- Default production config for NVDA off-exchange features
# Bin size: 60 seconds
# Default features: 32 enabled (all groups except VPIN), but the output vector is always 34 elements
# Horizons: 8 (1 min to 60 min)
# Expected sequences per day: ~308

[input]
# IMPORTANT: Update these paths to your local Databento data location.
data_dir = "./data/XNAS_BASIC/NVDA/cmbp1"
equs_summary_path = "./data/EQUS_SUMMARY/NVDA/equs-summary.ohlcv-1d.dbn.zst"
filename_pattern = "xnas-basic-{date}.cmbp-1.dbn.zst"
symbol = "NVDA"

[dates]
start_date = "2025-02-03"
end_date = "2026-01-06"
exclude_dates = []

[sampling]
strategy = "time_based"              # only "time_based" currently valid
bin_size_seconds = 60                # E5 validated: IC=0.380 at 60s bins
market_open_et = "09:30"
market_close_et = "16:00"

[classification]
signing_method = "midpoint"          # Barber et al. (2024): 94.8% accuracy
exclusion_band = 0.10
bjzz_lower = 0.001                   # Boehmer et al. (2021) BJZZ thresholds
bjzz_upper_sell = 0.40
bjzz_lower_buy = 0.60
bjzz_upper = 0.999

[features]
# (No `context` toggle — context, activity, safety_gates always-on)
signed_flow = true                   # trf_signed_imbalance IC=+0.103 at H=1 (E9-CV)
venue_metrics = true                 # dark_share IC=+0.035 at H=10 (E9-CV)
retail_metrics = true                # subpenny_intensity IC=+0.104 at H=60 (E9-CV)
bbo_dynamics = true
vpin = false                         # Default: disabled
trade_size = true
cross_venue = true

[labeling]
label_type = "point_return"          # E8 lesson: no smoothed labels
horizons = [1, 2, 3, 5, 10, 20, 30, 60]

[sequence]
window_size = 20                     # 20 bins = 20 minutes of history
stride = 1                           # Maximum overlap for training

[validation]
min_trades_per_bin = 10
bbo_staleness_max_ns = 5_000_000_000  # 5 seconds
warmup_bins = 3
block_threshold = 10_000
burst_threshold = 20
empty_bin_policy = "forward_fill_state"
auto_detect_close = true
close_detection_gap_bins = 10        # 10 empty minutes = end-of-day signal

[export]
output_dir = "./data/exports/basic_nvda_60s"
experiment = "basic_nvda_60s"
normalization = "none"               # Default: trainer normalizes downstream
continue_on_error = true

[export.split_dates]
train_end = "2025-09-30"
val_end = "2025-11-13"

# [publishers] section is DEFERRED — uses hardcoded PublisherClass::from_id() in src/reader/publisher.rs
```

### 15.2 `nvda_10s.toml` -- Fine-Grained Config (PLANNED EXAMPLE — not yet in `configs/`)

> **Status**: This config is illustrative — `configs/nvda_10s.toml` does NOT exist in the repo. It documents the intended schema for a fine-grained 10-second variant. To create one, copy `configs/nvda_60s.toml` and apply the differences below. Then place it at `configs/nvda_10s.toml` and update the deferred-config status here.

Higher temporal resolution for investigating short-horizon signals. Produces more bins per day but with noisier per-bin estimates.

```toml
# nvda_10s.toml -- Fine-grained 10-second bins
# Bin size: 10 seconds
# Features: 32 (all groups except VPIN)
# Horizons: 8 (10 sec to 600 sec = 10 min)
# Expected sequences per day: ~1870
#
# Use case: Investigating whether trf_signed_imbalance IC improves at
# sub-minute resolution. E9 found IC=+0.103 at H=1 (60s bins); this
# config tests whether finer binning captures more signal persistence.
# Tradeoff: more bins per day but ~6x fewer TRF trades per bin (~630 avg).

[input]
data_dir = "../data/XNAS_BASIC/NVDA/cmbp1_2025-02-03_to_2026-01-09"
equs_summary_path = "../data/EQUS_SUMMARY/NVDA/ohlcv1d_2025-02-03_to_2026-03-05/equs-summary-*.ohlcv-1d.dbn.zst"
filename_pattern = "xnas-basic-{date}.cmbp-1.dbn.zst"
symbol = "NVDA"

[dates]
start_date = "2025-02-03"
end_date = "2026-01-08"

[sampling]
strategy = "time_based"
bin_size_seconds = 10                # 10-second bins: 6x finer than default
market_open_et = "09:30"
market_close_et = "16:00"

[classification]
signing_method = "midpoint"
exclusion_band = 0.10
bjzz_lower = 0.001
bjzz_upper_sell = 0.40
bjzz_lower_buy = 0.60
bjzz_upper = 0.999

[features]
signed_flow = true
venue_metrics = true
retail_metrics = true
bbo_dynamics = true
vpin = false
trade_size = true
cross_venue = true
# (no `context` toggle — context, activity, safety_gates always-on)

[labeling]
# Horizons scaled to match real-time durations similar to 60s config:
# H=1 = 10s, H=6 = 1min, H=30 = 5min, H=60 = 10min
horizons = [1, 3, 6, 12, 30, 60, 120, 180]
label_type = "point_return"

[sequence]
window_size = 60                     # 60 bins = 10 minutes of history (same real-time as 60s config)
stride = 1

[export]
output_dir = "../data/exports/basic_nvda_10s"
split_dates = { train_end = "2025-09-30", val_end = "2025-11-13" }
normalization = "none"               # raw export — Python normalizes (T15)

[validation]
min_trades_per_bin = 3               # Lower threshold: ~630 avg TRF trades per 10s bin
bbo_staleness_max_ns = 2_000_000_000  # 2 seconds: tighter for finer bins
warmup_bins = 10                     # 100 seconds warmup (comparable to 3 bins at 60s)
empty_bin_policy = "forward_fill_state"
auto_detect_close = true
close_detection_gap_bins = 60        # 60 empty 10s bins = 10 minutes (matches new 60s default)

# [publishers] section — DEFERRED (uses hardcoded PublisherClass::from_id())
```

**Key differences from default**:
- `bin_size_seconds = 10`: 6x more bins (2,340 vs 390 per day)
- `window_size = 60`: Same real-time window (10 minutes) at higher resolution
- `min_trades_per_bin = 3`: Lower threshold because bins have ~630 TRF trades on average (vs ~3,800 at 60s)
- `warmup_bins = 10`: 100 seconds (comparable to 3 minutes at 60s)
- `close_detection_gap_bins = 60`: Same 10-minute real-time gap at 10s resolution
- `horizons` scaled to match similar real-time durations as the 60s config

### 15.3 `nvda_vpin.toml` -- VPIN-Focused Config (PLANNED EXAMPLE — not yet in `configs/`)

> **Status**: This config is illustrative — `configs/nvda_vpin.toml` does NOT exist in the repo. It documents the intended schema for a VPIN-enabled variant. Note that `bucket_volume_fraction` is not yet enforced by code; the absolute `bucket_volume = 5000` (default) is used regardless.

Enables VPIN computation alongside standard time-based features. VPIN uses a parallel volume-bar accumulator while other features use time bins.

```toml
# nvda_vpin.toml -- VPIN-enabled config
# Bin size: 60 seconds (time-based for all features)
# VPIN: volume-bar BVC, 1/50 daily volume per bucket (Easley et al. 2012)
# Features: 34 (all groups including VPIN)
# Horizons: 8
# Expected sequences per day: ~308
#
# Use case: Investigating whether VPIN (flow toxicity) from TRF prints
# has predictive power for NVDA intraday returns. Easley et al. (2012)
# found VPIN AR(1)=0.9958 and correlation with absolute returns r=0.400.
# The Andersen-Bondarenko (2014) critique is addressed by using volume-bar
# BVC (not time-bar) to avoid mechanical volume correlation.

[input]
data_dir = "../data/XNAS_BASIC/NVDA/cmbp1_2025-02-03_to_2026-01-09"
equs_summary_path = "../data/EQUS_SUMMARY/NVDA/ohlcv1d_2025-02-03_to_2026-03-05/equs-summary-*.ohlcv-1d.dbn.zst"
filename_pattern = "xnas-basic-{date}.cmbp-1.dbn.zst"
symbol = "NVDA"

[dates]
start_date = "2025-02-03"
end_date = "2026-01-08"

[sampling]
strategy = "time_based"              # Time-based for main features; VPIN uses internal volume bars
bin_size_seconds = 60
market_open_et = "09:30"
market_close_et = "16:00"

[classification]
signing_method = "midpoint"
exclusion_band = 0.10
bjzz_lower = 0.001
bjzz_upper_sell = 0.40
bjzz_lower_buy = 0.60
bjzz_upper = 0.999

[features]
signed_flow = true
venue_metrics = true
retail_metrics = true
bbo_dynamics = true
vpin = true                          # ENABLED: adds trf_vpin (idx 18) + lit_vpin (idx 19)
trade_size = true
cross_venue = true
# (no `context` toggle — context, activity, safety_gates always-on)

[vpin]
bucket_volume_fraction = 0.02        # 1/50 daily vol per bucket (Easley et al. 2012, Section 3)
lookback_buckets = 50                # ~1 day lookback
sigma_window_minutes = 1             # 1-minute BVC sigma (Easley et al. 2012)

[labeling]
horizons = [1, 2, 3, 5, 10, 20, 30, 60]
label_type = "point_return"

[sequence]
window_size = 20
stride = 1

[export]
output_dir = "../data/exports/basic_nvda_vpin"
split_dates = { train_end = "2025-09-30", val_end = "2025-11-13" }
normalization = "per_day_zscore"

[validation]
min_trades_per_bin = 10
bbo_staleness_max_ns = 5_000_000_000
warmup_bins = 3
empty_bin_policy = "forward_fill_state"
auto_detect_close = true
close_detection_gap_bins = 10

# [publishers] section — DEFERRED (uses hardcoded PublisherClass::from_id())
# trf = [82, 83]
# lit = [81]
# minor_lit = [88, 89]
# include_minor_lit_in_lit = true
```

**Key differences from default**:
- `features.vpin = true`: Enables 2 additional features (indices 18-19)
- `[vpin]` section configured with Easley et al. (2012) standard parameters
- `output_dir` points to a separate export directory to distinguish from non-VPIN exports
- Total feature count: 34 (vs 32 in default)

---

## 16. Experiment Tracking Integration

Every export run must be documented in the `EXPERIMENT_INDEX.md` ledger (see HFT pipeline convention (Rule 13: Experiment Tracking)). The config file is the primary identifier for each experiment.

### Required Metadata in `dataset_manifest.json`

The export step serializes the full config into the dataset manifest:

```json
{
  "schema_version": "1.0",
  "pipeline": "basic-quote-processor",
  "config": {
    "config_file": "configs/nvda_60s.toml",
    "config_hash": "sha256:abc123...",
    "bin_size_seconds": 60,
    "signing_method": "midpoint",
    "exclusion_band": 0.10,
    "feature_count": 32,
    "feature_groups": ["signed_flow", "venue_metrics", "retail_metrics", "bbo_dynamics", "trade_size", "cross_venue", "context"],
    "horizons": [1, 2, 3, 5, 10, 20, 30, 60],
    "window_size": 20,
    "normalization": "per_day_zscore"
  },
  "splits": {
    "train": { "days": ["2025-02-03", "..."], "n_days": 163, "n_sequences": 50000 },
    "val":   { "days": ["2025-10-01", "..."], "n_days": 35,  "n_sequences": 10000 },
    "test":  { "days": ["2025-11-14", "..."], "n_days": 35,  "n_sequences": 10000 }
  },
  "provenance": {
    "data_dir": "../data/XNAS_BASIC/NVDA/cmbp1_2025-02-03_to_2026-01-09",
    "symbol": "NVDA",
    "export_timestamp": "2026-03-22T10:30:00Z",
    "git_commit": "abc123",
    "pipeline_version": "0.1.0"
  }
}
```

### Config-to-Experiment Mapping

| Config Field | Experiment Index Field | Purpose |
|--------------|----------------------|---------|
| Config file path | `config` | Reproducibility: exact config used |
| `config_hash` | `config_hash` | Integrity: detect config modifications after export |
| `bin_size_seconds` | `sampling` | Quick scan for bin size across experiments |
| `feature_count` | `features` | Feature space dimensionality |
| `horizons` | `horizons` | Label horizons |
| `output_dir` | `data_path` | Where to find the exported data |

### Experiment Anti-Pattern Prevention

The config system prevents several documented anti-patterns (from HFT pipeline conventions):

| Anti-Pattern | Prevention |
|--------------|-----------|
| Training on smoothed labels | `label_type` only accepts `"point_return"`. Smoothed labels are architecturally excluded. |
| Undocumented experiment | `dataset_manifest.json` auto-serializes the full config. No experiment can produce exports without a traceable config. |
| Non-reproducible run | Config file + `config_hash` + `git_commit` in provenance enable exact reproduction. |
| Silent config drift | `config_hash` changes if the config file is modified after export. Downstream consumers can validate the hash. |
| Wrong feature count | `feature_count` is computed at startup from enabled groups and recorded in metadata. Shape `[N, T, F]` is validated against this count at export time. |
