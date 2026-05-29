//! Build script: capture git provenance at compile time.
//!
//! Emits `GIT_COMMIT_HASH` and `GIT_DIRTY` as `rustc-env` vars, read at runtime
//! via `option_env!` in `ExportMetadata::build()` (src/export/metadata.rs) so
//! every export's `provenance` block is forensically identifiable. Mirrors the
//! MBO extractor pattern (feature-extractor-MBO-LOB/crates/hft-extractor/build.rs)
//! for cross-pipeline consistency. Degrades gracefully to "unknown"/false when
//! git is unavailable (e.g. building from a source tarball).

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/heads/");

    let git_hash = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let git_dirty = Command::new("git")
        .args(["diff", "--quiet", "HEAD"])
        .output()
        .map(|o| !o.status.success())
        .unwrap_or(false);

    println!("cargo:rustc-env=GIT_COMMIT_HASH={}", git_hash);
    println!("cargo:rustc-env=GIT_DIRTY={}", git_dirty);
}
