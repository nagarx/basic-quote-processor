//! NPY file writing for off-exchange feature exports.
//!
//! Writes sequences (f32), labels (f64), and forward prices (f64) using
//! ndarray + ndarray-npy. Applies optional normalization during the f64→f32
//! downcast for sequences.
//!
//! All values are pre-scanned with `is_finite()` before writing to prevent
//! NaN/Inf reaching disk. This is stricter than the MBO pipeline's export
//! which does not pre-scan.
//!
//! Source: docs/design/03_DATA_FLOW.md §4 (Stage 4)

use std::fs::File;
use std::path::Path;

use ndarray::Array2;
use ndarray_npy::WriteNpyExt;

use crate::error::{ProcessorError, Result};
use crate::sequence_builder::FeatureVec;

use super::normalization::NormalizationComputer;

/// Write sequences as `[N, T, F]` float32 NPY.
///
/// Applies optional z-score normalization inline during the f64→f32 downcast.
/// Pre-scans ALL values with `is_finite()` before writing.
///
/// # Arguments
///
/// * `path` — Output .npy file path
/// * `sequences` — `[N][T]` of Arc<Vec<f64>> (each Vec has F elements)
/// * `normalizer` — If `Some`, applies z-score normalization per feature
/// * `n_features` — Expected feature count per bin (for shape validation)
///
/// # Errors
///
/// Returns `ProcessorError::Export` if any value is non-finite or file write fails.
pub fn write_sequences(
    path: &Path,
    sequences: &[Vec<FeatureVec>],
    normalizer: Option<&NormalizationComputer>,
    n_features: usize,
) -> Result<()> {
    if sequences.is_empty() {
        return Err(ProcessorError::export("Cannot write 0 sequences"));
    }
    let n_seq = sequences.len();
    let window_size = sequences[0].len();

    // Flatten to f32 with optional normalization + is_finite guard
    let total_elements = n_seq * window_size * n_features;
    let mut flat: Vec<f32> = Vec::with_capacity(total_elements);

    for (seq_idx, seq) in sequences.iter().enumerate() {
        if seq.len() != window_size {
            return Err(ProcessorError::export(format!(
                "Sequence {} has {} bins, expected {}",
                seq_idx, seq.len(), window_size
            )));
        }
        for (bin_idx, fv) in seq.iter().enumerate() {
            if fv.len() != n_features {
                return Err(ProcessorError::export(format!(
                    "Sequence {} bin {} has {} features, expected {}",
                    seq_idx, bin_idx, fv.len(), n_features
                )));
            }
            for (feat_idx, &val) in fv.iter().enumerate() {
                let normalized = match normalizer {
                    Some(nc) => nc.normalize_value(feat_idx, val),
                    None => val,
                };
                if !normalized.is_finite() {
                    return Err(ProcessorError::export(format!(
                        "Non-finite value at seq={}, bin={}, feat={}: raw={}, normalized={}",
                        seq_idx, bin_idx, feat_idx, val, normalized
                    )));
                }
                flat.push(normalized as f32);
            }
        }
    }

    // ndarray-npy doesn't support 3D directly; write as 2D [N * T, F] is not correct.
    // Use raw buffer with shape header via ndarray's shape_vec.
    // Actually ndarray Array3 is not in ndarray-npy 0.8. Use Array2 with manual reshaping:
    // Write as [N, T * F] then the Python side knows to reshape.
    // NO - we need proper [N, T, F] shape. Let me check ndarray-npy capabilities.
    //
    // ndarray-npy 0.8 supports ArrayD (dynamic). Let's use that.
    let shape = vec![n_seq, window_size, n_features];
    let array = ndarray::ArrayD::<f32>::from_shape_vec(
        ndarray::IxDyn(&shape),
        flat,
    ).map_err(|e| ProcessorError::export(format!("ndarray shape error: {e}")))?;

    let mut file = File::create(path)
        .map_err(|e| ProcessorError::export(format!("Failed to create {}: {e}", path.display())))?;
    array
        .write_npy(&mut file)
        .map_err(|e| ProcessorError::export(format!("Failed to write NPY: {e}")))?;

    Ok(())
}

/// Write labels as `[N, H]` float64 NPY.
///
/// All values must be finite (NaN-bearing rows already excluded by valid_mask).
///
/// # Arguments
///
/// * `path` — Output .npy file path
/// * `labels` — `[N][H]` float64 values in basis points
/// * `n_horizons` — Expected number of horizons per row
pub fn write_labels(path: &Path, labels: &[Vec<f64>], n_horizons: usize) -> Result<()> {
    if labels.is_empty() {
        return Err(ProcessorError::export("Cannot write 0 labels"));
    }
    let n = labels.len();

    let mut flat: Vec<f64> = Vec::with_capacity(n * n_horizons);
    for (i, row) in labels.iter().enumerate() {
        if row.len() != n_horizons {
            return Err(ProcessorError::export(format!(
                "Label row {} has {} horizons, expected {}",
                i, row.len(), n_horizons
            )));
        }
        for (j, &val) in row.iter().enumerate() {
            if !val.is_finite() {
                return Err(ProcessorError::export(format!(
                    "Non-finite label at row={}, horizon={}: {}",
                    i, j, val
                )));
            }
            flat.push(val);
        }
    }

    let array = Array2::<f64>::from_shape_vec((n, n_horizons), flat)
        .map_err(|e| ProcessorError::export(format!("ndarray shape error: {e}")))?;

    let mut file = File::create(path)
        .map_err(|e| ProcessorError::export(format!("Failed to create {}: {e}", path.display())))?;
    array
        .write_npy(&mut file)
        .map_err(|e| ProcessorError::export(format!("Failed to write NPY: {e}")))?;

    Ok(())
}

/// Write forward prices as `[N, max_H+1]` float64 NPY.
///
/// Forward prices may contain NaN for bins near end-of-day where
/// the full forward trajectory is not available. NaN in forward_prices
/// is acceptable (unlike labels, which must be fully finite).
///
/// # Arguments
///
/// * `path` — Output .npy file path
/// * `forward_prices` — `[N][max_H+1]` float64 values in USD
/// * `n_columns` — Expected number of columns (max_horizon + 1)
pub fn write_forward_prices(
    path: &Path,
    forward_prices: &[Vec<f64>],
    n_columns: usize,
) -> Result<()> {
    if forward_prices.is_empty() {
        return Err(ProcessorError::export("Cannot write 0 forward prices"));
    }
    let n = forward_prices.len();

    let mut flat: Vec<f64> = Vec::with_capacity(n * n_columns);
    for (i, row) in forward_prices.iter().enumerate() {
        if row.len() != n_columns {
            return Err(ProcessorError::export(format!(
                "Forward price row {} has {} columns, expected {}",
                i, row.len(), n_columns
            )));
        }
        flat.extend_from_slice(row);
    }

    let array = Array2::<f64>::from_shape_vec((n, n_columns), flat)
        .map_err(|e| ProcessorError::export(format!("ndarray shape error: {e}")))?;

    let mut file = File::create(path)
        .map_err(|e| ProcessorError::export(format!("Failed to create {}: {e}", path.display())))?;
    array
        .write_npy(&mut file)
        .map_err(|e| ProcessorError::export(format!("Failed to write NPY: {e}")))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use crate::contract::TOTAL_FEATURES;

    fn make_fv(val: f64) -> FeatureVec {
        Arc::new(vec![val; TOTAL_FEATURES])
    }

    #[test]
    fn test_write_sequences_shape() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_seq.npy");

        let sequences = vec![
            vec![make_fv(1.0), make_fv(2.0), make_fv(3.0)],  // window=3
            vec![make_fv(4.0), make_fv(5.0), make_fv(6.0)],
        ];
        write_sequences(&path, &sequences, None, TOTAL_FEATURES).unwrap();

        // Read back and verify shape
        let file = File::open(&path).unwrap();
        let array: ndarray::ArrayD<f32> = ndarray_npy::ReadNpyExt::read_npy(file).unwrap();
        assert_eq!(array.shape(), &[2, 3, TOTAL_FEATURES]);
    }

    #[test]
    fn test_write_sequences_f32_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_seq.npy");

        let sequences = vec![
            vec![make_fv(1.5)],
        ];
        write_sequences(&path, &sequences, None, TOTAL_FEATURES).unwrap();

        let file = File::open(&path).unwrap();
        let array: ndarray::ArrayD<f32> = ndarray_npy::ReadNpyExt::read_npy(file).unwrap();
        assert!((array[[0, 0, 0]] - 1.5_f32).abs() < 1e-6);
    }

    #[test]
    fn test_write_sequences_non_finite_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_nan.npy");

        let mut bad_fv = vec![1.0; TOTAL_FEATURES];
        bad_fv[5] = f64::NAN;
        let sequences = vec![vec![Arc::new(bad_fv)]];

        let err = write_sequences(&path, &sequences, None, TOTAL_FEATURES);
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("Non-finite"));
    }

    #[test]
    fn test_write_sequences_normalized() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_norm.npy");

        let config = crate::config::FeatureConfig::default();
        let mut nc = NormalizationComputer::new(TOTAL_FEATURES, &config);
        // Feed some data to establish mean/std
        nc.update(&vec![0.0; TOTAL_FEATURES]);
        nc.update(&vec![10.0; TOTAL_FEATURES]);
        // mean=5, std=5 for normalizable features

        let sequences = vec![vec![make_fv(10.0)]];
        write_sequences(&path, &sequences, Some(&nc), TOTAL_FEATURES).unwrap();

        let file = File::open(&path).unwrap();
        let array: ndarray::ArrayD<f32> = ndarray_npy::ReadNpyExt::read_npy(file).unwrap();
        // z-score of 10.0: (10 - 5) / 5 = 1.0 for normalizable features
        assert!((array[[0, 0, 0]] - 1.0_f32).abs() < 0.01, "Normalized value should be ~1.0");
    }

    #[test]
    fn test_write_sequences_raw() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_raw.npy");

        let sequences = vec![vec![make_fv(42.0)]];
        write_sequences(&path, &sequences, None, TOTAL_FEATURES).unwrap();

        let file = File::open(&path).unwrap();
        let array: ndarray::ArrayD<f32> = ndarray_npy::ReadNpyExt::read_npy(file).unwrap();
        assert!((array[[0, 0, 0]] - 42.0_f32).abs() < 0.01, "Raw value should pass through");
    }

    #[test]
    fn test_write_labels_shape_and_dtype() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_labels.npy");

        let labels = vec![
            vec![10.5, -5.2, 0.0],
            vec![20.1, 15.3, -8.7],
        ];
        write_labels(&path, &labels, 3).unwrap();

        let file = File::open(&path).unwrap();
        let array: ndarray::Array2<f64> = ndarray_npy::ReadNpyExt::read_npy(file).unwrap();
        assert_eq!(array.shape(), &[2, 3]);
        assert!((array[[0, 0]] - 10.5).abs() < 1e-10);
        assert!((array[[1, 2]] - (-8.7)).abs() < 1e-10);
    }

    #[test]
    fn test_write_forward_prices_shape_and_dtype() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_fwd.npy");

        let fwd = vec![
            vec![130.0, 130.01, 130.02],
            vec![130.01, 130.02, f64::NAN], // NaN OK in forward prices
        ];
        write_forward_prices(&path, &fwd, 3).unwrap();

        let file = File::open(&path).unwrap();
        let array: ndarray::Array2<f64> = ndarray_npy::ReadNpyExt::read_npy(file).unwrap();
        assert_eq!(array.shape(), &[2, 3]);
        assert!((array[[0, 0]] - 130.0).abs() < 1e-10);
        assert!(array[[1, 2]].is_nan()); // NaN preserved
    }

    #[test]
    fn test_write_zero_sequences_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_empty.npy");

        let sequences: Vec<Vec<FeatureVec>> = vec![];
        assert!(write_sequences(&path, &sequences, None, TOTAL_FEATURES).is_err());
    }
}
