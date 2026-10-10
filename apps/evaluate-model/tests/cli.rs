use rating_core::{EloTuneSettings, GameFile, LeagueConfig, Outcome, SimulatedSeasons, tuning::Split, write_json};
use serde_json::{Value, json};
use std::fs;
use test_support::{
    cli::{ToolDir, assert_contains, assert_read_only_and_repeatable, assert_requires_league, without_run_at},
    evaluation_fixture,
};

const BIN: &str = env!("CARGO_BIN_EXE_evaluate-model");
const SIMULATED: &str = "data/nfl/simulated-seasons.json";

fn tool() -> ToolDir {
    ToolDir::new(BIN, &["--json"])
}

/// The fixture's split with held-out seasons `tune_end + 1` through `test_end`.
fn split(tune_end: i32, test_end: i32) -> Split {
    Split {
        warmup_start: 2002,
        tune_start: 2003,
        tune_end,
        test_end,
    }
}

/// Writes `cfg` and `history`, then the `simulated-seasons.json` that simulate-season writes from them for `split`.
fn simulate(tool: &ToolDir, cfg: &LeagueConfig, history: &GameFile, split: Split) {
    let config_bytes = tool.write_config(cfg);
    let history_bytes = tool.write_history(history);
    let simulated = SimulatedSeasons::simulate(history, &history_bytes, cfg, &config_bytes, split, |_| {}).unwrap();
    write_json(&tool.path().join(SIMULATED), &simulated).unwrap();
}

fn overlap_noted(report: &Value) -> bool {
    report["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n.as_str().unwrap().contains("within a configured tuning range"))
}

#[test]
fn cli_is_read_only_repeatable_and_scores_both_methods_on_the_simulated_seasons() {
    let tool = tool();
    let (cfg, mut history) = evaluation_fixture();
    // The check below rewrites the same configuration and history bytes, so the simulation stays current.
    simulate(&tool, &cfg, &history, split(2004, 2006));
    let (report, stderr) = assert_read_only_and_repeatable(&tool, "model-evaluation-report", &cfg, &history);
    assert_eq!(
        report["split"],
        json!({"warmup_start": 2002, "tune_start": 2003, "tune_end": 2004, "test_end": 2006})
    );
    assert_eq!(report["holdout_seasons"], json!([2005, 2006]));
    assert_contains(&stderr, "Scoring 5 games in season 2006");
    assert!(!stderr.contains("Favorites agree"));
    for method in ["bayesian", "elo", "equal_strength"] {
        let evaluation = &report["predictors"][method];
        assert_eq!(evaluation["games"], 10, "{method}");
        assert_eq!(evaluation["pooled"]["decisive_games"], 8, "{method}");
        assert_eq!(evaluation["pooled"]["observed_ties"], 2, "{method}");
        assert_eq!(evaluation["seasons"].as_array().unwrap().len(), 2, "{method}");
    }
    assert_eq!(report["predictors"]["equal_strength"]["pooled"]["accuracy"], 0.5);
    let metrics: Vec<_> = report["comparison"]["metrics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["metric"].as_str().unwrap())
        .collect();
    assert_eq!(metrics, ["log_loss", "brier", "expected_score_mse", "accuracy"]);
    assert_eq!(report["comparison"]["picks"]["decisive_games"], 8);
    assert_eq!(report["comparison"]["metrics"][0]["seasons"].as_array().unwrap().len(), 2);
    assert_eq!(report["tie_weights"].as_array().unwrap().len(), 2);
    assert_eq!(report["elo_settings"], serde_json::to_value(&cfg.elo).unwrap());
    assert_eq!(report["bayesian_settings"], serde_json::to_value(&cfg.bayesian).unwrap());
    assert!(!overlap_noted(&report));

    // A changed history needs a new simulation. The final held-out game's result changes its score but none of the
    // predictions made before it.
    history.games.last_mut().unwrap().result = Some(Outcome::HomeWin);
    tool.write_history(&history);
    assert_contains(
        &tool.error(&[]),
        "simulated-seasons.json was generated from a different history.json; rerun simulate-season",
    );
    simulate(&tool, &cfg, &history, split(2004, 2006));
    let changed = tool.report(&[]);
    assert_eq!(report["tie_weights"], changed["tie_weights"]);
    assert_ne!(
        report["predictors"]["bayesian"]["pooled"],
        changed["predictors"]["bayesian"]["pooled"]
    );
    assert_eq!(
        report["predictors"]["bayesian"]["pooled"]["expected_ties"],
        changed["predictors"]["bayesian"]["pooled"]["expected_ties"]
    );
}

#[test]
fn cli_prints_the_summary_by_default_and_still_saves_the_json_report() {
    let tool = tool();
    let (cfg, history) = evaluation_fixture();
    simulate(&tool, &cfg, &history, split(2004, 2006));
    let output = tool.run(&["--report-dir", "reports"]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.starts_with("League nfl: held-out seasons 2005–2006, 10 games\n"),
        "{stdout}"
    );
    assert_contains(&stdout, "Favorites agree");
    assert!(serde_json::from_str::<Value>(&stdout).is_err());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_contains(&stderr, "Scoring 5 games in season 2006");
    assert_contains(&stderr, "Saved report to");
    assert!(!stderr.contains("Favorites agree"));
    let saved = tool
        .path()
        .join("reports")
        .read_dir()
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let saved: Value = serde_json::from_slice(&fs::read(saved).unwrap()).unwrap();
    assert_eq!(without_run_at(&saved), without_run_at(&tool.report(&[])));
}

#[test]
fn cli_scores_the_simulated_split_and_flags_seasons_inside_a_tuning_range() {
    let tool = tool();
    let (mut cfg, history) = evaluation_fixture();
    let run = |cfg: &LeagueConfig, split: Split| -> Value {
        simulate(&tool, cfg, &history, split);
        tool.report(&[])
    };
    let report = run(&cfg, split(2004, 2005));
    assert_eq!(
        report["split"],
        json!({"warmup_start": 2002, "tune_start": 2003, "tune_end": 2004, "test_end": 2005})
    );
    assert_eq!(report["holdout_seasons"], json!([2005, 2005]));
    assert_eq!(report["predictors"]["bayesian"]["games"], 5);
    assert!(!overlap_noted(&report));

    // A held-out season inside any configured tuning range is flagged.
    cfg.bayes_tune = Some(EloTuneSettings {
        tune_start: 2003,
        tune_end: 2005,
        test_end: 2006,
    });
    let report = run(&cfg, split(2005, 2006));
    assert_eq!(report["holdout_seasons"], json!([2006, 2006]));
    assert!(!overlap_noted(&report));
    let report = run(&cfg, split(2004, 2006));
    assert_eq!(report["holdout_seasons"], json!([2005, 2006]));
    assert!(overlap_noted(&report));
    assert!(
        report["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n.as_str().unwrap().starts_with("Seasons 2005–2005 "))
    );

    // With no configured tuning range, no held-out season is flagged.
    cfg.bayes_tune = None;
    cfg.elo_tune = None;
    assert!(!overlap_noted(&run(&cfg, split(2004, 2006))));
}

#[test]
fn cli_rejects_a_missing_incompatible_or_stale_simulation() {
    let tool = tool();
    let (mut cfg, history) = evaluation_fixture();
    tool.write_config(&cfg);
    assert_contains(&tool.error(&[]), "Read history.json; run import-history first");
    tool.write_history(&history);
    assert_contains(&tool.error(&[]), "Read simulated-seasons.json; run simulate-season first");

    simulate(&tool, &cfg, &history, split(2004, 2006));
    let path = tool.path().join(SIMULATED);
    let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for (field, value) in [("schema_version", json!(2)), ("league", json!("mlb"))] {
        let mut changed = saved.clone();
        changed[field] = value;
        fs::write(&path, serde_json::to_vec(&changed).unwrap()).unwrap();
        assert_contains(&tool.error(&[]), "Incompatible simulated-seasons.json; rerun simulate-season");
    }
    fs::write(&path, "previous format").unwrap();
    assert_contains(&tool.error(&[]), "Parse simulated-seasons.json; rerun simulate-season");

    fs::write(&path, serde_json::to_vec(&saved).unwrap()).unwrap();
    tool.report(&[]);
    cfg.elo.k += 1.0;
    tool.write_config(&cfg);
    assert_contains(
        &tool.error(&[]),
        "simulated-seasons.json was generated from a different configuration; rerun simulate-season",
    );
}

#[test]
fn cli_requires_league() {
    assert_requires_league(BIN);
}
