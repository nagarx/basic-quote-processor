//! Multi-day export CLI for off-exchange feature datasets.
//!
//! Usage: `export_dataset --config configs/nvda_60s.toml [--force]`

use std::path::{Path, PathBuf};
use std::time::Instant;

use clap::Parser;

use basic_quote_processor::config::DatasetConfig;
use basic_quote_processor::context::DailyContextLoader;
use basic_quote_processor::dates::{self, Split};
use basic_quote_processor::export::manifest::DatasetManifest;
use basic_quote_processor::export::DayExporter;
use basic_quote_processor::pipeline::DayPipeline;
use basic_quote_processor::reader::discover_files;

#[derive(Parser)]
#[command(
    name = "export_dataset",
    about = "Export off-exchange features to NPY sequences + labels"
)]
struct Args {
    /// Path to TOML config file
    #[arg(long)]
    config: String,

    /// Overwrite existing exports (skip manifest check)
    #[arg(long, default_value_t = false)]
    force: bool,
}

fn main() {
    env_logger::init();
    let args = Args::parse();

    if let Err(e) = run(&args) {
        eprintln!("ERROR: {e}");
        std::process::exit(1);
    }
}

fn run(args: &Args) -> basic_quote_processor::Result<()> {
    let config_path = Path::new(&args.config);
    let config = DatasetConfig::from_toml(config_path)?;

    let output_dir = PathBuf::from(&config.export.output_dir);

    // Overwrite protection
    let manifest_path = output_dir.join("dataset_manifest.json");
    if manifest_path.exists() && !args.force {
        return Err(basic_quote_processor::ProcessorError::export(format!(
            "Output directory already has dataset_manifest.json at {}. \
             Use --force to overwrite.",
            manifest_path.display()
        )));
    }

    // Load EQUS_SUMMARY context
    let context_loader = match &config.input.equs_summary_path {
        Some(path) => {
            eprintln!("Loading EQUS_SUMMARY from {path}...");
            DailyContextLoader::from_file(Path::new(path))?
        }
        None => {
            eprintln!("No equs_summary_path configured, proceeding without daily context");
            DailyContextLoader::empty()
        }
    };
    eprintln!("  {} dates loaded", context_loader.n_dates());

    // Enumerate dates
    let start = dates::parse_iso_date(&config.dates.start_date)?;
    let end = dates::parse_iso_date(&config.dates.end_date)?;
    let exclude: Vec<_> = config.dates.exclude_dates.iter()
        .filter_map(|s| dates::parse_iso_date(s).ok())
        .collect();
    let all_dates = dates::enumerate_weekdays_excluding(start, end, &exclude);
    eprintln!("Date range: {} to {} ({} weekdays)", start, end, all_dates.len());

    // Parse split boundaries
    let train_end = dates::parse_iso_date(&config.export.split_dates.train_end)?;
    let val_end = dates::parse_iso_date(&config.export.split_dates.val_end)?;

    // Discover available data files
    let available_files = discover_files(
        Path::new(&config.input.data_dir),
        &config.input.filename_pattern,
    )?;
    let file_map: std::collections::HashMap<String, PathBuf> = available_files
        .into_iter()
        .collect();
    eprintln!("  {} data files found", file_map.len());

    // Create output directories
    std::fs::create_dir_all(output_dir.join("train"))?;
    std::fs::create_dir_all(output_dir.join("val"))?;
    std::fs::create_dir_all(output_dir.join("test"))?;

    // Initialize pipeline and manifest
    let processor_config = config.to_processor_config();
    let mut pipeline = DayPipeline::new(&processor_config)?;
    let apply_norm = config.export.normalization != "none";

    // Phase 9.4 — canonical config hash for metadata provenance. Computed once
    // from ProcessorConfig (processing identity only — dates/output_dir/experiment
    // are not included so the same processing config produces the same hash across
    // different date ranges or output locations).
    let config_hash = processor_config.config_hash_hex()?;
    eprintln!("config_hash: {}", config_hash);
    pipeline.set_config_hash(config_hash.clone());

    // Phase 9.5 — thread the actual configured normalization strategy into
    // metadata. Replaces the previously-hardcoded "per_day_zscore" string.
    pipeline.set_normalization_strategy(config.export.normalization.clone());
    // Phase 9.4 — honest `applied` field paired with the strategy above.
    // Under T15 ("Raw Rust"), `apply_norm` is false for every production config.
    pipeline.set_normalization_applied(apply_norm);
    // Phase 9.4 / D13 — self-identifying metadata: experiment name from config.
    pipeline.set_experiment(config.export.experiment.clone());

    let mut manifest = DatasetManifest::new(
        &config.export.experiment,
        &config.input.symbol,
        config.sequence.window_size,
        config.sequence.stride,
        config.sampling.bin_size_seconds,
        config.labeling.horizons.clone(),
        &config.export.normalization,
    );
    // Phase 9.4 — surface the per-run config hash at the dataset level so
    // `dataset_manifest.json` is self-identifying without reading every
    // per-day metadata file.
    manifest.set_config_hash(&config_hash);

    // Process each date
    let total = all_dates.len();
    let mut errors: Vec<(String, String)> = Vec::new();

    for (idx, date) in all_dates.iter().enumerate() {
        let date_str = dates::date_to_file_date(*date);
        let iso_str = dates::date_to_iso(*date);
        let split = dates::assign_split(*date, train_end, val_end);

        // Find data file for this date
        let file_path = match file_map.get(&date_str) {
            Some(p) => p.clone(),
            None => {
                // Round 8 (Agent E2.4 fix): surface missing-file events at
                // `warn!` (not `debug!`) so they appear in the default log.
                // Silent skipping has hidden typo'd `filename_pattern` bugs.
                log::warn!("No data file for {iso_str}, skipping");
                continue;
            }
        };

        let timer = Instant::now();
        let context = context_loader.get(*date);
        let (year, month, day) = dates::parse_file_date(&date_str)?;

        // F1 — set the source file basename for this day's metadata provenance.
        // Basename (not full path) keeps metadata portable across machines; the
        // full path embeds user-specific filesystem layout.
        // P5 (#23): surface the previously-silent fallback. A discovered file
        // should always have a UTF-8 basename; warn loudly if not (otherwise the
        // metadata silently carries an empty `source_file` with no diagnostic).
        let source_basename = match file_path.file_name().and_then(|s| s.to_str()) {
            Some(name) => name.to_string(),
            None => {
                log::warn!(
                    "Could not resolve a UTF-8 basename for {}; \
                     provenance.source_file will be empty",
                    file_path.display()
                );
                String::new()
            }
        };
        pipeline.set_source_file(source_basename);

        // #28: forensic input-content hash of the raw (compressed) `.dbn.zst`,
        // via the crate's streaming `sha256_file` (reuses the existing `sha2`
        // dep). On failure, warn and continue with the hash omitted — provenance
        // enrichment must never abort a day's export.
        match basic_quote_processor::sha256_file(&file_path) {
            Ok(hash) => pipeline.set_data_file_sha256(hash),
            Err(e) => log::warn!(
                "Could not hash input {} for provenance: {e}; \
                 provenance.data_file_sha256 will be omitted",
                file_path.display()
            ),
        }

        // Process day
        pipeline.init_day_with_context(year, month, day, Some(context));
        let result = pipeline.stream_file(&file_path)
            .and_then(|()| pipeline.finalize());

        match result {
            Ok(export) => {
                let n_seq = export.sequences.len();
                let split_dir = output_dir.join(split.to_string());
                let exporter = DayExporter::from_dir(&split_dir, apply_norm)?;
                let exported = exporter.export_day(&export)?;

                // Record in manifest
                let split_detail = match split {
                    Split::Train => &mut manifest.splits.train,
                    Split::Val => &mut manifest.splits.val,
                    Split::Test => &mut manifest.splits.test,
                };
                split_detail.record_day(&iso_str, exported);

                // G3: aggregate the per-day diagnostics sidecar path. Only when
                // sequences were actually written — export_day writes nothing
                // (including no sidecar) for a 0-sequence day.
                if exported > 0 {
                    manifest
                        .diagnostics_files
                        .push(format!("{}/{}_diagnostics.json", split, iso_str));
                } else {
                    // #24: a day that streamed OK but produced 0 sequences is
                    // still recorded in `days[]` (counts unchanged) AND annotated
                    // here, so consumers can distinguish an empty day from a
                    // populated one. Does NOT flip `complete` (observation, not
                    // failure). export_day writes no files for a 0-seq day, so
                    // there is correctly no diagnostics sidecar to aggregate.
                    manifest.zero_sequence_days.push(iso_str.clone());
                }

                let elapsed = timer.elapsed();
                eprintln!(
                    "[{:>3}/{}] {} {:>4} sequences ({}), {:.1}s",
                    idx + 1, total, iso_str, n_seq, split, elapsed.as_secs_f64()
                );
            }
            Err(e) => {
                let err_msg = format!("{e}");
                eprintln!(
                    "[{:>3}/{}] {} FAILED: {}",
                    idx + 1, total, iso_str, err_msg
                );
                let split_detail = match split {
                    Split::Train => &mut manifest.splits.train,
                    Split::Val => &mut manifest.splits.val,
                    Split::Test => &mut manifest.splits.test,
                };
                split_detail.record_failure(&iso_str, &err_msg);
                errors.push((iso_str.clone(), err_msg));

                if !config.export.continue_on_error {
                    return Err(e);
                }
            }
        }

        pipeline.reset();

        // Write incremental manifest (complete: false)
        manifest.update_totals();
        manifest.write_to_file(&manifest_path)?;
    }

    // Finalize manifest
    manifest.mark_complete();
    manifest.write_to_file(&manifest_path)?;

    // Copy config TOML alongside manifest. P5 (#23): surface BOTH previously-
    // silent failures (re-read + write) at warn! — this is a best-effort
    // provenance sidecar, so warn-and-continue (never abort the completed export).
    let config_copy_path = output_dir.join("export_config.toml");
    match std::fs::read_to_string(config_path) {
        Ok(config_content) => {
            if let Err(e) = std::fs::write(&config_copy_path, config_content) {
                log::warn!(
                    "Failed to write config copy to {}: {e}",
                    config_copy_path.display()
                );
            }
        }
        Err(e) => log::warn!(
            "Failed to re-read config {} for copy: {e}",
            config_path.display()
        ),
    }

    // Print summary
    eprintln!("\n═══ Export Complete ═══");
    eprintln!("  Train: {} days, {} sequences", manifest.splits.train.n_days, manifest.splits.train.n_sequences);
    eprintln!("  Val:   {} days, {} sequences", manifest.splits.val.n_days, manifest.splits.val.n_sequences);
    eprintln!("  Test:  {} days, {} sequences", manifest.splits.test.n_days, manifest.splits.test.n_sequences);
    eprintln!("  Total: {} days, {} sequences", manifest.days_processed, manifest.total_sequences);
    if !errors.is_empty() {
        eprintln!("  Failed: {} days", errors.len());
        for (date, err) in &errors {
            eprintln!("    {date}: {err}");
        }
    }
    eprintln!("  Output: {}", output_dir.display());

    Ok(())
}
