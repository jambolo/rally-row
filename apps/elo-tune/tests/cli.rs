use rating_core::{BayesianGrid, EloGrid, LeagueConfig, Outcome, TuningGrids};
use serde_json::{Value, json};
use test_support::{
    cli::{
        ToolDir, assert_boundary_report, assert_read_only_and_repeatable, assert_rejects_unusable_history, assert_requires_league,
        assert_split_flags_override_config,
    },
    tuning_fixture,
};

const BIN: &str = env!("CARGO_BIN_EXE_elo-tune");
const PARAMETERS: [&str; 3] = ["k", "home_advantage", "offseason_regression"];

fn tool() -> ToolDir {
    ToolDir::new(BIN, &[])
}

#[test]
fn cli_is_read_only_repeatable_and_selects_independently_of_heldout_outcomes() {
    let tool = tool();
    let (cfg, mut history) = tuning_fixture();
    let (report, _) = assert_read_only_and_repeatable(&tool, "elo-tuning-report", &cfg, &history);
    assert_eq!(
        report["split"],
        json!({"warmup_start": 2002, "tune_start": 2003, "tune_end": 2004, "test_end": 2005})
    );
    assert_eq!(report["tuning"]["baseline"]["games"], 8);
    assert_eq!(report["holdout"]["baseline"]["games"], 4);
    assert_eq!(report["baseline_parameters"]["k"], cfg.elo.k);
    assert!(report["search"]["candidates_evaluated"].as_u64().unwrap() >= 125);

    for g in history.games.iter_mut().filter(|g| g.season == 2005) {
        g.result = Some(Outcome::AwayWin);
    }
    tool.write_history(&history);
    let changed = tool.report(&["--test-end", "2005"]);
    assert_eq!(report["selected_parameters"], changed["selected_parameters"]);
    assert_eq!(report["search"], changed["search"]);
    assert_eq!(report["tuning"], changed["tuning"]);
    assert_ne!(report["holdout"], changed["holdout"]);

    let mut other_bayesian = cfg;
    other_bayesian.bayesian.prior_sd_elo = 500.0;
    other_bayesian.bayesian.tie_prior_games = 1.0;
    other_bayesian.bayesian.tie_prior_rate = 0.25;
    tool.write_config(&other_bayesian);
    let independent = tool.report(&["--test-end", "2005"]);
    assert_eq!(changed["search"], independent["search"]);
    assert_eq!(changed["selected_parameters"], independent["selected_parameters"]);
    assert_eq!(changed["tuning"], independent["tuning"]);
    assert_eq!(changed["holdout"], independent["holdout"]);
}

#[test]
fn cli_boundaries_override_config_and_work_without_defaults() {
    let (cfg, history) = tuning_fixture();
    assert_split_flags_override_config(&tool(), cfg, &history);
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
    let default = run(&cfg);
    assert_eq!(default["search"]["grid_source"], "default");
    assert_eq!(
        default["search"]["coarse_grid"],
        json!({
            "k": [10.0, 15.0, 20.0, 30.0, 40.0],
            "home_advantage": [0.0, 25.0, 40.0, 55.0, 70.0],
            "offseason_regression": [0.0, 0.15, 1.0 / 3.0, 0.5, 0.75]
        })
    );
    assert_boundary_report(&default, PARAMETERS);

    cfg.tuning_grids = Some(TuningGrids {
        elo: None,
        bayesian: Some(BayesianGrid {
            prior_sd_elo: vec![60.0],
            tie_prior_games: vec![20.0],
            tie_prior_rate: vec![0.002],
        }),
    });
    let bayesian_only = run(&cfg);
    assert_eq!(bayesian_only["search"], default["search"]);
    assert_eq!(bayesian_only["selected_parameters"], default["selected_parameters"]);

    let grid = EloGrid {
        k: vec![15.0, 25.0],
        home_advantage: vec![30.0, 50.0],
        offseason_regression: vec![0.2, 0.4],
    };
    cfg.tuning_grids = Some(TuningGrids {
        elo: Some(grid.clone()),
        bayesian: None,
    });
    let configured = run(&cfg);
    assert_eq!(configured["search"]["grid_source"], "config");
    assert_eq!(configured["search"]["coarse_grid"], serde_json::to_value(&grid).unwrap());
    assert_boundary_report(&configured, PARAMETERS);
}
