use rating_core::*;
use test_support::{NFLVERSE_HEADER, history_file, league_config};

fn config() -> LeagueConfig {
    league_config("nfl")
}
fn csv() -> String {
    format!(
        "{NFLVERSE_HEADER}a,2002,REG,1,2002-09-01,13:00,SD,10,OAK,10,Neutral\nb,2002,REG,2,2002-09-08,,STL,,SEA,,Home\nc,2002,PRE,1,2002-08-01,13:00,SEA,7,SF,3,Home\n"
    )
}
#[test]
fn normalizes_franchises_ties_and_unplayed_games() {
    let games = parse_source(&csv(), &config()).unwrap();
    assert_eq!(games.len(), 2);
    assert_eq!(games[0].home_team, "LV");
    assert_eq!(games[0].away_team, "LAC");
    assert_eq!(games[0].result, Some(Outcome::Tie));
    assert_eq!(games[0].round, 1);
    assert_eq!(games[0].round_label, "REG");
    assert!(games[0].neutral);
    assert_eq!(games[1].away_team, "LAR");
    assert_eq!(games[1].result, None);
    assert_eq!(games[1].round, 2);
}
#[test]
fn rejects_duplicate_games_bad_dates_and_one_missing_score() {
    let cfg = config();
    let mut games = parse_source(&csv(), &cfg).unwrap();
    games.push(games[0].clone());
    assert!(validate_games(&mut games, &cfg).is_err());
    assert!(parse_source(&csv().replace("OAK,10", "OAK,"), &cfg).is_err());
    assert!(parse_source(&csv().replace("2002-09-01", "2002-02-30"), &cfg).is_err());
}
#[test]
fn seeds_are_chronological_tie_aware_and_regressed_exactly_once() {
    let mut cfg = config();
    cfg.elo.k = 20.0;
    cfg.elo.offseason_regression = 1.0 / 3.0;
    let mut games = parse_source(&csv(), &cfg).unwrap();
    games[0].result = Some(Outcome::HomeWin);
    let history = history_file(&cfg, 2002..=2002, games.clone());
    let bytes = serde_json::to_vec(&history).unwrap();
    let seed = build_seed(&history, &bytes, &cfg, b"config", 2003).unwrap();
    let lv = seed.ratings.iter().find(|t| t.team == "LV").unwrap();
    assert!((lv.elo - (cfg.elo.initial + 10.0 * 2.0 / 3.0)).abs() < 1e-9);
    assert_eq!(seed.completed_games, 1); // Scheduled game is not an observation.
    assert_eq!(seed.audit[0].home_before, cfg.elo.initial);
    assert_eq!(seed.audit[0].home_after, cfg.elo.initial + 10.0);
    assert!(seed.tie_weight > 0.0);
    let mut reversed = history.clone();
    reversed.games.reverse();
    let again = build_seed(&reversed, &bytes, &cfg, b"config", 2003).unwrap();
    assert_eq!(seed.ratings[0].elo, again.ratings[0].elo);
    assert!(build_seed(&history, &bytes, &cfg, b"config", 2004).is_err());
    let mut obsolete = history.clone();
    for version in [1, 2] {
        obsolete.schema_version = version;
        assert!(
            build_seed(&obsolete, &bytes, &cfg, b"config", 2003)
                .unwrap_err()
                .to_string()
                .contains("rerun history-importer")
        );
    }
    let mut leaked = history;
    leaked.games[0].season = 2003;
    assert!(build_seed(&leaked, &bytes, &cfg, b"config", 2003).is_err());
}
#[test]
fn postseason_ties_are_invalid() {
    let cfg = config();
    let mut games = parse_source(&csv(), &cfg).unwrap();
    games[0].phase = "postseason".into();
    assert!(validate_games(&mut games, &cfg).is_err());
}
#[test]
fn generic_json_adapter_has_no_nfl_team_count_dependency() {
    let mut cfg = config();
    cfg.id = "demo".into();
    cfg.source.kind = "canonical-json".into();
    let mut games = parse_source(&csv(), &config()).unwrap();
    for g in &mut games {
        g.league = "demo".into();
    }
    let source_games: Vec<_> = games
        .into_iter()
        .map(|g| {
            let mut row = serde_json::to_value(g).unwrap();
            row["date"] = serde_json::json!("2002-09-01");
            row["time"] = serde_json::json!("13:00");
            row["timezone"] = serde_json::json!("America/New_York");
            row
        })
        .collect();
    let mut input = serde_json::json!({"schema_version":2,"league":"demo","games":source_games});
    assert_eq!(parse_source(&input.to_string(), &cfg).unwrap().len(), 2);
    input["games"][0]["round"] = serde_json::json!(0);
    assert!(parse_source(&input.to_string(), &cfg).is_err());
    input["games"][0]["round"] = serde_json::json!(1);
    input["schema_version"] = serde_json::json!(1);
    assert!(parse_source(&input.to_string(), &cfg).is_err());
}

#[test]
fn identity_ranges_track_names_and_locations() {
    let cfg = config();
    assert_eq!(cfg.identity("LV", 2019).unwrap().name, "Oakland Raiders");
    assert_eq!(cfg.identity("LV", 2020).unwrap().name, "Las Vegas Raiders");
    assert_eq!(cfg.identity("LAC", 2016).unwrap().location, "San Diego");
    assert_eq!(cfg.identity("LAC", 2017).unwrap().location, "Los Angeles");
    assert_eq!(cfg.identity("WAS", 2021).unwrap().name, "Washington Football Team");
    assert_eq!(cfg.identity("WAS", 2022).unwrap().name, "Washington Commanders");
    let mut bad = cfg.clone();
    bad.teams.iter_mut().find(|t| t.id == "LV").unwrap().eras[0].through_season = Some(2020);
    assert!(bad.validate().is_err());
}

#[test]
fn relocation_preserves_one_rating_history_and_original_source_ids() {
    let mut cfg = config();
    cfg.history_start = 2019;
    let input = &format!(
        "{NFLVERSE_HEADER}a,2019,REG,1,2019-09-01,13:00,SEA,10,OAK,20,Neutral\nb,2020,REG,1,2020-09-01,13:00,SEA,10,LV,20,Neutral\n"
    );
    let games = parse_source(input, &cfg).unwrap();
    assert_eq!(games[0].home_team, games[1].home_team);
    assert_eq!(games[0].home_source_id, "OAK");
    assert_eq!(games[1].home_source_id, "LV");
    let history = history_file(&cfg, 2019..=2020, games);
    let output = build_seed(&history, b"history", &cfg, b"config", 2021).unwrap();
    let team = output.ratings.iter().find(|t| t.team == "LV").unwrap();
    assert_eq!(team.games, 2);
    assert!(team.elo > cfg.elo.initial);
    assert!(output.audit[1].home_before > cfg.elo.initial); // Rename did not reset the rating.
    assert!(parse_source(&input.replace("OAK", "LV"), &cfg).is_err());
}

#[test]
fn elo_replay_scores_before_updates_and_regresses_only_at_boundaries() {
    let mut cfg = config();
    cfg.elo.k = 20.0;
    cfg.elo.offseason_regression = 0.5;
    let mut games = parse_source(&csv(), &cfg).unwrap();
    games[0].result = Some(Outcome::HomeWin);
    let mut next = games[0].clone();
    next.id = "next-season".into();
    next.season = 2003;
    next.start_time_utc = "2003-09-01T17:00:00Z".parse().unwrap();
    next.result = Some(Outcome::Tie);
    games.insert(0, next);
    let replay = replay_elo(&games, &cfg, 2003).unwrap();
    assert_eq!(replay.audit.len(), 2);
    assert_eq!(replay.audit[0].expected_home_score, 0.5);
    assert_eq!(replay.audit[0].home_after, cfg.elo.initial + 10.0);
    assert_eq!(replay.audit[1].home_before, cfg.elo.initial + 5.0);
    assert_eq!(replay.audit[1].away_before, cfg.elo.initial - 5.0);
    assert_eq!(replay.audit[1].observed_home_score, 0.5);
    let expected = 1.0 / (1.0 + 10_f64.powf(-10.0 / 400.0));
    assert!((replay.audit[1].expected_home_score - expected).abs() < 1e-12);
    assert!((replay.audit[1].home_after - (cfg.elo.initial + 5.0 + 20.0 * (0.5 - expected))).abs() < 1e-12);
    let lv = replay.ratings.iter().find(|r| r.team == "LV").unwrap();
    assert_eq!(lv.elo, replay.audit[1].home_after);
    assert_eq!(lv.games, 2);
    let before = serde_json::to_value(&replay_elo(&games, &cfg, 2002).unwrap().audit).unwrap();
    games[0].result = Some(Outcome::AwayWin);
    let after = serde_json::to_value(&replay_elo(&games, &cfg, 2002).unwrap().audit).unwrap();
    assert_eq!(before, after);
    assert!(replay_elo(&games, &cfg, 2004).is_err());
}
