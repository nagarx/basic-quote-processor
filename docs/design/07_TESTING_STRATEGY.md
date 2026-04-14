# 07: Testing & Verification Strategy

**Date**: 2026-03-22
**Status**: APPROVED — **Implementation Status**: Phases 1-5 complete
**Module**: `basic-quote-processor`
**Test target**: ~105 (original) → **412 tests achieved** (365 lib + 47 integration across 5 test files)

---

## Table of Contents

1. [Phased Implementation Plan](#1-phased-implementation-plan)
2. [Documentation Consistency Notes](#2-documentation-consistency-notes)
3. [Cross-Validation Against E9](#3-cross-validation-against-e9)
4. [Test Categories](#4-test-categories)
5. [Future-Proofing Gates](#5-future-proofing-gates)
6. [Appendix: HFT Architect Review Summary](#appendix-hft-architect-review-summary)

---

## 1. Phased Implementation Plan

Six phases with explicit decision gates. Each gate is a hard prerequisite -- if a phase fails its gate, the next phase does NOT begin.

### Phase 1: Data Reader + BBO State (Foundation)

**Scope**:
- `reader/` module: read XNAS.BASIC `.dbn.zst` files via `dbn` crate
- `bbo_state/` module: track Nasdaq L1 BBO from quote updates
- `CmbpRecord` internal type with all needed fields (including both `ts_recv` and `ts_event`)
- `PublisherId` enum (XNAS=81, FINN=82, FINC=83, XBOS=88, XPSX=89)
- Price precision chain documented: `i64` nanodollars on wire -> `CmbpRecord` -> `f64` USD at BboState boundary

**Tests**:
- Read a real `.dbn.zst` file, verify total record count
- BBO tracking accuracy: bid/ask prices and sizes match expected values for known records
- Publisher ID classification: TRF (82, 83) vs lit (81) vs minor lit (88, 89)
- `ts_recv` vs `ts_event` both preserved in `CmbpRecord`
- Edge: file with zero records, corrupted file, file with only quote updates (no trades)

**Decision gate**: Can iterate all records for a day and track BBO correctly. Verify BBO mid/spread against a manual spot-check of 10 randomly selected records.

**Dependencies**: None (foundation phase).

---

### Phase 2: Trade Classifier

**Scope**:
- `trade_classifier/` module: midpoint signing (Barber 2024) + BJZZ retail identification (Boehmer 2021)
- `ClassifiedTrade` type with `direction` (Buy/Sell/Unsigned) + `retail_status` (Retail/Institutional/Unknown)
- Configurable exclusion band (default 0.10 of spread)
- BVC (Bulk Volume Classification) for VPIN computation (Easley 2012)

**Tests**:
- Hand-calculated signing examples (trade above mid -> Buy, below mid -> Sell, within exclusion band -> Unsigned)
- BJZZ thresholds: subpenny detection, retail buy/sell classification
- Edge: spread=0 (all trades Unsigned), trade exactly at midpoint, BBO with ask < bid (crossed)
- BVC sigma computation determinism (golden test for known price sequence)

**Decision gate**: Signing accuracy consistent with E9 validation metrics:
- Retail ID rate: 45.3% +/-2%
- Unsigned rate: 15.4% +/-2%

**Dependencies**: Phase 1 (requires BboState for midpoint computation).

---

### Phase 3: Feature Extraction

**Scope**:
- `features/` module: all 10 feature groups (34 features total)
- `accumulator/` module: per-bin accumulation with explicit reset semantics
- `sampling/` module: time-bin sampler (grid-aligned to 09:30 ET) and volume-bin sampler (for VPIN)
- Empty bin policy: forward-fill for state features, zero for flow features
- Safety gates: `bin_valid` and `bbo_valid`

**Tests**:
- Feature values for a known input day match E9 Python script output within tolerance
- Each feature group independently toggleable via config
- Accumulator reset produces clean state (no leakage between bins)
- Empty bin forward-fill and zero-fill produce correct values
- NaN guard: every division protected by EPS (1e-8)

**Decision gate**: IC values for key features match E9 within tolerance:
- `trf_signed_imbalance` IC at H=10: +0.040 +/-0.005
- `subpenny_intensity` IC at H=10: +0.048 +/-0.005
- `dark_share` IC at H=10: +0.035 +/-0.005

**Dependencies**: Phase 2 (requires ClassifiedTrade for signed flow and retail metrics).

---

### Phase 4: Sequence Building + Labels + Export

**Scope**:
- `sequence_builder/` module: sliding window over feature bins
- `labeling/` module: point-return labels at multiple horizons
- `export/` module: NPY + metadata JSON + normalization JSON
- Forward price trajectory export

**Tests**:
- Output shape validation: sequences [N, T, F], labels [N, H], forward_prices [N, max_H+1]
- Metadata JSON completeness: all required fields present per `pipeline_contract.toml`
- Label correctness: `point_return(t, H) = (mid[t+H] - mid[t]) / mid[t] * 10000` verified by hand for 5 samples
- No NaN or Inf in exported NPY files (`is_finite()` check)
- Sliding window stride correctness

**Decision gate**: Full day processed and exported successfully. Metadata validates against pipeline contract. Exported files loadable by Python `numpy.load()` with correct shapes and dtypes.

**Dependencies**: Phase 3 (requires feature vectors for sequence construction).

---

### Phase 5: EQUS_SUMMARY Integration + CLI Tools

**Scope**:
- `DailyContextLoader`: load EQUS_SUMMARY for consolidated volume
- `validate_coverage` CLI: cross-check TRF + lit volume against consolidated volume
- `profile_data` CLI: per-day statistics (trade counts, volumes, publisher breakdown)
- `export_dataset` CLI: production entry point for multi-day export with train/val/test splits
- Half-day auto-detection: 5 consecutive empty bins -> day complete

**Tests**:
- Coverage ratio matches expected range (81-85% +/-1%)
- `true_dark_share` = TRF_daily_volume / consolidated_volume produces reasonable values
- EQUS_SUMMARY missing: graceful fallback (per-bin features still work, daily `true_dark_share` unavailable)
- Half-day detection: day ending at 13:00 ET produces correct `session_progress = 1.0` at actual close
- Multi-day export with correct train/val/test split boundaries

**Decision gate**: Multi-day export with train/val/test splits. Total samples for 35 test days: 8,337 +/-0. Coverage ratios validated against EQUS_SUMMARY.

**Dependencies**: Phase 4 (requires export pipeline for CLI tools).

---

### Phase 6: Python Analysis Scripts

**Scope**:
- `analysis/signal_validation.py`: IC computation and horizon sweep with bootstrap CI
- `analysis/feature_correlation.py`: feature correlation matrix
- `analysis/coverage_analysis.py`: EQUS_SUMMARY cross-check visualization
- `analysis/explore_cmbp1.py`: data exploration and statistics

**Tests**:
- IC computation matches E9 cross-validation results within tolerance
- Bootstrap CIs computed correctly (1000 iterations, 95% CI)
- Correlation matrix symmetric and diagonal = 1.0

**Decision gate**: Reproduces E9 cross-validation results. All IC values within specified tolerances. Bootstrap CIs for top 3 features exclude zero.

**Dependencies**: Phase 5 (requires exported datasets for analysis).

---

### Phase Dependency Graph

```
Phase 1 (Reader + BBO)
   |
   v
Phase 2 (Trade Classifier)
   |
   v
Phase 3 (Feature Extraction)
   |
   v
Phase 4 (Sequences + Labels + Export)
   |
   v
Phase 5 (EQUS_SUMMARY + CLI)
   |
   v
Phase 6 (Python Analysis)
```

All dependencies are strictly sequential. No phase can begin until its predecessor's gate is satisfied.

---

## 2. Documentation Consistency Notes

Five inconsistencies identified during context loading. All are non-blocking for implementation but must be resolved.

| # | Document | Issue | Resolution | Status |
|---|----------|-------|------------|--------|
| 1 | `brainstorming/CONSOLIDATED_BRAINSTORMING.md` | Says "Do NOT create separate repo" | Already annotated with SUPERSEDED (2026-03-22) | **RESOLVED** |
| 2 | `plan/UNIFIED_PIPELINE_ARCHITECTURE_PLAN.md` (Section 21.1) | MF-4 status | Updated MF-2, MF-3, MF-4 to DONE (2026-03-23) | **RESOLVED** |
| 3 | `plan/UNIFIED_PIPELINE_ARCHITECTURE_PLAN.md` (Phase MF-4) | Status PENDING | Updated to DONE (Phases 1-5, 412 tests, 3 CLIs) | **RESOLVED** |
| 4 | `VALIDATED_TECHNICAL_REPORT.md` | H1: "Missing basic-quote-processor" | Updated to IMPLEMENTED (2026-03-23) | **RESOLVED** |
| 5 | `lob-model-trainer/EXPERIMENT_INDEX.md` (E9 entry) | Status: "INVESTIGATE" | Updated to RESOLVED (2026-03-23). Phase 6 Python IC validation still pending. | **RESOLVED** |

**All 5 documentation updates completed on 2026-03-23** in the same session as Phase 5 implementation completion.

---

## 3. Cross-Validation Against E9

The primary verification strategy is reproducing E9 results from the Rust implementation. E9 was conducted in Python using `databento` and `pandas`; the Rust implementation must produce statistically equivalent results.

### 3.1 Metrics Table

| Metric | E9 Value | Tolerance | Source File | How to Verify |
|--------|----------|-----------|-------------|---------------|
| `trf_signed_imbalance` IC (H=10, 60s bins) | +0.040 | +/-0.005 | `e9_crossval_results.json` | Spearman rank correlation between feature and H=10 point return, computed per day then averaged |
| `subpenny_intensity` IC (H=10, 60s bins) | +0.048 | +/-0.005 | `e9_crossval_results.json` | Same method |
| `dark_share` IC (H=10, 60s bins) | +0.035 | +/-0.005 | `e9_crossval_results.json` | Same method |
| Total samples (35 test days, 60s bins) | 8,337 | +/-0 (exact) | `e9_results_all_days.json` | Count of valid sequences in test split |
| Retail ID rate (BJZZ) | 45.3% | +/-2% | E9 Python script | `n_retail_trades / n_total_trf_trades` across full dataset |
| Coverage vs EQUS_SUMMARY | 81-85% | +/-1% | EQUS_SUMMARY cross-check | `(trf_volume + lit_volume) / consolidated_volume` daily average |

### 3.2 How to Run Cross-Validation

**Step 1**: Export a single test day using both the E9 Python script and the Rust `export_dataset` binary with identical config:
```bash
# Rust
./target/release/export_dataset --config configs/nvda_60s.toml --dates 2025-12-01

# Python (E9 reference)
python analysis/signal_validation.py --date 2025-12-01 --bin-size 60
```

**Step 2**: Compare feature vectors bin-by-bin:
- Load both outputs as numpy arrays
- Compute per-feature max absolute difference
- All differences must be < 1e-6 (f64 computation tolerance)

**Step 3**: Compute IC on full test split:
- Export all 35 test days from Rust implementation
- Run `analysis/signal_validation.py` on exported NPY files
- Compare IC values against E9 reference values in the table above

**Step 4**: Verify sample counts:
- Total sequences must match exactly (8,337 for 35 test days)
- Per-day sequence counts must match E9 per-day breakdown

### 3.3 What to Do If a Metric Does Not Match

| Failure Mode | Diagnosis | Resolution |
|---|---|---|
| IC outside tolerance (+/-0.005) | Check bin alignment: are bins grid-aligned to 09:30 ET? Check signing: is BBO updated BEFORE trade classification? Check empty bin policy: are NaN bins handled identically? | Compare bin-by-bin feature values for a single day to isolate the divergence |
| Sample count mismatch (not 8,337) | Check warmup bin count (should be 3). Check half-day handling. Check market_close cutoff. | Export per-day counts and compare against E9 per-day breakdown |
| Retail ID rate outside tolerance | Check BJZZ thresholds: bjzz_lower=0.001, bjzz_upper=0.999, bjzz_upper_sell=0.40, bjzz_lower_buy=0.60. Check subpenny extraction: `frac = price - floor(price)`, verify precision | Run BJZZ on a small set of known trades and compare classification |
| Coverage outside tolerance | Check publisher ID mapping: TRF=[82,83], lit=[81]. Check whether minor lit [88,89] are included. Check EQUS_SUMMARY date alignment | Profile a single day's publisher ID distribution |
| Per-feature max diff > 1e-6 | Floating-point divergence between Python and Rust implementations | Identify which feature diverges, trace the formula, check for order-of-operations differences (e.g., sum then divide vs divide then sum) |

**Escalation**: If any metric fails cross-validation after diagnosis, the Phase 3 or Phase 6 gate (depending on which metric) is BLOCKED. Do not proceed until the root cause is identified and resolved.

---

## 4. Test Categories

Six categories, ~105 tests total. All tests in `tests/` directory as Rust integration tests.

### 4.1 Contract Tests (~30 tests) -- `contract_test.rs`

Verify that feature indices, schema version, metadata format, and export structure match the pipeline contract.

**What to test**:

| Test | Description |
|------|-------------|
| Feature index range | Total feature count = 34, indices span [0, 33] |
| `signed_flow` group | Indices [0-3]: trf_signed_imbalance, mroib, inv_inst_direction, bvc_imbalance |
| `venue_metrics` group | Indices [4-7]: dark_share, trf_volume, lit_volume, total_volume |
| `retail_metrics` group | Indices [8-11]: subpenny_intensity, odd_lot_ratio, retail_trade_rate, retail_volume_fraction |
| `bbo_dynamics` group | Indices [12-17]: spread_bps, bid_pressure, ask_pressure, bbo_update_rate, quote_imbalance, spread_change_rate |
| `vpin` group | Indices [18-19]: trf_vpin, lit_vpin |
| `trade_size` group | Indices [20-23]: mean_trade_size, block_trade_ratio, trade_count, size_concentration |
| `cross_venue` group | Indices [24-26]: trf_burst_intensity, time_since_burst, trf_lit_volume_ratio |
| `activity` group | Indices [27-28]: bin_trade_count, bin_trf_trade_count |
| `safety_gates` group | Indices [29-30]: bin_valid, bbo_valid |
| `context` group | Indices [31-33]: session_progress, time_bucket, schema_version |
| Schema version | Feature at index 33 == 1.0 (off-exchange schema v1.0) |
| pipeline_contract.toml match | `contract.rs` constants match `[features.off_exchange]` in TOML |
| Metadata required fields | day, n_sequences, window_size, n_features, schema_version, contract_version, label_strategy, normalization, provenance, export_timestamp |
| Metadata label_strategy | Must be "point_return" (never "smoothed_average") |
| NPY sequences dtype | float32 |
| NPY labels dtype | float64 |
| NPY forward_prices dtype | float64 |
| Sequences shape | [N, T, F] where F matches enabled feature count |
| Labels shape | [N, H] where H = number of configured horizons |
| Forward prices shape | [N, max_H + 1] |
| Feature count formula | Count = sum of enabled group sizes |
| Group toggle | Disabling a group removes exactly that group's features, indices shift down |
| Sign convention | trf_signed_imbalance > 0 when buy_vol > sell_vol (bullish) |
| Sign convention (mroib) | mroib > 0 when retail buys > retail sells (retail buy pressure = bullish, per Boehmer et al. 2021). Note: inv_inst_direction = -mroib provides the institutional accumulation proxy |
| Categorical features | bin_valid, bbo_valid, time_bucket, and schema_version excluded from normalization (indices [29, 30, 32, 33]) |
| Safety gate values | bin_valid and bbo_valid are exactly 0.0 or 1.0, never intermediate |
| Publisher ID constants | TRF = [82, 83], LIT = [81], MINOR_LIT = [88, 89] |
| EPS constant | EPS = 1e-8 (exact match with pipeline-wide constant) |
| Label unit | Labels in basis points (bps), not raw returns |

### 4.2 Formula Tests (~20 tests) -- `formula_test.rs`

Verify that every formula matches its cited paper with hand-calculated expected values.

**What to test**:

| Test | Formula | Citation | Example Input | Expected Output |
|------|---------|----------|---------------|-----------------|
| Midpoint signing | `sign = if trade_px > mid + excl*spread then Buy; if trade_px < mid - excl*spread then Sell; else Unsigned` | Barber et al. (2024), Section 3 | bid=100.00, ask=100.10, excl=0.10, trade=100.08 | Buy (100.08 > mid(100.05) + 0.10*0.10(=0.01) = 100.06) |
| BJZZ subpenny | `frac_cent = (trade_px * 100.0) mod 1.0; retail = frac_cent > bjzz_lower AND frac_cent < bjzz_upper` | Boehmer et al. (2021), Section I.B; Z = 100 * mod(Price, 0.01) | trade_px=100.0035 | Retail sell (frac_cent=0.35, within (0.001, 0.40)) |
| BJZZ direction | `if frac_cent > bjzz_lower_buy then RetailBuy; if frac_cent < bjzz_upper_sell then RetailSell` | Boehmer et al. (2021) | trade_px=100.0075, frac_cent=0.75 | RetailBuy (0.75 > 0.60) |
| TRF signed imbalance | `(buy_vol - sell_vol) / (buy_vol + sell_vol + EPS)` | Cont et al. (2014) adapted | buy=500, sell=300 | +0.250 |
| MROIB | `(retail_buy_vol - retail_sell_vol) / (retail_buy_vol + retail_sell_vol + EPS)` | Boehmer et al. (2021), Section 4 | r_buy=200, r_sell=100 | +0.333 |
| Dark share | `trf_volume / (trf_volume + lit_volume + EPS)` | SEC Market Structure Reports | trf=600, lit=400 | 0.600 |
| Subpenny intensity | `n_subpenny_trades / (n_total_trades + EPS)` | Boehmer et al. (2021) | subpenny=45, total=100 | 0.450 |
| Odd lot ratio | `n_odd_lot_trades / (n_total_trades + EPS)` | SEC Rule 606 definition | odd=30, total=100 | 0.300 |
| BVC | `V_buy = V * Phi((P_i - P_{i-1}) / sigma)` | Easley et al. (2012), Eq. 3 | delta_P=0.05, sigma=0.10, V=1000 | V_buy = 1000 * Phi(0.5) = 691.5 |
| VPIN | `sum(abs(V_buy_i - V_sell_i)) / (n * V_bucket)` | Easley et al. (2012), Eq. 5 | 5 buckets, known buy/sell splits | Hand-computed value |
| Spread BPS | `(ask - bid) / mid * 10000` | Standard | bid=100.00, ask=100.10 | 10.0 bps |
| Quote imbalance | `(bid_sz - ask_sz) / (bid_sz + ask_sz + EPS)` | Standard | bid_sz=500, ask_sz=300 | +0.250 |
| Point return | `(mid[t+H] - mid[t]) / mid[t] * 10000` | Standard | mid_t=100.00, mid_t+H=100.05 | +5.0 bps |
| Session progress | `(t - market_open) / (market_close - market_open)` | Custom | t=12:45, open=09:30, close=16:00 | 0.500 |
| Time bucket | 7-regime classification: pre-market, open-auction, morning, midday, afternoon, close-auction, post-market | pipeline_contract.toml | 10:15 ET | morning (regime 2) |
| Inv institutional direction | `-(mroib)` sign flip | Boehmer et al. (2021) | mroib=+0.333 | inv_inst=-0.333 |
| Retail trade rate | `n_retail_trades / n_total_trf_trades` | Custom | retail=45, trf=100 | 0.450 |
| Block trade ratio | `n_block_trades / n_total_trades` | SEC Rule 600 (>10K shares) | block=5, total=200 | 0.025 |
| TRF burst intensity | `trf_trades_in_burst_window / burst_window_seconds` | Custom | 15 trades in 5s | 3.0 trades/sec |
| Kyle's lambda | `lambda = Cov(delta_P, signed_V) / Var(signed_V)` | Kyle (1985), Proposition 1 | Known price/volume series | Hand-computed OLS slope |

### 4.3 Edge Tests (~25 tests) -- `edge_test.rs`

Test numerical edge cases, degenerate inputs, and boundary conditions.

**What to test**:

| Test | Input Condition | Expected Behavior |
|------|----------------|-------------------|
| Zero spread | bid == ask | All trades Unsigned (exclusion band = 0). spread_bps = 0.0. No division error. |
| Negative spread (crossed BBO) | ask < bid | bbo_valid = 0.0. BBO state NOT updated (reject crossed quotes). |
| Zero TRF trades in bin | No TRF trades for 60s | Flow features = 0.0. State features = forward-fill. bin_valid = 0.0. |
| Zero lit trades in bin | Only TRF trades | dark_share = 1.0. lit_volume = 0.0. No division error. |
| Zero total trades in bin | No activity | All ratio features guarded by EPS. bin_valid = 0.0. bbo_valid depends on quote updates. |
| Trade price = NaN | Corrupted record | Record skipped with counter increment. No NaN propagation. |
| Trade size = 0 | Zero-size trade | Record skipped (invalid trade). |
| Trade price = Inf | Overflow record | Record skipped. No Inf propagation. |
| BBO price = 0 | Uninitialized state | bbo_valid = 0.0. Midpoint = 0.0. No signing attempted. |
| Very large trade size | size = u32::MAX | Accumulator handles without overflow (use u64 for volume sums). |
| Single trade in bin | n_trades = 1 | bin_valid = 0.0 (below min_trades_per_bin = 10). Features computed but gated. |
| All trades same direction | 100% buy | trf_signed_imbalance = +1.0. mroib depends on retail classification. |
| All trades Unsigned | exclusion_band = 1.0 | trf_signed_imbalance = 0.0 (buy_vol = sell_vol = 0). |
| Spread = 1 nanodollar | Minimum non-zero spread | spread_bps = very small positive. No precision issues with f64. |
| Half-day (market closes 13:00) | No records after 13:00 ET | Auto-detect after 5 empty bins. session_progress = 1.0 at detected close. Labels do not reference non-existent future bins. |
| First bin of day (warmup) | Bin index < warmup_bins | Bin discarded. Not included in sequences or normalization. |
| Last bin of day (label truncation) | Not enough future bins for max horizon | Labels for unavailable horizons = NaN. Sequence included only if at least one horizon is valid. |
| Day with only quote updates | No trades at all | Zero sequences produced. Metadata records n_sequences = 0. |
| Pre-market records | Records before 09:30 ET | BBO updated (warm start). No bins emitted before market_open. |
| Post-market records | Records after 16:00 ET | Processing stops. No bins emitted after market_close. |
| DST transition day | Spring forward / fall back | Bin boundaries correct via `hft_statistics::time::regime::utc_offset_for_date()` (returns -4 EDT or -5 EST). |
| Bin boundary exactly on record timestamp | ts_recv == bin_boundary_ns | Record belongs to the ENDING bin (bin includes its start, excludes its end). |
| Multiple records with identical timestamp | Ties in ts_recv | All processed in file order. Deterministic output. |
| EQUS_SUMMARY missing for a day | No consolidated volume | `true_dark_share` unavailable (NaN or omitted). Per-bin features still computed. |
| EQUS_SUMMARY volume = 0 | Trading halt or data error | Guarded by EPS. `true_dark_share` = 0.0. Warning logged. |

### 4.4 Signing Tests (~15 tests) -- `signing_test.rs`

Trade classification accuracy tests for midpoint signing, BJZZ, and BVC.

**What to test**:

| Test | Scenario | Expected |
|------|----------|----------|
| Clear buy | trade_px = ask (at the offer) | Buy |
| Clear sell | trade_px = bid (at the bid) | Sell |
| Midpoint trade | trade_px = (bid + ask) / 2 exactly | Unsigned (within exclusion band) |
| Just above exclusion band | trade_px = mid + excl*spread + 1e-9 | Buy |
| Just below exclusion band | trade_px = mid - excl*spread - 1e-9 | Sell |
| Inside exclusion band (upper) | trade_px = mid + excl*spread - 1e-9 | Unsigned |
| Inside exclusion band (lower) | trade_px = mid - excl*spread + 1e-9 | Unsigned |
| Exclusion band = 0 | All trades signed | Only exact-midpoint trades are Unsigned |
| Exclusion band = 1.0 | All trades in exclusion zone | All Unsigned |
| BJZZ retail buy | frac = 0.75 (> bjzz_lower_buy=0.60) | RetailBuy |
| BJZZ retail sell | frac = 0.25 (< bjzz_upper_sell=0.40) | RetailSell |
| BJZZ unknown retail direction | frac = 0.50 (between 0.40 and 0.60) | Retail, direction Unknown |
| BJZZ non-retail | frac = 0.00 (round dollar) | NotRetail |
| BJZZ boundary | frac = 0.001 exactly | Retail (>= bjzz_lower) |
| BBO update before signing | Trade record carries updated BBO | Signing uses the record's BBO, not the previous record's |

### 4.5 Integration Tests (~10 tests) -- `integration_test.rs`

End-to-end tests processing real or synthetic data through the full pipeline.

**What to test**:

| Test | Scope | Validation |
|------|-------|------------|
| Single day, full pipeline | Process one real `.dbn.zst` file end-to-end | Output files exist, shapes correct, metadata valid |
| Deterministic output | Process same day twice | Byte-identical NPY output |
| Pipeline reset | Process day A, reset, process day B | Day B output identical to processing day B alone |
| Multi-day sequential | Process 3 consecutive days | Each day independent, no state leakage |
| Config: all features enabled | 34 features | F dimension = 34 in output |
| Config: minimal features | Only signed_flow + safety_gates + context | F dimension = 4 + 2 + 3 = 9 |
| Config: VPIN enabled | Add vpin group | VPIN values are finite and in [0, 1] range |
| Train/val/test split | Multi-day with split dates | Files in correct split directories, no date overlap |
| Empty day handling | Day with 0 valid sequences | No crash, empty output directory or metadata with n_sequences=0 |
| Large day (stress test) | Day with maximum record count | Completes within performance budget, no OOM |

### 4.6 Golden Tests (~5 tests) -- `golden_test.rs`

Deterministic output tests for fixed synthetic input. These tests pin exact numerical values and detect any accidental change to computation logic.

**What to test**:

| Test | Input | Pinned Output |
|------|-------|---------------|
| Golden: midpoint signing | 10 synthetic trades with known prices and BBO | Exact classification vector: [Buy, Sell, Unsigned, Buy, ...] |
| Golden: BJZZ classification | 10 trades with known fractional cents | Exact retail status vector: [Retail, NotRetail, Retail, ...] |
| Golden: feature vector | 1 synthetic bin with known trades and BBO | Exact 34-element f64 vector (pinned to 10 decimal places) |
| Golden: BVC computation | 5 price changes with known sigma | Exact buy_volume fractions (pinned to 10 decimal places) |
| Golden: point return label | 3 consecutive bins with known midpoints | Exact label values in bps (pinned to 10 decimal places) |

**Golden test maintenance**: If any formula changes, the golden test MUST be updated in the same commit. A failing golden test blocks all CI.

---

## 5. Future-Proofing Gates

Ten gates from the pipeline architecture plan. Each gate specifies how `basic-quote-processor` passes it.

### Gate 1: Multi-symbol

**How this module passes**: Symbol is a TOML config parameter (`[input] symbol = "NVDA"`). No NVDA-specific constants exist in feature computation. The file pattern is parameterized. Changing `symbol = "AAPL"` requires only config change, no code change.

**Verification**: Run `export_dataset` with a non-NVDA symbol config (if data available). Alternatively, grep codebase for hardcoded "NVDA" strings outside of config files and test fixtures.

---

### Gate 2: Multi-asset

**How this module passes**: N/A for CMBP-1 (equity only). Types are generic and do not assume equity-specific properties (e.g., tick sizes, lot sizes are configurable, not hardcoded). If CMBP-1 data becomes available for other asset classes, the module should work without structural changes.

**Verification**: Code review -- no equity-specific assumptions in core computation types.

---

### Gate 3: Multi-exchange

**How this module passes**: Publisher IDs are configurable in `[publishers]` config section. TRF publishers ([82, 83]), lit publishers ([81]), and minor lit publishers ([88, 89]) are all config-driven. Works with any CMBP-1 feed that uses Databento publisher ID conventions.

**Verification**: Contract test confirms publisher IDs are read from config, not hardcoded in computation logic.

---

### Gate 4: Streaming-compatible

**How this module passes**: Core computation (BboState, TradeClassifier, BinAccumulator, FeatureExtractor) has NO file I/O. All types accept individual records or accumulated bin data via method calls. `DbnReader` is the sole I/O boundary. The computation core can be wrapped in a streaming adapter without modification.

**Verification**: Integration test demonstrating that the feature pipeline can be driven by a `Vec<CmbpRecord>` in memory (no file system access).

---

### Gate 5: Experiment-configurable

**How this module passes**: All parameters in TOML config. Three example configs provided (`nvda_60s.toml`, `nvda_10s.toml`, `nvda_vpin.toml`). Bin size, horizons, feature groups, classification thresholds, validation parameters -- all configurable without code changes.

**Verification**: Run exports with at least 2 different configs and verify distinct output (different feature counts, different bin sizes).

---

### Gate 6: Regime-aware

**How this module passes**: `session_progress` (feature index 31) provides continuous time-of-day context. `time_bucket` (feature index 32) provides discrete 7-regime classification (pre-market, open-auction, morning, midday, afternoon, close-auction, post-market) from `hft-statistics` `RegimeClassifier`.

**Verification**: Formula test for session_progress (see Section 4.2). Contract test for time_bucket values matching pipeline contract 7-regime definitions.

---

### Gate 7: Calendar-aware

**How this module passes**: DST handling via `hft_statistics::time::regime::utc_offset_for_date()` (exact DST rules: 2nd Sunday March, 1st Sunday November). Bin boundaries computed in ET (Eastern Time), correctly handling spring-forward and fall-back transitions. Half-day auto-detection: if no records arrive for 5 consecutive bins, the day is treated as complete, removing the need for a hardcoded market calendar.

**Verification**: Edge test for DST transition day (see Section 4.3). Edge test for half-day detection.

---

### Gate 8: Contract-registered

**How this module passes**: Off-exchange features are registered in `contracts/pipeline_contract.toml` under `[features.off_exchange]` section:

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

Local `src/contract.rs` mirrors these definitions. `contracts/verify_rust_constants.py` is extended to validate `basic-quote-processor/src/contract.rs` against this TOML section. Python constants auto-generated via `contracts/generate_python_contract.py` under `OffExchangeFeatureIndex` for downstream consumers.

**Verification**: Contract test in `contract_test.rs` verifies `contract.rs` constants against `pipeline_contract.toml`. CI runs `verify_rust_constants.py` which covers both MBO extractor and basic-quote-processor.

**Status**: PASSES. This was the sole FAIL in the initial architect review (Gate 8 failed because the original plan proposed a local-only contract without TOML registration). The plan was updated to include full TOML registration.

---

### Gate 9: Primitives-reused

**How this module passes**: Shared primitives from `hft-statistics`:
- `WelfordAccumulator` -- running mean/variance for normalization
- `StreamingDistribution` -- quantiles, skewness for trade size analysis
- `RegimeClassifier` -- 7-regime time classification
- `time::regime::{utc_offset_for_date, day_epoch_ns, time_regime}` -- EST/EDT handling for market hours
- `AcfComputer` -- autocorrelation for signal diagnostics

Kyle's lambda computation uses `KyleLambda` from `hft-statistics` (moved there per Recommendation 6, rather than implementing locally).

**Verification**: `Cargo.toml` dependency check -- `hft-statistics` is a path dependency. No re-implementation of Welford, streaming quantiles, or regime classification in this crate.

---

### Gate 10: Research-cited

**How this module passes**: Every formula in the feature specification traces to a cited paper. All citations are documented in `src/` docstrings and in `docs/design/01_THEORETICAL_FOUNDATION.md` (47 papers, 1,274 lines). Formula tests (Section 4.2) include citations for each expected value.

**Verification**: Code review -- every public function that computes a feature includes a docstring with paper reference (Author, Year, Section/Equation number).

---

## Appendix: HFT Architect Review Summary

Full structural review conducted 2026-03-22 by the HFT Pipeline Architect. Archived at `.claude/plans/golden-questing-moore-agent-aa0e8d553d863abd6.md` (archived in parent HFT-pipeline-v2 repo).

### Review Scope

- 7 Design Gates evaluated
- 10 Future-Proofing Gates checked
- 6 Structural Anti-Pattern audits
- Module boundary review
- Data flow integrity analysis
- Configuration design review
- Missing component identification

### Result Summary

| Section | Verdict |
|---------|---------|
| Design Gates (7) | 6 PASS, 1 PASS-with-issues |
| Future-Proofing (10) | 9 PASS, 1 FAIL (fixed) |
| Anti-Patterns | 1 TRIGGERED (fixed), 1 PARTIAL (fixed) |
| Module Boundaries | All correct |
| Data Flow | 3 HIGH risks (all fixed) |
| Contracts | Correct direction, underspecified (fixed) |
| Config | 6 missing parameters (all added) |
| Missing Components | 6 identified (all addressed) |

### 8 Recommendations and Incorporation

| # | Recommendation | Severity | How Incorporated |
|---|----------------|----------|------------------|
| R1 | Register off-exchange contract in `pipeline_contract.toml` | CRITICAL | Added `[features.off_exchange]` section to TOML. Extended `verify_rust_constants.py`. Extended `generate_python_contract.py` for `OffExchangeFeatureIndex`. Local `contract.rs` mirrors TOML definitions. Gate 8 now PASSES. |
| R2 | Specify BBO update order explicitly | HIGH | Processing lifecycle (Section 3.2 of plan) updated with explicit 3-step order for trade records: (1) `BboState.update_from_record(record)`, (2) `TradeClassifier.classify(record, &bbo_state)`, (3) `BinAccumulator.accumulate(classified_trade, &bbo_state)`. NOTE comment added explaining why BBO update before classification is MANDATORY. |
| R3 | Define empty bin policy | HIGH | Added `[validation]` config section with `empty_bin_policy = "forward_fill_state"`. State features (subpenny_intensity, odd_lot_ratio, dark_share) forward-fill. Flow features (trf_signed_imbalance, volumes) zero-fill. Added `bin_valid` safety gate feature. Added `warmup_bins = 3` (discard first N bins/day). Added `min_trades_per_bin = 10`. |
| R4 | Document price precision chain | MEDIUM | Added D7 (Price precision chain) to Section 2.2 of plan: `i64` nanodollars on wire -> `CmbpRecord` i64 -> `BboState` f64 USD (converted once at update boundary) -> feature computation f64 -> NPY export f32 (downcast with finite check). |
| R5 | Add half-day auto-detection | MEDIUM | Added D6 (Half-day auto-detection) to plan. Config: `auto_detect_close = true`, `close_detection_gap_bins = 5`. Session progress adjusted to actual detected close (1.0 at close, not at 16:00). No hardcoded calendar required. |
| R6 | Move Kyle's lambda to hft-statistics | LOW | Kyle's lambda implementation placed in `hft-statistics` shared crate (Level 5 temporal dynamics primitive). `basic-quote-processor` imports via `hft-statistics` path dependency. Feature file `kyle_lambda.rs` calls shared implementation. |
| R7 | Formalize fusion contract as separate document | MEDIUM | Fusion contract formalized in `06_INTEGRATION_POINTS.md`. Alignment validation rules: both metadata JSONs must agree on `bin_size_seconds`, `market_open_et`, `date`, `n_bins_per_day`. Missing bin handling specified. Feature dimension registration defined. |
| R8 | Add `bin_trade_count` as a feature | LOW | Added `activity` group (indices [27-28]) with `bin_trade_count` (all venues) and `bin_trf_trade_count` (TRF only). Total feature count increased from 30 to 34 (also includes `safety_gates` group [29-30] from R3). |

### Final Validation: 15/15 Checks PASS

| # | Check | Status |
|---|-------|--------|
| 1 | Contract registered in `pipeline_contract.toml` | PASS |
| 2 | `verify_rust_constants.py` extended for basic-quote-processor | PASS |
| 3 | Python constants auto-generated via `generate_python_contract.py` | PASS |
| 4 | BBO update order explicitly documented (update BEFORE classify) | PASS |
| 5 | Empty bin policy defined (forward-fill state, zero flow) | PASS |
| 6 | `bin_valid` safety gate feature added | PASS |
| 7 | `bbo_valid` safety gate feature added | PASS |
| 8 | Price precision chain documented (i64 -> f64 -> f32) | PASS |
| 9 | Half-day auto-detection specified | PASS |
| 10 | Kyle's lambda in hft-statistics (shared crate) | PASS |
| 11 | Fusion contract formalized in 06_INTEGRATION_POINTS.md | PASS |
| 12 | `bin_trade_count` and `bin_trf_trade_count` features added | PASS |
| 13 | `min_trades_per_bin` validation parameter added | PASS |
| 14 | `warmup_bins` parameter added | PASS |
| 15 | Sign convention documented (bullish > 0, bearish < 0) | PASS |
