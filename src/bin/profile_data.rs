//! Data profiling CLI: per-day statistics for diagnostics.
//!
//! Usage: `profile_data --config configs/nvda_60s.toml [--date 2025-02-03]`

use std::path::Path;

use clap::Parser;

use basic_quote_processor::config::DatasetConfig;
use basic_quote_processor::context::DailyContextLoader;
use basic_quote_processor::dates;
use basic_quote_processor::pipeline::DayPipeline;
use basic_quote_processor::reader::discover_files;

#[derive(Parser)]
#[command(
    name = "profile_data",
    about = "Print per-day statistics for diagnostics (no NPY export)"
)]
struct Args {
    /// Path to TOML config file
    #[arg(long)]
    config: String,

    /// Process only a single date (YYYY-MM-DD)
    #[arg(long)]
    date: Option<String>,
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
    let config = DatasetConfig::from_toml(Path::new(&args.config))?;

    let context_loader = match &config.input.equs_summary_path {
        Some(path) => DailyContextLoader::from_file(Path::new(path))?,
        None => DailyContextLoader::empty(),
    };

    let processor_config = config.to_processor_config();
    let mut pipeline = DayPipeline::new(&processor_config)?;

    let target_dates = if let Some(ref date_str) = args.date {
        vec![dates::parse_iso_date(date_str)?]
    } else {
        let start = dates::parse_iso_date(&config.dates.start_date)?;
        let end = dates::parse_iso_date(&config.dates.end_date)?;
        let exclude: Vec<_> = config.dates.exclude_dates.iter()
            .filter_map(|s| dates::parse_iso_date(s).ok())
            .collect();
        dates::enumerate_weekdays_excluding(start, end, &exclude)
    };

    let file_map: std::collections::HashMap<_, _> = discover_files(
        Path::new(&config.input.data_dir),
        &config.input.filename_pattern,
    )?.into_iter().collect();

    eprintln!("{:<12} {:>8} {:>8} {:>8} {:>8} {:>6} {:>6} {:>8}",
        "Date", "Records", "Trades", "TRF", "Lit", "Bins", "Empty", "Seqs");
    eprintln!("{}", "-".repeat(78));

    for date in &target_dates {
        let date_str = dates::date_to_file_date(*date);
        let iso_str = dates::date_to_iso(*date);

        let file_path = match file_map.get(&date_str) {
            Some(p) => p.clone(),
            None => continue,
        };

        let context = context_loader.get(*date);
        let (year, month, day) = dates::parse_file_date(&date_str)?;

        pipeline.init_day_with_context(year, month, day, Some(context));
        pipeline.stream_file(&file_path)?;
        let export = pipeline.finalize()?;
        let summary = pipeline.day_summary();

        eprintln!(
            "{:<12} {:>8} {:>8} {:>8} {:>8} {:>6} {:>6} {:>8}",
            iso_str,
            summary.total_records_processed,
            summary.total_trade_records,
            summary.total_trf_trades,
            summary.total_lit_trades,
            summary.total_bins_emitted,
            summary.total_empty_bins,
            export.sequences.len(),
        );

        pipeline.reset();
    }

    Ok(())
}
