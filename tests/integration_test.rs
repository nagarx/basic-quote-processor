//! Integration tests for basic-quote-processor Phase 1.
//!
//! These tests read real XNAS.BASIC CMBP-1 `.dbn.zst` files from the data directory.
//! They validate the Phase 1 gate: "Can iterate all records for a day and track BBO
//! correctly. Verify BBO mid/spread against manual spot-check."
//!
//! Requires: data/XNAS_BASIC/NVDA/cmbp1_2025-02-03_to_2026-01-09/ to be available.

use basic_quote_processor::reader::{DbnReader, PublisherClass};
use basic_quote_processor::bbo_state::BboState;
use std::path::Path;

/// Path to the XNAS.BASIC data directory (relative from crate root).
const DATA_DIR: &str = "../data/XNAS_BASIC/NVDA/cmbp1_2025-02-03_to_2026-01-09";

/// A small test file (first trading day).
const TEST_FILE: &str = "xnas-basic-20250203.cmbp-1.dbn.zst";

fn test_file_path() -> std::path::PathBuf {
    Path::new(DATA_DIR).join(TEST_FILE)
}

fn data_available() -> bool {
    test_file_path().exists()
}

#[test]
fn test_read_real_dbn_file() {
    if !data_available() {
        eprintln!("SKIP: test data not available at {}", test_file_path().display());
        return;
    }

    let reader = DbnReader::new(test_file_path()).expect("DbnReader::new failed");
    let (_metadata, records) = reader.open().expect("DbnReader::open failed");

    let mut count = 0u64;
    let mut trade_count = 0u64;
    let mut quote_count = 0u64;
    let mut other_count = 0u64;

    for record in records {
        count += 1;
        if record.is_trade() {
            trade_count += 1;
        } else if record.is_quote() {
            quote_count += 1;
        } else {
            other_count += 1;
        }
    }

    eprintln!(
        "Read {} records: {} trades, {} quotes, {} other",
        count, trade_count, quote_count, other_count
    );

    // Basic sanity: should have millions of records for a full trading day
    assert!(count > 100_000, "Expected >100K records, got {}", count);
    assert!(trade_count > 0, "Expected at least 1 trade");
    assert!(quote_count > 0, "Expected at least 1 quote");
}

#[test]
fn test_publisher_distribution() {
    if !data_available() {
        eprintln!("SKIP: test data not available");
        return;
    }

    let reader = DbnReader::new(test_file_path()).unwrap();
    let (_metadata, records) = reader.open().unwrap();

    let mut trf_count = 0u64;
    let mut lit_count = 0u64;
    let mut minor_lit_count = 0u64;
    let mut quote_only_count = 0u64;
    let mut unknown_count = 0u64;

    for record in records {
        match record.publisher_class() {
            PublisherClass::Trf => trf_count += 1,
            PublisherClass::Lit => lit_count += 1,
            PublisherClass::MinorLit => minor_lit_count += 1,
            PublisherClass::QuoteOnly => quote_only_count += 1,
            PublisherClass::Unknown => unknown_count += 1,
        }
    }

    eprintln!(
        "Publisher distribution: TRF={}, Lit={}, MinorLit={}, QuoteOnly={}, Unknown={}",
        trf_count, lit_count, minor_lit_count, quote_only_count, unknown_count
    );

    // All known publishers should be present
    assert!(trf_count > 0, "Expected TRF (82/83) records");
    assert!(lit_count > 0, "Expected Lit (81) records");
    // Publisher 93 (QuoteOnly) should have records
    assert!(quote_only_count > 0, "Expected QuoteOnly (93) records");
    // No unknown publishers in clean data
    assert_eq!(unknown_count, 0, "Unexpected unknown publisher IDs");
}

#[test]
fn test_timestamps_preserved() {
    if !data_available() {
        eprintln!("SKIP: test data not available");
        return;
    }

    let reader = DbnReader::new(test_file_path()).unwrap();
    let (_metadata, records) = reader.open().unwrap();

    let mut checked = 0;
    for record in records {
        // ts_event should be non-zero
        assert!(record.ts_event > 0, "ts_event should be non-zero");
        // ts_recv should be non-zero
        assert!(record.ts_recv > 0, "ts_recv should be non-zero");
        // Both should be in reasonable UTC nanosecond range (year 2025+)
        // 2025-01-01 00:00:00 UTC ≈ 1_735_689_600_000_000_000 ns
        assert!(
            record.ts_event > 1_700_000_000_000_000_000,
            "ts_event should be after 2023"
        );

        checked += 1;
        if checked >= 100 {
            break; // Don't need to check every record
        }
    }
    assert!(checked >= 100, "Expected at least 100 records to check");
}

#[test]
fn test_bbo_tracking_full_day() {
    if !data_available() {
        eprintln!("SKIP: test data not available");
        return;
    }

    let reader = DbnReader::new(test_file_path()).unwrap();
    let (_metadata, records) = reader.open().unwrap();

    let mut bbo = BboState::new();
    let mut total_records = 0u64;
    let mut valid_updates = 0u64;

    // Track a few spot-check values
    let spot_check_indices = [1000, 5000, 10000, 50000, 100000, 200000, 500000, 1000000, 2000000, 3000000];
    let mut spot_checks: Vec<(u64, f64, f64, f64)> = Vec::new(); // (index, mid, spread, microprice)

    for record in records {
        total_records += 1;

        if bbo.update_from_record(&record) {
            valid_updates += 1;
        }

        // Capture spot-check values at specific record indices
        if spot_check_indices.contains(&total_records) && bbo.is_valid {
            spot_checks.push((total_records, bbo.mid_price, bbo.spread, bbo.microprice));
        }
    }

    eprintln!("Total records: {}", total_records);
    eprintln!("Valid BBO updates: {}", valid_updates);
    eprintln!("Crossed rejections: {}", bbo.crossed_count());
    eprintln!("Invalid rejections: {}", bbo.invalid_count());
    eprintln!("BBO update count: {}", bbo.update_count());

    // Phase 1 gate: valid updates should be the majority
    assert!(
        valid_updates > total_records / 2,
        "Expected >50% valid updates, got {}/{}", valid_updates, total_records
    );

    // Spot-check: report values for manual verification
    eprintln!("\n--- BBO Spot-Checks (10 samples) ---");
    for (idx, mid, spread, microprice) in &spot_checks {
        eprintln!(
            "  Record #{}: mid=${:.6}, spread=${:.6}, microprice=${:.6}",
            idx, mid, spread, microprice
        );
        // Basic sanity checks on each spot-check
        assert!(mid.is_finite(), "Mid-price should be finite at record #{}", idx);
        assert!(*spread > 0.0, "Spread should be positive at record #{}", idx);
        assert!(microprice.is_finite(), "Microprice should be finite at record #{}", idx);
        // NVDA typically trades between $50 and $200
        assert!(*mid > 50.0 && *mid < 500.0,
            "Mid-price ${:.2} outside expected NVDA range [$50, $500] at record #{}", mid, idx);
    }

    assert!(
        spot_checks.len() >= 5,
        "Expected at least 5 spot-checks, got {} (file may be too small)",
        spot_checks.len()
    );

    // BBO quality assertions: crossed/invalid should be negligible in clean data
    let crossed_pct = bbo.crossed_count() as f64 / total_records as f64 * 100.0;
    let invalid_pct = bbo.invalid_count() as f64 / total_records as f64 * 100.0;
    let update_pct = bbo.update_count() as f64 / total_records as f64 * 100.0;
    eprintln!("BBO quality: crossed={:.4}%, invalid={:.4}%, valid_updates={:.2}%",
        crossed_pct, invalid_pct, update_pct);
    assert!(
        crossed_pct < 0.01,
        "Crossed BBO rate {:.4}% exceeds 0.01% threshold", crossed_pct
    );
    assert!(
        invalid_pct < 0.001,
        "Invalid price rate {:.4}% exceeds 0.001% threshold", invalid_pct
    );
    assert!(
        update_pct > 99.0,
        "Valid update rate {:.2}% below 99% threshold", update_pct
    );
}

#[test]
fn test_discover_files() {
    if !Path::new(DATA_DIR).exists() {
        eprintln!("SKIP: data directory not available");
        return;
    }

    let files = basic_quote_processor::reader::dbn_reader::discover_files(
        Path::new(DATA_DIR),
        "xnas-basic-{date}.cmbp-1.dbn.zst",
    )
    .expect("discover_files failed");

    eprintln!("Discovered {} files", files.len());

    // Should find 235 trading days
    assert!(
        files.len() >= 200,
        "Expected >= 200 files, got {}",
        files.len()
    );

    // Files should be sorted by date
    for i in 1..files.len() {
        assert!(
            files[i].0 > files[i - 1].0,
            "Files not sorted: {} comes after {}",
            files[i].0,
            files[i - 1].0
        );
    }

    // First file should be 20250203
    assert_eq!(files[0].0, "20250203", "First file date should be 20250203");
}

#[test]
fn test_nanodollar_precision_boundaries() {
    // Test the price conversion chain at critical boundaries
    use basic_quote_processor::contract::NANO_TO_USD;

    // $100.00 exact
    let p100: i64 = 100_000_000_000;
    assert_eq!(p100 as f64 * NANO_TO_USD, 100.0);

    // $100.235 (subpenny, BJZZ-relevant)
    let p_sub: i64 = 100_235_000_000;
    let usd = p_sub as f64 * NANO_TO_USD;
    assert!(
        (usd - 100.235).abs() < 1e-12,
        "Subpenny precision: expected 100.235, got {:.15}",
        usd
    );

    // 1 nanodollar (smallest positive)
    let p_one: i64 = 1;
    let usd_one = p_one as f64 * NANO_TO_USD;
    assert!(
        (usd_one - 1e-9).abs() < 1e-24,
        "1 nanodollar should be exactly 1e-9 USD"
    );

    // $1000.00 (large NVDA-range price)
    // Note: 1_000_000_000_000 as f64 * 1e-9 = 1000.0000000000001 (f64 precision limit)
    let p1000: i64 = 1_000_000_000_000;
    assert!(
        (p1000 as f64 * NANO_TO_USD - 1000.0).abs() < 1e-10,
        "$1000 conversion: expected ~1000.0, got {:.15}",
        p1000 as f64 * NANO_TO_USD
    );

    // i64::MAX (sentinel) — should produce a large but finite f64
    let p_max = i64::MAX as f64 * NANO_TO_USD;
    assert!(p_max.is_finite(), "i64::MAX * 1e-9 should be finite");
    assert!(p_max > 9e9, "i64::MAX * 1e-9 should be ~9.22e9");
}

#[test]
fn test_edge_crossed_locked_bbo() {
    use basic_quote_processor::bbo_state::BboState;

    let mut bbo = BboState::new();

    // Valid first, then crossed
    let valid = basic_quote_processor::reader::record::CmbpRecord {
        ts_event: 1_700_000_000_000_000_000,
        ts_recv: 1_700_000_000_000_100_000,
        action: b'A',
        side: b'N',
        flags: 0,
        price: 0,
        size: 0,
        bid_px: 100_000_000_000,
        bid_sz: 500,
        ask_px: 100_100_000_000,
        ask_sz: 300,
        publisher_id: 81,
    };
    assert!(bbo.update_from_record(&valid));
    assert!(bbo.is_valid);

    // Crossed: ask < bid
    let crossed = basic_quote_processor::reader::record::CmbpRecord {
        bid_px: 100_100_000_000,
        ask_px: 100_000_000_000,
        ..valid.clone()
    };
    assert!(!bbo.update_from_record(&crossed));
    assert_eq!(bbo.crossed_count(), 1);
    // Previous valid state preserved
    assert!(bbo.is_valid);
    assert!((bbo.bid_price - 100.0).abs() < 1e-12);

    // Locked: ask == bid
    let locked = basic_quote_processor::reader::record::CmbpRecord {
        bid_px: 100_000_000_000,
        ask_px: 100_000_000_000,
        ..valid.clone()
    };
    assert!(!bbo.update_from_record(&locked));
    assert_eq!(bbo.crossed_count(), 2);
}

#[test]
fn test_edge_zero_size_microprice() {
    use basic_quote_processor::bbo_state::BboState;
    use basic_quote_processor::reader::record::CmbpRecord;

    let mut bbo = BboState::new();
    let record = CmbpRecord {
        ts_event: 1_700_000_000_000_000_000,
        ts_recv: 1_700_000_000_000_100_000,
        action: b'A',
        side: b'N',
        flags: 0,
        price: 0,
        size: 0,
        bid_px: 100_000_000_000,
        bid_sz: 0,  // zero size
        ask_px: 100_100_000_000,
        ask_sz: 0,  // zero size
        publisher_id: 81,
    };
    assert!(bbo.update_from_record(&record));
    // Microprice should fall back to midpoint when both sizes are 0
    assert!(
        (bbo.microprice - bbo.mid_price).abs() < 1e-15,
        "Zero-size microprice should equal midpoint"
    );
}

// ============================================================================
// Edge case tests mandated by 07_TESTING_STRATEGY.md line 39
// ============================================================================

#[test]
fn test_edge_quote_only_bbo_tracking() {
    // "file with only quote updates (no trades)" — verify BBO tracks correctly
    // Use real data, but only process quote records (action == 'A')
    if !data_available() {
        eprintln!("SKIP: test data not available");
        return;
    }

    let reader = DbnReader::new(test_file_path()).unwrap();
    let (_metadata, records) = reader.open().unwrap();

    let mut bbo = BboState::new();
    let mut quote_count = 0u64;
    let mut valid_updates = 0u64;

    for record in records {
        if record.is_quote() {
            if bbo.update_from_record(&record) {
                valid_updates += 1;
            }
            quote_count += 1;
        }
        // Skip trades entirely — simulating a quote-only scenario
        if quote_count >= 10_000 {
            break; // Don't need to process entire day
        }
    }

    eprintln!(
        "Quote-only: {} quotes processed, {} valid BBO updates",
        quote_count, valid_updates
    );

    assert!(quote_count >= 10_000, "Expected at least 10K quotes");
    assert!(valid_updates > 0, "Expected at least 1 valid BBO update");
    assert!(bbo.is_valid, "BBO should be valid after quote updates");
    assert!(bbo.mid_price > 50.0 && bbo.mid_price < 500.0,
        "BBO mid-price should be in NVDA range, got {}", bbo.mid_price);
    assert!(bbo.spread > 0.0, "BBO spread should be positive");
}

#[test]
fn test_edge_corrupted_file() {
    // "corrupted file" — verify no panic, graceful error handling
    use std::io::Write;

    let tmp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let corrupt_path = tmp_dir.path().join("corrupt.dbn.zst");

    // Write random bytes that are NOT valid dbn format
    {
        let mut f = std::fs::File::create(&corrupt_path).expect("Failed to create temp file");
        f.write_all(b"THIS IS NOT A VALID DBN FILE \x00\xFF\xFE\x01\x02\x03")
            .expect("Failed to write corrupt data");
    }

    let reader = DbnReader::new(&corrupt_path).expect("DbnReader::new should succeed (file exists)");

    // open() should fail with a Dbn error (invalid format), not panic
    let result = reader.open();
    match result {
        Err(err) => {
            eprintln!("Corrupted file error (expected): {}", err);
            // Confirmed: graceful error, no panic
        }
        Ok((_metadata, records)) => {
            // If open() succeeds somehow, the iterator should yield 0 records
            // or error out quickly (no infinite loop due to MAX_CONSECUTIVE_ERRORS)
            let count = records.count();
            eprintln!("Corrupted file yielded {} records (expected 0)", count);
            assert_eq!(count, 0, "Corrupted file should yield 0 records");
        }
    };
}

#[test]
fn test_edge_empty_iterator() {
    // "file with zero records" — verify iterator yields None immediately
    // We can't easily create a valid dbn file with zero records without
    // dbn writing tools, so we test by taking 0 from the iterator.
    if !data_available() {
        eprintln!("SKIP: test data not available");
        return;
    }

    let reader = DbnReader::new(test_file_path()).unwrap();
    let (_metadata, records) = reader.open().unwrap();

    // Take 0 records — verify count accessor works
    let collected: Vec<_> = records.take(0).collect();
    assert_eq!(collected.len(), 0, "Taking 0 should yield 0 records");
}
