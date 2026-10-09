use anyhow::Result;
use chrono::Utc;
use clap::Parser;
use rating_core::{
    SIMULATED_SEASONS_SCHEMA_VERSION, SimulatedSeasons, digest, load_history, load_league_config, lock,
    tuning::{Split, validate},
    walk_forward::predict_season,
    write_json,
};
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
    let generated_at = Utc::now().to_rfc3339();
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
    validate(&history, &cfg, split)?;
    let (first, last) = (split.tune_end + 1, split.test_end);
    eprintln!("Simulating held-out seasons {first}–{last} with the configured settings...");
    let seasons = (first..=last)
        .map(|season| {
            let predicted = predict_season(&history, &cfg, season)?;
            eprintln!("Predicted {} games in season {season}", predicted.games.len());
            Ok(predicted)
        })
        .collect::<Result<Vec<_>>>()?;
    let games: usize = seasons.iter().map(|s| s.games.len()).sum();
    let predictions = SimulatedSeasons {
        schema_version: SIMULATED_SEASONS_SCHEMA_VERSION,
        league: cfg.id.clone(),
        generated_at,
        history_sha256: digest(&history_bytes),
        config_sha256: digest(&config_bytes),
        split,
        elo_settings: cfg.elo.clone(),
        bayesian_settings: cfg.bayesian.clone(),
        seasons,
    };
    let path = dir.join("simulated-seasons.json");
    write_json(&path, &predictions)?;
    println!(
        "Saved predictions for {games} games in seasons {first}–{last} to {}",
        path.display()
    );
    Ok(())
}
