use rating_core::{EloTuneSettings, Outcome};
use serde_json::Value;
use std::{fs, path::Path, process::Command};

mod common;
use common::fixture;

fn invoke(root: &Path, extra: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_evaluate-model"))
        .current_dir(root)
        .args(["--league", "nfl", "--config-dir", ".", "--data-dir", "data"])
        .args(extra)
        .output()
        .unwrap()
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
    let dir = tempfile::tempdir().unwrap();
    let (cfg, mut history) = fixture();
    let config_bytes = serde_json::to_vec(&cfg).unwrap();
    let history_bytes = serde_json::to_vec(&history).unwrap();
    fs::create_dir_all(dir.path().join("data/nfl")).unwrap();
    fs::write(dir.path().join("nfl.json"), &config_bytes).unwrap();
    fs::write(dir.path().join("data/nfl/history.json"), &history_bytes).unwrap();
    fs::write(dir.path().join("data/nfl/elo-2007.json"), "untouched seed").unwrap();
    let started = chrono::Utc::now();
    let output = invoke(dir.path(), &["--json", "--report-dir", "reports"]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report["split"],
        serde_json::json!({"warmup_start": 2002, "tune_start": 2003, "tune_end": 2004, "test_end": 2006})
    );
    assert_eq!(report["holdout_seasons"], serde_json::json!([2005, 2006]));
    let run_at = chrono::DateTime::parse_from_rfc3339(report["run_at"].as_str().unwrap()).unwrap();
    assert_eq!(run_at.offset().local_minus_utc(), 0);
    assert!(run_at >= started && run_at <= chrono::Utc::now());
    let report_path = dir
        .path()
        .join("reports")
        .join(format!("model-evaluation-report-nfl-{}.json", run_at.format("%Y-%m-%d")));
    let saved: Value = serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
    assert_eq!(saved, report);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Predicted 5 games in season 2006") && !stderr.contains("Favorites agree"));
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
    assert_eq!(report["config_sha256"], rating_core::digest(&config_bytes));
    assert_eq!(report["history_sha256"], rating_core::digest(&history_bytes));
    assert!(!overlap_noted(&report));

    let again = invoke(dir.path(), &["--json"]);
    assert!(again.status.success());
    let mut repeated: Value = serde_json::from_slice(&again.stdout).unwrap();
    let mut original = report.clone();
    original.as_object_mut().unwrap().remove("run_at");
    repeated.as_object_mut().unwrap().remove("run_at");
    assert_eq!(original, repeated);
    assert_eq!(fs::read(dir.path().join("nfl.json")).unwrap(), config_bytes);
    assert_eq!(fs::read(dir.path().join("data/nfl/history.json")).unwrap(), history_bytes);
    assert_eq!(
        fs::read_to_string(dir.path().join("data/nfl/elo-2007.json")).unwrap(),
        "untouched seed"
    );
    assert_eq!(fs::read_dir(dir.path().join("data/nfl")).unwrap().count(), 2);

    // Changing the final held-out game changes its score but none of the predictions made before it.
    history.games.last_mut().unwrap().result = Some(Outcome::HomeWin);
    fs::write(
        dir.path().join("data/nfl/history.json"),
        serde_json::to_vec(&history).unwrap(),
    )
    .unwrap();
    let changed = invoke(dir.path(), &["--json"]);
    assert!(changed.status.success());
    let changed: Value = serde_json::from_slice(&changed.stdout).unwrap();
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
    let dir = tempfile::tempdir().unwrap();
    let (cfg, history) = fixture();
    fs::create_dir_all(dir.path().join("data/nfl")).unwrap();
    fs::write(dir.path().join("nfl.json"), serde_json::to_vec(&cfg).unwrap()).unwrap();
    fs::write(
        dir.path().join("data/nfl/history.json"),
        serde_json::to_vec(&history).unwrap(),
    )
    .unwrap();
    let output = invoke(dir.path(), &["--report-dir", "reports"]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.starts_with("League nfl: held-out seasons 2005–2006, 10 games\n"),
        "{stdout}"
    );
    assert!(stdout.contains("Favorites agree"));
    assert!(serde_json::from_str::<Value>(&stdout).is_err());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Predicted 5 games in season 2006") && stderr.contains("Saved report to"));
    assert!(!stderr.contains("Favorites agree"));
    let saved = fs::read_dir(dir.path().join("reports"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut saved: Value = serde_json::from_slice(&fs::read(saved).unwrap()).unwrap();
    let json = invoke(dir.path(), &["--json"]);
    assert!(json.status.success(), "{}", String::from_utf8_lossy(&json.stderr));
    let mut printed: Value = serde_json::from_slice(&json.stdout).unwrap();
    saved.as_object_mut().unwrap().remove("run_at");
    printed.as_object_mut().unwrap().remove("run_at");
    assert_eq!(saved, printed);
}

#[test]
fn cli_boundaries_override_config_and_flag_seasons_inside_a_tuning_range() {
    let dir = tempfile::tempdir().unwrap();
    let (mut cfg, history) = fixture();
    fs::create_dir_all(dir.path().join("data/nfl")).unwrap();
    fs::write(
        dir.path().join("data/nfl/history.json"),
        serde_json::to_vec(&history).unwrap(),
    )
    .unwrap();
    let run = |cfg: &rating_core::LeagueConfig, extra: &[&str]| -> Value {
        fs::write(dir.path().join("nfl.json"), serde_json::to_vec(cfg).unwrap()).unwrap();
        let output = invoke(dir.path(), &[&["--json"], extra].concat());
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        serde_json::from_slice(&output.stdout).unwrap()
    };
    let report = run(&cfg, &["--test-end", "2005"]);
    assert_eq!(report["holdout_seasons"], serde_json::json!([2005, 2005]));
    assert_eq!(report["predictors"]["bayesian"]["games"], 5);
    assert!(!overlap_noted(&report));

    // bayes_tune supplies the defaults, but a held-out season inside any configured tuning range is flagged.
    cfg.bayes_tune = Some(EloTuneSettings {
        tune_start: 2003,
        tune_end: 2005,
        test_end: 2006,
    });
    let report = run(&cfg, &[]);
    assert_eq!(report["holdout_seasons"], serde_json::json!([2006, 2006]));
    assert!(!overlap_noted(&report));
    let report = run(&cfg, &["--tune-end", "2004"]);
    assert_eq!(report["holdout_seasons"], serde_json::json!([2005, 2006]));
    assert!(overlap_noted(&report));
    assert!(
        report["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n.as_str().unwrap().starts_with("Seasons 2005–2005 "))
    );

    cfg.elo_tune = None;
    cfg.bayes_tune = None;
    fs::write(dir.path().join("nfl.json"), serde_json::to_vec(&cfg).unwrap()).unwrap();
    let missing = invoke(dir.path(), &[]);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("elo_tune.tune_start"));
    let report = run(&cfg, &["--tune-start", "2003", "--tune-end", "2004", "--test-end", "2006"]);
    assert_eq!(report["holdout_seasons"], serde_json::json!([2005, 2006]));
    assert!(!overlap_noted(&report));
}

#[test]
fn cli_rejects_invalid_splits_and_unfinished_history() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, history) = fixture();
    fs::create_dir_all(dir.path().join("data/nfl")).unwrap();
    fs::write(dir.path().join("nfl.json"), serde_json::to_vec(&cfg).unwrap()).unwrap();
    let history_path = dir.path().join("data/nfl/history.json");
    fs::write(&history_path, serde_json::to_vec(&history).unwrap()).unwrap();
    let invalid = invoke(dir.path(), &["--test-end", "2004"]);
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("held-out seasons"));
    let short = invoke(dir.path(), &["--test-end", "2007"]);
    assert!(!short.status.success());
    assert!(String::from_utf8_lossy(&short.stderr).contains("History must cover"));
    let mut unfinished = history;
    unfinished.games.last_mut().unwrap().result = None;
    fs::write(&history_path, serde_json::to_vec(&unfinished).unwrap()).unwrap();
    let unfinished = invoke(dir.path(), &[]);
    assert!(!unfinished.status.success());
    assert!(String::from_utf8_lossy(&unfinished.stderr).contains("unreported game"));
}

#[test]
fn cli_requires_league() {
    let output = Command::new(env!("CARGO_BIN_EXE_evaluate-model")).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("--league <LEAGUE>"));
}
