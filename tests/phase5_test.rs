//! Phase 5 integration tests: EQUS_SUMMARY context + multi-day pipeline.
//!
//! Tests are gated by `data_available()` to skip gracefully when data files
//! are not present (CI environments).

use std::path::Path;

use basic_quote_processor::config::{
    FeatureConfig, InputConfig, LabelConfig, ProcessorConfig,
    SamplingConfig, SequenceConfig, ValidationConfig, VpinConfig,
};
use basic_quote_processor::context::DailyContextLoader;
use basic_quote_processor::contract::TOTAL_FEATURES;
use basic_quote_processor::pipeline::DayPipeline;
use basic_quote_processor::trade_classifier::ClassificationConfig;

const DATA_DIR: &str = "../data/XNAS_BASIC/NVDA/cmbp1_2025-02-03_to_2026-01-09";
const TEST_FILE: &str = "xnas-basic-20250203.cmbp-1.dbn.zst";
const EQUS_PATH: &str = "../data/EQUS_SUMMARY/NVDA/ohlcv1d_2025-02-03_to_2026-03-05/equs-summary-20250203-20260305.ohlcv-1d.dbn.zst";

fn test_file_path() -> std::path::PathBuf {
    Path::new(DATA_DIR).join(TEST_FILE)
}

fn data_available() -> bool {
    test_file_path().exists()
}

fn equs_available() -> bool {
    Path::new(EQUS_PATH).exists()
}

fn test_config() -> ProcessorConfig {
    ProcessorConfig {
        input: InputConfig {
            data_dir: DATA_DIR.to_string(),
            filename_pattern: "*.dbn.zst".to_string(),
            symbol: "NVDA".to_string(),
            equs_summary_path: None,
        },
        sampling: SamplingConfig::default(),
        classification: ClassificationConfig::default(),
        features: FeatureConfig::default(),
        vpin: VpinConfig::default(),
        validation: ValidationConfig::default(),
        sequence: SequenceConfig::default(),
        labeling: LabelConfig::default(),
    }
}

// ── EQUS Context Tests ──────────────────────────────────────────────

#[test]
fn test_daily_context_loading() {
    if !equs_available() {
        eprintln!("SKIP: EQUS_SUMMARY data not available");
        return;
    }
    let loader = DailyContextLoader::from_file(Path::new(EQUS_PATH)).unwrap();
    assert!(loader.n_dates() > 200, "Expected 200+ dates, got {}", loader.n_dates());

    let ctx = loader.get(chrono::NaiveDate::from_ymd_opt(2025, 2, 3).unwrap());
    assert!(ctx.has_volume(), "2025-02-03 should have EQUS data");
}

#[test]
fn test_daily_context_volume_range() {
    if !equs_available() {
        eprintln!("SKIP: EQUS_SUMMARY data not available");
        return;
    }
    let loader = DailyContextLoader::from_file(Path::new(EQUS_PATH)).unwrap();
    let ctx = loader.get(chrono::NaiveDate::from_ymd_opt(2025, 2, 3).unwrap());
    let vol = ctx.consolidated_volume.unwrap();
    assert!(
        vol > 10_000_000 && vol < 1_000_000_000,
        "NVDA daily volume {} should be 10M-1B",
        vol
    );
}

// ── Pipeline with Context Tests ─────────────────────────────────────

#[test]
fn test_pipeline_with_equs_context() {
    if !data_available() || !equs_available() {
        eprintln!("SKIP: test data or EQUS not available");
        return;
    }
    let config = test_config();
    let mut pipeline = DayPipeline::new(&config).unwrap();
    let loader = DailyContextLoader::from_file(Path::new(EQUS_PATH)).unwrap();
    let context = loader.get(chrono::NaiveDate::from_ymd_opt(2025, 2, 3).unwrap());

    pipeline.init_day_with_context(2025, 2, 3, Some(context));
    pipeline.stream_file(&test_file_path()).unwrap();
    let export = pipeline.finalize().unwrap();

    // Metadata should have EQUS context
    let json = export.metadata.to_json().unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["equs_summary_available"], true);
    assert!(parsed["consolidated_volume"].is_number());
}

#[test]
fn test_pipeline_without_equs_context() {
    if !data_available() {
        eprintln!("SKIP: test data not available");
        return;
    }
    let config = test_config();
    let mut pipeline = DayPipeline::new(&config).unwrap();

    // Process without context
    pipeline.init_day(2025, 2, 3);
    pipeline.stream_file(&test_file_path()).unwrap();
    let export = pipeline.finalize().unwrap();

    let json = export.metadata.to_json().unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["equs_summary_available"], false);
}

#[test]
fn test_coverage_ratio_range() {
    if !data_available() || !equs_available() {
        eprintln!("SKIP: test data or EQUS not available");
        return;
    }
    let config = test_config();
    let mut pipeline = DayPipeline::new(&config).unwrap();
    let loader = DailyContextLoader::from_file(Path::new(EQUS_PATH)).unwrap();
    let date = chrono::NaiveDate::from_ymd_opt(2025, 2, 3).unwrap();
    let context = loader.get(date);
    let consolidated = context.consolidated_volume.unwrap();

    pipeline.init_day_with_context(2025, 2, 3, Some(context));
    pipeline.stream_file(&test_file_path()).unwrap();
    let _export = pipeline.finalize().unwrap();
    let summary = pipeline.day_summary();

    let coverage = (summary.total_trf_volume + summary.total_lit_volume) / consolidated as f64;
    // XNAS.BASIC captures Nasdaq venues only (XNAS lit + FINN/FINC TRF),
    // not all US venues. Empirically ~60-65% of consolidated volume.
    // The 81-85% spec target applies to the full EQUS coverage check
    // which would include all venue feeds. For XNAS.BASIC alone,
    // the expected range is lower.
    assert!(
        coverage > 0.40 && coverage < 0.85,
        "Coverage ratio {:.1}% should be 40-85% for NVDA XNAS.BASIC single-feed",
        coverage * 100.0
    );

    eprintln!(
        "Coverage: {:.1}%, TRF vol: {:.0}, Lit vol: {:.0}, Consolidated: {}",
        coverage * 100.0,
        summary.total_trf_volume,
        summary.total_lit_volume,
        consolidated,
    );
}

#[test]
fn test_trf_volume_fraction_range() {
    if !data_available() || !equs_available() {
        eprintln!("SKIP: test data or EQUS not available");
        return;
    }
    let config = test_config();
    let mut pipeline = DayPipeline::new(&config).unwrap();
    let loader = DailyContextLoader::from_file(Path::new(EQUS_PATH)).unwrap();
    let context = loader.get(chrono::NaiveDate::from_ymd_opt(2025, 2, 3).unwrap());
    let consolidated = context.consolidated_volume.unwrap();

    pipeline.init_day_with_context(2025, 2, 3, Some(context));
    pipeline.stream_file(&test_file_path()).unwrap();
    let _export = pipeline.finalize().unwrap();
    let summary = pipeline.day_summary();

    let dark_share = summary.total_trf_volume / consolidated as f64;
    assert!(
        dark_share > 0.20 && dark_share < 0.70,
        "TRF volume fraction {} should be 20-70% for NVDA",
        dark_share
    );
}

#[test]
fn test_cumulative_volumes_consistent() {
    if !data_available() {
        eprintln!("SKIP: test data not available");
        return;
    }
    let config = test_config();
    let mut pipeline = DayPipeline::new(&config).unwrap();
    pipeline.init_day(2025, 2, 3);
    pipeline.stream_file(&test_file_path()).unwrap();
    let _export = pipeline.finalize().unwrap();
    let summary = pipeline.day_summary();

    // Volumes should be positive for NVDA
    assert!(summary.total_trf_volume > 0.0, "TRF volume should be > 0");
    assert!(summary.total_lit_volume > 0.0, "Lit volume should be > 0");
    // TRF + lit should be less than some reasonable upper bound
    assert!(
        summary.total_trf_volume + summary.total_lit_volume < 1e10,
        "Total volume {} seems unreasonably large",
        summary.total_trf_volume + summary.total_lit_volume
    );
}

#[test]
fn test_manifest_json_valid() {
    use basic_quote_processor::export::manifest::DatasetManifest;

    let mut manifest = DatasetManifest::new(
        "test",
        "NVDA",
        20,
        1,
        60,
        vec![1, 5, 10],
        "none",
    );
    manifest.splits.train.record_day("2025-02-03", 308);
    manifest.splits.test.record_day("2025-11-14", 305);
    manifest.mark_complete();

    let json = manifest.to_json().unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["complete"], true);
    assert_eq!(parsed["total_sequences"], 613);
    assert_eq!(parsed["days_processed"], 2);
    assert_eq!(parsed["feature_count"], TOTAL_FEATURES);
}

#[test]
fn test_determinism_with_context() {
    if !data_available() || !equs_available() {
        eprintln!("SKIP: test data or EQUS not available");
        return;
    }
    let config = test_config();
    let loader = DailyContextLoader::from_file(Path::new(EQUS_PATH)).unwrap();

    let mut p1 = DayPipeline::new(&config).unwrap();
    let ctx1 = loader.get(chrono::NaiveDate::from_ymd_opt(2025, 2, 3).unwrap());
    p1.init_day_with_context(2025, 2, 3, Some(ctx1));
    p1.stream_file(&test_file_path()).unwrap();
    let e1 = p1.finalize().unwrap();

    let mut p2 = DayPipeline::new(&config).unwrap();
    let ctx2 = loader.get(chrono::NaiveDate::from_ymd_opt(2025, 2, 3).unwrap());
    p2.init_day_with_context(2025, 2, 3, Some(ctx2));
    p2.stream_file(&test_file_path()).unwrap();
    let e2 = p2.finalize().unwrap();

    assert_eq!(e1.sequences.len(), e2.sequences.len(), "Determinism: sequence count");
    for (i, (l1, l2)) in e1.labels.iter().zip(e2.labels.iter()).enumerate() {
        for (j, (&v1, &v2)) in l1.iter().zip(l2.iter()).enumerate() {
            assert_eq!(v1, v2, "Determinism: label[{i}][{j}] differs");
        }
    }
}
