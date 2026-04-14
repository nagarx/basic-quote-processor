//! Off-exchange trade processing for XNAS.BASIC CMBP-1 data.
//!
//! Standalone Rust crate that reads Databento XNAS.BASIC `.dbn.zst` files,
//! classifies TRF trades (midpoint signing, BJZZ retail identification),
//! computes off-exchange features at configurable time bins, and exports
//! NPY sequences + labels for downstream ML consumption.
//!
//! # Architecture (Phase 1-4)
//!
//! **Phase 1 — Data Ingestion + BBO State**:
//! - `reader` — Reads `.dbn.zst` files, converts `dbn::CbboMsg` to `CmbpRecord`.
//! - `bbo_state` — L1 book state tracking (Nasdaq best bid/offer).
//!
//! **Phase 2 — Trade Classification**:
//! - `trade_classifier` — Midpoint signing (Barber 2024), BJZZ retail ID (Boehmer 2021), BVC.
//!
//! **Phase 3 — Feature Extraction**:
//! - `config` — TOML configuration: sampling, features, validation, VPIN, sequence, labels.
//! - `sampling` — Time-bin boundary detection (grid-aligned, DST-aware, gap detection).
//! - `accumulator` — Per-bin state accumulation with forward-fill.
//! - `features` — 34 off-exchange features with 3-level empty bin policy.
//! - `contract` — Schema version, feature indices, pipeline constants.
//!
//! **Phase 4 — Sequence Building + Labels + Export**:
//! - `sequence_builder` — Sliding window over feature bins (`Arc<Vec<f64>>` zero-copy).
//! - `labeling` — Point-return labels at multiple horizons + forward price trajectories.
//! - `export` — NPY writing (f32 sequences, f64 labels), normalization stats, metadata JSON.
//! - `pipeline` — `DayPipeline` orchestrator (init → stream → finalize).
//!
//! - `error` — Centralized error types.
//!
//! # Design Specification
//!
//! See `docs/design/` (7 documents) for the complete theoretical
//! foundation, module architecture, data flow, feature specification,
//! configuration schema, integration points, and testing strategy.

pub mod error;
pub mod contract;
pub mod reader;
pub mod bbo_state;
pub mod trade_classifier;
pub mod config;
pub mod sampling;
pub mod accumulator;
pub mod features;
pub mod sequence_builder;
pub mod labeling;
pub mod export;
pub mod pipeline;
pub mod context;
pub mod dates;

// Phase 1-2 re-exports
pub use error::{ProcessorError, Result};
pub use reader::{CmbpRecord, DbnReader, PublisherClass};
pub use bbo_state::BboState;
pub use trade_classifier::{
    TradeClassifier, ClassifiedTrade, TradeDirection, RetailStatus,
    ClassificationConfig, SigningMethod, BvcState,
};

// Phase 3 re-exports
pub use config::ProcessorConfig;
pub use sampling::{TimeBinSampler, BinBoundary};
pub use accumulator::{BinAccumulator, DaySummary};
pub use features::FeatureExtractor;

// Phase 4 re-exports
pub use sequence_builder::FeatureVec;
pub use labeling::{LabelComputer, LabelResult, ForwardPriceComputer};
pub use export::{NormalizationComputer, ExportMetadata, DatasetManifest};
pub use export::DayExport;
pub use pipeline::DayPipeline;

// Phase 5 re-exports
pub use context::{DailyContext, DailyContextLoader};
pub use reader::discover_files;
