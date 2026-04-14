//! Label computation for off-exchange feature vectors.
//!
//! Provides point-to-point return labels at multiple horizons
//! and forward mid-price trajectory export.
//!
//! Source: docs/design/04_FEATURE_SPECIFICATION.md §6

pub mod point_return;
pub mod forward_prices;

pub use point_return::{LabelComputer, LabelResult};
pub use forward_prices::ForwardPriceComputer;
