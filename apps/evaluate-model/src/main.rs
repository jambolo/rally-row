use anyhow::{Context, Result};
use clap::Parser;
use rating_core::{GameFile, digest, load_league_config, tuning::Split, write_json};
use std::{fs, io::Write, path::PathBuf};

mod evaluation;
#[cfg(test)]
#[path = "../tests/common/mod.rs"]
mod fixtures;

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
    let defaults = cfg.bayes_tune.as_ref().or(cfg.elo_tune.as_ref());
    let split = Split {
        warmup_start: cfg.history_start,
        tune_start: args
            .tune_start
            .or(defaults.map(|s| s.tune_start))
            .context("Set bayes_tune.tune_start (or elo_tune.tune_start) or pass --tune-start")?,
        tune_end: args
            .tune_end
            .or(defaults.map(|s| s.tune_end))
            .context("Set bayes_tune.tune_end (or elo_tune.tune_end) or pass --tune-end")?,
        test_end: args
            .test_end
            .or(defaults.map(|s| s.test_end))
            .context("Set bayes_tune.test_end (or elo_tune.test_end) or pass --test-end")?,
    };
    let history_bytes =
        fs::read(args.data_dir.join(&cfg.id).join("history.json")).context("Read history.json; run history-importer first")?;
    let history: GameFile = serde_json::from_slice(&history_bytes)
        .context("Parse history.json; start_time_utc, numeric round, and round_label are required; rerun history-importer")?;
    let report = evaluation::run(&history, &cfg, split, digest(&config_bytes), digest(&history_bytes))?;
    if let Some(dir) = args.report_dir {
        let run_at = chrono::DateTime::parse_from_rfc3339(&report.run_at)?.with_timezone(&chrono::Utc);
        let path = dir.join(format!(
            "model-evaluation-report-{}-{}.json",
            report.league,
            run_at.format("%Y-%m-%d")
        ));
        write_json(&path, &report).with_context(|| format!("Write report {}", path.display()))?;
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
