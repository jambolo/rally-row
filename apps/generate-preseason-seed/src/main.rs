use anyhow::Result;
use clap::Parser;
use rating_core::{build_seed, load_history, load_league_config, lock, write_json};
use std::path::PathBuf;

#[derive(Parser)]
#[command(about = "Calculate historical Elo and save independent preseason priors")]
struct Args {
    /// League id; the configuration is read from `<config-dir>/<league>.json`.
    #[arg(long)]
    league: String,
    /// Directory containing `<league>.json` league configurations.
    #[arg(long, default_value = "config")]
    config_dir: PathBuf,
    #[arg(long, default_value = "data")]
    data_dir: PathBuf,
    #[arg(long)]
    target_season: Option<i32>,
}
fn main() -> Result<()> {
    let args = Args::parse();
    let (cfg, config_bytes) = load_league_config(&args.config_dir, &args.league)?;
    let season = args.target_season.unwrap_or(cfg.current_season());
    let dir = args.data_dir.join(&cfg.id);
    let _lock = lock(&dir.join("elo.lock"))?;
    let (history, history_bytes) = load_history(&args.data_dir, &cfg)?;
    let (seed, audit) = build_seed(&history, &history_bytes, &cfg, &config_bytes, season)?;
    let path = dir.join(format!("elo-{season}.json"));
    let audit_path = dir.join(format!("elo-audit-{season}.json"));
    write_json(&audit_path, &audit)?;
    write_json(&path, &seed)?;
    println!(
        "Saved {} team priors for {season} from {} completed games ({} ties) to {}, and their replay to {}",
        seed.ratings.len(),
        seed.completed_games,
        seed.tied_games,
        path.display(),
        audit_path.display()
    );
    Ok(())
}
