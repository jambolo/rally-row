use anyhow::Result;
use clap::Parser;
use rating_core::{SimulatedSeasons, load_league_config, save_report};
use std::{io::Write, path::PathBuf};

mod evaluation;

#[derive(Parser)]
#[command(about = "Score the held-out Bayesian and in-season Elo predictions saved by simulate-season")]
struct Args {
    /// League id; the configuration is read from `<config-dir>/<league>.json`.
    #[arg(long)]
    league: String,
    /// Directory containing `<league>.json` league configurations.
    #[arg(long, default_value = "config")]
    config_dir: PathBuf,
    /// Reads `<data-dir>/<league>/simulated-seasons.json` and checks it against `<data-dir>/<league>/history.json`.
    #[arg(long, default_value = "data")]
    data_dir: PathBuf,
    /// Also save model-evaluation-report-<league>-<UTC date>.json in this directory.
    #[arg(long)]
    report_dir: Option<PathBuf>,
    /// Print the JSON report to stdout instead of the summary.
    #[arg(long)]
    json: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let (cfg, config_bytes) = load_league_config(&args.config_dir, &args.league)?;
    let simulated = SimulatedSeasons::load(&args.data_dir, &cfg, &config_bytes)?;
    let report = evaluation::run(&cfg, simulated)?;
    if let Some(dir) = args.report_dir {
        let path = save_report(&dir, "model-evaluation-report", &report.league, &report.run_at, &report)?;
        eprintln!("Saved report to {}", path.display());
    }
    let mut stdout = std::io::stdout().lock();
    if args.json {
        serde_json::to_writer_pretty(&mut stdout, &report)?;
        writeln!(stdout)?;
    } else {
        stdout.write_all(evaluation::summary(&report)?.as_bytes())?;
    }
    Ok(())
}
