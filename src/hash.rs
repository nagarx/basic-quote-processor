//! Streaming SHA-256 of a file's raw bytes.
//!
//! Reuses the `sha2` crate already used by [`crate::config::ProcessorConfig::config_hash_hex`]
//! (no new dependency). It is deliberately NOT `hft_statistics::io::*` — pulling
//! that path would activate the local `.cargo/config.toml` `[[patch.unused]]`
//! override (swapping `hft-statistics` git-`0.1.0` → local `0.3.0-dev`) and churn
//! the dependency graph; see CODEBASE.md "Audit Findings" §M-2.
//!
//! The file is streamed in fixed-size chunks rather than `std::fs::read` (which
//! would slurp a multi-hundred-MB `.dbn.zst` into memory) per hft-rules §12.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::Result;

/// Read-buffer size for the streaming hash (64 KiB).
const HASH_CHUNK_BYTES: usize = 64 * 1024;

/// Compute the lowercase-hex SHA-256 of a file's RAW bytes.
///
/// For a `.dbn.zst` input this hashes the **compressed** file exactly as
/// delivered — the right granularity to detect a Databento re-issue (any byte
/// change to the delivered artifact), and cross-checkable against the
/// ingest-side SHA-256. It does NOT decompress (that would be a second full
/// zstd decode of the corpus). Cost: one extra sequential read per file.
///
/// # Arguments
/// * `path` — path to the file to hash.
///
/// # Returns
/// 64-character lowercase hex SHA-256 of the file's bytes.
///
/// # Errors
/// Returns [`crate::error::ProcessorError::Io`] if the file cannot be opened or
/// read.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; HASH_CHUNK_BYTES];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::io::Write;

    #[test]
    fn test_sha256_file_empty() {
        // SHA-256 of the empty input — standard published value.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("empty.bin");
        File::create(&p).unwrap();
        assert_eq!(
            sha256_file(&p).unwrap(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_sha256_file_known_vector() {
        // SHA-256("abc") — NIST FIPS 180-2 §B.1 test vector.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("abc.bin");
        let mut f = File::create(&p).unwrap();
        f.write_all(b"abc").unwrap();
        f.flush().unwrap();
        assert_eq!(
            sha256_file(&p).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn test_sha256_file_multichunk_matches_single_shot() {
        // Larger than HASH_CHUNK_BYTES so the streaming loop spans multiple
        // reads. This is NOT a formula re-derivation — it verifies the chunk
        // loop has no boundary bug by asserting the streamed hash equals a
        // single-shot `update()` of the same bytes.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("big.bin");
        let block: Vec<u8> = (0..(HASH_CHUNK_BYTES + 123)).map(|i| (i % 251) as u8).collect();
        let mut f = File::create(&p).unwrap();
        f.write_all(&block).unwrap();
        f.flush().unwrap();

        let mut single = Sha256::new();
        single.update(&block);
        assert_eq!(sha256_file(&p).unwrap(), format!("{:x}", single.finalize()));
    }

    #[test]
    fn test_sha256_file_nonexistent_errors() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("does_not_exist.bin");
        assert!(sha256_file(&p).is_err());
    }
}
