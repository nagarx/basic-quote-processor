//! Centralized error types for basic-quote-processor.
//!
//! Follows the `thiserror`-based enum pattern used by `feature-extractor-MBO-LOB`.
//! `#[from]` attributes enable clean `?` propagation from `std::io::Error` and
//! `dbn::Error` (returned by `DynDecoder::inferred_with_buffer()` and `decode_record()`).

use thiserror::Error;

/// Result type alias for this crate.
pub type Result<T> = std::result::Result<T, ProcessorError>;

/// Errors produced by the basic-quote-processor pipeline.
#[derive(Error, Debug)]
pub enum ProcessorError {
    /// I/O error (file operations, directory access).
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),

    /// DBN decoding error (from `dbn` crate: file format, record decode).
    #[error("DBN: {0}")]
    Dbn(#[from] dbn::Error),

    /// Configuration error (invalid TOML parameters, missing required fields).
    #[error("Configuration: {msg}")]
    Config { msg: String },

    /// Data error (corrupt records, invalid prices, unexpected field values).
    #[error("Data: {msg}")]
    Data { msg: String },

    /// Contract violation (schema mismatch, feature count, metadata validation).
    #[error("Contract violation: {msg}")]
    Contract { msg: String },

    /// Export error (file creation, NPY writing, metadata serialization).
    #[error("Export: {msg}")]
    Export { msg: String },

    /// Label error (insufficient data, invalid horizon).
    #[error("Label: {msg}")]
    Label { msg: String },

    /// Generic error (escape hatch for uncommon error paths).
    #[error("{0}")]
    Other(String),
}

impl ProcessorError {
    /// Convenience constructor for configuration errors.
    pub fn config(msg: impl Into<String>) -> Self {
        Self::Config { msg: msg.into() }
    }

    /// Convenience constructor for data errors.
    pub fn data(msg: impl Into<String>) -> Self {
        Self::Data { msg: msg.into() }
    }

    /// Convenience constructor for contract violations.
    pub fn contract(msg: impl Into<String>) -> Self {
        Self::Contract { msg: msg.into() }
    }

    /// Convenience constructor for export errors.
    pub fn export(msg: impl Into<String>) -> Self {
        Self::Export { msg: msg.into() }
    }

    /// Convenience constructor for label errors.
    pub fn label(msg: impl Into<String>) -> Self {
        Self::Label { msg: msg.into() }
    }
}

impl From<String> for ProcessorError {
    fn from(s: String) -> Self {
        Self::Other(s)
    }
}

impl From<&str> for ProcessorError {
    fn from(s: &str) -> Self {
        Self::Other(s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display_io() {
        let err = ProcessorError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "file missing",
        ));
        assert!(err.to_string().contains("file missing"));
    }

    #[test]
    fn test_error_display_config() {
        let err = ProcessorError::config("bad threshold");
        assert_eq!(err.to_string(), "Configuration: bad threshold");
    }

    #[test]
    fn test_error_display_data() {
        let err = ProcessorError::data("corrupt record");
        assert_eq!(err.to_string(), "Data: corrupt record");
    }

    #[test]
    fn test_error_display_contract() {
        let err = ProcessorError::contract("schema mismatch");
        assert_eq!(err.to_string(), "Contract violation: schema mismatch");
    }

    #[test]
    fn test_error_display_export() {
        let err = ProcessorError::export("write failed");
        assert_eq!(err.to_string(), "Export: write failed");
    }

    #[test]
    fn test_error_display_label() {
        let err = ProcessorError::label("insufficient data");
        assert_eq!(err.to_string(), "Label: insufficient data");
    }

    #[test]
    fn test_error_from_string() {
        let err: ProcessorError = "something went wrong".into();
        assert_eq!(err.to_string(), "something went wrong");
    }

    #[test]
    fn test_result_type_alias() {
        let ok: Result<i32> = Ok(42);
        assert_eq!(ok.unwrap(), 42);
        let err: Result<i32> = Err(ProcessorError::config("test"));
        assert!(err.is_err());
    }
}
