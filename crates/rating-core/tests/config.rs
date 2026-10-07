use rating_core::LeagueConfig;
use serde_json::{Value, json};
use test_support::league_config_json;

fn nfl() -> Value {
    league_config_json("nfl")
}

fn check(v: Value) -> anyhow::Result<()> {
    serde_json::from_value::<LeagueConfig>(v)?.validate()
}

fn windows(season: (&str, &str), postseason: (&str, &str)) -> Value {
    let mut v = nfl();
    v["windows"] = json!({
        "season": {"start": season.0, "end": season.1},
        "postseason": {"start": postseason.0, "end": postseason.1},
    });
    v
}

fn error(v: Value) -> String {
    check(v).unwrap_err().to_string()
}

#[test]
fn nfl_config_is_valid_with_year_wrapping_windows() {
    check(nfl()).unwrap();
}

#[test]
fn accepts_non_wrapping_windows() {
    check(windows(("03-20", "11-05"), ("09-30", "11-05"))).unwrap();
}

#[test]
fn rejects_invalid_month_days() {
    for bad in ["02-29", "13-01", "04-31", "00-10", "01-00", "9-01", "09-1"] {
        assert_eq!(
            error(windows((bad, "02-15"), ("01-08", "02-15"))),
            format!("Invalid month-day: {bad}")
        );
    }
}

#[test]
fn rejects_postseason_outside_season() {
    for post in [("01-08", "02-20"), ("08-15", "02-15"), ("02-15", "01-08")] {
        assert_eq!(
            error(windows(("09-01", "02-15"), post)),
            "Postseason window must lie within the season window"
        );
    }
}

#[test]
fn rejects_equal_season_start_and_end() {
    assert_eq!(
        error(windows(("09-01", "09-01"), ("09-01", "09-01"))),
        "Season window start and end must be different"
    );
}

#[test]
fn rejects_schema_version_1() {
    let mut v = nfl();
    v["schema_version"] = json!(1);
    assert_eq!(error(v), "Unsupported config schema");
}

#[test]
fn rejects_unknown_schedule_filter_unit() {
    let mut v = nfl();
    v["display"]["schedule_filter"]["unit"] = json!("week");
    assert!(check(v).is_err());
}

#[test]
fn rejects_unknown_source_kind() {
    let mut v = nfl();
    v["source"]["kind"] = json!("bogus");
    assert_eq!(error(v), "Unknown source adapter");
}

#[test]
fn accepts_null_round_name() {
    let mut v = nfl();
    v["display"]["round_name"] = Value::Null;
    check(v).unwrap();
}

#[test]
fn serializes_display_and_windows_between_ties_allowed_in_and_elo() {
    let cfg: LeagueConfig = serde_json::from_value(nfl()).unwrap();
    let s = serde_json::to_string(&cfg).unwrap();
    let pos = |k: &str| s.find(k).unwrap_or_else(|| panic!("missing {k}"));
    let (t, d, w, e) = (
        pos("\"ties_allowed_in\":"),
        pos("\"display\":"),
        pos("\"windows\":"),
        pos("\"elo\":"),
    );
    assert!(t < d && d < w && w < e);
}

fn mlb() -> Value {
    league_config_json("mlb")
}

fn team<'a>(v: &'a mut Value, id: &str) -> &'a mut Value {
    v["teams"].as_array_mut().unwrap().iter_mut().find(|t| t["id"] == id).unwrap()
}

fn round(label: &str) -> Value {
    json!({"name": label, "short": label, "round_label": label, "pattern": "H", "home": "higher_seed"})
}

fn rejects(mutate: impl FnOnce(&mut Value), message: &str) {
    let mut v = nfl();
    mutate(&mut v);
    assert_eq!(error(v), message);
}

#[test]
fn rejects_team_division_without_postseason() {
    rejects(
        |v| {
            v.as_object_mut().unwrap().remove("postseason");
        },
        "Team division needs a postseason format: ARI",
    );
}

#[test]
fn rejects_postseason_ties() {
    rejects(
        |v| {
            v["ties_allowed_in"] = json!(["regular", "postseason"]);
        },
        "Postseason format requires postseason ties to be disallowed",
    );
}

#[test]
fn rejects_conference_count_not_power_of_two() {
    rejects(
        |v| {
            v["postseason"]["conferences"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id": "X", "name": "X", "divisions": [{"id": "X-1", "name": "X 1"}]}));
        },
        "Conference count must be a power of two",
    );
}

#[test]
fn rejects_duplicate_conference_or_division_id() {
    rejects(
        |v| {
            v["postseason"]["conferences"][1]["divisions"][0]["id"] = json!("AFC-E");
        },
        "Duplicate conference or division id",
    );
}

#[test]
fn rejects_playoff_field_smaller_than_two() {
    rejects(
        |v| {
            v["postseason"]["teams_per_conference"] = json!(1);
        },
        "Invalid playoff field size",
    );
}

#[test]
fn rejects_more_divisions_than_playoff_spots() {
    rejects(
        |v| {
            v["postseason"]["teams_per_conference"] = json!(3);
        },
        "More divisions than playoff spots",
    );
}

#[test]
fn rejects_round_count_mismatch() {
    rejects(
        |v| {
            v["postseason"]["rounds"].as_array_mut().unwrap().pop();
        },
        "Round count does not match the bracket",
    );
}

#[test]
fn rejects_duplicate_round_label() {
    rejects(
        |v| {
            v["postseason"]["rounds"][1]["round_label"] = json!("WC");
        },
        "Duplicate round label",
    );
}

#[test]
fn rejects_invalid_series_pattern() {
    rejects(
        |v| {
            v["postseason"]["rounds"][0]["pattern"] = json!("HH");
        },
        "Invalid series pattern",
    );
}

#[test]
fn rejects_seed_home_advantage_between_conferences() {
    rejects(
        |v| {
            v["postseason"]["rounds"][3]["pattern"] = json!("H");
            v["postseason"]["rounds"][3]["home"] = json!("higher_seed");
        },
        "Rounds between conferences cannot give home advantage by seed",
    );
}

#[test]
fn rejects_min_games_outside_common_games() {
    rejects(
        |v| {
            v["postseason"]["tiebreakers"]["division"][0]["min_games"] = json!(2);
        },
        "min_games applies only to common_games",
    );
}

#[test]
fn rejects_zero_min_games() {
    rejects(
        |v| {
            v["postseason"]["tiebreakers"]["conference"][2]["min_games"] = json!(0);
        },
        "min_games must be positive",
    );
}

#[test]
fn rejects_duplicate_tiebreaker() {
    rejects(
        |v| {
            v["postseason"]["tiebreakers"]["division"]
                .as_array_mut()
                .unwrap()
                .push(json!({"rule": "head_to_head"}));
        },
        "Duplicate tiebreaker",
    );
}

#[test]
fn rejects_unknown_division() {
    rejects(
        |v| {
            team(v, "ARI")["eras"][0]["division"] = json!("NOPE");
        },
        "Unknown division: ARI",
    );
}

#[test]
fn rejects_missing_current_division() {
    rejects(
        |v| {
            team(v, "ARI")["eras"][0].as_object_mut().unwrap().remove("division");
        },
        "Current division missing: ARI",
    );
}

#[test]
fn rejects_division_without_current_teams() {
    rejects(
        |v| {
            v["postseason"]["conferences"][0]["divisions"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id": "AFC-X", "name": "AFC X"}));
        },
        "Division has no current teams: AFC-X",
    );
}

#[test]
fn rejects_conference_short_of_playoff_spots() {
    rejects(
        |v| {
            v["postseason"]["teams_per_conference"] = json!(17);
            let rounds = v["postseason"]["rounds"].as_array_mut().unwrap();
            rounds.insert(0, round("R2"));
            rounds.insert(0, round("R1"));
        },
        "Conference has fewer teams than playoff spots: AFC",
    );
}

#[test]
fn rejects_empty_era_division() {
    rejects(
        |v| {
            team(v, "ARI")["eras"][0]["division"] = json!("");
        },
        "Incomplete team era: ARI",
    );
}

#[test]
fn mlb_config_is_valid() {
    check(mlb()).unwrap();
}

#[test]
fn older_eras_may_omit_division() {
    let mut v = nfl();
    team(&mut v, "LV")["eras"][0].as_object_mut().unwrap().remove("division");
    check(v).unwrap();
}

#[test]
fn config_without_postseason_or_divisions_is_valid() {
    let mut v = nfl();
    v.as_object_mut().unwrap().remove("postseason");
    for t in v["teams"].as_array_mut().unwrap() {
        for e in t["eras"].as_array_mut().unwrap() {
            e.as_object_mut().unwrap().remove("division");
        }
    }
    check(v).unwrap();
}

fn alignment(v: Value) -> Vec<Vec<usize>> {
    let cfg: LeagueConfig = serde_json::from_value(v).unwrap();
    cfg.postseason
        .as_ref()
        .unwrap()
        .conferences
        .iter()
        .map(|c| {
            c.divisions
                .iter()
                .map(|d| {
                    cfg.teams
                        .iter()
                        .filter(|t| t.eras.last().and_then(|e| e.division.as_deref()) == Some(d.id.as_str()))
                        .count()
                })
                .collect()
        })
        .collect()
}

#[test]
fn nfl_alignment_is_two_conferences_of_four_divisions_of_four_teams() {
    assert_eq!(alignment(nfl()), vec![vec![4; 4]; 2]);
}

#[test]
fn mlb_alignment_is_two_conferences_of_three_divisions_of_five_teams() {
    assert_eq!(alignment(mlb()), vec![vec![5; 3]; 2]);
}

#[test]
fn hou_moves_from_nl_central_to_al_west_in_2013() {
    let cfg: LeagueConfig = serde_json::from_value(mlb()).unwrap();
    assert_eq!(cfg.identity("HOU", 2012).unwrap().division.as_deref(), Some("NL-C"));
    assert_eq!(cfg.identity("HOU", 2013).unwrap().division.as_deref(), Some("AL-W"));
}

#[test]
fn serializes_postseason_between_windows_and_elo() {
    let cfg: LeagueConfig = serde_json::from_value(nfl()).unwrap();
    let s = serde_json::to_string(&cfg).unwrap();
    let pos = |k: &str| s.find(k).unwrap_or_else(|| panic!("missing {k}"));
    let (w, p, e) = (pos("\"windows\":"), pos("\"postseason\":{\"conferences\""), pos("\"elo\":"));
    assert!(w < p && p < e);
}
