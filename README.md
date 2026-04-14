# basic-quote-processor

Off-exchange trade processing for XNAS.BASIC CMBP-1 data. Extracts 34 off-exchange features from Nasdaq consolidated L1 quotes and trade prints, with TRF trade signing (Barber 2024), BJZZ retail identification (Boehmer 2021), and BVC volume classification (Easley 2012).

## Overview

This crate processes Databento XNAS.BASIC CMBP-1 files — consolidated Level 1 quotes and trade prints from Nasdaq-listed securities. It identifies off-exchange (TRF) trades, classifies them by direction (midpoint signing) and retail status (BJZZ subpenny analysis), accumulates per-bin statistics, and exports 34-feature time series as NumPy arrays for downstream ML training.

Key capabilities:

- **TRF trade classification**: Midpoint signing with exclusion band (Barber et al. 2024), BJZZ retail identification via fractional cent analysis (Boehmer et al. 2021), BVC probabilistic volume classification (Easley et al. 2012)
- **Time-binned feature extraction**: Configurable bin sizes (5s to 600s), 34 features across 10 groups with per-group toggles, 3-level empty bin policy (zero/forward-fill/conditional)
- **Point-return labels**: Multi-horizon basis-point returns with forward mid-price trajectories for downstream label recomputation
- **NPY export**: Sequences `[N,T,F]` float32, labels `[N,H]` float64, forward prices `[N,max_H+1]` float64, per-day metadata and normalization statistics

## Prerequisites

- **Rust** 1.82+ (see `rust-version` in Cargo.toml)
- **Databento XNAS.BASIC CMBP-1** data files (`.dbn.zst` format)
- **EQUS_SUMMARY** OHLCV-1D data (optional, for consolidated daily volume context)

## Quick Start

```bash
# Build
cargo build --release

# Run all tests (365 lib + 47 integration)
cargo test

# Lint
cargo clippy --all-targets

# Export multi-day dataset with train/val/test splits
export_dataset --config configs/nvda_60s.toml

# Validate coverage against EQUS_SUMMARY
validate_coverage --config configs/nvda_60s.toml

# Profile a single day's data
profile_data --config configs/nvda_60s.toml --date 2025-02-03
```

## Architecture

```
.dbn.zst (CMBP-1) --> reader/ --> CmbpRecord
                                      |
                         bbo_state/ <--|--> BboState (L1 BBO tracking)
                                      |
                    trade_classifier/ <--> ClassifiedTrade
                         |                 (midpoint signing + BJZZ retail + BVC)
                         v
                    accumulator/ --> BinAccumulator (per-bin state)
                         |           6 sub-accumulators:
                         |           flow, count, stats, burst, forward_fill, BVC
                         v
                    features/ --> FeatureExtractor --> Vec<f64> (34 features)
                         |
                         v
                    sequence_builder/ --> FeatureVec = Arc<Vec<f64>>
                    labeling/ --> LabelComputer (point-return bps)
                    export/ --> DayExporter (NPY + metadata + normalization)
                         |
                         v
                    pipeline.rs --> DayPipeline (init -> stream -> finalize)
                    context.rs --> DailyContextLoader (EQUS_SUMMARY)
                    dates.rs --> weekday enumeration, split assignment
```

13 modules, 41 Rust source files (35 inside the 13 modules + `lib.rs`/`error.rs`/`contract.rs`/`pipeline.rs`/`context.rs`/`dates.rs` at `src/` root, plus 3 binaries in `src/bin/`), 5 integration test files, **412 tests** (365 lib + 47 integration).

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

Categorical (non-normalizable): [29, 30, 32, 33]. See `docs/design/04_FEATURE_SPECIFICATION.md` for all 34 formulas with exact mathematics, sign conventions, and IC values.

## Output Format

| File | Shape | Dtype | Unit |
|------|-------|-------|------|
| `{day}_sequences.npy` | [N, 20, 34] | float32 | raw or normalized |
| `{day}_labels.npy` | [N, 8] | float64 | basis points |
| `{day}_forward_prices.npy` | [N, 61] | float64 | USD |
| `{day}_metadata.json` | -- | JSON | all spec fields |
| `{day}_normalization.json` | -- | JSON | per-feature stats |

## Configuration

Configuration is TOML-based with 11 sections: `[input]`, `[sampling]`, `[classification]`, `[features]`, `[vpin]`, `[validation]`, `[sequence]`, `[labeling]`, `[dates]`, `[export]`, `[export.split_dates]`. See `configs/nvda_60s.toml` for a production example.

Full config reference: `docs/design/05_CONFIGURATION_SCHEMA.md` (1,039 lines covering every parameter, valid ranges, defaults, and validation rules).

## Data Requirements

### XNAS.BASIC CMBP-1

Databento consolidated Best Bid and Offer data with trades. Files are `.dbn.zst` (Zstandard-compressed DBN format). Expected naming: `xnas-basic-{YYYYMMDD}.cmbp-1.dbn.zst`.

Each file contains:
- **Quotes**: L1 BBO updates from all Nasdaq-listed venues (XNAS, XBOS, XPSX) and TRF
- **Trades**: Trade prints including TRF (off-exchange) executions with publisher IDs

### EQUS_SUMMARY (Optional)

OHLCV-1D consolidated daily summary. Used for coverage validation and context features (total consolidated volume). The pipeline operates correctly without it.

## Documentation

| Document | Description |
|----------|-------------|
| `CODEBASE.md` | Operational technical reference: modules, types, build, tests |
| `docs/design/01_THEORETICAL_FOUNDATION.md` | 47 papers, trade classification theory, BJZZ, BVC |
| `docs/design/02_MODULE_ARCHITECTURE.md` | 13 modules, 8 design decisions, accumulator pattern |
| `docs/design/03_DATA_FLOW.md` | End-to-end data flow, BBO ordering, price precision chain |
| `docs/design/04_FEATURE_SPECIFICATION.md` | All 34 features with exact formulas and IC values |
| `docs/design/05_CONFIGURATION_SCHEMA.md` | Complete TOML config reference |
| `docs/design/06_INTEGRATION_POINTS.md` | MBO fusion, EQUS integration, export contract |
| `docs/design/07_TESTING_STRATEGY.md` | 6-phase implementation plan, decision gates |

## Dependencies

| Crate | Source | Purpose |
|-------|--------|---------|
| [hft-statistics](https://github.com/nagarx/hft-statistics) | git (branch=main) | Welford, VPIN, time regime, DST offset, phi() |
| [dbn](https://github.com/databento/dbn) | git (tag=v0.20.0) | CbboMsg, OhlcvMsg decode |
| ndarray + ndarray-npy | crates.io (0.15 / 0.8) | NPY array construction and writing |
| clap | crates.io (4) | CLI argument parsing |
| chrono | crates.io (0.4) | Date handling |
| serde + serde_json + toml | crates.io (1.0) | Config and metadata serialization |
| thiserror | crates.io (1.0) | Error type derivation |
| log + env_logger | crates.io (0.4 / 0.11) | Logging |
| tempfile (dev) | crates.io (3.8) | Test temporary directories |

## Testing

412 tests total: 365 library unit tests + 47 integration tests.

Integration tests require Databento XNAS.BASIC CMBP-1 data files. All integration tests are gated by `data_available()` and skip gracefully when data is not present — `cargo test` will always succeed on a fresh clone, but integration tests will report as passed (skipped) rather than ignored.

Golden tests for midpoint signing (10 vectors) and BJZZ classification (10 vectors) run without external data.

## License

Proprietary. See [LICENSE](LICENSE).
