use rating_core::{
    Outcome, apply_game, replay_elo,
    walk_forward::{Preseason, SeasonPredictions, predict_season, preseason},
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use test_support::evaluation_fixture;

#[test]
fn season_elo_updates_match_the_historical_replay() {
    let (cfg, history) = evaluation_fixture();
    let Preseason { seed, games, .. } = preseason(&history, &cfg, 2005).unwrap();
    let mut ratings: BTreeMap<_, _> = seed.ratings.iter().map(|r| (r.team.clone(), r.elo)).collect();
    for g in &games {
        apply_game(&mut ratings, g, &cfg.elo).unwrap();
    }
    for r in replay_elo(&history.games, &cfg, 2005).unwrap().ratings {
        assert!((ratings[&r.team] - r.elo).abs() < 1e-9, "{}", r.team);
    }
}

#[test]
fn predictions_keep_every_game_in_start_order_with_the_preseason_tie_weight() {
    let (cfg, history) = evaluation_fixture();
    let predicted = predict_season(&history, &cfg, 2005).unwrap();
    assert_eq!(predicted.season, 2005);
    assert_eq!(predicted.tie_weight, preseason(&history, &cfg, 2005).unwrap().seed.tie_weight);
    let games: Vec<_> = predicted.games.iter().map(|p| &p.game).collect();
    let expected: Vec<_> = history.games.iter().filter(|g| g.season == 2005).collect();
    assert_eq!(games, expected);
}

#[test]
fn same_utc_day_results_never_reach_either_predictor_but_earlier_days_do() {
    let (cfg, mut history) = evaluation_fixture();
    let before = predict_season(&history, &cfg, 2005).unwrap().games;
    // The fixture's first two 2005 games are a same-day doubleheader; the third is a week later.
    let first = history.games.iter_mut().find(|g| g.id == "2005-1").unwrap();
    first.result = Some(Outcome::AwayWin);
    let after = predict_season(&history, &cfg, 2005).unwrap().games;
    for i in 0..2 {
        assert_eq!(
            before[i].bayesian.probabilities.home_win,
            after[i].bayesian.probabilities.home_win
        );
        assert_eq!(before[i].elo.probabilities.home_win, after[i].elo.probabilities.home_win);
    }
    assert_ne!(
        before[2].bayesian.probabilities.home_win,
        after[2].bayesian.probabilities.home_win
    );
    assert_ne!(before[2].elo.probabilities.home_win, after[2].elo.probabilities.home_win);
}

#[test]
fn elo_probabilities_keep_elo_odds_and_drop_ties_where_forbidden() {
    let (cfg, history) = evaluation_fixture();
    let SeasonPredictions { tie_weight, games, .. } = predict_season(&history, &cfg, 2005).unwrap();
    assert!(tie_weight > 0.0);
    for p in &games {
        let (b, e) = (p.bayesian.probabilities, p.elo.probabilities);
        assert!((b.home_win + b.away_win + b.tie - 1.0).abs() < 1e-12);
        assert!((e.home_win + e.away_win + e.tie - 1.0).abs() < 1e-12);
        assert!((b.home_win + b.tie / 2.0 - p.bayesian.expected_home_score).abs() < 1e-15);
        assert!((e.home_win / (e.home_win + e.away_win) - p.elo.expected_home_score).abs() < 1e-12);
        if p.game.phase == "postseason" {
            assert_eq!(e.tie, 0.0);
            assert_eq!(b.tie, 0.0);
        } else {
            assert!((e.tie - tie_weight / (2.0 * ((e.home_win / e.away_win).ln() / 2.0).cosh() + tie_weight)).abs() < 1e-12);
            assert!(b.tie > 0.0);
        }
    }
}

#[test]
fn predicted_games_serialize_the_history_fields_beside_flat_forecasts_and_read_back() {
    let (cfg, history) = evaluation_fixture();
    let predicted = predict_season(&history, &cfg, 2005).unwrap();
    let json = serde_json::to_value(&predicted).unwrap();
    let game = &json["games"][0];
    let keys = |v: &Value| -> BTreeSet<String> { v.as_object().unwrap().keys().cloned().collect() };
    let mut expected = keys(&serde_json::to_value(&history.games[0]).unwrap());
    expected.extend(["bayesian".into(), "elo".into()]);
    assert_eq!(keys(game), expected);
    assert_eq!(game["start_time_utc"], "2005-09-07T17:00:00Z");
    for predictor in ["bayesian", "elo"] {
        assert_eq!(
            keys(&game[predictor]),
            ["home_win", "away_win", "tie", "expected_home_score"]
                .map(String::from)
                .into(),
            "{predictor}"
        );
    }
    let read: SeasonPredictions = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(serde_json::to_value(&read).unwrap(), json);
}
