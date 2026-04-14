# Theoretical Foundation: Off-Exchange Signal Extraction for NVDA

**Status**: Reference Document — **Implementation Status**: Phases 1-5 complete (412 tests)
**Date**: 2026-03-22 (spec), 2026-03-23 (implementation)
**Scope**: Mathematical and statistical foundations for off-exchange feature extraction from XNAS.BASIC CMBP-1 data
**Research base**: 47 peer-reviewed papers and working papers, 2 synthesis documents, E9/E9-CV empirical validation (35 days, 8,337 samples)

---

## Table of Contents

1. [Data Architecture](#1-data-architecture)
2. [Trade Classification Theory](#2-trade-classification-theory)
3. [Dark Pool Equilibrium Theory](#3-dark-pool-equilibrium-theory)
4. [Retail Flow Theory](#4-retail-flow-theory)
5. [Core Signal Mathematics](#5-core-signal-mathematics)
6. [Off-Exchange Feature Definitions](#6-off-exchange-feature-definitions)
7. [Cross-Venue Interaction Theory](#7-cross-venue-interaction-theory)
8. [Empirical Validation](#8-empirical-validation)
9. [Implementation Pitfalls](#9-implementation-pitfalls)
10. [Signal Priority Hierarchy](#10-signal-priority-hierarchy)

---

## 1. Data Architecture

### 1.1 XNAS.BASIC CMBP-1 Schema

The XNAS.BASIC dataset provides consolidated market-by-price Level-1 data for NVDA from the Nasdaq ecosystem. Schema **CMBP-1** includes top-of-book updates plus trades with publisher attribution.

**Record fields** (Python Databento SDK view / Rust `dbn::CbboMsg` mapping):

| Field (Python SDK) | Rust `CbboMsg` field | Type | Unit | Description |
|---------------------|---------------------|------|------|-------------|
| `ts_event` | `hd.ts_event` | u64 | UTC nanoseconds | Event timestamp |
| `ts_recv` | `ts_recv` | u64 | UTC nanoseconds | Capture/receipt timestamp |
| `action` | `action` | c_char | enum | `T` = trade, `A` = BBO update |
| `side` | `side` | c_char | enum | `N` for TRF trades (unsigned), `A`/`B` for lit quotes |
| `price` | `price` | i64 | nanodollars | Trade price (i64 fixed-point, multiply by 1e-9 for USD) |
| `size` | `size` | u32 | shares | Trade size or BBO size |
| `bid_px_00` | `levels[0].bid_px` | i64 | nanodollars | Best bid price (Nasdaq only, i64 fixed-point) |
| `ask_px_00` | `levels[0].ask_px` | i64 | nanodollars | Best ask price (Nasdaq only, i64 fixed-point) |
| `bid_sz_00` | `levels[0].bid_sz` | u32 | shares | Best bid size |
| `ask_sz_00` | `levels[0].ask_sz` | u32 | shares | Best ask size |
| `bid_ct_00` | N/A | u32 | count | Number of orders at best bid (**Python SDK only**; Rust `ConsolidatedBidAskPair` has `bid_pb: u16` publisher ID instead) |
| `ask_ct_00` | N/A | u32 | count | Number of orders at best ask (**Python SDK only**; Rust has `ask_pb: u16` instead) |
| `flags` | `flags` | FlagSet | bitfield | Includes TRF indicator |
| `publisher_id` | `hd.publisher_id` | u16 | enum | Venue identifier (on RecordHeader) |

**Rust vs Python field discrepancy**: The Rust `dbn` crate v0.20.0 uses `ConsolidatedBidAskPair` for CMBP-1 records, which stores `bid_pb`/`ask_pb` (publisher IDs at the BBO level) where the Python SDK exposes `bid_ct_00`/`ask_ct_00` (order counts). No feature formulas in this pipeline depend on order counts, so this does not affect implementation. All prices in the Rust crate are i64 nanodollars (FIXED_PRICE_SCALE = 1e-9), NOT f64 USD — conversion to f64 USD happens at the `BboState.update_from_record()` boundary.

### 1.2 Publisher ID Mapping

Six publisher IDs are present in the XNAS.BASIC feed for NVDA, validated across all 35 test days:

| Publisher ID | Venue | Type | Share of Trades | Share of Volume |
|-------------|-------|------|-----------------|-----------------|
| 81 | XNAS | Nasdaq lit market | ~31% | ~38% |
| 82 | FINN | FINRA TRF Carteret | ~67% | ~60% |
| 83 | FINC | FINRA TRF Chicago | ~2% | ~1% |
| 88 | XBOS | Nasdaq BX (lit exchange) | ~0.2% | ~0.1% |
| 89 | XPSX | Nasdaq PSX (lit exchange) | ~0.2% | ~0.1% |
| 93 | -- | Consolidated BBO quotes only | 0% trades | 0% (quotes only) |

Publishers 82 + 83 are FINRA Trade Reporting Facilities. All off-exchange transactions in NMS stocks executed otherwise than on an exchange must be reported to a FINRA facility such as a TRF (FINRA Rules 6282, 6380A, 6380B; SEC oversight under Exchange Act Section 15A). FINRA currently operates three active TRFs: two FINRA/Nasdaq TRFs (Carteret and Chicago, publisher IDs 82 and 83) and one FINRA/NYSE TRF.

Publisher 93 emits only BBO quote updates (~5.6M records/day) and zero trades. Publishers 88 and 89 are minor Nasdaq-affiliated lit exchanges with negligible volume (~0.2% combined).

### 1.3 Coverage Validation Against EQUS_SUMMARY

EQUS_SUMMARY (Databento `EQUS.SUMMARY` dataset, OHLCV-1D schema) provides the ground-truth consolidated volume across ALL lit exchanges, dark pools, and off-exchange venues for the official consolidated tape. It does NOT break down volume by venue.

**5-day cross-check (test period)**:

| Day | Consolidated Vol | XNAS.BASIC Vol | Coverage | True Dark (TRF/consol) | E9 Dark (TRF/(TRF+lit)) |
|-----|-----------------|----------------|----------|----------------------|------------------------|
| 20251114 | 186,591,856 | 152,064,596 | 81.5% | 49.7% | 61.2% |
| 20251117 | 173,628,858 | 146,537,710 | 84.4% | 48.6% | 57.7% |
| 20251201 | 188,125,478 | 158,186,694 | 84.1% | 53.3% | 63.5% |
| 20251229 | 120,040,226 | 97,677,022 | 81.4% | 52.9% | 65.1% |
| 20260106 | 176,890,516 | 149,765,850 | 84.7% | 53.0% | 62.7% |

**Key findings**:
- XNAS.BASIC covers **81-85%** of consolidated NVDA volume, consistent across days
- **True TRF share = 49-53%** of consolidated volume (FINRA TRF Carteret + Chicago)
- E9's per-bin dark_share of ~76% is NOT the true market dark share. It is TRF/(TRF+XNAS_lit_only), which overstates because the denominator excludes 15-19% of volume from other lit exchanges (ARCX/NYSE Arca, BATS/EDGX, IEX, MEMX, etc.)
- The additional discrepancy between E9's day-total dark_share (~61%) and per-bin average (~76%) is explained by **session composition**: E5 time bins cover 09:50-14:55 ET, excluding post-market hours where XNAS lit dominates and dark share drops to 24.5%. Within the E5 window, dark share is 75.5%.

### 1.4 What XNAS.BASIC Can and Cannot Compute

**CAN compute** (from CMBP-1 fields):
- Trade prices, sizes, timestamps, publisher attribution
- Best bid/offer (Nasdaq-only BBO, NOT full NBBO)
- Trade signing via midpoint comparison (Barber et al. 2024 method)
- Retail trade identification via subpenny detection (BJZZ method)
- Volume decomposition by venue (TRF vs lit)
- VPIN and BVC (require only trade prices and volumes)
- Time-of-day features, trade rate, odd-lot counts

**CANNOT compute** (missing data):
- Aggressor side for TRF trades (`side='N'`) — requires classification algorithms
- Full NBBO (only Nasdaq BBO available; other exchanges' quotes not in feed)
- Order lifecycle events (add, cancel, modify) — no order book reconstruction
- Queue position or depth beyond Level 1
- Order IDs or order flow per trader
- OFI or MLOFI (require full limit order book state changes)

**Relationship to MBO feeds**: XNAS.BASIC (L1 + trades) is complementary to XNAS.ITCH MBO (full 20-level book + order events). MBO provides OFI/MLOFI signal; XNAS.BASIC provides off-exchange venue decomposition. Neither alone captures both.

### 1.5 Dataset Specifications

| Property | Value |
|----------|-------|
| Instrument | NVDA (NVIDIA Corporation) |
| Source | Databento XNAS.BASIC, schema CMBP-1 |
| Period | 235 trading days (2025-02-03 to 2026-01-08) |
| Compressed size | 22.03 GB |
| File sizes | 33.7 MB (min) to 285.5 MB (max), median 82.8 MB |
| File pattern | `xnas-basic-YYYYMMDD.cmbp-1.dbn.zst` |
| Train/Val/Test | Same 163/35/35 day split as MBO experiments |
| Records per day | ~3M to 16M (varies with market activity) |
| TRF trades per day | ~1.0M to 2.1M |

---

## 2. Trade Classification Theory

Signing off-exchange prints — determining whether a trade was buyer-initiated or seller-initiated — is the foundational step for constructing directional signals from TRF data. The aggressor side is not reported for TRF trades in the XNAS.BASIC feed (`side='N'`), requiring algorithmic classification.

### 2.1 Lee-Ready Algorithm

**Source**: Lee, C.M.C. and M.J. Ready (1991). "Inferring Trade Direction from Intraday Data." *The Journal of Finance*, 46(2), 733-746.

**Algorithm**:
1. **Quote rule**: Compare trade price to prevailing midquote
   - Price > midquote → **buy**
   - Price < midquote → **sell**
2. **Tick test** (fallback for midquote trades):
   - Price > previous different price (uptick) → **buy**
   - Price < previous different price (downtick) → **sell**
3. Original recommendation: use quotes lagged by 5 seconds to account for quote-trade asynchrony

**Modern accuracy**: The Lee-Ready algorithm was designed for 1991 NYSE data where it achieved ~90% accuracy. On modern data:
- Chakrabarty, Moulton, and Shkilko (2012, *Journal of Financial Markets*, 15(4), 467-491): **31-32% misclassification** on Nasdaq, reducible to ~21% with 1-second lag
- Degradation causes: HFT quote flickering (median 17 quote changes/second in active periods), sub-penny executions, and the dominance of inside-the-spread trades

**Relevance to TRF prints**: Lee-Ready is the baseline reference but performs poorly on TRF prints because: (a) the Nasdaq BBO may differ from the NBBO at execution time, (b) TRF reporting delays introduce additional asynchrony, and (c) many TRF prints execute inside the spread (midpoint crosses, sub-penny improvements) where all classification algorithms perform worst.

### 2.2 EMO Algorithm

**Source**: Ellis, K., R. Michaely, and M. O'Hara (2000). "The Accuracy of Trade Classification Rules: Evidence from Nasdaq." *JFQA*, 35(4), 529-551.

**Algorithm**: Reverses Lee-Ready's priority for inside-spread trades:
- Price = ask → **buy**
- Price = bid → **sell**
- Otherwise → tick test

Achieves 83.7% accuracy on Nasdaq. Critical finding: all algorithms perform worst on inside-the-spread trades — the exact population dominating TRF midpoint crosses and wholesaler prints.

### 2.3 CLNV Algorithm

**Source**: Chakrabarty, B., B. Li, V. Nguyen, and R.A. Van Ness (2007). "Trade Classification Algorithms for Electronic Communications Network Trades." *Journal of Banking & Finance*, 31(12), 3806-3821.

**Algorithm**:
- Trade price from ask to 30% below ask → **buy**
- Trade price from bid to 30% above bid → **sell**
- Trade within 40% of midpoint, or outside quotes → tick test

### 2.4 Full-Information (FI) Algorithm

**Source**: Jurkatis, S. (2022). "Inferring Trade Directions in Fast Markets." *Journal of Financial Markets*, 58, 100635.

The FI algorithm identifies the "footprint" of a trade in the order book by matching volume changes, rather than relying on timestamp-based quote alignment.

**Key equation — change in volume at the j-th ask level** (Eq. in Section 3):

```
Δv_j^a = v_j^a - v_{j+1}^a    if a_j = a_{j+1}     (same price, volume consumed)
        = v_j^a                 if a_j < a_{j+1}     (price improved, all old volume consumed)
        = -1                    if a_j > a_{j+1}     (new better limit order, no trade)
```

A trade against the ask reduces ask volume; a trade against the bid reduces bid volume. The algorithm simultaneously determines quote correspondence and trade classification.

**Empirical performance** (NASDAQ TotalView-ITCH, May-Jul 2011, 134M transactions):
- At nanosecond timestamp precision: all algorithms achieve ~99%+ accuracy
- At second precision: FI = **95%** accuracy vs LR/EMO/CLNV = **~90%** (reduces misclassification by half)
- Timestamp interpolation (Holden-Jacobsen method: `t = s + (2i-1)/(2I)` for i-th trade in second s) **decreases** accuracy for traditional algorithms
- **Economic value**: A risk-averse investor would forgo up to **33 bps annually** to use FI over LR for transaction cost estimation (difference: 46 bps LR fees vs 13 bps FI fees)

**Applicability to TRF**: FI requires visible order book state (lit-market LOB). Cannot apply directly to TRF prints. However, the principle is applicable: use XNAS.ITCH order book events within a time window around each TRF print to determine which side of the book was consumed.

### 2.5 BJZZ Retail Trade IDENTIFICATION

**Source**: Boehmer, E., C.M. Jones, X. Zhang, and X. Zhang (2021). "Tracking Retail Investor Activity." *The Journal of Finance*, 76(5), 2249-2305.

**IMPORTANT DISTINCTION**: BJZZ is a retail trade **identification** method, NOT a trade **signing** method. It determines which TRF prints are likely retail-originated; a separate method (midpoint comparison or tick test) determines buy/sell direction.

**Theoretical basis**: SEC Reg NMS Rule 612 (Sub-Penny Rule) prohibits displaying or accepting quotations in NMS stocks in increments less than $0.01 for stocks priced above $1.00. However, sub-penny **executions** are permitted — and wholesalers routinely provide sub-penny price improvement on internalized retail orders. This creates a detectable signature.

**Identification formula** (equivalent mathematical forms):

```
Z = 100 × mod(Price, 0.01)                     — original notation
frac_cent = (Price × 100) mod 1                 — equivalent computation

Identification rule:
  Z ∈ (0, 0.40) → trade is likely RETAIL        (sub-penny price improvement on sell)
  Z ∈ (0.60, 1.0) → trade is likely RETAIL      (sub-penny price improvement on buy)
  Z = 0 (round penny) → EXCLUDED                (likely institutional or lit execution)
  Z ∈ [0.40, 0.60] (near half-penny) → EXCLUDED (likely institutional midpoint cross)
```

The sub-penny digit reflects the **price improvement** offered by wholesalers. A trade at $100.003 (Z=0.3) suggests a retail sell executed at $0.003 above the bid; a trade at $100.007 (Z=0.7) suggests a retail buy executed at $0.007 below the ask.

**Validation with our NVDA data (E9)**:
- 72.55% of TRF trades have subpenny pricing (Z ≠ 0)
- Retail identification rate: 45.3% (trades with Z in (0, 0.4) ∪ (0.6, 1.0))
- Remaining 27.45% are round-penny TRF trades (excluded from retail classification)

**Published accuracy** (Boehmer et al. 2021):
- Cross-validation with Nasdaq TRF proprietary data: 98.2% signing accuracy for subpenny trades

**Limitations** (validated by subsequent literature):
- Identifies only **35%** of actual retail trades (65% false negative rate; Barber et al. 2024)
- **28% signing error** when using the sub-penny digit alone for direction (Barber et al. 2024)
  - At 1-penny spread: ~93% signing accuracy
  - At 10+ cent spread: drops by 40.5 percentage points (to ~52%, near random)
- **24.45% of known institutional trades** executed by wholesalers also print at sub-penny prices (Battalio, Jennings, Saglam, and Wu 2022, University of Notre Dame working paper), contaminating retail identification

### 2.6 Midpoint Signing Method

**Source**: Barber, B.M., X. Huang, P. Jorion, T. Odean, and C. Schwarz (2024). "A (Sub)penny for Your Thoughts: Tracking Retail Investor Activity in TAQ." *The Journal of Finance*, 79(4), 2403-2427.

**Validation dataset**: 85,000 trades placed in 6 brokerage accounts at 5 retail brokers (E*Trade, Fidelity, Interactive Brokers Pro, Interactive Brokers Lite, Robinhood, TD Ameritrade) between December 2021 and June 2022.

**Algorithm** (recommended replacement for BJZZ sub-penny digit signing):
```
Trade price > NBBO midpoint → BUY
Trade price < NBBO midpoint → SELL
Exclude trades between 40% and 60% of NBBO spread
```

**Performance comparison**:

| Method | Signing Accuracy (EW) | Variation Across Spreads |
|--------|----------------------|-------------------------|
| BJZZ sub-penny digit | 72.2% | 93% at 1¢ spread, drops 40.5pp at 10+¢ |
| Quote midpoint | **94.8%** | Uniform across spread levels |

The midpoint method's key advantage is stability: accuracy does not degrade as spreads widen, whereas BJZZ accuracy collapses for wide-spread conditions.

**E9 implementation details**:
- Uses Nasdaq BBO midpoint (not full NBBO, which is unavailable in XNAS.BASIC)
- Exclusion band: `price > mid + 0.1 × spread` for buy, `price < mid - 0.1 × spread` for sell
- **Methodology note**: E9 uses `0.1 × spread` (full spread) as the exclusion threshold. The BJZZ literature convention uses `0.1 × half_spread`. Our implementation leaves 15.4% of trades unsigned vs 12.2% with the standard convention, affecting ~46,000 additional trades per day. This is a conservative choice (fewer misclassified trades at the cost of more unsigned trades). Impact on IC values is minor.
- Trades with spread ≤ 0 are excluded via `valid_bbo = isfinite(mid) & (spread > 0)`. Empirically, 100% of TRF trades in the test period have valid BBO.

### 2.7 Bulk Volume Classification (BVC)

**Source**: Easley, D., M. López de Prado, and M. O'Hara (2012). "Flow Toxicity and Liquidity in a High-Frequency World." *Review of Financial Studies*, 25(5), 1457-1493.

BVC classifies volume in aggregate using normalized price changes, completely sidestepping individual trade signing:

```
V_τ^B = Σ_{i=t(τ-1)+1}^{t(τ)} V_i × Φ((P_i - P_{i-1}) / σ_{ΔP})     (Eq. 7)
V_τ^S = V - V_τ^B
```

where Φ is the CDF of the standard normal distribution, V_i is the volume of the i-th trade, and σ_{ΔP} is the standard deviation of price changes (typically computed from 1-minute bars).

**Advantages for TRF data**:
- No timestamp alignment needed (no quote-matching)
- No individual trade signing errors to propagate
- Naturally aggregates across unknown trade types

**Accuracy tradeoff**: BVC produces 7-16 percentage points more misclassification than tick rules at the individual trade level (Chakrabarty, Pascual, and Shkilko 2015, *Journal of Financial Markets*, 25, 52-79). However, Panayides, Shohfi, and Smith (2019, *Journal of Banking and Finance*, 103, 113-129) found BVC-estimated order flow was the **only algorithm correlated with proxies of informed trading** — suggesting BVC captures information content better than classification accuracy implies.

### 2.8 Microprice as Superior Reference

**Source**: Stoikov, S. (2018). "The Micro-Price: A High-Frequency Estimator of Future Prices." *Quantitative Finance*, 18(12), 1959-1966.

The microprice adjusts the raw midpoint using order book imbalance to produce a better estimator of the true efficient price:

**Imbalance**:
```
I = Q^b / (Q^b + Q^a)                                          (Eq. 3)
```
where Q^b = best bid size, Q^a = best ask size.

**Microprice**:
```
P^{micro} = M + g(I, S)                                        (Eq. 4)
```
where M = midpoint, S = spread, and g(I, S) is the expected price adjustment given imbalance I and spread S.

**Computation via discrete Markov chain**: Model the LOB state as a discrete-state Markov chain with state vector X = (I, S). Compute transition matrices:
- Q_{xy}: probability of transitioning from state x to state y without a mid-price change
- R_{xk}: probability of a mid-price change of k half-ticks from state x
- T_{xy}: probability of transitioning with a mid-price change

**First-order adjustment**:
```
G^1(x) = (1 - Q)^{-1} × R × K                                 (Section 4)
```
where K = [-0.01, -0.005, 0.005, 0.01]^T (half-tick price change values).

**Higher-order recursion**:
```
G^{i+1}(x) = (1 - Q)^{-1} × T × G^i(x)                       (Eq. 7)
B = (1 - Q)^{-1} × T
P^{micro} = M + G^1 + Σ_{j=2}^{n_m} (λ_j / (1 - λ_j)) × B_j × G^1
```

**Convergence** (Theorem 3.1): If B* = lim_{k→∞} B^k and B* × G^1 = 0, the microprice converges. Convergence time: ~3 minutes for large-tick stocks (BAC), ~10 seconds for small-tick stocks (CVX).

**Relevance to TRF signing**: Using the microprice instead of the raw midpoint as the reference for classifying TRF prints would reduce effective spread estimation bias by ~18% (Hagströmer, "Bias in the Effective Bid-Ask Spread," working paper). This requires XNAS.ITCH MBO data (for queue sizes Q^b, Q^a) to be available alongside XNAS.BASIC data in the same session.

### 2.9 Implementation Decision

Our E9 implementation and future production pipeline use the **Barber et al. (2024) midpoint signing method** with a 10% spread exclusion band. This is the recommended approach based on:
1. 94.8% accuracy validated on actual retail trades
2. Uniform accuracy across spread levels
3. No dependency on full LOB state (works with L1 BBO)
4. Conservative exclusion reduces noise at the cost of unsigned volume

For production, upgrading to **microprice-based signing** (Stoikov 2018) would further improve accuracy when MBO data is co-located, but adds architectural complexity.

---

## 3. Dark Pool Equilibrium Theory

### 3.1 Execution Risk Asymmetry

**Source**: Zhu, H. (2014). "Do Dark Pools Harm Price Discovery?" *Review of Financial Studies*, 27(3), 747-789.

This is the most important theoretical model for interpreting TRF volume patterns.

**Model setup**: Asset value v is equally likely to be +σ or -σ. Informed traders (mass μ_I) observe v; uninformed liquidity traders (mass random, from distribution Z) do not. Fraction β of informed traders route to the dark pool; fraction α_e of uninformed route to the exchange; fraction α_d to the dark pool.

**Exchange spread** (zero-profit condition for market makers, Eq. 3):
```
S = ((1-β) × μ_I) / ((1-β) × μ_I + α_e × μ_z) × σ
```

**Dark pool crossing probabilities** (Eqs. 5-6):
```
r⁻ = E[min(1, α_d × Z⁻ / (α_d × Z⁺ + β × μ_I))]         (Eq. 5, sell-side)
r⁺ = E[min(1, (α_d × Z⁻ + β × μ_I) / (α_d × Z⁺))]       (Eq. 6, buy-side)
```

**Key inequality** (Eq. 7):
```
1 > r⁺ > r⁻ > 0     for all β > 0
```

This inequality is the core result: informed traders face **lower** execution probability (r⁻) than uninformed traders (r⁺) in dark pools, because informed orders cluster on the same side (all informed traders buy when v = +σ, creating an imbalance that reduces their fill rate).

**Signal-to-noise ratio** (price discovery measure, Eq. 35):
```
I(β, α_e) = (1-β) × μ_I / (α_e × σ_z)
```

This measures the ratio of informed order flow ("signal") to uninformed order flow standard deviation ("noise") on the exchange. Higher I = better price discovery on lit exchanges.

**Key predictions**:
1. **Dark pools IMPROVE price discovery** under natural conditions by concentrating informed flow on lit exchanges (informed traders exit dark pools because of low fill rates)
2. **Non-linear dark share**: Dark pool market share increases with low-to-moderate volatility but **decreases** during high volatility (traders flee to lit markets for execution certainty)
3. Adding a dark pool widens exchange spreads but improves the signal-to-noise ratio
4. Higher volatility increases informed dark pool participation but can reduce overall dark share (hump-shaped relationship)

**Empirical context**: Dark pool market share in the US roughly doubled from ~6.5% (July 2008) to ~12% (June 2011), based on Tabb Group and Rosenblatt Securities estimates (p. 749).

**Pipeline implication**: When dark_share is abnormally HIGH for NVDA, it means lit exchange order flow is MORE informationally dense (uninformed flow has migrated to dark pools). OFI signals from XNAS.ITCH should be weighted more heavily in this regime. Conversely, a sudden DROP in dark share during rising volatility signals informed traders returning to lit exchanges.

### 3.2 Inverted-U Relationship: Non-Block Dark Trading

**Source**: Comerton-Forde, C. and T.J. Putniņš (2015). "Dark Trading and Price Discovery." *Journal of Financial Economics*, 118(1), 70-92.

Empirically demonstrates a non-linear (inverted-U) relationship between dark trading and market quality:
- **Low levels** of non-block dark trading: benign or slightly beneficial (reduces exchange noise)
- **High levels** of non-block dark trading: harmful — increases adverse selection on lit venues and impedes informational efficiency
- **Block dark trades** (large-size executions) **do NOT impede** price discovery at any level

**Pipeline implication**: Decompose TRF prints by trade size. Large prints (blocks, e.g., > 10,000 shares for NVDA) should be treated differently — they are uninformative about short-term direction. The adverse selection signal lives in **small, frequent non-block dark prints**, which are dominated by wholesaler internalization and retail flow.

### 3.3 Dark LOB vs Midpoint Crossing

**Source**: Foley, S. and T.J. Putniņš (2016). "Should We Be Afraid of the Dark? Dark Trading and Market Quality." *Journal of Financial Economics*, 122(3), 456-481.

Uses natural experiments (regulatory restrictions on dark trading) to disaggregate dark trading into two types:

| Type | Mechanism | Market Quality Effect |
|------|-----------|---------------------|
| Dark limit order markets | Two-sided, price improvement | Beneficial |
| Midpoint crossing systems | One-sided matching | No consistent benefit |

**Classification from trade prices**:
```
|trade_price - midquote| ≤ ε → midpoint crossing activity
Other within-spread prices   → dark LOB activity (non-midpoint price improvement)
```

**Pipeline implication**: Track the ratio of midpoint-executed vs non-midpoint TRF prints as a separate regime feature. A shift toward midpoint crosses may indicate different market conditions (less wholesaler activity, more institutional crossing).

### 3.4 Venue Pecking Order

**Source**: Menkveld, A.J., B.Z. Yueshen, and H. Zhu (2017). "Shades of Darkness: A Pecking Order of Trading Venues." *Review of Financial Studies*, 30(12), 4321-4372.

Proposes and validates a **pecking order** of trading venues: investors sort venues by the tradeoff between cost (price improvement) and immediacy (execution certainty). The model predicts:
- Urgency/uncertainty shocks shift the venue composition predictably
- In calm conditions: more flow to dark pools and midpoint venues (lower cost)
- In volatile conditions: flow migrates to lit exchanges (higher immediacy)

**Pipeline implication**: The composition of TRF print types (midpoint vs sub-penny vs round-penny) should shift with intraday volatility and spread. Tracking these shifts provides a state variable for the regime detection system. In the XNAS.BASIC data, we can approximate this decomposition using trade price location relative to BBO.

### 3.5 Dark Trading and Information Acquisition

**Source**: Brogaard, J. and J. Pan (2022). "Dark Pool Trading and Information Acquisition." *Review of Financial Studies*, 35(5), 2625-2666.

Using FINRA ATS Transparency Data and the SEC Tick-Size Pilot as an exogenous shock, finds that more dark trading leads to **greater firm-specific fundamental information** in stock prices. The mechanism: dark trading encourages information acquisition by reducing the cost of informed trading (measured via SEC EDGAR search activity). Higher dark pool share for NVDA may indicate more active information acquisition by investors.

### 3.6 Strategic Informed Trading in Dark Pools

**Source**: Ye, M. and W. Zhu (2020). "Strategic Informed Trading and Dark Pools." Working paper.

Empirically validates using Schedule 13D filings (activist investor disclosure) that dark pool market share **increases** when an informed trader trades:
- One standard deviation increase in information value raises dark share by **5.8%**
- Simultaneously, price discovery declines by **9.7%**

**Pipeline implication**: Intraday spikes in TRF volume share relative to its rolling average could signal large informed traders hiding activity in dark pools, predicting subsequent directional moves when the information becomes public.

### 3.7 Hidden Liquidity Model

**Source**: Avellaneda, M., J. Reed, and S. Stoikov (2011). "Forecasting Prices from Level-I Quotes in the Presence of Hidden Liquidity." *Journal of Computational Finance*, 14(3), 35-61.

Models the probability of an upward mid-price move accounting for hidden liquidity from off-exchange venues and iceberg orders:

**Poisson queue dynamics**:
- λ = limit order arrival rate at bid
- μ = market order / cancellation arrival rate at bid
- η = simultaneous bid cancellation + ask limit order rate
- ρ = -η / (λ + η) (correlation between bid and ask queue changes, Eq. 2.3)

**Probability of upward move** (perfectly negatively correlated queues, Eq. 4.1):
```
p(x, y; H) = (x + H) / (x + y + 2H)
```
where x = bid queue size, y = ask queue size, H = hidden liquidity parameter. (Higher bid queue x → higher up-move probability, consistent with buy pressure.)

**Hidden liquidity estimation** (Eq. 4.2):
```
min_H Σ_{i,j} [u_{ij} - (i + H) / (i + j + 2H)]² × d_{ij}
```

**General solution** (Theorem 3.1, any correlation ρ, Eq. 3.8):
```
u(x, y) = (1/2) × (1 - Arctan(√((1+ρ)/(1-ρ)) × (y-x)/(y+x)) / Arctan(√((1+ρ)/(1-ρ))))
```

**Special cases**:
- ρ = 0: u(x, y) = (2/π) × Arctan(x/y) (Eq. 3.9)
- ρ → -1: u(x, y) = x / (x + y) — the standard queue imbalance formula (Eq. 3.10)

**Empirical findings** (implied hidden liquidity H):

| Ticker | NASDAQ H | NYSE H | BATS H |
|--------|----------|--------|--------|
| XLF | 0.15 | 0.17 | 0.17 |
| QQQQ | 0.21 | 0.04 | 0.18 |
| JPM | 0.17 | 0.17 | 0.11 |
| AAPL (s=1) | 0.16 | 0.90 | 0.65 |

Smaller H = more informative visible quotes. H increases with spread.

**Pipeline implication**: Hidden liquidity from off-exchange venues (TRF) explains why the standard queue imbalance formula `I = bid/(bid+ask)` underpredicts mid-price moves. For NVDA, where ~50% of volume is off-exchange, the hidden liquidity parameter H should be substantial. The adjusted probability `p(x,y;H)` provides a more accurate prediction than raw queue imbalance.

---

## 4. Retail Flow Theory

### 4.1 Mroib Construction

**Source**: Boehmer, E., C.M. Jones, X. Zhang, and X. Zhang (2021). "Tracking Retail Investor Activity." *The Journal of Finance*, 76(5), 2249-2305.

**Market-wide retail order imbalance** (Eqs. 1-2):
```
mroib_vol(i,t) = (mrb_vol(i,t) - mrs_vol(i,t)) / (mrb_vol(i,t) + mrs_vol(i,t))     (Eq. 1)
mroib_trd(i,t) = (mrb_trd(i,t) - mrs_trd(i,t)) / (mrb_trd(i,t) + mrs_trd(i,t))     (Eq. 2)
```
where mrb = marketable retail buy, mrs = marketable retail sell; vol = volume-weighted, trd = trade-count-weighted.

**Summary statistics**: Mroib_vol mean = -0.038, std = 0.464 (slightly more selling than buying on average across the cross-section of stocks).

**Return predictability** (Table III, Fama-MacBeth regressions):
- Mroib_vol coefficient for next-week returns: 0.0009 (t = 15.60)
- Interquartile weekly return difference: **10.89 bps** (5.66%/year annualized)
- Small stocks: 21.9 bps/week (11.39%/year); Large stocks: 2.6 bps/week (1.35%/year)
- Predictability persists for 6-8 weeks ahead (statistically significant through week 8)
- 1-week value-weighted long-short alpha: 0.092% raw, 0.084% FF3-adjusted

**Decomposition** of predictive power (Table V):
- ~50% from **persistence + contrarian** behavior (retail provides liquidity at short horizons, then mean-reverts)
- ~50% from residual "other" component (potential informed component or selection effects)
- Public news explains negligible predictive power

**E9 validation for NVDA intraday**: Mroib IC = +0.021 at H=10 (60s bins, marginal), ACF(1) = 0.050 (near-zero persistence). Cross-sectional weekly Mroib predictability does NOT transfer to intraday single-stock prediction. This is expected: the weekly effect relies on cross-sectional diversification across thousands of stocks, which single-stock intraday analysis cannot replicate.

### 4.2 Institutional Inverse Signal

**Source**: Barardehi, Y.H., D. Bernhardt, Z. Da, and M. Warachka (2021, 2025). "Institutional Liquidity Costs, Internalized Retail Trade Imbalances, and the Cross-Section of Stock Returns." *Journal of Financial and Quantitative Analysis* (forthcoming). Working paper SSRN: 3966059.

The most actionable finding for our pipeline. Observable retail trade imbalances (Mroib from BJZZ) are **not primarily driven by retail information** but by **wholesaler choices about which orders to internalize**:

- Wholesalers (Citadel Securities, Virtu, etc.) internalize ~80% of marketable retail orders (SEC 2022)
- When institutional liquidity demand is high, wholesalers internalize more retail flow to offset their inventory
- Result: Mroib is **inversely related** to institutional order flow

**Key empirical findings**:
- Institutional trade imbalances are inversely related to retail imbalances (Figure 1)
- Institutional price impacts are highest when |Mroib| is largest
- Large |Mroib| is associated with: abnormally low trading volumes, larger opposing short interest changes, abnormally low ATS volume
- Contemporaneous intraday returns move in the **same** direction as institutional trading, **opposite** direction of Mroib
- |Mroib| as a liquidity cost proxy yields annualized liquidity premia of **2.7-3.2%** (post-2010), outperforming all existing liquidity measures

**Causal evidence (Tick Size Pilot)**: The SEC Tick Size Pilot (2016-2018) provides exogenous variation confirming the wholesaler-choice mechanism: increasing tick size from 1 cent to 5 cents increased wholesaler internalization (more potential profit margin per trade), while simultaneously increasing the minimum price improvement from $0.0001 to $0.005 reduced internalization (50-fold cost increase). This causal evidence confirms that Mroib reflects wholesaler inventory management choices, not retail information — wholesalers adjust internalization based on their profitability, not based on retail order informativeness.

**Pipeline implication**: Heavy retail selling imbalance (negative Mroib) likely signals institutional **buying** pressure. The signal `inv_inst_direction = -mroib` should theoretically predict positive returns. Our E9 validation shows inv_inst_direction IC = -0.021 at H=10 (same magnitude as mroib with flipped sign, as expected), confirming the inverse relationship but at marginal intraday signal strength.

### 4.3 Signing Error Correction

**Source**: Barber et al. (2024). See Section 2.6.

The degradation of BJZZ's sub-penny signing is structural: as the fraction of stocks with 1-penny spreads dropped from ~80% (2010) to ~40% (2022), the sub-penny digit became less reliable for inferring direction. The midpoint method (94.8% accuracy) resolves this by being spread-invariant.

### 4.4 Institutional Contamination

**Source**: Battalio, R., R. Jennings, M. Saglam, and J. Wu (2022). "Identifying Market Maker Trades as 'Retail' from TAQ." University of Notre Dame working paper.

**24.45% of known institutional trades** executed by wholesalers also print at sub-penny prices, contaminating BJZZ retail identification. This means the BJZZ "retail" population includes a non-trivial fraction of institutional or market-maker trades.

**Mitigation**: Use spread and size filters. BJZZ is most reliable when: (a) NBBO spread = 1 cent, and (b) trade size is typical of retail (<1000 shares for NVDA). Larger trades at sub-penny prices are more likely institutional.

### 4.5 Attention-Driven Retail Behavior

**Sources**:
- Barber, B.M. and T. Odean (2008). "All That Glitters." *Review of Financial Studies*, 21(2), 785-818.
- Barber, B.M., S. Lin, and T. Odean (2024). "Resolving a Paradox: Retail Trades Positively Predict Returns but Are Not Profitable." *JFQA*, 59, 2547-2581.

Individual investors are **net buyers of attention-grabbing stocks** (structural buying-selling asymmetry). The buying-selling asymmetry arises because retail investors face thousands of potential stocks to buy but typically sell only what they own. On high-attention days (earnings announcements, product launches, AI news for NVDA), expect elevated retail buying pressure visible in TRF sub-penny prints.

Retail order imbalance positively predicts returns at short horizons, but retailers are net unprofitable because the bid-ask spread cost exceeds the signal. For our pipeline: the retail flow direction is a valid short-term predictor — the spread cost is our alpha source, not theirs.

### 4.6 Retail 0DTE Concentration

**Source**: Beckmeyer, H., N. Branger, and L. Gayda (2023). "Retail Traders Love 0DTE Options... But Should They?" Working paper, SSRN: 4404704.

**75% of retail S&P 500 option trades** now involve 0DTE contracts. Retail shifted from multi-leg to **predominantly single-leg strategies** since mid-2022. Despite lower effective spreads, retail investors experience substantial losses. This concentration of retail flow in 0DTE creates predictable hedging patterns for market makers, generating delta demand that propagates from options to equity.

---

## 5. Core Signal Mathematics

### 5.1 Order Flow Imbalance (OFI)

**Source**: Cont, R., A. Kukanov, and S. Stoikov (2014). "The Price Impact of Order Book Events." *Journal of Financial Econometrics*, 12(1), 47-88.

**Per-event contribution** (Section 2.3):
```
e_n = I_{P_n^B > P_{n-1}^B} × q_n^B           (bid improves → bullish)
    - I_{P_n^B < P_{n-1}^B} × q_{n-1}^B       (bid worsens → bearish)
    - I_{P_n^A < P_{n-1}^A} × q_n^A           (ask improves → bearish)
    + I_{P_n^A > P_{n-1}^A} × q_{n-1}^A       (ask worsens → bullish)
```
where I_{condition} is the indicator function, P^B/P^A are best bid/ask prices, q^B/q^A are best bid/ask sizes.

**Aggregated OFI** over time interval [t_{k-1}, t_k]:
```
OFI_k = Σ_{n=N(t_{k-1})+1}^{N(t_k)} e_n
```

**Linear price impact model** (Eq. 2):
```
ΔP_k = β × OFI_k + ε_k
```

**Empirical results** (50 S&P 500 stocks, April 2010, Δt = 10 seconds):
- Average **R² = 65%** across stocks (Table 2)
- β significant in **97%** of half-hour subsamples (z-test with White's standard errors, 5% level)
- Intercept α insignificant in **94%** of samples
- Adding quadratic term: R² = 68% (marginal improvement, quadratic coefficient mostly insignificant)

**Price impact coefficient vs average depth** (Eq. 3, 5):
```
β_i ≈ c / AD_i^λ        where λ ≈ 1
log(β̂_i) = α_L,i^2 - λ̂ × log(AD_i) + ε_{L,i}
```
Cannot reject λ = 1 for 35/50 stocks, confirming β is approximately inversely proportional to depth.

**Data source**: MBO data (XNAS.ITCH, ARCX.PILLAR). NOT computable from XNAS.BASIC (requires order book state changes at best bid/ask).

**Our pipeline**: Feature index 84 (`true_ofi`) and index 85 (`depth_norm_ofi`). E9 baseline: MBO true_ofi IC = -0.009 for point returns at 60s bins (consistent with the finding that OFI is contemporaneous, not predictive).

### 5.2 Multi-Level OFI (MLOFI)

**Source**: Xu, K., M.D. Gould, and S.D. Howison (2019). "Multi-Level Order-Flow Imbalance in a Limit Order Book." *Market Microstructure and Liquidity*, 4(03n04).

Extends OFI from best quotes to a vector across M price levels.

**Per-level bid flow at level m** (Eq. 9):
```
ΔW^m(τ_n) = r^m(τ_n)                        if b^m(τ_n) > b^m(τ_{n-1})    (price improved)
           = r^m(τ_n) - r^m(τ_{n-1})        if b^m(τ_n) = b^m(τ_{n-1})    (same price)
           = -r^m(τ_{n-1})                   if b^m(τ_n) < b^m(τ_{n-1})    (price worsened)
```

**Per-level ask flow** (Eq. 10):
```
ΔV^m(τ_n) = -q^m(τ_{n-1})                   if a^m(τ_n) > a^m(τ_{n-1})
           = q^m(τ_n) - q^m(τ_{n-1})        if a^m(τ_n) = a^m(τ_{n-1})
           = q^m(τ_n)                        if a^m(τ_n) < a^m(τ_{n-1})
```

**Per-level OFI** (Eq. 12):
```
e^m(τ_n) = ΔW^m(τ_n) - ΔV^m(τ_n)
MLOFI^m(t_{k-1}, t_k) = Σ_{n | t_{k-1} < τ_n ≤ t_k} e^m(τ_n)         (Eq. 11)
```

**Multivariate regression** (Eq. 16):
```
ΔP = α + Σ_{m=1}^{M} β^m × MLOFI^m + ε
```

**Empirical results** (6 Nasdaq stocks, full year 2016, LOBSTER data, 10-second intervals):
- β^1 is always the largest coefficient; coefficients decline with depth but remain significant through level 10
- Ridge MLOFI out-of-sample RMSE improvement over single-level OFI:
  - Small-tick stocks: AMZN 17%, TSLA 15%, NFLX 31%
  - Large-tick stocks: ORCL 68%, CSCO 74%, MU 64%
- Strong multicollinearity in MLOFI vector justifies Ridge regularization over OLS

**Data source**: MBO data only. NOT computable from XNAS.BASIC. Our pipeline: feature indices 116-127 (`total_mlofi`, `weighted_mlofi`, `ofi_level_1..10`).

### 5.3 Queue Imbalance

**Source**: Gould, M.D. and J. Bonart (2015). "Queue Imbalance as a One-Tick-Ahead Price Predictor in a Limit Order Book." Working paper.

**Queue imbalance** (Eq. 7):
```
I(t) = (n^b(b_t, t) - n^a(a_t, t)) / (n^b(b_t, t) + n^a(a_t, t))
```
where n^b = bid queue size (orders), n^a = ask queue size (orders). Range: I ∈ [-1, 1].

**Logistic regression** for next mid-price move direction (Eq. 17):
```
ŷ(I) = 1 / (1 + exp(-(x_0 + I × x_1)))
```

**Empirical performance** (10 Nasdaq stocks, full year 2014, LOBSTER data):

| Stock Type | AUC (out-of-sample) | MSR vs null |
|------------|--------------------| -----------|
| Large-tick (MSFT, INTC, CSCO) | 0.76-0.80 | 20-30% improvement |
| Small-tick (GOOG, AMZN, TSLA) | 0.58-0.64 | 2-6% improvement |

The slope x_1 is statistically significant at 99% for all 10 stocks. Intercept x_0 is insignificant for 7/10 stocks (symmetric buy/sell).

**Partial computability from XNAS.BASIC**: The Python Databento SDK exposes `bid_ct_00` and `ask_ct_00` (order counts at best bid/ask), but these fields are NOT available in the Rust `dbn` crate's `ConsolidatedBidAskPair` struct (which has `bid_pb`/`ask_pb` publisher IDs instead). Even if available, these would reflect Nasdaq-only orders, not the full visible order book. No feature formulas in this pipeline depend on order counts.

### 5.4 VPIN (Volume-Synchronized Probability of Informed Trading)

**Source**: Easley, D., M. López de Prado, and M. O'Hara (2012). "Flow Toxicity and Liquidity in a High-Frequency World." *Review of Financial Studies*, 25(5), 1457-1493.

**Bulk Volume Classification** (Eq. 7):
```
V_τ^B = Σ_{i=t(τ-1)+1}^{t(τ)} V_i × Φ((P_i - P_{i-1}) / σ_{ΔP})
V_τ^S = V - V_τ^B
```

**VPIN** (Eq. 10):
```
VPIN_L = (1/n) × Σ_{τ=L-n+1}^{L} |V_τ^S - V_τ^B| / V
```

**Standard parameters** (Section 3):
- V = 1/50 of average daily volume (bucket size)
- n = 50 buckets for VPIN calculation (~1-day lookback window)
- 1-minute time bars for BVC σ_{ΔP} computation (but see sampling recommendation below)

**Empirical properties** (E-mini S&P 500 futures):
- AR(1) = **0.9958** (extremely persistent)
- Correlation between ln(VPIN_{τ-1}) and |P_τ/P_{τ-1} - 1|: **0.400** (44,537 observations)
- 95% CI: [0.392, 0.408]
- VPIN peaked at highest level on May 6, 2010 (Flash Crash), reaching CDF > 0.9 at least 2 hours before the crash

**Sampling recommendation** (Easley et al. 2021): Dollar-volume bars outperform time bars for VPIN computation. A bar forms when cumulative dollar volume Σ(p_j × V_j) ≥ L. This synchronizes sampling with market activity rather than clock time, producing more stable estimates. For our TRF-specific VPIN, use TRF dollar-volume bars.

**Critique and TR-VPIN vs FB-VPIN distinction** (Andersen and Bondarenko 2014, "VPIN and the Flash Crash," *Journal of Financial Markets*, 17, 1-46):
- Standard "time-rule" VPIN (TR-VPIN) uses time bars for BVC classification, creating a mechanical positive correlation with trading intensity (r = 0.50-0.71 depending on time bar size; delta=10s: 0.50, delta=60s: 0.62, delta=300s: 0.71)
- VPIN correlated with uninformed volume benchmarks: r = 0.67-0.84
- **Fixed-bin VPIN (FB-VPIN)**, which uses actual trade indicators instead of time-bar classification, produces NEGATIVE correlation with volume and did NOT show exceptional values on Flash Crash day
- This suggests TR-VPIN's apparent predictive power is partially an artifact of time-bar aggregation inflating order imbalance during high trading intensity
- **Implementation recommendation**: Use volume-bar BVC (not time-bar) to compute VPIN, or residualize VPIN on volume, to avoid the mechanical volume correlation

**Application to XNAS.BASIC**: VPIN is fully computable from trade prices and volumes — no order book required. Compute **venue-specific VPINs** separately for TRF prints and XNAS lit trades. Use volume-bar BVC and residualize on volume to address the Andersen-Bondarenko critique.

**Feature importance methodology** (Easley, López de Prado, O'Hara, and Zhang 2021, "Microstructure in the Machine Age," *Review of Financial Studies*, 34(7), 3316-3363): When evaluating feature importance, distinguish in-sample MDI (Mean Decreased Impurity: `MDI(i) = (1/100) × Σ_N Σ_{n: v(s_n)=i} p(t) × IG(s_n, n)` for 100-tree forests) from out-of-sample MDA (Mean Decreased Accuracy: `MDA(i) = (p_0 - p_i) / p_0` where p_i is accuracy after permuting feature i). Features with high in-sample R² (e.g., Amihud, VIX) can have LOW out-of-sample predictive power. VPIN dominates via MDA for bid-ask spread and kurtosis prediction across 87 liquid futures, though the Roll measure dominates for sequential correlation prediction. Use MDA (with purged cross-validation and ~1-week embargo) for our off-exchange feature ranking.

### 5.5 Kyle's Lambda

**Source**: Kyle, A.S. (1985). "Continuous Auctions and Insider Trading." *Econometrica*, 53(6), 1315-1336.

```
λ = σ_v / σ_u
ΔP = λ × (signed volume) + ε
```

Kyle's lambda measures price impact per unit of net order flow. Higher λ = more informed trading relative to noise.

**Estimation**: Rolling OLS regression of price changes on signed volume over 5-15 minute windows. Requires signed trades (use midpoint signing from Section 2.6).

**Extended estimation**: Back, K., K. Crotty, and T. Li (2018, *Review of Financial Studies*, 31(6), 2277-2325) show that estimating λ from order flow alone is insufficient — both returns and order flows are needed to identify information asymmetry. Their "expected average lambda" outperforms PIN, VPIN, and standard Kyle's λ.

**Computability from XNAS.BASIC**: Yes, if trades can be signed. Use midpoint-signed TRF volume as the signed flow variable.

### 5.6 PIN (Probability of Informed Trading)

**Source**: Easley, D., N.M. Kiefer, M. O'Hara, and J.B. Paperman (1996). "Liquidity, Information, and Infrequently Traded Stocks." *The Journal of Finance*, 51(4), 1405-1436.

```
PIN = (α × μ) / (α × μ + 2ε)                   (Eq. in Section I)
```
where α = probability of information event, μ = informed order arrival rate, ε = uninformed order arrival rate.

**Intraday pattern**: U-shaped (high at open/close, low midday).

**Dynamic extension**: Easley, D., R.F. Engle, M. O'Hara, and L. Wu (2008, *Journal of Financial Econometrics*, 6(2), 171-207) extend to time-varying arrival rates, more suitable for intraday estimation.

### 5.7 Informed Trading Intensity (ITI)

**Source**: Bogousslavsky, V., V. Fos, and D. Muravyev (2024). "Informed Trading Intensity." *Journal of Finance*, 79(2), 903-948.

**Construction**:
- Training data: Schedule 13D filings (60-day disclosure window, 1,593 filings, 1994-2018)
- Method: XGBoost (Gradient Boosted Trees) with 41 input features
- Features: 4 from CRSP (price, return, |return|, volume) + 37 from TAQ intraday
- All features standardized: subtract mean, divide by std over prior year
- 5-fold cross-validation in calendar time
- ITI bounded between 0 and 1

**Key empirical results**:
- R² for 13D detection: **9.86%** (vs 4.61% for standard liquidity variables)
- ROC AUC: 71% (vs 50% random)
- Volume-related variables most important; volume × volatility interaction critical
- ITI increasing and concave in volume; decreasing and convex in volatility
- FF4 alpha for top-minus-bottom ITI decile: **52 bps/month** (6.4%/year), t = 6.2

**Return reversal test** (Table VII):
```
r_{i,t+1} = a_t + b1 × r_{i,t} + b2 × ITI_{i,t} + b3 × ITI_{i,t} × r_{i,t} + controls + ε
```
b3 > 0 (t = 6.445): less reversal on high-ITI days → permanent price impact = informed trading

**Patient vs impatient decomposition**: Bogousslavsky et al. further decompose ITI into ITI(patient) (trained on first 40 days of 60-day filing window, capturing limit order accumulation) and ITI(impatient) (trained on last 20 days, capturing aggressive market order execution). Key findings:
- Unconditional correlation between patient and impatient: **0.47** (distinct constructs)
- ITI(impatient) is positively associated with realized volatility; ITI overall is elevated 2 days before earnings (Table VI confirms ITI(impatient) has larger pre-earnings coefficients than ITI(patient))
- ITI(patient) is **negatively** associated with volatility; its pre-earnings increase is economically negligible compared to ITI(impatient)
- Patient informed trading (via limit orders) is harder to detect and has a different market footprint

**Applicability**: Not directly implementable in real-time from XNAS.BASIC alone (requires cross-sectional training on labeled informed trades). However, intraday proxies using volume, volatility, and their interaction can approximate ITI dynamics. The patient/impatient distinction matters: high volume + low volatility signals patient informed trading (limit order accumulation), while high volume + high volatility signals impatient informed trading (aggressive execution).

### 5.8 Volume Order Imbalance (VOI)

**Source**: Cartea, A., R. Donnelly, and S. Jaimungal (2018). "Enhancing Trading Strategies with Order Book Signals." *Applied Mathematical Finance*, 25(1), 1-35.

```
ρ_t = (V_t^b - V_t^a) / (V_t^b + V_t^a) ∈ [-1, 1]           (Eq. 1)
```
where V_t^b = volume at best bid, V_t^a = volume at best ask.

Predicts the direction of the next market order with high accuracy. When ρ > 1/3 (buy-heavy), next MO is overwhelmingly a buy; when ρ < -1/3 (sell-heavy), next MO is overwhelmingly a sell.

**Data source**: MBO data (best bid/ask volumes). Partially computable from XNAS.BASIC using `bid_sz_00`/`ask_sz_00`.

### 5.9 Option Volume Imbalance (OVI)

**Source**: Michael, N., M. Cucuringu, and S.D. Howison (2022). "Option Volume Imbalance as a Predictor for Equity Market Returns." *arXiv:2201.09319*.

```
OVI_{i,d,m}^[F] = Σ_j(X^{Up,F}_{i,j,d,m} - X^{Down,F}_{i,j,d,m}) / Σ_j(X^{Up,F}_{i,j,d,m} + X^{Down,F}_{i,j,d,m})     (Eq. 1)
```
where X = V (volume), T (trades), or P×V (nominal volume). OVI ∈ [-1, 1].

**Directional signals**:
- Positive (bullish): Call Buys + Put Sells
- Negative (bearish): Call Sells + Put Buys

**Market participant classes**: OVI is decomposed across 5 classes from NASDAQ Option Market data (10-minute intraday buckets, 39 per day):
1. **Firm** (proprietary trades)
2. **Brokers**
3. **Market Makers**
4. **Customers** (ordinary retail/institutional)
5. **Professional Customers**

**Key findings**:
- **Market-maker OVI** provides strongest signal (negative predictor — reflects counterparties' informed trading that MMs absorb)
- **Customer OVI** has a positive but weaker signal (opposite sign from MM OVI)
- Strongest predictability for **overnight returns** (close-to-open)
- **Put option volumes** more informative than calls
- Deep OTM options: highest predictive power
- Non-linear relationship: sign of OVI used as directional predictor

**Data source**: OPRA data. NOT available in XNAS.BASIC. Future pipeline extension.

---

## 6. Off-Exchange Feature Definitions

All features below are computable from the XNAS.BASIC CMBP-1 schema. Empirical validation from E9 and E9 cross-validation (35 test days, 8,337 samples, 60-second bins).

### 6.1 TRF Signed Imbalance

**Our primary signal.** Captures directional flow from ALL off-exchange trades (retail + institutional).

**Formula**:
```
mid = (bid_px_00 + ask_px_00) / 2
spread = ask_px_00 - bid_px_00
valid_bbo = isfinite(mid) AND (spread > 0)

For each TRF trade (publisher_id ∈ {82, 83}, action = 'T'):
  buy_flag = valid_bbo AND (price > mid + 0.1 × spread)
  sell_flag = valid_bbo AND (price < mid - 0.1 × spread)
  unsigned = NOT buy_flag AND NOT sell_flag        (within 20% of midpoint)

Per time bin:
  buy_vol = Σ size where buy_flag
  sell_vol = Σ size where sell_flag
  trf_signed_imbalance = (buy_vol - sell_vol) / (buy_vol + sell_vol) ∈ [-1, 1]
```

**E9 empirical results**:

| Horizon | IC | p-value | Significance |
|---------|-----|---------|-------------|
| H=1 (1 min) | **+0.103** | 4.05e-21 | Strongest signal in 15 experiments |
| H=2 (2 min) | +0.083 | 2.9e-14 | |
| H=3 (3 min) | +0.070 | 1.5e-10 | |
| H=5 (5 min) | +0.056 | 3.5e-07 | |
| H=10 (10 min) | +0.040 | 3.0e-04 | |
| H=20 (20 min) | +0.056 | 3.4e-07 | |
| H=30 (30 min) | +0.053 | 1.5e-06 | |
| H=60 (60 min) | +0.050 | 5.2e-06 | |

- Bootstrap 95% CI at H=10: [+0.019, +0.060] (excludes zero)
- Per-day stability: mean IC = 0.034, std = 0.082, positive 65.7% of days
- Lagged IC: drops sharply at lag=1 (0.040 → 0.014), partially recovers at lag=10 (0.034) — partially contemporaneous
- Contemporaneous IC (vs current-period return): +0.033 (NOT classified as contemporaneous since 0.033 < 2 × 0.040)

**Theoretical basis**: Combines Cont et al.'s OFI concept (directional flow imbalance predicts short-term returns) with TRF venue decomposition. Unlike MBO OFI which captures limit order book dynamics, TRF signed imbalance captures the net direction of ALL off-exchange trade executions — including both retail (wholesaler internalization) and institutional (dark pool, block) flow. The signal is strongest at H=1 because TRF-reported flow reflects trades that have already occurred, providing a 1-minute lead over the full market price adjustment.

### 6.2 Subpenny Intensity

**Slow-moving state variable.** Measures the fraction of off-exchange activity that is wholesaler/retail internalization.

**Formula**:
```
frac_cent = (price × 100) mod 1
is_subpenny = (frac_cent > 0.001) AND (frac_cent < 0.999)

Per time bin (TRF trades only):
  n_subpenny = count(trades where is_subpenny)
  n_trf = count(all TRF trades in bin)
  subpenny_intensity = n_subpenny / n_trf                    (COUNT ratio, not volume-weighted)
```

**E9 empirical results**:

| Horizon | IC | p-value | Pattern |
|---------|-----|---------|---------|
| H=1 (1 min) | +0.023 | 0.039 | Marginal at short horizons |
| H=10 (10 min) | +0.048 | 1.2e-05 | |
| H=20 (20 min) | +0.065 | 3.4e-09 | |
| H=60 (60 min) | **+0.104** | 1.3e-21 | INCREASES with horizon |

- Bootstrap 95% CI at H=10: [+0.027, +0.069] (excludes zero)
- Remarkably stable across lags: IC = 0.048 (lag 0) → 0.041 (lag 1) → 0.044 (lag 10) — NOT contemporaneous, a genuine slow-moving predictor
- ACF(1) = 0.889 (extremely persistent — state variable, not event-driven)

**Theoretical basis**: Exploits the Reg NMS Rule 612 artifact (Boehmer et al. 2021). High subpenny_intensity indicates more wholesaler internalization activity, which Barardehi et al. (2021, 2025) show is driven by institutional liquidity demand. The signal accumulates over longer horizons because it reflects a persistent state (wholesale activity level) rather than a transient event.

### 6.3 Dark Share

**Regime indicator.** Captures the fraction of visible volume executed off-exchange.

**Formula**:
```
Per time bin:
  trf_volume = Σ size for publisher_id ∈ {82, 83} AND action = 'T'
  lit_volume = Σ size for publisher_id = 81 AND action = 'T'
  dark_share = trf_volume / (trf_volume + lit_volume)
```

**CRITICAL CAVEAT**: This is TRF/(TRF + XNAS_lit), NOT TRF/consolidated. Our denominator excludes 15-19% of total market volume from other lit exchanges (ARCX, BATS, IEX, etc.). True market dark share for NVDA is ~50% (validated against EQUS_SUMMARY), but our per-bin average is ~76% due to both denominator limitation and session composition effects.

**E9 empirical results**:

| Horizon | IC | p-value | Interpretation |
|---------|-----|---------|---------------|
| H=1 (1 min) | +0.051 | 2.6e-06 | Short-term regime signal |
| H=10 (10 min) | +0.035 | 0.0015 | Decays |
| H=60 (60 min) | -0.013 | 0.228 | SIGN FLIP (mean-reverts) |

- Bootstrap 95% CI at H=10: [+0.013, +0.056] (excludes zero)
- Per-day stability: mean IC = 0.032, positive 62.9% of days
- ACF(1) = 0.418 (moderately persistent)

**Theoretical basis**: Zhu (2014) predicts a non-linear relationship between dark share and subsequent returns. When dark share is high, lit exchange order flow is more informationally dense (uninformed flow has migrated to dark pools). The sign flip at H=60 is consistent with mean-reversion: abnormally high dark share temporarily boosts lit OFI signal quality, but the market adjusts over longer horizons.

### 6.4 Mroib (Market Retail Order Imbalance)

**Retail-only directional signal.** Uses BJZZ to identify retail trades, then midpoint signing for direction.

**Formula**:
```
BJZZ identification:
  frac_cent = (price × 100) mod 1
  is_retail = (frac_cent > 0.001 AND frac_cent < 0.40) OR (frac_cent > 0.60 AND frac_cent < 0.999)

Midpoint signing (Barber 2024):
  retail_buy = is_retail AND valid_bbo AND (price > mid + 0.1 × spread)
  retail_sell = is_retail AND valid_bbo AND (price < mid - 0.1 × spread)

Per time bin:
  mroib = (retail_buy_vol - retail_sell_vol) / (retail_buy_vol + retail_sell_vol) ∈ [-1, 1]
```

**E9 results**: IC = +0.021 at H=10 (marginal, does NOT pass IC > 0.05 gate). ACF(1) = 0.050 (near-zero persistence). The cross-sectional weekly Mroib predictability does NOT transfer to intraday single-stock.

### 6.5 Abs(Mroib) — Institutional Urgency Proxy

```
abs_mroib = |mroib|
```

E9 IC = -0.005 at H=10 (no signal). The Barardehi et al. (2025) finding that |Mroib| proxies institutional costs at weekly frequency does not transfer to intraday.

### 6.6 Inverse Institutional Direction

```
inv_inst_direction = -mroib
```

E9 IC = -0.021 at H=10 (mechanically = -IC(mroib), since inv_inst_direction = -mroib). The negative IC means the institutional inverse theory does NOT produce a usable positive-IC signal at intraday timescale — the weekly cross-sectional effect does not transfer to single-stock minute-level prediction.

### 6.7 Odd Lot Ratio

**Formula**:
```
Per time bin (TRF trades only):
  n_odd_lot = count(trades where size < 100)
  n_trf = count(all TRF trades)
  odd_lot_ratio = n_odd_lot / n_trf                         (COUNT ratio)
```

**Source**: O'Hara, M., C. Yao, and M. Ye (2014). "What's Not There: Odd Lots and Market Data." *The Journal of Finance*, 69(5), 2199-2236. Odd-lot trades (<100 shares) are highly informative and disproportionately correlated with HFT activity.

**E9 results**: IC = +0.018 at H=10 (marginal). ACF(1) = 0.886 (extremely persistent — state variable).

### 6.8 Retail Trade Rate

```
Per 60s bin:
  retail_trade_rate = count(BJZZ-identified retail trades)
```

**E9 results**: IC = +0.027 at H=10 (marginal). ACF(1) = 0.976 (extremely persistent). Measures activity level, not direction.

### 6.9 Volume Features

```
trf_volume = Σ size for publisher_id ∈ {82, 83} AND action = 'T'
lit_volume = Σ size for publisher_id = 81 AND action = 'T'
total_volume = trf_volume + lit_volume
```

All three have IC < 0.03 at H=10. Volume features are state variables (useful for conditioning), not directional predictors.

---

## 7. Cross-Venue Interaction Theory

### 7.1 Information Shares

**Source**: Hasbrouck, J. (1995). "One Security, Many Markets: Determining the Contributions to Price Discovery." *Journal of Finance*, 50(4), 1175-1199.

Uses a Vector Error Correction Model (VECM) on cointegrated venue prices to attribute price discovery across venues. Key finding: quotes contribute 60-70% of price discovery; lit exchanges dominate despite fragmented volume.

**Application**: Hasbrouck (2021, "Price Discovery in High Resolution," NYU working paper) specifically studies NVDA at sub-millisecond resolution. For our pipeline: fit a simplified VECM to NVDA mid-prices from Nasdaq and implied TRF prices to estimate time-varying information shares. When a venue's IS is temporarily elevated, weight its flow signals more heavily.

### 7.2 Dark-Lit Informational Linkages

**Source**: Nimalendran, M. and S. Ray (2014). "Informational Linkages Between Dark and Lit Trading Venues." *Journal of Financial Markets*, 17, 230-261.

Finds signed trades (especially for less liquid stocks) can predict returns at horizons of **15 to 120 minutes**, with some activity types transmitting more information than others.

**Pipeline implication**: Build cross-excitation features:
- Does a TRF burst (cluster of TRF trades) precede a lit exchange sweep or spread change?
- Time since last TRF print burst
- Correlation of TRF signed imbalance with subsequent lit midquote moves

### 7.3 Cross-Asset OFI

**Source**: Cont, R., M. Cucuringu, and C. Zhang (2023). "Cross-Impact of Order Flow Imbalance in Equity Markets." *Quantitative Finance*, 23(10), 1373-1393.

Applies PCA to MLOFI vectors to create integrated OFI, then examines cross-asset impact using LASSO:
- **Contemporaneous** cross-asset OFI adds nothing once own-stock multi-level OFI is integrated
- **Lagged** cross-asset OFIs significantly improve forecasting at short horizons (decaying rapidly)
- **NVDA is explicitly identified as a "hub" stock** with high cross-predictive power

**Pipeline implication**: Lagged OFI from AMD, AVGO, SMH (semiconductor ETF), and QQQ (Nasdaq-100 ETF) can predict NVDA returns at sub-30-minute horizons. Implementing this requires cross-asset data feeds.

### 7.4 Venue Fragmentation Effects

**Sources**:
- O'Hara, M. and M. Ye (2011). "Is Market Fragmentation Harming Market Quality?" *Journal of Financial Economics*, 101(3), 459-474.
- Degryse, H., F. de Jong, and V. van Kervel (2015). "The Impact of Dark Trading and Visible Fragmentation on Market Quality." *Review of Finance*, 19(4), 1587-1622.

"Local" (single-venue) liquidity is NOT the same as "global" (consolidated) liquidity. Non-linear relationship between fragmentation and adverse selection.

**Pipeline implication**: Our signals built on Nasdaq-only BBO can behave differently from signals built on a consolidated view. Track both "local" liquidity state (Nasdaq-only depth, spread, queue metrics) and "global-ish" activity (Nasdaq + TRF combined volume, dark share), and let the model weight signals differently by state.

---

## 8. Empirical Validation

### 8.1 Horizon Sweep Interpretation

| Feature | H=1 IC | H=10 IC | H=60 IC | Theoretical Interpretation |
|---------|--------|---------|---------|---------------------------|
| trf_signed_imb | **+0.103** | +0.040 | +0.050 | Short-term flow impact; decays as new information arrives, recovers at H=20-60 (see below) |
| subpenny_int | +0.023 | +0.048 | **+0.104** | Slow-moving regime state; accumulates over time as wholesaler activity persists |
| dark_share | +0.051 | +0.035 | -0.013 | Short-term regime effect; mean-reverts, sign flips at long horizons |

**trf_signed_imbalance recovery at H=20-60**: After decaying from IC=0.103 (H=1) to IC=0.040 (H=10), the signal recovers to IC~0.050 at H=20-60. This U-shaped pattern is consistent with two-timescale dynamics: (1) immediate flow impact dissipates by H=5-10, then (2) institutional positioning revealed by TRF flow exerts a slower, persistent effect at longer horizons.

### 8.2 Contemporaneous vs Predictive Classification

| Feature | IC(current return) | IC(H=10 future) | Contemporaneous? |
|---------|-------------------|-----------------|-----------------|
| trf_signed_imb | +0.033 | +0.040 | No (IC_current < 2 × IC_future) |
| subpenny_int | +0.011 | +0.048 | No |
| dark_share | +0.036 | +0.035 | No |

None of our off-exchange features are classified as purely contemporaneous (defined as IC_current > 2 × IC_future). This is a **critical difference** from MBO OFI which IS purely contemporaneous (lag-1 IC < 0.006 at ALL scales).

**Why off-exchange features have predictive content while MBO OFI does not**: TRF prints aggregate information over reporting delay + venue routing delay, creating a natural smoothing that produces genuine (if weak) predictive content. The TRF-reported flow reflects trades that have already occurred but whose price impact has not yet fully propagated to the lit market.

### 8.3 Per-Day IC Stability

| Feature | Mean IC | Std IC | Mean/Std | % Days Positive | n Days |
|---------|---------|--------|----------|-----------------|--------|
| trf_signed_imb | +0.034 | 0.082 | 0.41 | 65.7% | 35 |
| subpenny_int | +0.026 | 0.155 | 0.17 | 60.0% | 35 |
| dark_share | +0.032 | 0.097 | 0.33 | 62.9% | 35 |

All features show high per-day instability (std >> mean). The trf_signed_imb has the best stability ratio (0.41) but is positive only 65.7% of days.

**Theoretical explanation**: The signal reflects market-specific conditions. Per Zhu's (2014) model, the informational content of dark-lit flow decomposition depends on the current mix of informed and uninformed traders, which varies daily. Per Menkveld's (2017) pecking order, venue choice shifts predictably with urgency/uncertainty — on calm days with little institutional activity, the off-exchange signals carry minimal directional information.

**This instability makes regime gating MORE valuable**: The regime system's job is to identify the ~66% of days where the signal works and avoid the ~34% where it doesn't. A regime detector that can predict signal-active vs signal-inactive states would concentrate returns into profitable periods.

### 8.4 Lagged IC Analysis

| Lag (bins) | trf_signed_imb | subpenny_int | dark_share |
|-----------|----------------|--------------|------------|
| 0 | +0.040 | +0.048 | +0.035 |
| 1 | +0.014 | +0.041 | +0.016 |
| 2 | +0.018 | +0.041 | +0.013 |
| 5 | +0.025 | +0.044 | +0.008 |
| 10 | +0.034 | +0.044 | +0.017 |

**trf_signed_imb**: Drops sharply at lag=1 (0.040 → 0.014), then partially recovers. ~35% of the H=10 signal is contemporaneous; ~65% is genuinely predictive but with a decay-recovery pattern.

**subpenny_int**: Remarkably stable across all lags (0.048 → 0.041 → 0.044). This is NOT a contemporaneous signal — it is a persistent state variable. The slight lag-1 drop likely reflects measurement noise, not signal decay.

**dark_share**: Decays rapidly (0.035 → 0.008 at lag=5). Mostly contemporaneous at the 10-minute horizon.

### 8.5 Coverage Validation Against EQUS_SUMMARY

| Metric | Value | Source |
|--------|-------|--------|
| XNAS.BASIC coverage | 81-85% of consolidated (5 days) | EQUS_SUMMARY |
| True TRF share | 49-53% of consolidated | EQUS_SUMMARY |
| E9 per-bin dark_share | ~76% mean | TRF/(TRF+XNAS_lit), session composition |
| Session composition | E5 window = 09:50-14:55 ET | Post-market excluded |
| Post-market dark share | 24.5% | XNAS lit dominates in post-market |
| Additional publishers | 88=XBOS, 89=XPSX (~0.2% combined) | Minor lit exchanges, not TRF |

### 8.6 Bootstrap Confidence Intervals (10,000 resamples, seed=42)

| Feature | Point IC (H=10) | 95% CI Lower | 95% CI Upper | Contains Zero? |
|---------|----------------|-------------|-------------|---------------|
| trf_signed_imb | +0.040 | +0.019 | +0.060 | No |
| subpenny_int | +0.048 | +0.027 | +0.069 | No |
| dark_share | +0.035 | +0.013 | +0.056 | No |

All three top features have CIs that exclude zero — the signals are statistically real, though weak (IC < 0.05 for the primary H=10 horizon).

---

## 9. Implementation Pitfalls

### 9.1 TRF Reporting Delay

**FINRA reporting rules**: Trades must be reported "as soon as practicable" and no later than 10 seconds after execution during TRF operating hours (FINRA Rules 6282, 6380A, 6380B). Late/as-of reporting designations exist for legitimate delays.

**Our empirical finding (E9)**: Actual reporting delay is <1ms for most NVDA TRF prints (ts_event ≈ ts_recv). The feared 10-second maximum delay is the **regulatory ceiling**, not the typical case. This is likely because NVDA is a highly liquid stock processed by automated wholesaler systems with sub-millisecond latency.

**Recommendation**: Use `ts_recv` as the simulation clock (what a strategy could plausibly observe). Preserve `ts_event` for latency studies. Run sensitivity tests by adding extra delays (10ms, 100ms, 1s) to TRF prints to quantify signal fragility to latency.

### 9.2 NBBO vs Nasdaq-Only BBO

XNAS.BASIC provides ONLY Nasdaq quotes — not the protected NBBO used for compliance. The Nasdaq BBO may differ from the true NBBO when another exchange is at-or-better:

- For NVDA with its typically 1-cent spread, the difference is usually negligible
- When the Nasdaq BBO is NOT the NBBO, our midpoint signing reference is slightly off
- The magnitude of signing errors from BBO ≠ NBBO is bounded by the cross-venue spread differential (typically <1 cent for NVDA)

**Robustness**: Focus on features less sensitive to the exact NBBO value (signed imbalance direction, TRF burstiness, subpenny intensity) rather than features that depend on exact spread measurement (price improvement, effective spread).

### 9.3 Trade Signing Error Propagation

At the midpoint method's ~5% error rate:
- Signed imbalance has a noise floor that attenuates the true signal
- The attenuation is approximately multiplicative: true IC × (1 - 2 × error_rate) ≈ true IC × 0.90
- Our observed IC = 0.103 at H=1 implies a pre-noise true IC of ~0.114

**Recommendation**: Track unsigned volume (trades within the exclusion band) as a separate feature — it represents ambiguous flow that may carry information about market maker inventory. Downweight signed flow from bins with high ambiguous-volume fraction.

### 9.4 Mixture Drift

What TRF volume represents (wholesaler internalization vs ATS dark pool vs other OTC) changes over time:
- Regulatory changes (SEC best-execution rules, potential Reg NMS II)
- Market structure evolution (new dark pools, wholesaler mergers)
- Seasonal patterns (year-end institutional rebalancing)

Papers explicitly show conclusions change when dark components are separated (Buti, Rindi, and Werner 2022, "Diving Into Dark Pools," *Financial Management*).

**Recommendation**: Build a periodic recalibration step (monthly) for latent-class thresholds (midpoint vs near-touch vs block-like prints). Include time-varying priors informed by FINRA ATS transparency data (published with a 2-week lag).

### 9.5 Retail Predictability Transfer Failure

The cross-sectional weekly Mroib predictability (10.89 bps/week; Boehmer et al. 2021) does **NOT transfer** to intraday single-stock prediction (Mroib IC = +0.021 at H=10, E9 validated). Reasons:

1. **Cross-sectional diversification**: Weekly Mroib works across thousands of stocks; single-stock noise is not diversifiable
2. **Timescale mismatch**: Retail flow dynamics at weekly frequency (persistence, contrarian rebalancing) differ from minute-frequency dynamics
3. **Sample size**: Each 60s bin has ~40-80 retail-identified trades for NVDA, vs daily cross-sections of thousands of stocks
4. **Identification noise**: BJZZ false negatives (65%) and institutional contamination (24%) add noise at high frequency

**Recommendation**: Treat mroib as a secondary signal, not primary. The aggregate TRF signed imbalance (which captures both retail and institutional flow) is more informative at high frequency.

### 9.6 dark_share Semantic Precision

Our dark_share feature = TRF/(TRF + XNAS_lit) overstates true market dark share by ~11 percentage points (61% vs 50% day-total) because the denominator excludes other lit exchanges. The per-bin average (~76%) is further inflated by session composition effects.

**Recommendation**: Use dark_share as a **relative** feature (within-day variation, deviations from rolling average) rather than interpreting its absolute level. Alternatively, compute true_dark_share = TRF_volume / EQUS_SUMMARY_volume for daily-level regime classification.

---

## 10. Signal Priority Hierarchy

Signals ranked by combined strength of empirical evidence (E9 IC) and theoretical support (number and quality of supporting papers).

| Priority | Signal | IC (best H) | Horizon | ACF(1) | Theoretical Support | Status |
|----------|--------|-------------|---------|--------|---------------------|--------|
| **1** | trf_signed_imbalance | +0.103 | H=1 | 0.093 | Cont OFI + venue decomposition + Barardehi institutional inverse | **E9 validated** |
| **2** | subpenny_intensity | +0.104 | H=60 | 0.889 | BJZZ Rule 612 + Barardehi wholesale activity | **E9 validated** |
| **3** | dark_share | +0.051 | H=1 | 0.418 | Zhu dark pool equilibrium + Comerton-Forde inverted-U | **E9 validated** |
| 4 | VPIN (TRF-specific) | Not tested | -- | -- | Easley et al., highest OOS predictor (MDA) | Untested |
| 5 | Microprice-based signing | Not tested | -- | -- | Stoikov 2018, 18% bias reduction | Untested |
| 6 | BVC signed flow | Not tested | -- | -- | Easley et al. 2012, informed trading proxy | Untested |
| 7 | Odd lot ratio | +0.018 | H=10 | 0.886 | O'Hara et al. 2014, HFT proxy | Marginal |
| 8 | Retail trade rate | +0.027 | H=10 | 0.976 | Activity level, not directional | Marginal |
| 9 | Mroib | +0.021 | H=10 | 0.050 | Boehmer et al. 2021 (cross-sectional weekly; fails intraday) | Marginal |

**Key insight**: The top 3 validated signals have different optimal horizons and dynamics:
- **trf_signed_imbalance** = fast, directional, partially contemporaneous → target H=1-3
- **subpenny_intensity** = slow state variable, accumulates → target H=20-60
- **dark_share** = regime indicator, mean-reverting → use for conditioning/gating

This suggests a multi-horizon strategy: use trf_signed_imbalance for short-term direction, subpenny_intensity for medium-term positioning, and dark_share for regime identification.

### Regime Architecture Framework

The individual theoretical results from Sections 3-5 integrate into a multi-layer regime system that determines when each signal source is most informative (synthesized from Zhu 2014, Kyle 1985, Menkveld 2017, Easley et al. 2012, Hamilton 1989):

| Layer | Signal Sources | States | Theoretical Basis |
|-------|---------------|--------|-------------------|
| **Volatility** | Realized vol, return distribution | Calm / Elevated / Crisis | Hamilton (1989) regime-switching HMM |
| **Information** | VPIN, Kyle's lambda | Low toxicity / Moderate / Toxic | Easley et al. (2012), Kyle (1985) |
| **Fragmentation** | dark_share, venue composition | Normal / Flight-to-lit / Dark accumulation | Zhu (2014), Menkveld et al. (2017) |
| **Quality** | Spread, depth, odd-lot ratio | Liquid / Thin / Stressed | Comerton-Forde & Putniņš (2015), O'Hara et al. (2014) |

**How layers interact with off-exchange signals**:
- **Flight-to-lit regime** (dark_share dropping during volatility spike): Lit OFI signals are amplified because both informed and uninformed traders return to exchanges seeking execution certainty (Zhu 2014). Weight MBO OFI more heavily; TRF flow becomes less informative.
- **Dark accumulation regime** (dark_share rising with low volatility): Institutional accumulation via dark pools. TRF signed imbalance becomes more informative; subpenny_intensity indicates wholesale activity level.
- **Toxic regime** (VPIN elevated): Signed flow signals are more informative but adverse selection risk is higher. Tighter position sizing, wider stop-losses.
- **Stressed quality regime** (spread widened, depth thin): All signals become noisier. Reduce overall signal weight, increase cost estimates.

The regime system is **Phase 2 work** — it should be built after the off-exchange feature pipeline is operational and validated. The off-exchange features provide the directional signal (Phase 1); the regime system identifies WHEN that signal is strongest (Phase 2).

---

## References

### Primary Papers (directly cited with equations)

1. Avellaneda, M., J. Reed, and S. Stoikov (2011). "Forecasting Prices from Level-I Quotes in the Presence of Hidden Liquidity." *J. Computational Finance*, 14(3), 35-61.
2. Barber, B.M., X. Huang, P. Jorion, T. Odean, and C. Schwarz (2024). "A (Sub)penny for Your Thoughts." *J. Finance*, 79(4), 2403-2427.
3. Barardehi, Y.H., D. Bernhardt, Z. Da, and M. Warachka (2021/2025). "Institutional Liquidity Costs, Internalized Retail Trade Imbalances." *JFQA* (forthcoming).
4. Boehmer, E., C.M. Jones, X. Zhang, and X. Zhang (2021). "Tracking Retail Investor Activity." *J. Finance*, 76(5), 2249-2305.
5. Bogousslavsky, V., V. Fos, and D. Muravyev (2024). "Informed Trading Intensity." *J. Finance*, 79(2), 903-948.
6. Cartea, A., R. Donnelly, and S. Jaimungal (2018). "Enhancing Trading Strategies with Order Book Signals." *Applied Mathematical Finance*, 25(1), 1-35.
7. Comerton-Forde, C. and T.J. Putniņš (2015). "Dark Trading and Price Discovery." *J. Financial Economics*, 118(1), 70-92.
8. Cont, R., A. Kukanov, and S. Stoikov (2014). "The Price Impact of Order Book Events." *J. Financial Econometrics*, 12(1), 47-88.
9. Cont, R., M. Cucuringu, and C. Zhang (2023). "Cross-Impact of Order Flow Imbalance." *Quantitative Finance*, 23(10), 1373-1393.
10. Easley, D., M. López de Prado, and M. O'Hara (2012). "Flow Toxicity and Liquidity." *Rev. Financial Studies*, 25(5), 1457-1493.
11. Easley, D., M. López de Prado, M. O'Hara, and Z. Zhang (2021). "Microstructure in the Machine Age." *Rev. Financial Studies*, 34(7), 3316-3363.
12. Ellis, K., R. Michaely, and M. O'Hara (2000). "Accuracy of Trade Classification Rules." *JFQA*, 35(4), 529-551.
13. Foley, S. and T.J. Putniņš (2016). "Should We Be Afraid of the Dark?" *J. Financial Economics*, 122(3), 456-481.
14. Gould, M.D. and J. Bonart (2015). "Queue Imbalance as a One-Tick-Ahead Price Predictor." Working paper.
15. Hasbrouck, J. (1995). "One Security, Many Markets." *J. Finance*, 50(4), 1175-1199.
16. Jurkatis, S. (2022). "Inferring Trade Directions in Fast Markets." *J. Financial Markets*, 58, 100635.
17. Kyle, A.S. (1985). "Continuous Auctions and Insider Trading." *Econometrica*, 53(6), 1315-1336.
18. Lee, C.M.C. and M.J. Ready (1991). "Inferring Trade Direction from Intraday Data." *J. Finance*, 46(2), 733-746.
19. Menkveld, A.J., B.Z. Yueshen, and H. Zhu (2017). "Shades of Darkness." *Rev. Financial Studies*, 30(12), 4321-4372.
20. Michael, N., M. Cucuringu, and S.D. Howison (2022). "Option Volume Imbalance." arXiv:2201.09319.
21. Stoikov, S. (2018). "The Micro-Price." *Quantitative Finance*, 18(12), 1959-1966.
22. Xu, K., M.D. Gould, and S.D. Howison (2019). "Multi-Level Order-Flow Imbalance." *Market Microstructure and Liquidity*, 4(03n04).
23. Zhu, H. (2014). "Do Dark Pools Harm Price Discovery?" *Rev. Financial Studies*, 27(3), 747-789.

### Supporting Papers (cited for context)

24. Andersen, T.G. and O. Bondarenko (2014). "VPIN and the Flash Crash." *J. Financial Markets*, 17, 1-46.
25. Back, K., K. Crotty, and T. Li (2018). "Identifying Information Asymmetry." *Rev. Financial Studies*, 31(6), 2277-2325.
26. Barber, B.M. and T. Odean (2008). "All That Glitters." *Rev. Financial Studies*, 21(2), 785-818.
27. Barber, B.M., S. Lin, and T. Odean (2024). "Resolving a Paradox." *JFQA*, 59, 2547-2581.
28. Battalio, R., R. Jennings, M. Saglam, and J. Wu (2022). "Identifying Market Maker Trades as 'Retail'." Working paper.
29. Beckmeyer, H., N. Branger, and L. Gayda (2023). "Retail Traders Love 0DTE Options." Working paper.
30. Brogaard, J. and J. Pan (2022). "Dark Pool Trading and Information Acquisition." *Rev. Financial Studies*, 35(5), 2625-2666.
31. Buti, S., B. Rindi, and I.M. Werner (2022). "Diving Into Dark Pools." *Financial Management*.
32. Chakrabarty, B., R. Moulton, and A. Shkilko (2012). "Short Sales, Long Sales, and the Lee-Ready Algorithm Revisited." *J. Financial Markets*, 15(4), 467-491.
33. Degryse, H., F. de Jong, and V. van Kervel (2015). "Dark Trading and Visible Fragmentation." *Rev. Finance*, 19(4), 1587-1622.
34. Easley, D., N.M. Kiefer, M. O'Hara, and J.B. Paperman (1996). "Liquidity, Information, and Infrequently Traded Stocks." *J. Finance*, 51(4), 1405-1436.
35. Kolm, P.N., J. Turiel, and N. Westray (2023). "Deep Order Flow Imbalance." *Mathematical Finance*, 33(4), 1044-1081.
36. Nimalendran, M. and S. Ray (2014). "Informational Linkages Between Dark and Lit Trading Venues." *J. Financial Markets*, 17, 230-261.
37. O'Hara, M., C. Yao, and M. Ye (2014). "What's Not There: Odd Lots." *J. Finance*, 69(5), 2199-2236.
38. Ye, M. and W. Zhu (2020). "Strategic Informed Trading and Dark Pools." Working paper.
39. Chakrabarty, B., B. Li, V. Nguyen, and R.A. Van Ness (2007). "Trade Classification Algorithms for ECN Trades." *J. Banking & Finance*, 31(12), 3806-3821.
40. Chakrabarty, B., R. Pascual, and A. Shkilko (2015). "Evaluating Trade Classification Algorithms." *J. Financial Markets*, 25, 52-79.
41. Easley, D., R.F. Engle, M. O'Hara, and L. Wu (2008). "Time-Varying Arrival Rates of Informed and Uninformed Trades." *J. Financial Econometrics*, 6(2), 171-207.
42. Hagströmer, B. "Bias in the Effective Bid-Ask Spread." Working paper.
43. Hamilton, J.D. (1989). "A New Approach to the Economic Analysis of Nonstationary Time Series and the Business Cycle." *Econometrica*, 57(2), 357-384.
44. Hasbrouck, J. (2021). "Price Discovery in High Resolution." NYU working paper.
45. Holden, C.W. and S. Jacobsen (2014). "Liquidity Measurement Problems in Fast, Competitive Markets." *J. Financial Economics*, 113(3), 559-574.
46. O'Hara, M. and M. Ye (2011). "Is Market Fragmentation Harming Market Quality?" *J. Financial Economics*, 101(3), 459-474.
47. Panayides, M.A., T.D. Shohfi, and J.D. Smith (2019). "Bulk Classification of Trading Activity." *J. Banking & Finance*, 103, 113-129.

### Pipeline-Internal References

48. E9 Off-Exchange Signal Validation: `scripts/e9_offexchange_signal_validation.py`, `scripts/e9_results_all_days.json`
49. E9 Cross-Validation Audit: `scripts/e9_cross_validation.py`, `scripts/e9_crossval_results.json`
50. E8 Model-Execution Diagnostic: `lob-model-trainer/reports/e8_model_execution_diagnostic_2026_03.md`
51. Off-Exchange Trading Research: `Off-Exchange Trading Research for NVDA.md`
52. Market Microstructure Research: `Market microstructure research for multi-venue HFT signal extraction.md`
