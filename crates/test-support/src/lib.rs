//! Fixtures and command-line checks shared by the workspace's tests.

pub mod cli;

use rating_core::{BayesianSettings, EloTuneSettings, Game, GameFile, HISTORY_SCHEMA_VERSION, LeagueConfig, Outcome};
use serde_json::Value;
use std::{fs, ops::RangeInclusive, path::Path};

/// The repository's league configuration directory.
pub const CONFIG_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../config");

/// Header row of an nflverse `games.csv` source document.
pub const NFLVERSE_HEADER: &str =
    "game_id,season,game_type,week,gameday,gametime,away_team,away_score,home_team,home_score,location\n";

/// The committed `config/<league>.json`.
pub fn league_config(league: &str) -> LeagueConfig {
    serde_json::from_value(league_config_json(league)).unwrap()
}

/// The committed `config/<league>.json` as raw JSON, for tests that edit fields before parsing.
pub fn league_config_json(league: &str) -> Value {
    serde_json::from_slice(&fs::read(Path::new(CONFIG_DIR).join(format!("{league}.json"))).unwrap()).unwrap()
}

/// A current-schema history file for `cfg` covering `seasons`, fetched the March after the last one.
pub fn history_file(cfg: &LeagueConfig, seasons: RangeInclusive<i32>, games: Vec<Game>) -> GameFile {
    GameFile {
        schema_version: HISTORY_SCHEMA_VERSION,
        league: cfg.id.clone(),
        fetched_at: format!("{}-03-01T00:00:00Z", seasons.end() + 1),
        source_url: cfg.source.url.clone(),
        from_season: *seasons.start(),
        through_season: *seasons.end(),
        teams: cfg.teams.clone(),
        games,
    }
}

/// One game repeated every fixture season: UTC "MM-DDTHH:MM:SSZ" start, home, away, phase, round, result.
type Slot = (&'static str, &'static str, &'static str, &'static str, u32, Outcome);

/// The NFL configuration narrowed to ARI, ATL and BAL without aliases, with an offline canonical-json source and an
/// `elo_tune` split that tunes 2003–2004 and holds out the seasons after it. Every season plays `schedule`;
/// postseason games are neutral-site Super Bowls, and game ids are `<season>-<position in schedule>`.
fn fixture(seasons: RangeInclusive<i32>, schedule: &[Slot]) -> (LeagueConfig, GameFile) {
    let mut cfg = league_config("nfl");
    cfg.teams.retain(|t| ["ARI", "ATL", "BAL"].contains(&t.id.as_str()));
    cfg.aliases.clear();
    cfg.source.kind = "canonical-json".into();
    cfg.source.url = "https://invalid.example.test/no-network-needed".into();
    cfg.bayes_tune = None;
    cfg.elo_tune = Some(EloTuneSettings {
        tune_start: 2003,
        tune_end: 2004,
        test_end: *seasons.end(),
    });
    let games = seasons
        .clone()
        .flat_map(|season| {
            schedule
                .iter()
                .enumerate()
                .map(move |(i, (start, home, away, phase, round, result))| Game {
                    id: format!("{season}-{}", i + 1),
                    league: "nfl".into(),
                    season,
                    start_time_utc: format!("{season}-{start}").parse().unwrap(),
                    phase: (*phase).into(),
                    round_label: if *phase == "postseason" { "SB" } else { "REG" }.into(),
                    round: *round,
                    home_team: (*home).into(),
                    away_team: (*away).into(),
                    home_source_id: (*home).into(),
                    away_source_id: (*away).into(),
                    neutral: *phase == "postseason",
                    result: Some(result.clone()),
                })
        })
        .collect();
    let history = history_file(&cfg, seasons, games);
    (cfg, history)
}

/// ARI hosts ATL four times a season, 2002–2005: a win, a regular-season tie, a loss, and a neutral-site Super Bowl win.
/// Tuning covers 2003–2004 and 2005 is held out. Bayesian settings sit inside the default `bayes-tune` grid.
pub fn tuning_fixture() -> (LeagueConfig, GameFile) {
    let (mut cfg, history) = fixture(
        2002..=2005,
        &[
            ("09-07T17:00:00Z", "ARI", "ATL", "regular", 1, Outcome::HomeWin),
            ("09-14T17:00:00Z", "ARI", "ATL", "regular", 2, Outcome::Tie),
            ("09-21T17:00:00Z", "ARI", "ATL", "regular", 3, Outcome::AwayWin),
            ("09-28T17:00:00Z", "ARI", "ATL", "postseason", 4, Outcome::HomeWin),
        ],
    );
    cfg.bayesian = BayesianSettings {
        prior_sd_elo: 150.0,
        tie_prior_games: 100.0,
        tie_prior_rate: 0.005,
    };
    (cfg, history)
}

/// Three teams, 2002–2006, five games a season: a same-UTC-day doubleheader, a regular-season tie, and a neutral
/// postseason game. Tuning covers 2003–2004 and 2005–2006 are held out.
pub fn evaluation_fixture() -> (LeagueConfig, GameFile) {
    fixture(
        2002..=2006,
        &[
            ("09-07T17:00:00Z", "ARI", "ATL", "regular", 1, Outcome::HomeWin),
            ("09-07T21:00:00Z", "ATL", "ARI", "regular", 1, Outcome::AwayWin),
            ("09-14T17:00:00Z", "BAL", "ARI", "regular", 2, Outcome::Tie),
            ("09-21T17:00:00Z", "ATL", "BAL", "regular", 3, Outcome::HomeWin),
            ("09-28T17:00:00Z", "ARI", "BAL", "postseason", 4, Outcome::AwayWin),
        ],
    )
}
