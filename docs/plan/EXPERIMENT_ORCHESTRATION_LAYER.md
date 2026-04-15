# Experiment Orchestration Layer — Phase 10 Design Specification

**Status**: Draft v1 — awaiting user review before implementation.
**Author**: Validation-driven synthesis across 9 rounds (~20 agent reports).
**Date**: 2026-04-15.
**Canonical location**: `basic-quote-processor/docs/plan/EXPERIMENT_ORCHESTRATION_LAYER.md`.
**Cross-reference**: `plan/EXPERIMENT_ORCHESTRATION_LAYER.md` (parent monorepo pointer).
**Supersedes**: Phase 10 narrow scope from `plans/twinkling-snacking-spark.md` § "Deferred to Future Phases" (build.rs only).
**Does NOT supersede**: `plans/gentle-brewing-quail.md` (Phase 3 Config Composition). This document builds ON that work and preserves its invariants.
**Implementation trigger**: Phase 11 — begins only after user review + approval of this design.

---

## Document Conventions

- **Normative sections** (MUST / MUST NOT / SHALL): §2 Design Principles, §3 Envelope Schema, §5 Ledger Storage, §6 Fingerprint Alignment, §7 Ingestion Pipeline, §14 Phase 3 Invariants. Other sections are informative.
- **Code examples**: SQLite DDL, Python type hints, JSON Schema Draft-07, TOML.
- **Cross-references**: `§N` for sections, `§N.M` for subsections, filenames with line numbers for citations.
- **Terminology**: "orchestrator" = `hft-ops`; "producer" = any pipeline stage that emits data (BQP, MBO extractor, trainer, backtester, evaluator, profilers); "consumer" = any tool that reads from the ledger.

---

## §0 — Executive Summary

### The Problem

The current experiment tracking infrastructure cannot scale to the user's stated long-term goal of "thousands of experiments using different models and feature sets configurations, in an empirically statistical precise traceable, trackable and monitorable way". Concrete evidence:

1. **100% retroactive ledger**: 34/34 entries in `hft-ops/ledger/records/*.json` have the `_retro` suffix. No production `hft-ops run` has ever successfully produced a ledger entry end-to-end. Root cause: pre-Phase-3 manifests were structurally invalid (see `gentle-brewing-quail.md:8`).
2. **Three disjoint registries**: `hft-ops/ledger/` (34 retro), `lob-model-trainer/outputs/experiments/_registry/`, and `lob-backtester/outputs/backtests/`. None is authoritative. Three markdown files (`EXPERIMENT_INDEX.md`, `BACKTEST_INDEX.md`, `EXPORT_INDEX.md`) are hand-maintained.
3. **Backtest metrics never captured into records**: `BacktestRunner.validate_outputs()` returns `[]` (`hft-ops/src/hft_ops/stages/backtesting.py:151-156`). Every hft-ops-produced record has `backtest_metrics: {}`.
4. **Phase 9 metadata unconsumed**: BQP now emits `config_hash`, `source_file`, `experiment`, `forward_prices`, `feature_groups_enabled`, `classification_config`. Zero readers in `hft-ops`.
5. **Query language impoverished**: `filter()` (`hft-ops/src/hft_ops/ledger/ledger.py:120-176`) supports ~8 hardcoded fields (`min_f1`, `min_accuracy`). No metric-agnostic filter, no cohort queries, no lineage traversal.
6. **Markdown approaching scale wall**: `EXPERIMENT_INDEX.md` is ~3,000 lines with 35 experiments. Hand-writing is O(1) per experiment; at 1000 experiments the file breaks grep/search/diff.

### The Solution

A **SQLite + Parquet hybrid ledger** (Ledger v2) with a **contract-first Experiment Envelope** ingestion protocol and an auto-regenerated markdown layer. Primary key is the **Phase 3 `experiment_fingerprint`** (not a newly-invented hash). Storage is a single file on disk. Query is SQL + pandas DataFrame. Cross-repo provenance is first-class. Gates are ordered and conditional. Anti-patterns from MLflow/W&B/Sacred explicitly rejected.

### Success Criteria (Phase 11 completion)

1. **Every new `hft-ops run` produces a non-retro ledger record** (closes the 100% retro anomaly).
2. **Existing 34 records migrate losslessly** — fingerprints preserved, not recomputed.
3. **Cross-producer envelope schema validated** by `hft-contracts` (SSoT) and enforced at ingest.
4. **Query pattern "show all TLOB experiments with test IC > 0.05 at H=10"** is a single CLI command returning a pandas DataFrame.
5. **Auto-regenerated markdown** stays within 2 seconds of SQLite state (post-ingest hook).
6. **At 1000 experiments** (projected 2 years out): all queries < 500ms on laptop SSD.
7. **At least 40 new `hft-ops` tests** documenting ingest, query, migration, failure recovery.
8. **Failure modes §12.1-§12.10 all recoverable** via documented CLI commands.
9. **Cross-repo lineage `hft-ops ledger lineage <id>`** walks BQP + MBO + trainer + backtester chain in one query.
10. **Contract compliance**: envelope schema lives in `contracts/pipeline_contract.toml`, codegen-generated to `hft_contracts.orchestration.Envelope` Pydantic model.

### Scope Decisions (from 9 validation rounds)

- **(D1) SQLite now**, overriding `gentle-brewing-quail.md`'s "defer past 500 records" note. Rationale: user's stated "thousands of experiments" goal outweighs the quail plan's pre-Phase-9 threshold. Phase 3's `_base:` fingerprint fix makes the infrastructure ready.
- **(D2) Contract-first**: envelope schema in `pipeline_contract.toml`, codegen to Python Pydantic (matches existing `FeatureIndex`/`LabelContract` pattern).
- **(D3) Primary key on `experiment_fingerprint` alone** (Round 10 refinement). The Phase 3 fingerprint (`dedup.py:284-391`) already embeds symbol (via extraction config components) and asset class (via contract_version + extraction schema). A composite `(fp, symbol, asset_class)` PK was evaluated in Rounds 5-9 and rejected in Round 10 as defensive over-engineering: single-symbol runs produce different fingerprints per symbol (no collision risk); multi-symbol pooled runs go in one row with `symbols_json` JSON array. Cross-symbol cohort queries use a derived `cohort_hash` column (excludes symbol + data_manifest). Multi-asset options support is trivial — `asset_class` remains a column with a CHECK constraint.
- **(D4) γ-full scope**: envelope + MetricKey + SQLite+Parquet + Sweep v2 + failure recovery + cross-repo lineage.
- **(D5) Primary location** in `basic-quote-processor/docs/plan/`; pointer from parent monorepo `plan/`.
- **(D6) Defer to Phase 12+**: knowledge-synthesis cohort reports, Optuna integration, streaming mode, web UI, feature-set resolver (Phase 4 of quail plan).

---

## §1 — Context + Non-Goals

### §1.1 Current State (post-Phase-3, 2026-04-15)

**Ledger**:
- `hft-ops/ledger/index.json` (single file) + `hft-ops/ledger/records/*.json` (34 files).
- All 34 entries are retroactive (`_retro` suffix). Zero production pipeline records.
- Schema: `ExperimentRecord` dataclass at `hft-ops/src/hft_ops/ledger/experiment_record.py:56-134` with **25 dataclass fields** (verified via `grep -c '^    [a-z_]*:' experiment_record.py`): `experiment_id, name, manifest_path, fingerprint, provenance, contract_version, extraction_config, training_config, backtest_params, training_metrics, backtest_metrics, dataset_health, tags, hypothesis, description, notes, created_at, duration_seconds, status, stages_completed, sweep_id, axis_values, record_type, sub_records, parent_experiment_id`. `provenance` is a nested `Provenance` dataclass (`hft-ops/src/hft_ops/provenance/lineage.py:195-245`) with fields `{git: GitInfo, config_hashes: dict, data_dir_hash, contract_version, timestamp_utc, retroactive, schema_version}`.

**Fingerprinting**:
- Phase 3 shipped §3.3b: `compute_fingerprint()` in `hft-ops/src/hft_ops/ledger/dedup.py:284-391` now resolves `_base:` inheritance before hashing via `resolve_inheritance()` at `lob-model-trainer/src/lobtrainer/config/merge.py:85-182` (loaded lazily by `_load_trainer_config_resolved()` at `dedup.py:117-195`, which walks `_base:` chains and applies `deep_merge()` before serialization).
- Algorithm: `sha256(json.dumps({extraction, training, backtest, data_manifest, contract_version}, sort_keys=True, default=str))` (verified at `dedup.py:390`).
- Excludes: `{name, description, tags, version, output_dir, log_level, verbose, experiment}` and entire `stages.validation` section (validation is observation, not treatment — see `_extract_fingerprint_fields()` at `dedup.py:255-282` for exclusion-set embedding + `dedup.py:364-368` for validation-stage-excluded comment).

**Config Composition** (Phase 3 Batch 1 in progress):
- `lob-model-trainer/src/lobtrainer/config/merge.py` (v2, hand-rolled, ~205 LOC) replaces `OmegaConf` (explicitly rejected after adversarial review).
- Supports `_base: str | list[str]`, left-to-right merge, child overrides.
- `_partial: true` sentinel for intermediate bases.
- **21 base files created across 4 categories** (Phase 3.5 progressive migration in-flight, initial 4-base commit has grown): `datasets/` (8 bases including `nvda_e5_60s.yaml`), `labels/` (4 bases including `regression_huber.yaml`), `models/` (5 bases including `tlob_compact_regression.yaml`), `train/` (4 bases including `regression_default.yaml`). Memory ledger confirms Phase 3.5 ongoing; 36 legacy configs migrate across 4 batches.
- 36 configs to migrate across 4 batches (progressive — not blocking).

**Producers** (5 primary, 1 future):
- `basic-quote-processor` (Phase 9 shipped: `forward_prices`, `config_hash`, `experiment`, honest `normalization`, provenance chain; 471 tests; standalone repo at github.com/nagarx/basic-quote-processor.git commit 97badff).
- `feature-extractor-MBO-LOB` (9-crate workspace, 692 tests, 148-feature schema).
- `lob-model-trainer` (11 model architectures, 807 tests, T9-T15 complete).
- `lob-backtester` (IBKR-calibrated costs, 338 tests, 8 backtest rounds documented).
- `hft-feature-evaluator` (5-path framework, 225 tests, 4-tier classification).
- **Future (≤2 years)**: OPRA feature/signal extractor (new repo, user-stated).

**Consumers**:
- `hft-ops` (orchestrator, 158 tests, 34 retroactive records).
- Human-maintained markdown: `EXPERIMENT_INDEX.md` (1,821 lines, 35 experiments), `BACKTEST_INDEX.md`, `EXPORT_INDEX.md`.

**Contracts**:
- SSoT: `contracts/pipeline_contract.toml` (auto-generates `hft-contracts/_generated.py` and validates Rust constants via `contracts/verify_rust_constants.py`).

**Test coverage** (reference points):
- hft-ops 158, trainer 176, hft-contracts 165, hft-metrics 298, hft-feature-evaluator 225, BQP 471, MBO extractor 692.

### §1.2 Stated Futures (must accommodate without rework)

Per user directive and validation agents:

1. **OPRA pipeline** (future repo). New producer. Options data: implied volatility, greeks, days-to-expiry, strike distance. Different labels (0DTE option returns). Different asset class.
2. **Multi-symbol** (Phase 12 per `twinkling-snacking-spark.md`). 15+ symbols × 4 bin sizes × 3 normalizations × N horizons = 1800+ variant runs.
3. **Multi-asset class**: equities + options + futures. Distinct metric families.
4. **Alternative labels**: triple-barrier (3 horizons), magnitude-ranked (distribution), regime-conditioned (tags). Scalar `horizon: int` is insufficient.
5. **New sampling strategies**: volume-based, event-based, tick-based (beyond time-based).
6. **New model architectures**: XGBoost (no epochs), GNN (graph state), Bayesian (posterior samples), ensemble (multi-model dispatch).
7. **Mandatory gates** per `hft-rules.md §13`: signal-quality (IC > 0.05), cost, baseline, evaluation-tool, optimization-execution-alignment. Ordered, conditional.
8. **Cross-repo provenance**: BQP + MBO + trainer + backtester + future-OPRA chain.
9. **Scale horizon**: 1000+ experiments in 2-3 years, 10k+ in 5 years.
10. **Streaming** (Phase 13 per `twinkling-snacking-spark.md`): live inference. Opt-in; keep batch semantics pure.

### §1.3 Non-Goals (v1)

Explicit exclusions to prevent scope creep:

- **Cloud sync / multi-machine** — single-researcher single-machine workflow. NFS unsupported (startup warns).
- **Web UI** — CLI + pandas + auto-generated markdown are sufficient until daily-use justifies otherwise.
- **Multi-user / ACL** — single researcher.
- **GraphQL / REST API** — consumers are CLI and Python notebooks only.
- **Real-time dashboards** — post-hoc analysis. Streaming is Phase 13.
- **Hyperparameter-optimization algorithms (Bayesian, TPE, HyperBand)** — grid + ablation axes are sufficient for Phase 11. Optuna integration is Phase 12.x optional.
- **Cross-commit diffing of configs** — `git diff` + `jq` are sufficient.
- **Automatic feature-set registry** — Phase 4 of `gentle-brewing-quail.md` owns this. Phase 10 leaves a placeholder that Phase 4 fills.
- **SQLite → PostgreSQL migration** — defer until 100k+ experiments or team-scale.
- **Parquet → Arrow Flight** — defer until single-file Parquet is a bottleneck.
- **Full-text search on notes** — `grep` on rendered markdown is sufficient at <1000 experiments.

### §1.4 Out-of-Scope for Phase 10 design doc; IN-scope for Phase 11+ implementation phases

- Backfill of the 34 retroactive records (Phase 11 executes).
- Switch-over of producers from legacy JSON writers (Phase 11 executes).
- Deprecation of legacy `hft-ops/ledger/records/*.json` after migration (Phase 11 executes).
- Knowledge-synthesis: auto-regenerated cohort reports (Phase 13).
- Sweep runner: parallel subprocess pool (Phase 12).

---

## §2 — Design Principles (Normative)

The following principles are **load-bearing** and MUST be preserved in all implementation work:

### §2.1 Content-Addressed Identity

**Principle**: Experiments are identified by their content, not by arrival order.

**Rationale**: Same config re-run must produce the same identity (enables dedup, reproducibility, skip-if-exists). Different configs must produce different identities. No UUIDs, no auto-increment integers, no random IDs.

**Implementation**: `experiment_fingerprint` = Phase 3's existing hash (`dedup.py:284-391`). Phase 10 REUSES it. Does not introduce a competing hash.

**Anti-pattern rejected**: MLflow/W&B auto-generated run IDs. Sacred's source-code hashing (whitespace-sensitive).

### §2.2 Append-Only Audit Log as Source of Truth

**Principle**: `hft-ops/ledger/records/*.json` is the authoritative store. SQLite is a rebuildable query cache.

**Rationale**: Git-friendly, easy to inspect with `jq`, survives SQLite corruption, rebuildable in seconds. Matches existing pattern (`ledger.py:53-62` rebuilds index from records).

**Implementation**: Ingest writes JSON record FIRST, then Parquet, then SQLite. If SQLite corrupt: `hft-ops ledger rebuild` replays records.

**Anti-pattern rejected**: Aim's opaque binary storage. DVC's git-ref-per-experiment (scales poorly).

### §2.3 Separation of Identity / Storage / Narrative

**Principle**: Three orthogonal concerns — identity (fingerprint), machine-readable state (SQLite + Parquet), human narrative (markdown notes) — must never be conflated.

**Rationale**: Each evolves independently. Storage migrations don't break narrative. Narrative edits don't change identity. Identity changes don't force full markdown regeneration.

**Implementation**:
- Identity: `experiment_fingerprint` column (immutable).
- Storage: SQLite tables (immutable rows; mutation via new rows).
- Narrative: `notes` table (append-only) + auto-regenerated markdown views.

**Anti-pattern rejected**: MLflow's confusion of `params` (identity) with `tags` (narrative). W&B's tight coupling.

### §2.4 Subprocess Isolation Invariant

**Principle**: `hft-ops` MUST NOT Python-import runner modules. All runners invoked as subprocesses.

**Rationale**: Prevents import-chain fragility; runners evolve independently; language-agnostic (Rust producers work the same as Python).

**Implementation**: Preserved from current state (`hft-ops/CODEBASE.md §1 invariant #1`). Two soft exceptions ALLOWED:
- `lobtrainer.config.merge.resolve_inheritance` (merge.py:85-182) — lazy import via `_load_trainer_merge_module()` at `dedup.py:73-114`; optional; fallback to raw YAML if missing (see the `return None` path at `dedup.py:95-99` + call-site at `dedup.py:190-194`).
- `hft_evaluator.fast_gate.run_fast_gate` — library import, explicit Phase 2b decision (`stages/validation.py:1-32`).

No new soft exceptions without updating this principle.

### §2.5 Contract-First Schema

**Principle**: Every cross-module structural contract lives in `contracts/pipeline_contract.toml` (SSoT) with codegen to consuming languages.

**Rationale**: Eliminates schema drift. Matches existing `FeatureIndex`/`LabelContract`/`ValidationConfig` patterns.

**Implementation**:
- Envelope schema defined in `pipeline_contract.toml` under `[orchestration.envelope]`.
- MetricKey enum defined in `pipeline_contract.toml` under `[orchestration.metric_keys]`.
- Python codegen: `contracts/generate_python_contract.py` produces `hft_contracts/orchestration.py` (Pydantic model + enum).
- Rust side: hand-maintained struct in `hft_contracts_rs::orchestration` crate — verified by TWO complementary checks (see §2.5.1).

**Anti-pattern rejected**: Independent schema definitions per module. MLflow's ad-hoc params.

#### §2.5.1 Rust `hft_contracts_rs` crate — location, layout, verification (Round 13)

**Location**: co-located with the Python `hft-contracts` package at `github.com/nagarx/hft-contracts.git`. Single-package Rust crate at the repo root, matching the `hft-statistics` precedent (single `[package]` root, no Cargo workspace — verified simpler and already-proven pattern).

**Repository layout**:

```
hft-contracts/                    (github.com/nagarx/hft-contracts.git)
├── Cargo.toml                    [package] name = "hft_contracts_rs"
│                                 [lib]  name = "hft_contracts_rs"
│                                        path = "rust/lib.rs"
├── pyproject.toml                Python package (unchanged from today)
├── src/hft_contracts/            Python source (unchanged — existing 165+ tests pass)
│   ├── __init__.py
│   ├── _generated.py             Python codegen output (existing)
│   └── orchestration/            NEW — Phase 11 Week 1
│       ├── __init__.py
│       ├── envelope.py           Pydantic model
│       ├── metric_keys.py        Enum + TOML loader
│       ├── gate_keys.py          Enum + TOML loader
│       ├── canonical_json.py     Shared serializer (§3.3.6)
│       └── upstream.py           .envelope-ref reader (§7.1.1)
├── rust/                         NEW — Rust source at non-conflicting path
│   ├── lib.rs
│   └── orchestration/
│       ├── mod.rs
│       ├── envelope.rs           Hand-written struct + SCHEMA_HASH const
│       ├── metric_keys.rs
│       ├── gate_keys.rs
│       ├── canonical_json.rs
│       ├── upstream.rs
│       └── fingerprint.rs        compute_fingerprint_export for Rust producers
└── tests/
    └── fixtures/
        ├── canonical_json/       Shared Python + Rust fixtures (§18.1b)
        └── envelopes/golden/     Canonical golden envelopes (§18.2)
```

**Rationale for this layout**: `[lib] path = "rust/lib.rs"` is standard Cargo syntax allowing Rust source outside `src/`. Keeps Python import path `src/hft_contracts/*` unchanged (no existing consumer breakage). Rust consumers depend on the root `[package]` via `hft_contracts_rs = { git = "https://github.com/nagarx/hft-contracts.git", branch = "main" }` — identical pattern to `hft-statistics`. No workspace complexity.

**Verification protocol — TWO complementary checks, both required in CI**:

1. **SCHEMA_HASH verification** (`contracts/verify_rust_envelope_schema.py`):
   - Computes `sha256(canonical_json_of_envelope_schema_from_toml)`.
   - Rust source at `rust/orchestration/envelope.rs` carries `pub const SCHEMA_HASH: &str = "<hex>";`.
   - CI asserts the TOML-derived hash equals the Rust-side const. **Catches TOML drift** (TOML edited without Rust update).

2. **Golden serialization test** (`rust/tests/envelope_golden.rs`):
   - Loads `tests/fixtures/envelopes/golden/01_complete.json` (canonical form, produced by Python).
   - Deserializes into Rust `Envelope` struct; re-serializes via `canonical_json_blob` (Rust mirror of `hft_contracts.canonical_hash.canonical_json_blob`).
   - Asserts byte-for-byte equality against the original fixture.
   - **Catches Rust struct drift** (Rust edited without TOML/fixture update) — SCHEMA_HASH alone cannot detect this.

Both checks are required because they catch DIFFERENT drift directions. See §18.2 for test specifications.

**Consumption patterns**:
- BQP (standalone repo): `hft_contracts_rs = { git = "https://github.com/nagarx/hft-contracts.git", branch = "main" }` (Phase 11.5 addition — BQP is NOT touched in Phase 11).
- MBO extractor workspace: same git dep + `.cargo/config.toml` path override for local dev (matches `hft-statistics` pattern documented in root CLAUDE.md).
- Future OPRA feature-extractor: same pattern when that repo is created.

**Python+Rust coexistence at repo root**: having both `Cargo.toml` and `pyproject.toml` at the same directory is legal and non-conflicting — the two tool ecosystems read disjoint config files. Python build (`pip install -e .`) ignores `Cargo.toml`; Rust build (`cargo build`) ignores `pyproject.toml`. Single git repo, single contract-change commit updates both.

### §2.6 Validate at Boundaries, Trust Internal Code

**Principle**: Envelope validation runs ONCE at ingest. Internal SQLite reads/writes assume valid state.

**Rationale**: `hft-contracts` pattern. Validation in hot paths is waste. Invalid envelopes go to quarantine; valid envelopes become authoritative.

**Implementation**: `hft_contracts.orchestration.validate_envelope(envelope: dict) -> ValidatedEnvelope` runs at ingest. Post-insert, all code assumes valid.

### §2.7 Atomic, Idempotent, Deterministic Operations

**Principle**: Every write is atomic (tmp+rename or SQL transaction). Every operation is idempotent (re-runnable without corruption). Operations on the same input produce identical output.

**Rationale**: Survives `kill -9`, power loss, double-invocation, resume-after-failure.

**Implementation**:
- Producer envelopes: content-addressable filename (`sha256(content).json`) → tmp+rename.
- Ingest: SQL BEGIN/COMMIT with foreign keys. Parquet-first, then SQLite insert, then COMMIT.
- Migration: `INSERT ... ON CONFLICT DO NOTHING`.
- Fingerprint: deterministic canonicalization.

### §2.8 Hard-Fail on Contract Violation, Warn-Only on Statistical Signal

**Principle**: Schema violation = hard fail. Gate failure (IC < 0.05) = warn-by-default (configurable to fail).

**Rationale**: Contracts are invariants; gates are statistical observations. `stages/validation.py:26-31` documents this precedent.

**Implementation**: `validate_envelope()` raises `ContractError` on schema mismatch. Gate outcomes logged as boolean in `gate_results` table; user's `on_fail` policy per-gate determines whether pipeline continues.

### §2.9 Human-Readable Projections Are Derived, Not Primary

**Principle**: `EXPERIMENT_INDEX.md`, `BACKTEST_INDEX.md`, `EXPORT_INDEX.md` are auto-generated views. Never hand-edited in v2+.

**Rationale**: Narrative lives in `notes` table. Projections stay consistent with state. No drift.

**Implementation**: `hft-ops ledger render-indexes` command. Header comment `<!-- GENERATED — DO NOT EDIT -->` on output files. Post-ingest hook re-renders.

**Migration**: salient prose from existing `EXPERIMENT_INDEX.md` is extracted one-time into the `notes` table (§13.2).

### §2.10 Simpler > Cleverer

**Principle**: Prefer stdlib + SQLite + 60-LOC helpers over frameworks.

**Rationale**: Matches Phase 3's pivot from OmegaConf to hand-rolled merge. User explicitly values simplicity (5-year maintenance horizon; single researcher).

**Implementation**: No Hydra, no Kedro, no Metaflow, no MLflow-server, no Optuna in v1. Pandas + SQLite + Pydantic. Optuna opt-in in Phase 12.x if HPO sweeps justify it.

---

## §3 — Envelope Schema (Contract-First)

### §3.1 The Experiment Envelope

Every producer writes a canonical JSON envelope to `hft-ops/ledger/inbox/{content_hash}.json`. The envelope is the ONLY cross-module contract. Producers are free to evolve internal tooling; the envelope must remain stable (versioned).

**Canonical location of the schema**: `contracts/pipeline_contract.toml` under `[orchestration.envelope]`. Codegen produces `hft_contracts/orchestration.py` with a Pydantic `Envelope` model.

### §3.2 JSON Schema (Draft-07)

```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "$id": "https://hft-pipeline/schemas/envelope-v1.json",
  "title": "HFT Pipeline Experiment Envelope",
  "type": "object",
  "required": [
    "envelope_version",
    "producer",
    "producer_version",
    "record_type",
    "experiment_id",
    "experiment_fingerprint",
    "created_at",
    "status",
    "envelope_schema_version",
    "pipeline_contract_version",
    "asset_class",
    "data_source_type",
    "feature_schema_ref",
    "symbol"
  ],
  "additionalProperties": false,
  "properties": {
    "envelope_version": {
      "type": "integer",
      "const": 1,
      "description": "First field read by ingest dispatcher. Mandatory for forward compatibility."
    },
    "envelope_schema_version": {
      "type": "string",
      "const": "1.0.0",
      "description": "Semantic version of the envelope schema itself."
    },
    "pipeline_contract_version": {
      "type": "string",
      "pattern": "^\\d+\\.\\d+(\\.\\d+)?$",
      "description": "The top-level `[contract].schema_version` from `contracts/pipeline_contract.toml` at producer runtime. Example values: '2.2', '2.3', '3.0'. This is the PIPELINE-WIDE contract version, not a feature-schema-specific version (those live in `feature_schema_ref`)."
    },
    "producer": {
      "type": "string",
      "enum": [
        "basic-quote-processor",
        "feature-extractor-MBO-LOB",
        "lob-model-trainer",
        "lob-backtester",
        "hft-feature-evaluator",
        "hft-ops",
        "MBO-LOB-reconstructor",
        "mbo-statistical-profiler",
        "opra-statistical-profiler",
        "opra-feature-extractor",
        "legacy-migrator"
      ],
      "description": "Enum extensible; new producers require schema version bump. Special value 'legacy-migrator' is used by §13.1 Step 2 to backfill the 34 pre-Phase-10 retroactive records; envelopes with this producer are exempted from the §14.8 and §14.9 integrity gates (§13.1b legacy-migrator exemption)."
    },
    "producer_version": {
      "type": "string",
      "description": "Cargo pkg version or pip version. Required; 'unknown' allowed for retroactive backfill."
    },
    "record_type": {
      "type": "string",
      "enum": [
        "export",
        "training",
        "analysis",
        "calibration",
        "backtest",
        "evaluation",
        "sweep_aggregate"
      ],
      "description": "Matches hft-ops/src/hft_ops/ledger/experiment_record.py:24-54 plus new 'export' value for BQP/extractor producers."
    },
    "experiment_id": {
      "type": "string",
      "pattern": "^[a-zA-Z0-9_\\-]{1,200}$",
      "description": "Human-readable identifier: {name}_{YYYYMMDDTHHMMSS}_{fingerprint[:8]}. Unique per (producer, record_type)."
    },
    "experiment_fingerprint": {
      "type": "string",
      "pattern": "^[a-f0-9]{64}$",
      "description": "Phase 3 hash: sha256(json.dumps({extraction, training, backtest, data_manifest, contract_version}, sort_keys=True, default=str)). See §6."
    },
    "fingerprint_version": {
      "type": "integer",
      "default": 1,
      "description": "Algorithm version. Increments only on canonicalization changes (rare). Enables multi-version lookup."
    },
    "created_at": {
      "type": "string",
      "format": "date-time",
      "pattern": "^\\d{4}-\\d{2}-\\d{2}T\\d{2}:\\d{2}:\\d{2}(\\.\\d+)?Z$",
      "description": "ISO 8601 UTC with Z suffix (NO offset like '+00:00'). Producer wall-clock start time. Round 13 B1: pattern enforces Z-only to prevent local-time leaks (e.g., Python datetime.now().isoformat() emits +00:00 which reads correctly but partitions Parquet yyyy_mm incorrectly near midnight UTC boundary)."
    },
    "finalized_at": {
      "type": ["string", "null"],
      "format": "date-time",
      "pattern": "^\\d{4}-\\d{2}-\\d{2}T\\d{2}:\\d{2}:\\d{2}(\\.\\d+)?Z$",
      "description": "ISO 8601 UTC with Z suffix. Producer wall-clock completion. NULL for status=running."
    },
    "heartbeat_at": {
      "type": ["string", "null"],
      "format": "date-time",
      "pattern": "^\\d{4}-\\d{2}-\\d{2}T\\d{2}:\\d{2}:\\d{2}(\\.\\d+)?Z$",
      "description": "ISO 8601 UTC with Z suffix. Updated periodically by long-running producers (streaming future). Ignored for batch."
    },
    "status": {
      "type": "string",
      "enum": ["pending", "running", "completed", "failed", "partial", "cancelled"],
      "description": "Record lifecycle. 'partial' = some days/axes failed but run finished; 'completed' = zero failures."
    },
    "wall_clock_ms": {
      "type": ["integer", "null"],
      "minimum": 0,
      "description": "Milliseconds from created_at to finalized_at."
    },
    "asset_class": {
      "type": "string",
      "enum": ["equity", "option", "future", "fx", "crypto", "synthetic"],
      "description": "Load-bearing for polymorphic metric schemas. Options != equity != futures metrics."
    },
    "data_source_type": {
      "type": "string",
      "enum": ["equity_lob_mbo", "equity_off_exchange_trf", "opra_options", "futures_cme", "synthetic"],
      "description": "Granular data source. Same asset_class can have multiple data_source_types."
    },
    "feature_schema_ref": {
      "type": "string",
      "description": "Registered feature schema identifier: 'equity_v2.2' (148 MBO features), 'off_exchange_1.0' (34 features), 'opra_v1' (future). Resolvable via hft_contracts.feature_schema_registry."
    },
    "symbol": {
      "type": "string",
      "pattern": "^[A-Z0-9._-]{1,16}$",
      "description": "Primary symbol. For multi-symbol runs, use 'symbols' array (takes precedence)."
    },
    "symbols": {
      "type": "array",
      "items": {"type": "string", "pattern": "^[A-Z0-9._-]{1,16}$"},
      "default": [],
      "description": "Multi-symbol runs. When non-empty, takes precedence over 'symbol' (which is set to first entry)."
    },
    "dataset": {
      "type": ["string", "null"],
      "description": "Logical dataset name (e.g., 'basic_nvda_60s', 'nvda_xnas_128feat_regression')."
    },
    "model_type": {
      "type": ["string", "null"],
      "description": "NULL for exports. e.g., 'tlob', 'hmhp', 'temporal_ridge', 'xgboost'."
    },
    "model_family": {
      "type": ["string", "null"],
      "enum": ["tree", "neural", "bayesian", "baseline", "linear", null],
      "description": "Cohort dimension for comparison across architectures."
    },
    "task": {
      "type": ["string", "null"],
      "enum": ["classification", "regression", "backtest", "analysis", null]
    },
    "sampling_strategy": {
      "type": ["string", "null"],
      "enum": ["time_based", "event_based", "volume_based", "tick_based", null],
      "description": "Explicit column for cohort queries. Avoids parsing config JSON."
    },
    "sampling_config": {
      "type": ["object", "null"],
      "description": "Per-strategy params (bin_size_seconds, volume_bar_shares, etc.)."
    },
    "label_family": {
      "type": ["string", "null"],
      "enum": [
        "point_return",
        "smoothed_tlob",
        "triple_barrier",
        "magnitude_ranked",
        "regime_conditioned",
        "opportunity",
        null
      ]
    },
    "primary_horizon": {
      "type": ["integer", "null"],
      "minimum": 0,
      "description": "Single-horizon identifier for indexing. NULL for multi-barrier labels."
    },
    "n_horizons": {
      "type": "integer",
      "minimum": 0,
      "default": 0,
      "description": "Size of horizons array (or 0 for non-horizon-based labels)."
    },
    "horizons": {
      "type": "array",
      "items": {"type": "integer", "minimum": 0},
      "default": [],
      "description": "Always an array, even length 1. Length matches n_horizons."
    },
    "label_spec": {
      "type": ["object", "null"],
      "description": "Full label specification. For triple_barrier: {theta_profit, theta_stop, H_time}. For magnitude_ranked: {n_bins, boundaries[]}. Opaque JSON blob indexed only by label_family."
    },
    "config_source": {
      "type": ["string", "null"],
      "description": "Raw TOML/YAML/JSON config text. Embed for small configs (<8KB); use artifacts[].kind='config' for large."
    },
    "config_format": {
      "type": ["string", "null"],
      "enum": ["toml", "yaml", "json", null]
    },
    "sweep_id": {
      "type": ["string", "null"],
      "description": "Groups envelopes produced by one sweep run. Matches sweeps.sweep_id FK."
    },
    "axis_values": {
      "type": ["object", "null"],
      "description": "For sweep members: {axis_name: axis_label}. NULL for non-sweep runs.",
      "additionalProperties": {"type": "string"}
    },
    "parent_id": {
      "type": ["string", "null"],
      "description": "experiment_id of parent run (calibration/retry semantics). See §11 lineage."
    },
    "upstream_experiment_ids": {
      "type": "array",
      "items": {"type": "string"},
      "default": [],
      "description": "Explicit graph edges: backtester → trainer → export chain. Each ref resolvable via experiments table."
    },
    "metrics": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["family", "name", "value"],
        "properties": {
          "family": {"type": "string", "description": "MetricFamily enum: 'ic_screening'|'training_loss'|'backtest_pnl'|'greeks_regression'|..."},
          "name": {"type": "string", "description": "MetricKey enum value: 'IC_H10'|'DA_H10'|..."},
          "split": {"type": ["string", "null"], "enum": ["train", "val", "test", "cv", "full", null]},
          "horizon": {"type": ["integer", "null"]},
          "epoch": {"anyOf": [{"type": "integer"}, {"type": "string"}, {"type": "null"}], "description": "int | 'best' | 'final' | null (non-epoch training)"},
          "value": {"type": ["number", "null"]},
          "ci_low": {"type": ["number", "null"]},
          "ci_high": {"type": ["number", "null"]},
          "tag": {"type": ["string", "null"], "description": "e.g., class label for per-class F1, sub-model name for ensemble"}
        }
      }
    },
    "gates": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["gate_name", "status", "gate_order"],
        "properties": {
          "gate_name": {"type": "string", "pattern": "^[a-z][a-z0-9_]*$", "description": "GateKey enum value (underscore form — TOML bare-key compatible). Canonical names live in pipeline_contract.toml:[orchestration.gate_keys]: 'ic_gt_0_05' | 'cost_breakeven' | 'baseline_ridge' | 'baseline_persistence' | 'evaluation_tool_sanity' | 'optimization_execution_alignment' | 'label_exec_alignment_gt_0_5' | 'sign_flip_rate_lt_0_5' | 'bh_fdr_significant'. Unregistered names accepted with WARN (§4.3)."},
          "gate_order": {"type": "integer", "minimum": 0, "description": "Execution order. Gate N depends_on Gate N-1 implicitly unless depends_on set."},
          "depends_on_gate": {"type": ["string", "null"], "description": "Explicit dependency. Absent = implicit on gate_order-1."},
          "status": {"type": "string", "enum": ["pending", "skipped", "passed", "failed", "overridden"]},
          "threshold": {"type": ["number", "null"]},
          "observed": {"type": ["number", "null"]},
          "note": {"type": ["string", "null"]},
          "override_by": {"type": ["string", "null"], "description": "User or CI identifier"},
          "override_reason": {"type": ["string", "null"]},
          "override_at": {"type": ["string", "null"], "format": "date-time", "pattern": "^\\d{4}-\\d{2}-\\d{2}T\\d{2}:\\d{2}:\\d{2}(\\.\\d+)?Z$"}
        }
      }
    },
    "artifacts": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["kind", "path"],
        "properties": {
          "kind": {
            "type": "string",
            "description": "Open vocabulary. Well-known: 'checkpoint'|'config'|'manifest'|'sequences_dir'|'equity_curve'|'result'|'training_history'|'logs'|'metadata'|'normalization'|'forward_prices'|'signals_dir'|'posterior_samples'|'graph_state'|'feature_importance'|'other'."
          },
          "path": {"type": "string", "description": "Path relative to workspace root or absolute."},
          "bytes": {"type": ["integer", "null"], "minimum": 0},
          "sha256": {"type": ["string", "null"], "pattern": "^[a-f0-9]{64}$"}
        }
      }
    },
    "lineage": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["source_kind", "source_name", "source_repo"],
        "properties": {
          "source_kind": {
            "type": "string",
            "enum": ["raw_data", "export", "trainer_run", "backtest_run", "evaluator_run", "signals", "profiler_report", "other"]
          },
          "source_name": {"type": "string", "description": "Human-readable ref (e.g., 'basic_nvda_60s')."},
          "source_hash": {"type": ["string", "null"], "pattern": "^[a-f0-9]{64}$", "description": "Content hash if available. NULL for unhashable sources."},
          "source_repo": {"type": "string", "description": "Repo producing this lineage: 'basic-quote-processor'|'feature-extractor-MBO-LOB'|'lob-model-trainer'|'lob-backtester'|'databento-raw'."},
          "source_commit": {"type": ["string", "null"], "pattern": "^([a-f0-9]{7,40}|not_git_tracked)$", "description": "Git SHA (7-40 lowercase hex) OR the exact sentinel 'not_git_tracked' for monorepo files that live outside a git-tracked directory (matches current provenance/lineage.py:56)."},
          "source_commit_dirty": {"type": ["boolean", "null"], "description": "Working-tree dirty state. NULL if source_commit is sentinel."}
        }
      }
    },
    "bulk_parquet": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["kind", "path"],
        "properties": {
          "kind": {
            "type": "string",
            "enum": ["training_curve", "feature_ic", "predictions_summary", "per_day_export_stats", "equity_curve", "trade_log", "posterior_samples", "other"]
          },
          "path": {"type": "string", "description": "Content-addressable: hft-ops/ledger/metrics/{yyyy_mm}/{content_hash}.parquet"},
          "schema": {"type": ["array", "null"], "items": {"type": "string"}, "description": "Column-type pairs, e.g. ['epoch:int32', 'val_loss:f64']"},
          "partition_keys": {"type": "array", "items": {"type": "string"}, "default": ["experiment_fingerprint"]},
          "row_count": {"type": ["integer", "null"]},
          "sha256": {"type": ["string", "null"], "pattern": "^[a-f0-9]{64}$"},
          "bytes": {"type": ["integer", "null"]}
        }
      }
    },
    "export_stats": {
      "type": ["object", "null"],
      "description": "Only for record_type='export'. Populated by BQP/MBO extractor producers.",
      "properties": {
        "days_processed": {"type": "integer", "minimum": 0},
        "total_sequences": {"type": "integer", "minimum": 0},
        "sequence_length": {"type": "integer", "minimum": 1, "description": "T in [N,T,F] sequence shape."},
        "stride": {"type": "integer", "minimum": 1},
        "bin_size_seconds": {"type": ["integer", "null"], "minimum": 1},
        "normalization": {"type": "string", "description": "Rust-side strategy: 'none' (default per T15), 'market_structure_zscore' (deprecated), 'per_day_zscore' (deprecated)."},
        "n_features": {"type": "integer", "minimum": 1, "description": "F in [N,T,F] sequence shape. MUST be ≤ the total_count declared in the referenced feature_schema_ref registry entry of pipeline_contract.toml. Cross-field invariant (§7.6.4)."},
        "feature_layout": {"type": "string", "enum": ["grouped", "lobster"], "description": "How features are arranged in the last axis. 'grouped' = [ask_prices(10), ask_sizes(10), bid_prices(10), bid_sizes(10), derived...]; 'lobster' = interleaved [ask_p_L1, ask_s_L1, bid_p_L1, bid_s_L1, ...]. Contract-registered via pipeline_contract.toml [features.layout]."},
        "feature_indices_subset": {"type": ["array", "null"], "items": {"type": "integer", "minimum": 0}, "uniqueItems": true, "description": "If this export uses a strict SUBSET of the registered feature_schema_ref (e.g., MBO stable-98 instead of full-148), the sorted list of active feature indices. NULL = all features in schema used."},
        "splits": {
          "type": "object",
          "properties": {
            "train": {"$ref": "#/definitions/split_detail"},
            "val": {"$ref": "#/definitions/split_detail"},
            "test": {"$ref": "#/definitions/split_detail"}
          }
        }
      }
    },
    "training_info": {
      "type": ["object", "null"],
      "description": "Only for record_type='training'. Trainer-specific.",
      "properties": {
        "best_epoch": {"type": ["integer", "null"]},
        "total_epochs": {"type": "integer"},
        "model_params": {"type": "integer"},
        "num_train_samples": {"type": "integer"},
        "num_val_samples": {"type": "integer"},
        "num_test_samples": {"type": "integer"},
        "training_time_seconds": {"type": "number"},
        "normalization_method": {
          "type": ["string", "null"],
          "enum": ["none", "hybrid", "global_zscore", "per_feature_minmax", "market_structure_zscore", null],
          "description": "Python-side normalization method applied by the trainer. 'none' means raw f64 features were consumed; 'hybrid'/'global_zscore'/'per_feature_minmax' are the three production strategies in lob-model-trainer data/normalization.py. Legacy 'market_structure_zscore' is Rust-side, kept for pre-T15 records. See root CLAUDE.md §T15."
        },
        "normalization_stats_sha256": {
          "type": ["string", "null"],
          "pattern": "^[a-f0-9]{64}$",
          "description": "SHA-256 of the canonical JSON representation of the fitted normalization statistics (means, stds or mins/maxes, per feature). NULL iff normalization_method == 'none'. Critical invariant: backtester's signal_provenance.normalization_stats_sha256 MUST equal this value (§14.8 / §7.6.1 rule 10). Guards against silent train/inference divergence under T15 'Raw Rust, Variable Python'."
        },
        "normalization_source_split": {
          "type": ["string", "null"],
          "enum": ["train", "all", "cv_folds", null],
          "description": "Which split the normalization stats were computed FROM. Default 'train' (no leakage from val/test). 'all' is a known-leakage mode (research-only); 'cv_folds' means per-fold refit under CVTrainer (§T11)."
        }
      }
    },
    "strategy_info": {
      "type": ["object", "null"],
      "description": "Only for record_type='backtest'.",
      "properties": {
        "n_entries": {"type": "integer"},
        "n_gate_pass": {"type": "integer"},
        "n_gate_fail": {"type": "integer"},
        "trade_rate": {"type": "number"},
        "holding_policy": {"type": "string"},
        "exit_reasons": {"type": "object", "additionalProperties": {"type": "integer"}}
      }
    },
    "signal_provenance": {
      "type": ["object", "null"],
      "description": "Only for record_type='backtest'. Mirrors the signal_metadata.json emitted by the trainer (see CLAUDE.md Trainer → Backtester signal export). Purpose: lets the ingester verify that the backtester loaded the same signal the trainer produced, under the same normalization.",
      "properties": {
        "trainer_fingerprint": {
          "type": "string",
          "pattern": "^[a-f0-9]{64}$",
          "description": "The experiment_fingerprint of the upstream trainer run. MUST equal exactly ONE of upstream_experiment_ids (resolved via fingerprint_history table). §7.6.1 rule 4 + §BQ7 upstream integrity."
        },
        "normalization_method": {
          "type": ["string", "null"],
          "enum": ["none", "hybrid", "global_zscore", "per_feature_minmax", "market_structure_zscore", null],
          "description": "Must equal the trainer's training_info.normalization_method."
        },
        "normalization_stats_sha256": {
          "type": ["string", "null"],
          "pattern": "^[a-f0-9]{64}$",
          "description": "MUST equal the trainer's training_info.normalization_stats_sha256. Ingest REJECTS on mismatch (§14.8). This is the single most critical cross-stage integrity gate: a silent normalization-stats drift between training and inference produces numerically-plausible but semantically-wrong signals."
        },
        "signal_file_sha256": {
          "type": ["string", "null"],
          "pattern": "^[a-f0-9]{64}$",
          "description": "SHA-256 of the predicted_returns.npy (regression) or predictions.npy (classification) file the backtester consumed."
        },
        "signal_split": {
          "type": ["string", "null"],
          "enum": ["train", "val", "test", "full", null],
          "description": "Which split the signal was generated on. Backtests typically use 'test'; 'train' backtests are diagnostics."
        }
      }
    },
    "sub_records": {
      "type": "array",
      "items": {"type": "object"},
      "default": [],
      "description": "For record_type='sweep_aggregate' only. Embedded child records (e.g., multi-strategy backtest)."
    },
    "tags": {"type": "array", "items": {"type": "string"}, "default": []},
    "hypothesis": {"type": "string", "default": "", "description": "Mandatory per hft-rules §13 but accepts empty string for retroactive."},
    "description": {"type": "string", "default": ""},
    "notes": {"type": "string", "default": ""},
    "git": {
      "type": ["object", "null"],
      "description": "Producer's git state at emit time. Authoritative for that producer; cross-repo lineage uses per-lineage source_commit.",
      "properties": {
        "commit_hash": {"type": "string"},
        "branch": {"type": ["string", "null"]},
        "dirty": {"type": "boolean"},
        "short_hash": {"type": ["string", "null"]}
      }
    },
    "metadata_json": {
      "type": "object",
      "default": {},
      "description": "Forward-compat escape hatch. Unknown fields go here; not indexed. Use sparingly."
    }
  },
  "definitions": {
    "split_detail": {
      "type": "object",
      "properties": {
        "n_days": {"type": "integer"},
        "n_sequences": {"type": "integer"},
        "date_range": {"type": "array", "items": {"type": "string", "format": "date"}, "minItems": 2, "maxItems": 2},
        "failed_days": {"type": "array", "items": {"type": "object"}}
      }
    }
  }
}
```

### §3.3 Producer-Specific Envelope Templates

Four concrete examples based on real producer outputs (Round 9 V1 agent's stress-test). These are COMPLETE envelope examples a producer would emit.

#### §3.3.1 basic-quote-processor (record_type='export')

```json
{
  "envelope_version": 1,
  "envelope_schema_version": "1.0.0",
  "pipeline_contract_version": "2.2",
  "producer": "basic-quote-processor",
  "producer_version": "0.1.0",
  "record_type": "export",
  "experiment_id": "basic_nvda_60s_20260415T120000_a3b8f1c4",
  "experiment_fingerprint": "a3b8f1c4d5e6...64hex...",
  "fingerprint_version": 1,
  "created_at": "2026-04-15T12:00:00Z",
  "finalized_at": "2026-04-15T13:45:00Z",
  "wall_clock_ms": 6300000,
  "status": "completed",
  "asset_class": "equity",
  "data_source_type": "equity_off_exchange_trf",
  "feature_schema_ref": "off_exchange_1.0",
  "symbol": "NVDA",
  "dataset": "basic_nvda_60s",
  "model_type": null,
  "model_family": null,
  "task": null,
  "sampling_strategy": "time_based",
  "sampling_config": {"bin_size_seconds": 60},
  "label_family": "point_return",
  "primary_horizon": 10,
  "n_horizons": 8,
  "horizons": [1, 2, 3, 5, 10, 20, 30, 60],
  "label_spec": {"family": "point_return", "horizons_bins": [1,2,3,5,10,20,30,60], "smoothing_k": 0},
  "metrics": [],
  "gates": [],
  "artifacts": [
    {"kind": "manifest", "path": "data/exports/basic_nvda_60s/dataset_manifest.json", "bytes": 8192, "sha256": "..."},
    {"kind": "sequences_dir", "path": "data/exports/basic_nvda_60s/", "bytes": 228000000}
  ],
  "lineage": [
    {
      "source_kind": "raw_data",
      "source_name": "XNAS.BASIC/NVDA/2025-02-03..2026-01-06",
      "source_hash": null,
      "source_repo": "databento-raw",
      "source_commit": "not_git_tracked",
      "source_commit_dirty": null
    }
  ],
  "export_stats": {
    "days_processed": 233,
    "total_sequences": 71764,
    "sequence_length": 20,
    "stride": 1,
    "bin_size_seconds": 60,
    "normalization": "none",
    "n_features": 34,
    "feature_layout": "grouped",
    "feature_indices_subset": null,
    "splits": {
      "train": {"n_days": 163, "n_sequences": 50201, "date_range": ["2025-02-03", "2025-09-30"], "failed_days": []},
      "val": {"n_days": 35, "n_sequences": 10780, "date_range": ["2025-10-01", "2025-11-13"], "failed_days": []},
      "test": {"n_days": 35, "n_sequences": 10783, "date_range": ["2025-11-14", "2026-01-06"], "failed_days": []}
    }
  },
  "bulk_parquet": [
    {
      "kind": "per_day_export_stats",
      "path": "hft-ops/ledger/metrics/2026_04/c4f82a1...parquet",
      "schema": ["day:string", "n_bins_total:int32", "n_bins_valid:int32", "n_bins_warmup_discarded:int32", "n_bins_label_truncated:int32", "n_total_records:int64", "n_trf_trades:int64", "n_lit_trades:int64", "consolidated_volume:int64", "trf_volume_fraction:f64", "first_bin_start_ns:int64", "last_bin_end_ns:int64"],
      "partition_keys": ["experiment_fingerprint"],
      "row_count": 233,
      "sha256": "c4f82a1...",
      "bytes": 12288
    }
  ],
  "tags": ["phase-9", "off-exchange", "regen-post-phase-9"],
  "hypothesis": "Phase 9 metadata schema unblocks T9 LabelFactory for BASIC-only training.",
  "description": "Regeneration of 233-day off-exchange export with full Phase 9 provenance.",
  "notes": "",
  "git": {
    "commit_hash": "97badff008813566a4c87e6f42ead666cde8f5ef",
    "branch": "main",
    "dirty": false,
    "short_hash": "97badff"
  },
  "metadata_json": {}
}
```

#### §3.3.2 lob-model-trainer (record_type='training')

```json
{
  "envelope_version": 1,
  "envelope_schema_version": "1.0.0",
  "pipeline_contract_version": "2.2",
  "producer": "lob-model-trainer",
  "producer_version": "0.1.0",
  "record_type": "training",
  "experiment_id": "TLOB_Triple_Barrier_v1_20260314T031729_a3b8f1c4",
  "experiment_fingerprint": "a3b8f1c4d5e6...64hex...",
  "fingerprint_version": 1,
  "created_at": "2026-03-14T03:17:29Z",
  "finalized_at": "2026-03-14T04:17:29Z",
  "wall_clock_ms": 3600000,
  "status": "completed",
  "asset_class": "equity",
  "data_source_type": "equity_lob_mbo",
  "feature_schema_ref": "equity_v2.2",
  "symbol": "NVDA",
  "dataset": "nvda_triple_barrier",
  "model_type": "tlob",
  "model_family": "neural",
  "task": "classification",
  "sampling_strategy": "event_based",
  "sampling_config": {"event_count": 100},
  "label_family": "triple_barrier",
  "primary_horizon": 10,
  "n_horizons": 3,
  "horizons": [10, 60, 300],
  "label_spec": {
    "family": "triple_barrier",
    "theta_profit_bps": 8.0,
    "theta_stop_bps": 5.0,
    "H_time_bins": 100,
    "horizons_bins": [10, 60, 300]
  },
  "config_source": "<embedded YAML, 1200 bytes>",
  "config_format": "yaml",
  "sweep_id": null,
  "axis_values": null,
  "parent_id": null,
  "upstream_experiment_ids": ["basic_nvda_60s_20260415T120000_a3b8f1c4"],
  "metrics": [
    {"family": "classification", "name": "ACCURACY", "split": "val", "horizon": 10, "epoch": 18, "value": 0.596, "ci_low": 0.591, "ci_high": 0.601, "tag": null},
    {"family": "classification", "name": "MACRO_F1", "split": "val", "horizon": 10, "epoch": "best", "value": 0.431, "tag": null},
    {"family": "classification", "name": "ACCURACY", "split": "test", "horizon": 10, "epoch": "final", "value": 0.596, "tag": null},
    {"family": "classification", "name": "F1_CLASS_UP", "split": "test", "horizon": 10, "value": 0.412, "tag": "Up"}
  ],
  "gates": [
    {
      "gate_name": "ic_gt_0_05",
      "gate_order": 0,
      "depends_on_gate": null,
      "status": "passed",
      "threshold": 0.05,
      "observed": 0.380,
      "note": null
    },
    {
      "gate_name": "baseline_ridge",
      "gate_order": 1,
      "depends_on_gate": "ic_gt_0_05",
      "status": "passed",
      "threshold": 0.616,
      "observed": 0.677,
      "note": "TLOB IC exceeds Ridge IC baseline by 10%."
    }
  ],
  "artifacts": [
    {"kind": "checkpoint", "path": "lob-model-trainer/outputs/TLOB_Triple_Barrier_v1/.../checkpoints/best.pt", "bytes": 370000, "sha256": "..."},
    {"kind": "training_history", "path": "lob-model-trainer/outputs/.../training_history.json", "bytes": 28000, "sha256": "..."},
    {"kind": "config", "path": "lob-model-trainer/outputs/.../config.yaml", "bytes": 1200, "sha256": "..."}
  ],
  "lineage": [
    {
      "source_kind": "export",
      "source_name": "nvda_triple_barrier",
      "source_hash": "export_fingerprint_xyz...",
      "source_repo": "feature-extractor-MBO-LOB",
      "source_commit": "abc1234",
      "source_commit_dirty": false
    }
  ],
  "training_info": {
    "best_epoch": 18,
    "total_epochs": 30,
    "model_params": 370000,
    "num_train_samples": 162999,
    "num_val_samples": 52885,
    "num_test_samples": 50724,
    "training_time_seconds": 3600
  },
  "bulk_parquet": [
    {
      "kind": "training_curve",
      "path": "hft-ops/ledger/metrics/2026_03/f921b8c...parquet",
      "schema": ["epoch:int32", "train_loss:f64", "train_accuracy:f64", "val_loss:f64", "val_accuracy:f64", "val_macro_f1:f64", "val_stoploss_precision:f64", "val_profit_target_precision:f64", "lr:f64"],
      "partition_keys": ["experiment_fingerprint"],
      "row_count": 30,
      "sha256": "f921b8c...",
      "bytes": 4096
    }
  ],
  "tags": ["tlob", "triple-barrier", "h10-primary"],
  "hypothesis": "Triple-barrier labels with theta_profit=8bps outperform point-return at H=10.",
  "description": "TLOB classification with triple-barrier labels on 128-feature MBO data.",
  "notes": "",
  "git": {
    "commit_hash": "trainer_commit_abc",
    "branch": "main",
    "dirty": false,
    "short_hash": "abc1234"
  },
  "metadata_json": {}
}
```

#### §3.3.3 lob-backtester (record_type='backtest')

```json
{
  "envelope_version": 1,
  "envelope_schema_version": "1.0.0",
  "pipeline_contract_version": "2.2",
  "producer": "lob-backtester",
  "producer_version": "0.1.0",
  "record_type": "backtest",
  "experiment_id": "ibkr_h60_hold_20260314T031730_d7e8f9a0",
  "experiment_fingerprint": "d7e8f9a0...64hex...",
  "fingerprint_version": 1,
  "created_at": "2026-03-14T03:17:30Z",
  "finalized_at": "2026-03-14T03:25:00Z",
  "wall_clock_ms": 450000,
  "status": "completed",
  "asset_class": "option",
  "data_source_type": "opra_options",
  "feature_schema_ref": "equity_v2.2",
  "symbol": "NVDA",
  "dataset": "nvda_xnas_128feat_regression",
  "model_type": "tlob_regression",
  "model_family": "neural",
  "task": "backtest",
  "label_family": "point_return",
  "primary_horizon": 60,
  "n_horizons": 3,
  "horizons": [10, 60, 300],
  "upstream_experiment_ids": [
    "TLOB_Regression_E5_20260313T061500_92fb8c12"
  ],
  "metrics": [
    {"family": "backtest_pnl", "name": "TOTAL_RETURN", "split": "test", "horizon": 60, "value": -0.0368},
    {"family": "backtest_pnl", "name": "SHARPE_RATIO", "split": "test", "horizon": 60, "value": -5.74},
    {"family": "backtest_pnl", "name": "WIN_RATE", "split": "test", "horizon": 60, "value": 0.478},
    {"family": "option_metrics", "name": "OPTION_TOTAL_RETURN", "split": "test", "horizon": 60, "value": -0.0366},
    {"family": "option_metrics", "name": "OPTION_WIN_RATE", "split": "test", "horizon": 60, "value": 0.437},
    {"family": "option_metrics", "name": "AVG_THETA_COST", "split": "test", "horizon": 60, "value": 0.42}
  ],
  "gates": [
    {"gate_name": "sharpe_gt_0", "gate_order": 0, "status": "failed", "threshold": 0, "observed": -5.74}
  ],
  "artifacts": [
    {"kind": "result", "path": "lob-backtester/outputs/backtests/ibkr_h60_hold_20260314T031730/result.json", "bytes": 2400},
    {"kind": "equity_curve", "path": "lob-backtester/outputs/backtests/.../equity_curve.npy", "bytes": null},
    {"kind": "config", "path": "lob-backtester/outputs/backtests/.../config.yaml", "bytes": 1100}
  ],
  "lineage": [
    {
      "source_kind": "signals",
      "source_name": "TLOB_Regression_E5",
      "source_hash": "signals_sha256",
      "source_repo": "lob-model-trainer",
      "source_commit": "abc1234",
      "source_commit_dirty": false
    },
    {
      "source_kind": "trainer_run",
      "source_name": "TLOB_Regression_E5_20260313T061500_92fb8c12",
      "source_hash": null,
      "source_repo": "lob-model-trainer",
      "source_commit": "abc1234",
      "source_commit_dirty": false
    }
  ],
  "strategy_info": {
    "n_entries": 787,
    "n_gate_pass": 787,
    "n_gate_fail": 2768,
    "trade_rate": 0.0155,
    "holding_policy": "horizon_aligned_60",
    "exit_reasons": {"time_limit": 654, "stop_loss": 89, "profit_target": 44}
  },
  "signal_provenance": {
    "primary_horizon": 60,
    "total_samples": 50724,
    "predictions_distribution": {"down": 0.31, "stable": 0.42, "up": 0.27}
  },
  "bulk_parquet": [
    {
      "kind": "equity_curve",
      "path": "hft-ops/ledger/metrics/2026_03/9e4d2c8...parquet",
      "schema": ["timestep:int32", "equity:f64", "pnl_step_bps:f64", "trade_active:bool"],
      "partition_keys": ["experiment_fingerprint"],
      "row_count": 50724
    }
  ],
  "tags": ["backtest", "0dte", "ibkr-cost", "e5-round-1"],
  "hypothesis": "TLOB E5 H60 signals produce positive Sharpe on deep-ITM 0DTE options.",
  "description": "IBKR-calibrated 0DTE backtest at H60 hold.",
  "notes": "Sharpe -5.74 confirms E5/E6 pattern: high IC does not translate to tradeable P&L.",
  "git": {"commit_hash": "backtester_commit", "branch": "main", "dirty": false, "short_hash": "abc1234"},
  "metadata_json": {}
}
```

#### §3.3.4 hft-feature-evaluator (record_type='evaluation')

```json
{
  "envelope_version": 1,
  "envelope_schema_version": "1.0.0",
  "pipeline_contract_version": "2.2",
  "producer": "hft-feature-evaluator",
  "producer_version": "0.1.0",
  "record_type": "evaluation",
  "experiment_id": "E10_off_exchange_5path_20260401T090000_b7c8d9e0",
  "experiment_fingerprint": "b7c8d9e0...64hex...",
  "fingerprint_version": 1,
  "created_at": "2026-04-01T09:00:00Z",
  "finalized_at": "2026-04-01T09:45:00Z",
  "wall_clock_ms": 2700000,
  "status": "completed",
  "asset_class": "equity",
  "data_source_type": "equity_off_exchange_trf",
  "feature_schema_ref": "off_exchange_1.0",
  "symbol": "NVDA",
  "dataset": "basic_nvda_60s",
  "model_type": null,
  "model_family": null,
  "task": "analysis",
  "label_family": "point_return",
  "primary_horizon": 10,
  "n_horizons": 8,
  "horizons": [1, 2, 3, 5, 10, 20, 30, 60],
  "upstream_experiment_ids": ["basic_nvda_60s_20260415T120000_a3b8f1c4"],
  "metrics": [
    {"family": "ic_screening", "name": "IC_H10", "split": "full", "horizon": 10, "value": 0.103, "tag": "trf_signed_imbalance"},
    {"family": "ic_screening", "name": "IC_H10", "split": "full", "horizon": 10, "value": 0.178, "tag": "spread_bps"}
  ],
  "gates": [],
  "artifacts": [
    {"kind": "other", "path": "hft-feature-evaluator/output/e10_report.md", "bytes": 24000}
  ],
  "lineage": [
    {"source_kind": "export", "source_name": "basic_nvda_60s", "source_repo": "basic-quote-processor", "source_commit": "97badff", "source_commit_dirty": false}
  ],
  "bulk_parquet": [
    {
      "kind": "feature_ic",
      "path": "hft-ops/ledger/metrics/2026_04/ab12cd...parquet",
      "schema": ["feature_idx:int32", "feature_name:string", "ic:f64", "dcor:f64", "mi:f64", "horizon:int32", "significance:f64", "tier:string"],
      "partition_keys": ["experiment_fingerprint"],
      "row_count": 272,
      "sha256": "ab12cd..."
    }
  ],
  "tags": ["e10", "off-exchange", "5path"],
  "hypothesis": "Off-exchange features have tradeable signal at H≤60 via 5-path evaluation.",
  "description": "5-path evaluation of 34 off-exchange features against forward returns.",
  "notes": "2 STRONG-KEEP (spread_bps, trf_signed_imbalance), 32 DISCARD. Confirms E12 tradeability follow-up needed.",
  "git": {"commit_hash": "evaluator_commit", "branch": "main", "dirty": false, "short_hash": "abc1234"},
  "metadata_json": {}
}
```

#### §3.3.5 feature-extractor-MBO-LOB (record_type='export')

MBO extractor is the second-most-complex producer (148-feature schema, 9-crate workspace). It emits one envelope per multi-day export run. Unlike BQP (single-symbol, single pipeline), MBO exports may span a rolling set of days and use MBO-specific sampling strategies (event-based, MLOFI tracker, Kolm-OF).

```json
{
  "envelope_version": 1,
  "envelope_schema_version": "1.0.0",
  "pipeline_contract_version": "2.2",
  "producer": "feature-extractor-MBO-LOB",
  "producer_version": "2.0.0-workspace",
  "record_type": "export",
  "experiment_id": "mbo_nvda_xnas_e5_60s_20260415T090000_b7d3e2a1",
  "experiment_fingerprint": "b7d3e2a1...64hex...",
  "fingerprint_version": 1,
  "created_at": "2026-04-15T09:00:00Z",
  "finalized_at": "2026-04-15T11:47:00Z",
  "wall_clock_ms": 9780000,
  "status": "completed",
  "asset_class": "equity",
  "data_source_type": "mbo_lob_reconstructed",
  "feature_schema_ref": "mbo_2.2",
  "symbol": "NVDA",
  "dataset": "mbo_nvda_xnas_e5_60s",
  "model_type": null,
  "model_family": null,
  "task": null,
  "sampling_strategy": "time_based",
  "sampling_config": {"bin_size_seconds": 60, "warmup_events": 100, "mbo_ready_index": 94},
  "label_family": "regression",
  "primary_horizon": 10,
  "n_horizons": 3,
  "horizons": [10, 60, 300],
  "label_spec": {"family": "smoothed_return", "horizons_bins": [10,60,300], "smoothing_k": 5, "units": "basis_points"},
  "metrics": [],
  "gates": [],
  "artifacts": [
    {"kind": "manifest", "path": "data/exports/mbo_nvda_xnas_e5_60s/dataset_manifest.json", "bytes": 16384, "sha256": "..."},
    {"kind": "sequences_dir", "path": "data/exports/mbo_nvda_xnas_e5_60s/", "bytes": 6200000000},
    {"kind": "normalization_stats", "path": "data/exports/mbo_nvda_xnas_e5_60s/train/normalization.json", "bytes": 4096, "sha256": "..."}
  ],
  "lineage": [
    {
      "source_kind": "raw_data",
      "source_name": "XNAS.MBO/NVDA/2025-02-03..2026-01-06",
      "source_hash": null,
      "source_repo": "databento-raw",
      "source_commit": "not_git_tracked",
      "source_commit_dirty": null
    },
    {
      "source_kind": "reconstructor",
      "source_name": "MBO-LOB-reconstructor",
      "source_hash": null,
      "source_repo": "MBO-LOB-reconstructor",
      "source_commit": "d4f2a18",
      "source_commit_dirty": false
    }
  ],
  "export_stats": {
    "days_processed": 233,
    "total_sequences": 266608,
    "sequence_length": 100,
    "stride": 1,
    "bin_size_seconds": 60,
    "normalization": "none",
    "n_features": 98,
    "feature_layout": "grouped",
    "feature_indices_subset": [0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32,33,34,35,36,37,38,39,40,41,42,43,44,45,46,47,48,49,50,51,52,53,54,55,56,57,58,59,60,61,62,63,64,65,66,67,68,69,70,71,72,73,74,75,76,77,78,79,80,81,82,83,84,85,86,87,88,89,90,91,92,93,94,95,96,97],
    "splits": {
      "train": {"n_days": 163, "n_sequences": 162999, "date_range": ["2025-02-03", "2025-09-30"], "failed_days": []},
      "val":   {"n_days":  35, "n_sequences":  52885, "date_range": ["2025-10-01", "2025-11-13"], "failed_days": []},
      "test":  {"n_days":  35, "n_sequences":  50724, "date_range": ["2025-11-14", "2026-01-06"], "failed_days": []}
    }
  },
  "bulk_parquet": [
    {
      "kind": "per_day_export_stats",
      "path": "hft-ops/ledger/metrics/2026_04/a2e7b1f8...parquet",
      "schema": ["day:string", "n_sequences:int32", "n_bins_valid:int32", "mbo_ready_at_event:int32", "book_valid_fraction:f64", "median_spread_bps:f64", "median_depth_total:int64"],
      "partition_keys": ["experiment_fingerprint"],
      "row_count": 233,
      "sha256": "a2e7b1f8...",
      "bytes": 16384
    }
  ],
  "tags": ["e5", "mbo", "tradeable-horizons", "time-based"],
  "hypothesis": "Time-based 60s sampling at H10 captures tradeable OFI persistence (profiler ACF=0.266).",
  "description": "E5 regeneration post-multi-crate decomposition.",
  "notes": "99.99% bit-exact vs archived monolith build.",
  "git": {
    "commit_hash": "3e8b7c1f...",
    "branch": "main",
    "dirty": false,
    "short_hash": "3e8b7c1f"
  },
  "metadata_json": {}
}
```

#### §3.3.6 Canonical JSON Serialization Spec (cross-language, delegating to SSoT)

**Round 16 correction (v1.3.3)**: this section previously specified a NEW `canonical_json_dumps` with compact separators and `allow_nan=False`. Verification against `hft_contracts/canonical_hash.py` (Phase 4 Batch 4c hardening, 2026-04-15) revealed this **violates the frozen monorepo canonical form** which uses DEFAULT separators (`", "` and `": "` — WITH spaces), DEFAULT `ensure_ascii=True`, DEFAULT `allow_nan=True`. Per `hft_contracts/canonical_hash.py:33-37`: **"Compact-separator variants would produce different bytes and break existing fingerprints; the whitespace convention is load-bearing."** And per PA §3024: **"Anti-pattern (eliminated 2026-04-15): pre-Phase-4c `canonical_hash` was re-implemented at 5 sites. ... Extracted to `hft_contracts.canonical_hash` SSoT. When adding a new hash/canonical-form primitive, always place it in `hft-contracts` first, then import; never re-implement."**

**Phase 10 REUSES the existing SSoT. No new Python canonical module is created.**

**Python side** (reuse — `hft_contracts.canonical_hash.canonical_json_blob`, frozen contract):

```python
# Phase 10 consumption pattern (used by ingest, audit log, content_hash):
from hft_contracts.canonical_hash import canonical_json_blob, sha256_hex

# Frozen contract (from hft_contracts/canonical_hash.py):
#   canonical_json_blob(obj) ≡ json.dumps(
#       obj, sort_keys=True, default=str
#   ).encode("utf-8")
# - DEFAULT separators: ", " (after comma, WITH space), ": " (after colon, WITH space)
# - DEFAULT ensure_ascii: True (non-ASCII → \uXXXX)
# - DEFAULT allow_nan: True (NaN → "NaN" token, non-strict-JSON)
# - sanitize=True option pre-processes NaN/Inf → None (strict-JSON safe)
# - default=str: Path, Enum fall back to str() representation
#
# Phase 10 MUST call with sanitize=True because envelope.metrics[].value
# legitimately carries NaN-as-missing semantics (e.g., "metric not computed"):
envelope_bytes = canonical_json_blob(envelope_dict, sanitize=True)
envelope_content_hash = sha256_hex(envelope_bytes)  # 64-char lowercase hex
```

**Consumers of this SSoT** (pre-existing, Phase 10 joins them):
- `hft_ops.ledger.dedup.compute_fingerprint` — experiment fingerprint (Phase 3 §3.3b)
- `hft_ops.provenance.lineage.hash_config_dict`
- `hft_ops.feature_sets.hashing.compute_feature_set_hash`
- `hft_evaluator.pipeline.compute_profile_hash`
- `lobtrainer.data.feature_set_resolver._compute_content_hash` (inlined for cross-venv isolation; parity-locked)
- **NEW Phase 10**: envelope `content_hash` (§3.5), `ingest_one` idempotency byte-compare (§7.3), `audit_log_append` sidecar content (§7.3.2)

**Rust side** — produce byte-identical output to Python's frozen form:

```rust
// hft-contracts/rust/orchestration/canonical_json.rs
//
// Mirrors Python's hft_contracts.canonical_hash.canonical_json_blob exactly.
// Frozen contract: json.dumps(obj, sort_keys=True, default=str).encode("utf-8")
//   — DEFAULT whitespace separators (", " and ": " — NOT compact)
//   — DEFAULT ensure_ascii=True (non-ASCII → \uXXXX)
//   — sanitize=True: NaN/Inf → null before serialization
//
// CRITICAL: serde_json's defaults DIFFER from Python's defaults on both
//   (a) separators (serde_json = compact ","/":" ; Python default = ", "/": ")
//   (b) non-ASCII (serde_json = raw UTF-8 bytes; Python default = \uXXXX escape)
// BOTH divergences must be corrected via post-processing to match Python byte-for-byte.

use serde::Serialize;
use serde_json::{to_string, Value};

pub fn canonical_json_blob<T: Serialize>(value: &T) -> Result<Vec<u8>, CanonicalError> {
    let intermediate: Value = serde_json::to_value(value)?;
    let sanitized = sanitize_non_finite(intermediate);      // NaN/Inf → Null (matches Python sanitize=True)
    let sorted = sort_keys_recursive(sanitized);
    let compact = to_string(&sorted)?;                      // compact JSON in UTF-8
    let spaced = restore_python_default_separators(&compact); // "," → ", "  and  ":" → ": "
    let ascii_escaped = escape_non_ascii_in_json(&spaced);   // non-ASCII → \uXXXX (RFC 8259 §7 surrogates)
    Ok(ascii_escaped.into_bytes())
}

pub fn sha256_hex(blob: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(blob))
}

fn sanitize_non_finite(v: Value) -> Value {
    match v {
        Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                if !f.is_finite() { return Value::Null; }
            }
            Value::Number(n)
        }
        Value::Object(m) => Value::Object(m.into_iter().map(|(k, vv)| (k, sanitize_non_finite(vv))).collect()),
        Value::Array(xs) => Value::Array(xs.into_iter().map(sanitize_non_finite).collect()),
        other => other,
    }
}

fn sort_keys_recursive(v: Value) -> Value {
    match v {
        Value::Object(m) => {
            let sorted: std::collections::BTreeMap<String, Value> =
                m.into_iter().map(|(k, vv)| (k, sort_keys_recursive(vv))).collect();
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(xs) => Value::Array(xs.into_iter().map(sort_keys_recursive).collect()),
        other => other,
    }
}

/// Post-process serde_json compact output to Python's default separators:
///   "," (between items) → ", " (WITH space)
///   ":" (between key-value) → ": " (WITH space)
/// Walks the string character-by-character, tracking string-literal context so
/// that separators INSIDE string values are NOT modified.
fn restore_python_default_separators(compact: &str) -> String {
    let mut out = String::with_capacity(compact.len() + compact.len() / 8);
    let mut in_string = false;
    let mut prev_backslash = false;
    for c in compact.chars() {
        if in_string {
            out.push(c);
            if c == '"' && !prev_backslash { in_string = false; }
            prev_backslash = c == '\\' && !prev_backslash;
            continue;
        }
        out.push(c);
        if c == '"' { in_string = true; prev_backslash = false; continue; }
        if c == ',' || c == ':' { out.push(' '); }
    }
    out
}

/// Escape non-ASCII characters as \uXXXX (surrogate pairs for supplementary code points).
/// Matches Python's json.dumps(..., ensure_ascii=True) output byte-for-byte.
fn escape_non_ascii_in_json(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut in_string = false;
    let mut prev_backslash = false;
    for c in raw.chars() {
        if !in_string {
            if c == '"' { in_string = true; }
            out.push(c);
            continue;
        }
        if c == '"' && !prev_backslash {
            in_string = false;
            out.push(c);
            prev_backslash = false;
            continue;
        }
        prev_backslash = (c == '\\') && !prev_backslash;
        if c.is_ascii() {
            out.push(c);
        } else {
            let code = c as u32;
            if code <= 0xFFFF {
                out.push_str(&format!("\\u{:04x}", code));
            } else {
                let adjusted = code - 0x10000;
                let high = 0xD800 + (adjusted >> 10);
                let low  = 0xDC00 + (adjusted & 0x3FF);
                out.push_str(&format!("\\u{:04x}\\u{:04x}", high, low));
            }
        }
    }
    out
}

#[derive(thiserror::Error, Debug)]
pub enum CanonicalError {
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
}

// Test fixtures anchor byte-exact parity with Python:
//   Input  {"sym": "Ω", "n": 10}
//   Python canonical_json_blob(obj) → '{"n": 10, "sym": "\u03a9"}'  (whitespace + ASCII escape)
//   Rust   canonical_json_blob(&obj) → same bytes
```

**Invariants (tested by cross-lang fixtures at `hft-contracts/tests/fixtures/canonical_json/`):**
1. **Key order**: lexicographic ASCII sort at every nesting level (matches Python `sort_keys=True`).
2. **Separators**: `", "` between items (WITH space), `": "` between key-value (WITH space) — Python default. Rust `restore_python_default_separators` post-processes serde_json's compact output.
3. **Encoding**: non-ASCII characters escaped as `\uXXXX` with surrogate pairs for supplementary code points (matches Python `ensure_ascii=True` default).
4. **NaN / Inf / -Inf**: pre-processed to `null` via `sanitize=True` (Python) / `sanitize_non_finite` (Rust). Callers that legitimately carry NaN-as-missing (envelope metrics) use sanitize=True; callers that cannot tolerate NaN should catch `ValueError` at Pydantic validation time BEFORE reaching canonical_json_blob.
5. **Integer representation**: JSON numbers without decimal point (e.g., `10` not `10.0`). Both sides.
6. **Unicode normalization**: NOT applied (same raw bytes; no NFC/NFD reshaping).
7. **Trailing whitespace**: none at end of output.

**Content hash formula** (used by §3.5 envelope filename, §7.3 idempotent compare, §14.8 integrity checks):
```
content_hash = sha256_hex(canonical_json_blob(envelope_dict, sanitize=True))
```

**Test fixture** (shipped in `hft-contracts/tests/fixtures/canonical_json/`):

```
fixtures/canonical_json/
├── 01_simple.json          → {"a":1,"b":"hello","c":[1,2,3]}
├── 02_nested.json          → {"outer":{"z":1,"a":2}}  canonical: {"outer":{"a":2,"z":1}}
├── 03_unicode.json         → {"sym":"Ω"}  canonical: {"sym":"\u03a9"}
├── 04_float_int.json       → {"x":10,"y":10.5}  rejects 10.0 as int? (decision: respect Python dumps → "10.5")
├── 05_nan_rejected.json    → {"x":NaN}  both sides error
├── 06_large_array.json     → stress test
└── expected_sha256.json    → {fixture: hex_hash} — identical across Python + Rust
```

The Rust `hft_contracts_rs` crate imports these fixtures and asserts `canonical_json_blob` produces the matching hash; the Python `hft_contracts.canonical_hash.canonical_json_blob` does the same. CI runs both suites on every PR.

**Non-goal for v1:** floating-point representation stability across Python ↔ Rust when numbers have >15 significant digits. Document as known limitation; producers should either (a) round to 12 digits before hashing, or (b) pass all floating-point values through Python first.

### §3.4 Envelope Sizing and Inlining Rules

- `config_source`: inline if config text < 8 KB; else use `artifacts[{kind:"config"}]` + `config_source: null`.
- `sub_records`: inline only for `sweep_aggregate`; max 50 children or 256 KB total; else each child is its own envelope with shared `sweep_id`.
- `bulk_parquet`: always separate files, never inlined.
- Total envelope size target: ≤ 128 KB. Hard limit: 1 MB. Rejected if exceeds hard limit.

### §3.5 Envelope Filename Convention

`hft-ops/ledger/inbox/{content_hash}.json` where `content_hash = sha256_hex(canonical_json_blob(envelope_dict, sanitize=True))` — the **full** envelope including `metadata_json`. Uses the monorepo SSoT form per §3.3.6 (Round 16 v1.3.3 correction: delegates to `hft_contracts.canonical_hash.canonical_json_blob` rather than a new `canonical_json_dumps`).

**Round 13 change (D3)**: previously, `metadata_json` was excluded from content hash on the theory that "metadata_json is an escape hatch and should not affect identity." Under stress-testing this creates a silent-data-loss path: if producer emits twice in rapid succession with DIFFERENT `metadata_json` (e.g., retry adds debug info), both envelopes get the same filename — `os.replace()` silently overwrites the first. Per `hft-rules §8` ("Never silently drop, clamp, or 'fix' data without recording diagnostics"), this is unacceptable.

Under the revised rule:
- **Byte-identical re-emission** (true duplicate) → same content_hash → same filename → `os.replace` is a no-op. Idempotency preserved for genuine duplicates.
- **Different `metadata_json`** → different content_hash → different filenames → both envelopes land in inbox. Ingester processes first → SUCCESS. Processes second → PK violation on `experiment_fingerprint` → quarantine with `.error` sidecar. **Both envelopes preserved for operator review; neither silently lost.**

Identity semantics unchanged: `experiment_fingerprint` remains the identity (PK). `content_hash` governs only the inbox filename.

### §3.6 envelope_version Dispatch

First field parsed is `envelope_version: int`. Ingest dispatches to `INGEST_HANDLERS[version]`:

```python
INGEST_HANDLERS: dict[int, Callable[[dict], ValidatedEnvelope]] = {
    0: ingest_v0_legacy_migration,
    1: ingest_v1,  # canonical
    # 2: ingest_v2,  # future
}

def ingest(envelope_path: Path) -> IngestResult:
    with open(envelope_path) as f:
        envelope = json.load(f)
    version = envelope.get("envelope_version")
    handler = INGEST_HANDLERS.get(version)
    if handler is None:
        return IngestResult.quarantine(
            envelope_path,
            reason=f"Unsupported envelope_version={version}; known={list(INGEST_HANDLERS.keys())}"
        )
    return handler(envelope)
```

Version 0 is reserved for the one-shot backfill of legacy 34 JSON records. Version 1 is canonical. Version 2 would introduce breaking changes (hypothetical; not planned).

---

## §4 — MetricKey Registry

### §4.1 Problem

Today, metric names are free-form strings scattered across producers:
- Trainer emits `"r2"`, `"ic"`, `"pearson"`, `"mae"`, `"rmse"`, `"profitable_accuracy"` (result.py:extra_metrics).
- Backtester emits `"SharpeRatio"`, `"SortinoRatio"`, `"MaxDrawdown"`, `"TotalReturn"`, `"WinRate"`, `"ProfitFactor"`, `"Expectancy"` — note CamelCase inconsistency.
- Evaluator emits `"ic"`, `"dcor"`, `"mi"`, `"tier"`, `"kept"`, `"discarded"` (varies by path).

Consequences:
- `hft-ops ledger compare -m sharpe_ratio` fails on backtester records that use `SharpeRatio`.
- "Show all experiments with test IC > 0.1" requires normalizing `ic`, `IC`, `pearson_ic` on the fly.
- New metric additions have no registry; typos propagate silently.

### §4.2 Solution: MetricKey enum in `hft-contracts`

Defined in `contracts/pipeline_contract.toml` under `[orchestration.metric_keys]`.

**Range convention (Round 10 fix):** TOML has no `null` literal, so half-bounded ranges use separate `lower_bound` / `upper_bound` scalars instead of `range = [null, x]`. Fully-bounded ranges still use `range = [lo, hi]`. Both forms are parsed as optional documentation — they do NOT enforce at ingest (warn-only; enforcement at gate time if needed).

```toml
[orchestration.metric_keys]
schema_version = "1.0.0"

# Classification metrics
ACCURACY                 = { family = "classification", description = "Overall accuracy", range = [0.0, 1.0] }
MACRO_F1                 = { family = "classification", description = "Macro-averaged F1", range = [0.0, 1.0] }
F1_CLASS_UP              = { family = "classification", description = "F1 for 'up' class", tag_required = true }
F1_CLASS_DOWN            = { family = "classification", description = "F1 for 'down' class", tag_required = true }
F1_CLASS_STABLE          = { family = "classification", description = "F1 for 'stable' class", tag_required = true }
DIRECTIONAL_ACCURACY     = { family = "classification", description = "DA — sign(pred) == sign(label)", range = [0.0, 1.0] }
STOPLOSS_PRECISION       = { family = "classification", description = "Per-class precision for stop-loss", range = [0.0, 1.0] }
PROFIT_TARGET_PRECISION  = { family = "classification", description = "Per-class precision for profit target", range = [0.0, 1.0] }

# Regression metrics
R2                       = { family = "regression", description = "Coefficient of determination", upper_bound = 1.0 }
IC                       = { family = "regression", description = "Information coefficient (Pearson)", range = [-1.0, 1.0] }
IC_H10                   = { family = "regression", description = "IC at 10-event horizon", range = [-1.0, 1.0] }
# Dynamic names IC_H{n} permitted; ingest accepts unregistered names with WARN (see §4.3)
MAE                      = { family = "regression", description = "Mean absolute error", lower_bound = 0.0 }
RMSE                     = { family = "regression", description = "Root mean squared error", lower_bound = 0.0 }
HUBER_LOSS               = { family = "regression", description = "Huber loss", lower_bound = 0.0 }
GMADL_LOSS               = { family = "regression", description = "GMADL loss" }

# Training dynamics (per-epoch time series; typically emitted to bulk_parquet)
TRAIN_LOSS               = { family = "training_dynamics", description = "Per-epoch training loss", lower_bound = 0.0 }
VAL_LOSS                 = { family = "training_dynamics", description = "Per-epoch validation loss", lower_bound = 0.0 }
LEARNING_RATE            = { family = "training_dynamics", description = "Current LR", lower_bound = 0.0 }
GRAD_NORM                = { family = "training_dynamics", description = "Gradient norm", lower_bound = 0.0 }

# Backtest P&L
TOTAL_RETURN             = { family = "backtest_pnl", description = "Total return over test period", lower_bound = -1.0 }
SHARPE_RATIO             = { family = "backtest_pnl", description = "Annualized Sharpe" }
SORTINO_RATIO            = { family = "backtest_pnl", description = "Annualized Sortino" }
MAX_DRAWDOWN             = { family = "backtest_pnl", description = "Peak-to-trough drawdown", range = [-1.0, 0.0] }
CALMAR_RATIO             = { family = "backtest_pnl", description = "Return / max drawdown" }
WIN_RATE                 = { family = "backtest_pnl", description = "Fraction of winning trades", range = [0.0, 1.0] }
PROFIT_FACTOR            = { family = "backtest_pnl", description = "Gross wins / gross losses", lower_bound = 0.0 }
EXPECTANCY               = { family = "backtest_pnl", description = "Expected P&L per trade" }
TOTAL_TRADES             = { family = "backtest_pnl", description = "Trade count", lower_bound = 0 }
AVG_HOLD_MS              = { family = "backtest_pnl", description = "Average holding period (ms)", lower_bound = 0 }

# Option-specific backtest
OPTION_TOTAL_RETURN      = { family = "option_metrics", description = "0DTE option P&L total return", lower_bound = -1.0 }
OPTION_WIN_RATE          = { family = "option_metrics", description = "Option-trade win rate", range = [0.0, 1.0] }
AVG_THETA_COST           = { family = "option_metrics", description = "Average theta decay per trade" }
AVG_SPREAD_COST_BPS      = { family = "option_metrics", description = "Average half-spread cost", lower_bound = 0.0 }
BREAKEVEN_BPS            = { family = "option_metrics", description = "Required P&L to overcome costs", lower_bound = 0.0 }

# Feature evaluation
IC_SCREENING             = { family = "feature_eval", description = "IC from ic_screening path", range = [-1.0, 1.0] }
DCOR_SCREENING           = { family = "feature_eval", description = "Distance correlation", range = [0.0, 1.0] }
MI_SCREENING             = { family = "feature_eval", description = "Mutual information (KSG)", lower_bound = 0.0 }
TRANSFER_ENTROPY         = { family = "feature_eval", description = "Transfer entropy", lower_bound = 0.0 }
TEMPORAL_IC_7D           = { family = "feature_eval", description = "7-day rolling IC", range = [-1.0, 1.0] }
BH_ADJUSTED_P            = { family = "feature_eval", description = "Benjamini-Hochberg adjusted p-value", range = [0.0, 1.0] }

# Opportunity / execution metrics
COST_BREAKEVEN_BPS       = { family = "cost_analysis", description = "Minimum return to break even", lower_bound = 0.0 }
EDGE_MARGIN_BPS          = { family = "cost_analysis", description = "Signal IC × cost / total" }

# Dataset analysis
LABEL_DISTRIBUTION_ENTROPY = { family = "dataset_health", description = "Shannon entropy of label distribution", lower_bound = 0.0 }
FEATURE_DRIFT_KL           = { family = "dataset_health", description = "KL divergence train vs test", lower_bound = 0.0 }
```

**Range semantics (validation-time):**
- `range = [lo, hi]` → both endpoints finite; ingest WARNs if `value < lo` or `value > hi`.
- `lower_bound = x` only → values `< x` WARN; no upper bound.
- `upper_bound = x` only → values `> x` WARN; no lower bound.
- Neither set → no bound check (e.g., `SHARPE_RATIO` can legitimately take any real value).
- Mixed forms are rejected at contract-load time (`lower_bound` + `range` together is an error).

### §4.3 Extensibility

**Adding a new metric**:
1. Propose in `pipeline_contract.toml` under `[orchestration.metric_keys]`.
2. Run `contracts/generate_python_contract.py` → produces new entry in `hft_contracts.orchestration.MetricKey` enum.
3. Bump `pipeline_contract.toml:[contract].schema_version` minor.
4. Producer code imports new enum value.

**Open-vocabulary escape**: if a producer needs a metric NOT in the registry, envelope's `metrics[].name` accepts any string BUT with `metrics[].family` required. Ingest emits WARNING for unregistered names. Registry encouraged, not enforced as hard error at v1 (warn-only to accommodate fast iteration).

### §4.4 MetricFamily enum

Grouping for cohort queries:

- `classification`
- `regression`
- `training_dynamics`
- `backtest_pnl`
- `option_metrics`
- `feature_eval`
- `cost_analysis`
- `dataset_health`
- `custom` (unregistered)

Queries like `SELECT * FROM metrics WHERE family='backtest_pnl' AND name='SHARPE_RATIO'` become indexed lookups.

### §4.5 GateKey enum (parallel concept)

Gate names also become an enum, mirroring MetricKey:

```toml
[orchestration.gate_keys]
schema_version = "1.0.0"

# Mandatory gates per hft-rules.md §13
ic_gt_0_05 = { description = "Signal quality gate: IC > 0.05 on primary horizon" }
cost_breakeven = { description = "Model return must exceed ATM call breakeven (4.9 bps) or Deep ITM (1.4 bps)" }
baseline_ridge = { description = "Model must exceed TemporalRidge baseline on R² or IC" }
baseline_persistence = { description = "Model must exceed persistence baseline" }
evaluation_tool_sanity = { description = "Evaluation module tested on known test cases before trusting output" }
optimization_execution_alignment = { description = "Training objective measures same quantity as execution objective" }

# Advisory gates
label_exec_alignment_gt_0_5 = { description = "Label-to-execution correlation > 0.5 (P0 validation)" }
sign_flip_rate_lt_0_5 = { description = "Feature sign-flip rate < 50% across CV folds" }
bh_fdr_significant = { description = "Feature survives Benjamini-Hochberg FDR correction" }

# Evaluator selection-criteria gates (Round 17 I10 — used by record_type='evaluation' envelopes)
# Phase 4 Batch 4a `SelectionCriteria` fields → envelope.gates[] entries (not a separate
# selection_criteria[] array; reuses the existing gates[] schema for consistency).
# A `record_type='evaluation'` envelope populates one gate per criterion with:
#   gate_name    = one of the keys below
#   threshold    = the criterion's threshold value (e.g., min_ic = 0.05)
#   observed     = the realized statistic on the evaluation split
#   status       = 'passed' if criterion satisfied, 'failed' otherwise
selection_min_ic = { description = "Evaluator SelectionCriteria.min_ic threshold: feature IC >= threshold" }
selection_min_abs_ic = { description = "Evaluator SelectionCriteria.min_abs_ic threshold: |IC| >= threshold" }
selection_require_holdout_confirmed = { description = "Evaluator SelectionCriteria.require_holdout_confirmed: holdout split confirms training-split signal (boolean; threshold=1, observed=1/0)" }
```

Gate enum allows hft-rules §13 enforcement to be SCHEMA-BACKED, not documentation-only. **Round 17 I10 decision**: evaluator `SelectionCriteria` (Phase 4 Batch 4a) fields reuse the `gates[]` schema rather than introducing a separate `selection_criteria[]` envelope array — one concept, consistent validation path, no new JSON Schema property.

### §4.6 Markdown Render Throttle (Round 10 — Agent D R3)

**Problem:** the initial design specified eager post-ingest markdown render (§7.5) — every successful envelope ingest re-renders `EXPERIMENT_INDEX.md`, `BACKTEST_INDEX.md`, and `EXPORT_INDEX.md`. At the "thousands of experiments" target, a sweep writing 500 envelopes in 2 minutes would trigger 500 full renders (O(N²) work — each render reads ALL N rows, producing ~250,000 SQLite scans for a 500-envelope sweep). A debounce makes this O(N).

**Design: debounced background render with rate cap.**

```python
# hft-ops/src/hft_ops/ledger/render_throttle.py

from __future__ import annotations
import threading
import time
from pathlib import Path

_render_lock = threading.Lock()
_last_render_at: float = 0.0           # monotonic time
_render_pending: bool = False
_render_thread: threading.Thread | None = None

DEBOUNCE_SECONDS = 5.0                 # minimum gap between renders
FORCE_RENDER_AFTER = 60.0              # absolute ceiling — a render MUST fire ≥ 1×/min
                                       # even if ingests keep retriggering it

def schedule_render_indexes(ledger_dir: Path) -> None:
    """Request a markdown render; coalesce bursts; never block the caller.

    Semantics:
      - First call after idle: render fires immediately.
      - Subsequent calls within DEBOUNCE_SECONDS: coalesced into one pending render.
      - Pending render fires DEBOUNCE_SECONDS after the LAST call (trailing edge).
      - If ingests keep arriving faster than DEBOUNCE_SECONDS for > FORCE_RENDER_AFTER,
        force a render to avoid unbounded staleness.
    """
    global _last_render_at, _render_pending, _render_thread
    now = time.monotonic()
    with _render_lock:
        if _render_thread is not None and _render_thread.is_alive():
            _render_pending = True
            return
        if now - _last_render_at < DEBOUNCE_SECONDS and not _render_pending:
            # Start a timer that will fire at (_last_render_at + DEBOUNCE_SECONDS)
            _render_pending = True
            delay = DEBOUNCE_SECONDS - (now - _last_render_at)
            _render_thread = threading.Thread(
                target=_delayed_render, args=(ledger_dir, delay), daemon=True
            )
            _render_thread.start()
            return
        # Fire immediately (first-call or long-idle path)
        _render_thread = threading.Thread(
            target=_render_now, args=(ledger_dir,), daemon=True
        )
        _render_thread.start()

def _delayed_render(ledger_dir: Path, delay: float) -> None:
    time.sleep(delay)
    _render_now(ledger_dir)

def _render_now(ledger_dir: Path) -> None:
    global _last_render_at, _render_pending
    try:
        from hft_ops.ledger.render import render_indexes
        render_indexes(ledger_dir)                       # reads SQLite, writes 3 MDs atomically
    except Exception as e:
        # Never fail ingest on render error; log and continue
        import logging
        logging.getLogger(__name__).warning("render_indexes failed: %s", e)
    finally:
        with _render_lock:
            _last_render_at = time.monotonic()
            _render_pending = False
```

**CLI override:** `hft-ops ledger render-indexes [--force]` synchronously renders, bypassing the throttle. Used for:
- End-of-sweep summary (sweep driver calls this after all envelopes ingested).
- CI doc generation (deterministic timing required).
- Debug: user wants to see current state.

**Tests (from §18.2 Render-throttle tests):**
- 100 rapid ingests → exactly 1 or 2 renders (not 100).
- Idle 10-sec period → first ingest renders immediately.
- Burst: 500 ingests in 10 sec → ≤ 3 renders (confirmed by counting log lines).
- Ingest error in `render_indexes` does NOT mark ingest as failed.

---

## §5 — Ledger Storage (SQLite + Parquet)

### §5.1 File Layout

```
hft-ops/ledger/
├── ledger.sqlite              # primary relational store
├── ledger.sqlite-wal          # WAL file (auto-managed)
├── ledger.sqlite-shm          # shared memory (auto-managed)
├── ledger.lock                # flock() file for exclusive ops
├── records/                   # append-only source of truth
│   └── {experiment_id}.json   # legacy-compatible per-experiment JSON
├── inbox/                     # producers write envelopes here
│   └── {content_hash}.json    # content-addressable; idempotent
├── quarantine/                # rejected envelopes
│   ├── {content_hash}.json    # the bad envelope
│   └── {content_hash}.error   # parse / validation error
└── metrics/                   # Parquet side files, partitioned
    ├── 2026_03/               # yyyy_mm partition
    │   └── {content_hash}.parquet
    ├── 2026_04/
    │   └── ...
    └── ...
```

**Invariants**:
- `records/*.json` is the source of truth. `ledger.sqlite` is rebuildable.
- Parquet files have content-addressable filenames; identical metric arrays dedup naturally.
- `inbox/` and `quarantine/` are transient (processed envelopes moved out).
- Monthly partition of `metrics/` keeps filesystem directory size bounded (<5k files per dir at expected scale).

### §5.2 SQLite PRAGMAs (Normative)

Applied at every open:

```sql
PRAGMA journal_mode = WAL;            -- concurrent readers during writes
PRAGMA synchronous = NORMAL;          -- safe with WAL; ~10x faster than FULL
PRAGMA busy_timeout = 5000;           -- 5s wait on lock contention
PRAGMA wal_autocheckpoint = 1000;     -- checkpoint every 1000 pages
PRAGMA foreign_keys = ON;             -- enforce FK constraints
PRAGMA temp_store = MEMORY;           -- faster sorts/aggregations
```

### §5.3 SQL Schema

**Round 10 refactor (Agent D R1/R2):** fingerprint-alone PK replaces composite `(fp, symbol, asset_class)` PK (Phase 3 fingerprint already embeds symbol via extraction config + data_manifest components — see `dedup.py:284-391`). Table count drops from 12 → 10: `symbol_cohort` folded into `experiments.symbols_json` + `cohort_hash` columns; `ingest_audit` replaced by append-only `ingest.log` JSONL file. All child FKs simplified to fingerprint-only. SQLite DDL syntax corrected: table-level PRIMARY KEY constraints now appear **after** all column declarations (mixing constraints with columns fails to parse — verified in Round 10).

```sql
-- ============================================================================
-- Ledger v2 Schema (schema_version = 1.0.0)
-- pipeline_contract.toml:[orchestration.envelope] is authoritative for the
-- envelope shape; this schema translates it into a normalized relational form.
-- ============================================================================

-- Schema version gate. Old code refuses newer DB.
CREATE TABLE IF NOT EXISTS schema_info (
    key                  TEXT PRIMARY KEY,
    value                TEXT NOT NULL
);
INSERT OR IGNORE INTO schema_info (key, value) VALUES ('schema_version', '1.0.0');
INSERT OR IGNORE INTO schema_info (key, value) VALUES ('envelope_version_max', '1');
-- Round 14 C6: schema_migration_num bootstrap. MUST be set at DB-creation
-- time (not only by the migration runner). Fresh DB initialized via the raw
-- §5.3 DDL has `schema_migration_num='0'`; the first migration (001_initial)
-- advances this to '1' idempotently via `INSERT OR REPLACE`. Without this
-- row, `apply_pending()` sees `row is None` and re-runs migration 001 each
-- time — which is harmless (CREATE TABLE IF NOT EXISTS + INSERT OR REPLACE
-- are idempotent) but confusing.
INSERT OR IGNORE INTO schema_info (key, value) VALUES ('schema_migration_num', '0');
INSERT OR IGNORE INTO schema_info (key, value)
    VALUES ('created_at', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));  -- ISO 8601 UTC

-- ----------------------------------------------------------------------------
-- 1. experiments (primary table; fingerprint-alone PK per R10 refinement)
-- ----------------------------------------------------------------------------
-- experiment_fingerprint is THE identity. experiment_id is a human-readable
-- alias (UNIQUE). Symbol handling:
--   - Single-symbol experiment: symbol = sole symbol; symbols_json = NULL.
--   - Multi-symbol pooled training: symbol = symbols[0] (primary);
--     symbols_json = JSON array (e.g., '["NVDA","HOOD","SNAP"]').
-- cohort_hash is computed by the ingester (NOT in envelope); enables
-- cross-symbol cohort queries (excludes symbol + data_manifest).
CREATE TABLE experiments (
    -- Identity
    experiment_fingerprint    TEXT NOT NULL,                 -- PK, 64-char lowercase hex
    experiment_id             TEXT NOT NULL,                 -- UNIQUE, human-readable
    fingerprint_version       INTEGER NOT NULL DEFAULT 1,

    -- Symbol(s) + asset class
    symbol                    TEXT NOT NULL,                 -- primary / sole symbol
    symbols_json              TEXT,                          -- NULL or JSON array for multi-symbol
    asset_class               TEXT NOT NULL,
    cohort_hash               TEXT,                          -- derived; excludes symbol

    -- Producer metadata
    producer                  TEXT NOT NULL,
    producer_version          TEXT,
    record_type               TEXT NOT NULL,
    envelope_schema_version   TEXT NOT NULL,
    pipeline_contract_version TEXT NOT NULL,

    -- Lifecycle
    status                    TEXT NOT NULL,                 -- pending|running|completed|failed|partial|cancelled
    created_at                TEXT NOT NULL,                 -- ISO 8601 (YYYY-MM-DDTHH:MM:SSZ)
    finalized_at              TEXT,                          -- ISO 8601 or NULL
    heartbeat_at              TEXT,                          -- streaming support
    wall_clock_ms             INTEGER,

    -- Data source + features
    data_source_type          TEXT NOT NULL,
    feature_schema_ref        TEXT NOT NULL,
    dataset                   TEXT,

    -- Model + task
    model_type                TEXT,
    model_family              TEXT,
    task                      TEXT,

    -- Sampling
    sampling_strategy         TEXT,
    sampling_config           TEXT,                          -- JSON blob

    -- Labels
    label_family              TEXT,
    primary_horizon           INTEGER,
    n_horizons                INTEGER NOT NULL DEFAULT 0,
    horizons                  TEXT,                          -- JSON array
    label_spec                TEXT,                          -- JSON blob

    -- Configs
    config_source             TEXT,                          -- raw TOML/YAML/JSON, nullable
    config_format             TEXT,                          -- toml|yaml|json|null

    -- Sweep membership
    sweep_id                  TEXT,
    axis_values               TEXT,                          -- JSON object

    -- Lineage (simple parent; complex lineage in lineage table)
    parent_id                 TEXT,                          -- experiment_id of parent
    upstream_ids              TEXT,                          -- JSON array of experiment_ids

    -- Git state of the producing repo
    git_commit                TEXT,
    git_branch                TEXT,
    git_dirty                 INTEGER,                       -- 0/1

    -- Human narrative
    hypothesis                TEXT NOT NULL DEFAULT '',
    description               TEXT NOT NULL DEFAULT '',

    -- Nested envelope objects (Round 14 C1): stored as canonical JSON strings.
    -- Separate columns (not folded into metadata_json) because these are TYPED
    -- envelope fields with integrity invariants (e.g., §14.8 reads
    -- training_info_json.normalization_stats_sha256). Queryable via SQLite's
    -- json_extract(). NULL when the envelope doesn't carry the nested object
    -- (e.g., export envelopes have no training_info).
    training_info_json        TEXT,                          -- §3.2 training_info object (record_type='training')
    signal_provenance_json    TEXT,                          -- §3.2 signal_provenance object (record_type='backtest')
    strategy_info_json        TEXT,                          -- §3.2 strategy_info object (record_type='backtest')
    export_stats_json         TEXT,                          -- §3.2 export_stats object (record_type='export')

    -- Forward-compat (Round 14 disambiguation: this is producer-specific unknown-fields
    -- catch-all; NOT for nested typed envelope objects above)
    metadata_json             TEXT NOT NULL DEFAULT '{}',

    -- Integrity
    json_record_path          TEXT NOT NULL,                 -- records/{experiment_id}.json
    ingested_at               TEXT NOT NULL,                 -- ISO 8601

    -- Table constraints (AFTER all columns — SQLite syntax requirement)
    PRIMARY KEY (experiment_fingerprint),
    UNIQUE (experiment_id),
    CHECK (status IN ('pending','running','completed','failed','partial','cancelled')),
    CHECK (asset_class IN ('equity','option','future','fx','crypto','synthetic')),
    CHECK (record_type IN ('export','training','analysis','calibration','backtest','evaluation','sweep_aggregate')),
    CHECK (fingerprint_version >= 1),
    FOREIGN KEY (sweep_id) REFERENCES sweeps(sweep_id)
);

-- Indexes on experiments (PK is auto-indexed; do not redeclare).
CREATE INDEX idx_experiments_id ON experiments(experiment_id);
CREATE INDEX idx_experiments_cohort_hash ON experiments(cohort_hash) WHERE cohort_hash IS NOT NULL;
-- Round 14 C1: functional index on training_info_json.normalization_stats_sha256
-- for fast §14.8 integrity lookup. Partial index (only training-type rows).
CREATE INDEX idx_experiments_training_norm_hash
    ON experiments(json_extract(training_info_json, '$.normalization_stats_sha256'))
    WHERE training_info_json IS NOT NULL;
CREATE INDEX idx_experiments_symbol_class_created ON experiments(asset_class, symbol, created_at DESC);
CREATE INDEX idx_experiments_model_family_label ON experiments(model_family, label_family, finalized_at DESC);
CREATE INDEX idx_experiments_sampling ON experiments(sampling_strategy, primary_horizon);
CREATE INDEX idx_experiments_sweep ON experiments(sweep_id) WHERE sweep_id IS NOT NULL;
CREATE INDEX idx_experiments_status_created ON experiments(status, created_at DESC);
CREATE INDEX idx_experiments_producer_type ON experiments(producer, record_type, finalized_at DESC);
CREATE INDEX idx_experiments_pipeline_contract ON experiments(pipeline_contract_version);

-- ----------------------------------------------------------------------------
-- 2. metrics (long-format: one row per metric observation)
-- ----------------------------------------------------------------------------
CREATE TABLE metrics (
    metric_id               INTEGER PRIMARY KEY AUTOINCREMENT,
    experiment_fingerprint  TEXT NOT NULL,

    family                  TEXT NOT NULL,              -- MetricFamily enum
    name                    TEXT NOT NULL,              -- MetricKey enum value
    split                   TEXT,                       -- train|val|test|cv|full|NULL
    horizon                 INTEGER,                    -- NULL for non-horizon metrics
    epoch                   TEXT,                       -- int-as-string|'best'|'final'|NULL
    value                   REAL,                       -- NULL if computed but undefined (e.g., NaN, guarded)
    ci_low                  REAL,
    ci_high                 REAL,
    tag                     TEXT,                       -- class label, sub-model name, etc.

    FOREIGN KEY (experiment_fingerprint)
        REFERENCES experiments(experiment_fingerprint) ON DELETE CASCADE
);
CREATE INDEX idx_metrics_exp ON metrics(experiment_fingerprint);
CREATE INDEX idx_metrics_family_name_split ON metrics(family, name, split);
CREATE INDEX idx_metrics_name_value ON metrics(name, value);
CREATE INDEX idx_metrics_split_horizon ON metrics(split, horizon);

-- ----------------------------------------------------------------------------
-- 3. gates (gate results with ordering + override)
-- ----------------------------------------------------------------------------
CREATE TABLE gates (
    gate_id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    experiment_fingerprint  TEXT NOT NULL,

    gate_name               TEXT NOT NULL,              -- GateKey enum value (underscore form: ic_gt_0_05)
    gate_order              INTEGER NOT NULL,           -- 0, 1, 2, ...
    depends_on_gate         TEXT,                       -- gate_name of prerequisite
    status                  TEXT NOT NULL,              -- pending|skipped|passed|failed|overridden
    threshold               REAL,
    observed                REAL,
    note                    TEXT,

    override_by             TEXT,
    override_reason         TEXT,
    override_at             TEXT,                       -- ISO 8601

    CHECK (status IN ('pending','skipped','passed','failed','overridden')),
    FOREIGN KEY (experiment_fingerprint)
        REFERENCES experiments(experiment_fingerprint) ON DELETE CASCADE
);
CREATE UNIQUE INDEX idx_gates_exp_name ON gates(experiment_fingerprint, gate_name);
CREATE INDEX idx_gates_exp_order ON gates(experiment_fingerprint, gate_order);
CREATE INDEX idx_gates_name_status ON gates(gate_name, status);

-- ----------------------------------------------------------------------------
-- 4. artifacts (references to files produced by experiments)
-- ----------------------------------------------------------------------------
CREATE TABLE artifacts (
    artifact_id             INTEGER PRIMARY KEY AUTOINCREMENT,
    experiment_fingerprint  TEXT NOT NULL,

    kind                    TEXT NOT NULL,              -- open vocabulary
    path                    TEXT NOT NULL,              -- relative or absolute
    bytes                   INTEGER,
    sha256                  TEXT,                       -- content hash if known
    schema_version          TEXT,                       -- optional: for versioned files

    FOREIGN KEY (experiment_fingerprint)
        REFERENCES experiments(experiment_fingerprint) ON DELETE CASCADE
);
CREATE INDEX idx_artifacts_exp ON artifacts(experiment_fingerprint);
CREATE INDEX idx_artifacts_kind ON artifacts(kind);
CREATE INDEX idx_artifacts_sha256 ON artifacts(sha256);

-- ----------------------------------------------------------------------------
-- 5. lineage (cross-repo edges: what an experiment consumed)
-- ----------------------------------------------------------------------------
CREATE TABLE lineage (
    lineage_id              INTEGER PRIMARY KEY AUTOINCREMENT,
    experiment_fingerprint  TEXT NOT NULL,

    source_kind             TEXT NOT NULL,              -- raw_data|export|trainer_run|backtest_run|...
    source_name             TEXT NOT NULL,
    source_hash             TEXT,                       -- content-addressable if resolvable
    source_repo             TEXT NOT NULL,
    source_commit           TEXT,                       -- git SHA or 'not_git_tracked' sentinel
    source_commit_dirty     INTEGER,                    -- 0/1 or NULL

    FOREIGN KEY (experiment_fingerprint)
        REFERENCES experiments(experiment_fingerprint) ON DELETE CASCADE
);
CREATE INDEX idx_lineage_exp ON lineage(experiment_fingerprint);
CREATE INDEX idx_lineage_source_repo_commit ON lineage(source_repo, source_commit);
CREATE INDEX idx_lineage_source_hash ON lineage(source_hash);
CREATE INDEX idx_lineage_source_name ON lineage(source_name);

-- ----------------------------------------------------------------------------
-- 6. bulk_parquet_refs (references to Parquet side files)
-- ----------------------------------------------------------------------------
CREATE TABLE bulk_parquet_refs (
    parquet_ref_id          INTEGER PRIMARY KEY AUTOINCREMENT,
    experiment_fingerprint  TEXT NOT NULL,

    kind                    TEXT NOT NULL,              -- training_curve|feature_ic|equity_curve|...
    path                    TEXT NOT NULL,              -- hft-ops/ledger/metrics/{yyyy_mm}/{hash}.parquet
    schema_json             TEXT,                       -- JSON array of 'col:type' strings
    partition_keys          TEXT,                       -- JSON array
    row_count               INTEGER,
    sha256                  TEXT NOT NULL,              -- content hash; path derivable from this
    bytes                   INTEGER,

    FOREIGN KEY (experiment_fingerprint)
        REFERENCES experiments(experiment_fingerprint) ON DELETE CASCADE
);
CREATE INDEX idx_parquet_exp ON bulk_parquet_refs(experiment_fingerprint);
CREATE INDEX idx_parquet_kind ON bulk_parquet_refs(kind);
CREATE UNIQUE INDEX idx_parquet_sha256 ON bulk_parquet_refs(sha256);

-- ----------------------------------------------------------------------------
-- 7. sweeps (sweep runs group experiments)
-- ----------------------------------------------------------------------------
CREATE TABLE sweeps (
    sweep_id                TEXT PRIMARY KEY,
    name                    TEXT NOT NULL,
    strategy                TEXT NOT NULL,              -- grid|random|latin_hypercube|optuna
    axes_json               TEXT NOT NULL,              -- JSON axis definitions
    created_at              TEXT NOT NULL,              -- ISO 8601
    finalized_at            TEXT,                       -- ISO 8601 or NULL
    status                  TEXT NOT NULL,              -- pending|running|completed|failed|aborted
    n_expected              INTEGER NOT NULL,           -- total grid points
    n_succeeded             INTEGER NOT NULL DEFAULT 0,
    n_failed                INTEGER NOT NULL DEFAULT 0,
    n_skipped               INTEGER NOT NULL DEFAULT 0, -- due to gate early-kill or dedup

    CHECK (status IN ('pending','running','completed','failed','aborted'))
);
CREATE INDEX idx_sweeps_created ON sweeps(created_at DESC);
CREATE INDEX idx_sweeps_status ON sweeps(status, created_at DESC);

-- ----------------------------------------------------------------------------
-- 8. tags
-- ----------------------------------------------------------------------------
CREATE TABLE tags (
    experiment_fingerprint  TEXT NOT NULL,
    tag                     TEXT NOT NULL,

    PRIMARY KEY (experiment_fingerprint, tag),
    FOREIGN KEY (experiment_fingerprint)
        REFERENCES experiments(experiment_fingerprint) ON DELETE CASCADE
);
CREATE INDEX idx_tags_tag ON tags(tag);

-- ----------------------------------------------------------------------------
-- 9. notes (append-only human narrative)
-- ----------------------------------------------------------------------------
CREATE TABLE notes (
    note_id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    experiment_fingerprint  TEXT NOT NULL,
    author                  TEXT,                       -- optional; e.g., 'nagarx' or 'claude-opus'
    created_at              TEXT NOT NULL,              -- ISO 8601
    body_md                 TEXT NOT NULL,

    FOREIGN KEY (experiment_fingerprint)
        REFERENCES experiments(experiment_fingerprint) ON DELETE CASCADE
);
CREATE INDEX idx_notes_exp ON notes(experiment_fingerprint, created_at DESC);

-- ----------------------------------------------------------------------------
-- 10. fingerprint_history (multi-version fingerprint tracking)
-- ----------------------------------------------------------------------------
-- When fingerprint algorithm version bumps (rare), old fingerprints are
-- preserved here so lookups work across versions.
CREATE TABLE fingerprint_history (
    experiment_id           TEXT NOT NULL,
    fingerprint_version     INTEGER NOT NULL,
    fingerprint_value       TEXT NOT NULL,
    computed_at             TEXT NOT NULL,              -- ISO 8601

    PRIMARY KEY (experiment_id, fingerprint_version)
);
CREATE INDEX idx_fingerprint_history_value ON fingerprint_history(fingerprint_value);

-- ----------------------------------------------------------------------------
-- NOT a SQLite table: ingest_audit is an append-only JSONL log file at
-- hft-ops/ledger/ingest.log (one record per ingest attempt). Rationale:
-- audit data is write-heavy, query-light; a log file is simpler, faster, and
-- does not compete for the SQLite write lock. Rotate monthly (ingest.log →
-- ingest.log.YYYY_MM). Query via `hft-ops ledger audit-log --since YYYY-MM-DD`.
-- Each line (UTF-8 newline-delimited JSON):
--   {
--     "ingested_at": "2026-04-15T14:03:22Z",
--     "envelope_content_hash": "<sha256 hex>",
--     "experiment_id": "TLOB_...",
--     "experiment_fingerprint": "<sha256 hex>",
--     "envelope_version": 1,
--     "ingested_by": "nagarx",
--     "duration_ms": 47,
--     "result": "inserted",           // inserted|duplicate_idempotent|duplicate_pk|rejected|error
--     "note": null
--   }
-- ----------------------------------------------------------------------------
```

### §5.4 Parquet Side Files

**Path convention**: `hft-ops/ledger/metrics/{yyyy_mm}/{content_hash}.parquet`

- `yyyy_mm` derived from `experiments.created_at` — ensures any given experiment's files land in one partition.
- `content_hash = sha256(parquet_bytes)` → identical arrays dedup to a single file on disk.

**Canonical kinds** (matching `bulk_parquet[].kind` enum):

| Kind | Typical rows | Columns | Producer |
|---|---|---|---|
| `training_curve` | 10–100 (epochs) | epoch, train_loss, val_loss, train_acc, val_acc, val_macro_f1, lr | trainer |
| `feature_ic` | 34–148 (features) | feature_idx, feature_name, ic, dcor, mi, horizon, significance, tier | evaluator |
| `predictions_summary` | 1k–100k (samples) | sample_idx, y_true, y_pred, y_pred_proba | trainer |
| `per_day_export_stats` | 233 (days) | day, n_bins_total, n_bins_valid, n_records, consolidated_volume, ... | BQP / MBO extractor |
| `equity_curve` | 10k–100k (timesteps) | timestep, equity, pnl_bps, trade_active | backtester |
| `trade_log` | 100–10k (trades) | trade_id, entry_ts, exit_ts, pnl_bps, exit_reason | backtester |
| `posterior_samples` | 1k–100k (samples) | sample_idx, param_name, value | Bayesian trainer |

**Rules**:
- Size target: < 10 MB per file; hard limit 100 MB.
- Compression: `snappy` (default for pandas).
- Schema evolution: additive only within a `kind`; breaking changes require new `kind`.
- Reading: always use explicit column projection (`read_table(columns=[...])`) for tolerance.

### §5.5 Write Order (Normative — must match §7 exactly)

Every ingest follows this exact order for atomicity:

```
1. Validate envelope against JSON Schema (§3.2). REJECT if invalid → quarantine.
2. Append-only JSON record: write tmp → fsync → rename to records/{experiment_id}.json.
3. Write Parquet files (if any) with content-addressable filenames.
4. BEGIN SQLite transaction.
5. INSERT OR IGNORE into experiments, metrics, gates, artifacts, lineage, bulk_parquet_refs, tags.
6. COMMIT.
7. Move envelope from inbox/ to records/ (atomic rename).
```

Step 1 failure → envelope to quarantine/.
Step 2 failure → abort; nothing written.
Step 3 failure → step 2 JSON record orphaned (rebuildable); retry.
Step 5-6 failure → JSON record + Parquet exist; SQLite rollback. `hft-ops ledger rebuild` recovers.
Step 7 failure (rare) → envelope remains in inbox; re-ingest is idempotent (INSERT OR IGNORE + content-addressable Parquet).

### §5.6 Rebuild Semantics

`hft-ops ledger rebuild` MUST produce a functionally equivalent SQLite DB from `records/*.json` alone (ignoring Parquet presence). Implementation:

```python
def rebuild(records_dir: Path, sqlite_path: Path) -> None:
    # Atomic swap: write to .new, then rename
    new_db = sqlite_path.with_suffix('.new')
    conn = sqlite3.connect(new_db)
    apply_schema(conn)
    for record_file in sorted(records_dir.glob('*.json')):
        raw = record_file.read_text()
        try:
            # Round 15 A2: rebuild MUST re-validate through Pydantic and use the
            # SAME insert-helpers as §7.3. Otherwise the rebuilt
            # training_info_json (etc.) serialization may diverge from the
            # original at-ingest serialization — breaking §14.8's json_extract
            # hash lookup. Do NOT bypass Pydantic by passing a raw dict.
            validated = validate_envelope_v1(json.loads(raw))   # same path as §7.3
            insert_experiment(conn, validated)                  # §7.3.1 INSERT template
            insert_metrics(conn, validated)
            insert_gates(conn, validated)
            insert_artifacts(conn, validated)
            insert_lineage(conn, validated)
            insert_bulk_parquet_refs(conn, validated)
            insert_tags(conn, validated)
            cohort_hash = compute_cohort_hash(validated)
            conn.execute(
                "UPDATE experiments SET cohort_hash = ? WHERE experiment_fingerprint = ?",
                (cohort_hash, validated.experiment_fingerprint),
            )
        except (ValidationError, SchemaViolation) as e:
            log.warn(f"Skip {record_file.name}: {e}")
    conn.commit()
    conn.close()
    os.replace(new_db, sqlite_path)  # atomic
```

Rebuild does NOT regenerate Parquet files. If Parquet is missing, `artifacts`/`bulk_parquet_refs` rows still point to (missing) paths; `hft-ops ledger check` surfaces the inconsistency.

**Round 15 A2 invariant**: rebuild re-runs the exact Pydantic validation + insert path used at ingest (§7.3). This is the ONLY way to guarantee `training_info_json`, `signal_provenance_json`, etc. are serialized with byte-identical canonical form on rebuild — which is what §14.8's `json_extract(training_info_json, '$.normalization_stats_sha256')` hash lookup depends on. A raw-dict-passthrough rebuild (prior pseudocode) could silently produce non-canonical JSON (e.g., different key order) → §14.8 lookup returns `None` → spurious `UpstreamNotYetIngestedError`. Tested by `test_fm_10_orphaned_record_heal_recovers` at §18.3.

### §5.7 Schema Migration Protocol (Round 13 — B6)

**Problem**: §5.3 uses bare `CREATE TABLE ...` (not `IF NOT EXISTS` except for `schema_info`). When schema version bumps v1.0.0 → v1.1.0, opening an existing `ledger.sqlite` with v1.1.0 code fails with `table already exists`. There is no migration framework today.

**Design**:

1. **Schema version is recorded in `schema_info` row** (`key='schema_version'`, value = semver string like `"1.0.0"`).
2. **Migrations live as numbered SQL files** at `hft-ops/src/hft_ops/ledger/migrations/NNN_*.sql`:
   - `001_initial_v1_0_0.sql` — creates all 10 tables (this IS §5.3's DDL).
   - `002_add_exchange_column.sql` — future Phase 11.5 for B8.
   - `003_...` — etc.
   Each file is immutable once committed. Never edit in place; always add a new numbered file.
3. **Migration runner** (`hft_ops.ledger.migrations.apply_pending`): reads current `schema_info.schema_version`, finds all `migrations/NNN_*.sql` with N greater than current version, applies each in order within a single transaction, and updates `schema_info.schema_version` at the end.
4. **CLI**: `hft-ops ledger migrate-schema [--to VERSION] [--dry-run]`. Default `--to` is "latest on disk."
5. **Idempotency**: re-applying a migration is a no-op because `schema_info.schema_version` advances. `001_initial_v1_0_0.sql` uses `CREATE TABLE IF NOT EXISTS` for bootstrap robustness; later migrations use `ALTER TABLE` semantics.
6. **Failure recovery**: if a migration fails mid-execution, the transaction rolls back (single-statement migrations only, OR explicit BEGIN/COMMIT for multi-statement). The `schema_info.schema_version` is NOT advanced, so a re-run picks up where it left off. For corrupted states, fall back to `hft-ops ledger rebuild --from-records --backup` (destroys SQLite, rebuilds from append-only JSON records; preserves the invariant that records/ is source of truth).

**Reference implementation**:

```python
# hft_ops/ledger/migrations/__init__.py
import re
from pathlib import Path
from typing import Iterator

MIGRATIONS_DIR = Path(__file__).parent
MIGRATION_FILENAME_RE = re.compile(r"^(\d{3})_.*\.sql$")

def discover_migrations() -> Iterator[tuple[int, Path]]:
    """Yield (version_number, migration_path) sorted by version."""
    entries = []
    for f in MIGRATIONS_DIR.glob("*.sql"):
        m = MIGRATION_FILENAME_RE.match(f.name)
        if m:
            entries.append((int(m.group(1)), f))
    entries.sort()
    yield from entries

def current_version(conn: sqlite3.Connection) -> int:
    try:
        row = conn.execute(
            "SELECT value FROM schema_info WHERE key = 'schema_migration_num'"
        ).fetchone()
        return int(row[0]) if row else 0
    except sqlite3.OperationalError:
        return 0  # schema_info table doesn't exist yet (fresh DB)

def apply_pending(conn: sqlite3.Connection) -> list[int]:
    """Apply all migrations with number > current. Returns list of applied migration numbers."""
    applied = []
    curr = current_version(conn)
    for num, path in discover_migrations():
        if num <= curr:
            continue
        sql = path.read_text()
        with conn:  # transaction
            conn.executescript(sql)
            conn.execute(
                "INSERT OR REPLACE INTO schema_info (key, value) VALUES ('schema_migration_num', ?)",
                (str(num),),
            )
        applied.append(num)
    return applied
```

**Semver pairing (Round 14 C6 — normative rules):**

- `schema_info.schema_version` (semver string, e.g., `"1.0.0"`) is the PUBLIC version exposed to tooling + documentation.
- `schema_info.schema_migration_num` (monotonic integer) is the INTERNAL counter the migration runner uses to skip-already-applied.
- **Bootstrap**: on fresh DB, §5.3 DDL initializes BOTH `schema_version='1.0.0'` AND `schema_migration_num='0'`. Migration `001_initial_v1_0_0.sql` (which is a wrapper around §5.3 DDL) advances `schema_migration_num` to `'1'` idempotently — it can run before or after the raw DDL and either way converges.
- **Every migration file MUST update BOTH keys as its LAST statements**:
  ```sql
  -- Migration body: CREATE TABLE / ALTER TABLE / CREATE INDEX / ...
  ...
  -- Required tail (Round 14 rule):
  INSERT OR REPLACE INTO schema_info (key, value) VALUES ('schema_migration_num', '<NNN>');
  INSERT OR REPLACE INTO schema_info (key, value) VALUES ('schema_version', '<semver>');
  ```
- **Semver bump rule**:
  - **Patch** (`1.0.0 → 1.0.1`): non-functional change (fix typo in a CHECK constraint, add a docstring). RARE.
  - **Minor** (`1.0.0 → 1.1.0`): additive — new table, new NULLABLE column, new index. Backward-compat (old code reads new DB fine; missing columns returned as NULL).
  - **Major** (`1.0.0 → 2.0.0`): breaking — column rename, column type change, DROP column, tightened CHECK constraint, NOT NULL on existing nullable column, new required enum value. Old code FAILS against new DB.
- **Atomic-commit rule** (Round 14 C6 — Agent C operational gap O7): every schema change MUST land in ONE commit: the TOML update (if envelope schema changes) + Python codegen output + Rust struct update + `NNN_*.sql` migration file. CI runs `verify_rust_envelope_schema.py` on every commit; a split-across-commits change is red until all pieces land. Enforcement: a pre-merge CI gate that requires either (a) none of `{pipeline_contract.toml, hft_contracts/orchestration/*, rust/orchestration/*, migrations/*.sql}` changed, OR (b) all four touched.
- **Round 15 A8 — local dev workflow**: developers are NOT required to install a pre-commit hook for the atomic-commit gate. Workflow: operate on a feature branch, interim commits MAY be red on CI (commit-1 updates TOML only, commit-2 adds codegen output, etc.), the PRE-MERGE check (final commit before merge to main) MUST be green. Land-as-series to a feature branch is the expected pattern. Direct commits to main are forbidden by branch protection.

**Test** (§18.2 Migration, 4 tests):
- Fresh DB → `apply_pending` runs all migrations → `schema_migration_num` matches highest on disk.
- Partial DB (migrations 1-2 applied, 3 on disk) → `apply_pending` runs only 3.
- Migration syntax error → transaction rolls back, `schema_migration_num` unchanged, re-run succeeds after fix.
- Re-running a successful migration → no-op (idempotent).

**What this replaces**: the previous design relied on `CREATE TABLE` without migration support, making any schema change a destructive rebuild. §5.7 makes schema evolution incremental and reversible (within SQLite limits — additive changes only; removing a column still requires rebuild).

---

## §6 — Fingerprint Alignment with Phase 3

### §6.1 Authoritative Fingerprint Source

Phase 3's `compute_fingerprint()` (`hft-ops/src/hft_ops/ledger/dedup.py:284-391`) is the ONLY fingerprint authority. Phase 10 REUSES it verbatim.

Algorithm:

```python
def compute_fingerprint(manifest: ExperimentManifest, paths: ResolvedContext) -> str:
    """Phase 3 — resolved _base: configs, then hashed."""
    components = {
        "extraction": _extract_extraction_config(manifest),
        "training": _load_trainer_config_resolved(manifest, paths),   # §3.3b: resolves _base:
        "backtest": _extract_backtest_params(manifest),
        "data_manifest": _hash_data_directory(manifest.extraction.output_dir),
        "contract_version": manifest.contract_version,
    }
    # Apply exclusion set (§6.2)
    cleaned = _extract_fingerprint_fields(components)
    serialized = json.dumps(cleaned, sort_keys=True, default=str)
    return hashlib.sha256(serialized.encode("utf-8")).hexdigest()
```

### §6.2 Exclusion Set (preserved verbatim from Phase 3)

Fields EXCLUDED from fingerprint computation:

- Top-level: `{name, description, tags, version, output_dir, log_level, verbose, experiment}`.
- Entire `stages.validation` subtree (validation is observation, not treatment).
- `provenance.retroactive` flag (retroactive vs. live should not affect identity).
- `created_at`, `finalized_at`, `heartbeat_at`, `wall_clock_ms` (time-derived).

Any addition to this list requires:
1. Design review (explicit change proposal).
2. `fingerprint_version` bump.
3. Migration: compute new hashes for existing records, store both in `fingerprint_history` table.

### §6.3 Phase 10 Envelope's Relationship to Fingerprint

The envelope `experiment_fingerprint` field is **populated by the producer** using Phase 3's algorithm. It is NOT recomputed by the ingester.

**Consequence**: Producers must call `hft_ops.fingerprint.compute_fingerprint()` (or its Rust equivalent for BQP) at emit time. This cross-module dependency is acceptable because:
- Producers already compute fingerprints today (just for local tracking).
- The algorithm is simple (JSON serialization + SHA-256), implementable in any language.
- `hft-contracts` can provide a reference implementation per language.

### §6.4 Multi-Symbol Fingerprint Strategy

**Phase 10 Decision: fingerprint INCLUDES symbol (via InputConfig.symbol in extraction config)** — preserves Phase 3 verbatim with zero algorithm change.

**Consequence for the data model (Round 10 refinement):**

| Case | Storage shape | Example |
|---|---|---|
| **Single-symbol** (the default today) | 1 row in `experiments`; `symbol = X`; `symbols_json = NULL`. Different symbols produce different fingerprints via extraction config → no PK collision. | NVDA run, HOOD run → 2 rows, 2 distinct fingerprints |
| **Multi-symbol pooled** (Phase 12 capability) | 1 row in `experiments`; `symbol = symbols[0]` (primary); `symbols_json = '["NVDA","HOOD","SNAP"]'`. One fingerprint represents the pooled config. | Pooled NVDA+HOOD+SNAP training → 1 row, 1 fingerprint |
| **Symbol sweep** (N single-symbol runs that differ only in symbol) | N rows; same `cohort_hash`; different `experiment_fingerprint` per row. | Sweep over {NVDA, HOOD, SNAP} → 3 rows |

**Cohort hash** (computed by the ingester, NOT in envelope):

```python
# sha256 of fingerprint components EXCLUDING symbol and data_manifest; identifies
# cross-symbol cohorts of otherwise-identical experiments.
cohort_hash = sha256(json.dumps(
    {k: v for k, v in fingerprint_components.items()
     if k not in ("extraction_symbol", "data_manifest")},
    sort_keys=True, default=str,
).encode()).hexdigest()
```

Cross-symbol cohort query:

```sql
SELECT symbol, experiment_fingerprint, experiment_id
FROM experiments
WHERE cohort_hash = ?
ORDER BY symbol;
```

Multi-symbol membership query (either pooled or sweep):

```sql
SELECT experiment_id, symbol, symbols_json
FROM experiments
WHERE symbol = ? OR symbols_json LIKE '%"' || ? || '"%';
```

### §6.4a Fingerprint Composition per record_type (Round 11 Agent B BQ7)

Phase 3's `compute_fingerprint()` already composes the fingerprint from `{extraction, training, backtest, data_manifest, contract_version}` for a fully-populated manifest. However, single-stage producers (backtest-only, evaluator-only) often have SPARSE manifests — the backtest stage is populated but `extraction` and `training` are empty. Without upstream integration, two backtests with identical `backtest_params` but DIFFERENT upstream trainers would produce the SAME fingerprint → a collision.

**Rule**: every producer MUST include the `sorted(upstream_fingerprints)` list as an explicit component of its fingerprint input when the producer consumes one or more ledger-resident upstream experiments.

**Authoritative component set per record_type** (amends `compute_fingerprint` behavior in dedup.py; enforced on emit, verified on ingest):

| record_type | Required components | Optional components |
|---|---|---|
| `export` | `extraction`, `data_manifest`, `contract_version` | `training` if extractor config declares downstream-affecting knobs |
| `training` | `training`, `data_manifest`, `contract_version`, `upstream_fingerprints` (exports) | `extraction` if inlined |
| `backtest` | `backtest`, `contract_version`, `upstream_fingerprints` (trainers) | — |
| `evaluation` | `evaluation`, `data_manifest`, `contract_version`, `upstream_fingerprints` (exports) | — |
| `analysis` | `analysis`, `data_manifest`, `contract_version`, `upstream_fingerprints` (if any) | — |
| `calibration` | `calibration`, `contract_version`, `upstream_fingerprints` (parent training) | — |
| `sweep_aggregate` | `sweep_id`, `contract_version`, `upstream_fingerprints` (each grid point) | — |

Where `upstream_fingerprints = sorted(set(resolve_upstream_ref(d).experiment_fingerprint for d in upstream_artifact_dirs))` — dedup + sort ensures determinism.

**dedup.py change**: `compute_fingerprint` accepts an optional `upstream_fingerprints: list[str]` kwarg; when present, it's serialized into `components["upstream_fingerprints"]`. Phase 10 Step 3 of §13.1 migration sets this kwarg on all producer call sites. Existing 34 records are grandfathered (fingerprint preserved verbatim; `upstream_fingerprints` NOT added retroactively).

**Test** (§18.2): two backtests with identical `backtest_params` but different upstream trainers produce DIFFERENT fingerprints. Two backtests with identical params AND identical upstream trainers produce the SAME fingerprint (dedup works correctly).

### §6.5 Fingerprint Version Bumping Protocol

If the algorithm must change (e.g., canonicalization fix, exclusion-set revision):

1. Author RFC document in `docs/plan/` describing the change and rationale.
2. Update `dedup.py` to compute v2.
3. Add `fingerprint_version = 2` column updates everywhere.
4. `hft-ops ledger verify-fingerprints` runs for all records: if v1 hash is stored and v2 computation differs, record flags as "fingerprint algorithm drift; v1 preserved".
5. Insert new row into `fingerprint_history` with (experiment_id, 2, <new_hash>, <timestamp>).
6. Lookups: `SELECT * FROM experiments WHERE experiment_fingerprint = ?` returns matches on ANY version (JOIN with `fingerprint_history`).

Existing records' fingerprints are NEVER recomputed in-place. Migration is ADDITIVE only.

### §6.6 Example: Two Experiments, One Config

Scenario: user runs TLOB on NVDA with point-return labels at bin_size=60s on 2026-04-15, and again on 2026-04-20 with identical config.

Expected behavior:
- Both runs compute `experiment_fingerprint = X` (same because config + data manifest identical, assuming same data snapshot).
- First ingest: INSERT succeeds; record in experiments table; `cohort_hash = C` computed.
- Second ingest: INSERT fails with `IntegrityError` on PK (`experiment_fingerprint` already present). Handler returns `duplicate_pk` outcome; envelope removed from inbox; one line appended to `ingest.log`; producer gets a warning.
- Query `SELECT * FROM experiments WHERE experiment_fingerprint = X` returns one row.

Scenario modification: second run uses a NEWER data snapshot.
- `data_manifest` hash differs → fingerprint differs → two distinct rows (each with its own `experiment_fingerprint` PK).

Scenario modification: second run is on HOOD instead of NVDA.
- Symbol is in extraction config → fingerprint differs → two distinct rows: PKs `fingerprint_A` (NVDA) and `fingerprint_B` (HOOD).
- `cohort_hash` for both is identical (excludes symbol) — enables `WHERE cohort_hash = C` to return both rows.

---

## §7 — Ingestion Pipeline

### §7.1 Producer Responsibilities

A producer MUST:

1. Compute `experiment_fingerprint` at emit time (via Phase 3 algorithm).
2. Construct a valid envelope conforming to `hft_contracts.orchestration.Envelope` Pydantic model.
3. Compute envelope `content_hash = sha256(envelope_json_without_metadata_json)` (uses canonical JSON per §3.3.6).
4. Write Parquet side files FIRST (if any) — content-addressable names to `hft-ops/ledger/metrics/{yyyy_mm}/{parquet_hash}.parquet`, with `fsync` after close. Parquet MUST be flushed before the envelope is placed in inbox. See §7.4.1 for the rollback matrix when this ordering is violated.
5. Write envelope atomically: `open(f"{content_hash}.json.tmp")` → write + fsync → `os.replace("{content_hash}.json")`.
6. Place envelope file in `hft-ops/ledger/inbox/`.
7. For `record_type='backtest'` and `record_type='training'` that consumes an upstream export/training: resolve every upstream producer's `experiment_fingerprint` via the §7.1.1 API and populate `signal_provenance.trainer_fingerprint` / `upstream_experiment_ids` / `lineage[].source_hash` accordingly — NEVER leave these as free-form strings when the upstream is ledger-resident.

A producer SHOULD:
- Log envelope write to stderr with `content_hash` for debugging.
- Not batch envelopes (each experiment writes its own envelope).
- Use `HFT_OPS_ORCHESTRATED=1` env var convention to detect orchestrated invocation (reused from current `stages/base.py:138`).

A producer MUST NOT:
- Write directly to `ledger.sqlite`.
- Write directly to `records/*.json`.
- Delete or modify another producer's envelope.
- Block on ingest completion (ingest is async from producer's perspective).

### §7.1a Inbox Path Resolution (Round 13 — cross-repo producers)

**Problem**: the parent monorepo at `/Users/knight/code_local/HFT-pipeline-v2/` is NOT git-tracked (per root CLAUDE.md). Producers live at various locations:
- `hft-ops` (parent monorepo relative path)
- `lob-model-trainer` (parent monorepo relative path)
- `lob-backtester` (parent monorepo relative path)
- `basic-quote-processor` (**standalone git repo** at `github.com/nagarx/basic-quote-processor.git`)
- `feature-extractor-MBO-LOB` (parent monorepo relative path, but with standalone workspace)
- future OPRA feature-extractor (**standalone git repo**)

A standalone-repo producer (BQP, future OPRA) cloned on a different machine has no `hft-ops/ledger/inbox/` visible via relative path. Every producer must have a uniform way to discover the inbox location.

**Resolution order** (producers implement this verbatim; deviations are bugs):

1. **Explicit CLI argument `--ledger-inbox <absolute_path>`** — highest precedence. For scripts that want deterministic path regardless of environment.
2. **Env var `HFT_OPS_LEDGER_INBOX=<absolute_path>`** — set by the orchestrator (`hft-ops run`) before spawning the producer subprocess. Matches the §14.5 `HFT_OPS_ORCHESTRATED=1` pattern.
3. **Fallback** — if neither (1) nor (2) is set:
   - If `HFT_OPS_ORCHESTRATED=1` is set AND inbox is unset → **HARD FAIL** with a precise error message. Rationale: orchestration explicitly promised envelope emission; silent skip under orchestration is a data-integrity hole.
   - If `HFT_OPS_ORCHESTRATED` is unset → **WARN and skip** envelope emission. Rationale: standalone testing (e.g., `cargo test` inside BQP, or running `export_dataset` manually for a one-off) should not fail just because inbox isn't configured. Producer still writes its native artifacts (`metadata.json`, NPY files, etc.) — only the envelope is skipped.

**Reference implementation** (Rust; identical semantics in Python):

```rust
pub fn resolve_inbox_path(cli_arg: Option<&std::path::Path>) -> Result<Option<std::path::PathBuf>, ProcessorError> {
    if let Some(p) = cli_arg {
        return Ok(Some(p.to_path_buf()));
    }
    if let Ok(env_val) = std::env::var("HFT_OPS_LEDGER_INBOX") {
        return Ok(Some(std::path::PathBuf::from(env_val)));
    }
    // Orchestrated but inbox missing = HARD FAIL (fail-fast per hft-rules §5)
    if std::env::var("HFT_OPS_ORCHESTRATED").as_deref() == Ok("1") {
        return Err(ProcessorError::config(
            "HFT_OPS_ORCHESTRATED=1 but HFT_OPS_LEDGER_INBOX is unset. \
             Orchestrator must set HFT_OPS_LEDGER_INBOX when spawning producer subprocesses. \
             This guard prevents silent envelope loss under orchestration.",
        ));
    }
    // Standalone (e.g., `cargo test`, manual CLI without orchestration): WARN + skip
    log::warn!(
        "HFT_OPS_LEDGER_INBOX unset and not running under orchestration; \
         envelope emission skipped (standalone mode). \
         To enable, set HFT_OPS_LEDGER_INBOX=<abs_path> or pass --ledger-inbox."
    );
    Ok(None)
}
```

**hft-ops orchestrator requirement**: whenever `hft-ops run` spawns a producer subprocess, it MUST set both `HFT_OPS_ORCHESTRATED=1` (preserving existing §14.5 convention) AND `HFT_OPS_LEDGER_INBOX=<abs_path_to>/hft-ops/ledger/inbox/`. This is enforced in `hft-ops/src/hft_ops/stages/base.py` (the subprocess-launch helper) alongside the existing env-var setup.

**Test** (§18.2 inbox resolution, 4 tests):
- CLI arg takes precedence over env var.
- Env var used when CLI arg absent.
- `HFT_OPS_ORCHESTRATED=1` + unset inbox → `ConfigError` raised.
- No `HFT_OPS_ORCHESTRATED` + unset inbox → WARN log, function returns `None`; caller skips envelope emission.

### §7.1.1 Upstream Fingerprint Resolution API (Round 11 Agent B BQ7)

**Problem**: a backtester at emit time knows the path to its signal directory (e.g., `lob-model-trainer/outputs/TLOB_E5_20260313T061500_...`) but NOT the `experiment_fingerprint` of the trainer that produced that signal. Without a programmatic way to look up the upstream fingerprint, lineage claims in the envelope degenerate into free-form strings (`source_name` only), and `§14.9` upstream integrity cannot be enforced.

**Solution**: every producer drops a small `.envelope-ref` breadcrumb file alongside its primary artifact directory. Consumers read this file to resolve upstream fingerprints.

**Breadcrumb file format** (`{artifact_dir}/.envelope-ref`):

```json
{
  "experiment_fingerprint": "a3b8f1c4d5e6...64hex...",
  "experiment_id": "TLOB_Regression_E5_20260313T061500_a3b8f1c4",
  "producer": "lob-model-trainer",
  "producer_version": "2.1.0",
  "record_type": "training",
  "envelope_content_hash": "b8c2d3e4...64hex...",
  "inbox_path": "hft-ops/ledger/inbox/b8c2d3e4...json",
  "emitted_at": "2026-03-13T06:15:00Z"
}
```

Written **before** the envelope reaches the inbox (step 5 of §7.1), so consumers invoked by the same orchestration run can find it immediately. Atomically via tmp+rename.

**Consumer API** (Python — `hft_contracts/orchestration/upstream.py`):

```python
from pathlib import Path
import json
from typing import NamedTuple, Optional

class UpstreamRef(NamedTuple):
    experiment_fingerprint: str
    experiment_id: str
    producer: str
    producer_version: str
    record_type: str
    envelope_content_hash: str
    inbox_path: str
    emitted_at: str

class UpstreamResolutionError(LookupError):
    """Raised when an upstream artifact directory has no .envelope-ref breadcrumb AND is not listed as a lineage source_kind in {"raw_data", "reconstructor"}."""

def resolve_upstream_ref(artifact_dir: Path) -> UpstreamRef:
    """Read the upstream producer's .envelope-ref breadcrumb.

    Raises UpstreamResolutionError if the breadcrumb is missing; callers MUST
    either (a) handle the error and set source_kind="raw_data" / "reconstructor"
    (free-form source_name permitted), or (b) fail-fast because the upstream
    is expected to be ledger-resident.
    """
    ref_path = artifact_dir / ".envelope-ref"
    if not ref_path.exists():
        raise UpstreamResolutionError(
            f"no .envelope-ref at {artifact_dir}. If this is ledger-resident "
            f"upstream, the producer failed to emit the breadcrumb. If this is "
            f"raw data or reconstructor output, use source_kind='raw_data' or "
            f"'reconstructor' and free-form source_name instead."
        )
    data = json.loads(ref_path.read_text())
    return UpstreamRef(**data)
```

**Rust equivalent** (`hft_contracts_rs::orchestration::upstream::resolve_upstream_ref`) — same semantics, `serde_json` for parse, custom error type.

**When a producer writes this breadcrumb** (mandatory):
- BQP, MBO extractor at the root of the export directory (`data/exports/{name}/.envelope-ref`).
- Trainer at the root of the output directory (`lob-model-trainer/outputs/{exp_id}/.envelope-ref`).
- Backtester at the root of the metrics directory (`lob-backtester/outputs/{exp_id}/.envelope-ref`).
- Evaluator at the root of the report directory.

**When a consumer reads it**:
- Trainer loading an export → reads `{export_dir}/.envelope-ref` → populates `lineage[]` with `source_hash = upstream_ref.experiment_fingerprint`, `source_kind = "export"`.
- Backtester loading a signal → reads `{signal_dir}/.envelope-ref` → populates `signal_provenance.trainer_fingerprint` + `upstream_experiment_ids = [upstream_ref.experiment_id]`.
- Evaluator loading exports → same pattern.

**Invariant**: if `lineage[].source_kind ∈ {"export", "trainer_run", "backtest_run", "evaluator_run"}` then `lineage[].source_hash` MUST be a 64-char hex (resolves to `experiments.experiment_fingerprint` OR `fingerprint_history.fingerprint_value`). Enforced by §14.9.

**Test** (§18.2 Upstream-resolution tests, 4 new):
- Producer emits breadcrumb → consumer reads → round-trip equality on experiment_fingerprint.
- Breadcrumb missing → `UpstreamResolutionError` raised, consumer falls back to free-form source_name.
- Breadcrumb present but malformed → `UpstreamResolutionError`.
- Producer writes breadcrumb atomically (power-cut mid-write → consumer sees either the old breadcrumb or no breadcrumb, never a corrupted one).

### §7.2 Ingest CLI

```bash
# Process all envelopes in inbox
hft-ops ledger ingest

# Limit batch size
hft-ops ledger ingest --batch-size 100

# Dry-run: validate envelopes, report what WOULD be inserted
hft-ops ledger ingest --dry-run

# Verbose: print per-envelope diagnostics
hft-ops ledger ingest --verbose

# Run as daemon (polls inbox every N seconds)
hft-ops ledger ingest --daemon --poll-interval 10
```

### §7.3 Ingest Algorithm

Pseudocode (Python):

```python
def ingest_all(ledger_dir: Path, batch_size: int = 100, dry_run: bool = False) -> IngestReport:
    """Ingest pending/ first (retries for out-of-order upstreams), then inbox/.

    Round 13 (D4 option α): every `hft-ops ledger ingest` call scans pending/
    before inbox/. This gives UpstreamNotYetIngestedError envelopes a
    deterministic retry cadence without a background daemon.
    """
    # Acquire exclusive lock (ingest is serialized; producers write inbox lock-free)
    with flock_exclusive(ledger_dir / "ledger.lock", timeout=5.0) as lock:
        report = IngestReport()

        # Step 1 (NEW): retry pending/ — envelopes waiting for upstream to arrive
        pending_dir = ledger_dir / "pending"
        pending_dir.mkdir(exist_ok=True)
        for envelope_path in sorted(pending_dir.glob("*.json")):
            # Age-out check: > 24h → escalate to quarantine
            age_hours = (time.time() - envelope_path.stat().st_mtime) / 3600
            if age_hours > 24:
                _escalate_pending_to_quarantine(
                    envelope_path, ledger_dir,
                    reason=f"upstream never arrived after {age_hours:.1f}h",
                )
                report.record(IngestOneResult.age_out_quarantined(envelope_path))
                continue
            # Retry: move back to inbox, then re-run ingest_one on it
            retry_target = ledger_dir / "inbox" / envelope_path.name
            os.replace(envelope_path, retry_target)
            try:
                result = ingest_one(retry_target, ledger_dir, dry_run=dry_run)
                report.record(result)
            except Exception as e:
                log.exception(f"Pending-retry failed: {retry_target.name}")
                report.error(retry_target, e)

        # Step 2: process inbox (the main ingest path)
        inbox = sorted((ledger_dir / "inbox").glob("*.json"))
        for envelope_path in inbox[:batch_size]:
            try:
                result = ingest_one(envelope_path, ledger_dir, dry_run=dry_run)
                report.record(result)
            except Exception as e:
                # Unexpected error: preserve envelope, log, continue
                log.exception(f"Ingest failed: {envelope_path.name}")
                report.error(envelope_path, e)
        report.finalize()
        return report

def ingest_one(envelope_path: Path, ledger_dir: Path, dry_run: bool = False) -> IngestOneResult:
    # Step 1: load + validate
    t0 = time.perf_counter()
    envelope = json.load(envelope_path.open())
    version = envelope.get("envelope_version")
    handler = INGEST_HANDLERS.get(version)
    if handler is None:
        return quarantine(envelope_path, f"Unsupported envelope_version={version}")
    try:
        validated = validate_envelope_v1(envelope)  # Pydantic
    except ValidationError as e:
        return quarantine(envelope_path, f"Schema: {e}")

    if dry_run:
        return IngestOneResult.would_insert(validated)

    # Step 2: append-only JSON record (atomic tmp+rename). Canonical form comparison for idempotency.
    # Round 13 (D3/B4) + Round 16 (v1.3.3): both sides compared via SSoT canonical form from
    # hft_contracts.canonical_hash (frozen contract; NOT a new canonical_json_dumps). Raw inbox file
    # may have whitespace/key-order variation even if semantically identical; Pydantic roundtrip
    # + canonical_json_blob canonicalizes. See §3.3.6 invariants.
    from hft_contracts.canonical_hash import canonical_json_blob, sha256_hex
    record_path = ledger_dir / "records" / f"{validated.experiment_id}.json"
    canonical_new = canonical_json_blob(
        validated.model_dump(by_alias=False, exclude_none=False),
        sanitize=True,  # NaN/Inf → None (envelope.metrics[].value may be NaN)
    ).decode('utf-8')
    if record_path.exists():
        canonical_existing = record_path.read_text()
        if canonical_existing == canonical_new:
            envelope_path.unlink()
            audit_log_append(ledger_dir, validated, result="duplicate_idempotent",
                             duration_ms=int(1000 * (time.perf_counter() - t0)))
            return IngestOneResult.duplicate_idempotent(validated)
        else:
            return quarantine(envelope_path, f"Conflict: record {validated.experiment_id} exists with different canonical content")
    atomic_write_text(record_path, canonical_new)  # write the canonical form, not the raw inbox bytes

    # Step 3: Parquet files (already written by producer; verify paths exist but do not block)
    for parquet_ref in validated.bulk_parquet:
        if not Path(parquet_ref.path).exists():
            log.warn(f"Parquet missing: {parquet_ref.path}")
            # Allow ingest; bulk_parquet_refs row will point to missing file;
            # `hft-ops ledger check` surfaces the discrepancy later.

    # Step 4-6: SQLite transaction (WAL; apply PRAGMAs once per connection)
    conn = sqlite3.connect(ledger_dir / "ledger.sqlite")
    apply_pragmas(conn)
    try:
        with conn:  # auto-BEGIN / COMMIT on success / ROLLBACK on exception
            # §14.8 norm-integrity + §14.9 upstream-integrity run BEFORE INSERT so they
            # can raise UpstreamNotYetIngestedError and route the envelope to pending/
            # WITHOUT modifying SQLite state.
            check_normalization_integrity(validated, conn)   # §14.8
            check_upstream_integrity(validated, conn)        # §14.9
            insert_experiment(conn, validated)               # §7.3.1 INSERT template (runtime-generated from Pydantic fields)
            insert_metrics(conn, validated)
            insert_gates(conn, validated)
            insert_artifacts(conn, validated)
            insert_lineage(conn, validated)
            insert_bulk_parquet_refs(conn, validated)
            insert_tags(conn, validated)
            # Compute derived cohort_hash and update in same transaction (§6.4)
            cohort_hash = compute_cohort_hash(validated)
            conn.execute(
                "UPDATE experiments SET cohort_hash = ? WHERE experiment_fingerprint = ?",
                (cohort_hash, validated.experiment_fingerprint),
            )
    except UpstreamNotYetIngestedError as e:
        # SOFT failure: route to pending/ for later retry (§14.8 / D4)
        # Rollback the JSON record we just wrote — it will be re-written when pending retries succeed
        record_path.unlink(missing_ok=True)
        pending_path = ledger_dir / "pending" / envelope_path.name
        pending_path.parent.mkdir(exist_ok=True)
        os.replace(envelope_path, pending_path)
        _write_waiting_sidecar(pending_path, missing_fingerprints=_extract_missing_fps(e, validated))
        audit_log_append(ledger_dir, validated, result="pending_upstream",
                         duration_ms=int(1000 * (time.perf_counter() - t0)))
        return IngestOneResult.pending_upstream(validated, str(e))
    except NormalizationStatsMismatchError as e:
        # HARD failure: real train/inference drift — quarantine with forensic sidecar (§14.8)
        record_path.unlink(missing_ok=True)
        return quarantine(envelope_path, f"NormalizationStatsMismatch: {e}")
    except sqlite3.IntegrityError as e:
        # PK violation = experiment_fingerprint already present (already ingested)
        # Transaction auto-rolled back by context manager on exception
        # See §7.4.1 for rollback semantics (JSON record written but SQLite rolled back)
        if "UNIQUE constraint failed: experiments.experiment_fingerprint" in str(e):
            audit_log_append(ledger_dir, validated, result="duplicate_pk",
                             duration_ms=int(1000 * (time.perf_counter() - t0)))
            envelope_path.unlink()
            # Leave the JSON record in place (idempotent audit trail); it matches existing.
            return IngestOneResult.duplicate_pk(validated)
        # Other integrity errors (FK violation, CHECK violation): quarantine.
        record_path.unlink(missing_ok=True)  # roll back the JSON write
        return quarantine(envelope_path, f"IntegrityError: {e}")

    # Step 7: move envelope out of inbox (successful commit)
    envelope_path.unlink()
    audit_log_append(ledger_dir, validated, result="inserted",
                     duration_ms=int(1000 * (time.perf_counter() - t0)))

    # Post-ingest hook: debounced markdown render (§4.6 / §7.5)
    schedule_render_indexes(ledger_dir)

    return IngestOneResult.inserted(validated)
```

**`audit_log_append`** writes one JSONL line per ingest attempt to `hft-ops/ledger/ingest.log` (atomic via `O_APPEND` flag — single-writer safe; `flock` already held by ingest). See §5.3 footnote for record shape.

### §7.3.1 SQL INSERT Templates (Normative)

**Round 13 (B5)**: INSERT statements are **generated at runtime from the Pydantic model field list** — NOT hard-coded. The templates below are for DOCUMENTATION only. A schema change (e.g., adding `exchange` column in Phase 11.5 without touching the INSERT) silently fills the wrong column under hard-coded INSERTs because placeholders bind positionally. Runtime generation from the Pydantic field ordering eliminates this class of bug.

```python
# hft_ops/ledger/sql_templates.py

def build_insert_stmt(table: str, ordered_fields: list[str]) -> str:
    """Generate INSERT statement from a Pydantic model's field order.

    Invariant: placeholder count == len(ordered_fields) == target table column count.
    Verified by test_insert_statement_column_count_matches_schema at §18.2.
    """
    columns = ", ".join(ordered_fields)
    placeholders = ", ".join("?" for _ in ordered_fields)
    return f"INSERT INTO {table} ({columns}) VALUES ({placeholders});"

# At module load time, freeze the templates:
INSERT_EXPERIMENTS_SQL = build_insert_stmt(
    "experiments",
    list(Envelope.model_fields.keys())  # order from Pydantic = order from pipeline_contract.toml codegen
    + ["cohort_hash", "json_record_path", "ingested_at"]  # ingester-set fields appended in fixed order
)
INSERT_METRICS_SQL = build_insert_stmt("metrics", [...])
# ... etc. for each child table
```

**Meta-test** (`tests/test_schema_parity.py`):
```python
def test_insert_statement_column_count_matches_schema():
    """Guard against silent column drift: INSERT placeholders must match DDL columns."""
    conn = sqlite3.connect(":memory:")
    conn.executescript(SCHEMA_SQL)
    for table_name, insert_stmt in KNOWN_INSERTS.items():
        ddl_cols = {row[1] for row in conn.execute(f"PRAGMA table_info({table_name})")}
        # Skip auto-increment PKs + CHECK-generated cols
        stmt_cols = extract_columns_from_insert(insert_stmt)
        # Every stmt column must exist in DDL; placeholder count must match
        placeholder_count = insert_stmt.count("?")
        assert len(stmt_cols) == placeholder_count, (
            f"{table_name} INSERT has {len(stmt_cols)} cols but {placeholder_count} placeholders"
        )
        assert stmt_cols.issubset(ddl_cols), (
            f"{table_name} INSERT references columns not in DDL: {stmt_cols - ddl_cols}"
        )
```

Below are the **column orderings** that runtime generation must produce. Any deviation means the Pydantic model or DDL drifted. Values are bound via `?` placeholders (never string-concat — SQL injection guard AND correct NULL handling).

```sql
-- insert_experiment: ONE row per envelope
-- Round 14 C1: adds 4 nested-object JSON columns (training_info_json,
-- signal_provenance_json, strategy_info_json, export_stats_json). Serialized
-- from the Pydantic sub-models via canonical_json_blob (SSoT) at ingest time;
-- queryable via SQLite json_extract().
INSERT INTO experiments (
    experiment_fingerprint, experiment_id, fingerprint_version,
    symbol, symbols_json, asset_class, cohort_hash,       -- cohort_hash set by post-insert UPDATE
    producer, producer_version, record_type,
    envelope_schema_version, pipeline_contract_version,
    status, created_at, finalized_at, heartbeat_at, wall_clock_ms,
    data_source_type, feature_schema_ref, dataset,
    model_type, model_family, task,
    sampling_strategy, sampling_config,
    label_family, primary_horizon, n_horizons, horizons, label_spec,
    config_source, config_format,
    sweep_id, axis_values,
    parent_id, upstream_ids,
    git_commit, git_branch, git_dirty,
    hypothesis, description,
    training_info_json, signal_provenance_json,           -- §14.8 reads training_info_json
    strategy_info_json, export_stats_json,                -- §3.2 record-type-specific blobs
    metadata_json,
    json_record_path, ingested_at
) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?);
-- cohort_hash initially NULL; set by post-insert UPDATE (same txn)
-- Placeholder count: 48 (was 44 pre-v1.3.1 — add 4 for nested-object columns).
-- The runtime build_insert_stmt (§7.3.1 B5) auto-derives this from the
-- Pydantic field list; meta-test asserts placeholder count == DDL column count.

-- insert_metrics: ZERO-OR-MORE rows per envelope
INSERT INTO metrics (
    experiment_fingerprint, family, name, split, horizon, epoch,
    value, ci_low, ci_high, tag
) VALUES (?,?,?,?,?,?,?,?,?,?);

-- insert_gates: ZERO-OR-MORE rows per envelope; order matters
INSERT INTO gates (
    experiment_fingerprint, gate_name, gate_order, depends_on_gate,
    status, threshold, observed, note,
    override_by, override_reason, override_at
) VALUES (?,?,?,?,?,?,?,?,?,?,?);

-- insert_artifacts: ZERO-OR-MORE rows per envelope
INSERT INTO artifacts (
    experiment_fingerprint, kind, path, bytes, sha256, schema_version
) VALUES (?,?,?,?,?,?);

-- insert_lineage: ZERO-OR-MORE rows per envelope
INSERT INTO lineage (
    experiment_fingerprint, source_kind, source_name, source_hash,
    source_repo, source_commit, source_commit_dirty
) VALUES (?,?,?,?,?,?,?);

-- insert_bulk_parquet_refs: ZERO-OR-MORE rows per envelope
INSERT INTO bulk_parquet_refs (
    experiment_fingerprint, kind, path,
    schema_json, partition_keys, row_count, sha256, bytes
) VALUES (?,?,?,?,?,?,?,?);

-- insert_tags: ZERO-OR-MORE rows per envelope
INSERT OR IGNORE INTO tags (experiment_fingerprint, tag) VALUES (?,?);
-- OR IGNORE because tags is a set (dedup on PK (fp, tag))

-- update_cohort_hash: ONE row per envelope (runs AFTER insert_experiment)
UPDATE experiments SET cohort_hash = ? WHERE experiment_fingerprint = ?;

-- upsert_sweep (only if envelope carries sweep_id that does not yet exist)
INSERT INTO sweeps (
    sweep_id, name, strategy, axes_json, created_at,
    status, n_expected
) VALUES (?,?,?,?,?,?,?)
ON CONFLICT(sweep_id) DO NOTHING;
-- status updates to sweeps happen through a separate `hft-ops sweep finalize` path

-- insert_fingerprint_history: ONE row per envelope (current fingerprint_version)
INSERT OR REPLACE INTO fingerprint_history
    (experiment_id, fingerprint_version, fingerprint_value, computed_at)
VALUES (?, ?, ?, ?);
```

**Invariants (tested):**
1. Column order in the INSERT statement matches the field order in `ValidatedEnvelope` — this is a Contract: the Pydantic model's `.as_sql_row()` method returns a tuple aligned to these INSERTs.
2. `sweep_id` FK is nullable in `experiments`; the `upsert_sweep` MUST run before `insert_experiment` if `sweep_id` is non-null.
3. `notes` uses a separate API path (`ledger.add_note()` CLI) — envelopes do NOT carry `notes` on first ingest. The envelope's `.notes` top-level string field maps to the `description` column, NOT the `notes` table.

### §7.3.2 Helper Functions (Round 14 C5 — 9 primitives referenced by §7.3/§7.4/§14.8)

The `ingest_one()` pseudocode at §7.3 references several helper primitives that are defined here. Each is a small utility (<20 LOC). Implementations live in `hft-ops/src/hft_ops/ledger/helpers.py` (Phase 11 W2 deliverable).

**Trivial primitives referenced by the 9 helpers below** (Round 15 addition — filling micro-gaps in the R14 C5 spec):

```python
# Module-level constant: mirrors schema_info.schema_version; embedded in .error sidecars
# for forensic version-stamping. Loaded once at module import via:
#   SCHEMA_INFO_VERSION = _load_schema_info_version()
# which reads pipeline_contract.toml → [contract].schema_version.
SCHEMA_INFO_VERSION: str = "1.0.0"  # kept in sync with pipeline_contract.toml by codegen

def _utc_now_iso_z() -> str:
    """Current wall-clock time as ISO 8601 UTC with Z suffix (matches B1 regex).

    Used by all sidecar writers (audit log, quarantine .error, pending .waiting).
    Pydantic serializer _to_iso_z() at §15.1a Day 3 uses the same output format.
    """
    from datetime import datetime, timezone
    dt = datetime.now(timezone.utc)
    s = dt.strftime('%Y-%m-%dT%H:%M:%S')
    if dt.microsecond:
        s += f'.{dt.microsecond:06d}'.rstrip('0').rstrip('.') or ''
    return s + 'Z'
```

**`IngestOneResult` class** (referenced by every `ingest_one` path; §7.3 calls 7 factory methods):

```python
from dataclasses import dataclass
from typing import Optional, ClassVar
from pathlib import Path

@dataclass(frozen=True)
class IngestOneResult:
    """Outcome of a single envelope ingest attempt.

    Immutable record of what happened; consumed by IngestReport for summary.
    Constructor methods cover all terminal states in §7.3's ingest_one() flow.
    """
    outcome: str                # 'would_insert' | 'duplicate_idempotent' | 'duplicate_pk' |
                                #  'inserted' | 'pending_upstream' | 'quarantined' | 'age_out_quarantined'
    envelope_path: Optional[Path] = None
    experiment_fingerprint: Optional[str] = None
    experiment_id: Optional[str] = None
    reason: Optional[str] = None          # for quarantined / pending / age-out
    missing_fingerprints: Optional[tuple] = None  # for pending_upstream

    # Factory methods (used throughout §7.3 / §7.4)
    @classmethod
    def would_insert(cls, v: "Envelope") -> "IngestOneResult":
        return cls(outcome='would_insert', experiment_fingerprint=v.experiment_fingerprint, experiment_id=v.experiment_id)
    @classmethod
    def duplicate_idempotent(cls, v: "Envelope") -> "IngestOneResult":
        return cls(outcome='duplicate_idempotent', experiment_fingerprint=v.experiment_fingerprint, experiment_id=v.experiment_id)
    @classmethod
    def duplicate_pk(cls, v: "Envelope") -> "IngestOneResult":
        return cls(outcome='duplicate_pk', experiment_fingerprint=v.experiment_fingerprint, experiment_id=v.experiment_id)
    @classmethod
    def inserted(cls, v: "Envelope") -> "IngestOneResult":
        return cls(outcome='inserted', experiment_fingerprint=v.experiment_fingerprint, experiment_id=v.experiment_id)
    @classmethod
    def pending_upstream(cls, v: "Envelope", reason: str) -> "IngestOneResult":
        return cls(outcome='pending_upstream', experiment_fingerprint=v.experiment_fingerprint,
                   experiment_id=v.experiment_id, reason=reason)
    @classmethod
    def quarantined(cls, envelope_path: Path, reason: str) -> "IngestOneResult":
        return cls(outcome='quarantined', envelope_path=envelope_path, reason=reason)
    @classmethod
    def age_out_quarantined(cls, envelope_path: Path) -> "IngestOneResult":
        return cls(outcome='age_out_quarantined', envelope_path=envelope_path,
                   reason='upstream never arrived; aged out after 24h')
```

**The 9 core helpers (unchanged from v1.3.1 — listed here for reference):**

**`atomic_write_text(path: Path, content: str) -> None`** — atomic file write via tmp+fsync+replace. Used for `records/{id}.json` writes (canonical form). The same primitive supports `.error`, `.waiting`, and `.migration_state` sidecar files.

```python
def atomic_write_text(path: Path, content: str) -> None:
    tmp = path.with_suffix(path.suffix + ".tmp")
    with open(tmp, "w", encoding="utf-8") as f:
        f.write(content)
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp, path)  # atomic POSIX rename
```

**`apply_pragmas(conn: sqlite3.Connection) -> None`** — applied once per connection. Reads §5.2 PRAGMAs verbatim.

```python
def apply_pragmas(conn: sqlite3.Connection) -> None:
    conn.execute("PRAGMA journal_mode=WAL")
    conn.execute("PRAGMA synchronous=NORMAL")
    conn.execute("PRAGMA foreign_keys=ON")
    conn.execute("PRAGMA busy_timeout=5000")  # 5s wait before SQLITE_BUSY
    conn.execute("PRAGMA temp_store=MEMORY")
    conn.execute("PRAGMA cache_size=-64000")  # 64 MiB page cache
```

**`audit_log_append(ledger_dir: Path, envelope: Envelope, result: str, duration_ms: int, note: str | None = None) -> None`** — appends ONE JSONL line to `hft-ops/ledger/ingest.log` using `O_APPEND` (atomic single-writer; `flock(ledger.lock)` is held by the caller).

```python
from hft_contracts.canonical_hash import canonical_json_blob, sha256_hex

def audit_log_append(ledger_dir: Path, envelope: Envelope, *, result: str,
                     duration_ms: int, note: str | None = None) -> None:
    envelope_blob = canonical_json_blob(envelope.model_dump(mode='json'), sanitize=True)
    line_dict = {
        "ingested_at": _utc_now_iso_z(),
        "envelope_content_hash": sha256_hex(envelope_blob),
        "experiment_id": envelope.experiment_id,
        "experiment_fingerprint": envelope.experiment_fingerprint,
        "envelope_version": envelope.envelope_version,
        "ingested_by": os.environ.get("USER", "unknown"),
        "duration_ms": duration_ms,
        "result": result,  # inserted | duplicate_idempotent | duplicate_pk | pending_upstream | rejected | error
        "note": note,
    }
    line = canonical_json_blob(line_dict, sanitize=True).decode('utf-8')
    log_path = ledger_dir / "ingest.log"
    with open(log_path, "a", encoding="utf-8") as f:
        f.write(line + "\n")
        f.flush()
        os.fsync(f.fileno())
```

**`quarantine(envelope_path: Path, reason: str) -> IngestOneResult`** — atomic move to `quarantine/` + `.error` sidecar. Returns an IngestOneResult for the caller to surface.

```python
def quarantine(envelope_path: Path, reason: str) -> IngestOneResult:
    ledger_dir = envelope_path.parent.parent  # inbox/ is directly under ledger_dir
    q_dir = ledger_dir / "quarantine"
    q_dir.mkdir(exist_ok=True)
    q_path = q_dir / envelope_path.name
    err_path = q_path.with_suffix(".error")
    # Write .error sidecar FIRST (so an interrupted move doesn't leave a quarantined envelope without context)
    atomic_write_text(err_path, canonical_json_blob({
        "reason": reason, "quarantined_at": _utc_now_iso_z(),
        "original_path": str(envelope_path), "handler_version": SCHEMA_INFO_VERSION,
    }, sanitize=True).decode('utf-8'))
    os.replace(envelope_path, q_path)
    audit_log_append(envelope_path.parent.parent, None, result="rejected", duration_ms=0, note=reason)
    return IngestOneResult.quarantined(envelope_path, reason)
```

**`_escalate_pending_to_quarantine(pending_path: Path, ledger_dir: Path, reason: str) -> None`** — for `pending/` envelopes that aged out past 24h. Moves pending/file.json → quarantine/file.json with an `.error` sidecar containing both the original pending reason and the age-out reason.

```python
def _escalate_pending_to_quarantine(pending_path: Path, ledger_dir: Path, reason: str) -> None:
    waiting_sidecar = pending_path.with_suffix(".waiting")
    waiting_data = json.loads(waiting_sidecar.read_text()) if waiting_sidecar.exists() else {}
    q_path = ledger_dir / "quarantine" / pending_path.name
    err_path = q_path.with_suffix(".error")
    atomic_write_text(err_path, canonical_json_blob({
        "reason": reason, "quarantined_at": _utc_now_iso_z(),
        "previously_pending_for": waiting_data.get("waiting_since"),
        "missing_fingerprints": waiting_data.get("missing_fingerprints", []),
    }, sanitize=True).decode('utf-8'))
    os.replace(pending_path, q_path)
    if waiting_sidecar.exists():
        waiting_sidecar.unlink()
```

**`_write_waiting_sidecar(pending_path: Path, missing_fingerprints: list[str]) -> None`** — writes a `.waiting` sidecar alongside a pending/ envelope. Format is documented contract (operators may `cat` these files to debug).

```python
def _write_waiting_sidecar(pending_path: Path, missing_fingerprints: list[str]) -> None:
    sidecar = pending_path.with_suffix(".waiting")
    atomic_write_text(sidecar, canonical_json_blob({
        "waiting_since": _utc_now_iso_z(),
        "missing_fingerprints": sorted(missing_fingerprints),
        "age_out_policy": "escalate_to_quarantine_after_24h",
    }, sanitize=True).decode('utf-8'))
```

**`_extract_missing_fps(exc: UpstreamNotYetIngestedError, validated: Envelope) -> list[str]`** — extracts the list of missing fingerprints for the sidecar. Uses attribute access (preferred) + exception message fallback.

```python
def _extract_missing_fps(exc: UpstreamNotYetIngestedError, validated: Envelope) -> list[str]:
    # Preferred: structured attribute set by raiser
    if hasattr(exc, "missing_fingerprints"):
        return exc.missing_fingerprints
    # Fallback: derive from validated envelope's declared upstream
    missing = []
    if validated.signal_provenance and validated.signal_provenance.trainer_fingerprint:
        missing.append(validated.signal_provenance.trainer_fingerprint)
    for item in validated.lineage or []:
        if item.source_kind in ("export", "trainer_run", "backtest_run") and item.source_hash:
            missing.append(item.source_hash)
    return missing
```

To support the preferred path, `UpstreamNotYetIngestedError.__init__` accepts a `missing_fingerprints` kwarg and stores it as an attribute.

**`check_upstream_integrity(validated: Envelope, conn: sqlite3.Connection) -> None`** — §14.9 enforcement. Raises `UpstreamNotYetIngestedError` if any `lineage[].source_kind in {"export", "trainer_run", ...}` references a `source_hash` not in `experiments.experiment_fingerprint` OR `fingerprint_history.fingerprint_value`.

```python
def check_upstream_integrity(validated: Envelope, conn: sqlite3.Connection) -> None:
    if validated.producer == "legacy-migrator":
        return  # §13.1b exemption
    LEDGER_KINDS = {"export", "trainer_run", "backtest_run", "evaluator_run"}
    missing = []
    for item in validated.lineage or []:
        if item.source_kind not in LEDGER_KINDS:
            continue
        if not item.source_hash:
            continue
        # Check primary + historical fingerprints
        row = conn.execute(
            """SELECT 1 FROM experiments WHERE experiment_fingerprint = ?
               UNION SELECT 1 FROM fingerprint_history WHERE fingerprint_value = ? LIMIT 1""",
            (item.source_hash, item.source_hash)
        ).fetchone()
        if row is None:
            missing.append(item.source_hash)
    if missing:
        raise UpstreamNotYetIngestedError(
            f"{len(missing)} upstream lineage fingerprints not in ledger: {missing[:3]}...",
            missing_fingerprints=missing,
        )
```

Phase 11 deploys this as WARN-only initially (catch, log, proceed); Phase 13 promotes to hard-fail per §14.9.

**`compute_cohort_hash(envelope: Envelope) -> str`** — §6.4 formula.

```python
def compute_cohort_hash(envelope: Envelope) -> str:
    components = envelope.model_dump(mode='json', exclude_none=False)
    # Drop fields that cross-symbol cohorts should share
    for field in ("symbol", "symbols_json", "experiment_fingerprint", "experiment_id",
                  "created_at", "finalized_at", "heartbeat_at", "ingested_at",
                  "json_record_path", "git"):
        components.pop(field, None)
    # Drop data_manifest sub-field inside extraction_config if present
    if "sampling_config" in components and isinstance(components["sampling_config"], dict):
        components["sampling_config"].pop("data_manifest", None)
    return sha256_hex(canonical_json_blob(components, sanitize=True))
```

All 9 helpers are pure-Python, stateless except as they touch the filesystem. They have unit tests in `hft-ops/tests/test_ingest_helpers.py` (~10-12 tests covering the happy path + 1-2 edge cases each).

### §7.4 Quarantine Protocol

Malformed envelopes go to `quarantine/`:

```
hft-ops/ledger/quarantine/
├── {content_hash}.json          # the rejected envelope
└── {content_hash}.error         # JSON with {reason, traceback, timestamp, handler_version}
```

CLI operations:

```bash
hft-ops ledger quarantine list                      # show quarantined envelopes with reasons
hft-ops ledger quarantine show <content_hash>       # inspect one
hft-ops ledger quarantine retry <content_hash>      # move back to inbox for re-ingest
hft-ops ledger quarantine drop <content_hash>       # permanent delete
hft-ops ledger quarantine drain                     # retry all (diagnostic)
```

### §7.4.1 Rollback Protocol (Partial-Commit Recovery)

Ingest is a 3-step atomic write with an intentional ordering (§5.5): JSON record → Parquet presence check → SQLite transaction. Failure between steps leaves the ledger in one of four states. The recovery matrix is normative:

| State | What is on disk | What is in SQLite | Recovery action |
|---|---|---|---|
| **Clean** | envelope in inbox (not consumed); no record, no Parquet | no row | Nothing to do. Re-running ingest picks it up. |
| **A — JSON written, SQLite rolled back** | `records/{id}.json` exists; inbox entry still there | no row | `ingest_one` auto-runs `record_path.unlink(missing_ok=True)` in its `except IntegrityError` branch (see §7.3). If the process crashes before that `unlink`, the next ingest detects `record_path.exists()` and content-matches — if match, treats as `duplicate_idempotent` (safe); if mismatch, quarantines. |
| **B — SQLite committed, render crashed** | JSON + SQLite row present; inbox entry removed | row present | Ingest is already COMPLETE. Render failure is cosmetic. Next ingest or `hft-ops ledger render-indexes --force` heals markdown. |
| **C — JSON written, SQLite committed, process killed before `envelope_path.unlink()`** | JSON + SQLite row + inbox entry all present | row present | Next ingest re-reads the inbox entry, `record_path.exists()` is true, content matches (byte-equal via §3.3.6 canonical JSON) → `duplicate_idempotent` path; inbox entry removed; idempotent recovery. |
| **D — Partial child-table insert (shouldn't happen)** | JSON + inbox entry | partial row in `experiments`, missing rows in `metrics`/`gates` | CANNOT occur: the entire child-table insert is inside `with conn:` which auto-rolls-back on exception. If observed in production, indicates a C-level SQLite bug or manual DB tampering; use `hft-ops ledger rebuild --from-records` to regenerate SQLite from `records/*.json`. |

**CLI recovery helpers (Phase 11):**

```bash
# Scan for state-A leftovers (JSON records with no SQLite row)
hft-ops ledger check --orphaned-records     # lists orphans
hft-ops ledger heal --from-orphans          # re-ingest or quarantine orphans

# Nuclear: rebuild entire SQLite from append-only JSON records
hft-ops ledger rebuild --from-records --backup
# writes ledger.sqlite.backup-{ts} first; then re-applies all records/*.json

# Audit parity: record count vs sqlite row count
hft-ops ledger check --parity
# EXPECTS: count(records/*.json) == count(SELECT * FROM experiments)
```

**Invariant guaranteed by this protocol:** the append-only `records/{experiment_id}.json` files are the SOURCE OF TRUTH. SQLite is a rebuildable cache. Any divergence (state A, state D) is resolvable by `rebuild --from-records`. Test §18.3 fault-injection validates each state is reachable and each recovery succeeds.

### §7.5 Post-Ingest Hook (debounced render — §4.6)

After each successful SQLite commit, the ingester calls `schedule_render_indexes(ledger_dir)` (see §4.6) which debounces bursts (first call after idle renders immediately; subsequent calls within 5 s coalesce; forced render at 60 s absolute ceiling). The actual render work:

1. Opens SQLite in **read-only** mode (`PRAGMA query_only=ON`).
2. Reads current state (all `experiments` rows + latest metrics/gates summary).
3. Generates markdown for `EXPERIMENT_INDEX.md`, `BACKTEST_INDEX.md`, `EXPORT_INDEX.md`.
4. Atomic write each file (tmp + `os.replace`).
5. Prepends `<!-- GENERATED @ {ISO8601} FROM ledger.sqlite (schema 1.0.0); DO NOT EDIT -->` header.

If the hook fails, it logs a warning; does NOT affect ingest success status. Users can manually run `hft-ops ledger render-indexes --force` to refresh synchronously.

**Rationale (Agent D R3):** eager per-ingest render is O(N²) across a sweep (render reads all N rows for every one of N ingests). Debounce makes it O(N). Users never notice the debounce in interactive use (renders happen within 5 s) and sweep drivers explicitly `--force` at end-of-sweep.

### §7.6 Pre-Ingest Validation Chain

Every envelope passes through:

1. **JSON parse** — syntactically valid JSON?
2. **Schema validation** — conforms to envelope-v1 JSON Schema?
3. **Enum validation** — all enum fields have valid values?
4. **Cross-field validation** — enumerated in §7.6.1 below (hard-fail set) and §7.6.2 (warn set).
5. **Fingerprint plausibility** — 64-char lowercase hex?
6. **Artifact/Parquet path existence** — warn (not fail) if referenced file is missing (allows producer-writes-envelope-first race to resolve; ingest may be delayed).
7. **Foreign key plausibility** — if `upstream_experiment_ids` non-empty, referenced IDs SHOULD exist in experiments table (warn if not — may be out-of-order ingest).

Failures at 1-3 and the hard-fail subset of 4 (§7.6.1) → quarantine + `.error` sidecar. Failures at 5 → quarantine. Failures at 6-7 and the warn subset of 4 (§7.6.2) → log warning and proceed.

#### §7.6.1 Hard-fail cross-field validations

Each rule is a single-line predicate on the validated envelope object (`e`). If false → quarantine.

1. `e.n_horizons == len(e.horizons)` — label-dimension consistency.
2. `e.record_type == "export"` ⇒ `e.export_stats is not None`.
3. `e.record_type == "training"` ⇒ `e.model_type is not None` AND `e.training_info is not None`.
4. `e.record_type == "backtest"` ⇒ `e.strategy_info is not None` AND `len(e.upstream_experiment_ids) >= 1`.
5. `e.record_type == "evaluation"` ⇒ `e.feature_eval_info is not None`.
6. `e.record_type == "sweep_aggregate"` ⇒ `e.sweep_id is not None`.
7. `e.export_stats.n_features <= feature_schema_registry[e.feature_schema_ref].total_count` — subset check; producer cannot claim more features than the registered schema declares.
8. If `e.export_stats.feature_indices_subset is not None`: `len(e.export_stats.feature_indices_subset) == e.export_stats.n_features` AND `all(0 <= i < total_count for i in feature_indices_subset)`.
9. `e.symbols_json is None` XOR `e.symbol in json.loads(e.symbols_json)` — primary symbol consistency for multi-symbol runs.
10. `e.training_info.normalization_stats_sha256 is not None` ⇔ `e.training_info.normalization_method in ("hybrid","global_zscore","per_feature_minmax")` — stats-hash required when a non-trivial normalization is declared (§BQ3 / §T2).
11. `e.sub_records == []` unless `e.record_type == "sweep_aggregate"` — non-aggregate envelopes cannot carry sub-records.
12. **Round 17 I4**: for `e.record_type == "export"`, if `e.artifacts[]` contains an entry with `kind == "metadata"` pointing to a readable `{day}_metadata.json`, the ingester invokes `hft_contracts.validation.validate_any_export_contract(json.load(artifact.path))`. On `ContractError` from the existing SSoT validator: **WARN-only in v1.0.0** (some legacy records lack metadata artifacts and still must ingest); promoted to **HARD-FAIL in v2.0.0** envelope schema bump. Gains free reuse of the existing 165+ validation tests covering SCHEMA_VERSION / feature count / normalization flag / provenance fields — no re-implementation in the ingest path.

#### §7.6.2 Warn-only cross-field validations

Violation logs a structured warning but ingest proceeds:

1. `metric.name not in MetricKey registry` — unregistered metric key (§4.3 permits; warn to encourage registration).
2. `gate.name not in GateKey registry` — same.
3. `bulk_parquet[].path` does not exist on disk — see §7.3 / §7.4.1 state A/B/C.
4. `upstream_experiment_ids` contains IDs not yet present in `experiments` table — out-of-order ingest; retry-later.
5. Range violation (metric `value` outside declared `range` / `lower_bound` / `upper_bound`) — value is preserved; downstream consumers may flag.
6. `e.git.commit_hash == ""` (empty) on a non-retroactive envelope — git not on PATH or producer not launched from a repo; ingest proceeds because `not_git_tracked` sentinel is the documented escape hatch (§3.2 `source_commit` pattern).

---

## §8 — Query API

### §8.1 Philosophy

Prefer SQL pass-through + pandas DataFrame output over bespoke query DSLs. Reasons:
- SQL is universal; researchers already know it.
- pandas is the lingua franca of Python data analysis.
- Zero new vocabulary.
- Rich ad-hoc exploration.

### §8.2 CLI Commands

```bash
# Lists
hft-ops ledger list                                              # latest N experiments
hft-ops ledger list --limit 100 --order-by created_at
hft-ops ledger list --producer lob-model-trainer --status completed
hft-ops ledger list --sweep-id sweep_nvda_bins_20260415

# Show one
hft-ops ledger show <experiment_id>                              # full details
hft-ops ledger show <fingerprint_prefix>                         # by fingerprint prefix (unambiguous)

# Query with SQL filter
hft-ops ledger query "SELECT * FROM experiments WHERE model_type='tlob' AND finalized_at > '2026-04-01'"
hft-ops ledger query "..." --format csv                          # pipe-friendly
hft-ops ledger query "..." --format markdown                     # table-pretty
hft-ops ledger query "..." --format json                         # for scripts

# Metric threshold queries
hft-ops ledger search --min-metric ic:0.1 --horizon 10 --split test
hft-ops ledger search --gate baseline_ridge:passed
hft-ops ledger search --tags phase-9,off-exchange

# Compare two or more
hft-ops ledger compare <id1> <id2> [<id3>...]                    # dvc-exp-show style table
hft-ops ledger compare <id1> <id2> --metrics IC_H10,ACCURACY

# Lineage traversal
hft-ops ledger lineage <experiment_id>                           # walk upstream
hft-ops ledger lineage <experiment_id> --downstream               # walk downstream
hft-ops ledger lineage <experiment_id> --format graphviz          # DOT output

# Regression alerts
hft-ops ledger regression-check <experiment_id>                  # vs. cohort rolling median

# Cohort
hft-ops ledger cohort --cohort-hash <hash>                       # all symbols for a logical experiment
hft-ops ledger cohort --model-family neural --label-family triple_barrier

# Notes
hft-ops ledger note add <experiment_id> --body "post-hoc observation"
hft-ops ledger note list <experiment_id>
hft-ops ledger note edit <note_id>                               # append mode only

# Tags
hft-ops ledger tag add <experiment_id> tag1 tag2
hft-ops ledger tag remove <experiment_id> tag1
hft-ops ledger tag list <tag>                                    # experiments with tag
```

### §8.3 Python SDK

```python
from hft_ops.ledger import Ledger

lg = Ledger()  # opens $REPO/hft-ops/ledger/ledger.sqlite

# Pandas DataFrame output
df = lg.query("SELECT * FROM experiments WHERE model_type='tlob'")

# Filter-string helpers
df = lg.search(
    model_type="tlob",
    min_metric=("IC_H10", 0.1),
    split="test",
    tags=["phase-9"],
    limit=50,
)

# Lineage traversal
lineage = lg.lineage("TLOB_Triple_Barrier_v1_20260314T031729_a3b8f1c4")
# returns nested dict: {source: {kind, name, repo, commit, children: [...]}}

# Metric arrays from Parquet
curve = lg.training_curve("TLOB_Triple_Barrier_v1_20260314T031729_a3b8f1c4")
# returns pd.DataFrame of per-epoch metrics

# Note management
lg.add_note("TLOB_...", body_md="P0 validation shows IC stable across CV folds.", author="claude-opus")
```

### §8.4 Common Query Patterns (documented examples)

```sql
-- Q1: "Show all TLOB experiments with test IC > 0.1 at H=10"
SELECT e.experiment_id, e.finalized_at, m.value AS test_ic
FROM experiments e
JOIN metrics m USING (experiment_fingerprint)
WHERE e.model_type = 'tlob'
  AND m.name = 'IC_H10' AND m.split = 'test'
  AND m.value > 0.1
ORDER BY m.value DESC;

-- Q2: "Cohort: same logical experiment, all symbols"
SELECT e.symbol, e.experiment_id, m.value AS test_acc
FROM experiments e
JOIN metrics m USING (experiment_fingerprint)
WHERE e.cohort_hash = 'abc123...'
  AND m.name = 'ACCURACY' AND m.split = 'test'
ORDER BY e.symbol;

-- Q3: "Recent gate failures"
SELECT e.experiment_id, e.producer, g.gate_name, g.observed
FROM experiments e
JOIN gates g USING (experiment_fingerprint)
WHERE g.status = 'failed'
  AND e.finalized_at > datetime('now', '-7 days')
ORDER BY e.finalized_at DESC;

-- Q4: "All experiments that consumed a specific BQP export"
SELECT e.experiment_id, e.producer, e.finalized_at
FROM experiments e
JOIN lineage l USING (experiment_fingerprint)
WHERE l.source_repo = 'basic-quote-processor'
  AND l.source_hash = '<bqp_fingerprint>'
ORDER BY e.finalized_at DESC;

-- Q5: "Multi-symbol membership (either pooled or sweep)"
SELECT experiment_id, symbol, symbols_json
FROM experiments
WHERE symbol = 'HOOD' OR symbols_json LIKE '%"HOOD"%';
```

### §8.5 Read-Only Guarantees

- CLI `hft-ops ledger query` opens SQLite in read-only mode (`PRAGMA query_only=ON`).
- Concurrent queries during ingest/rebuild are safe (WAL).
- Stale reads are OK within the window between two writes; `--fresh` flag forces `PRAGMA wal_checkpoint(TRUNCATE)` before read.

---

## §9 — Sweep v2

### §9.1 Goals

Replace current grid-only sequential sweep (`hft-ops/src/hft_ops/manifest/sweep.py`) with:

1. **Parallel subprocess pool** — configurable concurrency.
2. **Early-kill via gate propagation** — if gate fails for cohort, skip remaining axes.
3. **Resume via `--skip-existing`** — read existing ledger; skip fingerprints already present.
4. **Richer axis grammar** — conditional axes, Latin Hypercube (optional, Phase 12.x).
5. **Optuna integration** — for HPO-style searches (optional, Phase 12.x).

### §9.2 Preserved from Current

- `SweepConfig` dataclass shape (`hft-ops/src/hft_ops/manifest/schema.py:309-339`).
- Grid expansion (Cartesian product).
- Axis validation (8 rules at `sweep.py:44-121`).
- Cross-axis key conflict detection.
- `sweep_id = f"{name}_{timestamp}"` identifier.
- 1:N sweep → experiments mapping via `experiments.sweep_id` FK.

### §9.3 New: Parallel Execution

```yaml
# Example sweep manifest
sweep:
  name: tlob_bin_size_sweep
  strategy: grid
  parallelism: 4                        # NEW: subprocess pool size
  early_kill:                           # NEW: gate-based abort
    gate: ic_gt_0_05
    after_n_failures: 5                  # abort this axis after N consecutive failures
  axes:
    - name: bin_size
      values:
        - { label: "30s", overrides: { extraction.bin_size_seconds: 30 } }
        - { label: "60s", overrides: { extraction.bin_size_seconds: 60 } }
        - { label: "120s", overrides: { extraction.bin_size_seconds: 120 } }
    - name: horizon
      values:
        - { label: "H10", overrides: { training.horizon: 10 } }
        - { label: "H60", overrides: { training.horizon: 60 } }
```

Execution:

```
hft-ops sweep start <manifest.yaml> [--skip-existing] [--parallelism N]
```

1. Expand grid: 3 × 2 = 6 manifests.
2. Compute fingerprints for each.
3. If `--skip-existing`: filter out fingerprints already in experiments table.
4. Spawn subprocess pool (ProcessPoolExecutor, size=parallelism).
5. Each worker runs `hft-ops run` for one manifest.
6. Monitor: as envelopes land in inbox, ingest them.
7. Early-kill: if `after_n_failures` gate failures accumulate, abort pending axes.

### §9.4 Axis Grammar Extensions (v2, optional — Phase 12.x)

```yaml
# Conditional axes
- name: optimizer
  when: "training.model.family == 'neural'"
  values: [{label: "adam", ...}, {label: "sgd", ...}]

# Latin Hypercube
- name: lr
  strategy: lhs
  range: [1e-5, 1e-2]
  log_scale: true
  n_samples: 16

# Optuna integration (Phase 12.x)
- name: hp_study
  strategy: optuna
  objective: maximize_metric
  metric: IC_H10
  n_trials: 50
  sampler: TPESampler
```

### §9.5 Sweep Status Tracking

`sweeps` table records aggregate state:

| Column | Meaning |
|---|---|
| `n_expected` | total grid points pre-filter |
| `n_succeeded` | completed runs |
| `n_failed` | errored runs |
| `n_skipped` | filtered out (dedup or gate-abort) |
| `status` | pending → running → (completed | aborted | failed) |

CLI:

```bash
hft-ops sweep status <sweep_id>                     # live status
hft-ops sweep results <sweep_id>                    # table of completed runs
hft-ops sweep results <sweep_id> --sort-by IC_H10   # sorted
hft-ops sweep abort <sweep_id>                      # user-initiated early-kill
```

### §9.6 Sweep Output Conventions

Per-axis output dirs:

```
outputs/<sweep_id>/
├── sweep_manifest.yaml       # the expanded manifest
├── sweep_report.json         # aggregate status + per-axis results
├── <axis_combination_1>/     # per-grid-point dir
│   └── ... (trainer outputs, etc.)
└── <axis_combination_N>/
```

`sweep_id` is also the `sweep_id` column in the ledger.

### §9.7 Explicit Non-Goals for Sweep v2 v1

- **No GPU scheduling** — parallelism is CPU-subprocess.
- **No checkpoint-and-resume mid-axis** — abort + restart only.
- **No distributed sweep across machines** — single machine only.
- **No real-time dashboard** — CLI `hft-ops sweep status` is the interface.

---

## §10 — Knowledge Synthesis

### §10.1 Auto-Regenerated Markdown

Three markdown files become DERIVED views over the ledger:

- `lob-model-trainer/EXPERIMENT_INDEX.md` — all `record_type in ('training', 'evaluation', 'analysis')`.
- `lob-backtester/BACKTEST_INDEX.md` — all `record_type = 'backtest'`.
- `feature-extractor-MBO-LOB/EXPORT_INDEX.md` — all `record_type = 'export'` from MBO.
- (NEW) `basic-quote-processor/EXPORT_INDEX.md` — all `record_type = 'export'` from BQP.

Header on each generated file:

```markdown
<!-- 
  GENERATED @ 2026-04-15T12:34:56Z
  SOURCE: hft-ops/ledger/ledger.sqlite (rows: 412, schema: 1.0.0)
  RENDERER: hft-ops ledger render-indexes
  DO NOT EDIT — changes overwritten on next render.
  To add notes: hft-ops ledger note add <experiment_id> --body "..."
-->
```

### §10.2 Rendering Layout

Each experiment entry:

```markdown
### {experiment_id}

| Field | Value |
|---|---|
| Fingerprint | `a3b8f1c4...` (prefix 8) |
| Producer | lob-model-trainer v0.1.0 |
| Created | 2026-03-14T03:17:29Z |
| Finalized | 2026-03-14T04:17:29Z (60 min) |
| Symbol | NVDA |
| Task | classification (triple_barrier, H=10) |
| Model | tlob (neural, 370k params) |
| Dataset | nvda_triple_barrier |
| Status | ✅ completed |

**Hypothesis**: {hypothesis}

**Metrics** (test split):
| Name | Value | Horizon |
|---|---|---|
| ACCURACY | 0.596 | 10 |
| MACRO_F1 | 0.431 | 10 |

**Gates**:
- ✅ ic_gt_0_05 (observed 0.380, threshold 0.05)
- ✅ baseline_ridge (observed 0.677, threshold 0.616)

**Lineage**:
- export ← `nvda_triple_barrier` @ feature-extractor-MBO-LOB commit abc1234

**Notes**:
- [2026-04-02] Re-analysis: IC stable across 5 CV folds — _claude-opus_

---
```

### §10.3 Pagination for Scale

At 200+ experiments per file, split into sub-files by month:

```
EXPERIMENT_INDEX.md                       # master index (TOC + links)
EXPERIMENT_INDEX/2026_04.md               # monthly detail
EXPERIMENT_INDEX/2026_03.md
EXPERIMENT_INDEX/2026_02.md
...
```

At 1000+ experiments, additional slicing by:
- asset_class
- model_family
- record_type

### §10.4 Cohort Reports (Phase 13)

Deferred to Phase 13 — not v1 scope. Sketches:

- **Model comparison report**: for each model_family, aggregate (mean, std, best, worst) metrics across all experiments. Identifies outliers.
- **Feature-set IC trend report**: for each (feature_set, label_family), plot IC over time. Detects regime drift.
- **Cost-adjusted backtest leaderboard**: for each (strategy, holding_policy), rank by option_total_return AFTER cost adjustment.

### §10.5 Failure Taxonomy (Phase 13)

Tag every record with a `failure_category` from a controlled vocabulary:

- `signal_quality_insufficient` — IC < threshold
- `cost_dominates` — model return < cost breakeven
- `label_exec_misalignment` — smoothed-label vs. point-label discrepancy
- `overfitting` — train-val gap > threshold
- `baseline_tied` — non-linear no better than Ridge
- `data_leak` — lookahead detected
- `regime_shift` — temporal IC drops across splits
- `not_tradeable` — high IC but backtest negative

At 1000 experiments, the distribution of failure categories becomes a research instrument: "we've tried model architectures 20 times; signal quality was insufficient 15 of those. Time to stop iterating on models and focus on features."

---

## §11 — Multi-Repo Provenance Chain

### §11.1 The Chain

For a typical research experiment:

```
Raw data (Databento)
  → basic-quote-processor (or feature-extractor-MBO-LOB)
    → export artifact (NPY + metadata.json + manifest)
      → lob-model-trainer
        → checkpoint + training_history.json
          → lob-backtester
            → backtest_result.json + equity_curve.npy
```

Each step records its OWN envelope with full git state. Downstream steps record UPSTREAM hash references.

### §11.2 Cross-Repo Source Identification

Every `lineage` row contains:

| Field | Example | Purpose |
|---|---|---|
| `source_repo` | `basic-quote-processor` | Which repo produced the input |
| `source_name` | `basic_nvda_60s` | Logical name of the input |
| `source_hash` | `a3b8f1c4...` (64-hex) | Content-addressable ID |
| `source_commit` | `97badff` | git SHA of the source repo at emit time |
| `source_commit_dirty` | `false` | Was there uncommitted state? |

### §11.3 Lineage Traversal

`hft-ops ledger lineage <experiment_id>` recursively walks the `lineage` table:

```
Experiment: tlob_regression_e5_20260313T061500_92fb8c12
└── Consumed:
    export@feature-extractor-MBO-LOB#abc1234 (hash=5bdbdd8d...)
    └── Consumed:
        raw_data@databento-raw (XNAS.ITCH/NVDA/2025-02-03..)
```

Downstream walk:

```
Experiment: basic_nvda_60s_20260415T120000_a3b8f1c4
└── Consumed by:
    ├── tlob_offexchange_20260420T... (source_repo=lob-model-trainer, commit=def5678)
    ├── E10_5path_20260401T... (source_repo=hft-feature-evaluator)
    └── ...
```

### §11.4 Cross-Repo Git State Capture

Each producer MUST record its OWN git state in envelope `git` field. The `lineage.source_commit` is populated by the CONSUMER from the upstream envelope's `git` field at emit time.

Example: trainer consumes an export; trainer's envelope has:

```json
{
  "git": {"commit_hash": "trainer_sha_abc1234", "dirty": false, ...},
  "lineage": [
    {
      "source_kind": "export",
      "source_name": "nvda_triple_barrier",
      "source_hash": "export_content_hash",
      "source_repo": "feature-extractor-MBO-LOB",
      "source_commit": "extractor_sha_xyz7890",  // from export envelope's git.commit_hash
      "source_commit_dirty": false
    }
  ]
}
```

### §11.5 Future OPRA Integration

OPRA feature extractor (future repo) must emit envelopes matching the v1 schema with:

- `producer = "opra-feature-extractor"` (add to enum in contract bump)
- `asset_class = "option"`
- `data_source_type = "opra_options"`
- `feature_schema_ref = "opra_v1"` (new schema registered in `pipeline_contract.toml`)

OPRA-specific feature_schema is registered via:

```toml
[orchestration.feature_schemas.opra_v1]
total_features = 24
schema_version = "1.0"
feature_names = [
    "implied_volatility",
    "delta",
    "gamma",
    "theta",
    "vega",
    "moneyness",
    "days_to_expiry",
    # ...
]
```

Downstream trainers can consume OPRA exports with:

```json
"lineage": [
  {"source_kind": "export", "source_repo": "opra-feature-extractor", "source_commit": "..."}
]
```

No schema changes needed for Phase 10 ingest — v1 already accommodates via `feature_schema_ref`.

---

## §12 — Failure Modes + Recovery

### §12.1 Top 10 Must-Handle Failures

Per Round 9 V3 agent. Each has: plausibility, detection, recovery, prevention.

| # | Failure | Plausibility | Detection | Recovery | Prevention |
|---|---|---|---|---|---|
| 1 | Ingest interrupted after JSON record, before Parquet/SQLite | Plausible (kill -9, power loss) | Orphaned JSON record; SQLite missing | `hft-ops ledger rebuild` replays JSON log | Write order: JSON → Parquet → SQLite COMMIT |
| 2 | SQLite COMMIT fails mid-transaction | Plausible (disk full) | WAL rollback; no partial state | Retry ingest (JSON + Parquet still valid) | 2-phase pattern; content-addressed idempotent retry |
| 3 | SQLite file corruption | Rare (cosmic ray, OS bug) | `PRAGMA integrity_check` fails at startup | Rebuild from records/ JSON | WAL + `synchronous=NORMAL`; nightly `.backup` |
| 4 | Two producers write same content_hash to inbox | Plausible (idempotent re-emit) | `os.replace` overwrite is no-op | None needed (idempotent) | Content-addressable filenames |
| 5 | Ingest + Rebuild run concurrently | Plausible (user error) | Lock acquisition fails | Retry (locks are brief) | `flock()` on ledger.lock |
| 6 | Envelope schema violation | Plausible (producer bug) | Pydantic validation fails | Quarantine + `.error` sidecar; `hft-ops ledger quarantine list/retry/drop` | Strict schema at producer + ingester |
| 7 | envelope_version unsupported | Plausible (newer producer) | Dispatcher lookup fails | Quarantine; require orchestrator upgrade | Versioned handler registry; forward-compat `metadata_json` |
| 8 | Fingerprint algorithm changes | Rare (planned bump) | Recomputed hash differs from stored | `fingerprint_history` table + multi-version lookup | Never modify algorithm without bump; preserve old hashes |
| 9 | Migration step fails halfway (e.g., 17 of 34 records) | Plausible | Progress file in `.migration_state` | `hft-ops ledger migrate --resume` | Idempotent INSERT OR IGNORE |
| 10 | User manually edits `records/*.json` | Possible | `hft-ops ledger check` detects SQLite-vs-JSON drift | Rebuild | Docs warn against; no programmatic prevention |

### §12.2 Top 10 Defer-Until-Production

| # | Failure | Why Defer |
|---|---|---|
| 1 | SQLite > 10 GB | Not year-1 issue; monitor |
| 2 | Single Parquet > 500 MB | Pre-flight size check at write; unlikely |
| 3 | NFS multi-writer | Explicitly unsupported v1; warn at startup |
| 4 | 10k+ envelopes in inbox | Batch processing handles; users notice earlier |
| 5 | Timezone confusion in queries | Docs; ISO 8601 UTC everywhere |
| 6 | Path leakage in shared ledger export | Manual anonymize CLI |
| 7 | Parquet schema evolution breaks reads | Strict column projection |
| 8 | `git pull` during ingest | `.gitignore` rules |
| 9 | Rollback of migration with new data | `schema_version` pragma gate |
| 10 | Render-indexes race with ingest | Eventual consistency acceptable |

### §12.3 Recovery Operations (CLI)

```bash
# Integrity checks
hft-ops ledger check                                  # SQLite integrity + JSON/SQLite sync + Parquet refs
hft-ops ledger check --verbose
hft-ops ledger check --fix-orphans                    # remove Parquet files not referenced by SQLite

# Rebuild
hft-ops ledger rebuild                                # drop + recreate SQLite from records/
hft-ops ledger rebuild --fingerprint-version 1        # explicit algorithm
hft-ops ledger rebuild --dry-run

# Backup
hft-ops ledger backup --to backups/2026-04-15.sqlite  # sqlite3 .backup
hft-ops ledger restore --from backups/2026-04-15.sqlite

# Migration
hft-ops ledger migrate                                # initial 4-step migration
hft-ops ledger migrate --resume                       # resume from checkpoint
hft-ops ledger migrate --step 2                       # run specific step

# Quarantine
hft-ops ledger quarantine list
hft-ops ledger quarantine retry <hash>
hft-ops ledger quarantine drop <hash>
hft-ops ledger quarantine drain                       # retry all

# Render indexes (idempotent)
hft-ops ledger render-indexes
hft-ops ledger render-indexes --force                 # overwrite even if timestamp is current

# Fingerprint drift detection
hft-ops ledger verify-fingerprints                    # recompute for sample; flag drift
hft-ops ledger verify-fingerprints --all              # exhaustive

# Parquet regeneration from source outputs
hft-ops ledger regenerate-parquet --from-outputs lob-model-trainer/outputs/
```

### §12.4 Monitoring Signals

**ERROR** (block operation, require user action):
- `PRAGMA integrity_check` != 'ok'
- Migration step non-resumable failure
- Envelope schema violation with no fallback version
- Parquet content hash mismatch
- NFS filesystem detected at startup
- Disk space < 10× estimated ingest batch size

**WARN** (log, proceed):
- Orphaned Parquet files detected
- Envelope in inbox > 24h
- Fingerprint collision across `envelope_version`
- WAL file > 100 MB
- Envelope has unknown fields (`metadata_json` absorbs)
- Gate result missing for a commonly-run gate
- Quarantine non-empty at startup
- Schema version gap (DB older than expected)

**INFO** (audit trail):
- Every successful ingest: envelope_content_hash, experiment_id, fingerprint, bytes
- Every rebuild: records_scanned, rows_inserted, duration_ms
- Every migration step completion
- Every render-indexes run

---

## §13 — Migration Plan

### §13.1 4-Step Migration (Phase 11 executes)

**Step 1: Build alongside**. 
- Create SQLite DB + schema at `hft-ops/ledger/ledger.sqlite`.
- Deploy new `hft-ops ledger ingest/query/show/compare` CLI.
- Producers continue writing legacy JSON records AND envelopes (dual-write).
- Readers use old OR new query paths.
- **Risk**: producer envelope bugs. **Mitigation**: `--dry-run` mode, tests.

**Step 2: Backfill 34 existing records**.
- Run `hft-ops ledger migrate --step backfill-legacy`.
- For each `records/*.json`, produce v0 envelope (permissive schema), ingest.
- Data loss documented: `producer_version` may be `"unknown"`; some metrics flatten loosely.
- Verify: count matches; rendered `EXPERIMENT_INDEX.md` diff vs. hand-written is reasonable (not byte-identical — semantic equivalence only).
- Existing fingerprints PRESERVED verbatim; no recomputation.
- **Risk**: malformed legacy record. **Mitigation**: quarantine + skip; manual repair post-hoc.

**Step 3: Switch writers**.
- Producers stop dual-writing; emit envelopes only.
- Legacy `hft-ops/ledger/records/*.json` becomes the authoritative log (already was; now sole source).
- Deprecated CLI commands emit warnings (pointing to new).
- **Risk**: lost writes during switchover. **Mitigation**: deploy during quiet period; monitor.

**Step 4: Deprecate old JSON format**.
- Tag git commit `pre-ledger-v2-migration`.
- Keep `hft-ops/ledger/records/*.json` (source of truth).
- Old tools that scan `records/*.json` continue to work.
- Remove legacy `hft-ops/src/hft_ops/ledger/{ledger.py,experiment_record.py}` code paths.
- **Risk**: tool-chain breakage. **Mitigation**: `CHANGELOG.md` entry; search-and-replace for affected tools.

### §13.1b Legacy `ExperimentRecord` → envelope-v0 Field Mapping

**Round 11 rewrite**: the previous table referenced 18 field names that DO NOT exist on the `ExperimentRecord` dataclass AND omitted 9 fields that DO exist. This version enumerates the actual 25 dataclass fields at `experiment_record.py:88-132` plus the 7 nested fields of `Provenance` at `lineage.py:195-245`. Ground truth verified by direct grep at Round 11 time.

#### Actual `ExperimentRecord` fields (25 dataclass fields; `experiment_record.py:88-132`)

| # | Legacy field (type) | → envelope-v1 target | Transformation |
|---|---|---|---|
| 1 | `experiment_id: str` | `.experiment_id` | Direct copy. |
| 2 | `name: str` | `.metadata_json.legacy_name` AND prepended to `.description` | Legacy "name" is a free-form label (e.g., "HMHP_128feat_XNAS_v2"); v1 envelope has no `name` field. Preserve verbatim for traceability. |
| 3 | `manifest_path: str` | `.metadata_json.legacy_manifest_path` | Historical YAML manifest absolute path; not a v1 concept. Preserved. |
| 4 | `fingerprint: str` | `.experiment_fingerprint` | Direct copy; `fingerprint_version = 1`. Byte-identical preservation is the §13.1 success criterion #2. |
| 5 | `provenance: Provenance` | `.git` + `.lineage[]` + `.metadata_json.legacy_provenance_schema_version` | Decomposed per §13.1b-Provenance below. |
| 6 | `contract_version: str` | `.pipeline_contract_version` | Direct copy; default `"2.2"` if empty. |
| 7 | `extraction_config: Dict[str, Any]` | `.config_source` (serialized JSON) + `.config_format="json"` | If `.config_source` would exceed 8 KB, redirect to `.artifacts[]` with `kind="config"`, `path="archive/legacy/{exp_id}/extraction_config.json"`, and set `.config_source = null`. ★ |
| 8 | `training_config: Dict[str, Any]` | `.artifacts[]` with `kind="training_config"` + key subset duplicated into `.model_type`, `.model_family`, `.task`, `.sampling_strategy`, `.sampling_config`, `.label_family`, `.label_spec`, `.primary_horizon`, `.n_horizons`, `.horizons` | Parse trainer YAML-equivalent dict; extract typed fields. `training_config.model.model_type` → `.model_type`; `training_config.data.labeling_strategy` → `.label_family`; etc. Full dict preserved as artifact. ★ |
| 9 | `backtest_params: Dict[str, Any]` | `.artifacts[]` with `kind="backtest_params"` + subset into `.strategy_info` | Typed fields extracted into `.strategy_info`; full dict preserved. ★ |
| 10 | `training_metrics: Dict[str, Any]` | `.metrics[]` long-format rows | One row per key: `{family: <inferred>, name: key.upper(), split: <inferred or "full">, horizon: <inferred or null>, epoch: "final", value: v}`. Family inferred by regex: `^ic` / `^pearson` / `^r2` → `regression`; `^sharpe` / `^total_return` / `^drawdown` / `^win` → `backtest_pnl`; `^acc` / `^f1` / `^precision` / `^recall` → `classification`; else `custom`. ★ |
| 11 | `backtest_metrics: Dict[str, Any]` | `.metrics[]` with `family="backtest_pnl"` (default) | Same flattening as training_metrics. Keys with `^option_` prefix get `family="option_metrics"`. |
| 12 | `dataset_health: Dict[str, Any]` | `.metrics[]` with `family="dataset_health"` | Same flattening. |
| 13 | `tags: List[str]` | `.tags[]` | Direct copy (each string becomes one row in `tags` table via §5.3). |
| 14 | `hypothesis: str` | `.hypothesis` | Direct copy. Empty string allowed for retroactive (§3.2 says "mandatory per hft-rules §13 but accepts empty for retroactive"). |
| 15 | `description: str` | `.description` | Direct copy. Concatenated with `name` for readability: `f"[{name}] {description}"`. |
| 16 | `notes: str` | `.notes` (top-level) | Direct copy. Additionally, if `len(notes) > 64`, insert one row into `notes` table via post-ingest pass `extract_legacy_notes()`; author="legacy-migrator"; created_at=legacy `created_at`. |
| 17 | `created_at: str` | `.created_at` | Direct copy (legacy uses `datetime.now(timezone.utc).isoformat()` → UTC with offset suffix; migration rewrites `+00:00` to `Z` for §14.8 consistency). |
| 18 | `duration_seconds: float` | `.wall_clock_ms` | `int(round(duration_seconds * 1000))`. |
| 19 | `status: str` | `.status` | Direct copy. Legacy values `{pending, completed, failed, partial}` all match v1 enum; any other value → quarantine. |
| 20 | `stages_completed: List[str]` | `.metadata_json.legacy_stages_completed` | Free-form list of stage names; v1 records stage completion via per-stage envelopes instead. Preserved. |
| 21 | `sweep_id: str` | `.sweep_id` | Direct copy. Empty → NULL. |
| 22 | `axis_values: Dict[str, str]` | `.axis_values` (JSON-serialized) | Direct copy. Empty dict → NULL. |
| 23 | `record_type: str` | `.record_type` | Direct copy; all 6 legacy values match v1 CHECK constraint (§5.3). |
| 24 | `sub_records: List[Dict[str, Any]]` | `.sub_records` | Direct copy (each entry is a sweep-child summary dict). Empty for non-aggregate types. |
| 25 | `parent_experiment_id: str` | `.parent_id` | Direct copy. Empty → NULL. |

#### `Provenance` sub-structure (`lineage.py:195-245`; decomposed on migration)

| Nested field | → envelope-v1 target | Transformation |
|---|---|---|
| `provenance.git.commit_hash` | `.git.commit_hash` | Direct copy. `"not_git_tracked"` sentinel passes §3.2 regex. |
| `provenance.git.branch` | `.git.branch` | Direct copy. |
| `provenance.git.dirty` | `.git.dirty` | Direct copy (bool). |
| `provenance.git.short_hash` | `.git.short_hash` | Direct copy. |
| `provenance.config_hashes` (dict) | `.metadata_json.legacy_config_hashes` | `{extractor: sha, trainer: sha, manifest: sha}`. v1 doesn't hash config files separately (the extraction/training configs themselves are in `.config_source` or artifacts). Preserved for forensic comparison. |
| `provenance.data_dir_hash` | `.lineage[]` with `source_kind="export"` or `"raw_data"`, `source_hash = <dir_hash>`, `source_name = <absolute_path>` | Creates ONE lineage row pointing back to the data directory. `source_repo` inferred from path prefix: `"databento-raw"` if under `data/raw/`, `"basic-quote-processor"` if under `data/exports/basic_*/`, etc. |
| `provenance.contract_version` | (duplicate of record-level `.pipeline_contract_version`; drop if same value) | — |
| `provenance.timestamp_utc` | `.metadata_json.legacy_provenance_timestamp` | Retained for audit comparison with `.created_at`. |
| `provenance.retroactive` | `.metadata_json.legacy_retroactive = true` | v1 envelope has no `retroactive` field at top level; flagged in metadata_json. All 34 legacy records have `retroactive=true`. |
| `provenance.schema_version` | `.metadata_json.legacy_provenance_schema_version` | Currently `"1.0"`. |

#### v1 envelope fields set by the migrator (not sourced from legacy)

| Envelope-v1 target | Value set by migrator | Rationale |
|---|---|---|
| `.envelope_version` | `1` | All migrated records use the v1 envelope. |
| `.envelope_schema_version` | `"1.0.0"` | Current v1 schema version. |
| `.producer` | `"legacy-migrator"` | Distinguishes migrated records from live producer emissions. §3.2 `producer` enum MUST include this entry for Phase 11. |
| `.producer_version` | `"0.0.0"` | Sentinel for "legacy migrator v0". |
| `.symbol` | `"NVDA"` | All 34 legacy records are NVDA (verified at migration time by inspecting the `training_config.data.symbol` fallback; quarantine with `.error` if symbol cannot be determined). |
| `.symbols_json` | `null` | Single-symbol. |
| `.asset_class` | `"equity"` | All 34 legacy records are NVDA equity. |
| `.data_source_type` | Inferred from `training_config.data.source_type` OR from lineage.source_repo: `"equity_lob_mbo"` if producer touched MBO extractor, `"equity_off_exchange_trf"` if BQP, else `"legacy-unknown"`. | ★ |
| `.feature_schema_ref` | Inferred: `"equity_v2.2"` for MBO records; `"off_exchange_1.0"` for BQP records; else `"legacy-unknown"`. | ★ |
| `.json_record_path` | `records/{experiment_id}.json` | Set by ingester (§7.3). |
| `.ingested_at` | `now()` UTC at migration time | Set by ingester. |
| `.heartbeat_at` | `null` | Legacy doesn't stream. |
| `.finalized_at` | `.created_at + timedelta(seconds=duration_seconds)` — reconstructed | Legacy has no explicit finalized_at; approximated from created_at + duration. |
| `.dataset` | Inferred: `training_config.data.dataset_name` OR `null`. | |
| `.bulk_parquet[]` | `[]` | Legacy has no Parquet side files. |
| `.export_stats` | `null` (for non-export record_types); for export legacy records, best-effort extraction from `training_metrics` keys prefixed `export_stats.` | |
| `.training_info` | For `record_type='training'` only: extracted from `training_config` + `training_metrics` (best_epoch, total_epochs, model_params). **`.training_info.normalization_*` left as `null`** — legacy records don't carry this (§14.8 invariant EXEMPTS `producer='legacy-migrator'`). | ★ |
| `.signal_provenance` | For `record_type='backtest'` only: populated from `backtest_params` + parent_experiment_id. `.signal_provenance.trainer_fingerprint` looked up via `parent_experiment_id → experiments.experiment_fingerprint`. If lookup fails, ingest proceeds but §14.8 check is skipped (legacy-migrator exemption). | ★ |
| `.cohort_hash` | Computed by ingester post-INSERT (same path as v1; §6.4). | |
| `.gates[]` | `[]` (legacy records have no structured gate data — gates were narrative-only). | |
| `.lineage[]` | Populated from `provenance.data_dir_hash` + `parent_experiment_id` (if non-empty → one extra lineage row with `source_kind="trainer_run"`, `source_hash = parent.experiment_fingerprint`). | ★ |
| `.artifacts[]` | Populated from `extraction_config`, `training_config`, `backtest_params` (each serialized to a file under `archive/legacy/{exp_id}/` + hashed). | ★ |
| `.metrics[]` | Long-format flattening of `training_metrics` + `backtest_metrics` + `dataset_health`. | ★ |

#### Legacy-migrator exemptions from Phase 11 integrity gates

The §14.8 (normalization_stats_sha256) and §14.9 (upstream fingerprint) invariants are HARD FAILS for live producers but EXEMPT for envelopes where `producer == "legacy-migrator"`. Rationale: the 34 retroactive records pre-date both conventions and retroactively fabricating hashes would break the §13.1 "fingerprints preserved verbatim" guarantee. Legacy exemption is enforced by adding `producer_version_exempt_from_integrity` to the ingester's policy module (Phase 11 deliverable).

#### Migration test suite (§18.5, expanded)

For each of the 34 legacy records:

1. Load `records/{id}.json` → instantiate `ExperimentRecord` via `from_dict()`.
2. Transform to envelope-v1 via `legacy_to_envelope_v1(record)`.
3. Ingest via the standard §7.3 pipeline.
4. Verify **byte-identical `experiment_fingerprint`** pre- and post-migration (§13.1 success criterion #2).
5. Verify `SELECT COUNT(*) FROM experiments WHERE producer = 'legacy-migrator'` returns 34.
6. Verify all 25 ExperimentRecord fields were mapped (no silent data loss; `.metadata_json.legacy_*` keys cover any field not mapped to a typed envelope slot).
7. Re-render `EXPERIMENT_INDEX.md` section for each legacy experiment; fuzzy-match against the hand-written entry in `lob-model-trainer/EXPERIMENT_INDEX.md` (field coverage, key metrics, gate results — not byte-identical).
8. Verify §14.8 / §14.9 integrity gates DO NOT fire (exempted by producer="legacy-migrator").
9. Property test: replaying the migration twice is idempotent (inbox→records→SQLite path unchanged on second run).

### §13.1c Phase 4 Batch 4c.3 / 4c.4 Coordination (Round 17 I2 + I3)

**Pending couplings that affect Phase 10 fingerprint + field-mapping invariants:**

**Batch 4c.3 (fingerprint hook) cutover** — `hft-ops/src/hft_ops/ledger/dedup.py:352-373` currently passes `feature_set` / `feature_preset` through UNRESOLVED into `components["training"]` for the Phase 3 canonical-JSON hash. When Batch 4c.3 wires `feature_set → sorted feature_indices` resolution BEFORE the hash (per PA §3097), **every live record whose `training_config.data.feature_set` was set pre-4c.3 will silently change fingerprint** on the cutover.

§13.1 success criterion #2 ("byte-identical fingerprint preservation") protects ONLY the 34 retroactive records (none of which use `feature_set`), NOT live records produced between Batch 4b ship (2026-04-15) and Batch 4c.3 ship. Phase 11 ingest reuses `compute_fingerprint` verbatim (§6.1) — so the cutover impacts it.

**Required coordination**:
1. Before Batch 4c.3 merges, enumerate affected live records via:
   ```sql
   SELECT experiment_fingerprint, experiment_id
   FROM experiments
   WHERE json_extract(config_source, '$.data.feature_set') IS NOT NULL;
   ```
2. For each affected row, compute the new fingerprint (post-4c.3 algorithm) and write an entry into `fingerprint_history(experiment_id, fingerprint_version, fingerprint_value, computed_at)` per §5.3. Set `fingerprint_version = 2` (algorithm bump per §6.5).
3. Update `experiments.experiment_fingerprint` to the new value (the PK changes — drop and re-INSERT; CASCADE FKs follow).
4. This IS a `compute_fingerprint` algorithm version bump — follows the §6.5 version-bump protocol. Add an RFC doc to `docs/plan/` describing the rationale.

**Batch 4c.4 (provenance plumbing) cutover** — Batch 4c.4 (PA §3098) adds `feature_set_ref` as the **26th field** to `ExperimentRecord`. §13.1b's current mapping table enumerates 25 fields (Round 11 ground-truth). When 4c.4 lands:

- §13.1b must gain a row #26: `feature_set_ref: {name, content_hash} | None` →
  - OPTION A (minimal): `.metadata_json.legacy_feature_set_ref` (escape hatch; preserves the value without a typed envelope slot)
  - OPTION B (typed): new top-level envelope field `feature_set_ref: Optional[FeatureSetRef]` (adds a Pydantic sub-model `FeatureSetRef(BaseModel): name: str; content_hash: str`; requires envelope_schema_version bump to `1.1.0`)

Recommend OPTION A for Batch 4c.4 ship; OPTION B deferred to Phase 11.5 if typed lineage becomes useful. Coordination: the PR that adds Batch 4c.4's `ExperimentRecord.feature_set_ref` MUST also (a) amend §13.1b to row #26, (b) update the `legacy_to_envelope_v1` migrator to route the new field, (c) add a test in `hft-ops/tests/test_legacy_migration.py` asserting the new field round-trips. Track in `gentle-brewing-quail.md` pending items.

**Gate**: Phase 11 ingest coding (Week 2) SHOULD NOT begin the `legacy_to_envelope_v1` migrator until Batch 4c.4's `feature_set_ref` status is decided (landed OR deferred). If deferred, migrator uses the current 25-field table verbatim. If landed, migrator pulls row #26 from the amended §13.1b.

### §13.2 Markdown Ledger Extraction

One-time pass over `EXPERIMENT_INDEX.md`, `BACKTEST_INDEX.md`, `EXPORT_INDEX.md`:

```python
def extract_prose_to_notes(md_path: Path, ledger: Ledger) -> None:
    sections = parse_markdown_sections(md_path)  # split by ### or ##
    for section in sections:
        exp_id = infer_experiment_id(section.title)
        if exp_id and ledger.has_experiment(exp_id):
            ledger.add_note(
                exp_id,
                body_md=section.body,
                author=f"migrated-from-{md_path.name}",
                created_at=file_mtime,
            )
```

Handles section header patterns (`### E10:`, `### HMHP H10 Primary`, `## Round 1:`, etc.) via regex heuristics + manual review for edge cases.

After extraction:
- Original markdown files archived to `archive/markdown-ledger-v0/`.
- New auto-generated markdown replaces them.
- All prose preserved in `notes` table, retrievable via `hft-ops ledger note list <id>`.

### §13.3 Phase 3.5 Batch Coordination

The 36-config migration (Phase 3.5, 4 batches) is ORTHOGONAL to Phase 10 migration but must coordinate:

- Phase 10 Step 1-2 can proceed in parallel with Phase 3.5.
- Phase 10 Step 3 requires Phase 3.5 Batch 1 complete (all 5 E5 configs migrated to `_base:` multi-base).
- **Round 13 (B3)**: gate widened from "Batch 1 only" to **"every Phase 11 Step-3 producer-rollout milestone must pass `hft-ops sweep fingerprints --verify` clean across ALL 4 Phase 3.5 batches."** Rationale: mid-Phase-11 migrations of Batches 2-4 mutate fingerprint inputs (a trainer config moves from legacy form to `_base:` form); if the resolved-config output of `resolve_inheritance` differs from the pre-migration serialization, the fingerprint changes and the ledger sees two distinct rows for what should be the same experiment.
- Reason: Phase 10 Step 3 producers must compute fingerprints using `resolve_inheritance` — Phase 3.5 is what populates the bases.

**Coordination checkpoints (weekly, Phase 11 Weeks 1-10)**:

```bash
# Every Monday of Phase 11:
hft-ops sweep fingerprints --verify --configs-dir lob-model-trainer/configs/
# Expected: zero drift. Any drift halts producer rollout for that week.

# Per-batch gate: before declaring a Phase 3.5 batch "complete", run:
hft-ops sweep fingerprints --verify --batch {1,2,3,4} --strict
# Halt if any config's resolved-form hash differs pre- vs post-migration.
```

**Incident protocol**: if fingerprint drift is detected mid-Phase-11:
1. `fingerprint_history` captures both old and new hashes.
2. Affected producers emit envelopes with the NEW fingerprint; the `fingerprint_history` JOIN preserves cross-version lookup.
3. Phase 3.5 batch owner authors an RFC explaining the semantic divergence and either (a) commits the drift as intentional, or (b) rolls back the migration step.
4. No producer rollout in the affected stage until the drift is resolved.

### §13.4 Rollback Plan

Every step has a rollback:

| Step | Rollback |
|---|---|
| 1 | `rm hft-ops/ledger/ledger.sqlite*`; revert ingest CLI code |
| 2 | Delete new records created by backfill (known by `ingested_at`); restore JSON from git |
| 3 | Re-enable dual-write in producers; `rm ledger.sqlite` |
| 4 | `git revert pre-ledger-v2-migration`; restore legacy code |

Each rollback is a single git revert + `rm` operation — no data loss if rolled back before next commit.

---

## §14 — Phase 3 Compatibility Invariants (Normative)

The following invariants MUST be preserved by Phase 10 implementation:

### §14.1 Fingerprint Algorithm (Phase 3.3b)

- `compute_fingerprint()` logic at `hft-ops/src/hft_ops/ledger/dedup.py:284-391` is CANONICAL.
- `_load_trainer_config_resolved()` (`dedup.py:117-195`) resolves `_base:` before hashing via `lobtrainer.config.merge.resolve_inheritance` (`merge.py:85-182`). Phase 10 does NOT bypass this.
- Inline and path-based trainer_config forms produce identical fingerprints. Phase 10 preserves.

### §14.2 Exclusion Set

Fields excluded from fingerprint (§6.2):
- `{name, description, tags, version, output_dir, log_level, verbose, experiment}`
- Entire `stages.validation`
- `provenance.retroactive`, `created_at`, `finalized_at`, `heartbeat_at`, `wall_clock_ms`

Phase 10 does NOT add or remove from this set without protocol (§6.5).

### §14.3 Record Immutability

`ExperimentRecord` fields are immutable after `register()` EXCEPT `notes`. Phase 10 preserves:
- `experiments` table: no UPDATE except `heartbeat_at`, `finalized_at`, `status` (lifecycle progression only).
- `metrics`, `gates`, `artifacts`, `lineage`, `bulk_parquet_refs`: INSERT only; no UPDATE, no DELETE (except via CASCADE from experiment DELETE — which is only for rollback/cleanup, not normal ops).
- `notes`: APPEND only; no UPDATE on existing notes (new note with revised content is the correction pattern).
- `tags`: INSERT and DELETE allowed (tags are human-curated).

### §14.4 `_partial: true` Sentinel

Phase 3's `_partial: true` YAML sentinel (used in intermediate base configs) is preserved. Phase 10 envelope ingestion DOES NOT resolve `_partial` — resolution happens at config-load time before fingerprinting.

### §14.5 `HFT_OPS_ORCHESTRATED=1` Env Var

Phase 10 preserves the convention that subprocesses invoked by hft-ops get this env var (`stages/base.py:138`). Producers check it to adjust behavior (e.g., suppress deprecation warnings).

### §14.6 Subprocess Isolation

hft-ops continues to NOT Python-import runner modules. Two soft exceptions (`lobtrainer.config.merge`, `hft_evaluator.fast_gate`) are preserved. Phase 10 does not add new soft exceptions without documented rationale.

### §14.7 Stage Order

Extraction → raw_analysis → dataset_analysis → validation → training → signal_export → backtesting. Phase 10 preserves this order for `hft-ops run`. The sweep runner v2 respects the same order per grid point.

### §14.8 Normalization-Stats Integrity (Round 11 Agent B BQ3)

**Invariant**: for any `record_type='backtest'` envelope `B` with `upstream_experiment_ids = [T_id, ...]`, the backtester's `signal_provenance.normalization_stats_sha256` MUST byte-equal the trainer's `training_info.normalization_stats_sha256` where the trainer is `experiments.experiment_id == T_id`.

**Enforcement**: §7.6.1 rule 10 + a dedicated check at ingest time. **Round 13 refinement (D4)**: the check distinguishes two failure modes — "upstream hasn't arrived yet" (soft retry via `pending/`) vs "upstream stats mismatch" (hard quarantine). These MUST NOT share an exception type.

```python
class UpstreamNotYetIngestedError(LookupError):
    """Soft failure: upstream producer's envelope hasn't been ingested yet.
    Envelope routed to pending/; retried on every subsequent `hft-ops ledger
    ingest` call; escalates to quarantine after age-out (default 24h)."""

class NormalizationStatsMismatchError(ValueError):
    """HARD failure: backtester's stats hash differs from trainer's stats hash.
    Envelope immediately quarantined; indicates real train/inference drift."""

def check_normalization_integrity(validated: Envelope, conn: sqlite3.Connection) -> None:
    """Raise UpstreamNotYetIngestedError if trainer is absent (retry).
    Raise NormalizationStatsMismatchError if hashes differ (quarantine).
    """
    if validated.record_type != "backtest":
        return
    if validated.producer == "legacy-migrator":
        return  # §13.1b legacy-migrator exemption
    if validated.signal_provenance is None or validated.signal_provenance.normalization_stats_sha256 is None:
        if validated.signal_provenance and validated.signal_provenance.normalization_method in (None, "none"):
            return  # trainer used raw features — no stats to check
        raise NormalizationStatsMismatchError("backtest missing signal_provenance.normalization_stats_sha256 despite non-'none' normalization_method")

    trainer_fp = validated.signal_provenance.trainer_fingerprint
    # Round 14 C1: read training_info_json column directly (not metadata_json).
    # The JSON blob is stored whole as canonical JSON; json_extract pulls the
    # single hash field without full-blob Python parse.
    row = conn.execute(
        "SELECT json_extract(training_info_json, '$.normalization_stats_sha256') FROM experiments WHERE experiment_fingerprint = ?",
        (trainer_fp,)
    ).fetchone()
    if row is None:
        # SOFT failure: upstream not yet in ledger — route to pending/
        raise UpstreamNotYetIngestedError(
            f"trainer fingerprint {trainer_fp[:8]}... not yet in ledger; envelope will retry on next ingest call"
        )
    trainer_sha = row[0]  # already the hash string or NULL via json_extract
    if trainer_sha != validated.signal_provenance.normalization_stats_sha256:
        # HARD failure: real hash mismatch — train/inference drift detected
        raise NormalizationStatsMismatchError(
            f"normalization_stats_sha256 mismatch: trainer={trainer_sha[:8] if trainer_sha else 'NULL'}, "
            f"backtest={validated.signal_provenance.normalization_stats_sha256[:8]}. "
            f"Train/inference drift detected under T15; backtest would produce silently-wrong signals."
        )
```

**Ingest dispatch** (refined in §7.3):
- `UpstreamNotYetIngestedError` → move envelope to `hft-ops/ledger/pending/{content_hash}.json` + `.waiting` sidecar listing missing fingerprints. Next `hft-ops ledger ingest` call retries pending/ first (step 1 of the algorithm). Age > 24h → escalate to quarantine with `.error = "upstream never arrived"`.
- `NormalizationStatsMismatchError` → immediate quarantine with `.error` sidecar containing both hashes for forensic review.

**Rationale**: T15 "Raw Rust, Variable Python" means normalization is the **single largest source of silent train/inference divergence**. Without this check, two backtest envelopes with identical fingerprints could consume DIFFERENT normalization stats (e.g., if the trainer's checkpoint was re-saved with a different seed and stats got reshuffled). The fingerprint does NOT include normalization_stats because stats are runtime-computed, not config — so content-addressed identity cannot catch this class of bug. §14.8 closes the gap, AND distinguishes ordering races (harmless, retry) from real drift (fatal, quarantine) so operators aren't drowned in false alarms during sweeps.

**Tests** (§18.2 Normalization integrity + pending retry, 4 tests):
- Insert trainer envelope with `normalization_stats_sha256 = "abc..."`; then ingest backtest with matching sha → passes.
- Ingest backtest with MISmatching sha → `NormalizationStatsMismatchError` → quarantine; `.error` sidecar contains both hashes.
- Ingest backtest BEFORE trainer → `UpstreamNotYetIngestedError` → `pending/`. Then ingest trainer → next ingest call retries pending/ → backtest succeeds.
- Ingest backtest with upstream never arriving → after simulated 24h, escalates from `pending/` to quarantine.

### §14.9 Upstream Fingerprint Integrity (Round 11 Agent B BQ7)

**Invariant**: every `lineage[].source_hash` that names an in-ledger producer experiment MUST resolve to exactly one `experiments.experiment_fingerprint` row (or to a `fingerprint_history.fingerprint_value` row). Free-form `source_name` references are permitted only for raw data (source_kind == "raw_data"), reconstructor output (source_kind == "reconstructor"), and other non-ledger artifacts.

**Enforcement**: §7.6.2 warn-only rule 4 (upstream ingest ordering race) becomes §7.6.1 HARD-FAIL after a grace period. Phase 11 deploys as warn; Phase 13 promotes to hard-fail once producer rollouts stabilize (see §15 timeline).

See §7.1.1 for the producer-side API that resolves upstream fingerprints at emit time.

---

## §15 — Implementation Phases (Phase 11-14+)

### §15.1 Phase 11: Ledger v2 Core + Migration (8-10 weeks, weekly go/no-go gates)

**Round 13 timeline correction (D1)**: the original "3 weeks" (Round 9) and "4 weeks" (Round 10 bump) estimates were both optimistic. Agent P1 (Round 12) mid-estimate for Python-producers-only scope: ~355 focused hours ≈ 8-9 weeks at 40h/wk; high-end ~11 weeks. State the range honestly to avoid schedule pressure cutting Tier-1 invariants. Each week ends with an explicit go/no-go gate (§15.1a) — if a gate fails, re-scope the next week rather than force the schedule.

**Scope (Round 13 — D1 descope)**: Python producers only in Phase 11. Rust producer wiring (BQP envelope emit, MBO extractor envelope emit) deferred to Phase 11.5. This descope is NOT a quality compromise — it's an explicit acknowledgment that Rust/Python canonical JSON parity (the single largest hidden risk per Rounds 10-12) must be DE-RISKED in Phase 11 Week 1 via fixture-based parity tests, but WIRING the Rust producers to actually emit envelopes can happen in Phase 11.5 once parity is proven.

**Producers covered in Phase 11**:
- Trainer (`lob-model-trainer`, Python) — H21 in dependency DAG
- Backtester (`lob-backtester`, Python) — H22, required to exercise §14.8 normalization-integrity gate end-to-end

**Producers deferred to Phase 11.5 (≈ 2 weeks later)**:
- BQP (`basic-quote-processor`, Rust) — H23
- MBO extractor (`feature-extractor-MBO-LOB`, Rust) — H24
- Evaluator (`hft-feature-evaluator`, Python) — lower priority; §14.8 doesn't depend on it

**Tier-2 items promoted to Phase 11 Week 1 (schema-affecting, cannot defer)**:
- B1 Z-timezone pattern on ISO 8601 fields (§3.2 amended) — schema-breaking if deferred, invalidates fixtures

**Tier-2 items confirmed deferred to Phase 11.5 (additive, backward-compatible)**:
- B8 `exchange` column (schema v1.1.0 minor bump; additive)
- C-FF6 JSON-Schema-from-TOML codegen + CI drift-check
- C-FF7 `metadata_json.<producer>.*` namespace warn rule
- C-SC8 cross-repo contract-bump coordination playbook

**Deliverables**:
- Contract-first envelope schema in `pipeline_contract.toml` (envelope v1 JSON Schema, MetricKey, GateKey, with Z-timezone patterns per B1).
- Python codegen → `hft_contracts/orchestration.py` Pydantic model + MetricKey/GateKey enums + canonical JSON library.
- **Rust `hft_contracts_rs` crate skeleton** — single-package layout per §2.5.1; hand-maintained `Envelope` struct + `SCHEMA_HASH` const + canonical JSON + cross-lang fixtures. **Verified by both SCHEMA_HASH check AND golden serialization test.** Rust producer wiring NOT in scope.
- Cross-lang canonical JSON fixtures (9 cases including datetime/Path/None-vs-null fallbacks) — CI parity gate.
- MetricKey + GateKey enums in `hft-contracts` (underscore form registry per §4.5; §4.2 registry parsed by TOML loader).
- SQLite schema (10 tables) + PRAGMAs (WAL, synchronous=NORMAL, foreign_keys=ON) + **§5.7 migration protocol** (numbered migrations, `hft-ops ledger migrate-schema` CLI, rollback on mid-migration failure).
- Append-only `ingest.log` JSONL file (replaces the previously-proposed `ingest_audit` table).
- **Inbox path resolution** (§7.1a): `HFT_OPS_LEDGER_INBOX` env var + `--ledger-inbox` CLI arg + orchestrated-fail-fast / standalone-warn fallback.
- **`pending/` directory** for out-of-order ingest (§14.8 / D4): `UpstreamNotYetIngestedError` routes to pending/; `ingest_all` retries pending/ first; 24h age-out to quarantine.
- CLI: `hft-ops ledger {ingest, query, show, compare, diff, list, rebuild, check, backup, quarantine, audit-log, note, tag, render-indexes, pending}`.
- **Round 15 A11**: `hft-ops sweep fingerprints --verify [--configs-dir <path>] [--batch N] [--strict]` CLI (~30 LOC) — walks each trainer config in `lob-model-trainer/configs/`, resolves `_base:` chain via `resolve_inheritance()`, hashes the resolved-effective dict with the Phase 3 algorithm (`dedup.py:284-391`), compares against pre-recorded per-config fingerprints stored in `hft-ops/ledger/fingerprint_baselines.json`. Reports drift. Invoked by §13.3 weekly Phase 3.5 coordination gate. Phase 11 Week 2 scope (~4h implementation).
- Python SDK: `Ledger` class + helpers; optional `pandas` passthrough.
- Migration CLI: 4-step backfill of 34 retroactive records (§13.1).
- **Legacy → envelope-v0 field mapping** (§13.1b) with tests that migrate 34/34 records with byte-identical fingerprint preservation.
- **Post-ingest debounced index render** (§4.6) replaces eager render; `flock`-based inter-process coordination.
- **Rollback protocol** (§7.4.1) — partial commit recovery; CLI `check --orphaned-records` + `heal --from-orphans` + `rebuild --from-records --backup`.
- **Runtime-generated INSERT statements** (§7.3.1 / B5) — from Pydantic field ordering; meta-test guards against column drift.
- **Test coverage: ≥ 80 new tests** (40+ hft-ops + 15 hft-contracts + 10 trainer + 10 backtester + 5 cross-lang parity).

**Success criteria (Phase 11 complete)**:
- One `hft-ops run` (trainer → backtester sequence) produces TWO non-retro ledger records end-to-end.
- §14.8 normalization-integrity gate verified end-to-end: matching hashes pass; mismatched hashes quarantine with forensic sidecar.
- §14.9 upstream-integrity + `pending/` retry verified: out-of-order ingest routes to pending/, subsequent ingest resolves.
- All 34 legacy records migrated with byte-identical fingerprints preserved.
- Query Q1 ("all TLOB experiments with test IC > 0.05") returns in < 100 ms p95 on 1000-record synthetic ledger.
- All 10 §12.1 failure modes covered by `test_fm_NN_*` tests (§18.3 mapping).
- `sqlite3 ledger.sqlite < schema.sql` parses cleanly (Round 10 regression — DDL syntax-correctness gate).
- `test_rust_envelope_serialization_matches_python_golden` passes (Round 13 D2 verification).
- `test_schema_hash_verify_catches_toml_drift` passes.

### §15.1a Phase 11 Week 1 Bootstrap Order (Round 13 — B7)

**Problem**: the test fixtures at §18.1b assume the Pydantic envelope model exists. The Pydantic model is codegen'd from `[orchestration.envelope]` in `pipeline_contract.toml`. That section does not exist yet. Chicken-and-egg.

**Resolution**: day-by-day Week 1 order. Strict sequential dependencies.

**Day 1 — Schema freeze in TOML** (resolves B1, D2, unblocks everything else):
- Add `[orchestration.envelope]` section to `pipeline_contract.toml`. **Round 14 C3 decision**: the JSON Schema Draft-07 object (~450 lines) is stored as a single TOML **literal multiline string** (`'''...'''`), NOT as nested inline TOML tables. Rationale: JSON Schema uses `\uXXXX`, regex `\d`, etc. — embedding as TOML basic string `"""..."""` would require doubling every `\` to `\\`; TOML literal strings (`'''...'''`) take contents verbatim. Structure:
  ```toml
  [orchestration.envelope]
  json_schema = '''
  {
    "$schema": "http://json-schema.org/draft-07/schema#",
    "type": "object",
    "required": [...],
    "properties": { ... 52 properties ... }
  }
  '''
  ```
  Python loader: `json.loads(tomllib.load(f)['orchestration']['envelope']['json_schema'])`.
- Add `[orchestration.metric_keys]` with 44 keys as native TOML tables (§4.2 v1.2 registry — KEEP native TOML form, no JSON encoding).
- Add `[orchestration.gate_keys]` with 9 keys as native TOML tables (§4.5 — same).
- Add `[orchestration.feature_schemas.equity_v2_2]`, `[orchestration.feature_schemas.off_exchange_1_0]` declarations with fields `total_count`, `layout`, `source_toml_section` (e.g., `"[features]"` or `"[features.off_exchange]"`).
- Verify: `tomllib.loads(pipeline_contract.toml)` produces a valid `orchestration.*` tree AND `json.loads(orchestration.envelope.json_schema)` produces a valid Draft-07 schema.
- **Round 15 A5 — TOML literal-string escape guard**: before committing Day 1 TOML changes, run `grep -c "'''" pipeline_contract.toml` and assert the count is even AND that no `'''` appears between the opening and closing literal-string delimiters of the `json_schema` field. TOML literal multiline strings cannot contain `'''` in their body — this would silently close the string early. Automated check: a small Python snippet in CI that parses the TOML and verifies the extracted `json_schema` string round-trips through `json.loads()` without truncation.
- Gate: schema is frozen. Restart protocol: if Day 3+ discovers a schema gap, `git reset --hard` to pre-Day-1 commit and redo Days 1-2. Each Day is its own commit on the `phase-11-w1` branch; restart = branch delete + recreate.

**Day 2 — Python codegen extension + dependency strategy**:
- **Round 14 C7 decision**: Pydantic is declared as an OPTIONAL-dependency extra, NOT a hard dep. Update `hft-contracts/pyproject.toml`:
  ```toml
  [project.optional-dependencies]
  generate = ["tomli>=2.0"]                           # existing
  orchestration = ["pydantic>=2.0,<3.0"]              # NEW Round 14 C7
  ```
  The top-level `__init__.py` continues to work without pydantic. `hft_contracts.orchestration.*` guards imports:
  ```python
  # hft_contracts/orchestration/__init__.py
  try:
      from pydantic import BaseModel
  except ImportError as e:
      raise ImportError(
          "hft_contracts.orchestration requires pydantic. "
          "Install with: pip install hft-contracts[orchestration]"
      ) from e
  from .envelope import Envelope  # noqa: E402
  from .metric_keys import MetricKey  # noqa: E402
  from .gate_keys import GateKey  # noqa: E402
  # Round 16 v1.3.3: NO new canonical_json module. Re-export from SSoT.
  from hft_contracts.canonical_hash import canonical_json_blob, sha256_hex, sanitize_for_hash  # noqa: E402,F401
  from .upstream import resolve_upstream_ref, UpstreamRef  # noqa: E402
  ```
- Extend `contracts/generate_python_contract.py` to emit `hft_contracts/orchestration/envelope.py` (Pydantic v2 model), `metric_keys.py` (enum), `gate_keys.py` (enum) from the TOML. The existing single-output-file pattern becomes multi-output — refactor `OUTPUT_PATH` into a dict keyed by generator name.
- **Round 15 B9 — Pydantic sub-model naming convention**: codegen emits one `BaseModel` subclass per nested object declared in §3.2, named in TitleCase with singular form for array items: `LineageEntry`, `ArtifactEntry`, `MetricEntry`, `GateEntry`, `BulkParquetEntry`, `ExportStats`, `TrainingInfo`, `StrategyInfo`, `SignalProvenance`, `GitInfo`. Required so the C4 `@field_serializer('override_at')` on `GateEntry` and the C4 `@field_serializer('created_at', 'finalized_at', 'heartbeat_at')` on `Envelope` attach at the correct class. `sub_records` stays `List[Dict[str, Any]]` (§3.2 declares items as opaque `{type: object}` with no properties — no nested Pydantic needed).
- **Round 15 A9 — hft-ops transitive dependency**: hft-ops consumes `hft_contracts.orchestration` (envelope validation, canonical JSON, etc.), so `hft-ops/pyproject.toml` MUST declare its hft-contracts dependency with the `orchestration` extra: `hft-contracts[orchestration]>=X.Y.Z` (NOT bare `hft-contracts`). Any consumer of hft-ops installing it via `pip install hft-ops` thus gets Pydantic transitively. Same applies to lob-model-trainer and lob-backtester's pyproject.toml when they start emitting envelopes (Week 4). Verify during Week 2 pyproject updates.
- **Round 16 v1.3.3 change**: DO NOT hand-write `hft_contracts/orchestration/canonical_json.py`. The monorepo already has the canonical form SSoT at `hft_contracts/canonical_hash.py` (Phase 4 Batch 4c, 2026-04-15). Phase 10 REUSES it. `orchestration/__init__.py` re-exports `canonical_json_blob`, `sha256_hex`, `sanitize_for_hash` for caller convenience but adds ZERO new canonical-form logic. This eliminates duplication per hft-rules §0 "Reuse-first".
- Run `python contracts/generate_python_contract.py` → verify 3 output files parse as Python (envelope.py, metric_keys.py, gate_keys.py — NO canonical_json.py).
- `pip install -e .[orchestration] && python -c "from hft_contracts.orchestration import Envelope, canonical_json_blob"` succeeds.

**Day 3 — Hand-crafted golden fixture + Pydantic Z-serializer**:
- **Round 14 C4 decision**: Pydantic v2's `model_dump(mode='json')` emits datetimes as `+00:00` UTC offset — this FAILS the B1 regex requiring `Z` suffix. The codegen'd Pydantic model MUST declare `@field_serializer` methods on every timestamp field. Add to `envelope.py`:
  ```python
  from pydantic import BaseModel, field_serializer
  from datetime import datetime
  from typing import Optional

  def _to_iso_z(dt: Optional[datetime]) -> Optional[str]:
      if dt is None:
          return None
      s = dt.strftime('%Y-%m-%dT%H:%M:%S')
      if dt.microsecond:
          s += f'.{dt.microsecond:06d}'.rstrip('0').rstrip('.') or ''
      return s + 'Z'

  class Envelope(BaseModel):
      created_at: datetime
      finalized_at: Optional[datetime] = None
      heartbeat_at: Optional[datetime] = None
      # ... other fields

      @field_serializer('created_at', 'finalized_at', 'heartbeat_at')
      def _ser_z(self, v: Optional[datetime]) -> Optional[str]:
          return _to_iso_z(v)
  ```
  Similar for `gates[].override_at` (via nested `GateEntry` sub-model's own `@field_serializer`).
- By hand, construct ONE valid envelope JSON conforming to every §3.2 field (including nested `lineage[]`, `artifacts[]`, `bulk_parquet[]`, `metrics[]`, `gates[]`, `training_info`, `signal_provenance`, `strategy_info`, `export_stats`). Avoid `.parquet` paths that would require Parquet fixtures (use `bulk_parquet: []`).
- Validate it: `Envelope.model_validate_json(sample.read_text())` must succeed.
- Save to `hft-contracts/tests/fixtures/envelopes/golden/01_complete.json`.
- Compute canonical form (Round 16 v1.3.3 — uses SSoT):
  ```python
  from hft_contracts.canonical_hash import canonical_json_blob
  env = Envelope.model_validate_json(sample.read_text())
  # sanitize=True because envelope.metrics[].value may legitimately be NaN
  canonical_bytes = canonical_json_blob(env.model_dump(mode='json', exclude_none=False), sanitize=True)
  (golden_dir / '01_complete_canonical.json').write_bytes(canonical_bytes)
  ```
- These TWO files become the cross-lang parity ground truth for Day 5.

**Day 4 — Parameterize remaining fixtures**:
- Generate 5 more envelope fixtures derived from the canonical golden (manual copy + edit, documented as "derived from 01_complete via field override"): `02_minimal_required.json` (only required fields), `03_multi_symbol.json` (symbols_json set), `04_missing_optional.json` (null-valued optionals), `05_legacy_migrator.json` (producer=legacy-migrator with `normalization_*` NULL), `06_pending_upstream.json` (backtest with valid upstream_fingerprint but trainer not-yet-ingested).
- Plus 9 canonical_json test fixtures (NOT envelopes — raw JSON test cases for the canonical serializer): `01_simple.json` through `09_none_vs_null.json` per §3.3.6. Each accompanied by `expected_sha256.json` with pre-computed hashes.
- Plus 7 invalid envelope fixtures (§18.1b) — one per representative §7.6.1 hard-fail rule.

**Day 5 — Rust crate skeleton + parity test**:
- Create `hft-contracts/Cargo.toml` (single `[package]` at repo root per §2.5.1 — NO workspace) + `hft-contracts/rust/lib.rs` + `rust/orchestration/mod.rs` + `rust/orchestration/envelope.rs` (hand-written struct mirroring the Pydantic model).
- Declare dependencies in `Cargo.toml`: `serde = { version = "1", features = ["derive"] }`, `serde_json = "1"`, `sha2 = "0.10"`, `thiserror = "1"`. Set `publish = false`.
- Implement `canonical_json_blob` in Rust per §3.3.6 (must mirror Python's DEFAULT form: whitespace separators via `restore_python_default_separators`, `escape_non_ascii_in_json` post-processor per R14 C2, `sanitize_non_finite` pre-processor for NaN/Inf → null). **Round 16 v1.3.3**: renamed from `canonical_json_dumps` to match Python SSoT name.
- Add `pub const SCHEMA_HASH: &str = "..."` in `rust/orchestration/schema_hash.rs` (separate file so the verifier script can rewrite it without disturbing the struct definition).
- Write `rust/tests/envelope_golden.rs` that deserializes `tests/fixtures/envelopes/golden/01_complete.json`, re-serializes via `canonical_json_blob` (Rust), asserts byte-equal against `golden/01_complete_canonical.json` (produced by Python SSoT).
- Run `cargo test` (NOT `cargo test -p` — no workspace) — passes.
- Run `contracts/verify_rust_envelope_schema.py` — computes TOML hash, writes `SCHEMA_HASH` const via regex-replace in `rust/orchestration/schema_hash.rs`, commits.
- **Week 1 exit gate**: `test_rust_envelope_serialization_matches_python_golden` AND `test_schema_hash_verify_catches_toml_drift` both pass. If parity fails, STOP: defer all Rust work to Phase 11.5 and re-plan Weeks 2-4 without Rust dependencies.

**Week 1 total scope** (Round 14 revised): ~54-76 hours of focused work. At 50h/wk this is ~1.1-1.5 calendar weeks. The original §15.1 "8-10 weeks" budget absorbs a 1.5-week Week-1 without endangering later milestones.

**Round 15 B7 — explicit scope boundary**: Week 1 delivers the **envelope contract plane only** (TOML → Python Pydantic + enums + canonical JSON + Rust struct + parity test). The SQLite DDL (§5.3), migration file `001_initial_v1_0_0.sql`, and ledger runtime (`hft_ops.ledger.*` Python module) are **Week 2 scope**. Week 1 exit gate depends ONLY on envelope+codegen+parity — NOT on SQLite being functional. This boundary prevents scope creep from pulling Week-2 plumbing into Week 1.

**Round 15 B4 (Round 16 v1.3.3 updated) — fixture canonicalization convention**: when producing Day 4 derived fixtures by hand-editing the golden, the workflow is: (a) edit file as desired, (b) run through `Envelope.model_validate_json(text).model_dump(mode='json', exclude_none=False)`, (c) feed to `canonical_json_blob(..., sanitize=True)` (the SSoT from `hft_contracts.canonical_hash`), (d) write THAT output — not the raw hand-edit — as the final fixture. The `expected_sha256.json` hashes are computed from step (d) output via `sha256_hex(canonical_json_blob(...))`. This guarantees every fixture is canonical-form regardless of hand-editing drift.

### §15.2 Phase 12: Sweep v2 + Parallel Execution (2 weeks)

**Deliverables**:
- `hft-ops sweep {start, status, results, abort}` CLI.
- Parallel subprocess pool (ProcessPoolExecutor).
- Early-kill via gate propagation.
- `--skip-existing` via fingerprint dedup.
- Sweep manifest grammar v2 (preserved grid; conditional axes stub).

**Success criteria**:
- 6-axis grid sweep (3 × 2) runs 6 experiments in parallel on 4-worker pool.
- Early-kill aborts remaining axes after 5 consecutive gate failures.
- Re-run with `--skip-existing` completes in < 10% of first-run time.

### §15.3 Phase 13: Knowledge Synthesis + Cohort Reports (2-3 weeks)

**Deliverables**:
- Auto-regenerated markdown with pagination.
- Cohort reports (per model_family, per feature_set, per asset_class).
- Failure taxonomy tagging.
- Regression detection CLI.

### §15.4 Phase 14+ Deferred Items

- Streaming ingestion mode (when Phase 13 streaming arrives).
- Optuna integration (when first HPO sweep justifies).
- Web dashboard (when daily-use warrants).
- Multi-machine ledger sync (when team-scale).
- DuckDB analytical companion (when 10k+ experiments).

---

## §16 — Risks + Open Questions

### §16.1 Known Risks

**R1 — SQLite contention at sweep-scale**. Mitigation: parallel producers write envelopes; single ingester serializes SQL writes; WAL handles concurrent readers.

**R2 — Parquet file explosion**. Mitigation: yyyy_mm partitioning; content-addressable dedup; `hft-ops ledger gc` orphan cleanup.

**R3 — Markdown render divergence**. Mitigation: auto-regenerate post-ingest; CI sanity check that generated markdown is deterministic.

**R4 — Cross-repo contract drift**. Mitigation: `pipeline_contract.toml` codegen + `verify_rust_constants.py`-style hash check between Python Pydantic and Rust struct.

**R5 — Producer MD5 vs. SHA-256 hash mismatch**. Currently trainer uses `md5[:8]` in experiment_id suffix (`result.py:290`); envelope requires SHA-256 64-hex for `experiment_fingerprint`. Mitigation: trainer updated to produce both (SHA-256 for envelope, short hash for human-readable id).

**R6 — 34 retroactive records have sparse data**. Mitigation: v0 envelope handler accepts many-NULL fields; migration is best-effort not byte-exact.

**R7 — Phase 3.5 config migration creates temporary fingerprint instability**. Mitigation: `gentle-brewing-quail.md` already guarantees fingerprint preservation via `resolve_inheritance`; Phase 10 Step 3 gated on Phase 3.5 Batch 1 complete.

**R8 — User changes git state during run**. Mitigation: producer captures git state at emit time; downstream consumers reference that captured state.

### §16.2 Open Questions (to resolve during Phase 11 implementation)

**Q1 — Where does the trainer's MD5 short hash live in the envelope?** Proposal: retain as part of `experiment_id` string format (`{name}_{timestamp}_{md5[:8]}`); `experiment_fingerprint` separately holds SHA-256.

**Q2 — Should OPRA be added to producer enum now or at Phase 12+?** Proposal: add now (forward-compat, no cost).

**Q3 — Should envelope_schema_version use semantic versioning or integer?** Proposal: semver ("1.0.0") — matches pipeline_contract convention.

**Q4 — Should Parquet files be checksummed at ingest?** Proposal: yes, compute sha256 at write time; store in `bulk_parquet_refs.sha256`; verify lazily on read.

**Q5 — What's the exact grep pattern to migrate EXPERIMENT_INDEX.md sections?** Proposal: heuristic + manual review during Phase 11 Step 2; track uncertain mappings in `.migration_state`.

**Q6 — Should `hft-ops ledger query` support `git diff`-style diffs between two snapshots?** Proposal: defer to Phase 13; Phase 11 provides `compare` which is sufficient.

**Q7 — What happens to the existing quail plan Phase 4 (FeatureSet registry)?** Proposal: Phase 10 leaves a placeholder. Phase 4 fills it (reads `feature_schema_ref`, resolves to feature_indices list). No conflict.

---

## §17 — Anti-patterns Explicitly Rejected

### §17.1 Identity Anti-patterns

**AP1 — UUID/auto-increment identity**. Rejected. Content-addressed via `experiment_fingerprint`.
- _Reason_: Re-running same config must dedup, not create a new row.

**AP2 — Config-as-source-code hashing (Sacred)**. Rejected.
- _Reason_: Whitespace/comment changes invalidate hash.

**AP3 — Free-form experiment IDs**. Rejected. Pattern `^[a-zA-Z0-9_\-]{1,200}$` enforced.
- _Reason_: Filesystem compatibility + greppability.

### §17.2 Storage Anti-patterns

**AP4 — Opaque binary stores (Aim)**. Rejected. SQLite (queryable with `sqlite3` shell) + Parquet (queryable with `duckdb`/`pyarrow`).
- _Reason_: Debuggability, migration, export.

**AP5 — Git-ref-per-experiment (DVC `dvc exp`)**. Rejected.
- _Reason_: `git gc` unusable at 10k+ refs.

**AP6 — Single-table JSON blob**. Rejected. Normalized 11-table schema.
- _Reason_: Queries become O(N) JSON parse.

**AP7 — Cloud-hosted primary store (W&B, Neptune)**. Rejected.
- _Reason_: Single-user, no cloud, no vendor lock-in.

**AP8 — Server-required offline mode (ClearML via Docker)**. Rejected.
- _Reason_: Adds maintenance burden.

### §17.3 API Anti-patterns

**AP9 — Custom query DSL (Aim Python predicates)**. Rejected. Raw SQL + pandas.
- _Reason_: Don't reinvent what SQL already does.

**AP10 — Framework DSL (Metaflow `@step`, Kedro nodes)**. Rejected.
- _Reason_: Pipeline already has runners; orchestrator is ledger, not executor.

### §17.4 Schema Anti-patterns

**AP11 — Scalar `horizon: int`**. Rejected. Array `horizons: int[]` even for single-horizon.
- _Reason_: Triple-barrier, multi-horizon regression coming.

**AP12 — Multi-symbol pooled runs stored as one row per symbol**. Rejected. A pooled multi-symbol training run produces ONE model from ONE config — it is one experiment, not N. Stored as a single row with `symbols_json` JSON array. Symbol sweeps (N single-symbol runs) remain N rows, one per fingerprint.
- _Reason_: "One model ↔ one experiment" invariant; sweeps are distinct fingerprints by construction (symbol differs in extraction config).

**AP12b — Silent multi-symbol fingerprint collision**. Rejected. Phase 3 fingerprint includes the extraction config (which declares the symbol[s]); single-symbol runs on different symbols produce different fingerprints. Composite `(fp, symbol, asset_class)` PK was considered (as an N-row safety net) but rejected in Round 10 as unnecessary given the fingerprint's symbol-aware input.

**AP13 — String enums without registry (`"sharpe_ratio"` vs `"SharpeRatio"`)**. Rejected. MetricKey/GateKey enums.
- _Reason_: Typo tolerance at 1000+ experiments.

**AP14 — Config-as-source-code hashing**. See AP2.

### §17.5 Process Anti-patterns

**AP15 — Retroactive backfill as primary source**. Rejected after Phase 11.
- _Reason_: 100% retro today is a failure mode, not a feature.

**AP16 — Manual markdown index editing**. Rejected. Auto-generated views.
- _Reason_: Drift. At 1000 experiments, hand-maintenance fails.

**AP17 — Non-idempotent ingest**. Rejected. Content-addressable + INSERT OR IGNORE.
- _Reason_: Safe retry after `kill -9`.

---

## §18 — Test Coverage Plan

### §18.1 Minimum Test Counts

Phase 11 deliverables:

- **hft-ops**: +40 tests (current 158 → 198+)
- **hft-contracts**: +15 tests (envelope validation, MetricKey, GateKey, canonical JSON) (current 165 → 180+)
- **lob-model-trainer**: +10 tests (envelope emission) (current 176 → 186+)
- **lob-backtester**: +10 tests (envelope emission) (current 338 → 348+)
- **basic-quote-processor**: +5 tests (envelope emission from Rust; canonical JSON parity with Python) (current 471 → 476+)

**Grand total**: +80 tests minimum across the pipeline.

### §18.1b Test Fixture Directory Layout

**Round 14 C8 resolution**: fixtures live under `hft-contracts/tests/fixtures/` (single canonical location); `hft-ops/tests/fixtures/ledger/` consists of symlinks + hft-ops-specific fixtures. Rationale: cross-language parity fixtures (envelopes + canonical_json) must be readable by both Python and Rust; putting them in `hft-contracts/` (where both language suites live) avoids cross-repo path complexity.

**Canonical layout** (shared Python + Rust):

```
hft-contracts/tests/fixtures/
├── envelopes/
│   ├── golden/                          # Round 14 C8 — cross-lang parity anchor
│   │   ├── 01_complete.json             # hand-crafted; every §3.2 field populated
│   │   └── 01_complete_canonical.json   # canonical_json_blob(..., sanitize=True) output of the above (Python SSoT)
│   ├── valid/                           # one per record_type, representative
│   │   ├── v1_bqp_export.json
│   │   ├── v1_mbo_export.json
│   │   ├── v1_trainer_training.json
│   │   ├── v1_backtester_backtest.json
│   │   ├── v1_evaluator_evaluation.json
│   │   └── v1_sweep_aggregate.json
│   ├── invalid/                         # one per failure mode §7.6 can catch
│   │   ├── 01_bad_json.json
│   │   ├── 02_schema_violation.json
│   │   ├── 03_enum_mismatch.json
│   │   ├── 04_cross_field_mismatch.json
│   │   ├── 05_fingerprint_malformed.json
│   │   ├── 06_missing_artifact.json
│   │   └── 07_oversized.json
│   └── future/                          # envelope_version=2, 3, 99 for dispatch testing
│       ├── v2_unknown.json
│       └── v99_unknown.json
└── canonical_json/                      # cross-lang fingerprint fixtures (§3.3.6)
    ├── 01_simple.json
    ├── 02_nested.json
    ├── 03_unicode.json
    ├── 04_float_int.json
    ├── 05_nan_rejected.json
    ├── 06_large_array.json
    ├── 07_datetime_fallback.json        # Round 14 added
    ├── 08_path_fallback.json            # Round 14 added
    ├── 09_none_vs_null.json             # Round 14 added
    └── expected_sha256.json             # hash of each above; checked by Py + Rust
```

**hft-ops-side layout** (ledger-specific, not cross-lang):

```
hft-ops/tests/fixtures/ledger/
├── envelopes/                           # SYMLINK to ../../../hft-contracts/tests/fixtures/envelopes/
│                                        #   (relative symlink survives git clone + moves)
├── canonical_json/                      # SYMLINK to ../../../hft-contracts/tests/fixtures/canonical_json/
├── legacy/                              # 34 legacy ExperimentRecord fixtures (hft-ops-specific)
│   ├── hmhp_128feat_2026_03_13.json
│   ├── tlob_regression_2026_03_15.json
│   └── ...(32 more — one per legacy record)
├── ledgers/                             # pre-canned SQLite DBs for query/rebuild tests
│   ├── empty.sqlite
│   ├── one_experiment.sqlite
│   ├── hundred_experiments.sqlite
│   └── corrupt_missing_indexes.sqlite
├── parquet/                             # side files referenced by envelopes
│   ├── training_curve_500epochs.parquet
│   ├── feature_ic_148.parquet
│   └── equity_curve_233days.parquet
└── golden/                              # migration + render golden outputs (hft-ops-specific)
    ├── migrated_experiments.json
    ├── rendered_experiment_index.md
    └── rendered_backtest_index.md
```

**Fixture conventions:**
- All JSON fixtures are canonical form (§3.3.6) — no trailing newlines, sorted keys, ASCII-escaped.
- `expected_sha256.json` maps fixture path → content hash; test failure if hash drifts (catches accidental fixture edits).
- Parquet fixtures built via `hft-ops/tests/conftest.py::build_fixtures()` at session start (deterministic, reproducible from a seed).
- Symlinks are relative paths (`../../../hft-contracts/tests/fixtures/...`) so they survive `git clone`, sibling-directory moves, and worktree-based development. `git` stores symlinks as content mode 120000; no special handling needed.

**Rust-side consumption**: the Rust crate at `hft-contracts/rust/` consumes `hft-contracts/tests/fixtures/envelopes/golden/*.json` DIRECTLY (no symlinks needed — same repo). Rust `#[test]` functions use `include_str!("../tests/fixtures/envelopes/golden/01_complete.json")` or filesystem-relative `Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/...")`.

**Round 15 A10 — fixture resolution for pip-installed consumers**: the symlink mechanism above assumes the consumer runs tests from a working directory within the monorepo layout (sibling `hft-contracts/` directory accessible). This is true for monorepo-internal dev (`cd hft-ops && pytest`) but BREAKS when a CI job pip-installs `hft-contracts` from the standalone GitHub repo (the installed wheel has no symlink target). Resolution:

1. **`hft-contracts/pyproject.toml` declares `package_data`** so fixtures ship with the wheel:
   ```toml
   [tool.setuptools.package-data]
   "hft_contracts" = ["py.typed", "tests/fixtures/**/*.json", "tests/fixtures/**/*.toml"]
   ```
2. **Fixture resolution API** in `hft_contracts.orchestration.fixtures`:
   ```python
   from importlib.resources import files
   def fixture_path(rel: str) -> Path:
       """Resolve a fixture path, preferring importlib.resources (works for pip-install)
       and falling back to filesystem walk-up for monorepo dev."""
       try:
           return Path(str(files("hft_contracts") / "tests" / "fixtures" / rel))
       except (ModuleNotFoundError, FileNotFoundError):
           # Dev-mode fallback: walk up from caller
           return Path(__file__).parent.parent.parent / "tests" / "fixtures" / rel
   ```
3. **hft-ops test code** uses `fixture_path()` instead of hardcoded symlinked paths. Symlinks remain as dev-only convenience (they survive `git clone` inside the monorepo but are NOT the canonical resolution mechanism).

Tests `test_fixture_resolution_via_importlib` (pip-install context) + `test_fixture_resolution_via_filesystem` (monorepo context) covered under §18.2 new test category.

### §18.2 Test Categories

**Envelope validation tests** (~20):
- Valid v1 envelope accepted
- Schema violations rejected (each of ~30 required fields + type mismatches)
- `envelope_version=0` legacy migration
- `envelope_version=2` (unknown) → quarantine
- `metadata_json` escape hatch tolerates unknown fields
- Content hash stability (same envelope → same hash)

**Ingest pipeline tests** (~15):
- Happy path: single envelope → full ingest
- Idempotent re-ingest (same content_hash)
- Duplicate PK (same `experiment_fingerprint`) → `result=duplicate_pk`, append to `ingest.log`, remove envelope from inbox
- Write order: kill -9 after JSON, after Parquet, after SQLite BEGIN
- `--dry-run` doesn't write
- Concurrent ingest + query (WAL)
- `flock` contention

**Migration tests** (~10):
- Step 1 create-alongside — idempotent
- Step 2 backfill all 34 records — count matches
- Step 2 with malformed record → quarantine + skip
- Step 2 `--resume` from `.migration_state`
- Fingerprint preservation (existing records' fingerprints NOT recomputed)

**Query tests** (~10):
- `list` returns DataFrame
- `query` SQL pass-through
- `search` with filters
- `compare` N-way
- `lineage` upstream + downstream traversal
- Read-only guarantee (query doesn't write)

**Failure recovery tests** (~15):
- Rebuild from JSON records
- Rebuild preserves fingerprints
- Integrity check detects drift
- Quarantine drain
- Parquet regenerate from outputs
- SQLite corruption → rebuild works
- Orphan Parquet cleanup

**Fingerprint tests** (~5):
- Phase 3 reuse: envelope fingerprint == dedup.py output
- `_base:` resolution before hashing
- fingerprint_version bump + multi-version lookup
- Exclusion set correctness (name change doesn't change hash)

**Cross-repo tests** (~5):
- Trainer envelope references BQP export hash
- Backtester envelope references trainer hash
- Lineage walks BQP → trainer → backtester

**Cross-language parity tests** (~8, Round 13 — D2 verification):
- `test_rust_envelope_serialization_matches_python_golden` — load canonical golden envelope in Rust; re-serialize via `hft_contracts_rs::canonical_json::dumps`; assert byte-equal to fixture. **Catches Rust struct drift.**
- `test_python_envelope_serialization_matches_golden` — same test in Python, asserting stability.
- `test_schema_hash_verify_catches_toml_drift` — modify TOML, recompute hash; assert mismatches stored Rust `SCHEMA_HASH`.
- `test_canonical_json_fixtures_01_through_09` — 9 cross-lang fixtures (simple, nested, unicode, float_int, nan_rejected, large_array, datetime_fallback, path_fallback, none_vs_null); Python and Rust both produce identical SHA-256 on each.

**Normalization integrity tests** (~4, Round 13 — D4):
- `test_backtest_stats_hash_match_passes` — trainer envelope `normalization_stats_sha256="abc..."` ingested; backtest with matching hash → succeeds.
- `test_backtest_stats_hash_mismatch_quarantined` — mismatch → `NormalizationStatsMismatchError` → quarantine; `.error` sidecar contains both hashes.
- `test_backtest_upstream_not_ingested_routes_to_pending` — backtest ingested before trainer → `UpstreamNotYetIngestedError` → `pending/{content_hash}.json`.
- `test_pending_retry_succeeds_after_trainer_arrives` — ingest trainer after backtest is pending; next `ingest_all` call retries pending/ → backtest succeeds; `pending/` is empty.

**Pending retry tests** (~3):
- `test_pending_age_out_escalates_to_quarantine` — simulate 24.5h old pending file via `os.utime`; next ingest escalates to quarantine.
- `test_pending_retry_cli_explicit` — `hft-ops ledger pending retry-all` processes all pending envelopes.
- `test_pending_sidecar_preserved` — `.waiting` sidecar lists missing fingerprints; preserved across age-out.

**Inbox resolution tests** (~4, Round 13 — B2):
- `test_cli_arg_precedence_over_env` — `--ledger-inbox=/tmp/x` overrides `HFT_OPS_LEDGER_INBOX=/tmp/y`.
- `test_env_var_used_when_cli_absent`.
- `test_orchestrated_missing_inbox_fails` — `HFT_OPS_ORCHESTRATED=1` + unset `HFT_OPS_LEDGER_INBOX` → `ConfigError` raised.
- `test_standalone_missing_inbox_warns_and_skips` — neither env var set + unset inbox → WARN log + envelope emission skipped.

**Schema migration tests** (~4, Round 13 — B6):
- `test_migration_fresh_db_applies_all` — empty DB + 3 migration files → `schema_migration_num=3` after `apply_pending`.
- `test_migration_partial_db_applies_delta` — `schema_migration_num=2` + 3 on disk → only migration 003 runs.
- `test_migration_syntax_error_rollback` — broken SQL in migration 003 → `schema_migration_num=2` (unchanged) after failure; re-run with fixed SQL succeeds.
- `test_migration_idempotent_replay` — running `apply_pending` twice is a no-op on second call.

**Schema parity test** (~1, Round 13 — B5):
- `test_insert_statement_column_count_matches_schema` — for each `INSERT_*_SQL` runtime-generated from Pydantic fields, assert placeholder count equals target DDL column count AND column names are a subset of DDL columns.

### §18.3 Fault Injection Testing

Critical for reliability. Use `pytest` + `unittest.mock.patch`:

```python
def test_ingest_kill_after_json_record(tmp_path):
    """Simulate kill -9 after JSON write, before Parquet write."""
    with patch('pyarrow.parquet.write_table', side_effect=KeyboardInterrupt):
        with pytest.raises(KeyboardInterrupt):
            ingest_one(envelope_path, tmp_path)
    
    # Record should exist (written first)
    assert (tmp_path / 'records' / f'{exp_id}.json').exists()
    # Parquet should NOT exist (write failed)
    assert not list((tmp_path / 'metrics').glob('**/*.parquet'))
    
    # Rebuild recovers
    rebuild(tmp_path)
    assert Ledger(tmp_path).has_experiment(exp_id)
```

Similar tests for kill-after-Parquet, kill-after-SQL-BEGIN, etc.

**Round 13 — explicit failure-mode-to-test-name mapping** (covers §12.1 completeness metric #8 "10 of 10 failure modes have tests"):

| §12.1 ID | Failure mode | Test name (in `tests/test_fault_injection.py`) |
|---|---|---|
| FM-01 | Producer writes envelope, ingester crashes before SQLite commit | `test_fm_01_kill_after_json_record_rebuild_recovers` |
| FM-02 | SQLite commits, markdown render crashes | `test_fm_02_render_crash_does_not_fail_ingest` |
| FM-03 | Producer writes inbox, Parquet missing from disk | `test_fm_03_parquet_missing_warn_only_proceeds` |
| FM-04 | Two concurrent ingesters (flock contention) | `test_fm_04_concurrent_ingest_serialized_by_flock` |
| FM-05 | Out-of-order ingest (backtest before trainer) | `test_fm_05_out_of_order_ingest_pending_then_resolves` |
| FM-06 | Normalization-stats mismatch (train vs inference drift) | `test_fm_06_norm_stats_mismatch_quarantined` |
| FM-07 | Schema migration syntax error mid-apply | `test_fm_07_migration_rollback_preserves_version` |
| FM-08 | Inbox envelope with truncated JSON | `test_fm_08_malformed_json_quarantined` |
| FM-09 | Duplicate `experiment_fingerprint` (re-emission race) | `test_fm_09_duplicate_pk_idempotent_audit` |
| FM-10 | `records/{id}.json` exists but SQLite row missing (state A) | `test_fm_10_orphaned_record_heal_recovers` |

**Meta-test** `test_all_failure_modes_have_tests`:
```python
def test_all_failure_modes_have_tests():
    """Enforce §12.1 coverage: every failure mode must have a named test."""
    import inspect, tests.test_fault_injection as tfi
    test_names = {name for name, _ in inspect.getmembers(tfi, inspect.isfunction) if name.startswith("test_fm_")}
    expected = {f"test_fm_{nn:02d}_" for nn in range(1, 11)}
    for prefix in expected:
        matches = [n for n in test_names if n.startswith(prefix)]
        assert matches, f"Missing test for failure mode starting with {prefix}"
```

### §18.4 Performance Tests

- Ingest 1000 envelopes in < 60 seconds.
- Query "all TLOB with IC > 0.05" in < 100 ms on 1000-experiment ledger.
- Markdown render for 1000 experiments in < 30 seconds.

### §18.5 Migration Validation

One-off validation for Phase 11 Step 2:
- All 34 legacy records produce valid v0 envelopes.
- Round-trip: dump v0 envelope → re-ingest → identical state.
- `EXPERIMENT_INDEX.md` auto-generated covers all 34 sections.

---

## §19 — References

### §19.1 Internal Plans + Memory

- `/Users/knight/.claude/plans/twinkling-snacking-spark.md` — BQP Phase 9 plan (shipped).
- `/Users/knight/.claude/plans/gentle-brewing-quail.md` — hft-ops Phase 3 Config Composition (partially shipped 2026-04-15, Phase 3.5 batch migration pending).
- `/Users/knight/.claude/plans/generic-juggling-floyd.md` — Feature Evaluator Redesign (Phase 0 applied).
- `/Users/knight/code_local/HFT-pipeline-v2/plan/EXPERIMENTATION_FIRST_ARCHITECTURE.md` — T-series (T9-T15) complete.
- `/Users/knight/code_local/HFT-pipeline-v2/plan/UNIFIED_PIPELINE_ARCHITECTURE_PLAN.md` — 10-phase pipeline roadmap.

### §19.2 Code Citations (current state)

- `hft-ops/src/hft_ops/ledger/ledger.py:53-62` — current JSON-index rebuild.
- `hft-ops/src/hft_ops/ledger/experiment_record.py:24-54,57-134` — current record shape + record_type enum.
- `hft-ops/src/hft_ops/ledger/dedup.py:117-195,284-391` — fingerprint algorithm (§3.3b fix); `lob-model-trainer/src/lobtrainer/config/merge.py:85-182` — `resolve_inheritance()` for `_base:` chains (lazy-loaded by dedup.py:73-114).
- `hft-ops/src/hft_ops/manifest/sweep.py:44-121,157` — current sweep grid expansion + validation.
- `hft-ops/src/hft_ops/manifest/schema.py:309-339` — SweepConfig dataclass.
- `hft-ops/src/hft_ops/stages/base.py:138` — HFT_OPS_ORCHESTRATED env var.
- `hft-ops/src/hft_ops/stages/validation.py:1-32,157-160` — ValidationRunner (fast_gate library import, warn-only default).
- `hft-ops/src/hft_ops/stages/backtesting.py:151-156` — BacktestRunner (validate_outputs returns `[]`; bug noted in §12).
- `hft-ops/src/hft_ops/provenance/lineage.py:56,196-245` — git sentinel, Provenance dataclass.
- `hft-contracts/src/hft_contracts/validation.py:338-414` — validate_off_exchange_export_contract (does NOT yet validate forward_prices; Phase 9 deferred).
- `hft-contracts/src/hft_contracts/label_factory.py:100-108,152-174` — ForwardPriceContract (Phase 9 unblock).
- `lob-model-trainer/src/lobtrainer/config/merge.py` — Phase 3 hand-rolled multi-base (v2).
- `lob-model-trainer/src/lobtrainer/experiments/registry.py:100-141` — current trainer registry.
- `lob-model-trainer/src/lobtrainer/experiments/result.py:33-170,287-292` — ExperimentResult + MD5[:8] short hash.
- `lob-backtester/src/lobbacktest/registry.py:72-123` — current backtester registry.
- `basic-quote-processor/src/export/metadata.rs:19-105` — Phase 9 ExportMetadata struct.
- `basic-quote-processor/src/export/manifest.rs:20-43` — Phase 9 DatasetManifest struct.

### §19.3 Prior Art (external)

Reviewed in Round 9 V2 agent:
- **MLflow**: SQLite backend; `search_runs(filter_string)` borrowed; UUID-as-ID rejected.
- **Weights & Biases**: sweep YAML grammar borrowed; cloud-lock-in rejected.
- **Sacred + Omniboard**: named configs borrowed; MongoDB rejected.
- **DVC**: content-addressed stage cache borrowed directly; `dvc exp show` table borrowed.
- **Kedro**: skipped (DAG DSL unnecessary).
- **Metaflow**: resume-at-step partial borrow; `@step` decorator rejected.
- **Hydra + Optuna**: config composition pattern informative; Optuna SQLite study opt-in for Phase 12.x.
- **ClearML**: auto-capture rejected (explicit > implicit for HFT audit).
- **Aim**: Python-predicate query partial inspiration; custom binary storage rejected.
- **Neptune.ai**: skipped (cloud-locked).

### §19.4 Agent Reports (Round 9)

- V1 Producer envelope stress-test (agent `a499031e89427c0d2`).
- V2 2-year future-proofing audit (agent `a6f81248d9764c0af`).
- V3 Failure-mode + reliability audit (agent `a5bc46b25a8c86fc5`).
- V4 Phase 3 alignment + cross-module (agent `a04a4396815db751f`).

Plus prior rounds' agents (Rounds 1-8) for Phase 9 context.

---

## Appendix A — Glossary

| Term | Definition |
|---|---|
| **envelope** | Canonical JSON record a producer writes to share an experiment with the ledger. |
| **experiment_fingerprint** | Phase 3's SHA-256 hash of canonicalized config components; content-addressed experiment identity. |
| **cohort_hash** | Derived hash excluding symbol and data_manifest; enables cross-symbol cohort queries. |
| **record_type** | Enum: export, training, analysis, calibration, backtest, evaluation, sweep_aggregate. |
| **producer** | Any pipeline component that emits envelopes (BQP, MBO extractor, trainer, backtester, evaluator, profilers). |
| **consumer** | Any tool that reads from the ledger (CLI, notebooks, auto-render). |
| **inbox** | `hft-ops/ledger/inbox/` — producer-to-ingester queue. |
| **quarantine** | `hft-ops/ledger/quarantine/` — rejected envelopes awaiting review. |
| **Ledger v2** | This design. Supersedes current `hft-ops/ledger/index.json` + per-experiment JSON. |
| **γ-full scope** | Phase 10 scope choice D4: envelope + MetricKey + SQLite + Parquet + Sweep v2 + failure recovery + lineage. |
| **bulk_parquet** | Side files for columnar metric arrays (training curves, feature IC, etc.). |
| **MetricKey** | Enum of well-known metric names (IC_H10, SHARPE_RATIO, etc.); registered in contract §4.2. |
| **GateKey** | Enum of well-known gate names (`ic_gt_0_05`, `baseline_ridge`, etc.); registered in contract §4.5. Names use underscore form (TOML-bare-key-safe). |
| **fingerprint PK** | `experiment_fingerprint` — single-column primary key of `experiments` table (Round 10). The fingerprint embeds symbol and contract_version via its component serialization (Phase 3 `dedup.py`), so fingerprint-alone suffices for uniqueness. |
| **cohort_hash** | SHA-256 derived hash that EXCLUDES symbol + data_manifest; stored on `experiments`. Enables cross-symbol cohort queries (`WHERE cohort_hash = ?`). |
| **symbols_json** | JSON-array column on `experiments`. NULL for single-symbol experiments; non-NULL for multi-symbol pooled training. |
| **ingest.log** | Append-only JSONL audit file at `hft-ops/ledger/ingest.log` (replaces the previously-proposed `ingest_audit` SQLite table — Round 10). Rotated monthly. Queryable via `hft-ops ledger audit-log`. |
| **`pending/`** | `hft-ops/ledger/pending/` — directory for envelopes whose upstream has not yet been ingested (§14.8 `UpstreamNotYetIngestedError`). Scanned first by every `ingest_all` call; age-out to quarantine at 24h. Round 13 D4 addition. |
| **`UpstreamNotYetIngestedError`** | Soft ingest failure signaling the upstream producer's envelope hasn't arrived yet. Routes to `pending/`, retried on subsequent ingest calls, escalates to quarantine after 24h. NOT to be conflated with `IntegrityError` (hard failure) or `NormalizationStatsMismatchError` (real drift, immediate quarantine). |
| **`SCHEMA_HASH`** | `pub const` in `hft_contracts_rs::orchestration::envelope` that equals `sha256(canonical_json(envelope_schema_from_toml))`. CI-verified by `contracts/verify_rust_envelope_schema.py`. Catches TOML-side drift. Round 13 D2 addition. |
| **Golden envelope fixture** | `tests/fixtures/envelopes/golden/01_complete.json` + its canonical form. Rust golden-serialization test asserts Rust `Envelope` struct re-serialization is byte-equal. Catches Rust-side struct drift (complements SCHEMA_HASH which catches TOML-side drift). |
| **Canonical form of envelope** | `canonical_json_blob(Envelope.model_validate_json(raw).model_dump(mode='json', exclude_none=False), sanitize=True)` — Pydantic roundtrip through the monorepo SSoT (`hft_contracts.canonical_hash.canonical_json_blob`; frozen contract: `json.dumps(obj, sort_keys=True, default=str).encode("utf-8")`). Used for `records/{id}.json` byte-storage AND for the idempotent byte-compare at ingest. Distinct from the raw inbox bytes which may have non-canonical whitespace/key-order variation. Round 16 v1.3.3 correction: was `canonical_json_dumps` with compact separators — replaced to reuse SSoT per hft-rules §0 "Reuse-first". |
| **HFT_OPS_LEDGER_INBOX** | Env var set by the orchestrator to the absolute path of `hft-ops/ledger/inbox/`. Producers read this to know where to write envelopes. Missing under `HFT_OPS_ORCHESTRATED=1` → fail-fast; missing in standalone mode → warn-skip. Round 13 B2 addition. |

---

## Appendix B — Phase 10 Success Metrics (measurable)

| # | Metric | Target |
|---|---|---|
| 1 | Production ledger records (non-retroactive) | ≥ 1 at Phase 11 Step 3 completion |
| 2 | Migrated legacy records | 34 of 34 (100%) |
| 3 | Fingerprints preserved through migration | 34 of 34 (100%) |
| 4 | Envelope schema violations in CI | 0 |
| 5 | Query latency on 1k-record ledger | < 100 ms p95 |
| 6 | Markdown render latency (post-ingest hook) | < 2 s |
| 7 | New hft-ops tests | ≥ 40 |
| 8 | Failure modes §12.1 covered by tests | 10 of 10 (100%) |
| 9 | Cross-repo lineage traversal | works for BQP → trainer → backtester |
| 10 | Envelope producers | 5 (BQP, MBO, trainer, backtester, evaluator) |

---

## Appendix C — Change Log

| Version | Date | Change |
|---|---|---|
| Draft v1 | 2026-04-15 | Initial draft synthesizing 9 validation rounds (~20 agent reports). |
| Draft v1.1 — Round 10 | 2026-04-15 | Applied Round 10 validation findings (4 parallel agents: consistency / completeness / correctness / better-alternatives). See amended change list below. |
| Draft v1.2 — Round 11 | 2026-04-15 | Applied Round 11 Tier-1 findings (4 parallel agents: radical alternatives / data-flow integrity / 5-year decay / code-reality alignment). All 7 Tier-1 items fixed. See Round 11 Tier-1 Amendments below. |
| Draft v1.3 — Round 12/13 | 2026-04-15 | Applied Round 12 pre-implementation validation (3 agents: sequencing, cross-phase integration, implementation-surface) + Round 13 self-stress-test refinements. Added 8 blockers (B1-B8) + 4 decision amendments (D1-D4). See Round 12/13 Amendments below. |
| Draft v1.3.1 — Round 14 | 2026-04-15 | Applied Round 14 validation (3 agents: v1.3 adversarial audit, Week-1 implementer dry-run, 3-month production-pilot stress-test). 9 critical fixes C1-C9 applied; 6 operational gaps documented as Phase 11 W2-3 additions. See Round 14 Amendments below. |
| Draft v1.3.2 — Round 15 | 2026-04-15 | Applied Round 15 validation (2 agents: post-v1.3.1 amendment stress-test, Day-1 GO/NO-GO gate + self-check). 13 micro-fixes: A2 rebuild Pydantic-roundtrip parity; A5 TOML `'''` escape guard; A8 local-dev workflow UX; A9 hft-ops transitive dep; A10 `importlib.resources` fallback for pip-install CI; A11 `sweep fingerprints --verify` CLI added to deliverables; B4 fixture canonicalization workflow; B5 `CanonicalError` enum defined in §3.3.6; B7 Week 1 scope boundary (SQLite is Week 2); B9 Pydantic sub-model naming convention; self-check `SCHEMA_INFO_VERSION` + `_utc_now_iso_z` + `IngestOneResult` class. **Day 1 coding GREEN-LIT.** |
| Draft v1.3.3 — Round 16 | 2026-04-16 | **Critical SSoT correction after Day 1 verification**: every prior round specified a NEW `canonical_json_dumps` with compact separators + `allow_nan=False` for Phase 10. Day 1 verification against root CLAUDE.md + PIPELINE_ARCHITECTURE.md §2988/3024 revealed this violates the **frozen monorepo canonical form** at `hft_contracts/canonical_hash.py` (Phase 4 Batch 4c hardening, 2026-04-15). The SSoT uses DEFAULT separators (`", "` and `": "` — WITH spaces), DEFAULT `ensure_ascii=True`, default `allow_nan=True` with `sanitize=True` option. Per the frozen contract: "Compact-separator variants would produce different bytes and break existing fingerprints." Per PA §3024: "When adding a new hash/canonical-form primitive, always place it in `hft-contracts` first, then import; never re-implement." **Fix**: Phase 10 REUSES `hft_contracts.canonical_hash.canonical_json_blob` — no new Python module. §3.3.6 rewritten to delegate. Rust side mirrors Python's DEFAULT form (whitespace separators + ASCII escape + NaN→null sanitize). §15.1a Day 2 deliverable "hand-write canonical_json.py" DELETED. All call sites updated (§3.5, §7.3, §7.3.2 helpers, §14.8, §15.1a Day 3, §18.1b fixture description, Appendix A glossary). **Day 1 output (pipeline_contract.toml) unaffected** — it contains only schema data, no canonical-form references. Day 2 coding unblocked with corrected spec. |
| Draft v1.3.4 — Round 17 | 2026-04-16 | Applied Round 17 Phase 4 × Phase 10 integration audit (one agent, focused on non-canonical_hash SSoT primitives). 6 gaps fixed (0 blockers). **I10 decision**: evaluator `SelectionCriteria` (Phase 4 Batch 4a) reuses `gates[]` with new `selection_min_ic` / `selection_min_abs_ic` / `selection_require_holdout_confirmed` keys added to `[orchestration.gate_keys]` registry — one concept, no new envelope field. **I4**: new §7.6.1 rule 12 — ingester invokes `hft_contracts.validation.validate_any_export_contract` for `record_type='export'` envelopes (WARN-only v1, HARD-FAIL v2). **I2+I3**: new §13.1c Phase 4 Batch 4c.3/4c.4 coordination — flags the Batch 4c.3 fingerprint cutover for live feature_set-using records (triggers §6.5 algorithm version bump) and pre-plans §13.1b row #26 (`feature_set_ref`) for Batch 4c.4. **I9**: DOCUMENTATION_INDEX.md:277 refresh from "v1.1 Round-10-refined, 3,616 lines" → current. **I8**: PA §14.2 + §14 diagram Phase 10 entry flagged for Phase 11 coding PR. Day 1 pipeline_contract.toml extended with 3 new selection gate keys. |

### Round 10 Amendments (v1 → v1.1)

**Must-fix (technical defects found by Agent C — verified by SQLite `executescript()` and `tomllib.loads()`):**

1. **§5.3 DDL syntax** — rewrote `CREATE TABLE experiments` (and `tags`): all column definitions now precede the table-level `PRIMARY KEY` constraint (SQLite requires this ordering — previous form failed to parse). Confirmed by ad-hoc `sqlite3.OperationalError: near "c": syntax error` reproducer in validation pass.
2. **§5.3 missing `cohort_hash` column** — added `cohort_hash TEXT` on `experiments` (previously referenced in UPDATE/SELECT but never declared).
3. **§4.2 TOML `null` in arrays** — replaced `range = [null, 1.0]` / `range = [-1.0, null]` / `range = [0, null]` forms with separate `upper_bound` / `lower_bound` scalars (TOML has no null literal). Added explicit "range semantics" paragraph.
4. **§3.2 / §4.5 GateKey spelling unified** — all gate references use the underscore form (`ic_gt_0_05`, `label_exec_alignment_gt_0_5`, `sign_flip_rate_lt_0_5`) which is TOML-bare-key compatible. Updated JSON Schema example, §3.3.2 envelope example, glossary.
5. **§3.2 `source_commit` regex** — changed from `^[a-f0-9]{7,40}$` to `^([a-f0-9]{7,40}|not_git_tracked)$` so the documented sentinel actually passes validation.
6. **§6.4 SQL trailing comma** — rewrote §6.4 SQL block (also dropped composite PK references); no trailing commas remain.
7. **§5.3 `CURRENT_TIMESTAMP` format** — replaced with `strftime('%Y-%m-%dT%H:%M:%SZ', 'now')` to emit ISO 8601 UTC (matching every other timestamp column in the schema).
8. **§5.3 `symbol_cohort` FK contradiction** — resolved by dropping `symbol_cohort` table entirely (refinement R1/R2, below).

**Should-refine (architectural improvements found by Agent D):**

9. **R1 — Primary key simplified** — composite `(experiment_fingerprint, symbol, asset_class)` PK dropped; `experiment_fingerprint` alone is the PK. Phase 3 fingerprint (`dedup.py:284-391`) already embeds symbol via the extraction config. Updated §0 D3, §5.3 DDL, §6.4 strategy, §6.6 example, §7.3 ingest algorithm, §8.4 query examples, §17.4 AP12 + new AP12b, Appendix A glossary.
10. **R2 — Table count 12 → 10** — dropped `symbol_cohort` (replaced by `symbols_json` + `cohort_hash` columns on `experiments`) and `ingest_audit` (replaced by append-only `hft-ops/ledger/ingest.log` JSONL file). Simpler child-table FKs (all fingerprint-only); faster writes (one less UNIQUE index); easier rebuild semantics.
11. **R3 — Markdown render debounced** — new §4.6 `Markdown Render Throttle` specifies trailing-edge debounce (5 s default, 60 s absolute ceiling) replacing eager post-ingest render. Sweeps no longer trigger O(N²) render work. §7.5 updated to reference §4.6.
12. **R4 — Phase 11 timeline 3 → 4 weeks** — §15.1 updated to absorb cross-language envelope codegen, SQLite lock-contention hardening, and 4 parallel producer rollouts.

**Completeness gaps filled (top 7 from Agent B — 20 total flagged):**

13. **§3.3.5 MBO extractor envelope template** — new complete JSON example for `feature-extractor-MBO-LOB` (record_type='export'), mirroring the BQP template at §3.3.1.
14. **§3.3.6 Canonical JSON serialization spec** — new section with paired Python + Rust implementations + 7 invariants + cross-lang test fixture specification; ensures fingerprint byte-parity across producer languages.
15. **§7.3.1 SQL INSERT templates** — normative column-ordered INSERT statements for every table; binds via `?` placeholders; includes `upsert_sweep` + `insert_fingerprint_history` helpers.
16. **§7.4.1 Rollback protocol** — recovery matrix for 4 partial-commit states (A/B/C/D); CLI helpers (`check --orphaned-records`, `heal --from-orphans`, `rebuild --from-records --backup`); explicit invariant that `records/*.json` is the source of truth.
17. **§13.1b Legacy `ExperimentRecord` → envelope-v0 field mapping** — full mapping table (24 legacy fields) + 7 v1 fields defaulted to NULL + migration test checklist (fingerprint preservation bit-identical).
18. **§18.1b Test fixture directory layout** — explicit `hft-ops/tests/fixtures/ledger/` layout (envelopes/valid, invalid, legacy, future; ledgers; parquet; canonical_json; golden).
19. **Appendix A glossary updated** — added/revised `fingerprint PK`, `cohort_hash`, `symbols_json`, `ingest.log`; removed stale `composite PK` entry.

**Validation evidence (embedded in doc's own tests for Phase 11):**

- §5.3 DDL: executes cleanly in SQLite in-memory; CASCADE delete verified; `cohort_hash` UPDATE verified; multi-symbol `symbols_json LIKE` query verified; CHECK constraints fire as expected.
- §4.2 TOML: parses via `tomllib.loads()`; 44 registered metric keys; no null-in-range remaining; half-bounded form correct.
- §4.5 TOML: all GateKeys use underscore form (no dot).
- §3.2 JSON Schema: parses as Draft-07 JSON; source_commit pattern accepts both 7-char and 40-char SHAs AND the `not_git_tracked` sentinel.
- §6.4 SQL blocks: no trailing commas; reference only valid columns.

### Round 12/13 Amendments (v1.2 → v1.3)

**Round 12 agents (sequencing / cross-phase / implementation-surface) + Round 13 self-stress-test** produced 8 blockers and 4 decision amendments. All applied before commit.

**8 blockers resolved:**

1. **B1 Z-timezone pattern** — §3.2 `created_at`/`finalized_at`/`heartbeat_at`/`override_at` now carry `pattern: "^\\d{4}-\\d{2}-\\d{2}T\\d{2}:\\d{2}:\\d{2}(\\.\\d+)?Z$"` (Z-only, rejects `+00:00` offset form). Prevents Parquet `yyyy_mm` partition drift at midnight-UTC boundaries and markdown-render timezone ambiguity. Promoted from Tier-2 to Phase 11 Week 1 (schema-breaking if deferred).

2. **B2 Inbox path resolution** — new §7.1a "Inbox Path Resolution" with `HFT_OPS_LEDGER_INBOX` env var + `--ledger-inbox` CLI arg + fail-fast when orchestrated / warn-skip when standalone. Producers cloned into standalone repos (BQP, future OPRA) need uniform path discovery regardless of their working directory.

3. **B3 Phase 3.5 gate widening** — §13.3 updated to require `hft-ops sweep fingerprints --verify` clean at every Phase 11 producer-rollout milestone, not just Batch 1. Mid-Phase-11 Phase 3.5 migrations (Batches 2-4) could drift fingerprint inputs otherwise.

4. **B4 Pydantic-canonical byte-compare** — §7.3 idempotent check now compares `canonical_json_dumps(validated.model_dump())` on BOTH sides, not raw inbox bytes. Raw inbox may have non-canonical whitespace/key-order; Pydantic roundtrip canonicalizes. The `records/` file is the CANONICAL form, never the raw inbox form.

5. **B5 Runtime-generated INSERT** — §7.3.1 now specifies INSERTs are built at runtime from Pydantic field list (positional `?` matches field order); meta-test `test_insert_statement_column_count_matches_schema` guards against column drift across DDL changes.

6. **B6 Schema migration protocol** — new §5.7 "Schema Migration Protocol" with numbered `migrations/NNN_*.sql` files + `hft-ops ledger migrate-schema` CLI + rollback on mid-migration failure. Replaces the implicit `CREATE TABLE` approach which would fail on any v1.0 → v1.1 version bump with existing ledger on disk.

7. **B7 Week 1 bootstrap order** — new §15.1a "Phase 11 Week 1 Bootstrap Order" with strict Day 1-5 sequencing to resolve the chicken-and-egg between Pydantic codegen, JSON Schema, and test fixtures. Day 1 TOML freeze; Day 2 Python codegen; Day 3 hand-crafted golden envelope; Day 4 fixture parameterization; Day 5 Rust skeleton + parity test (end-of-W1 gate).

8. **B8 `UpstreamNotYetIngestedError` vs quarantine** — §14.8 refactored to distinguish two failure modes: soft `UpstreamNotYetIngestedError` (routes to `pending/`, retried) vs hard `NormalizationStatsMismatchError` (immediate quarantine). Prior v1.2 collapsed both into `IntegrityError` → all quarantine → false alarms during sweeps with legitimate ordering races.

**4 decision amendments:**

- **D1 Timeline honest** — §15.1 rescoped from "4 weeks" (Round 10 bump) to "8-10 weeks with weekly go/no-go gates." Agent P1 mid-estimate for Python-producers-only scope is ~355 focused hours ≈ 8-9 weeks at 40h/wk. State the range honestly to avoid schedule pressure cutting Tier-1 invariants. Rust producer wiring (BQP, MBO) explicitly deferred to Phase 11.5; Rust parity validation (crate skeleton + golden test) stays IN Phase 11 Week 1 to de-risk the biggest unknown.

- **D2 Rust crate simplified layout** — §2.5.1 codified: single-package Rust crate at repo root (`[package]` + `[lib] path = "rust/lib.rs"`), matching `hft-statistics` precedent (verified by reading `hft-statistics/Cargo.toml`). NO Cargo workspace. BOTH `SCHEMA_HASH` (TOML-side drift) AND golden serialization test (Rust-side struct drift) required — each catches a different failure direction; one alone is insufficient.

- **D3 `metadata_json` IN content hash** — §3.5 flipped: `content_hash = sha256(canonical_json_dumps(envelope))` covers FULL envelope including `metadata_json`. Prevents silent producer-race data loss (previously, two envelopes differing only in metadata_json shared the same filename and `os.replace` overwrote the first). Identity semantics unchanged (PK remains `experiment_fingerprint`); `content_hash` only governs inbox filenames.

- **D4 `pending/` retry on every ingest call** — §7.3 `ingest_all` now scans `pending/` BEFORE `inbox/` on every invocation (option α, no background daemon). Age-out to quarantine at 24h. Orchestrator-between-stages-ingest for deterministic ordering is Phase 12, NOT Phase 11 — Phase 11 relies solely on `pending/` for out-of-order handling.

**Scope descope for Phase 11 (D1)**:
- Python producers only: trainer (H21) + backtester (H22). Both required to exercise §14.8 end-to-end.
- Rust crate skeleton + parity tests: IN scope (de-risks biggest unknown).
- Rust producer wiring (H23 BQP, H24 MBO, evaluator): DEFERRED to Phase 11.5 (≈ 2 weeks additional).
- Tier-2 B1 promoted to W1; B8, C-FF6, C-FF7, C-SC8 stay in Phase 11.5.

**New tests added to §18.2**:
- Cross-language parity tests (8)
- Normalization integrity tests (4)
- Pending retry tests (3)
- Inbox resolution tests (4)
- Schema migration tests (4)
- Schema parity test (1)
- Total new: ~24 tests on top of the prior §18.2 categories.

**§18.3 failure-mode-to-test-name mapping** — explicit `test_fm_01_*` through `test_fm_10_*` names for every §12.1 failure mode + meta-test enforcing coverage.

**Validation evidence** (embedded for Phase 11 implementation):
- §5.3 DDL re-parses cleanly post-amendments.
- §4.2 + §4.5 TOML re-parse cleanly.
- §3.2 envelope JSON Schema adds `pattern` on 4 timestamp fields; Draft-07 parser accepts.
- `pipeline_contract_version` pattern still rejects legacy `off_exchange_1.0`.
- All 5 producer envelope examples still parse.
- Legacy `ExperimentRecord` field mapping (§13.1b) still enumerates 25 fields.

### Round 11 Tier-1 Amendments (v1.1 → v1.2)

**Agent verdicts:** Agent A (radical alternatives) → CURRENT DESIGN OPTIMAL with one worth-exploring (A6 auto-render vs hand-narrative split — deferred). Agent B (data-flow integrity) → 7 silent-corruption risks, 3 CRITICAL/HIGH addressed in Tier-1. Agent C (5-year decay) → healthy through 10k experiments; top drift risks deferred to Phase 11+. Agent D (reality check vs code) → 9 stale refs + 1 CRITICAL factual error; all fixed.

**Tier-1 must-fix (blocking Phase 11 code):**

**T1 — §13.1b legacy field mapping rewritten from ground truth.** The v1.1 mapping table invented 18 field names that do not exist on `ExperimentRecord` (e.g., `config_files`, `outputs`, `metrics` as top-level dict, `upstream_experiment_ids`, `gates`, `git_state`, `data_paths`, `errors`) and omitted 9 fields that do exist (`manifest_path`, `extraction_config`, `training_config`, `backtest_params`, `dataset_health`, `hypothesis`, `stages_completed`, `sweep_id`, `axis_values`). Rewrote from direct read of `experiment_record.py:88-132` (25 dataclass fields) + `lineage.py:195-245` (Provenance sub-structure, 7 fields). Added per-field transformation rules, legacy-migrator exemption from §14.8/§14.9 integrity gates, and a 9-step migration test suite. Added `"legacy-migrator"` to `producer` enum.

**T2 — Normalization state threading added to envelope.** Under T15 "Raw Rust, Variable Python", normalization is the single largest source of silent train/inference divergence. Added:
- `training_info.normalization_method` (enum: none / hybrid / global_zscore / per_feature_minmax / market_structure_zscore)
- `training_info.normalization_stats_sha256` (64-hex)
- `training_info.normalization_source_split` (train / all / cv_folds)
- Same three fields on `signal_provenance` + new `trainer_fingerprint` (64-hex, resolves upstream) + `signal_file_sha256`
- New §14.8 "Normalization-Stats Integrity" invariant with enforcement pseudocode: backtester's `signal_provenance.normalization_stats_sha256` MUST byte-equal the upstream trainer's `training_info.normalization_stats_sha256`; ingest REJECTS on mismatch.

**T3 — Upstream fingerprint threading.** v1.1 had no concrete API for retrieving the upstream trainer's `experiment_fingerprint` at backtester emit time. Added:
- New §7.1.1 "Upstream Fingerprint Resolution API" with `.envelope-ref` breadcrumb-file protocol (Python + Rust implementations spec'd).
- New §6.4a "Fingerprint Composition per record_type" table: backtest/training/evaluation/calibration fingerprints MUST include `sorted(upstream_fingerprints)` as a component. Prevents fingerprint collision across distinct upstream trainers with identical backtest params.
- New §14.9 "Upstream Fingerprint Integrity" invariant: non-raw-data `lineage[].source_hash` MUST resolve to `experiments.experiment_fingerprint`.
- §7.1 Producer Responsibilities updated: producers MUST call `resolve_upstream_ref()` and populate typed fingerprint fields (not free-form `source_name`) when the upstream is ledger-resident.

**T4 — `n_features` / `feature_layout` / `feature_indices_subset` added to formal §3.2 JSON Schema.** Previously these fields appeared in the §3.3.5 MBO envelope example but were NOT in the schema — a producer emitting `n_features: 42` accidentally would validate silently and fail only at trainer load time with a cryptic shape-mismatch error. Added to `export_stats` properties with:
- `n_features` (integer, ≥1, must be ≤ registered `total_count` in `feature_schema_ref`)
- `feature_layout` (enum: "grouped" | "lobster")
- `feature_indices_subset` (optional sorted integer array; NULL = all features)
- New §7.6.1 hard-fail cross-field validations (11 rules) + §7.6.2 warn-only rules (6 rules). n_features subset check is rule #7. Related rules also cover symbol consistency, normalization-hash/method pairing, sub_records gating.

**T5 — `pipeline_contract_version` invented value fixed.** v1.1 §3.3.1 BQP example claimed `pipeline_contract_version = "off_exchange_1.0"` but that string does not exist in `pipeline_contract.toml` — the actual value is `schema_version = "1.0"` under `[features.off_exchange]` (feature-specific) vs top-level `[contract].schema_version = "2.2"` (pipeline-wide). Fixed to `"2.2"` in the BQP example and tightened the JSON Schema with pattern `^\\d+\\.\\d+(\\.\\d+)?$` that rejects the legacy string. `feature_schema_ref` remains the per-producer schema slot.

**T6 — File:line citations corrected.** v1.1 repeatedly cited:
- `dedup.py:174-281` → should be `dedup.py:284-391` (compute_fingerprint)
- `dedup.py:65-109` → should be `lob-model-trainer/src/lobtrainer/config/merge.py:85-182` (resolve_inheritance lives in a different file entirely)
- `dedup.py:253-257` → should be `dedup.py:255-282` (_extract_fingerprint_fields) + `dedup.py:364-368` (validation-exclusion comment)
- `dedup.py:94-100` → should be `dedup.py:95-99` (return-None path inside `_load_trainer_merge_module`)
Fixed throughout §1.1, §3.3, §6, §14, §19.

**T7 — Stale stats corrected.** "`ExperimentRecord` dataclass ... with 20 fields" → 25 fields (enumerated). "4 of 17 orthogonal bases created" → 21 bases across 4 categories (8 datasets, 4 labels, 5 models, 4 train).

**Validation**: all 7 Tier-1 fixes verified by a 31-check Python self-audit (DDL re-parse + TOML re-parse + JSON Schema re-parse + 5 producer envelope re-parse + field presence checks + invented-claim absence checks). Round 10 regressions confirmed green. See `hft-architect` agent reports + `doc-alignment-auditor` report in session 0b055834.

**Tier-2 items deferred to Phase 11 Week 1:**
- (B1) Z-timezone pattern enforcement on all ISO 8601 fields
- (B6) Parquet-before-envelope ordering + §7.4.1 Parquet state in recovery matrix
- (B8) First-class `exchange` field on envelope
- (C-FF6) JSON Schema codegen from TOML + CI drift-check
- (C-SC8) Cross-repo contract-bump coordination playbook
- (C-FF7) `metadata_json.<producer>.*` namespace convention

**Tier-3 items deferred (not blocking):** A6 summary/narrative split, C-SC2 bi-level Parquet partitioning (year-5), C-SC3 OPRA option fields (when OPRA ships), C-FF3 debounce-constants parameterization.

### Round 14 Amendments (v1.3 → v1.3.1)

**Agent verdicts (3 parallel agents):**
- **R14-A** (v1.3 adversarial audit): 7 new inconsistencies + 3 gaps blocking Week 1. Primary finding: **§14.8 normalization integrity gate was un-implementable** because nested envelope objects (`training_info`, `signal_provenance`, `strategy_info`, `export_stats`) had no SQLite storage.
- **R14-B** (Week 1 implementer dry-run): 9 ambiguities, realistic budget 54-76h vs spec's 36-40h. Primary finding: **Rust canonical JSON does NOT escape non-ASCII by default** — §3.3.6's claim was false; Day 5 parity test would fail on first unicode fixture.
- **R14-C** (3-month production pilot): 6 operational gaps, 0 blockers. Primary findings: silent quarantine accumulation (no `ledger status` CLI); `ledger backup` covers SQLite only (contradicts §2.2 source-of-truth rule); `ingest.log` rotation unimplemented.

**9 critical fixes (C1-C9) applied:**

**C1 — Storage layer for nested envelope objects.** Added 4 new TEXT columns to `experiments` table: `training_info_json`, `signal_provenance_json`, `strategy_info_json`, `export_stats_json`. Serialized via `canonical_json_dumps(sub_model.model_dump())`. Queryable via SQLite `json_extract()`. Added partial index `idx_experiments_training_norm_hash` on `json_extract(training_info_json, '$.normalization_stats_sha256')`. Updated §14.8 query from `SELECT metadata_json` to `SELECT json_extract(training_info_json, '$.normalization_stats_sha256')` — single-field lookup instead of full-blob parse. Updated §7.3.1 INSERT template (placeholder count 44 → 48).

**C2 — Rust canonical JSON non-ASCII escape.** Fixed §3.3.6's false claim that `serde_json` escapes non-ASCII. Added `escape_non_ascii_in_json()` post-processor that walks the serde_json output, tracks string-literal boundaries, escapes non-ASCII to `\uXXXX` (with surrogate pairs for supplementary code points per RFC 8259). Byte-matches Python's `json.dumps(..., ensure_ascii=True)`. Without this fix, Day 5 parity test fails on fixture `03_unicode.json`.

**C3 — TOML encoding of the envelope JSON Schema.** §15.1a Day 1 now explicitly specifies: envelope schema is a TOML **literal multiline string** (`'''...'''`) containing raw JSON Schema Draft-07 text. MetricKey/GateKey remain native TOML tables (§4.2/§4.5). Rationale: TOML literal strings take content verbatim (no backslash escaping), so JSON's `\uXXXX` and regex `\d` embed cleanly. Python loader: `json.loads(tomllib.load(f)['orchestration']['envelope']['json_schema'])`.

**C4 — Pydantic v2 Z-suffix serializer.** §15.1a Day 3 now specifies a `@field_serializer` on every datetime field in the codegen'd `Envelope` model. Pydantic v2's default `model_dump(mode='json')` emits `+00:00` offset — which FAILS the B1 Z-timezone regex. Custom `_to_iso_z()` helper emits `YYYY-MM-DDTHH:MM:SS[.fractional]Z`. Applied to `created_at`, `finalized_at`, `heartbeat_at`, `gates[].override_at`.

**C5 — §7.3.2 Helper Functions.** New subsection specifies 9 helpers referenced by §7.3/§7.4/§14.8 but previously undefined: `atomic_write_text`, `apply_pragmas`, `audit_log_append`, `quarantine`, `_escalate_pending_to_quarantine`, `_write_waiting_sidecar`, `_extract_missing_fps`, `check_upstream_integrity`, `compute_cohort_hash`. Each with 5-20 LOC Python reference implementation. `UpstreamNotYetIngestedError.__init__` accepts `missing_fingerprints` kwarg for structured sidecar generation.

**C6 — Schema migration bootstrap rule.** §5.3 now initializes `schema_migration_num='0'` at fresh-DB creation (was undefined — `apply_pending` could behave differently on raw-DDL vs migration-runner bootstrap). §5.7 codifies semver pairing: every migration's LAST statements MUST update both `schema_migration_num` AND `schema_version`; additive→minor bump; breaking→major bump. Atomic-commit rule: TOML + Python codegen + Rust struct + migration SQL MUST land in ONE commit; CI enforces.

**C7 — Pydantic dependency strategy.** §15.1a Day 2 specifies Pydantic as an OPTIONAL-dependency extra in `pyproject.toml`: `[project.optional-dependencies] orchestration = ["pydantic>=2.0,<3.0"]`. Core `hft_contracts` imports are unaffected (preserves backward compat for lean consumers). `hft_contracts.orchestration.*` modules guard imports with a clear error message pointing to `pip install hft-contracts[orchestration]`.

**C8 — Fixture path unified.** Resolved §15.1a vs §18.1b contradiction. Cross-language fixtures (envelopes + canonical_json) live canonically at `hft-contracts/tests/fixtures/`. hft-ops tests symlink to those paths (relative symlinks via git mode 120000). Rust tests read directly (same repo). Legacy + ledger-specific fixtures (ledgers, Parquet, migration golden outputs) stay at `hft-ops/tests/fixtures/ledger/`.

**C9 — §13.3 body updated.** The v1.3 change log claimed Round 13 widened Phase 3.5 gate across all 4 batches, but the §13.3 body still referenced Batch 1 only. Now amended to specify weekly `hft-ops sweep fingerprints --verify` runs across ALL 36 configs throughout Phase 11, with per-batch strict gates and an RFC-required incident protocol for any detected drift. (Fixed in self-check during R14 dispatch.)

**6 operational gaps (R14-C — documented as Phase 11 W2-3 additions, not blockers):**

- G1: `hft-ops ledger status` summary command (~30 LOC) — HIGH priority; prevents silent quarantine accumulation.
- G2: `ledger backup` fix to tar records/+metrics/+log (not SQLite only) — HIGH priority; fixes §2.2 contradiction.
- G3: `ledger rotate-log` CLI (~10 LOC) — LOW priority; year-scale.
- G4: `ledger gc --orphan-parquet` spec — LOW priority; year-scale.
- G5: Schema-change atomic-commit rule in §5.7 (~50 lines doc) — **APPLIED in C6** above.
- G6: Progress bars on rebuild/migrate/drain (~20 LOC tqdm) — LOW priority; UX only.

Total effort for G1+G2+G3+G4+G6: ~100 LOC + ~50 lines doc ≈ 1 day of work, added to Phase 11 Week 2-3 scope.

**Validation evidence**: 4 new R14 checks added to commit-time self-audit:
- §5.3 DDL reparse with 4 new nested-object columns (48-column experiments table).
- §14.8 query path via `json_extract(training_info_json, '$.normalization_stats_sha256')` — tested end-to-end with synthetic trainer envelope.
- §7.3.2 helper specs present (9 helpers documented).
- §5.3 `schema_migration_num='0'` bootstrap row present.

---

_End of design specification._
