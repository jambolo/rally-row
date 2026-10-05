use rating_core::{EloTuneSettings, Game, GameFile, HISTORY_SCHEMA_VERSION, LeagueConfig, Outcome};

/// Three teams, 2002–2006, five games a season: a same-UTC-day doubleheader, a regular-season tie, and a neutral
/// postseason game. Holdout defaults to 2005–2006.
pub fn fixture() -> (LeagueConfig, GameFile) {
    let mut cfg: LeagueConfig = serde_json::from_str(include_str!("../../../../config/nfl.json")).unwrap();
    cfg.teams.retain(|t| ["ARI", "ATL", "BAL"].contains(&t.id.as_str()));
    cfg.aliases.clear();
    cfg.source.kind = "canonical-json".into();
    cfg.source.url = "https://invalid.example.test/no-network-needed".into();
    cfg.bayes_tune = None;
    cfg.elo_tune = Some(EloTuneSettings {
        tune_start: 2003,
        tune_end: 2004,
        test_end: 2006,
    });
    let schedule = [
        ("09-07T17:00:00Z", "ARI", "ATL", "regular", 1, Outcome::HomeWin),
        ("09-07T21:00:00Z", "ATL", "ARI", "regular", 1, Outcome::AwayWin),
        ("09-14T17:00:00Z", "BAL", "ARI", "regular", 2, Outcome::Tie),
        ("09-21T17:00:00Z", "ATL", "BAL", "regular", 3, Outcome::HomeWin),
        ("09-28T17:00:00Z", "ARI", "BAL", "postseason", 4, Outcome::AwayWin),
    ];
    let games = (2002..=2006)
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
    let history = GameFile {
        schema_version: HISTORY_SCHEMA_VERSION,
        league: cfg.id.clone(),
        fetched_at: "2007-03-01T00:00:00Z".into(),
        source_url: cfg.source.url.clone(),
        from_season: 2002,
        through_season: 2006,
        teams: cfg.teams.clone(),
        games,
    };
    (cfg, history)
}
