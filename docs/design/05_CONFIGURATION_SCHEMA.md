# Configuration Schema: basic-quote-processor TOML Reference

**Status**: Reference Document — **Implementation Status**: Phases 1-5 complete (412 tests)
**Date**: 2026-03-22 (spec), 2026-03-23 (implementation aligned)
**Scope**: Complete TOML configuration schema for the basic-quote-processor pipeline, including all parameters, validation rules, defaults, and rationale

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
[input]
data_dir = "../data/XNAS_BASIC/NVDA/cmbp1_2025-02-03_to_2026-01-09"
equs_summary_path = "../data/EQUS_SUMMARY/NVDA/ohlcv1d_2025-02-03_to_2026-03-05/equs-summary-*.ohlcv-1d.dbn.zst"
filename_pattern = "xnas-basic-{date}.cmbp-1.dbn.zst"
start_date = "2025-02-03"
end_date = "2026-01-08"
symbol = "NVDA"

[sampling]
strategy = "time_based"
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
vpin = false
trade_size = true
cross_venue = true
context = true

[vpin]
bucket_volume_fraction = 0.02
lookback_buckets = 50
sigma_window_minutes = 1

[labeling]
horizons = [1, 2, 3, 5, 10, 20, 30, 60]
label_type = "point_return"

[sequence]
window_size = 20
stride = 1

[export]
output_dir = "../data/exports/basic_nvda_60s"
split_dates = { train_end = "2025-09-30", val_end = "2025-11-13" }
normalization = "per_day_zscore"

[validation]
min_trades_per_bin = 10
bbo_staleness_max_ns = 5_000_000_000
warmup_bins = 3
empty_bin_policy = "forward_fill_state"
auto_detect_close = true
close_detection_gap_bins = 5

[publishers]
trf = [82, 83]
lit = [81]
minor_lit = [88, 89]
include_minor_lit_in_lit = true
```

---

## 3. Section Reference: `[input]`

Controls data source paths and date range for processing.

| Parameter | Type | Default | Valid Range | Description |
|-----------|------|---------|-------------|-------------|
| `data_dir` | string | (required) | Valid directory path | Root directory containing XNAS.BASIC `.dbn.zst` files. One file per trading day. |
| `equs_summary_path` | string | (required) | Valid glob path | Path (supports glob) to EQUS_SUMMARY `.dbn.zst` files. Provides daily consolidated volume for true dark share computation. |
| `filename_pattern` | string | `"xnas-basic-{date}.cmbp-1.dbn.zst"` | Must contain `{date}` placeholder | Filename template for per-day XNAS.BASIC files. The `{date}` token is replaced with `YYYYMMDD`. |
| `start_date` | string (date) | (required) | `YYYY-MM-DD`, must be a valid calendar date | First trading day to process (inclusive). Days without a corresponding file are skipped silently (weekends, holidays). |
| `end_date` | string (date) | (required) | `YYYY-MM-DD`, must be >= `start_date` | Last trading day to process (inclusive). |
| `symbol` | string | `"NVDA"` | Non-empty string | Ticker symbol. Used for metadata, logging, and EQUS_SUMMARY lookup. |

### Rationale

- **`data_dir`**: Relative paths resolve from the config file's parent directory, consistent with MBO extractor conventions.
- **`equs_summary_path`**: Glob support allows a single path to match multiple EQUS_SUMMARY files (one per download batch). The loader merges all matched files by date.
- **`filename_pattern`**: The `{date}` placeholder pattern matches the Databento file naming convention. Alternative schemas (e.g., different naming for ARCX) can be specified without code changes.
- **`start_date` / `end_date`**: The pipeline iterates calendar dates in this range. Missing files (weekends, holidays, gaps in data) produce a log entry and continue. This avoids requiring a separate trading calendar.

### Impact of Changes

| Change | Effect |
|--------|--------|
| Narrowing date range | Fewer days processed. No impact on per-day results. |
| Removing `equs_summary_path` | Error at startup. EQUS_SUMMARY is required for `dark_share` computation (true denominator). |
| Wrong `filename_pattern` | No files found. Pipeline exits with zero-day error at startup. |

---

## 4. Section Reference: `[sampling]`

Controls how raw records are aggregated into discrete bins for feature computation.

| Parameter | Type | Default | Valid Range | Description |
|-----------|------|---------|-------------|-------------|
| `strategy` | string (enum) | `"time_based"` | `"time_based"` or `"volume_based"` | Sampling strategy. `time_based` uses fixed clock-time bins. `volume_based` uses dollar-volume bars (primarily for VPIN; see Section 7). |
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
| `strategy = "volume_based"` | Bins form when cumulative dollar volume reaches a threshold. Produces variable-length time intervals. Only valid with `[vpin]` section. Not supported for general feature export. |
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
| `context` | bool | `true` | 3 | 31-33 | Session progress, time bucket, schema version |

Safety gates (indices 29-30: `bin_valid`, `bbo_valid`) and activity counts (indices 27-28: `bin_trade_count`, `bin_trf_trade_count`) are always emitted regardless of feature group toggles. They are structural fields required for downstream data integrity.

### Feature Count Formula

```
F = (signed_flow ? 4 : 0)
  + (venue_metrics ? 4 : 0)
  + (retail_metrics ? 4 : 0)
  + (bbo_dynamics ? 6 : 0)
  + (vpin ? 2 : 0)
  + (trade_size ? 4 : 0)
  + (cross_venue ? 3 : 0)
  + 2                           # activity (always present)
  + 2                           # safety_gates (always present)
  + (context ? 3 : 0)

Default: 4 + 4 + 4 + 6 + 0 + 4 + 3 + 2 + 2 + 3 = 32 features
All enabled: 4 + 4 + 4 + 6 + 2 + 4 + 3 + 2 + 2 + 3 = 34 features
```

### Rationale for Defaults

- **`signed_flow = true`**: Contains `trf_signed_imbalance` (IC=+0.103 at H=1, E9 cross-validation), the strongest off-exchange predictive signal found in 233 days of NVDA data.
- **`venue_metrics = true`**: Contains `dark_share` (IC=+0.035 at H=10). Required for understanding the lit/dark volume composition.
- **`retail_metrics = true`**: Contains `subpenny_intensity` (IC=+0.104 at H=60), the strongest signal for longer horizons.
- **`bbo_dynamics = true`**: L1 spread and quote pressure features. `spread_bps` is used for cost-aware modeling.
- **`vpin = false`**: VPIN requires volume-based binning (dollar-volume bars), which is architecturally separate from the time-based bin pipeline. Enabling VPIN adds overhead and is only recommended when specifically investigating toxicity signals. See the `nvda_vpin.toml` example config.
- **`trade_size = true`**: Block detection and trade size distribution. `block_trade_ratio` distinguishes uninformative large prints (Comerton-Forde and Putnins 2015) from informative small prints.
- **`cross_venue = true`**: TRF burst detection captures sudden off-exchange activity shifts that may precede lit-market price moves.
- **`context = true`**: Session progress and time bucket are required for intraday seasonality control.

### Impact of Changes

| Change | Effect |
|--------|--------|
| Disabling `signed_flow` | Loses the strongest predictive feature. Not recommended for production exports. Valid for ablation experiments. |
| Enabling `vpin` | Adds 2 features (indices 18-19). Requires that the `[vpin]` section is also configured. VPIN computation is independent of time-bin feature extraction: it runs a parallel volume-bar accumulator. |
| Disabling `context` | Loses session_progress and time_bucket. Sequences lose intraday position awareness. Downstream models cannot condition on time-of-day. |
| Disabling all optional groups | Feature vector has only 4 columns (activity + safety_gates). Not useful for modeling but valid for data integrity testing. |

---

## 7. Section Reference: `[vpin]`

Configures Volume-Synchronized Probability of Informed Trading computation. Only relevant when `[features].vpin = true`.

| Parameter | Type | Default | Valid Range | Description |
|-----------|------|---------|-------------|-------------|
| `bucket_volume_fraction` | f64 | `0.02` | `(0.0, 0.20]` | Fraction of average daily volume per VPIN bucket. `0.02` means each bucket captures 1/50 of daily volume. |
| `lookback_buckets` | u32 | `50` | `[10, 500]` | Number of volume buckets in the VPIN rolling window. `50` means VPIN spans approximately one full trading day of volume. |
| `sigma_window_minutes` | u32 | `1` | `[1, 60]` | Window size (in minutes) for computing the standard deviation of price changes used in BVC (Bulk Volume Classification). |

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
| `split_dates` | table | (required) | See below | Temporal split boundaries for train/val/test. |
| `split_dates.train_end` | string (date) | `"2025-09-30"` | `YYYY-MM-DD`, must be >= `start_date` and < `val_end` | Last date in the training set (inclusive). |
| `split_dates.val_end` | string (date) | `"2025-11-13"` | `YYYY-MM-DD`, must be > `train_end` and < `end_date` | Last date in the validation set (inclusive). Days after `val_end` through `end_date` are the test set. |
| `normalization` | string (enum) | `"per_day_zscore"` | `"per_day_zscore"`, `"global_zscore"`, `"none"` | Normalization strategy applied to feature sequences before NPY export. |

### Split Logic

```
Training set:  start_date <= day <= train_end
Validation set: train_end < day <= val_end
Test set:       val_end < day <= end_date
```

This matches the temporal split used across all MBO pipeline experiments (163/35/35 days). Zero temporal overlap is enforced.

### Normalization Strategies

**`per_day_zscore`** (default):
Per-day, per-feature z-score normalization. For each day independently:
```
feature_normalized = (feature - mean_day) / (std_day + EPS)
```
Statistics (mean, std) are computed from the training set only. For val/test days, the per-day training-set statistics are used. Categorical features (time_bucket, schema_version) are excluded from normalization.

This is consistent with the MBO pipeline's `market_structure_zscore` strategy.

**`global_zscore`**:
Compute mean and std across all training days, apply uniformly:
```
feature_normalized = (feature - mean_global) / (std_global + EPS)
```
Simpler but does not account for daily regime variation. May underperform for features with strong intraday patterns (e.g., dark_share varies systematically by day-of-week).

**`none`**:
No normalization. Features exported in raw units. Useful for analysis scripts and debugging. Not recommended for model training (features have different scales: volume in millions, spread in bps, ratios in [0,1]).

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
| `normalization = "none"` | Raw feature values. Requires downstream normalization in the trainer. Useful for custom normalization experiments. |
| `normalization = "global_zscore"` | Single set of statistics across all training days. Simpler but less adaptive to daily regime shifts. |
| Changing `split_dates` | Moves the boundary between train/val/test. Ensure sufficient days in each split (minimum ~20 for meaningful statistics). |

---

## 11. Section Reference: `[validation]`

Controls data quality gates, warmup, and empty-bin handling.

| Parameter | Type | Default | Valid Range | Description |
|-----------|------|---------|-------------|-------------|
| `min_trades_per_bin` | u32 | `10` | `[1, 1000]` | Minimum number of TRF trades in a bin for `bin_valid = 1.0`. Below this threshold, `bin_valid = 0.0`, signaling to downstream models that the bin's flow features may be unreliable. |
| `bbo_staleness_max_ns` | u64 | `5_000_000_000` | `[100_000_000, 60_000_000_000]` (100ms to 60s) | Maximum nanoseconds since the last BBO update before `bbo_valid = 0.0`. A stale BBO means the midpoint used for trade signing may be inaccurate. |
| `warmup_bins` | u32 | `3` | `[0, 30]` | Number of bins to discard at the start of each trading day. Accumulators (running statistics, forward-fill state) need initial data before producing reliable features. |
| `empty_bin_policy` | string (enum) | `"forward_fill_state"` | `"forward_fill_state"`, `"zero_all"`, `"nan_all"` | How to handle bins with zero TRF trades. |
| `auto_detect_close` | bool | `true` | -- | If true, the pipeline detects early market close (NYSE half-days: July 3, day after Thanksgiving, Christmas Eve) by monitoring for consecutive empty bins. |
| `close_detection_gap_bins` | u32 | `10` | `[2, 30]` | Number of consecutive bins with zero activity (no trades AND no BBO updates) before declaring the trading day complete. Default 10 at 60s bins (10 minutes) avoids LULD halt false positives. Only used when `auto_detect_close = true`. |

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
- **`close_detection_gap_bins = 5`**: Five consecutive empty bins (5 minutes at 60s) is long enough to distinguish a genuine close from a brief lull in trading, but short enough to detect the close promptly. During active NVDA trading, even brief lulls almost never produce 5 consecutive bins with zero trades across all publishers.

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

## 12. Section Reference: `[publishers]`

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

The following values are hardcoded constants that must NOT be exposed as configuration parameters. They are defined in `src/contract.rs` and verified against `contracts/pipeline_contract.toml` by `verify_rust_constants.py`.

| Constant | Value | Type | Rationale |
|----------|-------|------|-----------|
| `EPS` | `1e-8` | f64 | Division guard for all denominators. Consistent across all pipeline modules (HFT pipeline convention). Changing this value would alter the numerical behavior of every feature computation. |
| `FLOAT_CMP_EPS` | `1e-10` | f64 | Golden test comparison tolerance. Consistent across all modules. |
| `SCHEMA_VERSION` | `"1.0"` | string | Schema version for off-exchange features. Emitted at feature index 33 as `1.0`. Changing requires a version bump in `pipeline_contract.toml` with synchronized updates in all consumers. |
| `NANODOLLAR_SCALE` | `1e-9` | f64 | Databento FIXED_PRICE_SCALE for converting i64 wire prices to f64 USD. Defined by the Databento CMBP-1 schema. Not a pipeline choice. |
| `BPS_SCALE` | `10000.0` | f64 | Basis points per unit: `return_bps = (p1/p0 - 1) * 10000`. Standard financial convention. |
| Sign convention | `> 0 = Bullish` | -- | All signed features follow `> 0 = bullish, < 0 = bearish, = 0 = neutral`. Enforced in contract tests. Consistent with MBO pipeline (HFT pipeline convention (Rule 10: Sign Conventions)). |
| Feature index assignments | See Section 6 | u32 | Feature-to-index mapping is a contract registered in `pipeline_contract.toml` under `[features.off_exchange]`. Changing indices is a breaking change. |
| Categorical feature indices | `[31, 33]` | -- | `time_bucket` (index 31) and `schema_version` (index 33) are categorical. Never normalized. Consistent with MBO pipeline treatment of categorical indices. |

### Why These Are Not Configurable

- **EPS / FLOAT_CMP_EPS**: These are numerical infrastructure constants, not behavioral parameters. Making them configurable would create silent numerical divergence between experiments and between modules. Every division in the pipeline uses EPS. If one experiment uses 1e-8 and another uses 1e-10, their features are subtly different and non-comparable.
- **SCHEMA_VERSION**: The schema version is a contract identifier, not a preference. It must match between producer (this pipeline) and consumer (trainer, backtester). It changes only when the feature layout changes (breaking change protocol).
- **Sign convention**: Directional semantics must be consistent across all features in all modules. A sign flip in one feature would silently invert all downstream signal interpretations.
- **Feature indices**: Indices are a contract surface consumed by Python code (`hft-contracts` auto-generation). Changing them silently breaks all downstream consumers.

---

## 14. Config Validation Rules

All validation occurs at config parse time in `PipelineConfig::from_toml()`. Any validation failure produces a descriptive error message and prevents pipeline startup. The pipeline never starts with invalid configuration.

### Date Validation

| Rule | Error Message |
|------|---------------|
| `start_date` must parse as valid `YYYY-MM-DD` | `"Invalid start_date format: expected YYYY-MM-DD, got '{value}'"` |
| `end_date` must parse as valid `YYYY-MM-DD` | `"Invalid end_date format: expected YYYY-MM-DD, got '{value}'"` |
| `end_date >= start_date` | `"end_date ({end_date}) must be >= start_date ({start_date})"` |
| `train_end >= start_date` | `"split_dates.train_end ({train_end}) must be >= start_date ({start_date})"` |
| `val_end > train_end` | `"split_dates.val_end ({val_end}) must be > train_end ({train_end})"` |
| `val_end < end_date` | `"split_dates.val_end ({val_end}) must be < end_date ({end_date})"` |

### Sampling Validation

| Rule | Error Message |
|------|---------------|
| `strategy` must be `"time_based"` or `"volume_based"` | `"Invalid sampling strategy: '{value}'. Must be 'time_based' or 'volume_based'"` |
| `bin_size_seconds` must be in `{5, 10, 15, 30, 60, 120, 300, 600}` | `"Invalid bin_size_seconds: {value}. Must be one of [5, 10, 15, 30, 60, 120, 300, 600]"` |
| `market_open_et` must parse as `HH:MM` | `"Invalid market_open_et: expected HH:MM, got '{value}'"` |
| `market_close_et` must parse as `HH:MM` | `"Invalid market_close_et: expected HH:MM, got '{value}'"` |
| `market_close_et > market_open_et` | `"market_close_et ({close}) must be after market_open_et ({open})"` |
| Session duration must be >= `bin_size_seconds` | `"Session duration ({dur}s) must be >= bin_size_seconds ({bin}s)"` |

### Classification Validation

| Rule | Error Message |
|------|---------------|
| `signing_method` must be `"midpoint"` or `"tick_test"` | `"Invalid signing_method: '{value}'"`. `"tick_test"` is reserved: returns `"tick_test signing not yet implemented; use 'midpoint' (default)"` |
| `exclusion_band` in `[0.0, 0.50]` | `"exclusion_band ({value}) must be in [0.0, 0.50]"` |
| `0 < bjzz_lower < bjzz_upper_sell < 0.50` | `"BJZZ sell zone invalid: need 0 < lower ({l}) < upper_sell ({u}) < 0.50"` |
| `0.50 < bjzz_lower_buy < bjzz_upper < 1.0` | `"BJZZ buy zone invalid: need 0.50 < lower_buy ({l}) < upper ({u}) < 1.0"` |

### Feature Validation

| Rule | Error Message |
|------|---------------|
| At least one feature group must be enabled | `"All feature groups disabled. At least one must be enabled."` |
| If `vpin = true`, the `[vpin]` section must be present | `"features.vpin = true requires a [vpin] section"` |

### VPIN Validation (only if `[features].vpin = true`)

| Rule | Error Message |
|------|---------------|
| `bucket_volume_fraction` in `(0.0, 0.20]` | `"bucket_volume_fraction ({value}) must be in (0.0, 0.20]"` |
| `lookback_buckets` in `[10, 500]` | `"lookback_buckets ({value}) must be in [10, 500]"` |
| `sigma_window_minutes` in `[1, 60]` | `"sigma_window_minutes ({value}) must be in [1, 60]"` |

### Labeling Validation

| Rule | Error Message |
|------|---------------|
| `horizons` must be non-empty | `"horizons must not be empty"` |
| Each horizon in `[1, 200]` | `"horizon {value} must be in [1, 200]"` |
| `horizons` must be strictly ascending | `"horizons must be strictly ascending: {h1} >= {h2} at positions {i}, {i+1}"` |
| No duplicate horizons | Implied by strictly ascending check |
| `label_type` must be `"point_return"` | `"Invalid label_type: '{value}'. Only 'point_return' is supported."` |

### Sequence Validation

| Rule | Error Message |
|------|---------------|
| `window_size` in `[1, 200]` | `"window_size ({value}) must be in [1, 200]"` |
| `stride` in `[1, window_size]` | `"stride ({value}) must be in [1, window_size ({ws})]"` |
| Enough bins for at least 1 sequence: `usable_bins - max(horizons) >= window_size` | `"Insufficient bins for sequences: usable_bins ({ub}) - max_horizon ({mh}) = {lb} < window_size ({ws}). Reduce window_size, max horizon, or increase session duration."` |

### Export Validation

| Rule | Error Message |
|------|---------------|
| `normalization` must be `"per_day_zscore"`, `"global_zscore"`, or `"none"` | `"Invalid normalization: '{value}'"` |

### Publisher Validation

| Rule | Error Message |
|------|---------------|
| `trf` must be non-empty | `"publishers.trf must not be empty"` |
| `lit` must be non-empty | `"publishers.lit must not be empty"` |
| No overlap between `trf`, `lit`, `minor_lit` | `"Publisher ID {id} appears in multiple categories"` |

### Cross-Section Validation

These rules validate consistency across multiple sections:

| Rule | Error Message |
|------|---------------|
| `strategy = "volume_based"` requires `features.vpin = true` | `"sampling.strategy = 'volume_based' is only valid with features.vpin = true"` |
| Total feature count > 0 | `"Computed feature count is 0. Enable at least one feature group."` |

---

## 15. Example Configs

### 15.1 `nvda_60s.toml` -- Default Production Config

The standard configuration for 60-second bins. Matches the E5 bin size that produced the best tradeable results in the MBO pipeline.

```toml
# nvda_60s.toml -- Default production config for NVDA off-exchange features
# Bin size: 60 seconds
# Features: 32 (all groups except VPIN)
# Horizons: 8 (1 min to 60 min)
# Expected sequences per day: ~308

[input]
data_dir = "../data/XNAS_BASIC/NVDA/cmbp1_2025-02-03_to_2026-01-09"
equs_summary_path = "../data/EQUS_SUMMARY/NVDA/ohlcv1d_2025-02-03_to_2026-03-05/equs-summary-*.ohlcv-1d.dbn.zst"
filename_pattern = "xnas-basic-{date}.cmbp-1.dbn.zst"
start_date = "2025-02-03"
end_date = "2026-01-08"
symbol = "NVDA"

[sampling]
strategy = "time_based"
bin_size_seconds = 60                # E5 validated: IC=0.380 at 60s bins
market_open_et = "09:30"             # Nasdaq regular session open
market_close_et = "16:00"            # Nasdaq regular session close

[classification]
signing_method = "midpoint"          # Barber et al. (2024): 94.8% accuracy
exclusion_band = 0.10                # 10% of spread -- conservative (15.4% unsigned)
bjzz_lower = 0.001                   # Boehmer et al. (2021) BJZZ thresholds
bjzz_upper_sell = 0.40
bjzz_lower_buy = 0.60
bjzz_upper = 0.999

[features]
signed_flow = true                   # trf_signed_imbalance IC=+0.103 at H=1 (E9-CV)
venue_metrics = true                 # dark_share IC=+0.035 at H=10 (E9-CV)
retail_metrics = true                # subpenny_intensity IC=+0.104 at H=60 (E9-CV)
bbo_dynamics = true                  # L1 spread dynamics
vpin = false                         # Disabled: requires volume-bar architecture
trade_size = true                    # Block detection, trade size distribution
cross_venue = true                   # TRF burst detection
context = true                       # Session progress, time bucket

[labeling]
horizons = [1, 2, 3, 5, 10, 20, 30, 60]  # Point returns at 1 min to 60 min
label_type = "point_return"               # E8 lesson: no smoothed labels

[sequence]
window_size = 20                     # 20 bins = 20 minutes of history
stride = 1                          # Maximum overlap for training

[export]
output_dir = "../data/exports/basic_nvda_60s"
split_dates = { train_end = "2025-09-30", val_end = "2025-11-13" }  # 163/35/35 days
normalization = "per_day_zscore"     # Consistent with MBO pipeline

[validation]
min_trades_per_bin = 10              # Safety gate: ~3800 avg TRF trades per 60s bin
bbo_staleness_max_ns = 5_000_000_000  # 5 seconds
warmup_bins = 3                      # 3 minutes warmup at session open
empty_bin_policy = "forward_fill_state"
auto_detect_close = true
close_detection_gap_bins = 5         # 5 empty minutes = end of day

[publishers]
trf = [82, 83]                       # FINRA TRF Carteret + Chicago
lit = [81]                           # XNAS lit
minor_lit = [88, 89]                 # XBOS, XPSX
include_minor_lit_in_lit = true
```

### 15.2 `nvda_10s.toml` -- Fine-Grained Config

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
start_date = "2025-02-03"
end_date = "2026-01-08"
symbol = "NVDA"

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
context = true

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
normalization = "per_day_zscore"

[validation]
min_trades_per_bin = 3               # Lower threshold: ~630 avg TRF trades per 10s bin
bbo_staleness_max_ns = 2_000_000_000  # 2 seconds: tighter for finer bins
warmup_bins = 10                     # 100 seconds warmup (comparable to 3 bins at 60s)
empty_bin_policy = "forward_fill_state"
auto_detect_close = true
close_detection_gap_bins = 30        # 30 empty 10s bins = 5 minutes (same real-time gap)

[publishers]
trf = [82, 83]
lit = [81]
minor_lit = [88, 89]
include_minor_lit_in_lit = true
```

**Key differences from default**:
- `bin_size_seconds = 10`: 6x more bins (2,340 vs 390 per day)
- `window_size = 60`: Same real-time window (10 minutes) at higher resolution
- `min_trades_per_bin = 3`: Lower threshold because bins have ~630 TRF trades on average (vs ~3,800 at 60s)
- `warmup_bins = 10`: 100 seconds (comparable to 3 minutes at 60s)
- `close_detection_gap_bins = 30`: Same 5-minute real-time gap at 10s resolution
- `horizons` scaled to match similar real-time durations as the 60s config

### 15.3 `nvda_vpin.toml` -- VPIN-Focused Config

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
start_date = "2025-02-03"
end_date = "2026-01-08"
symbol = "NVDA"

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
context = true

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
close_detection_gap_bins = 5

[publishers]
trf = [82, 83]
lit = [81]
minor_lit = [88, 89]
include_minor_lit_in_lit = true
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
