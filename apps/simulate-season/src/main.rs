use anyhow::Result;
use clap::Parser;
use rating_core::{SimulatedSeasons, load_history, load_league_config, lock, tuning::Split, write_json};
use std::path::PathBuf;

#[derive(Parser)]
#[command(about = "Simulate the held-out seasons with the configured settings and save every game's pregame predictions")]
struct Args {
    /// League id; the configuration is read from `<config-dir>/<league>.json`.
    #[arg(long)]
    league: String,
    /// Directory containing `<league>.json` league configurations.
    #[arg(long, default_value = "config")]
    config_dir: PathBuf,
    /// Reads `<data-dir>/<league>/history.json` and writes `<data-dir>/<league>/simulated-seasons.json`.
    #[arg(long, default_value = "data")]
    data_dir: PathBuf,
    /// First tuning season (validated only); defaults to bayes_tune.tune_start, then elo_tune.tune_start.
    #[arg(long)]
    tune_start: Option<i32>,
    /// Last tuning season; held-out seasons follow it. Defaults to bayes_tune.tune_end, then elo_tune.tune_end.
    #[arg(long)]
    tune_end: Option<i32>,
    /// Final held-out season; defaults to bayes_tune.test_end, then elo_tune.test_end.
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
        &[("bayes_tune", cfg.bayes_tune.as_ref()), ("elo_tune", cfg.elo_tune.as_ref())],
    )?;
    let dir = args.data_dir.join(&cfg.id);
    let _lock = lock(&dir.join("simulated-seasons.lock"))?;
    let (history, history_bytes) = load_history(&args.data_dir, &cfg)?;
    let (first, last) = (split.tune_end + 1, split.test_end);
    eprintln!("Simulating held-out seasons {first}–{last} with the configured settings...");
    let simulated = SimulatedSeasons::simulate(&history, &history_bytes, &cfg, &config_bytes, split, |predicted| {
        eprintln!("Predicted {} games in season {}", predicted.games.len(), predicted.season);
    })?;
    let games: usize = simulated.seasons.iter().map(|s| s.games.len()).sum();
    let path = dir.join("simulated-seasons.json");
    write_json(&path, &simulated)?;
    println!(
        "Saved predictions for {games} games in seasons {first}–{last} to {}",
        path.display()
    );
    Ok(())
}
