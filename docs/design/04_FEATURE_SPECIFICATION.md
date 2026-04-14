# Feature Specification: Off-Exchange Pipeline (basic-quote-processor)

**Status**: Reference Document — **Implementation Status**: All 34 features implemented and verified (412 tests)
**Date**: 2026-03-22 (spec), 2026-03-23 (implementation)
**Schema Version**: 1.0
**Total Features**: 34 (indices 0-33)
**Source Data**: XNAS.BASIC CMBP-1 (Nasdaq consolidated L1 + trades with publisher attribution)

This document is the authoritative reference for all 34 features produced by the basic-quote-processor. Every formula, index assignment, and data dependency is specified here. Implementation code in `src/contract.rs` and `src/features/` must match this document exactly.

**Relationship to MBO pipeline**: These 34 features occupy their own index space (0-33), independent of the MBO pipeline's 148-feature space (0-147). A downstream fusion layer handles alignment when combining both feature sets.

---

## Table of Contents

1. [Feature Group Overview](#1-feature-group-overview)
2. [Feature Index Assignment](#2-feature-index-assignment)
3. [Sign Convention](#3-sign-convention)
4. [Shared Primitives](#4-shared-primitives)
5. [Feature Definitions](#5-feature-definitions)
   - [5.1 Signed Flow (0-3)](#51-signed-flow-indices-0-3)
   - [5.2 Venue Metrics (4-7)](#52-venue-metrics-indices-4-7)
   - [5.3 Retail Metrics (8-11)](#53-retail-metrics-indices-8-11)
   - [5.4 BBO Dynamics (12-17)](#54-bbo-dynamics-indices-12-17)
   - [5.5 VPIN (18-19)](#55-vpin-indices-18-19)
   - [5.6 Trade Size (20-23)](#56-trade-size-indices-20-23)
   - [5.7 Cross-Venue (24-26)](#57-cross-venue-indices-24-26)
   - [5.8 Activity (27-28)](#58-activity-indices-27-28)
   - [5.9 Safety Gates (29-30)](#59-safety-gates-indices-29-30)
   - [5.10 Context (31-33)](#510-context-indices-31-33)
6. [Label Contract](#6-label-contract)
7. [Categorical and Non-Normalizable Features](#7-categorical-and-non-normalizable-features)
8. [Empty Bin Policy](#8-empty-bin-policy)
9. [Precision Chain](#9-precision-chain)
10. [E9 Validation Summary](#10-e9-validation-summary)

---

## 1. Feature Group Overview

| Group | Index Range | Count | Features | Requires | Config Key |
|-------|-------------|-------|----------|----------|------------|
| **signed_flow** | 0-3 | 4 | trf_signed_imbalance, mroib, inv_inst_direction, bvc_imbalance | Trade classifier (midpoint signing + BJZZ + BVC) | `features.signed_flow` |
| **venue_metrics** | 4-7 | 4 | dark_share, trf_volume, lit_volume, total_volume | Publisher ID classification | `features.venue_metrics` |
| **retail_metrics** | 8-11 | 4 | subpenny_intensity, odd_lot_ratio, retail_trade_rate, retail_volume_fraction | BJZZ retail identification | `features.retail_metrics` |
| **bbo_dynamics** | 12-17 | 6 | spread_bps, bid_pressure, ask_pressure, bbo_update_rate, quote_imbalance, spread_change_rate | BBO state tracking | `features.bbo_dynamics` |
| **vpin** | 18-19 | 2 | trf_vpin, lit_vpin | Volume-bar BVC (Easley et al. 2012) | `features.vpin` |
| **trade_size** | 20-23 | 4 | mean_trade_size, block_trade_ratio, trade_count, size_concentration | Trade records | `features.trade_size` |
| **cross_venue** | 24-26 | 3 | trf_burst_intensity, time_since_burst, trf_lit_volume_ratio | Timing + venue classification | `features.cross_venue` |
| **activity** | 27-28 | 2 | bin_trade_count, bin_trf_trade_count | Trade records | Always enabled |
| **safety_gates** | 29-30 | 2 | bin_valid, bbo_valid | Validation state | Always enabled |
| **context** | 31-33 | 3 | session_progress, time_bucket, schema_version | Clock + config | Always enabled |
| **Total** | 0-33 | **34** | | | |

Each group (except activity, safety_gates, context) is independently toggleable via the `[features]` section of the TOML config. Groups activity, safety_gates, and context are always emitted.

---

## 2. Feature Index Assignment

```
Index  Name                     Group           Classification
-----  ----                     -----           --------------
  0    trf_signed_imbalance     signed_flow     flow variable
  1    mroib                    signed_flow     flow variable
  2    inv_inst_direction       signed_flow     flow variable
  3    bvc_imbalance            signed_flow     flow variable
  4    dark_share               venue_metrics   state variable
  5    trf_volume               venue_metrics   flow variable
  6    lit_volume               venue_metrics   flow variable
  7    total_volume             venue_metrics   flow variable
  8    subpenny_intensity       retail_metrics  state variable
  9    odd_lot_ratio            retail_metrics  state variable
 10    retail_trade_rate        retail_metrics  flow variable
 11    retail_volume_fraction   retail_metrics  state variable
 12    spread_bps               bbo_dynamics    state variable
 13    bid_pressure             bbo_dynamics    flow variable
 14    ask_pressure             bbo_dynamics    flow variable
 15    bbo_update_rate          bbo_dynamics    flow variable
 16    quote_imbalance          bbo_dynamics    state variable
 17    spread_change_rate       bbo_dynamics    flow variable
 18    trf_vpin                 vpin            state variable
 19    lit_vpin                 vpin            state variable
 20    mean_trade_size          trade_size      state variable
 21    block_trade_ratio        trade_size      state variable
 22    trade_count              trade_size      flow variable
 23    size_concentration       trade_size      state variable
 24    trf_burst_intensity      cross_venue     flow variable
 25    time_since_burst         cross_venue     state variable
 26    trf_lit_volume_ratio     cross_venue     state variable
 27    bin_trade_count          activity        flow variable
 28    bin_trf_trade_count      activity        flow variable
 29    bin_valid                safety_gates    categorical
 30    bbo_valid                safety_gates    categorical
 31    session_progress         context         state variable
 32    time_bucket              context         categorical
 33    schema_version           context         categorical
```

---

## 3. Sign Convention

All directional features follow the pipeline-wide sign convention:

| Value | Meaning |
|-------|---------|
| `> 0` | Bullish / Buy pressure |
| `< 0` | Bearish / Sell pressure |
| `= 0` | Neutral / No signal |

Features with this convention: `trf_signed_imbalance` (0), `mroib` (1), `inv_inst_direction` (2), `bvc_imbalance` (3), `bid_pressure` (13), `ask_pressure` (14), `quote_imbalance` (16).

Features without sign convention (unsigned / always non-negative): `dark_share` (4), all volume features (5-7), all retail metrics (8-11), `spread_bps` (12), `bbo_update_rate` (15), `spread_change_rate` (17), VPIN (18-19), all trade_size (20-23), all cross_venue (24-26), all activity (27-28), all safety_gates (29-30), all context (31-33).

---

## 4. Shared Primitives

These values are used across multiple feature computations and must be computed once per record, not per feature.

### 4.1 BBO Midpoint and Spread

```
mid = (bid_px_00 + ask_px_00) / 2.0
spread = ask_px_00 - bid_px_00
valid_bbo = isfinite(mid) AND (spread > 0)
```

Computed from the CMBP-1 record's `bid_px_00` and `ask_px_00` fields. Updated on every record (both quote updates and trade prints carry BBO fields). The BBO must be updated BEFORE trade classification on each trade record -- the CMBP-1 trade record carries the contemporaneous BBO.

### 4.2 TRF Publisher Classification

```
is_trf = publisher_id IN {82, 83}       // FINRA TRF Carteret (82) + Chicago (83)
is_lit = publisher_id IN {81}           // XNAS (Nasdaq lit)
is_minor_lit = publisher_id IN {88, 89} // XBOS (88), XPSX (89)
```

When `config.publishers.include_minor_lit_in_lit = true` (default), `is_lit` includes minor lit venues: `publisher_id IN {81, 88, 89}`.

### 4.3 Division Guard

All divisions use `EPS = 1e-8` as the denominator floor:

```
safe_divide(num, den) = num / max(den, EPS)
```

For ratio features where the denominator can be zero (e.g., `buy_vol + sell_vol = 0`), the feature value is 0.0 (not NaN), guarded by the empty-bin policy (Section 8).

### 4.4 Midpoint Signing (Barber et al. 2024)

Applied to every trade with `valid_bbo = true`:

```
exclusion_band = config.classification.exclusion_band    // default: 0.10
buy_threshold = mid + exclusion_band * spread
sell_threshold = mid - exclusion_band * spread

direction =
    BUY   if price > buy_threshold
    SELL  if price < sell_threshold
    UNSIGNED  otherwise (within exclusion band)
```

**Source**: Barber, B.M., X. Huang, P. Jorion, T. Odean, and C. Schwarz (2024). "A (Sub)penny for Your Thoughts." *J. Finance*, 79(4), 2403-2427. Signing accuracy: 94.8% (equal-weighted), uniform across spread levels.

### 4.5 BJZZ Retail Identification (Boehmer et al. 2021)

Applied to TRF trades only (`is_trf = true`):

```
frac_cent = (price * 100.0) mod 1.0

is_subpenny = (frac_cent > 0.001) AND (frac_cent < 0.999)

is_retail =
    (frac_cent > config.classification.bjzz_lower AND frac_cent < config.classification.bjzz_upper_sell)
    OR
    (frac_cent > config.classification.bjzz_lower_buy AND frac_cent < config.classification.bjzz_upper)

// Default thresholds:
//   bjzz_lower = 0.001, bjzz_upper_sell = 0.40
//   bjzz_lower_buy = 0.60, bjzz_upper = 0.999
```

**Source**: Boehmer, E., C.M. Jones, X. Zhang, and X. Zhang (2021). "Tracking Retail Investor Activity." *J. Finance*, 76(5), 2249-2305.

The excluded zone [0.40, 0.60] filters out likely institutional midpoint crosses and round-penny prints (frac_cent = 0). Published accuracy: 98.2% for subpenny trades (Boehmer et al. 2021). Known limitations: 65% false negative rate, 24.45% institutional contamination (Battalio et al. 2022; Barber et al. 2024).

---

## 5. Feature Definitions

### 5.1 Signed Flow (Indices 0-3)

#### Index 0: `trf_signed_imbalance`

**The primary directional signal.** Net signed volume imbalance of all off-exchange (TRF) trades within the time bin.

**Classification**: Flow variable (zero on empty bin)

**Formula**:
```
Per time bin, for all trades where is_trf = true:
    buy_vol  = SUM(size) where direction = BUY
    sell_vol = SUM(size) where direction = SELL

    trf_signed_imbalance = (buy_vol - sell_vol) / max(buy_vol + sell_vol, EPS)
```

**Units**: Dimensionless ratio
**Range**: [-1.0, +1.0]
**Data dependencies**: Trade records with `publisher_id IN {82, 83}`, BBO state for midpoint signing

**E9 validation**:

| Horizon | IC | p-value |
|---------|-----|---------|
| H=1 (1 min) | **+0.103** | 4.05e-21 |
| H=2 (2 min) | +0.083 | 2.9e-14 |
| H=3 (3 min) | +0.070 | 1.5e-10 |
| H=5 (5 min) | +0.056 | 3.5e-07 |
| H=10 (10 min) | +0.040 | 3.0e-04 |
| H=20 (20 min) | +0.056 | 3.4e-07 |
| H=30 (30 min) | +0.053 | 1.5e-06 |
| H=60 (60 min) | +0.050 | 5.2e-06 |

- ACF(1) = 0.093 (low persistence, event-driven)
- Bootstrap 95% CI at H=10: [+0.019, +0.060] (excludes zero)
- Per-day stability: mean IC = 0.034, std = 0.082, positive 65.7% of days
- Contemporaneous IC (vs current-period return): +0.033 (NOT classified as contemporaneous since 0.033 < 2 x 0.040)
- Lagged IC: drops sharply at lag=1 (0.040 -> 0.014), partially recovers at lag=10 (0.034)

**Theoretical basis**: Combines Cont et al. (2014) OFI concept with TRF venue decomposition. Unlike MBO OFI which is purely contemporaneous (lag-1 IC < 0.006), TRF signed imbalance captures trades whose price impact has not yet fully propagated to the lit market due to reporting delay and venue routing.

**Source**: Cont, R., A. Kukanov, and S. Stoikov (2014). "The Price Impact of Order Book Events." *J. Financial Econometrics*, 12(1), 47-88. Adapted from OFI to TRF signed volume.

---

#### Index 1: `mroib`

**Market Retail Order Imbalance.** Net signed volume of BJZZ-identified retail trades only.

**Classification**: Flow variable (zero on empty bin)

**Formula**:
```
Per time bin, for all trades where is_trf = true AND is_retail = true:
    retail_buy_vol  = SUM(size) where direction = BUY
    retail_sell_vol = SUM(size) where direction = SELL

    mroib = (retail_buy_vol - retail_sell_vol) / max(retail_buy_vol + retail_sell_vol, EPS)
```

If `retail_buy_vol + retail_sell_vol = 0` (no signed retail trades in bin), `mroib = 0.0`.

**Units**: Dimensionless ratio
**Range**: [-1.0, +1.0]
**Data dependencies**: Trade records with TRF publisher IDs, BJZZ retail identification, midpoint signing

**E9 validation**:
- IC = +0.021 at H=10 (marginal, does NOT pass IC > 0.05 gate)
- ACF(1) = 0.050 (near-zero persistence)
- Cross-sectional weekly Mroib predictability (10.89 bps/week; Boehmer et al. 2021) does NOT transfer to intraday single-stock

**Theoretical basis**: Boehmer et al. (2021) demonstrate weekly return predictability from retail order imbalances across thousands of stocks. However, single-stock intraday application lacks cross-sectional diversification. Retained as secondary signal and input for `inv_inst_direction`.

**Source**: Boehmer, E., C.M. Jones, X. Zhang, and X. Zhang (2021). "Tracking Retail Investor Activity." *J. Finance*, 76(5), 2249-2305. Eqs. 1-2.

---

#### Index 2: `inv_inst_direction`

**Inverse Institutional Direction.** Inferred institutional flow direction based on the Barardehi et al. (2021, 2025) finding that observable retail imbalance is inversely related to institutional order flow.

**Classification**: Flow variable (zero on empty bin)

**Formula**:
```
inv_inst_direction = -mroib
```

**Units**: Dimensionless ratio
**Range**: [-1.0, +1.0]
**Data dependencies**: Same as `mroib` (index 1)

**E9 validation**:
- IC = -0.021 at H=10 (mechanically = -IC(mroib); the negative IC means the institutional inverse theory does NOT produce a usable positive-IC signal at intraday timescale)
- The weekly cross-sectional effect does not transfer to single-stock minute-level prediction

**Theoretical basis**: Wholesalers internalize ~80% of marketable retail orders and adjust internalization to offset their inventory against institutional demand (Barardehi et al. 2021, 2025). Negative mroib (retail selling) implies institutional buying. This relationship is validated at weekly frequency but produces marginal signal intraday.

**Source**: Barardehi, Y.H., D. Bernhardt, Z. Da, and M. Warachka (2021/2025). "Institutional Liquidity Costs, Internalized Retail Trade Imbalances, and the Cross-Section of Stock Returns." *JFQA* (forthcoming).

---

#### Index 3: `bvc_imbalance`

**Bulk Volume Classification Imbalance.** Volume-weighted directional imbalance using normalized price changes rather than individual trade signing, following the BVC method.

**Classification**: Flow variable (zero on empty bin)

**Formula**:
```
For each trade i within the time bin (ALL trades, not TRF-only):
    delta_p_i = price_i - price_{i-1}
    sigma = std(delta_p) over config.vpin.sigma_window_minutes (default: 1 minute)

    // Standard normal CDF
    z_i = delta_p_i / max(sigma, EPS)
    phi_i = Phi(z_i)                         // Phi = standard normal CDF

    buy_vol_i  = size_i * phi_i
    sell_vol_i = size_i * (1.0 - phi_i)

Per time bin:
    bvc_buy_vol  = SUM(buy_vol_i)
    bvc_sell_vol = SUM(sell_vol_i)

    bvc_imbalance = (bvc_buy_vol - bvc_sell_vol) / max(bvc_buy_vol + bvc_sell_vol, EPS)
```

When sigma = 0 (no price variation), phi = 0.5 for all trades, producing `bvc_imbalance = 0.0`.

**Units**: Dimensionless ratio
**Range**: [-1.0, +1.0]
**Data dependencies**: Trade prices and sizes (all venues), rolling price change standard deviation

**E9 validation**: Not directly tested in E9 (BVC was computed for VPIN only, not as a standalone imbalance feature). Expected to have lower per-trade accuracy than midpoint signing but may capture different information content (Panayides, Shohfi, and Smith 2019 found BVC-estimated order flow is the only algorithm correlated with proxies of informed trading).

**Theoretical basis**: BVC sidesteps individual trade signing entirely, using aggregate price-volume dynamics. The standard normal CDF transforms price changes into buy/sell probabilities. This is a complementary signal to `trf_signed_imbalance` -- BVC captures information that midpoint signing may miss (inside-spread trades, unsigned trades).

**Source**: Easley, D., M. Lopez de Prado, and M. O'Hara (2012). "Flow Toxicity and Liquidity in a High-Frequency World." *Rev. Financial Studies*, 25(5), 1457-1493. Eq. 7.

---

### 5.2 Venue Metrics (Indices 4-7)

#### Index 4: `dark_share`

**Regime indicator.** Fraction of visible volume executed off-exchange (TRF) within the time bin.

**Classification**: State variable (forward-fill on empty bin)

**Formula**:
```
Per time bin:
    trf_vol = SUM(size) for trades where is_trf = true
    lit_vol = SUM(size) for trades where is_lit = true

    dark_share = trf_vol / max(trf_vol + lit_vol, EPS)
```

**CRITICAL CAVEAT**: This is TRF/(TRF + XNAS_lit), NOT TRF/consolidated. The denominator excludes 15-19% of total market volume from other lit exchanges (ARCX, BATS, IEX, MEMX, etc.). True market dark share for NVDA is ~50% (validated against EQUS_SUMMARY). Our per-bin average is ~76% due to denominator limitation and session composition effects. Use as a **relative** feature (within-day variation), not as an absolute measure.

**Units**: Dimensionless ratio
**Range**: [0.0, 1.0]
**Data dependencies**: Trade records with publisher ID classification

**E9 validation**:

| Horizon | IC | p-value |
|---------|-----|---------|
| H=1 (1 min) | +0.051 | 2.6e-06 |
| H=10 (10 min) | +0.035 | 0.0015 |
| H=60 (60 min) | -0.013 | 0.228 |

- ACF(1) = 0.418 (moderately persistent)
- Bootstrap 95% CI at H=10: [+0.013, +0.056] (excludes zero)
- Per-day stability: mean IC = 0.032, positive 62.9% of days
- **Sign flip at H=60**: Mean-reverts at longer horizons (consistent with Zhu 2014 dark pool equilibrium)

**Theoretical basis**: Zhu (2014) predicts that when dark_share is high, lit exchange order flow is more informationally dense (uninformed flow has migrated to dark pools). The sign flip at H=60 is consistent with mean-reversion of the dark-lit equilibrium.

**Source**: Zhu, H. (2014). "Do Dark Pools Harm Price Discovery?" *Rev. Financial Studies*, 27(3), 747-789.

---

#### Index 5: `trf_volume`

**Total off-exchange trade volume in the bin.**

**Classification**: Flow variable (zero on empty bin)

**Formula**:
```
Per time bin:
    trf_volume = SUM(size) for trades where is_trf = true
```

**Units**: Shares
**Range**: [0, +inf)
**Data dependencies**: Trade records with TRF publisher IDs

**E9 validation**: IC < 0.03 at H=10. Volume is a state variable useful for conditioning, not a directional predictor.

**Source**: Standard volume decomposition.

---

#### Index 6: `lit_volume`

**Total lit exchange trade volume in the bin.**

**Classification**: Flow variable (zero on empty bin)

**Formula**:
```
Per time bin:
    lit_volume = SUM(size) for trades where is_lit = true
```

**Units**: Shares
**Range**: [0, +inf)
**Data dependencies**: Trade records with lit publisher IDs

**E9 validation**: IC < 0.03 at H=10. Volume is a state variable useful for conditioning, not a directional predictor.

**Source**: Standard volume decomposition.

---

#### Index 7: `total_volume`

**Combined TRF + lit volume in the bin.**

**Classification**: Flow variable (zero on empty bin)

**Formula**:
```
Per time bin:
    total_volume = trf_volume + lit_volume
```

**Units**: Shares
**Range**: [0, +inf)
**Data dependencies**: Indices 5 and 6

**E9 validation**: IC < 0.03 at H=10.

**Source**: Standard volume decomposition.

---

### 5.3 Retail Metrics (Indices 8-11)

#### Index 8: `subpenny_intensity`

**Slow-moving state variable.** Fraction of TRF trades executed at sub-penny prices, measuring wholesaler/retail internalization activity level.

**Classification**: State variable (forward-fill on empty bin)

**Formula**:
```
Per time bin, for all trades where is_trf = true:
    frac_cent = (price * 100.0) mod 1.0
    is_subpenny = (frac_cent > 0.001) AND (frac_cent < 0.999)

    n_subpenny = COUNT(trades where is_subpenny = true)
    n_trf = COUNT(all TRF trades in bin)

    subpenny_intensity = n_subpenny / max(n_trf, 1)
```

Note: This is a COUNT ratio, not volume-weighted.

**Units**: Dimensionless ratio
**Range**: [0.0, 1.0]
**Data dependencies**: TRF trade prices

**E9 validation**:

| Horizon | IC | p-value |
|---------|-----|---------|
| H=1 (1 min) | +0.023 | 0.039 |
| H=10 (10 min) | +0.048 | 1.2e-05 |
| H=20 (20 min) | +0.065 | 3.4e-09 |
| H=60 (60 min) | **+0.104** | 1.3e-21 |

- ACF(1) = 0.889 (extremely persistent -- genuine state variable)
- Bootstrap 95% CI at H=10: [+0.027, +0.069] (excludes zero)
- Remarkably stable across lags: IC = 0.048 (lag 0) -> 0.041 (lag 1) -> 0.044 (lag 10) -- NOT contemporaneous
- IC **increases** with horizon -- unique among all features. Accumulates as wholesaler activity persists.
- 72.55% of TRF trades have subpenny pricing (Z != 0) in E9

**Theoretical basis**: Exploits the Reg NMS Rule 612 artifact. High subpenny_intensity indicates more wholesaler internalization, which Barardehi et al. (2021, 2025) show is driven by institutional liquidity demand. The signal accumulates over longer horizons because it reflects a persistent state (wholesale activity level).

**Source**: Boehmer, E., C.M. Jones, X. Zhang, and X. Zhang (2021). "Tracking Retail Investor Activity." *J. Finance*, 76(5), 2249-2305. BJZZ identification method, adapted as intensity metric.

---

#### Index 9: `odd_lot_ratio`

**Fraction of TRF trades that are odd lots (< 100 shares).**

**Classification**: State variable (forward-fill on empty bin)

**Formula**:
```
Per time bin, for all trades where is_trf = true:
    n_odd_lot = COUNT(trades where size < 100)
    n_trf = COUNT(all TRF trades in bin)

    odd_lot_ratio = n_odd_lot / max(n_trf, 1)
```

COUNT ratio, not volume-weighted.

**Units**: Dimensionless ratio
**Range**: [0.0, 1.0]
**Data dependencies**: TRF trade sizes

**E9 validation**:
- IC = +0.018 at H=10 (marginal)
- ACF(1) = 0.886 (extremely persistent -- state variable)

**Theoretical basis**: Odd-lot trades are highly informative and disproportionately correlated with HFT activity. Their exclusion from the consolidated tape (prior to 2014 rule changes) meant they carried hidden information.

**Source**: O'Hara, M., C. Yao, and M. Ye (2014). "What's Not There: Odd Lots and Market Data." *J. Finance*, 69(5), 2199-2236.

---

#### Index 10: `retail_trade_rate`

**Fraction of TRF trades that are BJZZ-identified retail within the bin.**

**Classification**: Flow variable (zero on empty bin)

**Formula**:
```
Per time bin:
    n_retail = COUNT(trades where is_trf = true AND is_retail = true)
    n_trf    = COUNT(trades where is_trf = true)

    retail_trade_rate = n_retail / max(n_trf, EPS)
```

If `n_trf = 0` (no TRF trades in bin), `retail_trade_rate = 0.0`.

**Units**: Dimensionless ratio
**Range**: [0.0, 1.0]
**Data dependencies**: TRF trade prices (for BJZZ identification)

**E9 validation**:
- IC = +0.027 at H=10 (marginal)
- ACF(1) = 0.976 (extremely persistent)
- Measures activity level, not direction

**Source**: Derived from BJZZ identification (Boehmer et al. 2021).

---

#### Index 11: `retail_volume_fraction`

**Fraction of total TRF volume attributable to BJZZ-identified retail trades.**

**Classification**: State variable (forward-fill on empty bin)

**Formula**:
```
Per time bin:
    retail_vol = SUM(size) for trades where is_trf = true AND is_retail = true
    trf_vol = SUM(size) for trades where is_trf = true

    retail_volume_fraction = retail_vol / max(trf_vol, EPS)
```

**Units**: Dimensionless ratio
**Range**: [0.0, 1.0]
**Data dependencies**: TRF trade prices and sizes, BJZZ classification

**E9 validation**: Not directly tested as standalone feature in E9. Expected ACF > 0.8 (state variable, similar persistence to subpenny_intensity). Measures retail participation level in off-exchange activity.

**Theoretical basis**: The composition of TRF prints (retail vs institutional) shifts predictably with urgency/uncertainty (Menkveld, Yueshen, and Zhu 2017, pecking order). Higher retail fraction indicates calmer market conditions with more wholesaler internalization.

**Source**: Menkveld, A.J., B.Z. Yueshen, and H. Zhu (2017). "Shades of Darkness." *Rev. Financial Studies*, 30(12), 4321-4372.

---

### 5.4 BBO Dynamics (Indices 12-17)

All BBO dynamics features are computed from Nasdaq L1 BBO quote updates within the time bin. These capture the lit-market microstructure state that complements the off-exchange flow features.

#### Index 12: `spread_bps`

**Nasdaq best bid-offer spread in basis points, time-weighted average over the bin.**

**Classification**: State variable (forward-fill on empty bin)

**Formula**:
```
For each BBO state observed within the time bin (on every quote update):
    instant_spread_bps = (ask_px_00 - bid_px_00) / max(mid, EPS) * 10000.0

Time-weighted average:
    spread_bps = SUM(instant_spread_bps_i * duration_i) / max(SUM(duration_i), EPS)

where duration_i = time from BBO state i to BBO state i+1 (or bin boundary)
```

If no BBO updates occur in the bin, forward-fill from prior bin.

**Units**: Basis points (bps)
**Range**: [0.0, +inf) -- typically 0.5-5.0 bps for NVDA
**Data dependencies**: BBO state (bid_px_00, ask_px_00) with timestamps

**E9 validation**: Not directly tested as standalone feature. Spread is a conditioning/regime variable. NVDA typically has 1-cent spread (~0.8 bps), consistent with MBO profiler data (XNAS effective spread = 0.80 bps).

**Theoretical basis**: Standard microstructure measure. Spread reflects adverse selection cost and is a primary determinant of trade classification accuracy (Barber et al. 2024: signing accuracy drops 40.5pp from 1-cent to 10+ cent spread for BJZZ).

**Source**: Standard market microstructure.

---

#### Index 13: `bid_pressure`

**Net change in best bid size within the bin, normalized by initial bid size. Positive = bid strengthening (bullish).**

**Classification**: Flow variable (zero on empty bin)

**Formula**:
```
Per time bin:
    bid_size_start = bid_sz_00 at bin start (or first BBO update)
    bid_size_end = bid_sz_00 at bin end (or last BBO update)

    bid_pressure = (bid_size_end - bid_size_start) / max(bid_size_start, EPS)
```

When no BBO updates occur in the bin, `bid_pressure = 0.0`.

**Units**: Dimensionless ratio (fractional change)
**Range**: (-inf, +inf) -- typically [-1.0, +5.0]
**Data dependencies**: BBO state (bid_sz_00) at bin boundaries

**E9 validation**: Not directly tested. Captures L1 quote dynamics from the Nasdaq BBO. Expected to be complementary to trade-based flow features.

**Theoretical basis**: Queue imbalance dynamics. Gould and Bonart (2015) show that bid queue size change is a one-tick-ahead price predictor. Cartea, Donnelly, and Jaimungal (2018) formalize Volume Order Imbalance (VOI) as `(V^b - V^a)/(V^b + V^a)` with strong predictive power for next market order direction.

**Source**: Gould, M.D. and J. Bonart (2015). "Queue Imbalance as a One-Tick-Ahead Price Predictor." Working paper. Adapted from level change to fractional change.

---

#### Index 14: `ask_pressure`

**Net change in best ask size within the bin, normalized by initial ask size. Positive = ask strengthening (bearish -- more supply).**

**Classification**: Flow variable (zero on empty bin)

**Formula**:
```
Per time bin:
    ask_size_start = ask_sz_00 at bin start (or first BBO update)
    ask_size_end = ask_sz_00 at bin end (or last BBO update)

    ask_pressure = (ask_size_end - ask_size_start) / max(ask_size_start, EPS)
```

Note: Increasing ask size is bearish (more supply). The raw value is unsigned -- the sign convention is applied via the `quote_imbalance` feature (index 16) which combines bid and ask pressure with proper directional semantics.

**Units**: Dimensionless ratio (fractional change)
**Range**: (-inf, +inf) -- typically [-1.0, +5.0]
**Data dependencies**: BBO state (ask_sz_00) at bin boundaries

**E9 validation**: Not directly tested.

**Source**: Same as bid_pressure (index 13).

---

#### Index 15: `bbo_update_rate`

**Number of BBO quote updates received within the bin.**

**Classification**: Flow variable (zero on empty bin)

**Formula**:
```
Per time bin:
    bbo_update_rate = COUNT(records where action = 'A' OR (action = 'T' AND BBO changed))
```

This counts every event that modifies the BBO state (either pure quote updates or trade prints that shift the BBO).

**Units**: Count (updates per bin)
**Range**: [0, +inf) -- typically 100-10,000 per 60s bin for NVDA
**Data dependencies**: All records with BBO fields

**E9 validation**: Not directly tested. Measures lit-market activity intensity. High values indicate active quoting and potential quote flickering.

**Source**: Standard market activity measure.

---

#### Index 16: `quote_imbalance`

**Size imbalance at the best bid vs best ask at the end of the bin. Positive = more bid depth (bullish).**

**Classification**: State variable (forward-fill on empty bin)

**Formula**:
```
At end of time bin:
    quote_imbalance = (bid_sz_00 - ask_sz_00) / max(bid_sz_00 + ask_sz_00, EPS)
```

**Units**: Dimensionless ratio
**Range**: [-1.0, +1.0]
**Data dependencies**: BBO state at bin end (bid_sz_00, ask_sz_00)

**E9 validation**: Not directly tested. Partially computable analogue of queue imbalance from XNAS.ITCH MBO data.

**Theoretical basis**: Gould and Bonart (2015) demonstrate AUC = 0.76-0.80 for next mid-price move prediction on large-tick Nasdaq stocks using queue imbalance. Stoikov (2018) uses imbalance `I = Q^b / (Q^b + Q^a)` as the key state variable for microprice computation. Note: our `quote_imbalance` is the centered version `(Q^b - Q^a)/(Q^b + Q^a) = 2*I - 1`.

**Source**: Gould, M.D. and J. Bonart (2015). "Queue Imbalance as a One-Tick-Ahead Price Predictor." Working paper. Eq. 7. Also: Stoikov, S. (2018). "The Micro-Price." *Quantitative Finance*, 18(12), 1959-1966.

---

#### Index 17: `spread_change_rate`

**Net change in spread over the bin, in basis points.**

**Classification**: Flow variable (zero on empty bin)

**Formula**:
```
Per time bin:
    spread_start_bps = (ask_px_00 - bid_px_00) / max(mid, EPS) * 10000.0  at bin start
    spread_end_bps = (ask_px_00 - bid_px_00) / max(mid, EPS) * 10000.0  at bin end

    spread_change_rate = spread_end_bps - spread_start_bps
```

Positive values indicate spread widening (deteriorating liquidity); negative values indicate spread tightening (improving liquidity).

**Units**: Basis points change
**Range**: (-inf, +inf) -- typically [-2.0, +2.0] for NVDA
**Data dependencies**: BBO state at bin start and end

**E9 validation**: Not directly tested. Spread dynamics capture liquidity regime changes. Per Comerton-Forde and Putnins (2015), spread widening during high dark share indicates adverse selection spillover from dark venues.

**Source**: Standard market microstructure. Comerton-Forde, C. and T.J. Putnins (2015). "Dark Trading and Price Discovery." *J. Financial Economics*, 118(1), 70-92.

---

### 5.5 VPIN (Indices 18-19)

VPIN features are disabled by default (`features.vpin = false`) because they require a separate volume-bar sampling mechanism that runs alongside the time-bin sampler.

#### Index 18: `trf_vpin`

**Volume-Synchronized Probability of Informed Trading, computed from TRF trades only.**

**Classification**: State variable (forward-fill on empty bin)

**Formula**:
```
// Step 1: Volume bars (NOT time bars, per Andersen and Bondarenko 2014 critique)
V_bucket = config.vpin.bucket_volume_fraction * daily_average_trf_volume

// Step 2: BVC classification per volume bucket tau
For each trade i within volume bucket tau:
    delta_p_i = price_i - price_{i-1}
    sigma = std(delta_p) over config.vpin.sigma_window_minutes

    buy_vol_i = size_i * Phi(delta_p_i / max(sigma, EPS))
    sell_vol_i = size_i * (1.0 - Phi(delta_p_i / max(sigma, EPS)))

V_tau_B = SUM(buy_vol_i)
V_tau_S = SUM(sell_vol_i)

// Step 3: VPIN over lookback window
n = config.vpin.lookback_buckets                      // default: 50

trf_vpin = (1/n) * SUM_{tau=L-n+1}^{L} |V_tau_S - V_tau_B| / V_bucket
```

Default parameters: `bucket_volume_fraction = 0.02` (1/50 of daily volume), `lookback_buckets = 50`, `sigma_window_minutes = 1`.

**Units**: Dimensionless ratio
**Range**: [0.0, 1.0] -- 0 = perfectly balanced flow, 1 = maximally toxic
**Data dependencies**: TRF trade prices and sizes, rolling price change sigma

**E9 validation**: Not tested in E9. VPIN is the highest out-of-sample predictor (MDA) across 87 liquid futures for bid-ask spread and kurtosis prediction (Easley, Lopez de Prado, O'Hara, and Zhang 2021).

**Theoretical basis**: VPIN estimates the probability of informed trading without requiring a structural model (unlike PIN). Volume-bar sampling synchronizes with market activity. The Andersen-Bondarenko (2014) critique shows time-bar VPIN (TR-VPIN) has mechanical volume correlation; volume-bar BVC avoids this artifact.

**Source**: Easley, D., M. Lopez de Prado, and M. O'Hara (2012). "Flow Toxicity and Liquidity in a High-Frequency World." *Rev. Financial Studies*, 25(5), 1457-1493. Eq. 7 (BVC), Eq. 10 (VPIN). Volume-bar recommendation: Easley et al. (2021).

---

#### Index 19: `lit_vpin`

**VPIN computed from XNAS lit trades only.**

**Classification**: State variable (forward-fill on empty bin)

**Formula**: Same as `trf_vpin` (index 18) but restricted to trades where `is_lit = true`. Volume bucket size uses daily average lit volume.

```
V_bucket = config.vpin.bucket_volume_fraction * daily_average_lit_volume
```

All other computation steps identical to index 18.

**Units**: Dimensionless ratio
**Range**: [0.0, 1.0]
**Data dependencies**: Lit trade prices and sizes, rolling price change sigma

**E9 validation**: Not tested. Comparing `trf_vpin` vs `lit_vpin` may reveal whether informed trading concentrates in dark or lit venues (per Zhu 2014: informed traders face lower fill rates in dark pools, so lit VPIN may be higher during informed episodes).

**Source**: Same as index 18.

---

### 5.6 Trade Size (Indices 20-23)

#### Index 20: `mean_trade_size`

**Average trade size across all trades in the bin.**

**Classification**: State variable (forward-fill on empty bin)

**Formula**:
```
Per time bin (all trades, both TRF and lit):
    mean_trade_size = SUM(size) / max(COUNT(trades), 1)
```

**Units**: Shares
**Range**: [0, +inf) -- typically 50-500 for NVDA
**Data dependencies**: All trade records

**E9 validation**: Not directly tested. Trade size distribution carries information about market participant composition (Comerton-Forde and Putnins 2015: block vs non-block dark trades have different price discovery impacts).

**Source**: Standard market microstructure.

---

#### Index 21: `block_trade_ratio`

**Fraction of trades that are block-sized (>= 10,000 shares for NVDA).**

**Classification**: State variable (forward-fill on empty bin)

**Formula**:
```
Per time bin (all trades):
    block_threshold = 10000                     // shares, configurable
    n_block = COUNT(trades where size >= block_threshold)
    n_total = COUNT(all trades in bin)

    block_trade_ratio = n_block / max(n_total, 1)
```

COUNT ratio, not volume-weighted.

**Units**: Dimensionless ratio
**Range**: [0.0, 1.0] -- typically very low (< 0.01 for NVDA)
**Data dependencies**: Trade sizes

**E9 validation**: Not directly tested. Expected to be a rare-event feature (most bins will have 0 block trades).

**Theoretical basis**: Comerton-Forde and Putnins (2015) find that block dark trades do NOT impede price discovery at any level, while small non-block dark trades do. Block detection separates these two populations.

**Source**: Comerton-Forde, C. and T.J. Putnins (2015). "Dark Trading and Price Discovery." *J. Financial Economics*, 118(1), 70-92.

---

#### Index 22: `trade_count`

**Total number of trades in the bin (all venues).**

**Classification**: Flow variable (zero on empty bin)

**Formula**:
```
Per time bin:
    trade_count = COUNT(all trades where action = 'T')
```

**Units**: Count
**Range**: [0, +inf) -- typically 500-5000 per 60s bin for NVDA
**Data dependencies**: All trade records

**E9 validation**: Not directly tested. Activity measure, not directional.

**Source**: Standard.

---

#### Index 23: `size_concentration`

**Herfindahl-Hirschman Index (HHI) of trade sizes within the bin. Measures concentration: 0 = uniform distribution, 1 = single trade dominates.**

**Classification**: State variable (forward-fill on empty bin)

**Formula**:
```
Per time bin with n trades:
    total_vol = SUM(size_i) for all trades i in bin

    size_concentration = SUM_i (size_i / max(total_vol, EPS))^2
```

When `n = 0`, `size_concentration = 0.0` (empty bin).
When `n = 1`, `size_concentration = 1.0` (single trade).

**Units**: Dimensionless (HHI)
**Range**: [0.0, 1.0]
**Data dependencies**: Trade sizes

**E9 validation**: Not directly tested. High concentration indicates a small number of large trades dominating the bin -- potential institutional activity.

**Source**: Standard concentration measure (Herfindahl-Hirschman Index).

---

### 5.7 Cross-Venue (Indices 24-26)

#### Index 24: `trf_burst_intensity`

**Measures clustering of TRF trades within the bin. High values indicate bursts of off-exchange activity, which Nimalendran and Ray (2014) show can precede lit-market price moves.**

**Classification**: Flow variable (zero on empty bin)

**Formula**:
```
Per time bin:
    trf_trades = list of TRF trade timestamps (sorted)
    n_trf = len(trf_trades)

    if n_trf < 2:
        trf_burst_intensity = 0.0
    else:
        inter_arrival_times = [trf_trades[i+1] - trf_trades[i] for i in 0..n_trf-1]
        mean_iat = mean(inter_arrival_times)
        std_iat = std(inter_arrival_times)

        // Coefficient of variation: high CV = bursty, low CV = regular
        trf_burst_intensity = std_iat / max(mean_iat, EPS)
```

A Poisson process has CV = 1.0. Values > 1.0 indicate bursty (clustered) arrivals; values < 1.0 indicate regular (evenly spaced) arrivals.

**Units**: Dimensionless (coefficient of variation)
**Range**: [0.0, +inf) -- typically [0.5, 5.0]
**Data dependencies**: TRF trade timestamps within the bin

**E9 validation**: Not directly tested. Cross-venue burst detection is motivated by Nimalendran and Ray (2014) who find signed trades can predict returns at 15-120 minute horizons, with activity bursts transmitting more information.

**Source**: Nimalendran, M. and S. Ray (2014). "Informational Linkages Between Dark and Lit Trading Venues." *J. Financial Markets*, 17, 230-261.

---

#### Index 25: `time_since_burst`

**Time in seconds since the most recent TRF burst event. A burst is defined as N or more TRF trades arriving within a configurable window.**

**Classification**: State variable (forward-fill on empty bin)

**Formula**:
```
burst_threshold = 20           // trades within window (configurable)
burst_window_ms = 1000         // 1-second window (configurable)

// Track running count of TRF trades in trailing burst_window_ms
// A burst event occurs when count >= burst_threshold within the window

Per time bin:
    if burst occurred within this bin:
        time_since_burst = (bin_end_time - last_burst_time) in seconds
    else if previous burst exists:
        time_since_burst = (bin_end_time - last_burst_time) in seconds
    else:
        time_since_burst = bin_size_seconds * warmup_bins   // cap at warmup period
```

**Units**: Seconds
**Range**: When no burst has been observed yet (warmup or genuinely no bursts), the value is exactly `warmup_bins * bin_size_seconds` (e.g., 3 × 60s = **180.0** with defaults). After the first burst, the value grows monotonically as `(bin_end_ts - last_burst_ts) / 1e9` until the next burst resets it. Not capped at a session length — if no further burst occurs, it grows linearly toward the end-of-session value (~23,400s in a full 6.5h session).
**Data dependencies**: TRF trade timestamps, burst detection state (`BurstTracker.last_burst_ts`)

**E9 validation**: Not directly tested. Derived from cross-venue interaction theory.

**Source**: Nimalendran, M. and S. Ray (2014). See index 24.

---

#### Index 26: `trf_lit_volume_ratio`

**Ratio of TRF volume to lit volume in the bin. Captures relative activity across venues.**

**Classification**: State variable (forward-fill on empty bin)

**Formula**:
```
Per time bin:
    trf_lit_volume_ratio = trf_volume / max(lit_volume, EPS)
```

Note: This is unbounded (unlike dark_share which is bounded [0,1]). Values > 1.0 indicate TRF volume exceeds lit volume (typical for NVDA: mean ~3.0 due to ~75% TRF share within our data scope).

**Units**: Dimensionless ratio
**Range**: [0.0, +inf) -- typically [1.0, 10.0] for NVDA
**Data dependencies**: trf_volume (index 5), lit_volume (index 6)

**E9 validation**: Not directly tested. Complementary to dark_share with different dynamic range and sensitivity to extreme values.

**Source**: Standard venue decomposition.

---

### 5.8 Activity (Indices 27-28)

Always enabled (not configurable).

#### Index 27: `bin_trade_count`

**Total number of trades across all venues in the bin.**

**Classification**: Flow variable (zero on empty bin)

**Formula**:
```
Per time bin:
    bin_trade_count = COUNT(all records where action = 'T')
```

Equivalent to `trade_count` (index 22) but always present (not gated by `features.trade_size`). When the trade_size group is enabled, both are emitted -- this duplication is intentional since activity features serve a different role (safety gating) than trade_size features (distribution analysis).

**Units**: Count
**Range**: [0, +inf)
**Data dependencies**: All trade records

**E9 validation**: Not directly tested.

**Source**: Standard.

---

#### Index 28: `bin_trf_trade_count`

**Number of TRF trades in the bin.**

**Classification**: Flow variable (zero on empty bin)

**Formula**:
```
Per time bin:
    bin_trf_trade_count = COUNT(records where action = 'T' AND is_trf = true)
```

**Units**: Count
**Range**: [0, +inf)
**Data dependencies**: TRF trade records

**E9 validation**: Not directly tested.

**Source**: Standard.

---

### 5.9 Safety Gates (Indices 29-30)

Always enabled. Categorical features (not normalized). Used by downstream consumers to filter invalid bins.

#### Index 29: `bin_valid`

**Indicates whether the bin has sufficient data for reliable feature computation.**

**Classification**: Categorical (never normalized)

**Formula**:
```
bin_valid =
    1.0  if bin_trf_trade_count >= config.validation.min_trades_per_bin
    0.0  otherwise

// Default: min_trades_per_bin = 10
```

A bin with fewer than `min_trades_per_bin` TRF trades has insufficient data for reliable ratio features (subpenny_intensity, dark_share, signed imbalance). Setting `bin_valid = 0.0` signals downstream consumers to gate this bin.

**Units**: Binary indicator
**Range**: {0.0, 1.0}
**Data dependencies**: bin_trf_trade_count (index 28), config.validation.min_trades_per_bin

**Condition details**:
- Threshold is on **TRF** trades specifically (not total trades), since the primary features are TRF-derived
- Default threshold of 10 ensures denominators in ratio features are stable
- Configurable to allow tighter (20+) or looser (5) gating

---

#### Index 30: `bbo_valid`

**Indicates whether the BBO was recently updated and is usable for trade classification.**

**Classification**: Categorical (never normalized)

**Formula**:
```
bbo_valid =
    1.0  if valid_bbo = true AND (bin_end_time - last_bbo_update_time) <= config.validation.bbo_staleness_max_ns
    0.0  otherwise

// Default: bbo_staleness_max_ns = 5_000_000_000 (5 seconds)
// valid_bbo = isfinite(mid) AND (spread > 0)
```

A stale BBO (not updated within 5 seconds of bin end) degrades midpoint signing accuracy because the reference midpoint may not reflect current market conditions. Empirically, 100% of TRF trades in the E9 test period have valid BBO, so this gate primarily catches data gaps and edge-of-session conditions.

**Units**: Binary indicator
**Range**: {0.0, 1.0}
**Data dependencies**: BBO state, timestamp of last BBO update, config.validation.bbo_staleness_max_ns

**Condition details**:
- Checks BOTH that BBO values are valid (finite, positive spread) AND that the BBO is fresh (updated within staleness threshold)
- 5-second default matches the FINRA TRF reporting ceiling -- if BBO is older than 5 seconds, trade classification is unreliable
- A bin can have `bin_valid = 1.0` (enough TRF trades) but `bbo_valid = 0.0` (stale BBO) -- in this case, signed flow features are unreliable but volume/count features are valid

---

### 5.10 Context (Indices 31-33)

Always enabled. Provide temporal and schema context.

#### Index 31: `session_progress`

**Fractional position within the trading session. 0.0 at market open, 1.0 at market close.**

**Classification**: State variable (forward-fill on empty bin)

**Formula**:
```
market_open = config.sampling.market_open_et       // default: 09:30 ET
market_close = config.sampling.market_close_et     // default: 16:00 ET
session_duration = market_close - market_open       // default: 6.5 hours = 23400 seconds

session_progress = (bin_midpoint_time - market_open) / session_duration
session_progress = clamp(session_progress, 0.0, 1.0)
```

For half-day auto-detected sessions (auto_detect_close = true), session_progress is adjusted: the actual detected close time replaces `market_close`, so 1.0 corresponds to the actual end of trading, not the standard 16:00 close.

**Units**: Dimensionless ratio
**Range**: [0.0, 1.0]
**Data dependencies**: Bin timestamp, market session config

**E9 validation**: Not directly tested. Standard temporal feature for capturing intraday seasonality (U-shaped volume, spread dynamics).

**Source**: Standard. Related to Bogousslavsky et al. (2024): ITI has intraday U-shaped pattern.

---

#### Index 32: `time_bucket`

**Discrete time-of-day bucket for regime classification.**

**Classification**: Categorical (never normalized)

**Formula**:
```
time_bucket =
    0  if time < 09:35 ET        // open auction
    1  if 09:35 <= time < 10:30  // morning
    2  if 10:30 <= time < 15:00  // midday
    3  if 15:00 <= time < 15:55  // afternoon
    4  if 15:55 <= time < 16:00  // close auction
    5  if time >= 16:00          // post-market
```

Follows the 7-regime classification from hft-statistics and pipeline_contract.toml (mapped to 6 buckets here since pre-market is not covered by XNAS.BASIC processing window). Bucket 0 is "open_auction" (9:30-9:35), not "pre-market" (before 9:30).

**Units**: Integer category
**Range**: {0, 1, 2, 3, 4, 5}
**Data dependencies**: Bin timestamp, DST-aware ET conversion

---

#### Index 33: `schema_version`

**Schema version identifier for contract validation.**

**Classification**: Categorical (never normalized)

**Formula**:
```
schema_version = 1.0    // Constant for schema version 1.0
```

Emitted as a constant in every feature vector. Allows downstream consumers to validate compatibility: if a consumer expects schema 1.0 features and encounters a different version, it must reject the data.

**Units**: Version number
**Range**: {1.0} (current version)
**Data dependencies**: None (constant)

---

## 6. Label Contract

**Point returns ONLY.** No smoothed-average labels. This is the core lesson from E8: smoothed labels produce models that predict the smoothing residual (R^2 = 45%) rather than the tradeable point component (R^2 = 0.02%). All 8 backtests with smoothed labels produced negative returns.

### 6.1 Point Return Formula

```
point_return(t, H) = (mid_price[t + H] - mid_price[t]) / mid_price[t] * 10000    [basis points]
```

Where:
- `mid_price[t]` = Nasdaq BBO midpoint at the end of time bin `t`
- `H` = horizon in bins (NOT seconds; actual time = H * bin_size_seconds)
- The return is measured from the midpoint at the END of bin `t` to the midpoint at the END of bin `t + H`

### 6.2 Multi-Horizon Export

Labels are exported as a 2D array `[N, len(horizons)]` with one column per horizon:

```
horizons = config.labeling.horizons          // default: [1, 2, 3, 5, 10, 20, 30, 60]

labels[i, h] = point_return(t_i, horizons[h])    for h = 0..len(horizons)-1
```

At 60-second bins: H=1 = 1 minute, H=10 = 10 minutes, H=60 = 1 hour.

### 6.3 Forward Prices

Forward mid-price trajectories are exported alongside labels for downstream flexibility:

```
forward_prices[i, k] = mid_price[t_i + k]    for k = 0..max(horizons)

// forward_prices[i, 0] = mid_price at end of bin t_i (the reference price)
// forward_prices[i, H] = mid_price H bins ahead (used for point_return computation)
```

**Shape**: `[N, max(horizons) + 1]` float64, units = USD.

### 6.4 Label Edge Cases

- If `t + H` exceeds the end of the trading day, `point_return(t, H) = NaN`. Sequences with any NaN labels for any configured horizon are **excluded** from the export.
- At end-of-day: the last `max(horizons)` bins cannot produce complete labels and are dropped.
- At end-of-session (half-day close detected): same truncation applies.

---

## 7. Categorical and Non-Normalizable Features

The following features are **never normalized** (excluded from z-score or any other normalization):

| Index | Name | Reason |
|-------|------|--------|
| 29 | bin_valid | Binary indicator {0.0, 1.0} |
| 30 | bbo_valid | Binary indicator {0.0, 1.0} |
| 32 | time_bucket | Discrete category {0, 1, 2, 3, 4, 5} |
| 33 | schema_version | Constant (1.0) |

All other features (indices 0-28, 31) are subject to the configured normalization strategy (`per_day_zscore` by default).

### Non-Normalizable Feature Indices (for contract.rs)

```rust
pub const CATEGORICAL_INDICES: &[usize] = &[29, 30, 32, 33];
```

---

## 8. Empty Bin Policy

When a time bin contains zero trades (or zero TRF trades), features must be handled explicitly to prevent NaN propagation.

### 8.1 State Variables (Forward-Fill)

State variables represent a condition that persists until new data arrives. When a bin has no new data, the previous bin's value is carried forward.

| Index | Feature | Forward-Fill Behavior |
|-------|---------|----------------------|
| 4 | dark_share | Last observed dark_share |
| 8 | subpenny_intensity | Last observed ratio |
| 9 | odd_lot_ratio | Last observed ratio |
| 11 | retail_volume_fraction | Last observed ratio |
| 12 | spread_bps | Last BBO spread |
| 16 | quote_imbalance | Last BBO imbalance |
| 18 | trf_vpin | Last VPIN value |
| 19 | lit_vpin | Last VPIN value |
| 20 | mean_trade_size | Last observed mean |
| 21 | block_trade_ratio | Last observed ratio |
| 23 | size_concentration | Last observed HHI |
| 25 | time_since_burst | Continues incrementing (time passes) |
| 26 | trf_lit_volume_ratio | Last observed ratio |
| 31 | session_progress | Computed from clock (always available) |

### 8.2 Flow Variables (Zero)

Flow variables represent activity that occurred during the bin. No activity = zero.

| Index | Feature | Empty-Bin Value |
|-------|---------|----------------|
| 0 | trf_signed_imbalance | 0.0 |
| 1 | mroib | 0.0 |
| 2 | inv_inst_direction | 0.0 |
| 3 | bvc_imbalance | 0.0 |
| 5 | trf_volume | 0.0 |
| 6 | lit_volume | 0.0 |
| 7 | total_volume | 0.0 |
| 10 | retail_trade_rate | 0.0 |
| 13 | bid_pressure | 0.0 |
| 14 | ask_pressure | 0.0 |
| 15 | bbo_update_rate | 0.0 |
| 17 | spread_change_rate | 0.0 |
| 22 | trade_count | 0.0 |
| 24 | trf_burst_intensity | 0.0 |
| 27 | bin_trade_count | 0.0 |
| 28 | bin_trf_trade_count | 0.0 |

### 8.3 Categorical Features

Categorical features have well-defined values regardless of bin content:

| Index | Feature | Empty-Bin Value |
|-------|---------|----------------|
| 29 | bin_valid | 0.0 (insufficient trades) |
| 30 | bbo_valid | Depends on BBO state freshness |
| 32 | time_bucket | Computed from clock |
| 33 | schema_version | 1.0 (constant) |

### 8.4 Warmup Period

The first `config.validation.warmup_bins` bins per day (default: 3) are discarded before accumulators are stable. These bins are processed (to build BBO state and accumulator history) but their feature vectors are not included in the export.

---

## 9. Precision Chain

Explicit type and unit at every pipeline stage:

```
Databento wire:       i64 nanodollars (FIXED_PRICE_SCALE = 1e-9)
  |
  v
dbn crate decode:     i64 preserved (nanodollar integers)
  |
  v
CmbpRecord:           i64 prices (nanodollars), u32 sizes (shares)
  |
  v
BboState:             f64 prices (USD, converted once at update boundary)
                      Conversion: price_usd = price_nano as f64 * 1e-9
  |
  v
Midpoint computation: f64 USD (from BboState)
  |
  v
Feature vectors:      f64 (all computations in f64)
  |
  v
NPY export:           f32 (downcast at export boundary, with isfinite() check)
                      Labels: f64 (point returns in bps, no downcast)
                      Forward prices: f64 (USD, no downcast)
```

**Division guard**: `EPS = 1e-8` (defined in `src/contract.rs`) for all denominators.
**Float comparison in tests**: `1e-10` (inline literal) for golden test comparisons. Not a public constant in `src/contract.rs`.
**NaN guard**: Every feature vector element is checked with `is_finite()` before NPY export. Any non-finite value is a hard error (`assert!` in `features/mod.rs:159`).

---

## 10. E9 Validation Summary

Consolidated results from E9 and E9 cross-validation (35 test days, 8,337 samples, 60-second bins).

### 10.1 Signal Priority (by IC strength)

| Rank | Feature | Index | Best IC | Best Horizon | ACF(1) | Bootstrap 95% CI (H=10) | Type |
|------|---------|-------|---------|-------------|--------|--------------------------|------|
| 1 | trf_signed_imbalance | 0 | +0.103 | H=1 | 0.093 | [+0.019, +0.060] | Fast directional |
| 2 | subpenny_intensity | 8 | +0.104 | H=60 | 0.889 | [+0.027, +0.069] | Slow state |
| 3 | dark_share | 4 | +0.051 | H=1 | 0.418 | [+0.013, +0.056] | Regime indicator |
| 4 | retail_trade_rate | 10 | +0.027 | H=10 | 0.976 | -- | Activity level |
| 5 | mroib | 1 | +0.021 | H=10 | 0.050 | -- | Marginal |
| 6 | odd_lot_ratio | 9 | +0.018 | H=10 | 0.886 | -- | Marginal |
| -- | inv_inst_direction | 2 | -0.021 | H=10 | 0.050 | -- | Negative IC |
| -- | Volume features | 5-7 | < 0.03 | H=10 | -- | -- | Non-directional |

### 10.2 Horizon Dynamics

| Feature | H=1 IC | H=10 IC | H=60 IC | Dynamics |
|---------|--------|---------|---------|----------|
| trf_signed_imbalance | **+0.103** | +0.040 | +0.050 | Fast decay, partial recovery (two-timescale) |
| subpenny_intensity | +0.023 | +0.048 | **+0.104** | Increases with horizon (accumulating state) |
| dark_share | **+0.051** | +0.035 | -0.013 | Short-term regime, mean-reverts (sign flip) |

### 10.3 Contemporaneous vs Predictive

| Feature | IC(current return) | IC(H=10 future) | Classification |
|---------|-------------------|-----------------|----------------|
| trf_signed_imbalance | +0.033 | +0.040 | Predictive (0.033 < 2 x 0.040) |
| subpenny_intensity | +0.011 | +0.048 | Predictive |
| dark_share | +0.036 | +0.035 | Predictive (borderline) |

None classified as purely contemporaneous (defined as IC_current > 2 x IC_future). This is the critical difference from MBO OFI which IS purely contemporaneous (lag-1 IC < 0.006 at ALL scales).

### 10.4 Lagged IC Decay

| Lag (bins) | trf_signed_imb | subpenny_int | dark_share |
|-----------|----------------|--------------|------------|
| 0 | +0.040 | +0.048 | +0.035 |
| 1 | +0.014 | +0.041 | +0.016 |
| 2 | +0.018 | +0.041 | +0.013 |
| 5 | +0.025 | +0.044 | +0.008 |
| 10 | +0.034 | +0.044 | +0.017 |

**subpenny_intensity** is remarkably stable across lags -- confirmed as a genuine slow-moving state variable, not contemporaneous.

### 10.5 Features NOT Tested in E9

The following features are new to this pipeline and were not validated in E9:

- bvc_imbalance (3): BVC-based, complementary to midpoint signing
- retail_volume_fraction (11): Retail participation level
- All bbo_dynamics (12-17): L1 quote dynamics
- trf_vpin (18), lit_vpin (19): Volume-synchronized toxicity
- All trade_size (20-23): Distribution analysis
- All cross_venue (24-26): Burst detection and venue interaction

These features require implementation before validation. Signal-first validation (compute IC before model training) is mandatory per pipeline development rules.

---

## Appendix A: Feature Count Formula

```
total = 4  (signed_flow,     if enabled)
      + 4  (venue_metrics,   if enabled)
      + 4  (retail_metrics,  if enabled)
      + 6  (bbo_dynamics,    if enabled)
      + 2  (vpin,            if enabled)
      + 4  (trade_size,      if enabled)
      + 3  (cross_venue,     if enabled)
      + 2  (activity,        always)
      + 2  (safety_gates,    always)
      + 3  (context,         always)
      ─────
      = 34 (all groups enabled)
      = 7  (minimum: activity + safety_gates + context only)
```

When groups are disabled, feature indices are NOT remapped. Disabled features are simply not computed and not emitted. The feature vector length equals the number of enabled features, but index assignments are fixed (a feature's semantic meaning is always tied to its defined index).

**Implementation note**: The `contract.rs` file defines `GROUP_OFFSET` and `GROUP_COUNT` constants for each group, enabling runtime feature vector construction from the enabled group list while preserving index semantics in metadata.

---

## Appendix B: Contract Verification

The feature specification is registered in `contracts/pipeline_contract.toml` under `[features.off_exchange]`:

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

Local Rust constants in `src/contract.rs` mirror these values. `verify_rust_constants.py` validates Rust source against the TOML. Python constants are auto-generated via `generate_python_contract.py` under `OffExchangeFeatureIndex` for downstream consumers.

Any modification to stable feature indices (0-33) or label encoding constitutes a **breaking change** and requires a `schema_version` bump.
