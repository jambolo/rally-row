use rating_core::{EloTuneSettings, LeagueConfig, Outcome};
use serde_json::{Value, json};
use test_support::{
    cli::{
        ToolDir, assert_contains, assert_read_only_and_repeatable, assert_rejects_unusable_history, assert_requires_league,
        assert_split_flags_override_config, without_run_at,
    },
    evaluation_fixture,
};

const BIN: &str = env!("CARGO_BIN_EXE_evaluate-model");

fn tool() -> ToolDir {
    ToolDir::new(BIN, &["--json"])
}

fn overlap_noted(report: &Value) -> bool {
    report["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n.as_str().unwrap().contains("within a configured tuning range"))
}

#[test]
fn cli_is_read_only_repeatable_and_scores_both_methods_on_heldout_seasons() {
    let tool = tool();
    let (cfg, mut history) = evaluation_fixture();
    let (report, stderr) = assert_read_only_and_repeatable(&tool, "model-evaluation-report", &cfg, &history);
    assert_eq!(
        report["split"],
        json!({"warmup_start": 2002, "tune_start": 2003, "tune_end": 2004, "test_end": 2006})
    );
    assert_eq!(report["holdout_seasons"], json!([2005, 2006]));
    assert_contains(&stderr, "Predicted 5 games in season 2006");
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

    // Changing the final held-out game changes its score but none of the predictions made before it.
    history.games.last_mut().unwrap().result = Some(Outcome::HomeWin);
    tool.write_history(&history);
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
    tool.write_config(&cfg);
    tool.write_history(&history);
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
    assert_contains(&stderr, "Predicted 5 games in season 2006");
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
    let saved: Value = serde_json::from_slice(&std::fs::read(saved).unwrap()).unwrap();
    assert_eq!(without_run_at(&saved), without_run_at(&tool.report(&[])));
}

#[test]
fn cli_boundaries_override_config_and_flag_seasons_inside_a_tuning_range() {
    let tool = tool();
    let (mut cfg, history) = evaluation_fixture();
    tool.write_history(&history);
    let run = |cfg: &LeagueConfig, extra: &[&str]| -> Value {
        tool.write_config(cfg);
        tool.report(extra)
    };
    let report = run(&cfg, &["--test-end", "2005"]);
    assert_eq!(report["holdout_seasons"], json!([2005, 2005]));
    assert_eq!(report["predictors"]["bayesian"]["games"], 5);
    assert!(!overlap_noted(&report));

    // bayes_tune supplies the defaults, but a held-out season inside any configured tuning range is flagged.
    cfg.bayes_tune = Some(EloTuneSettings {
        tune_start: 2003,
        tune_end: 2005,
        test_end: 2006,
    });
    let report = run(&cfg, &[]);
    assert_eq!(report["holdout_seasons"], json!([2006, 2006]));
    assert!(!overlap_noted(&report));
    let report = run(&cfg, &["--tune-end", "2004"]);
    assert_eq!(report["holdout_seasons"], json!([2005, 2006]));
    assert!(overlap_noted(&report));
    assert!(
        report["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n.as_str().unwrap().starts_with("Seasons 2005–2005 "))
    );
}

#[test]
fn cli_split_flags_work_without_configured_defaults() {
    let tool = tool();
    let (cfg, history) = evaluation_fixture();
    assert_split_flags_override_config(&tool, cfg, &history);
    // With no configured tuning range, no held-out season is flagged.
    let report = tool.report(&["--tune-start", "2003", "--tune-end", "2004", "--test-end", "2006"]);
    assert!(!overlap_noted(&report));
}

#[test]
fn cli_rejects_invalid_splits_and_unfinished_history() {
    let (cfg, history) = evaluation_fixture();
    assert_rejects_unusable_history(&tool(), &cfg, &history);
}

#[test]
fn cli_requires_league() {
    assert_requires_league(BIN);
}
