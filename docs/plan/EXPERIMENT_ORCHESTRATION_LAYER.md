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
- Schema: `ExperimentRecord` dataclass at `hft-ops/src/hft_ops/ledger/experiment_record.py:57-134` with 20 fields.

**Fingerprinting**:
- Phase 3 shipped §3.3b: `compute_fingerprint()` in `hft-ops/src/hft_ops/ledger/dedup.py:174-281` now resolves `_base:` inheritance before hashing via `resolve_inheritance()` at `dedup.py:65-109` (runtime-lazy import of `lobtrainer.config.merge`).
- Algorithm: `sha256(json.dumps({extraction, training, backtest, data_manifest, contract_version}, sort_keys=True, default=str))`.
- Excludes: `{name, description, tags, version, output_dir, log_level, verbose, experiment}` and entire `stages.validation` section (validation is observation, not treatment — `dedup.py:253-257`).

**Config Composition** (Phase 3 Batch 1 in progress):
- `lob-model-trainer/src/lobtrainer/config/merge.py` (v2, hand-rolled, ~205 LOC) replaces `OmegaConf` (explicitly rejected after adversarial review).
- Supports `_base: str | list[str]`, left-to-right merge, child overrides.
- `_partial: true` sentinel for intermediate bases.
- 4 of 17 orthogonal bases created: `models/tlob_compact_regression.yaml`, `datasets/nvda_e5_60s.yaml`, `labels/regression_huber.yaml`, `train/regression_default.yaml`.
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

**Implementation**: `experiment_fingerprint` = Phase 3's existing hash (`dedup.py:174-281`). Phase 10 REUSES it. Does not introduce a competing hash.

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
- `lobtrainer.config.merge.resolve_inheritance` — lazy import, optional, fallback to raw YAML if missing (`dedup.py:94-100`).
- `hft_evaluator.fast_gate.run_fast_gate` — library import, explicit Phase 2b decision (`stages/validation.py:1-32`).

No new soft exceptions without updating this principle.

### §2.5 Contract-First Schema

**Principle**: Every cross-module structural contract lives in `contracts/pipeline_contract.toml` (SSoT) with codegen to consuming languages.

**Rationale**: Eliminates schema drift. Matches existing `FeatureIndex`/`LabelContract`/`ValidationConfig` patterns.

**Implementation**: 
- Envelope schema defined in `pipeline_contract.toml` under `[orchestration.envelope]`.
- MetricKey enum defined in `pipeline_contract.toml` under `[orchestration.metric_keys]`.
- Codegen: `contracts/generate_python_contract.py` produces `hft_contracts/orchestration.py` (Pydantic model + enum).
- Rust producers verify against the codegen'd Python via `verify_rust_constants.py`-style hash check.

**Anti-pattern rejected**: Independent schema definitions per module. MLflow's ad-hoc params.

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
      "description": "The `pipeline_contract.toml` contract edition at producer runtime (e.g., '2.2' or 'off_exchange_1.0')."
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
        "opra-feature-extractor"
      ],
      "description": "Enum extensible; new producers require schema version bump."
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
      "description": "ISO 8601 with UTC timezone. Producer wall-clock start time."
    },
    "finalized_at": {
      "type": ["string", "null"],
      "format": "date-time",
      "description": "ISO 8601. Producer wall-clock completion. NULL for status=running."
    },
    "heartbeat_at": {
      "type": ["string", "null"],
      "format": "date-time",
      "description": "Updated periodically by long-running producers (streaming future). Ignored for batch."
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
          "override_at": {"type": ["string", "null"], "format": "date-time"}
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
        "days_processed": {"type": "integer"},
        "total_sequences": {"type": "integer"},
        "sequence_length": {"type": "integer"},
        "stride": {"type": "integer"},
        "bin_size_seconds": {"type": ["integer", "null"]},
        "normalization": {"type": "string"},
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
        "training_time_seconds": {"type": "number"}
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
      "description": "Only for record_type='backtest'. Mirrors signal_metadata."
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
  "pipeline_contract_version": "off_exchange_1.0",
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

#### §3.3.6 Canonical JSON Serialization Spec (cross-language)

**Purpose:** the envelope `experiment_fingerprint` is computed by the producer (Phase 3 `dedup.py`) in Python, but future Rust producers (BQP, MBO extractor) must produce byte-identical fingerprints OR route their fingerprint through Python. This section specifies a single canonical JSON serialization that guarantees parity.

**Python side** (authoritative — matches current `dedup.py:390`):

```python
# hft_contracts/orchestration/canonical_json.py
import json

def canonical_json_dumps(obj) -> str:
    """Deterministic JSON for content hashing + fingerprinting.
    Matches dedup.py:390 behavior exactly.
    """
    return json.dumps(
        obj,
        sort_keys=True,
        separators=(",", ":"),    # no whitespace — compact form
        ensure_ascii=True,        # escape non-ASCII; avoids encoding drift
        allow_nan=False,          # reject NaN / Inf; caller must sanitize
        default=str,              # match dedup.py — non-JSON types fall back to str()
    )
```

**Rust side** (must produce byte-identical output):

```rust
// hft_contracts_rs/src/canonical_json.rs
// Cargo.toml: serde_json = { version = "1", features = ["preserve_order"] }  // NOT used here
use serde::Serialize;
use serde_json::{to_string, Value};

pub fn canonical_json_dumps<T: Serialize>(value: &T) -> Result<String, CanonicalError> {
    // Serialize to an intermediate Value tree, then re-serialize with sorted keys.
    // serde_json by default escapes non-ASCII and uses compact separators; matches
    // Python's ensure_ascii=True + separators=(",", ":"). Sort keys via a custom
    // walker because serde_json::to_string does not sort by default.
    let intermediate: Value = serde_json::to_value(value)?;
    let sorted = sort_keys_recursive(intermediate);
    // Reject NaN/Inf early (serde_json serializes to `null` by default — WRONG)
    reject_non_finite(&sorted)?;
    Ok(to_string(&sorted)?)  // serde_json::to_string is compact + ASCII-escaped
}

fn sort_keys_recursive(v: Value) -> Value {
    match v {
        Value::Object(m) => {
            let mut sorted: std::collections::BTreeMap<String, Value> =
                m.into_iter().map(|(k, v)| (k, sort_keys_recursive(v))).collect();
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(xs) => Value::Array(xs.into_iter().map(sort_keys_recursive).collect()),
        other => other,
    }
}

fn reject_non_finite(v: &Value) -> Result<(), CanonicalError> {
    match v {
        Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                if !f.is_finite() {
                    return Err(CanonicalError::NonFinite);
                }
            }
            Ok(())
        }
        Value::Object(m) => m.values().try_for_each(reject_non_finite),
        Value::Array(xs) => xs.iter().try_for_each(reject_non_finite),
        _ => Ok(()),
    }
}
```

**Invariants (tested by cross-lang fixtures):**
1. **Key order**: lexicographic ASCII sort at every nesting level (matches Python `sort_keys=True`).
2. **Separators**: `","` between items, `":"` between key-value (no spaces).
3. **Encoding**: non-ASCII characters escaped as `\uXXXX` (matches Python `ensure_ascii=True`).
4. **NaN / Inf / -Inf**: rejected with error (Python `allow_nan=False`; Rust `reject_non_finite`).
5. **Integer representation**: JSON numbers without decimal point (e.g., `10` not `10.0`). Python handles this via `json.dumps`; Rust via `serde_json::Number::Integer`.
6. **Unicode normalization**: NOT applied (same raw bytes on both sides; no NFC/NFD reshaping).
7. **Trailing whitespace**: none.

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

The Rust `hft_contracts` crate imports these fixtures and asserts `canonical_json_dumps` produces the matching hash; the Python `hft_contracts` package does the same. CI runs both suites on every PR.

**Non-goal for v1:** floating-point representation stability across Python ↔ Rust when numbers have >15 significant digits. Document as known limitation; producers should either (a) round to 12 digits before hashing, or (b) pass all floating-point values through Python first.

### §3.4 Envelope Sizing and Inlining Rules

- `config_source`: inline if config text < 8 KB; else use `artifacts[{kind:"config"}]` + `config_source: null`.
- `sub_records`: inline only for `sweep_aggregate`; max 50 children or 256 KB total; else each child is its own envelope with shared `sweep_id`.
- `bulk_parquet`: always separate files, never inlined.
- Total envelope size target: ≤ 128 KB. Hard limit: 1 MB. Rejected if exceeds hard limit.

### §3.5 Envelope Filename Convention

`hft-ops/ledger/inbox/{content_hash}.json` where `content_hash = sha256(envelope_minus_metadata_json)`.

Content hash excludes `metadata_json` (escape hatch — forward compat must not affect identity).

This makes envelopes **idempotent**: the same envelope emitted twice has the same filename and `os.replace()` is a no-op. Eliminates FM-B1 collision.

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
```

Gate enum allows hft-rules §13 enforcement to be SCHEMA-BACKED, not documentation-only.

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

    -- Forward-compat
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
        envelope = json.load(record_file.open())
        try:
            insert_envelope(conn, envelope)
        except SchemaViolation as e:
            log.warn(f"Skip {record_file.name}: {e}")
    conn.commit()
    conn.close()
    os.replace(new_db, sqlite_path)  # atomic
```

Rebuild does NOT regenerate Parquet files. If Parquet is missing, `artifacts`/`bulk_parquet_refs` rows still point to (missing) paths; `hft-ops ledger check` surfaces the inconsistency.

---

## §6 — Fingerprint Alignment with Phase 3

### §6.1 Authoritative Fingerprint Source

Phase 3's `compute_fingerprint()` (`hft-ops/src/hft_ops/ledger/dedup.py:174-281`) is the ONLY fingerprint authority. Phase 10 REUSES it verbatim.

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
3. Compute envelope `content_hash = sha256(envelope_json_without_metadata_json)`.
4. Write envelope atomically: `open(f"{content_hash}.json.tmp")` → write + fsync → `os.replace("{content_hash}.json")`.
5. Place in `hft-ops/ledger/inbox/`.
6. Emit Parquet side files (if any) with content-addressable names to `hft-ops/ledger/metrics/{yyyy_mm}/{parquet_hash}.parquet`.

A producer SHOULD:
- Log envelope write to stderr with `content_hash` for debugging.
- Not batch envelopes (each experiment writes its own envelope).
- Use `HFT_OPS_ORCHESTRATED=1` env var convention to detect orchestrated invocation (reused from current `stages/base.py:138`).

A producer MUST NOT:
- Write directly to `ledger.sqlite`.
- Write directly to `records/*.json`.
- Delete or modify another producer's envelope.
- Block on ingest completion (ingest is async from producer's perspective).

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
    # Acquire exclusive lock
    with flock_exclusive(ledger_dir / "ledger.lock", timeout=5.0) as lock:
        inbox = sorted((ledger_dir / "inbox").glob("*.json"))
        report = IngestReport()
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

    # Step 2: append-only JSON record (atomic tmp+rename)
    record_path = ledger_dir / "records" / f"{validated.experiment_id}.json"
    if record_path.exists():
        # Idempotent: compare content byte-for-byte (canonical JSON via §3.3.7).
        if record_path.read_bytes() == envelope_path.read_bytes():
            envelope_path.unlink()
            audit_log_append(ledger_dir, validated, result="duplicate_idempotent",
                             duration_ms=int(1000 * (time.perf_counter() - t0)))
            return IngestOneResult.duplicate_idempotent(validated)
        else:
            return quarantine(envelope_path, f"Conflict: record {validated.experiment_id} exists with different content")
    atomic_write_json(record_path, envelope)

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
            insert_experiment(conn, validated)       # §7.3.1 INSERT template
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

Each `insert_*` helper in `ingest_one` follows a consistent template. Values are bound via `?` placeholders (never string-concat — SQL injection guard AND correct NULL handling).

```sql
-- insert_experiment: ONE row per envelope
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
    metadata_json,
    json_record_path, ingested_at
) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?);
-- cohort_hash initially NULL; set by post-insert UPDATE (same txn)

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
4. **Cross-field validation** — e.g., `n_horizons == len(horizons)`; `record_type='export'` requires `export_stats` non-null; `record_type='training'` requires `model_type` non-null.
5. **Fingerprint plausibility** — 64-char lowercase hex?
6. **Artifact/Parquet path existence** — warn (not fail) if referenced file is missing (allows producer-writes-envelope-first race to resolve; ingest may be delayed).
7. **Foreign key plausibility** — if `upstream_experiment_ids` non-empty, referenced IDs SHOULD exist in experiments table (warn if not — may be out-of-order ingest).

Failures at 1-5: quarantine + `.error` sidecar. Failures at 6-7: log warning but proceed.

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

The 34 retroactive records live as `ExperimentRecord` dataclass instances at `hft-ops/src/hft_ops/ledger/experiment_record.py:57-134`. Step 2 of §13.1 transforms each into a v0-permissive envelope before ingesting via the standard pipeline. Mapping is 1:1 where possible; v0-specific accommodations are marked with ★.

| Legacy `ExperimentRecord` field | Envelope v0 target | Notes |
|---|---|---|
| `experiment_id` | `.experiment_id` | Direct copy. |
| `fingerprint` | `.experiment_fingerprint` | Direct copy; `fingerprint_version = 1`. |
| `name` | `.description` (fallback) OR `.metadata_json.legacy_name` | Legacy "name" is a free-form label; not used in v1 envelope schema. Preserved for traceability. |
| `description` | `.description` | Direct copy (may concatenate with legacy name). |
| `record_type` | `.record_type` | Direct copy; must be in v1 enum. Legacy `calibration`/`analysis` pass through. |
| `created_at` | `.created_at` | Direct copy (legacy format is ISO 8601). |
| `finalized_at` | `.finalized_at` | Direct copy (may be NULL for in-progress legacy records). |
| `status` | `.status` | Direct copy. Legacy status values (`completed`, `failed`, etc.) match v1 enum. |
| `config_files` (list) | `.artifacts[]` with `kind="config"` | One artifact per config file. `path` = absolute; `bytes` = file size; `sha256` computed at migration time. ★ |
| `outputs` (dict of path→meta) | `.artifacts[]` | Flattened. Each output becomes one artifact row. `kind` inferred from path extension + heuristic. ★ |
| `metrics` (dict: name→value) | `.metrics[]` long-format | `{name: v}` → `{family: <inferred>, name: name.upper(), value: v, split: null, horizon: null, epoch: null, tag: null}`. Family inferred via regex (`ic*` → regression, `sharpe*` → backtest_pnl, `acc*` → classification). ★ |
| `upstream_experiment_ids` | `.upstream_ids` | Direct copy (JSON array of experiment_ids). |
| `gates` (dict: gate→{status, threshold, observed}) | `.gates[]` list | One row per gate. `gate_order = index` in iteration order (legacy dict ordering is not guaranteed; fallback to alphabetical). ★ |
| `tags` (list) | `.tags[]` | Direct copy. |
| `git_state` (dict: commit, branch, dirty) | `.git` | Direct copy (key rename: `commit` → `commit_hash`). |
| `data_paths` (list) | `.lineage[]` with `source_kind="raw_data"` OR `"export"` | Inferred. `source_hash` from `hash_directory_manifest(path)` if directory exists; else NULL. ★ |
| `notes` (str) | `.notes` (top-level string) | Direct copy. Separately, if non-empty AND > 64 chars, a row is also inserted in `notes` table by the migration post-pass. |
| `errors` (list) | `.metadata_json.legacy_errors` | v1 envelope has no `errors` field; preserved as escape-hatch. |
| `duration_seconds` | `.wall_clock_ms` | Multiply by 1000; round to int. |
| `parent_experiment_id` | `.parent_id` | Direct copy. |
| `producer` (field may not exist in legacy) | `.producer` | Default: `"legacy-migrator"` if absent. ★ |
| `producer_version` | `.producer_version` | Default: `"unknown"`. ★ |
| `envelope_schema_version` | `.envelope_schema_version` | Hardcoded: `"0.0.0-legacy"`. ★ Explicitly signals v0 permissive path. |
| `pipeline_contract_version` | `.pipeline_contract_version` | Default: `"2.2"` (current); override if legacy record specifies. |
| `symbol` | `.symbol` | Default: `"NVDA"` (all 34 legacy records are NVDA). ★ |
| `asset_class` | `.asset_class` | Default: `"equity"`. ★ |
| `data_source_type` | `.data_source_type` | Inferred from `data_paths` + `producer`: `"mbo_lob_reconstructed"`, `"equity_off_exchange_trf"`, `"legacy-unknown"`. ★ |
| `feature_schema_ref` | `.feature_schema_ref` | Inferred similarly; default `"legacy-unknown"`. ★ |
| `json_record_path` | `.json_record_path` | Set at ingest time: `records/{experiment_id}.json`. |
| `ingested_at` | `.ingested_at` | Set at ingest time: `now()`. |
| `metadata_json` (any other dataclass field) | `.metadata_json.*` | Catch-all; forward-compatible. |

**Fields in v1 envelope NOT present in legacy (defaulted to NULL/empty):**
- `.heartbeat_at` (NULL — legacy doesn't stream)
- `.dataset` (NULL — legacy doesn't track dataset identifier)
- `.model_type`, `.model_family`, `.task` (NULL unless in `metadata_json`)
- `.sampling_strategy`, `.sampling_config` (NULL)
- `.label_family`, `.primary_horizon`, `.n_horizons`, `.horizons`, `.label_spec` (NULL)
- `.config_source`, `.config_format` (legacy uses `config_files` artifacts instead)
- `.sweep_id`, `.axis_values` (NULL — no sweeps retroactively)
- `.bulk_parquet[]` (empty — legacy has no Parquet side files)
- `.export_stats` (NULL — not captured in legacy)
- `.cohort_hash` (computed by ingester post-insert; same path as v1)

**Migration test (§18.5):** for each of 34 legacy records, verify:
1. v0 envelope round-trips through the ingester without quarantine.
2. `experiment_fingerprint` bytes-identical pre- and post-migration.
3. Post-migration `records/{id}.json` contents are CANONICAL v1 form (not a copy of the legacy record).
4. `SELECT COUNT(*) FROM experiments` = 34 after Step 2 completes.
5. Rendered `EXPERIMENT_INDEX.md` sections for each legacy experiment semantically match the hand-written entries (field coverage, key metrics, gate results — fuzzy match, not byte-identical).

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
- Reason: Phase 10 Step 3 producers must compute fingerprints using `resolve_inheritance` — Phase 3.5 is what populates the bases.

Coordination checkpoint: before Step 3, run `hft-ops sweep fingerprints --verify` on all 36 configs. If any fingerprint drift is detected, halt Step 3 until Phase 3.5 batch completes.

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

- `compute_fingerprint()` logic at `hft-ops/src/hft_ops/ledger/dedup.py:174-281` is CANONICAL.
- `_load_trainer_config_resolved()` (`dedup.py:65-109`) resolves `_base:` before hashing. Phase 10 does NOT bypass this.
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

---

## §15 — Implementation Phases (Phase 11-14+)

### §15.1 Phase 11: Ledger v2 Core + Migration (4 weeks)

**Timeline note (Round 10):** bumped from 3 → 4 weeks to absorb cross-language envelope codegen (Python Pydantic + Rust serde), SQLite lock-contention hardening under concurrency, and the four parallel subprocess producer rollouts (BQP, MBO extractor, trainer, backtester).

**Deliverables**:
- Contract-first envelope schema in `pipeline_contract.toml` (envelope v1 JSON Schema, MetricKey, GateKey).
- Codegen → `hft_contracts/orchestration.py` Pydantic model + Rust `hft_contracts::orchestration` struct.
- **Canonical JSON serialization library** (shared Python + Rust) — see §3.3.7 — so the cross-language fingerprint computation stays byte-identical.
- MetricKey + GateKey enums in `hft-contracts` (underscore form registry per §4.5; §4.2 registry parsed by TOML loader).
- SQLite schema (**10 tables** after Round 10 consolidation) + PRAGMAs (WAL, synchronous=NORMAL, foreign_keys=ON).
- Append-only `ingest.log` JSONL file (replaces the previously-proposed `ingest_audit` table).
- CLI: `hft-ops ledger {ingest, query, show, compare, diff, list, rebuild, check, backup, quarantine, audit-log, note, tag, render-indexes, regenerate-parquet}`.
- Python SDK: `Ledger` class + helpers; optional `pandas` passthrough.
- Migration CLI: 4-step backfill of 34 retroactive records (§13.1).
- **Legacy → envelope-v0 field mapping** (§13.1b) with tests that migrate 34/34 records byte-identically.
- **Post-ingest debounced index render** (§4.6) replaces eager render.
- **Rollback protocol** (§7.4.1) — partial commit recovery documented and tested.
- **Test coverage: ≥ 40 new hft-ops tests** (ingest, query, migration, failure recovery, rebuild, atomic writes, rollback, fault-injection).

**Success criteria**:
- One `hft-ops run` produces a non-retro ledger record end-to-end.
- All 34 legacy records migrated (fingerprints preserved; §13.1b mapping table verifies byte-for-byte).
- Query Q1 ("all TLOB experiments with test IC > 0.05") returns in < 100 ms p95 on 1000-record ledger.
- Failure modes §12.1.1-§12.1.10 each verified by at least one fault-injection test.
- `sqlite3 ledger.sqlite < schema.sql` parses cleanly (Round 10 regression — DDL syntax-correctness gate).

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

Every test category below references fixtures under `hft-ops/tests/fixtures/ledger/` (unless otherwise noted). The layout is:

```
hft-ops/tests/fixtures/ledger/
├── envelopes/
│   ├── valid/                           # one per record_type, representative
│   │   ├── v1_bqp_export.json
│   │   ├── v1_mbo_export.json
│   │   ├── v1_trainer_training.json
│   │   ├── v1_backtester_backtest.json
│   │   ├── v1_evaluator_evaluation.json
│   │   └── v1_sweep_aggregate.json
│   ├── invalid/                         # one per failure mode §7.6 can catch
│   │   ├── 01_bad_json.json             # syntactically invalid
│   │   ├── 02_schema_violation.json     # missing required field
│   │   ├── 03_enum_mismatch.json        # asset_class='stocks'
│   │   ├── 04_cross_field_mismatch.json # n_horizons != len(horizons)
│   │   ├── 05_fingerprint_malformed.json  # non-hex
│   │   ├── 06_missing_artifact.json     # path references nonexistent file
│   │   └── 07_oversized.json            # > 1 MB hard limit
│   ├── legacy/                          # 34 legacy ExperimentRecord fixtures
│   │   ├── hmhp_128feat_2026_03_13.json
│   │   ├── tlob_regression_2026_03_15.json
│   │   └── ...(32 more — one per legacy record)
│   └── future/                          # envelope_version=2, 3, 99 for dispatch testing
│       ├── v2_unknown.json
│       └── v99_unknown.json
├── ledgers/                             # pre-canned SQLite DBs for query/rebuild tests
│   ├── empty.sqlite
│   ├── one_experiment.sqlite
│   ├── hundred_experiments.sqlite
│   └── corrupt_missing_indexes.sqlite
├── parquet/                             # side files referenced by envelopes
│   ├── training_curve_500epochs.parquet
│   ├── feature_ic_148.parquet
│   └── equity_curve_233days.parquet
├── canonical_json/                      # cross-lang fingerprint fixtures (§3.3.6)
│   ├── 01_simple.json
│   ├── 02_nested.json
│   ├── 03_unicode.json
│   ├── 04_float_int.json
│   ├── 05_nan_rejected.json
│   ├── 06_large_array.json
│   └── expected_sha256.json             # hash of each above; checked by Py + Rust
└── golden/                              # migration + render golden outputs
    ├── migrated_experiments.json        # expected post-migration experiments rows
    ├── rendered_experiment_index.md     # expected EXPERIMENT_INDEX.md after render
    └── rendered_backtest_index.md
```

**Fixture conventions:**
- All JSON fixtures are canonical form (§3.3.6) — no trailing newlines, sorted keys, ASCII-escaped.
- `expected_sha256.json` maps fixture path → content hash; test failure if hash drifts (catches accidental fixture edits).
- Parquet fixtures built via `tests/conftest.py::build_fixtures()` at session start (deterministic, reproducible from a seed).

**Rust-side fixtures** (`hft-contracts` Rust crate, when available): symlinks to `hft-ops/tests/fixtures/ledger/canonical_json/` and `envelopes/valid/`. Rust tests assert `canonical_json_dumps` + `serde_json::from_str::<Envelope>()` produce bit-identical results to Python.

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
- `hft-ops/src/hft_ops/ledger/dedup.py:65-109,174-281` — fingerprint algorithm (§3.3b fix).
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

---

_End of design specification._
