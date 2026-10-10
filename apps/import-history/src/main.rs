use anyhow::{Context, Result, bail, ensure};
use clap::Parser;
use rating_core::{
    GameFile, HISTORY_SCHEMA_VERSION, adapter_for, fetch_source, load_league_config, lock, parse_documents, write_json,
};
use std::{fs, path::PathBuf};

#[derive(Parser)]
#[command(about = "Download and normalize completed historical seasons; never write current-season data")]
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
    through_season: Option<i32>,
    /// Read already-downloaded provider files instead of making network requests; per-season sources take one file per season.
    #[arg(long, num_args = 1..)]
    input: Vec<PathBuf>,
}
fn main() -> Result<()> {
    let args = Args::parse();
    let (cfg, _) = load_league_config(&args.config_dir, &args.league)?;
    let through = args.through_season.unwrap_or(cfg.current_season() - 1);
    ensure!(
        through >= cfg.history_start && through < cfg.current_season(),
        "Historical range must end before the current season"
    );
    let dir = args.data_dir.join(&cfg.id);
    let _lock = lock(&dir.join("history.lock"))?;
    let adapter = adapter_for(&cfg.source.kind)?;
    let documents = if args.input.is_empty() {
        adapter
            .history_urls(&cfg, cfg.history_start, through)
            .iter()
            .map(|url| fetch_source(url))
            .collect::<Result<Vec<_>>>()?
    } else {
        args.input
            .iter()
            .map(|p| fs::read_to_string(p).context("Read source file"))
            .collect::<Result<Vec<_>>>()?
    };
    let games = parse_documents(&documents, &cfg, |warning| eprintln!("Warning: {warning}"))?
        .into_iter()
        .filter(|g| g.season >= cfg.history_start && g.season <= through)
        .collect::<Vec<_>>();
    for year in cfg.history_start..=through {
        ensure!(
            games.iter().any(|g| g.season == year && g.result.is_some()),
            "Source missing completed season {year}; previous file has been kept"
        );
        if let Some(reason) = adapter.season_incomplete(&games, year) {
            bail!("Season {year} has {reason}; previous file has been kept");
        }
    }
    let file = GameFile {
        schema_version: HISTORY_SCHEMA_VERSION,
        league: cfg.id.clone(),
        fetched_at: chrono::Utc::now().to_rfc3339(),
        source_url: cfg.source.url,
        from_season: cfg.history_start,
        through_season: through,
        teams: cfg.teams.clone(),
        games,
    };
    let path = dir.join("history.json");
    write_json(&path, &file)?;
    println!(
        "Saved {} games, seasons {}–{}, to {}",
        file.games.len(),
        file.from_season,
        file.through_season,
        path.display()
    );
    Ok(())
}
