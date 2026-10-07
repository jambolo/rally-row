use anyhow::{Context, Result};
use rating_core::{
    BayesianSettings, EloSettings, GameFile, LeagueConfig, Outcome, apply_game,
    bayesian::{Probabilities, outcome_probabilities},
    expected_home,
    scoring::{self, CalibrationBin, calibration, mean, standard_error},
    tuning::{Split, validate},
    walk_forward::{Preseason, posteriors_by_utc_date, preseason},
};
use serde::Serialize;
use std::{cmp::Ordering, collections::BTreeMap, fmt::Write as _};

/// One method's pregame forecast for a game.
#[derive(Clone, Copy)]
struct Prediction {
    probabilities: Probabilities,
    /// The method's expected fractional home score: win 1, tie 1/2, loss 0.
    expected_home_score: f64,
}

impl Prediction {
    fn from_probabilities(probabilities: Probabilities) -> Self {
        Self {
            probabilities,
            expected_home_score: probabilities.home_win + probabilities.tie / 2.0,
        }
    }

    fn favorite(&self) -> Ordering {
        self.probabilities.home_win.total_cmp(&self.probabilities.away_win)
    }
}

/// Every method's forecast for one held-out game, all made from the same pregame information.
struct GameRow {
    season: i32,
    outcome: Outcome,
    bayesian: Prediction,
    elo: Prediction,
    equal_strength: Prediction,
}

#[derive(Clone, Copy)]
enum Method {
    Bayesian,
    Elo,
    EqualStrength,
}

impl Method {
    fn prediction(self, row: &GameRow) -> &Prediction {
        match self {
            Method::Bayesian => &row.bayesian,
            Method::Elo => &row.elo,
            Method::EqualStrength => &row.equal_strength,
        }
    }
}

/// Predicts every game of `season` from earlier UTC dates only; returns the season's tie weight and the rows.
fn predict_season(history: &GameFile, cfg: &LeagueConfig, season: i32) -> Result<(f64, Vec<GameRow>)> {
    // Both methods start from the same preseason Elo ratings and tie weight, built from earlier seasons only.
    let Preseason { seed, games, .. } = preseason(history, cfg, season)?;
    let mut ratings: BTreeMap<_, _> = seed.ratings.iter().map(|r| (r.team.clone(), r.elo)).collect();
    let factor = std::f64::consts::LN_10 / cfg.elo.scale;
    let mut rows = Vec::with_capacity(games.len());
    for fit in posteriors_by_utc_date(&seed, &games, cfg) {
        let (model, date_games) = fit?;
        for g in date_games {
            let nu = if cfg.ties_allowed_in.contains(&g.phase) {
                seed.tie_weight
            } else {
                0.0
            };
            let (home, away) = (ratings[&g.home_team], ratings[&g.away_team]);
            let advantage = if g.neutral { 0.0 } else { cfg.elo.home_advantage };
            rows.push(GameRow {
                season,
                outcome: g.result.clone().context("Cannot score an unreported game")?,
                bayesian: Prediction::from_probabilities(model.predict(&g.home_team, &g.away_team, g.neutral, &g.phase)?),
                elo: Prediction {
                    probabilities: outcome_probabilities(factor * (home - away + advantage), nu),
                    expected_home_score: expected_home(home, away, g.neutral, &cfg.elo),
                },
                equal_strength: Prediction::from_probabilities(outcome_probabilities(0.0, nu)),
            });
        }
        // Elo learns from a date only after all of its games are predicted, matching the Bayesian information set.
        for g in date_games {
            apply_game(&mut ratings, g, &cfg.elo)?;
        }
    }
    Ok((seed.tie_weight, rows))
}

#[derive(Clone, Copy)]
struct GameLoss {
    log_loss: f64,
    brier: f64,
    squared_error: f64,
    /// Credit for the favored team in a decisive game: 1 if it won, 0 if it lost, 1/2 for a toss-up; `None` for a tie.
    pick: Option<f64>,
}

fn loss(prediction: &Prediction, outcome: &Outcome) -> GameLoss {
    let p = prediction.probabilities;
    let credit = |winner: f64, loser: f64| match winner.total_cmp(&loser) {
        Ordering::Greater => 1.0,
        Ordering::Less => 0.0,
        Ordering::Equal => 0.5,
    };
    GameLoss {
        log_loss: scoring::log_loss(p, outcome),
        brier: scoring::brier(p, outcome),
        squared_error: (prediction.expected_home_score - outcome.home_score()).powi(2),
        pick: match outcome {
            Outcome::HomeWin => Some(credit(p.home_win, p.away_win)),
            Outcome::AwayWin => Some(credit(p.away_win, p.home_win)),
            Outcome::Tie => None,
        },
    }
}

#[derive(Clone, Serialize)]
pub struct Score {
    games: usize,
    log_loss: f64,
    brier: f64,
    expected_score_mse: f64,
    decisive_games: usize,
    correct_picks: f64,
    accuracy: Option<f64>,
    expected_ties: f64,
    observed_ties: usize,
}

fn score(rows: &[GameRow], method: Method) -> Option<Score> {
    if rows.is_empty() {
        return None;
    }
    let mut s = Score {
        games: rows.len(),
        log_loss: 0.0,
        brier: 0.0,
        expected_score_mse: 0.0,
        decisive_games: 0,
        correct_picks: 0.0,
        accuracy: None,
        expected_ties: 0.0,
        observed_ties: 0,
    };
    for row in rows {
        let prediction = method.prediction(row);
        let l = loss(prediction, &row.outcome);
        s.log_loss += l.log_loss;
        s.brier += l.brier;
        s.expected_score_mse += l.squared_error;
        if let Some(credit) = l.pick {
            s.decisive_games += 1;
            s.correct_picks += credit;
        }
        s.expected_ties += prediction.probabilities.tie;
        s.observed_ties += usize::from(row.outcome == Outcome::Tie);
    }
    let n = rows.len() as f64;
    s.log_loss /= n;
    s.brier /= n;
    s.expected_score_mse /= n;
    s.accuracy = (s.decisive_games > 0).then(|| s.correct_picks / s.decisive_games as f64);
    Some(s)
}

#[derive(Clone, Copy)]
enum Metric {
    LogLoss,
    Brier,
    ExpectedScoreMse,
    Accuracy,
}

impl Metric {
    const ALL: [Metric; 4] = [Metric::LogLoss, Metric::Brier, Metric::ExpectedScoreMse, Metric::Accuracy];

    fn name(self) -> &'static str {
        match self {
            Metric::LogLoss => "log_loss",
            Metric::Brier => "brier",
            Metric::ExpectedScoreMse => "expected_score_mse",
            Metric::Accuracy => "accuracy",
        }
    }

    fn higher_is_better(self) -> bool {
        matches!(self, Metric::Accuracy)
    }

    fn of_score(self, s: &Score) -> Option<f64> {
        match self {
            Metric::LogLoss => Some(s.log_loss),
            Metric::Brier => Some(s.brier),
            Metric::ExpectedScoreMse => Some(s.expected_score_mse),
            Metric::Accuracy => s.accuracy,
        }
    }

    fn of_loss(self, l: &GameLoss) -> Option<f64> {
        match self {
            Metric::LogLoss => Some(l.log_loss),
            Metric::Brier => Some(l.brier),
            Metric::ExpectedScoreMse => Some(l.squared_error),
            Metric::Accuracy => l.pick,
        }
    }

    /// Positive when the Bayesian value is better than the Elo value.
    fn bayesian_advantage(self, bayesian: f64, elo: f64) -> f64 {
        if self.higher_is_better() {
            bayesian - elo
        } else {
            elo - bayesian
        }
    }
}

#[derive(Clone, Serialize)]
pub struct SeasonScore {
    season: i32,
    overall: Score,
    first_half: Option<Score>,
    second_half: Option<Score>,
}

/// Equally weighted means of the per-season scores.
#[derive(Clone, Serialize)]
pub struct MeanSeason {
    log_loss: f64,
    brier: f64,
    expected_score_mse: f64,
    accuracy: Option<f64>,
}

#[derive(Clone, Serialize)]
pub struct Evaluation {
    games: usize,
    mean_season: MeanSeason,
    pooled: Score,
    seasons: Vec<SeasonScore>,
    calibration: Vec<CalibrationBin>,
}

fn evaluate(rows: &[GameRow], method: Method) -> Result<Evaluation> {
    let seasons = rows
        .chunk_by(|a, b| a.season == b.season)
        .map(|games| {
            let midpoint = games.len() / 2;
            SeasonScore {
                season: games[0].season,
                overall: score(games, method).unwrap(),
                first_half: score(&games[..midpoint], method),
                second_half: score(&games[midpoint..], method),
            }
        })
        .collect::<Vec<_>>();
    let mean_of = |metric: Metric| mean(&seasons.iter().filter_map(|s| metric.of_score(&s.overall)).collect::<Vec<_>>());
    Ok(Evaluation {
        games: rows.len(),
        mean_season: MeanSeason {
            log_loss: mean_of(Metric::LogLoss).context("Empty evaluation")?,
            brier: mean_of(Metric::Brier).context("Empty evaluation")?,
            expected_score_mse: mean_of(Metric::ExpectedScoreMse).context("Empty evaluation")?,
            accuracy: mean_of(Metric::Accuracy),
        },
        pooled: score(rows, method).context("Empty evaluation")?,
        seasons,
        calibration: calibration(rows, |row| (method.prediction(row).probabilities, &row.outcome)),
    })
}

#[derive(Serialize)]
pub struct SeasonAdvantage {
    season: i32,
    bayesian_advantage: Option<f64>,
}

/// Paired Bayesian-versus-Elo difference in one metric; positive `bayesian_advantage` means Bayesian scored better.
#[derive(Serialize)]
pub struct MetricComparison {
    metric: &'static str,
    higher_is_better: bool,
    bayesian_advantage_mean_season: Option<f64>,
    paired_season_standard_error: Option<f64>,
    bayesian_advantage_pooled: Option<f64>,
    paired_game_standard_error: Option<f64>,
    seasons: Vec<SeasonAdvantage>,
}

#[derive(Default, Serialize)]
pub struct PickAgreement {
    decisive_games: usize,
    same_favorite: usize,
    different_favorite: usize,
    bayesian_correct_when_different: f64,
    elo_correct_when_different: f64,
}

#[derive(Serialize)]
pub struct Comparison {
    metrics: Vec<MetricComparison>,
    picks: PickAgreement,
    mean_absolute_home_win_difference: f64,
    max_absolute_home_win_difference: f64,
}

fn compare(rows: &[GameRow], bayesian: &Evaluation, elo: &Evaluation) -> Comparison {
    let losses: Vec<_> = rows
        .iter()
        .map(|row| (loss(&row.bayesian, &row.outcome), loss(&row.elo, &row.outcome)))
        .collect();
    let metrics = Metric::ALL
        .into_iter()
        .map(|metric| {
            let seasons: Vec<_> = bayesian
                .seasons
                .iter()
                .zip(&elo.seasons)
                .map(|(b, e)| SeasonAdvantage {
                    season: b.season,
                    bayesian_advantage: metric
                        .of_score(&b.overall)
                        .zip(metric.of_score(&e.overall))
                        .map(|(b, e)| metric.bayesian_advantage(b, e)),
                })
                .collect();
            let by_season: Vec<_> = seasons.iter().filter_map(|s| s.bayesian_advantage).collect();
            let by_game: Vec<_> = losses
                .iter()
                .filter_map(|(b, e)| metric.of_loss(b).zip(metric.of_loss(e)))
                .map(|(b, e)| metric.bayesian_advantage(b, e))
                .collect();
            MetricComparison {
                metric: metric.name(),
                higher_is_better: metric.higher_is_better(),
                bayesian_advantage_mean_season: mean(&by_season),
                paired_season_standard_error: standard_error(&by_season),
                bayesian_advantage_pooled: mean(&by_game),
                paired_game_standard_error: standard_error(&by_game),
                seasons,
            }
        })
        .collect();
    let mut picks = PickAgreement::default();
    for (row, (b, e)) in rows.iter().zip(&losses) {
        let (Some(b), Some(e)) = (b.pick, e.pick) else { continue };
        picks.decisive_games += 1;
        if row.bayesian.favorite() == row.elo.favorite() {
            picks.same_favorite += 1;
        } else {
            picks.different_favorite += 1;
            picks.bayesian_correct_when_different += b;
            picks.elo_correct_when_different += e;
        }
    }
    let differences: Vec<_> = rows
        .iter()
        .map(|row| (row.bayesian.probabilities.home_win - row.elo.probabilities.home_win).abs())
        .collect();
    Comparison {
        metrics,
        picks,
        mean_absolute_home_win_difference: mean(&differences).unwrap_or(0.0),
        max_absolute_home_win_difference: differences.into_iter().fold(0.0, f64::max),
    }
}

#[derive(Serialize)]
pub struct Predictors {
    bayesian: Evaluation,
    elo: Evaluation,
    equal_strength: Evaluation,
}

#[derive(Serialize)]
pub struct SeasonTieWeight {
    season: i32,
    tie_weight: f64,
}

#[derive(Serialize)]
pub struct Report {
    pub run_at: String,
    pub league: String,
    config_sha256: String,
    history_sha256: String,
    method: &'static str,
    split: Split,
    holdout_seasons: [i32; 2],
    elo_settings: EloSettings,
    bayesian_settings: BayesianSettings,
    tie_weights: Vec<SeasonTieWeight>,
    predictors: Predictors,
    comparison: Comparison,
    notes: Vec<String>,
}

pub fn run(history: &GameFile, cfg: &LeagueConfig, split: Split, config_sha256: String, history_sha256: String) -> Result<Report> {
    let run_at = chrono::Utc::now().to_rfc3339();
    validate(history, cfg, split)?;
    let (first, last) = (split.tune_end + 1, split.test_end);
    eprintln!("Evaluating held-out seasons {first}–{last} with the configured settings...");
    let mut rows = Vec::new();
    let mut tie_weights = Vec::new();
    for season in first..=last {
        let (tie_weight, predicted) = predict_season(history, cfg, season)?;
        eprintln!("Predicted {} games in season {season}", predicted.len());
        tie_weights.push(SeasonTieWeight { season, tie_weight });
        rows.extend(predicted);
    }
    let bayesian = evaluate(&rows, Method::Bayesian)?;
    let elo = evaluate(&rows, Method::Elo)?;
    let comparison = compare(&rows, &bayesian, &elo);
    let mut notes: Vec<String> = [
        "Offline: predictions use the league configuration's (tuned) Elo and Bayesian settings.",
        "Each held-out season starts from preseason Elo ratings and a tie weight built from earlier seasons only; both methods share them.",
        "Bayesian: each UTC date is predicted from a fresh posterior fit to earlier dates of the season, as in bayes-tune.",
        "Elo: ratings update after every game in start order, as in the historical replay, but each UTC date is predicted from the ratings at the start of that date, so neither method sees same-day outcomes. elo-tune also updates between games on the same date, so its held-out MSE can differ slightly.",
        "Elo three-way probabilities apply the Davidson tie term (the season's tie weight, zero where ties are not allowed) to the pregame Elo difference, keeping Elo's win odds in decisive games. Elo's expected score is its logistic expected score; the other methods use P(home win) + P(tie)/2.",
        "Equal strength: no home advantage, the non-tie probability split evenly, and the season's tie weight at equal strength, as in the Node backtest.",
        "Accuracy counts decisive games whose favored team won (an exact toss-up earns 1/2); ties are excluded from accuracy.",
        "Log loss uses natural logs and a 1e-15 probability floor; Brier score sums squared errors across all three outcomes; expected-score MSE compares the expected fractional home score with the observed score (win 1, tie 1/2, loss 0).",
        "Positive bayesian_advantage means the Bayesian predictions scored better (lower loss or higher accuracy). Standard errors are descriptive: held-out seasons are few, and games within a season share team-strength errors.",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    let tuned_through = cfg.elo_tune.iter().chain(&cfg.bayes_tune).map(|s| s.tune_end).max();
    if let Some(end) = tuned_through.filter(|&end| end >= first) {
        notes.push(format!(
            "Seasons {first}–{} lie within a configured tuning range, so they are not fully held out from parameter selection.",
            end.min(last)
        ));
    }
    Ok(Report {
        run_at,
        league: cfg.id.clone(),
        config_sha256,
        history_sha256,
        method: "Configured settings; preseason priors and tie weight from earlier seasons only; Bayesian Laplace posterior refit per UTC date versus Elo ratings updated after each game; both predict each UTC date from earlier dates only",
        split,
        holdout_seasons: [first, last],
        elo_settings: cfg.elo.clone(),
        bayesian_settings: cfg.bayesian.clone(),
        tie_weights,
        predictors: Predictors {
            equal_strength: evaluate(&rows, Method::EqualStrength)?,
            bayesian,
            elo,
        },
        comparison,
        notes,
    })
}

/// Human-readable digest of the report; the default stdout output.
pub fn summary(report: &Report) -> Result<String, std::fmt::Error> {
    let percent = |v: Option<f64>| v.map_or_else(|| "n/a".to_owned(), |v| format!("{:.1}%", 100.0 * v));
    let p = &report.predictors;
    let [first, last] = report.holdout_seasons;
    let mut out = String::new();
    writeln!(
        out,
        "League {}: held-out seasons {first}–{last}, {} games",
        report.league, p.bayesian.games
    )?;
    writeln!(
        out,
        "{:<18}{:>10}{:>10}{:>16}{:>10}",
        "Mean of seasons", "Log loss", "Brier", "Exp. score MSE", "Accuracy"
    )?;
    for (name, e) in [
        ("Bayesian", &p.bayesian),
        ("Elo", &p.elo),
        ("Equal strength", &p.equal_strength),
    ] {
        let m = &e.mean_season;
        writeln!(
            out,
            "{name:<18}{:>10.4}{:>10.4}{:>16.4}{:>10}",
            m.log_loss,
            m.brier,
            m.expected_score_mse,
            percent(m.accuracy)
        )?;
    }
    writeln!(
        out,
        "Bayesian advantage over Elo (positive favors Bayesian), mean of seasons ± paired-season SE:"
    )?;
    for c in &report.comparison.metrics {
        let value = c
            .bayesian_advantage_mean_season
            .map_or_else(|| "n/a".to_owned(), |v| format!("{v:+.4}"));
        let se = c
            .paired_season_standard_error
            .map_or_else(|| "n/a".to_owned(), |v| format!("{v:.4}"));
        writeln!(out, "  {:<20}{value} ± {se}", c.metric)?;
    }
    writeln!(
        out,
        "{:<8}{:>22}{:>22}",
        "Season", "Log loss (Bayes/Elo)", "Accuracy (Bayes/Elo)"
    )?;
    for (b, e) in p.bayesian.seasons.iter().zip(&p.elo.seasons) {
        writeln!(
            out,
            "{:<8}{:>22}{:>22}",
            b.season,
            format!("{:.4} / {:.4}", b.overall.log_loss, e.overall.log_loss),
            format!("{} / {}", percent(b.overall.accuracy), percent(e.overall.accuracy))
        )?;
    }
    let picks = &report.comparison.picks;
    writeln!(
        out,
        "Favorites agree in {} of {} decisive games. In the other {}, the Bayesian favorite won {} and the Elo favorite won {}.",
        picks.same_favorite,
        picks.decisive_games,
        picks.different_favorite,
        picks.bayesian_correct_when_different,
        picks.elo_correct_when_different
    )?;
    writeln!(
        out,
        "Home-win probabilities differ by {:.1} points on average (at most {:.1}).",
        100.0 * report.comparison.mean_absolute_home_win_difference,
        100.0 * report.comparison.max_absolute_home_win_difference
    )?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rating_core::replay_elo;

    fn prediction(home_win: f64, away_win: f64, tie: f64) -> Prediction {
        Prediction::from_probabilities(Probabilities { home_win, away_win, tie })
    }

    fn row(season: i32, outcome: Outcome, bayesian: Prediction, elo: Prediction) -> GameRow {
        GameRow {
            season,
            outcome,
            bayesian,
            elo,
            equal_strength: prediction(0.5, 0.5, 0.0),
        }
    }

    #[test]
    fn season_elo_updates_match_the_historical_replay() {
        let (cfg, history) = test_support::evaluation_fixture();
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
    fn same_utc_day_results_never_reach_either_method_but_earlier_days_do() {
        let (cfg, mut history) = test_support::evaluation_fixture();
        let (_, before) = predict_season(&history, &cfg, 2005).unwrap();
        // The fixture's first two 2005 games are a same-day doubleheader; the third is a week later.
        let first = history.games.iter_mut().find(|g| g.id == "2005-1").unwrap();
        first.result = Some(Outcome::AwayWin);
        let (_, after) = predict_season(&history, &cfg, 2005).unwrap();
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
        let (cfg, history) = test_support::evaluation_fixture();
        let (tie_weight, rows) = predict_season(&history, &cfg, 2005).unwrap();
        assert!(tie_weight > 0.0);
        for (row, game) in rows.iter().zip(history.games.iter().filter(|g| g.season == 2005)) {
            let p = row.elo.probabilities;
            assert!((p.home_win + p.away_win + p.tie - 1.0).abs() < 1e-12);
            assert!((p.home_win / (p.home_win + p.away_win) - row.elo.expected_home_score).abs() < 1e-12);
            let e = row.equal_strength.probabilities;
            assert_eq!(e.home_win, e.away_win);
            assert!((row.equal_strength.expected_home_score - 0.5).abs() < 1e-15);
            if game.phase == "postseason" {
                assert_eq!(p.tie, 0.0);
                assert_eq!(row.bayesian.probabilities.tie, 0.0);
                assert_eq!(e.tie, 0.0);
            } else {
                assert!((p.tie - tie_weight / (2.0 * ((p.home_win / p.away_win).ln() / 2.0).cosh() + tie_weight)).abs() < 1e-12);
                assert!((e.tie - tie_weight / (2.0 + tie_weight)).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn scores_every_metric_and_leaves_ties_out_of_accuracy() {
        let rows = [
            row(2005, Outcome::Tie, prediction(0.5, 0.3, 0.2), prediction(0.5, 0.5, 0.0)),
            row(2005, Outcome::HomeWin, prediction(0.6, 0.4, 0.0), prediction(0.5, 0.5, 0.0)),
            row(2005, Outcome::AwayWin, prediction(0.7, 0.3, 0.0), prediction(0.5, 0.5, 0.0)),
        ];
        let s = score(&rows, Method::Bayesian).unwrap();
        assert!((s.log_loss - -(0.2_f64.ln() + 0.6_f64.ln() + 0.3_f64.ln()) / 3.0).abs() < 1e-12);
        assert!((s.brier - (0.98 + 0.32 + 0.98) / 3.0).abs() < 1e-12);
        assert!((s.expected_score_mse - (0.1_f64.powi(2) + 0.4_f64.powi(2) + 0.7_f64.powi(2)) / 3.0).abs() < 1e-12);
        assert_eq!((s.decisive_games, s.correct_picks, s.accuracy), (2, 1.0, Some(0.5)));
        assert!((s.expected_ties - 0.2).abs() < 1e-12);
        assert_eq!(s.observed_ties, 1);
        let toss_up = score(&rows, Method::Elo).unwrap();
        assert_eq!((toss_up.correct_picks, toss_up.accuracy), (1.0, Some(0.5)));
        assert_eq!(score(&rows[..1], Method::Bayesian).unwrap().accuracy, None);
        assert!(score(&rows[..0], Method::Bayesian).is_none());
    }

    #[test]
    fn comparison_is_positive_when_bayesian_scores_better_and_tallies_split_favorites() {
        let rows = [
            row(2004, Outcome::HomeWin, prediction(0.7, 0.3, 0.0), prediction(0.4, 0.6, 0.0)),
            row(2004, Outcome::AwayWin, prediction(0.3, 0.7, 0.0), prediction(0.4, 0.6, 0.0)),
            row(2005, Outcome::HomeWin, prediction(0.8, 0.2, 0.0), prediction(0.6, 0.4, 0.0)),
            row(2005, Outcome::Tie, prediction(0.45, 0.45, 0.1), prediction(0.5, 0.5, 0.0)),
        ];
        let bayesian = evaluate(&rows, Method::Bayesian).unwrap();
        let elo = evaluate(&rows, Method::Elo).unwrap();
        assert_eq!(bayesian.seasons.len(), 2);
        assert_eq!(bayesian.mean_season.accuracy, Some(1.0));
        assert_eq!(elo.mean_season.accuracy, Some(0.75));
        let comparison = compare(&rows, &bayesian, &elo);
        for c in &comparison.metrics {
            assert!(c.bayesian_advantage_mean_season.unwrap() > 0.0, "{}", c.metric);
            assert!(c.bayesian_advantage_pooled.unwrap() > 0.0, "{}", c.metric);
            assert_eq!(c.seasons.len(), 2);
        }
        let accuracy = comparison.metrics.iter().find(|c| c.metric == "accuracy").unwrap();
        assert_eq!(accuracy.seasons[0].bayesian_advantage, Some(0.5));
        assert_eq!(accuracy.seasons[1].bayesian_advantage, Some(0.0));
        assert_eq!(accuracy.bayesian_advantage_pooled, Some(1.0 / 3.0));
        let picks = &comparison.picks;
        assert_eq!(
            (picks.decisive_games, picks.same_favorite, picks.different_favorite),
            (3, 2, 1)
        );
        assert_eq!(
            (picks.bayesian_correct_when_different, picks.elo_correct_when_different),
            (1.0, 0.0)
        );
        assert!((comparison.max_absolute_home_win_difference - 0.3).abs() < 1e-12);
        assert!((comparison.mean_absolute_home_win_difference - (0.3 + 0.1 + 0.2 + 0.05) / 4.0).abs() < 1e-12);
    }
}
