use rating_core::{
    EloSeed, Game, LeagueConfig, Outcome, SEED_SCHEMA_VERSION,
    bayesian::{fit_posterior, outcome_probabilities},
};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct Fixture {
    config: LeagueConfig,
    seed: EloSeed,
    games: Vec<Game>,
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Case {
    training_count: usize,
    means: Vec<f64>,
    covariance: Vec<Vec<f64>>,
    predictions: Vec<Value>,
}
fn fixture() -> Fixture {
    serde_json::from_str(include_str!("fixtures/bayesian-parity.json")).unwrap()
}
fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} != {b}");
}

#[test]
fn matches_browser_means_full_covariance_and_integrated_probabilities() {
    let f = fixture();
    assert_eq!(f.seed.schema_version, SEED_SCHEMA_VERSION);
    for case in f.cases {
        let model = fit_posterior(&f.seed, &f.games[..case.training_count], &f.config).unwrap();
        for (&a, b) in model.means.iter().zip(case.means) {
            close(a, b);
        }
        for (a, b) in model.covariance.iter().flatten().zip(case.covariance.iter().flatten()) {
            close(*a, *b);
        }
        for q in case.predictions {
            let actual = model
                .predict(
                    q["home"].as_str().unwrap(),
                    q["away"].as_str().unwrap(),
                    q["neutral"].as_bool().unwrap(),
                    q["phase"].as_str().unwrap(),
                )
                .unwrap();
            close(actual.home_win, q["home_win"].as_f64().unwrap());
            close(actual.away_win, q["away_win"].as_f64().unwrap());
            close(actual.tie, q["tie"].as_f64().unwrap());
            close(actual.home_win + actual.away_win + actual.tie, 1.0);
        }
    }
}

#[test]
fn equal_strength_tie_matches_analytic_hessian_and_prior_variance() {
    let mut f = fixture();
    for r in &mut f.seed.ratings {
        r.elo = f.config.elo.initial;
    }
    let mut game = f.games[0].clone();
    game.neutral = true;
    game.result = Some(Outcome::Tie);
    let prior = fit_posterior(&f.seed, &[], &f.config).unwrap();
    let variance = (f.config.bayesian.prior_sd_elo * std::f64::consts::LN_10 / f.config.elo.scale).powi(2);
    close(prior.covariance[0][0], variance);
    let model = fit_posterior(&f.seed, &[game], &f.config).unwrap();
    let precision = 1.0 / variance;
    let h = 1.0 / (2.0 * (2.0 + f.seed.tie_weight));
    let determinant = precision * precision + 2.0 * precision * h;
    close(model.means[0], 0.0);
    close(model.covariance[0][0], (precision + h) / determinant);
    close(model.covariance[0][1], h / determinant);
    close(model.covariance[2][2], variance);
}

#[test]
fn refits_are_repeatable_symmetric_and_validate_observations() {
    let mut f = fixture();
    let model = fit_posterior(&f.seed, &f.games, &f.config).unwrap();
    f.games.reverse();
    let reversed = fit_posterior(&f.seed, &f.games, &f.config).unwrap();
    for (&a, &b) in model.means.iter().zip(&reversed.means) {
        close(a, b);
    }
    let p = model.predict("ARI", "ATL", true, "regular").unwrap();
    let reverse = model.predict("ATL", "ARI", true, "regular").unwrap();
    close(p.home_win, reverse.away_win);
    close(p.tie, reverse.tie);
    assert!(model.predict("ARI", "ATL", false, "regular").unwrap().home_win > p.home_win);
    assert_eq!(model.predict("ARI", "ATL", true, "postseason").unwrap().tie, 0.0);
    assert!(model.predict("ARI", "ARI", true, "regular").is_err());
    assert!(model.predict("UNKNOWN", "ARI", true, "regular").is_err());
    let mut g = f.games[0].clone();
    assert!(fit_posterior(&f.seed, &[g.clone(), g.clone()], &f.config).is_err());
    g.result = Some(Outcome::Tie);
    g.phase = "postseason".into();
    assert!(fit_posterior(&f.seed, &[g.clone()], &f.config).is_err());
    g.phase = "regular".into();
    g.season -= 1;
    assert!(fit_posterior(&f.seed, &[g], &f.config).is_err());
    f.seed.ratings.pop();
    assert!(fit_posterior(&f.seed, &[], &f.config).is_err());
}

#[test]
fn extreme_probabilities_are_finite_and_normalized() {
    for d in [-10000.0, -100.0, 0.0, 100.0, 10000.0] {
        for nu in [0.0, 0.02] {
            let p = outcome_probabilities(d, nu);
            close(p.home_win + p.away_win + p.tie, 1.0);
            assert!(
                [p.home_win, p.away_win, p.tie]
                    .iter()
                    .all(|p| p.is_finite() && (0.0..=1.0).contains(p))
            );
        }
    }
}
