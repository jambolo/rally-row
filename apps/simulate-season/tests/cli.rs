use chrono::{DateTime, Utc};
use rating_core::{Outcome, SimulatedSeasons, digest, walk_forward::predict_season};
use serde_json::{Value, json};
use std::fs;
use test_support::{
    cli::{ToolDir, assert_contains, assert_rejects_unusable_history, assert_requires_league},
    evaluation_fixture,
};

const BIN: &str = env!("CARGO_BIN_EXE_simulate-season");
const OUTPUT: &str = "data/nfl/simulated-seasons.json";

fn tool() -> ToolDir {
    ToolDir::new(BIN, &[])
}

/// Runs the tool, requires success, and returns stdout, stderr, and the saved file.
fn simulate(tool: &ToolDir, args: &[&str]) -> (String, String, Value) {
    let output = tool.run(args);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "{stderr}");
    let saved = serde_json::from_slice(&fs::read(tool.path().join(OUTPUT)).unwrap()).unwrap();
    (String::from_utf8(output.stdout).unwrap(), stderr, saved)
}

fn without_generated_at(saved: &Value) -> Value {
    let mut saved = saved.clone();
    saved.as_object_mut().unwrap().remove("generated_at");
    saved
}

#[test]
fn cli_saves_every_heldout_game_predicted_from_earlier_dates_and_leaves_inputs_untouched() {
    let tool = tool();
    let (cfg, history) = evaluation_fixture();
    let config_bytes = tool.write_config(&cfg);
    let history_bytes = tool.write_history(&history);
    let started = Utc::now();
    let (stdout, stderr, saved) = simulate(&tool, &[]);
    assert_contains(&stdout, "Saved predictions for 10 games in seasons 2005–2006 to ");
    assert_contains(&stderr, "Predicted 5 games in season 2006");

    let generated_at = DateTime::parse_from_rfc3339(saved["generated_at"].as_str().unwrap()).unwrap();
    assert_eq!(generated_at.offset().local_minus_utc(), 0);
    assert!(generated_at >= started && generated_at <= Utc::now());
    assert_eq!(saved["schema_version"], 1);
    assert_eq!(saved["league"], "nfl");
    assert_eq!(saved["config_sha256"], digest(&config_bytes));
    assert_eq!(saved["history_sha256"], digest(&history_bytes));
    assert_eq!(
        saved["split"],
        json!({"warmup_start": 2002, "tune_start": 2003, "tune_end": 2004, "test_end": 2006})
    );
    assert_eq!(saved["elo_settings"], serde_json::to_value(&cfg.elo).unwrap());
    assert_eq!(saved["bayesian_settings"], serde_json::to_value(&cfg.bayesian).unwrap());

    let file: SimulatedSeasons = serde_json::from_value(saved.clone()).unwrap();
    assert_eq!(file.seasons.iter().map(|s| s.season).collect::<Vec<_>>(), [2005, 2006]);
    for (i, season) in file.seasons.iter().enumerate() {
        // Both sides are parsed from JSON text, because serde_json's default float parsing may differ in the last bit.
        let expected = serde_json::to_string(&predict_season(&history, &cfg, season.season).unwrap()).unwrap();
        assert_eq!(saved["seasons"][i], serde_json::from_str::<Value>(&expected).unwrap());
        let games: Vec<_> = season.games.iter().map(|p| &p.game).collect();
        let expected: Vec<_> = history.games.iter().filter(|g| g.season == season.season).collect();
        assert_eq!(games, expected);
    }

    let (_, _, repeated) = simulate(&tool, &[]);
    assert_eq!(without_generated_at(&saved), without_generated_at(&repeated));
    assert_eq!(fs::read(tool.path().join("nfl.json")).unwrap(), config_bytes);
    assert_eq!(fs::read(tool.path().join("data/nfl/history.json")).unwrap(), history_bytes);
}

#[test]
fn cli_a_games_result_never_changes_its_own_prediction() {
    let tool = tool();
    let (cfg, mut history) = evaluation_fixture();
    tool.write_config(&cfg);
    tool.write_history(&history);
    let (_, _, before) = simulate(&tool, &[]);
    history.games.last_mut().unwrap().result = Some(Outcome::HomeWin);
    tool.write_history(&history);
    let (_, _, mut after) = simulate(&tool, &[]);
    assert_eq!(after["seasons"][1]["games"][4]["result"], "home_win");
    after["seasons"][1]["games"][4]["result"] = before["seasons"][1]["games"][4]["result"].clone();
    assert_eq!(before["seasons"], after["seasons"]);
}

#[test]
fn cli_split_flags_override_the_configured_holdout_seasons() {
    let tool = tool();
    let (mut cfg, history) = evaluation_fixture();
    tool.write_config(&cfg);
    tool.write_history(&history);
    let seasons = |saved: &Value| -> Vec<i64> {
        saved["seasons"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["season"].as_i64().unwrap())
            .collect()
    };
    let (stdout, _, saved) = simulate(&tool, &["--test-end", "2005"]);
    assert_contains(&stdout, "Saved predictions for 5 games in seasons 2005–2005 to ");
    assert_eq!(seasons(&saved), [2005]);
    let (_, _, saved) = simulate(&tool, &["--tune-end", "2005"]);
    assert_eq!(seasons(&saved), [2006]);

    cfg.elo_tune = None;
    tool.write_config(&cfg);
    assert_contains(
        &tool.error(&[]),
        "Set bayes_tune.tune_start (or elo_tune.tune_start) in the config or pass --tune-start",
    );
    let (_, _, saved) = simulate(&tool, &["--tune-start", "2003", "--tune-end", "2004", "--test-end", "2006"]);
    assert_eq!(seasons(&saved), [2005, 2006]);
}

#[test]
fn cli_rejects_invalid_splits_and_unfinished_history_and_keeps_the_previous_file() {
    let tool = tool();
    let (cfg, history) = evaluation_fixture();
    fs::write(tool.path().join(OUTPUT), "previous predictions").unwrap();
    assert_rejects_unusable_history(&tool, &cfg, &history);
    assert_eq!(fs::read_to_string(tool.path().join(OUTPUT)).unwrap(), "previous predictions");
}

#[test]
fn cli_requires_league() {
    assert_requires_league(BIN);
}
