use anyhow::{Context, Result, bail, ensure};
use clap::Parser;
use rating_core::{
    GameFile, HISTORY_SCHEMA_VERSION, LeagueConfig, adapter_for, fetch_source, load_history, load_league_config, lock,
    parse_documents, validate_games, write_json,
};
use std::{
    fs,
    path::{Path, PathBuf},
};

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
    /// Download every season, including those the saved history.json already holds.
    #[arg(long, conflicts_with = "input")]
    refresh: bool,
}

/// The saved history when its seasons can be kept: it is current for `cfg`, its games still validate, and it ends no
/// later than `through`.
fn reusable(data_dir: &Path, cfg: &LeagueConfig, through: i32) -> Option<GameFile> {
    let (mut history, _) = load_history(data_dir, cfg).ok()?;
    let current = history.schema_version == HISTORY_SCHEMA_VERSION
        && history.league == cfg.id
        && history.source_url == cfg.source.url
        && history.teams_sha256 == cfg.teams_sha256()
        && history.from_season == cfg.history_start
        && history.through_season <= through;
    (current && validate_games(&mut history.games, cfg).is_ok()).then_some(history)
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
    let path = dir.join("history.json");
    let adapter = adapter_for(&cfg.source.kind)?;
    // Completed seasons rarely change, so a download skips the seasons the saved file already holds.
    let saved = (args.input.is_empty() && !args.refresh)
        .then(|| reusable(&args.data_dir, &cfg, through))
        .flatten();
    let first = saved.as_ref().map_or(cfg.history_start, |h| h.through_season + 1);
    if first > through {
        println!(
            "{} already holds seasons {}–{through}; nothing to download",
            path.display(),
            cfg.history_start
        );
        return Ok(());
    }
    if saved.is_some() {
        eprintln!(
            "Keeping seasons {}–{} from {}; downloading {first}–{through}",
            cfg.history_start,
            first - 1,
            path.display()
        );
    }
    let documents = if args.input.is_empty() {
        adapter
            .history_urls(&cfg, first, through)
            .iter()
            .map(|url| fetch_source(url))
            .collect::<Result<Vec<_>>>()?
    } else {
        args.input
            .iter()
            .map(|p| fs::read_to_string(p).context("Read source file"))
            .collect::<Result<Vec<_>>>()?
    };
    let mut games = saved.map(|h| h.games).unwrap_or_default();
    games.extend(
        parse_documents(&documents, &cfg, |warning| eprintln!("Warning: {warning}"))?
            .into_iter()
            .filter(|g| g.season >= first && g.season <= through),
    );
    validate_games(&mut games, &cfg)?;
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
        source_url: cfg.source.url.clone(),
        from_season: cfg.history_start,
        through_season: through,
        teams_sha256: cfg.teams_sha256(),
        games,
    };
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
