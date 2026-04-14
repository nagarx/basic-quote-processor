//! Contract conformance integration test (F5, Phase 9.7).
//!
//! Validates that metadata emitted by `ExportMetadataBuilder` conforms to the
//! off-exchange contract consumed by `hft-contracts` (Python). This test is
//! the primary contract gate: `cargo test` catches drift even without the
//! Python validator (standalone repo, or CI lacking the Python venv).
//!
//! UPSTREAM SSoT: `contracts/pipeline_contract.toml` in the parent monorepo
//! UPSTREAM VALIDATOR: `hft-contracts/src/hft_contracts/validation.py:338-414`
//! UPSTREAM T9 CONSUMER: `hft-contracts/src/hft_contracts/label_factory.py:165-174`
//!
//! When the upstream contract changes, update the hardcoded mirror list
//! (`OFFEX_REQUIRED_FIELDS` below) AND the upstream Python validator in the
//! same commit. The `basic-quote-processor` standalone repo cannot read the
//! parent monorepo's TOML, so we embed the required-field set here.

use basic_quote_processor::ExportMetadata;
use serde_json::Value;
use tempfile::tempdir;

/// Mirror of `offex_required` at `hft-contracts/validation.py:391-396`.
///
/// DRIFT PROTECTION: if a field is added here without updating the Python
/// validator (or vice versa), the upstream validate call will silently pass
/// / fail out of sync with this test. Keep in lockstep.
const OFFEX_REQUIRED_FIELDS: &[&str] = &[
    "day",
    "n_sequences",
    "window_size",
    "n_features",
    "schema_version",
    "contract_version",
    "label_strategy",
    "label_encoding",
    "horizons",
    "bin_size_seconds",
    "normalization",
    "provenance",
    "export_timestamp",
];

/// Build a fully-populated `ExportMetadata` that a production CLI run would
/// emit. Every Phase 9 field is set so we can validate each one.
fn canonical_metadata() -> ExportMetadata {
    ExportMetadata::builder()
        .day("2025-02-03")
        .n_sequences(308)
        .window_size(20)
        .horizons(vec![1, 2, 3, 5, 10, 20, 30, 60])
        .bin_size_seconds(60)
        .symbol("NVDA")
        // Phase 9.4 — provenance
        .config_hash(&"a".repeat(64))
        .provenance_source_file("xnas-basic-20250203.cmbp-1.dbn.zst")
        // Phase 9.5 — honest normalization strategy
        .normalization_strategy("none")
        // Round 7 / D13 — experiment identifier
        .experiment("basic_nvda_60s_phase9")
        // Phase 9.1 — forward_prices block (required by T9 LabelFactory)
        .forward_prices(60)
        // F2 — active FeatureConfig + ClassificationConfig snapshot
        .feature_groups_enabled(serde_json::json!({
            "signed_flow": true,
            "venue_metrics": true,
            "retail_metrics": true,
            "bbo_dynamics": true,
            "vpin": false,
            "trade_size": true,
            "cross_venue": true,
        }))
        .classification_config(serde_json::json!({
            "signing_method": "midpoint",
            "exclusion_band": 0.10,
        }))
        .build()
        .expect("builder must succeed with canonical fields")
}

/// Write canonical metadata to a temp directory and re-read as `serde_json::Value`.
fn write_and_parse(meta: &ExportMetadata) -> Value {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_metadata.json");
    meta.write_to_file(&path).expect("write metadata JSON");
    let content = std::fs::read_to_string(&path).expect("read back metadata");
    serde_json::from_str(&content).expect("JSON must parse")
}

// ──────────────────────────────────────────────────────────────────────
// Contract fields (validation.py offex_required)
// ──────────────────────────────────────────────────────────────────────

#[test]
fn test_all_offex_required_fields_present() {
    let meta = canonical_metadata();
    let parsed = write_and_parse(&meta);

    for field in OFFEX_REQUIRED_FIELDS {
        assert!(
            parsed.get(field).is_some(),
            "Required field '{}' missing — Python validator would reject. \
             Fields present: {:?}",
            field,
            parsed.as_object().map(|o| o.keys().collect::<Vec<_>>())
        );
    }
}

#[test]
fn test_schema_version_is_off_exchange_1_0() {
    // validation.py:338-345 — schema_version must equal OFF_EXCHANGE_SCHEMA_VERSION ("1.0")
    let meta = canonical_metadata();
    let parsed = write_and_parse(&meta);
    assert_eq!(
        parsed["schema_version"], "1.0",
        "schema_version must equal \"1.0\" (OFF_EXCHANGE_SCHEMA_VERSION)"
    );
}

#[test]
fn test_contract_version_prefix_off_exchange() {
    // validation.py:347-352 — contract_version must start with "off_exchange"
    let meta = canonical_metadata();
    let parsed = write_and_parse(&meta);
    let cv = parsed["contract_version"]
        .as_str()
        .expect("contract_version must be a string");
    assert!(
        cv.starts_with("off_exchange"),
        "contract_version must start with 'off_exchange', got '{cv}'"
    );
}

#[test]
fn test_n_features_is_34() {
    // validation.py:354-359 — n_features must equal OFF_EXCHANGE_FEATURE_COUNT (34)
    let meta = canonical_metadata();
    let parsed = write_and_parse(&meta);
    assert_eq!(
        parsed["n_features"], 34,
        "n_features must be 34 — matches OffExchangeFeatureIndex count"
    );
}

#[test]
fn test_normalization_applied_is_false() {
    // validation.py:361-365 — normalization.applied must be false (Rust exports raw per T15)
    let meta = canonical_metadata();
    let parsed = write_and_parse(&meta);
    assert_eq!(
        parsed["normalization"]["applied"], false,
        "normalization.applied must be false — Rust exports raw f64 per T15"
    );
}

// ──────────────────────────────────────────────────────────────────────
// Provenance (Phase 9.4)
// ──────────────────────────────────────────────────────────────────────

#[test]
fn test_provenance_has_required_sub_fields() {
    // validation.py:405-413 — provenance requires processor_version AND export_timestamp_utc
    let meta = canonical_metadata();
    let parsed = write_and_parse(&meta);
    let prov = &parsed["provenance"];
    assert!(
        prov.get("processor_version").is_some(),
        "provenance.processor_version required (validate_provenance_present)"
    );
    assert!(
        prov.get("export_timestamp_utc").is_some(),
        "provenance.export_timestamp_utc required (validate_provenance_present)"
    );
}

#[test]
fn test_provenance_config_hash_is_64_char_hex() {
    // Phase 9.4 — SHA-256 of canonical TOML → 64 lowercase hex chars.
    // Downstream (EXPERIMENT_INDEX.md linkage, trainer ExperimentSpec) assumes this shape.
    let meta = canonical_metadata();
    let parsed = write_and_parse(&meta);
    let hash = parsed["provenance"]["config_hash"]
        .as_str()
        .expect("config_hash present when set via builder");
    assert_eq!(hash.len(), 64, "config_hash must be 64 chars, got {}", hash.len());
    assert!(
        hash.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "config_hash must be lowercase hex, got: {hash}"
    );
}

#[test]
fn test_provenance_source_file_is_non_empty_basename() {
    // F1 (Phase 9.4) — source_file must be the actual `.dbn.zst` basename,
    // not the empty string that pre-Phase-9 metadata carried.
    let meta = canonical_metadata();
    let parsed = write_and_parse(&meta);
    let src = parsed["provenance"]["source_file"]
        .as_str()
        .expect("source_file present");
    assert!(!src.is_empty(), "source_file must not be empty (was '' in v0)");
    assert!(
        src.ends_with(".dbn.zst"),
        "source_file should reference a .dbn.zst file, got '{src}'"
    );
    assert!(
        !src.contains('/') && !src.contains('\\'),
        "source_file must be basename only, not a path; got '{src}'"
    );
}

// ──────────────────────────────────────────────────────────────────────
// Active config snapshot (Phase 9.4 / F2)
// ──────────────────────────────────────────────────────────────────────

#[test]
fn test_feature_groups_enabled_is_non_empty_object() {
    // F2 (Phase 9.4) — must carry the active FeatureConfig, not empty `{}`.
    let meta = canonical_metadata();
    let parsed = write_and_parse(&meta);
    let fge = parsed["feature_groups_enabled"]
        .as_object()
        .expect("feature_groups_enabled must be a JSON object");
    assert!(
        !fge.is_empty(),
        "feature_groups_enabled must NOT be empty (was `{{}}` in v0)"
    );
    // Spot-check known FeatureConfig keys
    assert!(
        fge.contains_key("signed_flow"),
        "feature_groups_enabled missing 'signed_flow' key"
    );
    assert!(
        fge.contains_key("vpin"),
        "feature_groups_enabled missing 'vpin' key"
    );
}

#[test]
fn test_classification_config_is_non_empty_object() {
    // F2 (Phase 9.4) — must carry the active ClassificationConfig, not empty `{}`.
    let meta = canonical_metadata();
    let parsed = write_and_parse(&meta);
    let cc = parsed["classification_config"]
        .as_object()
        .expect("classification_config must be a JSON object");
    assert!(
        !cc.is_empty(),
        "classification_config must NOT be empty (was `{{}}` in v0)"
    );
}

// ──────────────────────────────────────────────────────────────────────
// Normalization strategy honesty (Phase 9.5)
// ──────────────────────────────────────────────────────────────────────

#[test]
fn test_normalization_strategy_reflects_input() {
    // Phase 9.5 — strategy field is NOT hardcoded "per_day_zscore";
    // it reflects what the builder was told (here: "none").
    let meta = canonical_metadata();
    let parsed = write_and_parse(&meta);
    assert_eq!(
        parsed["normalization"]["strategy"], "none",
        "Phase 9.5: metadata must report the actual configured strategy"
    );
}

// ──────────────────────────────────────────────────────────────────────
// Forward-prices metadata block (Phase 9.1 — unblocks T9 LabelFactory)
// ──────────────────────────────────────────────────────────────────────

#[test]
fn test_forward_prices_has_all_six_fields() {
    // Phase 9.1 — forward_prices block must have all 6 required fields from
    // `pipeline_contract.toml [forward_prices.metadata]`.
    // Parsed by `hft-contracts.ForwardPriceContract.from_metadata()` (4 of 6).
    let meta = canonical_metadata();
    let parsed = write_and_parse(&meta);
    let fp = parsed
        .get("forward_prices")
        .expect("forward_prices block required when builder.forward_prices() called");

    for field in [
        "exported",
        "smoothing_window_offset",
        "max_horizon",
        "n_columns",
        "units",
        "column_layout",
    ] {
        assert!(
            fp.get(field).is_some(),
            "forward_prices.{field} required by pipeline_contract.toml"
        );
    }
}

#[test]
fn test_forward_prices_n_columns_invariant() {
    // Invariant from `label_factory.py:100-108` (ForwardPriceContract.__post_init__):
    //   n_columns == smoothing_window_offset + max_horizon + 1
    // If violated, the trainer raises at load time.
    let meta = canonical_metadata();
    let parsed = write_and_parse(&meta);
    let fp = &parsed["forward_prices"];
    let k = fp["smoothing_window_offset"].as_u64().unwrap();
    let h = fp["max_horizon"].as_u64().unwrap();
    let n = fp["n_columns"].as_u64().unwrap();
    assert_eq!(
        n,
        k + h + 1,
        "n_columns invariant violated: n_columns={n} != k({k}) + H({h}) + 1"
    );
}

#[test]
fn test_forward_prices_smoothing_offset_zero_and_units_usd() {
    // basic-quote-processor-specific hardcoded values: k=0, units=USD.
    // A future BASIC variant with past-smoothing must update both the storage
    // layout AND this metadata in lockstep.
    let meta = canonical_metadata();
    let parsed = write_and_parse(&meta);
    let fp = &parsed["forward_prices"];
    assert_eq!(
        fp["smoothing_window_offset"], 0,
        "basic-quote-processor does NOT smooth → smoothing_window_offset must be 0"
    );
    assert_eq!(fp["units"], "USD", "forward_prices.units must be USD");
    assert_eq!(fp["exported"], true, "forward_prices.exported must be true for new exports");
}

// ──────────────────────────────────────────────────────────────────────
// Backward compatibility: pre-Phase-9 metadata shape
// ──────────────────────────────────────────────────────────────────────

// ──────────────────────────────────────────────────────────────────────
// Experiment identifier (Round 7 / D13)
// ──────────────────────────────────────────────────────────────────────

#[test]
fn test_experiment_field_is_present() {
    // Round 7 / D13: metadata must carry the experiment identifier so a
    // downstream ledger (EXPERIMENT_INDEX.md) can link without path parsing.
    let meta = canonical_metadata();
    let parsed = write_and_parse(&meta);
    assert_eq!(
        parsed["experiment"],
        "basic_nvda_60s_phase9",
        "experiment identifier must be present in metadata"
    );
}

// ──────────────────────────────────────────────────────────────────────
// Backward compatibility: pre-Phase-9 metadata shape
// ──────────────────────────────────────────────────────────────────────

#[test]
fn test_backward_compat_minimal_metadata_still_conforms() {
    // Pre-Phase-9 metadata files lack `forward_prices` entirely. The core
    // required fields must still pass. Builders that don't call
    // `.forward_prices()` produce JSON without it via #[serde(skip_serializing_if)].
    let meta = ExportMetadata::builder()
        .day("2025-01-01")
        .n_sequences(100)
        .window_size(20)
        .horizons(vec![1, 5])
        .bin_size_seconds(60)
        // Do NOT call .forward_prices() — simulates pre-Phase-9 export
        .build()
        .expect("minimal builder");

    assert!(
        meta.forward_prices.is_none(),
        "Unset forward_prices must be Option::None"
    );
    let parsed = write_and_parse(&meta);
    assert!(
        parsed.get("forward_prices").is_none(),
        "Unset forward_prices must be skipped in JSON output"
    );
    // BUT: core required fields must still be present
    for field in OFFEX_REQUIRED_FIELDS {
        assert!(
            parsed.get(field).is_some(),
            "Core required field '{field}' missing even without forward_prices"
        );
    }
}
