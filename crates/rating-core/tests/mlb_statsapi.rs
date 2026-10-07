use rating_core::{Game, LeagueConfig, Outcome, adapter_for, load_league_config, parse_documents};
use serde_json::{Value, json};
use std::path::Path;
use test_support::{CONFIG_DIR, league_config};

fn mlb() -> LeagueConfig {
    league_config("mlb")
}

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/mlb-statsapi.json")).unwrap()
}

/// Each fixture document re-serialized to one JSON string, in array order.
fn fixture_documents() -> Vec<String> {
    fixture()["documents"]
        .as_array()
        .unwrap()
        .iter()
        .map(Value::to_string)
        .collect()
}

fn parse_fixture() -> (Vec<Game>, Vec<String>) {
    let mut warnings = Vec::new();
    let games = parse_documents(&fixture_documents(), &mlb(), |w| warnings.push(w)).unwrap();
    (games, warnings)
}

fn find<'a>(games: &'a [Game], id: &str) -> Vec<&'a Game> {
    games.iter().filter(|g| g.id == id).collect()
}

/// One schedule entry; `None` scores or tie flag omit the key, as the API does.
fn entry(
    pk: u64,
    game_type: &str,
    detailed: &str,
    home: (u32, Option<u32>),
    away: (u32, Option<u32>),
    is_tie: Option<bool>,
) -> Value {
    let side = |(id, score): (u32, Option<u32>)| match score {
        Some(s) => json!({"score": s, "team": {"id": id, "name": "Team"}}),
        None => json!({"team": {"id": id, "name": "Team"}}),
    };
    let mut e = json!({
        "gamePk": pk,
        "gameType": game_type,
        "season": "2025",
        "gameDate": "2025-10-25T00:08:00Z",
        "officialDate": "2025-10-24",
        "status": {"abstractGameState": "Final", "codedGameState": "F", "detailedState": detailed, "statusCode": "F", "startTimeTBD": false},
        "teams": {"away": side(away), "home": side(home)},
    });
    if let Some(t) = is_tie {
        e["isTie"] = json!(t);
    }
    e
}

fn parse_entries(entries: Vec<Value>) -> anyhow::Result<Vec<Game>> {
    let document = json!({"dates": [{"date": "2025-10-24", "games": entries}]}).to_string();
    parse_documents(&[document], &mlb(), |_| {})
}

fn parse_error(entries: Vec<Value>) -> String {
    parse_entries(entries).unwrap_err().to_string()
}

#[test]
fn fixture_parses_to_the_expected_games() {
    let (games, warnings) = parse_fixture();
    assert_eq!(serde_json::to_value(&games).unwrap(), fixture()["expected"]);
    assert_eq!(fixture()["league"], "mlb");
    assert_eq!(warnings, vec!["Game 4207: final status without scores; ignored".to_owned()]);
}

#[test]
fn duplicate_listings_collapse_to_one_game() {
    let (games, _) = parse_fixture();
    let postponed_then_final = find(&games, "776691");
    assert_eq!(postponed_then_final.len(), 1);
    assert_eq!(postponed_then_final[0].result, Some(Outcome::HomeWin));
    let resumed = find(&games, "776907");
    assert_eq!(resumed.len(), 1);
    assert_eq!(resumed[0].start_time_utc.to_rfc3339(), "2025-08-03T17:05:00+00:00");
    assert_eq!(resumed[0].result, Some(Outcome::AwayWin));
}

#[test]
fn completed_early_is_final() {
    let (games, _) = parse_fixture();
    for id in ["778370", "900004"] {
        let game = find(&games, id);
        assert_eq!(game.len(), 1, "{id}");
        assert!(game[0].result.is_some(), "{id}");
    }
}

#[test]
fn unplayed_and_excluded_listings_are_not_games() {
    let (games, _) = parse_fixture();
    for id in ["449187", "449246", "4207", "900001", "900002", "900003"] {
        assert!(find(&games, id).is_empty(), "{id}");
    }
}

#[test]
fn suspended_and_scheduled_games_have_no_result() {
    let (games, _) = parse_fixture();
    for id in ["900005", "900006"] {
        let game = find(&games, id);
        assert_eq!(game.len(), 1, "{id}");
        assert_eq!(game[0].result, None, "{id}");
    }
    assert_eq!(
        find(&games, "900005")[0].start_time_utc.to_rfc3339(),
        "2026-05-16T17:10:00+00:00"
    );
}

#[test]
fn regular_season_tie_is_a_tie() {
    let (games, _) = parse_fixture();
    let tie = find(&games, "449244");
    assert_eq!(tie.len(), 1);
    assert_eq!(tie[0].phase, "regular");
    assert_eq!(tie[0].result, Some(Outcome::Tie));
}

#[test]
fn identity_events_map_to_their_franchises() {
    let cfg = mlb();
    let (games, _) = parse_fixture();
    let cases = [
        ("900011", "120", 2004, "WSH", "Montreal Expos"),
        ("900012", "120", 2005, "WSH", "Washington Nationals"),
        ("900013", "108", 2004, "LAA", "Anaheim Angels"),
        ("900014", "108", 2005, "LAA", "Los Angeles Angels of Anaheim"),
        ("900015", "108", 2015, "LAA", "Los Angeles Angels of Anaheim"),
        ("900016", "108", 2016, "LAA", "Los Angeles Angels"),
        ("900017", "139", 2007, "TB", "Tampa Bay Devil Rays"),
        ("900018", "139", 2008, "TB", "Tampa Bay Rays"),
        ("900019", "146", 2011, "MIA", "Florida Marlins"),
        ("900020", "146", 2012, "MIA", "Miami Marlins"),
        ("900021", "114", 2021, "CLE", "Cleveland Indians"),
        ("900022", "114", 2022, "CLE", "Cleveland Guardians"),
        ("900023", "133", 2024, "ATH", "Oakland Athletics"),
        ("900024", "133", 2025, "ATH", "Athletics"),
    ];
    for (id, source, season, team, name) in cases {
        let game = find(&games, id);
        assert_eq!(game.len(), 1, "{id}");
        let g = game[0];
        assert_eq!(g.season, season, "{id}");
        let mapped = if g.home_source_id == source {
            &g.home_team
        } else {
            assert_eq!(g.away_source_id, source, "{id}");
            &g.away_team
        };
        assert_eq!(mapped, team, "{id}");
        assert_eq!(cfg.identity(team, season).unwrap().name, name, "{id}");
    }
}

#[test]
fn games_are_never_neutral() {
    let (games, _) = parse_fixture();
    assert!(!games.is_empty());
    assert!(games.iter().all(|g| !g.neutral));
}

#[test]
fn postseason_tie_is_rejected() {
    let error = parse_error(vec![entry(
        910001,
        "W",
        "Final: Tied",
        (141, Some(3)),
        (119, Some(3)),
        Some(true),
    )]);
    assert!(error.contains("Tie in a phase that forbids ties"), "{error}");
}

#[test]
fn unknown_game_type_is_rejected() {
    let error = parse_error(vec![entry(910002, "X", "Final", (141, Some(3)), (119, Some(2)), Some(false))]);
    assert_eq!(error, "Unknown game type: X");
}

#[test]
fn tie_flag_must_match_the_scores() {
    let error = parse_error(vec![entry(910003, "R", "Final", (141, Some(2)), (119, Some(2)), Some(false))]);
    assert_eq!(error, "Inconsistent tie flag for game 910003");
    let error = parse_error(vec![entry(910004, "R", "Final", (141, Some(3)), (119, Some(2)), Some(true))]);
    assert_eq!(error, "Inconsistent tie flag for game 910004");
}

#[test]
fn conflicting_finals_are_rejected() {
    let error = parse_error(vec![
        entry(910005, "R", "Final", (141, Some(3)), (119, Some(2)), Some(false)),
        entry(910005, "R", "Final", (141, Some(4)), (119, Some(2)), Some(false)),
    ]);
    assert_eq!(error, "Conflicting final results for game 910005");
}

#[test]
fn unknown_team_is_rejected() {
    let error = parse_error(vec![entry(910006, "R", "Final", (999, Some(3)), (119, Some(2)), Some(false))]);
    assert!(error.contains("Unknown team for"), "{error}");
}

#[test]
fn history_urls_are_one_per_season() {
    let cfg = mlb();
    let urls = adapter_for("mlb-statsapi").unwrap().history_urls(&cfg, 1998, 2000);
    let expected: Vec<String> = (1998..=2000)
        .map(|s| format!("https://statsapi.mlb.com/api/v1/schedule?sportId=1&season={s}&gameType=R,F,D,L,W"))
        .collect();
    assert_eq!(urls, expected);
}

#[test]
fn source_url_needs_a_season_placeholder() {
    let mut cfg = mlb();
    cfg.validate().unwrap();
    cfg.source.url = "https://statsapi.mlb.com/api/v1/schedule?sportId=1".into();
    assert_eq!(
        cfg.validate().unwrap_err().to_string(),
        "Source URL needs a {season} placeholder"
    );
}

#[test]
fn every_team_era_needs_a_nonempty_abbreviation() {
    let mut cfg = mlb();
    cfg.teams[0].eras[0].abbreviation = None;
    assert_eq!(
        cfg.validate().unwrap_err().to_string(),
        "Every team era needs an abbreviation"
    );
    cfg.teams[0].eras[0].abbreviation = Some(String::new());
    assert_eq!(cfg.validate().unwrap_err().to_string(), "Incomplete team era: ATH");
}

#[test]
fn season_incomplete_requires_a_completed_world_series() {
    let adapter = adapter_for("mlb-statsapi").unwrap();
    let (games, _) = parse_fixture();
    assert_eq!(adapter.season_incomplete(&games, 2025), None);
    let without: Vec<Game> = games.into_iter().filter(|g| g.round_label != "W").collect();
    assert_eq!(
        adapter.season_incomplete(&without, 2025),
        Some("no completed World Series".to_owned())
    );
}

#[test]
fn registry_resolves_the_stats_api_adapter() {
    let adapter = adapter_for("mlb-statsapi").unwrap();
    assert_eq!(adapter.kind(), "mlb-statsapi");
    assert!(adapter.strict_source_ids());
}

#[test]
fn league_config_loads_and_validates() {
    let (cfg, _) = load_league_config(Path::new(CONFIG_DIR), "mlb").unwrap();
    assert_eq!(cfg.id, "mlb");
    assert_eq!(cfg.source.kind, "mlb-statsapi");
    assert_eq!(cfg.teams.len(), 30);
    assert_eq!(cfg.aliases.len(), 30);
    assert_eq!(cfg.history_start, 1998);
    assert_eq!(cfg.ties_allowed_in, vec!["regular".to_owned()]);
    assert!(cfg.bayes_tune.is_none());
}
