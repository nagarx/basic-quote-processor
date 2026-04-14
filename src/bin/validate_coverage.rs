//! Coverage validation CLI: cross-checks TRF+lit volume against EQUS_SUMMARY.
//!
//! Usage: `validate_coverage --config configs/nvda_60s.toml`

use std::path::Path;

use clap::Parser;

use basic_quote_processor::config::DatasetConfig;
use basic_quote_processor::context::DailyContextLoader;
use basic_quote_processor::dates;
use basic_quote_processor::pipeline::DayPipeline;
use basic_quote_processor::reader::discover_files;

#[derive(Parser)]
#[command(
    name = "validate_coverage",
    about = "Cross-check TRF+lit volume against EQUS_SUMMARY consolidated volume"
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
        None => {
            eprintln!("ERROR: equs_summary_path is required for coverage validation");
            return Err(basic_quote_processor::ProcessorError::config(
                "equs_summary_path required for validate_coverage",
            ));
        }
    };

    let processor_config = config.to_processor_config();
    let mut pipeline = DayPipeline::new(&processor_config)?;

    // Build date list
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

    eprintln!("{:<12} {:>12} {:>12} {:>14} {:>10} {:>12}",
        "Date", "TRF Vol", "Lit Vol", "Consolidated", "Coverage", "DarkShare");
    eprintln!("{}", "-".repeat(78));

    let mut coverage_values: Vec<f64> = Vec::new();
    let mut dark_share_values: Vec<f64> = Vec::new();

    for date in &target_dates {
        let date_str = dates::date_to_file_date(*date);
        let iso_str = dates::date_to_iso(*date);

        let file_path = match file_map.get(&date_str) {
            Some(p) => p.clone(),
            None => continue,
        };

        let context = context_loader.get(*date);
        let consolidated = context.consolidated_volume;
        let (year, month, day) = dates::parse_file_date(&date_str)?;

        pipeline.init_day_with_context(year, month, day, Some(context));
        pipeline.stream_file(&file_path)?;
        let _export = pipeline.finalize()?;
        let summary = pipeline.day_summary();

        let trf_vol = summary.total_trf_volume;
        let lit_vol = summary.total_lit_volume;

        if let Some(cv) = consolidated {
            let coverage = (trf_vol + lit_vol) / cv as f64;
            let dark_share = trf_vol / cv as f64;
            coverage_values.push(coverage);
            dark_share_values.push(dark_share);

            eprintln!(
                "{:<12} {:>12.0} {:>12.0} {:>14} {:>9.1}% {:>11.1}%",
                iso_str, trf_vol, lit_vol, cv,
                coverage * 100.0, dark_share * 100.0
            );
        } else {
            eprintln!("{:<12} {:>12.0} {:>12.0} {:>14} {:>10} {:>12}",
                iso_str, trf_vol, lit_vol, "N/A", "N/A", "N/A");
        }

        pipeline.reset();
    }

    // Summary
    if !coverage_values.is_empty() {
        let n = coverage_values.len() as f64;
        let mean_cov = coverage_values.iter().sum::<f64>() / n;
        let mean_ds = dark_share_values.iter().sum::<f64>() / n;
        let min_cov = coverage_values.iter().cloned().fold(f64::INFINITY, f64::min);
        let max_cov = coverage_values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

        eprintln!("\n═══ Coverage Summary ({} days) ═══", coverage_values.len());
        eprintln!("  Mean coverage:   {:.1}%", mean_cov * 100.0);
        eprintln!("  Range:           {:.1}% — {:.1}%", min_cov * 100.0, max_cov * 100.0);
        eprintln!("  Mean dark share: {:.1}%", mean_ds * 100.0);

        let in_range = mean_cov >= 0.80 && mean_cov <= 0.86;
        if in_range {
            eprintln!("  Status: PASS (within 81-85% expected range)");
        } else {
            eprintln!("  Status: WARNING (outside 81-85% expected range)");
        }
    }

    Ok(())
}
