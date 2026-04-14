//! NPY export pipeline for off-exchange features.
//!
//! Provides sequence writing (f32), label writing (f64), forward price writing (f64),
//! normalization statistics (Welford per-feature), metadata JSON export, and
//! the `DayExporter` orchestrator for atomic day-level file writes.
//!
//! Source: docs/design/06_INTEGRATION_POINTS.md §5

pub mod normalization;
pub mod npy_writer;
pub mod metadata;
pub mod manifest;

pub use normalization::NormalizationComputer;
pub use metadata::ExportMetadata;
pub use manifest::DatasetManifest;

use std::path::{Path, PathBuf};

use crate::config::ExportConfig;
use crate::contract::TOTAL_FEATURES;
use crate::error::{ProcessorError, Result};
use crate::sequence_builder::FeatureVec;

/// Everything needed to export a single day's data.
///
/// Produced by `DayPipeline::finalize()`, consumed by `DayExporter::export_day()`.
pub struct DayExport {
    /// ISO date string (YYYY-MM-DD).
    pub day: String,
    /// Aligned sequences `[N][T]` of Arc<Vec<f64>>. Raw f64 (normalization at write time).
    pub sequences: Vec<Vec<FeatureVec>>,
    /// Aligned labels `[N][H]` in basis points. All values finite.
    pub labels: Vec<Vec<f64>>,
    /// Aligned forward prices `[N][max_H+1]` in USD. May contain NaN at tail.
    pub forward_prices: Vec<Vec<f64>>,
    /// Complete metadata for JSON export.
    pub metadata: ExportMetadata,
    /// Normalization computer with accumulated per-feature stats.
    pub normalizer: NormalizationComputer,
    /// Pre-serialized normalization JSON.
    pub normalization_json: String,
}

/// Writes all export files for a single day with atomic semantics.
///
/// Files are written to a temp directory first, then renamed to the
/// final location. On failure, already-renamed files are rolled back.
///
/// # Files Written
///
/// - `{day}_sequences.npy` — `[N, T, F]` float32
/// - `{day}_labels.npy` — `[N, H]` float64 (bps)
/// - `{day}_forward_prices.npy` — `[N, max_H+1]` float64 (USD)
/// - `{day}_metadata.json` — complete metadata
/// - `{day}_normalization.json` — per-feature stats
pub struct DayExporter {
    output_dir: PathBuf,
    apply_normalization: bool,
}

impl DayExporter {
    /// Create a new exporter. Creates the output directory if it doesn't exist.
    pub fn new(config: &ExportConfig) -> Result<Self> {
        std::fs::create_dir_all(&config.output_dir)
            .map_err(|e| ProcessorError::export(format!(
                "Failed to create output dir {}: {e}", config.output_dir.display()
            )))?;
        Ok(Self {
            output_dir: config.output_dir.clone(),
            apply_normalization: config.apply_normalization,
        })
    }

    /// Create from raw parameters (convenience for tests).
    pub fn from_dir(output_dir: &Path, apply_normalization: bool) -> Result<Self> {
        std::fs::create_dir_all(output_dir)
            .map_err(|e| ProcessorError::export(format!(
                "Failed to create output dir {}: {e}", output_dir.display()
            )))?;
        Ok(Self {
            output_dir: output_dir.to_path_buf(),
            apply_normalization,
        })
    }

    /// Export a complete day. Returns the number of sequences written.
    ///
    /// Returns `Ok(0)` if there are 0 sequences (day skipped with warning).
    pub fn export_day(&self, export: &DayExport) -> Result<usize> {
        let day = &export.day;
        let n = export.sequences.len();
        if n == 0 {
            log::warn!("Day {day}: 0 sequences, skipping export");
            return Ok(0);
        }

        let n_horizons = if !export.labels.is_empty() {
            export.labels[0].len()
        } else {
            return Err(ProcessorError::export("Labels are empty despite non-zero sequences"));
        };

        let n_fwd_cols = if !export.forward_prices.is_empty() {
            export.forward_prices[0].len()
        } else {
            return Err(ProcessorError::export("Forward prices are empty despite non-zero sequences"));
        };

        // FIX #11: Atomic writes via temp directory
        let tmp_dir = self.output_dir.join(format!(".tmp_{day}"));
        std::fs::create_dir_all(&tmp_dir)
            .map_err(|e| ProcessorError::export(format!("Failed to create temp dir: {e}")))?;

        let normalizer = if self.apply_normalization {
            Some(&export.normalizer)
        } else {
            None
        };

        // Write all 5 files to temp dir
        let write_result = self.write_all_files(&tmp_dir, day, export, normalizer, n_horizons, n_fwd_cols);

        if let Err(e) = write_result {
            // Clean up temp dir on failure
            let _ = std::fs::remove_dir_all(&tmp_dir);
            return Err(e);
        }

        // FIX #27: Atomic rename with rollback on failure
        let mut renamed: Vec<PathBuf> = Vec::new();
        let entries: Vec<_> = std::fs::read_dir(&tmp_dir)
            .map_err(|e| ProcessorError::export(format!("Failed to read temp dir: {e}")))?
            .filter_map(|e| e.ok())
            .collect();

        for entry in &entries {
            let dest = self.output_dir.join(entry.file_name());
            match std::fs::rename(entry.path(), &dest) {
                Ok(()) => renamed.push(dest),
                Err(e) => {
                    // Rollback: remove already-renamed files
                    for path in &renamed {
                        let _ = std::fs::remove_file(path);
                    }
                    let _ = std::fs::remove_dir_all(&tmp_dir);
                    return Err(ProcessorError::export(format!(
                        "Failed to rename {}: {e}", entry.path().display()
                    )));
                }
            }
        }
        let _ = std::fs::remove_dir(&tmp_dir);

        Ok(n)
    }

    fn write_all_files(
        &self,
        tmp_dir: &Path,
        day: &str,
        export: &DayExport,
        normalizer: Option<&NormalizationComputer>,
        n_horizons: usize,
        n_fwd_cols: usize,
    ) -> Result<()> {
        npy_writer::write_sequences(
            &tmp_dir.join(format!("{day}_sequences.npy")),
            &export.sequences,
            normalizer,
            TOTAL_FEATURES,
        )?;
        npy_writer::write_labels(
            &tmp_dir.join(format!("{day}_labels.npy")),
            &export.labels,
            n_horizons,
        )?;
        npy_writer::write_forward_prices(
            &tmp_dir.join(format!("{day}_forward_prices.npy")),
            &export.forward_prices,
            n_fwd_cols,
        )?;
        export.metadata.write_to_file(
            &tmp_dir.join(format!("{day}_metadata.json")),
        )?;
        std::fs::write(
            tmp_dir.join(format!("{day}_normalization.json")),
            &export.normalization_json,
        ).map_err(|e| ProcessorError::export(format!("Failed to write normalization JSON: {e}")))?;

        Ok(())
    }

    /// Output directory path.
    pub fn output_dir(&self) -> &Path {
        &self.output_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use crate::config::FeatureConfig;

    fn make_fv(val: f64) -> FeatureVec {
        Arc::new(vec![val; TOTAL_FEATURES])
    }

    fn make_test_export(day: &str, n_sequences: usize) -> DayExport {
        let config = FeatureConfig::default();
        let mut normalizer = NormalizationComputer::new(TOTAL_FEATURES, &config);

        let sequences: Vec<Vec<FeatureVec>> = (0..n_sequences)
            .map(|_| (0..3).map(|j| make_fv(j as f64 + 1.0)).collect())
            .collect();

        // Feed normalizer
        for seq in &sequences {
            for fv in seq {
                normalizer.update(fv);
            }
        }

        let labels: Vec<Vec<f64>> = (0..n_sequences)
            .map(|i| vec![i as f64 * 10.0, i as f64 * 5.0])
            .collect();
        let forward_prices: Vec<Vec<f64>> = (0..n_sequences)
            .map(|i| vec![130.0 + i as f64 * 0.01; 4])
            .collect();

        let metadata = ExportMetadata::builder()
            .day(day)
            .n_sequences(n_sequences)
            .window_size(3)
            .horizons(vec![1, 5])
            .bin_size_seconds(60)
            .build()
            .unwrap();

        let norm_json = normalizer.to_json(day).unwrap();

        DayExport {
            day: day.to_string(),
            sequences,
            labels,
            forward_prices,
            metadata,
            normalizer,
            normalization_json: norm_json,
        }
    }

    #[test]
    fn test_day_exporter_creates_output_dir() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("new_subdir");
        assert!(!out.exists());
        let _exporter = DayExporter::from_dir(&out, false).unwrap();
        assert!(out.exists());
    }

    #[test]
    fn test_day_exporter_writes_all_files() {
        let dir = tempfile::tempdir().unwrap();
        let exporter = DayExporter::from_dir(dir.path(), false).unwrap();
        let export = make_test_export("2025-02-03", 5);

        let n = exporter.export_day(&export).unwrap();
        assert_eq!(n, 5);

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
                "Expected file {} not found", fname
            );
        }

        // Verify temp dir is cleaned up
        assert!(
            !dir.path().join(".tmp_2025-02-03").exists(),
            "Temp dir should be removed after successful export"
        );
    }

    #[test]
    fn test_day_exporter_zero_sequences_skips() {
        let dir = tempfile::tempdir().unwrap();
        let exporter = DayExporter::from_dir(dir.path(), false).unwrap();
        let export = make_test_export("2025-02-03", 0);

        let n = exporter.export_day(&export).unwrap();
        assert_eq!(n, 0);

        // No files should be written
        assert!(
            std::fs::read_dir(dir.path()).unwrap().count() == 0,
            "No files should be written for 0 sequences"
        );
    }

    #[test]
    fn test_day_exporter_file_naming() {
        let dir = tempfile::tempdir().unwrap();
        let exporter = DayExporter::from_dir(dir.path(), false).unwrap();
        let export = make_test_export("2025-06-15", 2);
        exporter.export_day(&export).unwrap();

        // Verify naming convention
        assert!(dir.path().join("2025-06-15_sequences.npy").exists());
        assert!(dir.path().join("2025-06-15_metadata.json").exists());
    }
}
