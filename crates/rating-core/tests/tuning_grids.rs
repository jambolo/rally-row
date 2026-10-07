use rating_core::{BayesianGrid, EloGrid, EloTuneSettings, LeagueConfig, TuningGrids};
use serde_json::{Value, json};
use test_support::league_config_json;

fn nfl() -> Value {
    league_config_json("nfl")
}

fn grids() -> TuningGrids {
    TuningGrids {
        elo: Some(EloGrid {
            k: vec![10.0, 15.0, 20.0, 30.0, 40.0],
            home_advantage: vec![0.0, 25.0, 40.0, 55.0, 70.0],
            offseason_regression: vec![0.0, 0.15, 1.0 / 3.0, 0.5, 0.75],
        }),
        bayesian: Some(BayesianGrid {
            prior_sd_elo: vec![50.0, 75.0, 100.0, 150.0, 200.0, 300.0],
            tie_prior_games: vec![10.0, 30.0, 100.0, 300.0, 1000.0],
            tie_prior_rate: vec![0.001, 0.0025, 0.005, 0.01],
        }),
    }
}

fn with_grids(grids: TuningGrids) -> LeagueConfig {
    let mut cfg: LeagueConfig = serde_json::from_value(nfl()).unwrap();
    cfg.tuning_grids = Some(grids);
    cfg
}

fn assert_rejected(case: &str, mutate: impl FnOnce(&mut TuningGrids)) {
    let mut g = grids();
    mutate(&mut g);
    let error = with_grids(g).validate().unwrap_err().to_string();
    assert_eq!(error, "Invalid tuning grid", "{case}");
}

#[test]
fn config_without_tuning_grids_validates_and_serializes_without_the_key() {
    let cfg: LeagueConfig = serde_json::from_value(nfl()).unwrap();
    assert!(cfg.tuning_grids.is_none());
    cfg.validate().unwrap();
    assert!(!serde_json::to_string(&cfg).unwrap().contains("tuning_grids"));
}

#[test]
fn accepts_full_partial_and_empty_grids_and_round_trips_them() {
    let full = grids();
    let cases = [
        full.clone(),
        TuningGrids {
            elo: full.elo.clone(),
            bayesian: None,
        },
        TuningGrids {
            elo: None,
            bayesian: full.bayesian.clone(),
        },
        TuningGrids {
            elo: None,
            bayesian: None,
        },
    ];
    for g in cases {
        let cfg = with_grids(g.clone());
        cfg.validate().unwrap();
        let value = serde_json::to_value(&cfg).unwrap();
        let grids = value["tuning_grids"].as_object().unwrap();
        assert_eq!(grids.contains_key("elo"), g.elo.is_some());
        assert_eq!(grids.contains_key("bayesian"), g.bayesian.is_some());
        let back: LeagueConfig = serde_json::from_value(value).unwrap();
        assert_eq!(back.tuning_grids, Some(g));
    }
}

#[test]
fn serializes_tuning_grids_last_after_bayes_tune() {
    let mut cfg = with_grids(grids());
    cfg.bayes_tune = Some(EloTuneSettings {
        tune_start: 2010,
        tune_end: 2022,
        test_end: 2025,
    });
    let s = serde_json::to_string(&cfg).unwrap();
    let pos = |k: &str| s.find(k).unwrap_or_else(|| panic!("missing {k}"));
    assert!(pos("\"bayesian\":{\"prior_sd_elo\"") < pos("\"elo_tune\":"));
    assert!(pos("\"elo_tune\":") < pos("\"bayes_tune\":"));
    assert!(pos("\"bayes_tune\":") < pos("\"tuning_grids\":"));
}

#[test]
fn reads_integer_json_grids_and_accepts_domain_limits() {
    let mut v = nfl();
    v["tuning_grids"] = json!({
        "elo": {"k": [1, 4, 8], "home_advantage": [0, 24], "offseason_regression": [0, 1]},
        "bayesian": {"prior_sd_elo": [1], "tie_prior_games": [0.5], "tie_prior_rate": [0.001, 0.999]}
    });
    let cfg: LeagueConfig = serde_json::from_value(v).unwrap();
    cfg.validate().unwrap();
    let elo = cfg.tuning_grids.unwrap().elo.unwrap();
    assert_eq!(elo.k, [1.0, 4.0, 8.0]);
    assert_eq!(elo.offseason_regression, [0.0, 1.0]);
}

#[test]
fn rejects_empty_unsorted_non_finite_and_out_of_domain_grids() {
    assert_rejected("empty k", |g| g.elo.as_mut().unwrap().k.clear());
    assert_rejected("descending k", |g| g.elo.as_mut().unwrap().k = vec![20.0, 10.0]);
    assert_rejected("repeated k", |g| g.elo.as_mut().unwrap().k = vec![10.0, 10.0]);
    assert_rejected("NaN k", |g| g.elo.as_mut().unwrap().k = vec![f64::NAN]);
    assert_rejected("zero k", |g| g.elo.as_mut().unwrap().k = vec![0.0, 10.0]);
    assert_rejected("infinite home advantage", |g| {
        g.elo.as_mut().unwrap().home_advantage = vec![0.0, f64::INFINITY]
    });
    assert_rejected("negative home advantage", |g| {
        g.elo.as_mut().unwrap().home_advantage = vec![-1.0, 25.0]
    });
    assert_rejected("negative regression", |g| {
        g.elo.as_mut().unwrap().offseason_regression = vec![-0.1, 0.5]
    });
    assert_rejected("regression above one", |g| {
        g.elo.as_mut().unwrap().offseason_regression = vec![0.5, 1.5]
    });
    assert_rejected("empty tie rate", |g| g.bayesian.as_mut().unwrap().tie_prior_rate.clear());
    assert_rejected("descending prior sd", |g| {
        g.bayesian.as_mut().unwrap().prior_sd_elo = vec![100.0, 50.0]
    });
    assert_rejected("NaN tie games", |g| {
        g.bayesian.as_mut().unwrap().tie_prior_games = vec![10.0, f64::NAN]
    });
    assert_rejected("zero prior sd", |g| {
        g.bayesian.as_mut().unwrap().prior_sd_elo = vec![0.0, 50.0]
    });
    assert_rejected("zero tie games", |g| {
        g.bayesian.as_mut().unwrap().tie_prior_games = vec![0.0, 10.0]
    });
    assert_rejected("zero tie rate", |g| {
        g.bayesian.as_mut().unwrap().tie_prior_rate = vec![0.0, 0.01]
    });
    assert_rejected("tie rate of one", |g| {
        g.bayesian.as_mut().unwrap().tie_prior_rate = vec![0.5, 1.0]
    });
    let mut v = nfl();
    v["tuning_grids"] = json!({"bayesian": {"prior_sd_elo": [], "tie_prior_games": [10], "tie_prior_rate": [0.01]}});
    let error = serde_json::from_value::<LeagueConfig>(v).unwrap().validate().unwrap_err();
    assert_eq!(error.to_string(), "Invalid tuning grid");
}
