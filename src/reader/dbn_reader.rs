//! Databento `.dbn.zst` file reader for XNAS.BASIC CMBP-1 data.
//!
//! Reads compressed `.dbn.zst` files using the `dbn` crate and yields
//! `CmbpRecord` values via a streaming iterator. The conversion from
//! `dbn::CbboMsg` to `CmbpRecord` happens inside the iterator — all
//! downstream code works with our internal type only.
//!
//! # Usage
//!
//! ```no_run
//! use basic_quote_processor::reader::DbnReader;
//!
//! let reader = DbnReader::new("path/to/file.dbn.zst").unwrap();
//! let (_metadata, records) = reader.open().unwrap();
//! for record in records {
//!     println!("ts={} action={} pub={}", record.ts_recv, record.action as char, record.publisher_id);
//! }
//! ```
//!
//! # Pattern Source
//!
//! Adapted from `opra-statistical-profiler/src/loader.rs` (same dbn crate
//! version v0.20.0, same CbboMsg type for CMBP-1 schema).
//!
//! Source: docs/design/02_MODULE_ARCHITECTURE.md §4.1

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use dbn::decode::{DbnMetadata, DecodeRecord, DynDecoder};
use dbn::enums::VersionUpgradePolicy;
use dbn::{CbboMsg, Metadata};

use crate::error::{ProcessorError, Result};
use super::record::CmbpRecord;

/// I/O buffer size: 1 MB for optimal throughput on modern SSDs.
/// Matches the buffer size used by `opra-statistical-profiler` and
/// `MBO-LOB-reconstructor` for consistent I/O behavior.
const IO_BUFFER_SIZE: usize = 1024 * 1024;

/// Maximum consecutive decode errors before the iterator aborts.
/// Prevents infinite loops on corrupted files where the decoder
/// repeatedly returns Err without advancing past the bad bytes.
const MAX_CONSECUTIVE_ERRORS: u64 = 1000;

/// Reader for Databento XNAS.BASIC `.dbn.zst` files.
///
/// Creates a streaming iterator over `CmbpRecord` values. Each file
/// is opened independently — no state persists between files.
#[derive(Debug)]
pub struct DbnReader {
    path: PathBuf,
}

impl DbnReader {
    /// Create a new reader for the given file path.
    ///
    /// Validates that the file exists at construction time.
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            return Err(ProcessorError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("File not found: {}", path.display()),
            )));
        }
        Ok(Self { path })
    }

    /// Open the file and return metadata + streaming record iterator.
    ///
    /// The `Metadata` contains symbology mappings (instrument_id -> symbol)
    /// and dataset information. The iterator yields `CmbpRecord` values
    /// converted from `dbn::CbboMsg` on the fly.
    pub fn open(&self) -> Result<(Metadata, RecordIterator<'_>)> {
        let file = File::open(&self.path)?;
        let reader = BufReader::with_capacity(IO_BUFFER_SIZE, file);
        let decoder =
            DynDecoder::inferred_with_buffer(reader, VersionUpgradePolicy::AsIs)?;
        let metadata = decoder.metadata().clone();

        Ok((
            metadata,
            RecordIterator {
                decoder,
                count: 0,
                decode_errors: 0,
            },
        ))
    }

    /// Return the file path this reader was constructed with.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Streaming iterator over `CmbpRecord` values from a `.dbn.zst` file.
///
/// Converts `dbn::CbboMsg` to `CmbpRecord` inside `next()`. Decode errors
/// are logged at WARN level and skipped — the iterator continues to the
/// next valid record.
pub struct RecordIterator<'a> {
    decoder: DynDecoder<'a, BufReader<File>>,
    count: u64,
    decode_errors: u64,
}

impl RecordIterator<'_> {
    /// Number of records successfully yielded so far.
    pub fn count(&self) -> u64 {
        self.count
    }

    /// Number of decode errors encountered (records skipped).
    pub fn decode_errors(&self) -> u64 {
        self.decode_errors
    }
}

impl Iterator for RecordIterator<'_> {
    type Item = CmbpRecord;

    fn next(&mut self) -> Option<Self::Item> {
        let mut consecutive_errors = 0u64;
        loop {
            match self.decoder.decode_record::<CbboMsg>() {
                Ok(Some(record)) => {
                    self.count += 1;
                    return Some(CmbpRecord::from_cbbo(record));
                }
                Ok(None) => return None,
                Err(e) => {
                    self.decode_errors += 1;
                    consecutive_errors += 1;
                    log::warn!(
                        "Decode error at record #{}: {}",
                        self.count + self.decode_errors,
                        e
                    );
                    if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                        log::error!(
                            "Aborting iterator: {} consecutive decode errors (limit: {})",
                            consecutive_errors,
                            MAX_CONSECUTIVE_ERRORS
                        );
                        return None;
                    }
                    continue;
                }
            }
        }
    }
}

/// Discover `.dbn.zst` files in a directory matching a filename pattern.
///
/// The pattern must contain `{date}` as a placeholder for the YYYYMMDD date string.
/// Returns a sorted vector of `(date_string, path)` tuples.
///
/// # Example
///
/// ```no_run
/// use std::path::Path;
/// use basic_quote_processor::reader::dbn_reader::discover_files;
///
/// let files = discover_files(
///     Path::new("../data/XNAS_BASIC/NVDA/cmbp1_2025-02-03_to_2026-01-09"),
///     "xnas-basic-{date}.cmbp-1.dbn.zst",
/// ).unwrap();
///
/// for (date, path) in &files {
///     println!("{}: {}", date, path.display());
/// }
/// ```
pub fn discover_files(
    dir: &Path,
    pattern: &str,
) -> Result<Vec<(String, PathBuf)>> {
    if !dir.is_dir() {
        return Err(ProcessorError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("Directory not found: {}", dir.display()),
        )));
    }

    // Split pattern on {date} to get prefix and suffix
    let parts: Vec<&str> = pattern.split("{date}").collect();
    if parts.len() != 2 {
        return Err(ProcessorError::config(format!(
            "Pattern must contain exactly one {{date}} placeholder, got: {}",
            pattern
        )));
    }
    let prefix = parts[0];
    let suffix = parts[1];

    let mut files = Vec::new();

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();

        if let Some(rest) = name.strip_prefix(prefix) {
            if let Some(date_str) = rest.strip_suffix(suffix) {
                // Validate date format: exactly 8 digits (YYYYMMDD)
                if date_str.len() == 8 && date_str.chars().all(|c| c.is_ascii_digit()) {
                    files.push((date_str.to_string(), entry.path()));
                }
            }
        }
    }

    // Sort by date string (lexicographic = chronological for YYYYMMDD)
    files.sort_by(|a, b| a.0.cmp(&b.0));

    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reader_nonexistent_file() {
        let result = DbnReader::new("/nonexistent/path.dbn.zst");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, ProcessorError::Io(_)),
            "Expected Io error, got: {:?}",
            err
        );
    }

    #[test]
    fn test_discover_files_bad_dir() {
        let result = discover_files(Path::new("/nonexistent/dir"), "test-{date}.dbn.zst");
        assert!(result.is_err());
    }

    #[test]
    fn test_discover_files_bad_pattern() {
        let dir = std::env::temp_dir();
        let result = discover_files(&dir, "no-placeholder.dbn.zst");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, ProcessorError::Config { .. }),
            "Expected Config error for missing {{date}}, got: {:?}",
            err
        );
    }
}
