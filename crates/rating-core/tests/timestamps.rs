use rating_core::{Game, LeagueConfig, parse_source, parse_source_with_warnings, replay_elo, validate_games};
use serde_json::{Value, json};
use test_support::{NFLVERSE_HEADER, league_config};

fn config() -> LeagueConfig {
    let mut cfg = league_config("nfl");
    cfg.source.kind = "canonical-json".into();
    cfg
}

fn row(id: &str, date: &str, time: Option<&str>, timezone: &str) -> Value {
    json!({
        "id": id, "league": "nfl", "season": 2002, "date": date, "time": time,
        "timezone": timezone, "phase": "regular", "round_label": "REG", "round": 1,
        "home_team": "SEA", "away_team": "SF", "home_source_id": "SEA",
        "away_source_id": "SF", "neutral": true, "result": "home_win"
    })
}

fn source(rows: Vec<Value>) -> String {
    json!({"schema_version": 2, "league": "nfl", "games": rows}).to_string()
}

#[test]
fn converts_offsets_and_discards_local_fields_with_explicit_dst_policies() {
    for (date, time, zone, expected, warning) in [
        ("2002-09-01", Some("13:00"), "America/New_York", "2002-09-01T17:00:00Z", ""),
        ("2002-01-01", Some("13:00"), "America/New_York", "2002-01-01T18:00:00Z", ""),
        ("2002-09-01", Some("23:30"), "America/Los_Angeles", "2002-09-02T06:30:00Z", ""),
        ("2002-09-02", Some("08:00"), "Asia/Tokyo", "2002-09-01T23:00:00Z", ""),
        ("2002-09-01", None, "America/New_York", "2002-09-01T04:00:00Z", "missing time"),
        (
            "2002-09-01",
            Some(""),
            "America/New_York",
            "2002-09-01T04:00:00Z",
            "missing time",
        ),
        (
            "2002-10-27",
            Some("01:30"),
            "America/New_York",
            "2002-10-27T05:30:00Z",
            "ambiguous local time",
        ),
        (
            "2002-04-07",
            Some("02:30"),
            "America/New_York",
            "2002-04-07T05:00:00Z",
            "nonexistent local time",
        ),
    ] {
        let input = source(vec![row("g", date, time, zone)]);
        let mut warnings = Vec::new();
        let games = parse_source_with_warnings(&input, &config(), |w| warnings.push(w)).unwrap();
        assert_eq!(serde_json::to_value(&games[0]).unwrap()["start_time_utc"], expected);
        let stored = serde_json::to_value(&games[0]).unwrap();
        for field in ["date", "time", "timezone"] {
            assert!(stored.get(field).is_none());
        }
        if warning.is_empty() {
            assert!(warnings.is_empty());
        } else {
            assert_eq!(warnings.len(), 1);
            assert!(warnings[0].contains(warning), "{}", warnings[0]);
            assert!(warnings[0].contains("Game g:"));
        }
    }
}

#[test]
fn sorts_and_replays_shared_teams_by_utc_even_across_local_dates() {
    let mut later = row("later", "2002-09-01", Some("23:30"), "America/Los_Angeles");
    later["result"] = json!("away_win");
    let earlier = row("earlier", "2002-09-02", Some("08:00"), "Asia/Tokyo");
    let mut cfg = config();
    cfg.elo.k = 20.0;
    let games = parse_source(&source(vec![later, earlier]), &cfg).unwrap();
    assert_eq!(games.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(), ["earlier", "later"]);
    let mut stored: Vec<Game> = serde_json::from_str(&serde_json::to_string(&games).unwrap()).unwrap();
    stored.reverse();
    let replay = replay_elo(&stored, &cfg, 2002).unwrap();
    assert_eq!(replay.audit[0].game_id, "earlier");
    assert_eq!(replay.audit[1].home_before, cfg.elo.initial + 10.0);
    assert!(replay.audit[1].home_after < cfg.elo.initial);
    let mut changed = games;
    changed[0].start_time_utc = "2002-09-03T00:00:00Z".parse().unwrap();
    validate_games(&mut changed, &cfg).unwrap();
    assert_eq!(changed[0].id, "later");
}

#[test]
fn simultaneous_games_use_byte_order_after_season_and_utc() {
    let a = row("A", "2002-09-01", Some("14:00"), "Europe/London");
    let z = row("z", "2002-09-01", Some("09:00"), "America/New_York");
    let mut next = a.clone();
    next["id"] = json!("next-season");
    next["season"] = json!(2003);
    let games = parse_source(&source(vec![z, a, next]), &config()).unwrap();
    assert_eq!(
        games.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(),
        ["A", "z", "next-season"]
    );
}

#[test]
fn rejects_missing_dates_invalid_zones_and_unrepresentable_midnight() {
    let base = row("bad-game", "2002-09-01", Some("13:00"), "America/New_York");
    for date in [Value::Null, json!(""), json!("2002-02-30")] {
        let mut bad = base.clone();
        bad["date"] = date;
        assert!(parse_source(&source(vec![bad]), &config()).is_err());
    }
    let mut missing = base.clone();
    missing.as_object_mut().unwrap().remove("date");
    let error = parse_source(&source(vec![missing]), &config()).unwrap_err();
    assert!(error.to_string().contains("Missing source date for game bad-game"));
    let mut bad_zone = base;
    bad_zone["timezone"] = json!("Mars/Olympus");
    assert!(parse_source(&source(vec![bad_zone]), &config()).is_err());
    let skipped_date = row("skipped", "2011-12-30", None, "Pacific/Apia");
    assert!(
        parse_source(&source(vec![skipped_date]), &config())
            .unwrap_err()
            .to_string()
            .contains("midnight")
    );
}

#[test]
fn csv_missing_time_warns_and_serializes_utc() {
    let cfg = league_config("nfl");
    let csv = &format!("{NFLVERSE_HEADER}g,2002,REG,1,2002-09-01,,SEA,10,SF,20,Home\n");
    let mut warnings = Vec::new();
    let games = parse_source_with_warnings(csv, &cfg, |w| warnings.push(w)).unwrap();
    assert_eq!(warnings.len(), 1);
    assert_eq!(
        serde_json::to_value(&games[0]).unwrap()["start_time_utc"],
        "2002-09-01T04:00:00Z"
    );
    assert!(parse_source(&csv.replace("2002-09-01", ""), &cfg).is_err());
    let json = serde_json::to_string(&games).unwrap();
    let round_trip: Vec<Game> = serde_json::from_str(&json).unwrap();
    assert_eq!(round_trip, games);
}

#[test]
fn historical_reader_uses_only_stored_utc_and_ignores_local_fields() {
    let mut input = row("utc-only", "invalid-local-date", Some("invalid-local-time"), "invalid-zone");
    input["start_time_utc"] = json!("2002-09-01T17:00:00Z");
    let with_local: Game = serde_json::from_value(input.clone()).unwrap();
    for field in ["date", "time", "timezone"] {
        input.as_object_mut().unwrap().remove(field);
    }
    let without_local: Game = serde_json::from_value(input).unwrap();
    let mut games = vec![with_local];
    validate_games(&mut games, &config()).unwrap();
    assert_eq!(games[0], without_local);
    assert_eq!(
        serde_json::to_value(&games[0]).unwrap()["start_time_utc"],
        "2002-09-01T17:00:00Z"
    );
    assert_eq!(replay_elo(&games, &config(), 2002).unwrap().audit.len(), 1);
}

#[test]
fn historical_reader_rejects_missing_or_invalid_utc_without_local_fallback() {
    let input = row("missing-utc", "2002-09-01", Some("13:00"), "America/New_York");
    assert!(
        serde_json::from_value::<Game>(input.clone())
            .unwrap_err()
            .to_string()
            .contains("start_time_utc")
    );
    for invalid in [
        Value::Null,
        json!(""),
        json!("2002-09-01T17:00:00"),
        json!("2002-02-30T17:00:00Z"),
    ] {
        let mut bad = input.clone();
        bad["start_time_utc"] = invalid;
        assert!(serde_json::from_value::<Game>(bad).is_err());
    }
}
