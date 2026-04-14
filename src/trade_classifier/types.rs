//! Core types for trade classification: direction, retail status, classified trade.
//!
//! These types are the output of the trade classification pipeline.
//! `TradeDirection` comes from midpoint signing (Barber et al. 2024).
//! `RetailStatus` comes from BJZZ subpenny identification (Boehmer et al. 2021).
//!
//! Source: docs/design/03_DATA_FLOW.md §3 (enum types)

use serde::Deserialize;
use crate::error::{ProcessorError, Result};

/// Trade direction determined by midpoint signing.
///
/// Source: Barber, B.M. et al. (2024). "A (Sub)penny for Your Thoughts."
///         *J. Finance*, 79(4), 2403-2427. Section III.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TradeDirection {
    /// Trade price > midpoint + exclusion_band * spread.
    Buy,
    /// Trade price < midpoint - exclusion_band * spread.
    Sell,
    /// Within exclusion band, BBO invalid, or non-TRF trade.
    Unsigned,
}

impl std::fmt::Display for TradeDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Buy => write!(f, "Buy"),
            Self::Sell => write!(f, "Sell"),
            Self::Unsigned => write!(f, "Unsigned"),
        }
    }
}

/// Retail status from BJZZ subpenny identification.
///
/// Source: Boehmer, E. et al. (2021). "Tracking Retail Investor Activity."
///         *J. Finance*, 76(5), 2249-2305. Section I.B, p. 2251.
///
/// IMPORTANT: BJZZ is a retail IDENTIFICATION method, NOT a trade SIGNING method.
/// Direction comes from midpoint signing; retail status comes from BJZZ.
/// See 01_THEORETICAL_FOUNDATION.md Section 2.5 for the distinction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RetailStatus {
    /// Subpenny fractional cent in retail zone: (0.001, 0.40) or (0.60, 0.999).
    Retail,
    /// Round penny (frac_cent ≈ 0), excluded midpoint zone [0.40, 0.60], or non-TRF trade.
    Institutional,
    /// BBO was invalid at time of trade — classification impossible.
    Unknown,
}

impl std::fmt::Display for RetailStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Retail => write!(f, "Retail"),
            Self::Institutional => write!(f, "Institutional"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

/// A trade with direction and retail status attached.
///
/// Produced by `TradeClassifier::classify()`. Contains all information
/// needed by the downstream `BinAccumulator` (Phase 3) for feature computation.
///
/// Source: docs/design/03_DATA_FLOW.md §3
#[derive(Debug, Clone)]
pub struct ClassifiedTrade {
    /// Trade direction from midpoint signing.
    pub direction: TradeDirection,
    /// Retail status from BJZZ subpenny identification.
    pub retail_status: RetailStatus,
    /// Trade price in USD (converted from i64 nanodollars).
    pub price: f64,
    /// Trade size in shares.
    pub size: u32,
    /// Original publisher ID for venue attribution.
    pub publisher_id: u16,
    /// Receipt timestamp (UTC nanoseconds) for time-bin assignment.
    pub ts_recv: u64,
}

/// Signing method for trade direction determination.
///
/// Source: docs/design/05_CONFIGURATION_SCHEMA.md [classification]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[derive(Default)]
#[serde(rename_all = "snake_case")]
pub enum SigningMethod {
    /// Barber et al. (2024) midpoint signing. Default. 94.8% accuracy.
    #[default]
    Midpoint,
    /// Lee-Ready (1991) tick test. Reserved — fails fast if selected.
    TickTest,
}

/// Configuration for the trade classifier.
///
/// Deserialized from `[classification]` TOML section.
/// All thresholds are configurable for experiment variation.
///
/// Source: docs/design/05_CONFIGURATION_SCHEMA.md §5
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ClassificationConfig {
    /// Algorithm for determining trade direction.
    pub signing_method: SigningMethod,
    /// Fraction of BBO spread defining the unsigned zone around midpoint.
    /// Uses FULL spread (not half-spread). Default 0.10 produces ~15.4% unsigned.
    /// Range: [0.0, 0.50].
    ///
    /// Source: Barber et al. (2024), Section III.
    pub exclusion_band: f64,
    /// Lower bound of subpenny fractional cent for retail identification.
    /// Excludes round-penny trades (frac_cent < bjzz_lower). Default 0.001.
    pub bjzz_lower: f64,
    /// Upper bound of retail sell zone. frac_cent in (bjzz_lower, bjzz_upper_sell) = retail.
    /// Default 0.40.
    pub bjzz_upper_sell: f64,
    /// Lower bound of retail buy zone. frac_cent in (bjzz_lower_buy, bjzz_upper) = retail.
    /// Default 0.60.
    pub bjzz_lower_buy: f64,
    /// Upper bound of retail buy zone. Default 0.999.
    pub bjzz_upper: f64,
}

impl Default for ClassificationConfig {
    /// E9-validated default parameters.
    fn default() -> Self {
        Self {
            signing_method: SigningMethod::Midpoint,
            exclusion_band: 0.10,
            bjzz_lower: 0.001,
            bjzz_upper_sell: 0.40,
            bjzz_lower_buy: 0.60,
            bjzz_upper: 0.999,
        }
    }
}

impl ClassificationConfig {
    /// Validate configuration parameters.
    ///
    /// Fails fast with descriptive error per pipeline convention (fail-fast configuration validation).
    pub fn validate(&self) -> Result<()> {
        if self.signing_method == SigningMethod::TickTest {
            return Err(ProcessorError::config(
                "tick_test signing not yet implemented; use 'midpoint' (default)",
            ));
        }
        if !(0.0..=0.50).contains(&self.exclusion_band) {
            return Err(ProcessorError::config(format!(
                "exclusion_band ({}) must be in [0.0, 0.50]",
                self.exclusion_band
            )));
        }
        if !(self.bjzz_lower > 0.0
            && self.bjzz_lower < self.bjzz_upper_sell
            && self.bjzz_upper_sell < 0.50)
        {
            return Err(ProcessorError::config(format!(
                "BJZZ sell zone invalid: need 0 < lower ({}) < upper_sell ({}) < 0.50",
                self.bjzz_lower, self.bjzz_upper_sell
            )));
        }
        if !(self.bjzz_lower_buy > 0.50
            && self.bjzz_lower_buy < self.bjzz_upper
            && self.bjzz_upper < 1.0)
        {
            return Err(ProcessorError::config(format!(
                "BJZZ buy zone invalid: need 0.50 < lower_buy ({}) < upper ({}) < 1.0",
                self.bjzz_lower_buy, self.bjzz_upper
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_matches_e9() {
        let config = ClassificationConfig::default();
        assert_eq!(config.signing_method, SigningMethod::Midpoint);
        assert_eq!(config.exclusion_band, 0.10);
        assert_eq!(config.bjzz_lower, 0.001);
        assert_eq!(config.bjzz_upper_sell, 0.40);
        assert_eq!(config.bjzz_lower_buy, 0.60);
        assert_eq!(config.bjzz_upper, 0.999);
    }

    #[test]
    fn test_default_config_validates() {
        let config = ClassificationConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_tick_test_fails_fast() {
        let config = ClassificationConfig {
            signing_method: SigningMethod::TickTest,
            ..Default::default()
        };
        let err = config.validate().unwrap_err();
        assert!(
            err.to_string().contains("tick_test"),
            "Should mention tick_test: {}",
            err
        );
    }

    #[test]
    fn test_invalid_exclusion_band() {
        let config = ClassificationConfig {
            exclusion_band: 0.60,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_invalid_bjzz_sell_zone() {
        let config = ClassificationConfig {
            bjzz_lower: 0.50, // > bjzz_upper_sell
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_invalid_bjzz_buy_zone() {
        let config = ClassificationConfig {
            bjzz_lower_buy: 0.40, // < 0.50
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_direction_display() {
        assert_eq!(format!("{}", TradeDirection::Buy), "Buy");
        assert_eq!(format!("{}", TradeDirection::Sell), "Sell");
        assert_eq!(format!("{}", TradeDirection::Unsigned), "Unsigned");
    }

    #[test]
    fn test_retail_status_display() {
        assert_eq!(format!("{}", RetailStatus::Retail), "Retail");
        assert_eq!(format!("{}", RetailStatus::Institutional), "Institutional");
        assert_eq!(format!("{}", RetailStatus::Unknown), "Unknown");
    }
}
