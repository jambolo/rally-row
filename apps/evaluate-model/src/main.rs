use anyhow::Result;
use clap::Parser;
use rating_core::{digest, load_history, load_league_config, save_report, tuning::Split};
use std::{io::Write, path::PathBuf};

mod evaluation;

#[derive(Parser)]
#[command(about = "Score Bayesian and in-season Elo predictions, made with the configured settings, on held-out seasons")]
struct Args {
    /// League id; the configuration is read from `<config-dir>/<league>.json`.
    #[arg(long)]
    league: String,
    /// Directory containing `<league>.json` league configurations.
    #[arg(long, default_value = "config")]
    config_dir: PathBuf,
    #[arg(long, default_value = "data")]
    data_dir: PathBuf,
    /// Also save model-evaluation-report-<league>-<UTC date>.json in this directory.
    #[arg(long)]
    report_dir: Option<PathBuf>,
    /// First tuning season (validated only); defaults to bayes_tune.tune_start, then elo_tune.tune_start.
    #[arg(long)]
    tune_start: Option<i32>,
    /// Last tuning season; held-out seasons follow it. Defaults to bayes_tune.tune_end, then elo_tune.tune_end.
    #[arg(long)]
    tune_end: Option<i32>,
    /// Final held-out season; defaults to bayes_tune.test_end, then elo_tune.test_end.
    #[arg(long)]
    test_end: Option<i32>,
    /// Print the JSON report to stdout instead of the summary.
    #[arg(long)]
    json: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let (cfg, config_bytes) = load_league_config(&args.config_dir, &args.league)?;
    let split = Split::resolve(
        &cfg,
        args.tune_start,
        args.tune_end,
        args.test_end,
        &[("bayes_tune", cfg.bayes_tune.as_ref()), ("elo_tune", cfg.elo_tune.as_ref())],
    )?;
    let (history, history_bytes) = load_history(&args.data_dir, &cfg)?;
    let report = evaluation::run(&history, &cfg, split, digest(&config_bytes), digest(&history_bytes))?;
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
