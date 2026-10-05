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
