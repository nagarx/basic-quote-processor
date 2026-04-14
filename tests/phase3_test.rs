//! Phase 3 integration tests: feature extraction pipeline on real NVIDIA data.
//!
//! Tests the complete data flow: CmbpRecord → BboState → TradeClassifier →
//! BinAccumulator → FeatureExtractor → Vec<f64> feature vectors.
//!
//! Tests are gated by `data_available()` to skip gracefully when .dbn.zst
//! files are not present (CI environments).

use std::path::Path;

use basic_quote_processor::accumulator::{BinAccumulator, DaySummary};
use basic_quote_processor::bbo_state::BboState;
use basic_quote_processor::config::{FeatureConfig, SamplingConfig, ValidationConfig, VpinConfig};
use basic_quote_processor::contract::TOTAL_FEATURES;
use basic_quote_processor::features::indices::*;
use basic_quote_processor::features::FeatureExtractor;
use basic_quote_processor::reader::DbnReader;
use basic_quote_processor::sampling::{BinBoundary, TimeBinSampler};
use basic_quote_processor::trade_classifier::TradeClassifier;

const DATA_DIR: &str = "../data/XNAS_BASIC/NVDA/cmbp1_2025-02-03_to_2026-01-09";
const TEST_FILE: &str = "xnas-basic-20250203.cmbp-1.dbn.zst";

fn test_file_path() -> std::path::PathBuf {
    Path::new(DATA_DIR).join(TEST_FILE)
}

fn data_available() -> bool {
    test_file_path().exists()
}

/// Process a full day through the complete pipeline, returning feature vectors.
fn process_day(
    file_path: &Path,
    year: i32,
    month: u32,
    day: u32,
) -> (Vec<Vec<f64>>, DaySummary) {
    let reader = DbnReader::new(file_path).unwrap();
    let (_metadata, records) = reader.open().unwrap();

    let feature_config = FeatureConfig::default();
    let validation_config = ValidationConfig::default();
    let vpin_config = VpinConfig::default();
    let sampling_config = SamplingConfig::default();

    let mut bbo = BboState::new();
    let mut classifier = TradeClassifier::with_defaults().unwrap();
    let mut sampler = TimeBinSampler::new(sampling_config.bin_size_seconds);
    let mut accumulator = BinAccumulator::new(&validation_config, &vpin_config, &feature_config);
    let mut extractor = FeatureExtractor::new(
        &feature_config,
        &validation_config,
        sampling_config.bin_size_seconds,
    );

    // Per-day initialization
    sampler.init_day(year, month, day);
    accumulator.set_bin_size_ns(sampler.bin_size_ns());
    extractor.init_day(
        sampler.utc_offset_hours(),
        sampler.market_open_ns(),
        sampler.market_close_ns(),
    );

    let warmup_bins = validation_config.warmup_bins as u64;
    let mut output_bins: Vec<Vec<f64>> = Vec::new();
    let mut feature_buffer: Vec<f64> = Vec::with_capacity(TOTAL_FEATURES);

    for record in records {
        // 1. Check bin boundary FIRST
        if let Some(boundary) = sampler.check_boundary(record.ts_recv) {
            accumulator.prepare_for_extraction(boundary.bin_end_ts);
            extractor.extract(&accumulator, &bbo, &boundary, &mut feature_buffer);

            // Forward-fill update (even during warmup per R2)
            if accumulator.trf_trades() > 0 {
                accumulator.update_forward_fill(&feature_buffer, feature_config.vpin);
            }
            if accumulator.bbo_update_count() > 0 {
                accumulator.update_forward_fill_bbo(
                    feature_buffer[SPREAD_BPS],
                    feature_buffer[QUOTE_IMBALANCE],
                );
            }

            // Warmup gate
            if accumulator.bin_index() >= warmup_bins {
                output_bins.push(feature_buffer.clone());
                let is_empty = accumulator.trf_trades() == 0;
                accumulator.record_bin_emitted_with_ts(is_empty, boundary.bin_end_ts);
            } else {
                accumulator.record_warmup_discard();
            }

            accumulator.reset_bin();

            // Emit gap bins individually (FIX #1)
            for gap_i in 0..boundary.gap_bins {
                let gap_end = boundary.bin_end_ts + (gap_i + 1) * sampler.bin_size_ns();
                let gap_boundary = BinBoundary {
                    bin_end_ts: gap_end,
                    bin_midpoint_ts: gap_end - sampler.bin_size_ns() / 2,
                    bin_index: boundary.bin_index + 1 + gap_i,
                    gap_bins: 0,
                };
                extractor.extract(&accumulator, &bbo, &gap_boundary, &mut feature_buffer);
                if accumulator.bin_index() >= warmup_bins {
                    output_bins.push(feature_buffer.clone());
                    accumulator.record_gap_bin();
                }
                accumulator.reset_bin();
            }
        }

        // 2. BBO update (always — enables pre-market warm-start)
        if bbo.update_from_record(&record) {
            if sampler.is_in_session(record.ts_recv) {
                accumulator.accumulate_bbo_update(&bbo, record.ts_recv);
            }
        }

        // 3. Trade classification + accumulation (session-gated, FIX #3)
        if record.is_trade() && sampler.is_in_session(record.ts_recv) {
            let classified = classifier.classify(&record, &bbo);
            accumulator.accumulate(&classified);
        }
    }

    // Flush last partial bin (FIX #11)
    accumulator.prepare_for_extraction(sampler.market_close_ns());
    if accumulator.has_trades() {
        let final_boundary = BinBoundary {
            bin_end_ts: sampler.market_close_ns(),
            bin_midpoint_ts: sampler.market_close_ns() - sampler.bin_size_ns() / 2,
            bin_index: accumulator.bin_index(),
            gap_bins: 0,
        };
        extractor.extract(&accumulator, &bbo, &final_boundary, &mut feature_buffer);
        if accumulator.trf_trades() > 0 {
            accumulator.update_forward_fill(&feature_buffer, feature_config.vpin);
        }
        if accumulator.bbo_update_count() > 0 {
            accumulator.update_forward_fill_bbo(
                feature_buffer[SPREAD_BPS],
                feature_buffer[QUOTE_IMBALANCE],
            );
        }
        if accumulator.bin_index() >= warmup_bins {
            output_bins.push(feature_buffer.clone());
            let is_empty = accumulator.trf_trades() == 0;
            accumulator.record_bin_emitted_with_ts(is_empty, sampler.market_close_ns());
        }
    }

    let summary = accumulator.day_summary();
    (output_bins, summary)
}

// ── Integration Tests ──────────────────────────────────────────────────

#[test]
fn test_full_day_feature_extraction() {
    if !data_available() {
        eprintln!("SKIP: test data not available");
        return;
    }

    let (bins, summary) = process_day(&test_file_path(), 2025, 2, 3);

    // Bin count: 60s bins over 6.5h ≈ 390, minus warmup(3) ≈ 387
    assert!(
        bins.len() > 350 && bins.len() < 400,
        "Expected ~387 bins, got {}",
        bins.len()
    );

    // Every feature vector has exactly 34 elements
    for (i, bin) in bins.iter().enumerate() {
        assert_eq!(
            bin.len(),
            TOTAL_FEATURES,
            "Bin {} has {} features, expected {}",
            i,
            bin.len(),
            TOTAL_FEATURES
        );
    }

    // No NaN/Inf in any feature vector
    for (i, bin) in bins.iter().enumerate() {
        for (j, &val) in bin.iter().enumerate() {
            assert!(
                val.is_finite(),
                "Bin {}, feature {}: non-finite value {}",
                i, j, val
            );
        }
    }

    // Schema version = 1.0 in every bin
    for (i, bin) in bins.iter().enumerate() {
        assert_eq!(
            bin[SCHEMA_VERSION_IDX], 1.0,
            "Bin {}: schema_version should be 1.0",
            i
        );
    }

    // Session progress monotonically increasing (within tolerance for gap bins)
    for i in 1..bins.len() {
        assert!(
            bins[i][SESSION_PROGRESS] >= bins[i - 1][SESSION_PROGRESS] - 1e-10,
            "Session progress not monotonic: bin {} = {}, bin {} = {}",
            i - 1,
            bins[i - 1][SESSION_PROGRESS],
            i,
            bins[i][SESSION_PROGRESS]
        );
    }

    // bin_valid = 1.0 for most bins (> 80%)
    let valid_count = bins.iter().filter(|b| b[BIN_VALID] == 1.0).count();
    let valid_pct = valid_count as f64 / bins.len() as f64;
    assert!(
        valid_pct > 0.80,
        "Expected > 80% valid bins, got {:.1}% ({}/{})",
        valid_pct * 100.0,
        valid_count,
        bins.len()
    );

    // Diagnostic counters
    assert!(summary.total_bins_emitted > 0);
    assert_eq!(summary.warmup_bins_discarded, 3);
    assert!(summary.total_records_processed > 10_000, "Expected many trades");

    eprintln!(
        "PASS: {} bins emitted, {} empty, {} gaps, {} warmup discarded, {} trades",
        summary.total_bins_emitted,
        summary.total_empty_bins,
        summary.gap_bins_emitted,
        summary.warmup_bins_discarded,
        summary.total_records_processed,
    );
}

#[test]
fn test_determinism() {
    if !data_available() {
        eprintln!("SKIP: test data not available");
        return;
    }

    let (bins1, _) = process_day(&test_file_path(), 2025, 2, 3);
    let (bins2, _) = process_day(&test_file_path(), 2025, 2, 3);

    assert_eq!(bins1.len(), bins2.len(), "Same day should produce same bin count");
    for (i, (b1, b2)) in bins1.iter().zip(bins2.iter()).enumerate() {
        for j in 0..TOTAL_FEATURES {
            assert_eq!(
                b1[j], b2[j],
                "Determinism: bin {} feature {} differs: {} vs {}",
                i, j, b1[j], b2[j]
            );
        }
    }
}

#[test]
fn test_cross_day_state_isolation() {
    if !data_available() {
        eprintln!("SKIP: test data not available");
        return;
    }

    let (bins1, _) = process_day(&test_file_path(), 2025, 2, 3);

    // Process the same day again (simulating a fresh start)
    let (bins2, _) = process_day(&test_file_path(), 2025, 2, 3);

    // Should produce identical results (no state leakage)
    assert_eq!(bins1.len(), bins2.len());
    for i in 0..bins1.len().min(10) {
        for j in 0..TOTAL_FEATURES {
            assert_eq!(
                bins1[i][j], bins2[i][j],
                "State isolation: bin {} feature {} differs",
                i, j
            );
        }
    }
}

#[test]
fn test_feature_ranges() {
    if !data_available() {
        eprintln!("SKIP: test data not available");
        return;
    }

    let (bins, _) = process_day(&test_file_path(), 2025, 2, 3);

    for (i, bin) in bins.iter().enumerate() {
        // Imbalance features in [-1.0, 1.0]
        for &idx in &[TRF_SIGNED_IMBALANCE, MROIB, INV_INST_DIRECTION, BVC_IMBALANCE, QUOTE_IMBALANCE] {
            assert!(
                bin[idx] >= -1.0 - 1e-10 && bin[idx] <= 1.0 + 1e-10,
                "Bin {} feature {}: imbalance {} out of [-1, 1]",
                i, idx, bin[idx]
            );
        }

        // Ratio features in [0.0, 1.0]
        for &idx in &[DARK_SHARE, SUBPENNY_INTENSITY, ODD_LOT_RATIO, RETAIL_TRADE_RATE, RETAIL_VOLUME_FRACTION] {
            assert!(
                bin[idx] >= -1e-10 && bin[idx] <= 1.0 + 1e-10,
                "Bin {} feature {}: ratio {} out of [0, 1]",
                i, idx, bin[idx]
            );
        }

        // Safety gates exactly 0 or 1
        assert!(
            bin[BIN_VALID] == 0.0 || bin[BIN_VALID] == 1.0,
            "Bin {}: bin_valid = {}",
            i, bin[BIN_VALID]
        );
        assert!(
            bin[BBO_VALID] == 0.0 || bin[BBO_VALID] == 1.0,
            "Bin {}: bbo_valid = {}",
            i, bin[BBO_VALID]
        );

        // Session progress in [0, 1]
        assert!(
            bin[SESSION_PROGRESS] >= 0.0 && bin[SESSION_PROGRESS] <= 1.0,
            "Bin {}: session_progress = {}",
            i, bin[SESSION_PROGRESS]
        );

        // Time bucket in {0, 1, 2, 3, 4, 5}
        assert!(
            (0.0..=5.0).contains(&bin[TIME_BUCKET]),
            "Bin {}: time_bucket = {}",
            i, bin[TIME_BUCKET]
        );

        // Volumes non-negative
        assert!(bin[TRF_VOLUME] >= 0.0, "Bin {}: trf_volume = {}", i, bin[TRF_VOLUME]);
        assert!(bin[LIT_VOLUME] >= 0.0, "Bin {}: lit_volume = {}", i, bin[LIT_VOLUME]);
        assert!(bin[TOTAL_VOLUME] >= 0.0, "Bin {}: total_volume = {}", i, bin[TOTAL_VOLUME]);
    }
}

#[test]
fn test_warmup_discards_first_bins() {
    if !data_available() {
        eprintln!("SKIP: test data not available");
        return;
    }

    let (bins, summary) = process_day(&test_file_path(), 2025, 2, 3);

    // Warmup = 3 bins discarded
    assert_eq!(summary.warmup_bins_discarded, 3);

    // First emitted bin should have session_progress > warmup period
    // With 60s bins and 3 warmup bins, session starts at ~3 minutes
    // session_progress = 3*60 / (6.5*3600) = 180/23400 ≈ 0.0077
    assert!(
        bins[0][SESSION_PROGRESS] > 0.005,
        "First emitted bin progress should be > warmup period: {}",
        bins[0][SESSION_PROGRESS]
    );
}

#[test]
fn test_bvc_processes_all_trades() {
    if !data_available() {
        eprintln!("SKIP: test data not available");
        return;
    }

    let (bins, _) = process_day(&test_file_path(), 2025, 2, 3);

    // BVC imbalance should be non-zero for bins with trades
    let bvc_nonzero = bins
        .iter()
        .filter(|b| b[BIN_TRADE_COUNT] > 0.0 && b[BVC_IMBALANCE].abs() > 1e-10)
        .count();
    let bins_with_trades = bins
        .iter()
        .filter(|b| b[BIN_TRADE_COUNT] > 0.0)
        .count();

    // Most bins with trades should have non-zero BVC (BVC uses all trades, not just TRF)
    let bvc_pct = bvc_nonzero as f64 / bins_with_trades.max(1) as f64;
    assert!(
        bvc_pct > 0.5,
        "Expected > 50% of bins with trades to have non-zero BVC, got {:.1}%",
        bvc_pct * 100.0
    );
}
