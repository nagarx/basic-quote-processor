//! Date utilities for multi-day processing and train/val/test splitting.
//!
//! Provides weekday enumeration (excluding weekends and holidays),
//! chronological split assignment, and date format conversions.

use chrono::{Datelike, NaiveDate, Weekday};

use crate::error::{ProcessorError, Result};

/// Train/val/test split assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Split {
    Train,
    Val,
    Test,
}

impl std::fmt::Display for Split {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Split::Train => write!(f, "train"),
            Split::Val => write!(f, "val"),
            Split::Test => write!(f, "test"),
        }
    }
}

/// Enumerate all weekdays (Mon-Fri) in an inclusive date range.
///
/// Returns dates in chronological order. Weekends are excluded.
pub fn enumerate_weekdays(start: NaiveDate, end: NaiveDate) -> Vec<NaiveDate> {
    if start > end {
        return vec![];
    }
    let mut dates = Vec::new();
    let mut current = start;
    while current <= end {
        match current.weekday() {
            Weekday::Sat | Weekday::Sun => {}
            _ => dates.push(current),
        }
        current = current.succ_opt().unwrap_or(end);
        if current == end && current != start {
            // Check the last day
            match current.weekday() {
                Weekday::Sat | Weekday::Sun => {}
                _ => {
                    if !dates.last().is_some_and(|&d| d == current) {
                        dates.push(current);
                    }
                }
            }
            break;
        }
    }
    dates
}

/// Enumerate weekdays in range, excluding specific dates (holidays).
pub fn enumerate_weekdays_excluding(
    start: NaiveDate,
    end: NaiveDate,
    exclude: &[NaiveDate],
) -> Vec<NaiveDate> {
    enumerate_weekdays(start, end)
        .into_iter()
        .filter(|d| !exclude.contains(d))
        .collect()
}

/// Assign a date to a train/val/test split based on date boundaries.
///
/// - `date <= train_end` → Train
/// - `train_end < date <= val_end` → Val
/// - `date > val_end` → Test
pub fn assign_split(date: NaiveDate, train_end: NaiveDate, val_end: NaiveDate) -> Split {
    if date <= train_end {
        Split::Train
    } else if date <= val_end {
        Split::Val
    } else {
        Split::Test
    }
}

/// Convert NaiveDate to "YYYYMMDD" format for filename patterns.
pub fn date_to_file_date(date: NaiveDate) -> String {
    date.format("%Y%m%d").to_string()
}

/// Convert NaiveDate to "YYYY-MM-DD" ISO format for metadata.
pub fn date_to_iso(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// Parse "YYYYMMDD" string to (year, month, day) for `DayPipeline::init_day()`.
pub fn parse_file_date(s: &str) -> Result<(i32, u32, u32)> {
    if s.len() != 8 {
        return Err(ProcessorError::config(format!(
            "Date string '{}' must be 8 characters (YYYYMMDD)", s
        )));
    }
    let year: i32 = s[0..4].parse()
        .map_err(|_| ProcessorError::config(format!("Invalid year in '{}'", s)))?;
    let month: u32 = s[4..6].parse()
        .map_err(|_| ProcessorError::config(format!("Invalid month in '{}'", s)))?;
    let day: u32 = s[6..8].parse()
        .map_err(|_| ProcessorError::config(format!("Invalid day in '{}'", s)))?;

    // Validate the date is real
    NaiveDate::from_ymd_opt(year, month, day)
        .ok_or_else(|| ProcessorError::config(format!(
            "Invalid date: {}-{:02}-{:02}", year, month, day
        )))?;

    Ok((year, month, day))
}

/// Parse "YYYY-MM-DD" ISO string to NaiveDate.
pub fn parse_iso_date(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|e| ProcessorError::config(format!(
            "Invalid ISO date '{}': {}", s, e
        )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn test_enumerate_weekdays_skips_weekends() {
        // 2025-02-03 (Mon) to 2025-02-09 (Sun) = 5 weekdays
        let dates = enumerate_weekdays(d(2025, 2, 3), d(2025, 2, 9));
        assert_eq!(dates.len(), 5);
        assert_eq!(dates[0], d(2025, 2, 3)); // Mon
        assert_eq!(dates[4], d(2025, 2, 7)); // Fri
    }

    #[test]
    fn test_enumerate_weekdays_empty_range() {
        let dates = enumerate_weekdays(d(2025, 2, 10), d(2025, 2, 3));
        assert!(dates.is_empty());
    }

    #[test]
    fn test_enumerate_weekdays_single_day() {
        let dates = enumerate_weekdays(d(2025, 2, 3), d(2025, 2, 3));
        assert_eq!(dates.len(), 1);
        assert_eq!(dates[0], d(2025, 2, 3));
    }

    #[test]
    fn test_enumerate_weekdays_single_weekend() {
        let dates = enumerate_weekdays(d(2025, 2, 1), d(2025, 2, 1)); // Saturday
        assert!(dates.is_empty());
    }

    #[test]
    fn test_enumerate_excluding_holidays() {
        let dates = enumerate_weekdays_excluding(
            d(2025, 2, 3), d(2025, 2, 7),
            &[d(2025, 2, 5)], // Exclude Wednesday
        );
        assert_eq!(dates.len(), 4); // Mon, Tue, Thu, Fri
        assert!(!dates.contains(&d(2025, 2, 5)));
    }

    #[test]
    fn test_assign_split() {
        let train_end = d(2025, 9, 30);
        let val_end = d(2025, 11, 13);

        assert_eq!(assign_split(d(2025, 2, 3), train_end, val_end), Split::Train);
        assert_eq!(assign_split(d(2025, 9, 30), train_end, val_end), Split::Train); // boundary
        assert_eq!(assign_split(d(2025, 10, 1), train_end, val_end), Split::Val);
        assert_eq!(assign_split(d(2025, 11, 13), train_end, val_end), Split::Val); // boundary
        assert_eq!(assign_split(d(2025, 11, 14), train_end, val_end), Split::Test);
        assert_eq!(assign_split(d(2026, 1, 6), train_end, val_end), Split::Test);
    }

    #[test]
    fn test_date_to_file_date() {
        assert_eq!(date_to_file_date(d(2025, 2, 3)), "20250203");
        assert_eq!(date_to_file_date(d(2025, 12, 31)), "20251231");
    }

    #[test]
    fn test_date_to_iso() {
        assert_eq!(date_to_iso(d(2025, 2, 3)), "2025-02-03");
    }

    #[test]
    fn test_parse_file_date() {
        let (y, m, day) = parse_file_date("20250203").unwrap();
        assert_eq!((y, m, day), (2025, 2, 3));
    }

    #[test]
    fn test_parse_file_date_invalid() {
        assert!(parse_file_date("2025020").is_err()); // too short
        assert!(parse_file_date("20251301").is_err()); // invalid month
        assert!(parse_file_date("20250230").is_err()); // invalid day
    }

    #[test]
    fn test_parse_iso_date() {
        let date = parse_iso_date("2025-02-03").unwrap();
        assert_eq!(date, d(2025, 2, 3));
    }

    #[test]
    fn test_split_display() {
        assert_eq!(format!("{}", Split::Train), "train");
        assert_eq!(format!("{}", Split::Val), "val");
        assert_eq!(format!("{}", Split::Test), "test");
    }
}
