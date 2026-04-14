//! Data ingestion: reads XNAS.BASIC `.dbn.zst` files and emits `CmbpRecord` values.
//!
//! This module is stateless — it creates a new reader per file.
//! All downstream processing uses `CmbpRecord` (our internal type),
//! never raw `dbn::CbboMsg`.
//!
//! Source: docs/design/02_MODULE_ARCHITECTURE.md §4.1

pub mod publisher;
pub mod record;
pub mod dbn_reader;

pub use publisher::PublisherClass;
pub use record::CmbpRecord;
pub use dbn_reader::{DbnReader, discover_files};
