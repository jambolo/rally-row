use anyhow::Result;
use clap::Parser;
use rating_core::{digest, load_history, load_league_config, save_report, tuning::Split};
use std::{io::Write, path::PathBuf};

mod tuning;

#[derive(Parser)]
#[command(about = "Tune Elo K, home advantage, and offseason regression; report results")]
struct Args {
    /// League id; the configuration is read from `<config-dir>/<league>.json`.
    #[arg(long)]
    league: String,
    /// Directory containing `<league>.json` league configurations.
    #[arg(long, default_value = "config")]
    config_dir: PathBuf,
    #[arg(long, default_value = "data")]
    data_dir: PathBuf,
    /// Also save elo-tuning-report-<league>-<UTC date>.json in this directory.
    #[arg(long)]
    report_dir: Option<PathBuf>,
    /// First scored tuning season; defaults to config elo_tune.tune_start.
    #[arg(long)]
    tune_start: Option<i32>,
    /// Last scored tuning season; defaults to config elo_tune.tune_end.
    #[arg(long)]
    tune_end: Option<i32>,
    /// Final held-out season; defaults to config elo_tune.test_end.
    #[arg(long)]
    test_end: Option<i32>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let (cfg, config_bytes) = load_league_config(&args.config_dir, &args.league)?;
    let split = Split::resolve(
        &cfg,
        args.tune_start,
        args.tune_end,
        args.test_end,
        &[("elo_tune", cfg.elo_tune.as_ref())],
    )?;
    let (history, history_bytes) = load_history(&args.data_dir, &cfg)?;
    let report = tuning::run(&history, &cfg, split, digest(&config_bytes), digest(&history_bytes))?;
    if let Some(dir) = args.report_dir {
        let path = save_report(&dir, "elo-tuning-report", &report.league, &report.run_at, &report)?;
        eprintln!("Saved report to {}", path.display());
    }
    let mut stdout = std::io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, &report)?;
    writeln!(stdout)?;
    Ok(())
}
