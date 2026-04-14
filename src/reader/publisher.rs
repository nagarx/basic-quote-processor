//! Publisher ID classification for XNAS.BASIC CMBP-1 venues.
//!
//! Six publisher IDs appear in the XNAS.BASIC feed for NVDA:
//!
//! | ID | Venue | Type | Share of Trades |
//! |----|-------|------|-----------------|
//! | 81 | XNAS  | Lit  | ~31% |
//! | 82 | FINN  | TRF  | ~67% |
//! | 83 | FINC  | TRF  | ~2%  |
//! | 88 | XBOS  | Minor Lit | ~0.2% |
//! | 89 | XPSX  | Minor Lit | ~0.2% |
//! | 93 | --    | Quote Only | 0% trades |
//!
//! TRF = FINRA Trade Reporting Facility (off-exchange).
//! Publisher 93 emits only BBO quote updates (~5.6M records/day) and zero trades.
//!
//! Source: docs/design/01_THEORETICAL_FOUNDATION.md §1.2

/// Named publisher ID constants.
pub const XNAS: u16 = 81;
pub const FINN: u16 = 82;
pub const FINC: u16 = 83;
pub const XBOS: u16 = 88;
pub const XPSX: u16 = 89;
pub const CONSOLIDATED_QUOTE: u16 = 93;

/// Venue classification derived from publisher ID.
///
/// Source: docs/design/01_THEORETICAL_FOUNDATION.md §1.2
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PublisherClass {
    /// FINRA TRF (off-exchange): FINN (82), FINC (83).
    /// All off-exchange flow features derive from trades with these publisher IDs.
    Trf,

    /// Primary lit exchange: XNAS / Nasdaq (81).
    Lit,

    /// Secondary lit exchanges: XBOS / Nasdaq BX (88), XPSX / Nasdaq PSX (89).
    /// Configurable: `include_minor_lit_in_lit` controls whether these count as lit.
    MinorLit,

    /// Consolidated BBO quote source (93). Emits only quote updates, zero trades.
    QuoteOnly,

    /// Unrecognized publisher ID.
    Unknown,
}

impl PublisherClass {
    /// Classify a publisher ID into a venue category.
    #[inline]
    pub fn from_id(id: u16) -> Self {
        match id {
            XNAS => Self::Lit,
            FINN | FINC => Self::Trf,
            XBOS | XPSX => Self::MinorLit,
            CONSOLIDATED_QUOTE => Self::QuoteOnly,
            _ => Self::Unknown,
        }
    }

    /// True if this publisher is a FINRA TRF (off-exchange).
    #[inline]
    pub fn is_trf(self) -> bool {
        matches!(self, Self::Trf)
    }

    /// True if this publisher is any lit exchange (XNAS + minor lit).
    #[inline]
    pub fn is_lit(self) -> bool {
        matches!(self, Self::Lit | Self::MinorLit)
    }

    /// True if this publisher is the primary lit exchange (XNAS only).
    /// Use this when `include_minor_lit_in_lit = false`.
    #[inline]
    pub fn is_lit_strict(self) -> bool {
        matches!(self, Self::Lit)
    }
}

impl std::fmt::Display for PublisherClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Trf => write!(f, "TRF"),
            Self::Lit => write!(f, "Lit"),
            Self::MinorLit => write!(f, "MinorLit"),
            Self::QuoteOnly => write!(f, "QuoteOnly"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trf_classification() {
        assert_eq!(PublisherClass::from_id(82), PublisherClass::Trf);
        assert_eq!(PublisherClass::from_id(83), PublisherClass::Trf);
        assert!(PublisherClass::from_id(82).is_trf());
        assert!(PublisherClass::from_id(83).is_trf());
    }

    #[test]
    fn test_lit_classification() {
        assert_eq!(PublisherClass::from_id(81), PublisherClass::Lit);
        assert!(PublisherClass::from_id(81).is_lit());
        assert!(PublisherClass::from_id(81).is_lit_strict());
    }

    #[test]
    fn test_minor_lit_classification() {
        assert_eq!(PublisherClass::from_id(88), PublisherClass::MinorLit);
        assert_eq!(PublisherClass::from_id(89), PublisherClass::MinorLit);
        // Minor lit IS lit (broad definition)
        assert!(PublisherClass::from_id(88).is_lit());
        // Minor lit is NOT lit_strict
        assert!(!PublisherClass::from_id(88).is_lit_strict());
    }

    #[test]
    fn test_quote_only_classification() {
        assert_eq!(PublisherClass::from_id(93), PublisherClass::QuoteOnly);
        assert!(!PublisherClass::from_id(93).is_trf());
        assert!(!PublisherClass::from_id(93).is_lit());
    }

    #[test]
    fn test_unknown_classification() {
        assert_eq!(PublisherClass::from_id(0), PublisherClass::Unknown);
        assert_eq!(PublisherClass::from_id(999), PublisherClass::Unknown);
        assert!(!PublisherClass::from_id(0).is_trf());
        assert!(!PublisherClass::from_id(0).is_lit());
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", PublisherClass::Trf), "TRF");
        assert_eq!(format!("{}", PublisherClass::Lit), "Lit");
        assert_eq!(format!("{}", PublisherClass::Unknown), "Unknown");
    }

    #[test]
    fn test_all_known_publisher_ids() {
        // Verify no ID is accidentally Unknown
        let known_ids = [81, 82, 83, 88, 89, 93];
        for id in known_ids {
            assert_ne!(
                PublisherClass::from_id(id),
                PublisherClass::Unknown,
                "Publisher ID {} should not be Unknown",
                id
            );
        }
    }
}
