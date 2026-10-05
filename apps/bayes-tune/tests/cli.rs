use rating_core::{BayesianGrid, EloGrid, EloTuneSettings, LeagueConfig, Outcome, TuningGrids};
use serde_json::{Value, json};
use test_support::{
    cli::{
        ToolDir, assert_boundary_report, assert_read_only_and_repeatable, assert_rejects_unusable_history, assert_requires_league,
        assert_split_flags_override_config,
    },
    tuning_fixture,
};

const BIN: &str = env!("CARGO_BIN_EXE_bayes-tune");
const PARAMETERS: [&str; 3] = ["prior_sd_elo", "tie_prior_games", "tie_prior_rate"];

fn tool() -> ToolDir {
    ToolDir::new(BIN, &[])
}

#[test]
fn cli_is_read_only_repeatable_and_selects_independently_of_heldout_outcomes() {
    let tool = tool();
    let (cfg, mut history) = tuning_fixture();
    let (report, _) = assert_read_only_and_repeatable(&tool, "bayes-tuning-report", &cfg, &history);
    assert_eq!(
        report["split"],
        json!({"warmup_start": 2002, "tune_start": 2003, "tune_end": 2004, "test_end": 2005})
    );
    assert_eq!(report["tuning"]["baseline"]["games"], 8);
    assert_eq!(report["holdout"]["baseline"]["games"], 4);
    assert_eq!(report["fixed_elo"], serde_json::to_value(&cfg.elo).unwrap());
    assert_eq!(report["baseline_parameters"]["prior_sd_elo"], cfg.bayesian.prior_sd_elo);
    assert!(report["search"]["candidates_evaluated"].as_u64().unwrap() >= 120);

    for g in history.games.iter_mut().filter(|g| g.season == 2005) {
        g.result = Some(Outcome::AwayWin);
    }
    tool.write_history(&history);
    let changed = tool.report(&["--test-end", "2005"]);
    assert_eq!(report["selected_parameters"], changed["selected_parameters"]);
    assert_eq!(report["search"], changed["search"]);
    assert_eq!(report["tuning"], changed["tuning"]);
    assert_ne!(report["holdout"], changed["holdout"]);
}

#[test]
fn cli_boundaries_override_config_and_work_without_defaults() {
    let tool = tool();
    let (mut cfg, history) = tuning_fixture();
    assert_split_flags_override_config(&tool, cfg.clone(), &history);
    // A bayes_tune split takes precedence over elo_tune.
    cfg.elo_tune = Some(EloTuneSettings {
        tune_start: 2010,
        tune_end: 2022,
        test_end: 2025,
    });
    cfg.bayes_tune = Some(EloTuneSettings {
        tune_start: 2003,
        tune_end: 2004,
        test_end: 2005,
    });
    tool.write_config(&cfg);
    assert_eq!(tool.report(&[])["split"]["tune_start"], 2003);
}

#[test]
fn cli_rejects_invalid_splits_missing_seasons_and_unfinished_history() {
    let (cfg, history) = tuning_fixture();
    assert_rejects_unusable_history(&tool(), &cfg, &history);
}

#[test]
fn cli_requires_league() {
    assert_requires_league(BIN);
}

#[test]
fn cli_reports_grid_source_evaluated_ranges_and_boundary_flags() {
    let tool = tool();
    let (mut cfg, history) = tuning_fixture();
    tool.write_history(&history);
    let run = |cfg: &LeagueConfig| -> Value {
        tool.write_config(cfg);
        tool.report(&[])
    };
    cfg.tuning_grids = Some(TuningGrids {
        elo: Some(EloGrid {
            k: vec![15.0, 25.0],
            home_advantage: vec![30.0, 50.0],
            offseason_regression: vec![0.2, 0.4],
        }),
        bayesian: None,
    });
    let elo_only = run(&cfg);
    assert_eq!(elo_only["search"]["grid_source"], "default");
    assert_eq!(
        elo_only["search"]["coarse_grid"],
        json!({
            "prior_sd_elo": [50.0, 75.0, 100.0, 150.0, 200.0, 300.0],
            "tie_prior_games": [10.0, 30.0, 100.0, 300.0, 1000.0],
            "tie_prior_rate": [0.001, 0.0025, 0.005, 0.01]
        })
    );
    assert!(elo_only["search"]["candidates_evaluated"].as_u64().unwrap() >= 120);
    assert_boundary_report(&elo_only, PARAMETERS);

    let grid = BayesianGrid {
        prior_sd_elo: vec![100.0, 200.0],
        tie_prior_games: vec![50.0, 150.0],
        tie_prior_rate: vec![0.004, 0.006],
    };
    cfg.tuning_grids = Some(TuningGrids {
        elo: None,
        bayesian: Some(grid.clone()),
    });
    let configured = run(&cfg);
    assert_eq!(configured["search"]["grid_source"], "config");
    assert_eq!(configured["search"]["coarse_grid"], serde_json::to_value(&grid).unwrap());
    assert_boundary_report(&configured, PARAMETERS);
}
