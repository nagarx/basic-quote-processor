//! Phase 4 integration tests: pipeline + export on real NVIDIA data.
//!
//! Tests the complete lifecycle: DayPipeline (init → stream → finalize)
//! then DayExporter writes NPY + metadata + normalization JSON.
//!
//! Tests are gated by `data_available()` to skip gracefully when .dbn.zst
//! files are not present (CI environments).

use std::path::Path;

use basic_quote_processor::config::{
    FeatureConfig, InputConfig, LabelConfig,
    ProcessorConfig, SamplingConfig, SequenceConfig, ValidationConfig, VpinConfig,
};
use basic_quote_processor::contract::TOTAL_FEATURES;
use basic_quote_processor::export::DayExporter;
use basic_quote_processor::pipeline::DayPipeline;
use basic_quote_processor::trade_classifier::ClassificationConfig;

const DATA_DIR: &str = "../data/XNAS_BASIC/NVDA/cmbp1_2025-02-03_to_2026-01-09";
const TEST_FILE: &str = "xnas-basic-20250203.cmbp-1.dbn.zst";

fn test_file_path() -> std::path::PathBuf {
    Path::new(DATA_DIR).join(TEST_FILE)
}

fn data_available() -> bool {
    test_file_path().exists()
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

// ── Integration Tests ──────────────────────────────────────────────────

#[test]
fn test_full_day_pipeline() {
    if !data_available() {
        eprintln!("SKIP: test data not available");
        return;
    }

    let config = test_config();
    let mut pipeline = DayPipeline::new(&config).unwrap();
    let export = pipeline.process_day(&test_file_path(), 2025, 2, 3).unwrap();

    // With default config: ~387 bins, max_H=60, window=20, stride=1
    // Expected sequences: ~308 (387 - 60 - 20 + 1 = 308)
    assert!(
        export.sequences.len() > 250 && export.sequences.len() < 350,
        "Expected ~308 sequences, got {}",
        export.sequences.len()
    );

    assert_eq!(export.labels.len(), export.sequences.len(),
        "Labels count must match sequences");
    assert_eq!(export.forward_prices.len(), export.sequences.len(),
        "Forward prices count must match sequences");

    eprintln!(
        "PASS: {} sequences, {} labels, {} forward_prices",
        export.sequences.len(),
        export.labels.len(),
        export.forward_prices.len(),
    );
}

#[test]
fn test_full_day_export() {
    if !data_available() {
        eprintln!("SKIP: test data not available");
        return;
    }

    let config = test_config();
    let mut pipeline = DayPipeline::new(&config).unwrap();
    let export = pipeline.process_day(&test_file_path(), 2025, 2, 3).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let exporter = DayExporter::from_dir(dir.path(), false).unwrap();
    let n = exporter.export_day(&export).unwrap();
    assert!(n > 0, "Should have exported sequences");

    // Verify all 5 files exist
    let files = [
        "2025-02-03_sequences.npy",
        "2025-02-03_labels.npy",
        "2025-02-03_forward_prices.npy",
        "2025-02-03_metadata.json",
        "2025-02-03_normalization.json",
    ];
    for fname in &files {
        assert!(
            dir.path().join(fname).exists(),
            "Expected file {} not found in {}",
            fname,
            dir.path().display()
        );
    }

    eprintln!("PASS: {} sequences exported to {}", n, dir.path().display());
}

#[test]
fn test_output_shapes() {
    if !data_available() {
        eprintln!("SKIP: test data not available");
        return;
    }

    let config = test_config();
    let mut pipeline = DayPipeline::new(&config).unwrap();
    let export = pipeline.process_day(&test_file_path(), 2025, 2, 3).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let exporter = DayExporter::from_dir(dir.path(), false).unwrap();
    exporter.export_day(&export).unwrap();

    // Read sequences NPY and verify shape
    let seq_file = std::fs::File::open(dir.path().join("2025-02-03_sequences.npy")).unwrap();
    let seq_array: ndarray::ArrayD<f32> = ndarray_npy::ReadNpyExt::read_npy(seq_file).unwrap();
    assert_eq!(seq_array.ndim(), 3);
    assert_eq!(seq_array.shape()[1], 20, "Window size should be 20");
    assert_eq!(seq_array.shape()[2], TOTAL_FEATURES, "Features should be {}", TOTAL_FEATURES);

    // Read labels NPY and verify shape
    let lab_file = std::fs::File::open(dir.path().join("2025-02-03_labels.npy")).unwrap();
    let lab_array: ndarray::Array2<f64> = ndarray_npy::ReadNpyExt::read_npy(lab_file).unwrap();
    assert_eq!(lab_array.shape()[0], seq_array.shape()[0], "N_labels must match N_sequences");
    assert_eq!(lab_array.shape()[1], 8, "8 default horizons");

    // Read forward prices NPY
    let fwd_file = std::fs::File::open(dir.path().join("2025-02-03_forward_prices.npy")).unwrap();
    let fwd_array: ndarray::Array2<f64> = ndarray_npy::ReadNpyExt::read_npy(fwd_file).unwrap();
    assert_eq!(fwd_array.shape()[0], seq_array.shape()[0], "N_fwd must match N_sequences");
    assert_eq!(fwd_array.shape()[1], 61, "max_H=60 → 61 columns");

    eprintln!(
        "PASS: shapes seq={:?}, lab={:?}, fwd={:?}",
        seq_array.shape(), lab_array.shape(), fwd_array.shape()
    );
}

#[test]
fn test_sequences_all_finite() {
    if !data_available() {
        eprintln!("SKIP: test data not available");
        return;
    }

    let config = test_config();
    let mut pipeline = DayPipeline::new(&config).unwrap();
    let export = pipeline.process_day(&test_file_path(), 2025, 2, 3).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let exporter = DayExporter::from_dir(dir.path(), false).unwrap();
    exporter.export_day(&export).unwrap();

    let file = std::fs::File::open(dir.path().join("2025-02-03_sequences.npy")).unwrap();
    let array: ndarray::ArrayD<f32> = ndarray_npy::ReadNpyExt::read_npy(file).unwrap();

    for &val in array.iter() {
        assert!(val.is_finite(), "Non-finite value in sequences: {}", val);
    }
}

#[test]
fn test_labels_all_finite() {
    if !data_available() {
        eprintln!("SKIP: test data not available");
        return;
    }

    let config = test_config();
    let mut pipeline = DayPipeline::new(&config).unwrap();
    let export = pipeline.process_day(&test_file_path(), 2025, 2, 3).unwrap();

    // All exported labels should be finite (NaN rows were filtered by valid_mask)
    for (i, row) in export.labels.iter().enumerate() {
        for (j, &val) in row.iter().enumerate() {
            assert!(
                val.is_finite(),
                "Non-finite label at [{i}][{j}]: {val}"
            );
        }
    }
}

#[test]
fn test_labels_spot_check() {
    if !data_available() {
        eprintln!("SKIP: test data not available");
        return;
    }

    let config = test_config();
    let mut pipeline = DayPipeline::new(&config).unwrap();
    let export = pipeline.process_day(&test_file_path(), 2025, 2, 3).unwrap();

    // Verify that labels are in reasonable range for NVDA (typically ±50 bps)
    for (i, row) in export.labels.iter().take(5).enumerate() {
        for (j, &val) in row.iter().enumerate() {
            assert!(
                val.abs() < 500.0,
                "Label[{i}][{j}] = {val} bps — suspiciously large for NVDA"
            );
        }
    }
}

#[test]
fn test_forward_prices_spot_check() {
    if !data_available() {
        eprintln!("SKIP: test data not available");
        return;
    }

    let config = test_config();
    let mut pipeline = DayPipeline::new(&config).unwrap();
    let export = pipeline.process_day(&test_file_path(), 2025, 2, 3).unwrap();

    // Column 0 should be a valid NVDA price ($100-$300 range)
    for (i, row) in export.forward_prices.iter().take(5).enumerate() {
        let base_price = row[0];
        assert!(
            base_price > 50.0 && base_price < 500.0,
            "Forward price[{i}][0] = {} — not a valid NVDA price",
            base_price
        );
    }
}

#[test]
fn test_metadata_json_complete() {
    if !data_available() {
        eprintln!("SKIP: test data not available");
        return;
    }

    let config = test_config();
    let mut pipeline = DayPipeline::new(&config).unwrap();
    let export = pipeline.process_day(&test_file_path(), 2025, 2, 3).unwrap();

    let json = export.metadata.to_json().unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    // Check key fields
    assert_eq!(parsed["day"], "2025-02-03");
    assert_eq!(parsed["label_strategy"], "point_return");
    assert_eq!(parsed["label_encoding"], "continuous_bps");
    assert_eq!(parsed["data_source"], "XNAS.BASIC");
    assert_eq!(parsed["schema"], "cmbp-1");
    assert_eq!(parsed["symbol"], "NVDA");
    assert_eq!(parsed["n_features"], TOTAL_FEATURES);
    assert_eq!(parsed["window_size"], 20);
    assert_eq!(parsed["bin_size_seconds"], 60);

    // Verify required fields exist
    for field in &["normalization", "provenance", "export_timestamp",
                   "first_bin_start_ns", "last_bin_end_ns",
                   "n_bins_total", "n_bins_valid", "n_trf_trades"] {
        assert!(
            parsed.get(field).is_some(),
            "Missing required metadata field: {}", field
        );
    }
}

#[test]
fn test_normalization_json_34_features() {
    if !data_available() {
        eprintln!("SKIP: test data not available");
        return;
    }

    let config = test_config();
    let mut pipeline = DayPipeline::new(&config).unwrap();
    let export = pipeline.process_day(&test_file_path(), 2025, 2, 3).unwrap();

    let parsed: serde_json::Value =
        serde_json::from_str(&export.normalization_json).unwrap();

    assert_eq!(parsed["strategy"], "per_day_zscore");
    let features = parsed["features"].as_array().unwrap();
    assert_eq!(features.len(), TOTAL_FEATURES, "Should have 34 feature stats");

    // Verify categorical features marked as non-normalizable
    for &idx in &[29_usize, 30, 32, 33] {
        assert_eq!(
            features[idx]["normalizable"], false,
            "Feature {} should be non-normalizable (categorical)", idx
        );
    }

    // Verify VPIN features non-normalizable (disabled by default)
    assert_eq!(
        features[18]["normalizable"], false,
        "trf_vpin (18) should be non-normalizable (VPIN disabled)"
    );
}

#[test]
fn test_determinism() {
    if !data_available() {
        eprintln!("SKIP: test data not available");
        return;
    }

    let config = test_config();

    let mut p1 = DayPipeline::new(&config).unwrap();
    let e1 = p1.process_day(&test_file_path(), 2025, 2, 3).unwrap();

    let mut p2 = DayPipeline::new(&config).unwrap();
    let e2 = p2.process_day(&test_file_path(), 2025, 2, 3).unwrap();

    assert_eq!(e1.sequences.len(), e2.sequences.len(), "Sequence count must be deterministic");
    for (i, (l1, l2)) in e1.labels.iter().zip(e2.labels.iter()).enumerate() {
        for (j, (&v1, &v2)) in l1.iter().zip(l2.iter()).enumerate() {
            assert_eq!(v1, v2, "Determinism: label[{i}][{j}] differs: {v1} vs {v2}");
        }
    }
}
