use anyhow::{Context, Result};
use rating_core::{
    BayesianGrid, BayesianSettings, EloSettings, GameFile, LeagueConfig, Outcome,
    bayesian::Probabilities,
    scoring::{self, CalibrationBin, calibration, standard_error},
    tuning::{Ranges, SearchParameters, Split, add_candidates, boundary_note, grid_candidates, select, validate},
    walk_forward::{Preseason, posteriors_by_utc_date, preseason},
};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Parameters {
    pub prior_sd_elo: f64,
    pub tie_prior_games: f64,
    pub tie_prior_rate: f64,
}

impl Parameters {
    fn from_settings(s: &BayesianSettings) -> Self {
        Self {
            prior_sd_elo: s.prior_sd_elo,
            tie_prior_games: s.tie_prior_games,
            tie_prior_rate: s.tie_prior_rate,
        }
    }
    fn settings(self) -> BayesianSettings {
        BayesianSettings {
            prior_sd_elo: self.prior_sd_elo,
            tie_prior_games: self.tie_prior_games,
            tie_prior_rate: self.tie_prior_rate,
        }
    }
}

impl SearchParameters for Parameters {
    const NAMES: [&'static str; 3] = ["prior_sd_elo", "tie_prior_games", "tie_prior_rate"];

    fn from_values([prior_sd_elo, tie_prior_games, tie_prior_rate]: [f64; 3]) -> Self {
        Self {
            prior_sd_elo,
            tie_prior_games,
            tie_prior_rate,
        }
    }

    fn values(self) -> [f64; 3] {
        [self.prior_sd_elo, self.tie_prior_games, self.tie_prior_rate]
    }

    fn distance(self, baseline: Self) -> f64 {
        (self.prior_sd_elo / baseline.prior_sd_elo).ln().powi(2)
            + (self.tie_prior_games / baseline.tie_prior_games).ln().powi(2)
            + (self.tie_prior_rate / baseline.tie_prior_rate).ln().powi(2)
    }
}

#[derive(Clone, Serialize)]
pub struct Grid {
    prior_sd_elo: Vec<f64>,
    tie_prior_games: Vec<f64>,
    tie_prior_rate: Vec<f64>,
}

impl Grid {
    fn coarse() -> Self {
        Self {
            prior_sd_elo: vec![50.0, 75.0, 100.0, 150.0, 200.0, 300.0],
            tie_prior_games: vec![10.0, 30.0, 100.0, 300.0, 1000.0],
            tie_prior_rate: vec![0.001, 0.0025, 0.005, 0.01],
        }
    }
    /// The league's configured starting grid if present, else the default coarse grid, with its source label.
    fn starting(configured: Option<&BayesianGrid>) -> (Self, &'static str) {
        match configured {
            Some(g) => (
                Self {
                    prior_sd_elo: g.prior_sd_elo.clone(),
                    tie_prior_games: g.tie_prior_games.clone(),
                    tie_prior_rate: g.tie_prior_rate.clone(),
                },
                "config",
            ),
            None => (Self::coarse(), "default"),
        }
    }
    fn candidates(&self) -> Vec<Parameters> {
        grid_candidates([&self.prior_sd_elo, &self.tie_prior_games, &self.tie_prior_rate])
    }
    fn around(center: Parameters) -> Self {
        let neighbors = |v: f64| vec![v / std::f64::consts::SQRT_2, v, v * std::f64::consts::SQRT_2];
        Self {
            prior_sd_elo: neighbors(center.prior_sd_elo),
            tie_prior_games: neighbors(center.tie_prior_games),
            tie_prior_rate: neighbors(center.tie_prior_rate).into_iter().filter(|r| *r < 1.0).collect(),
        }
    }
}

struct PreparedSeason {
    preseason: Preseason,
    /// Tie weights already estimated, keyed by `(tie_prior_games, tie_prior_rate)`.
    tie_weights: Vec<(f64, f64, f64)>,
}

fn prepare(history: &GameFile, cfg: &LeagueConfig, start: i32, end: i32) -> Result<Vec<PreparedSeason>> {
    (start..=end)
        .map(|season| {
            Ok(PreparedSeason {
                preseason: preseason(history, cfg, season)?,
                tie_weights: Vec::new(),
            })
        })
        .collect()
}

struct PredictionRow {
    probabilities: Probabilities,
    outcome: Outcome,
}

fn predict_season(season: &mut PreparedSeason, cfg: &LeagueConfig) -> Result<Vec<PredictionRow>> {
    let b = &cfg.bayesian;
    let weight = season
        .tie_weights
        .iter()
        .find(|(games, rate, _)| *games == b.tie_prior_games && *rate == b.tie_prior_rate);
    season.preseason.seed.tie_weight = if let Some(&(_, _, weight)) = weight {
        weight
    } else {
        let weight = season.preseason.ties.estimate(b);
        season.tie_weights.push((b.tie_prior_games, b.tie_prior_rate, weight));
        weight
    };
    let Preseason { seed, games, .. } = &season.preseason;
    let mut predictions = Vec::with_capacity(games.len());
    for fit in posteriors_by_utc_date(seed, games, cfg) {
        let (model, date_games) = fit?;
        for g in date_games {
            predictions.push(PredictionRow {
                probabilities: model.predict(&g.home_team, &g.away_team, g.neutral, &g.phase)?,
                outcome: g.result.clone().context("Cannot score an unreported game")?,
            });
        }
    }
    Ok(predictions)
}

#[derive(Clone, Serialize)]
pub struct Score {
    games: usize,
    log_loss: f64,
    brier: f64,
    expected_ties: f64,
    observed_ties: usize,
}

fn score(rows: &[PredictionRow]) -> Option<Score> {
    if rows.is_empty() {
        return None;
    }
    let mut log_loss = 0.0;
    let mut brier = 0.0;
    let mut expected_ties = 0.0;
    let mut observed_ties = 0;
    for row in rows {
        log_loss += scoring::log_loss(row.probabilities, &row.outcome);
        brier += scoring::brier(row.probabilities, &row.outcome);
        expected_ties += row.probabilities.tie;
        observed_ties += usize::from(row.outcome == Outcome::Tie);
    }
    Some(Score {
        games: rows.len(),
        log_loss: log_loss / rows.len() as f64,
        brier: brier / rows.len() as f64,
        expected_ties,
        observed_ties,
    })
}

#[derive(Clone, Serialize)]
pub struct SeasonScore {
    season: i32,
    tie_weight: f64,
    overall: Score,
    first_half: Option<Score>,
    second_half: Option<Score>,
}

#[derive(Clone, Serialize)]
pub struct Evaluation {
    games: usize,
    mean_season_log_loss: f64,
    mean_season_brier: f64,
    pooled: Score,
    seasons: Vec<SeasonScore>,
    calibration: Vec<CalibrationBin>,
}

fn evaluate(prepared: &mut [PreparedSeason], cfg: &LeagueConfig, parameters: Parameters) -> Result<Evaluation> {
    let mut cfg = cfg.clone();
    cfg.bayesian = parameters.settings();
    let mut rows = Vec::new();
    let mut seasons = Vec::new();
    for season in prepared {
        let predictions = predict_season(season, &cfg)?;
        let midpoint = predictions.len() / 2;
        seasons.push(SeasonScore {
            season: season.preseason.seed.target_season,
            tie_weight: season.preseason.seed.tie_weight,
            overall: score(&predictions).context("Empty evaluation season")?,
            first_half: score(&predictions[..midpoint]),
            second_half: score(&predictions[midpoint..]),
        });
        rows.extend(predictions);
    }
    Ok(Evaluation {
        games: rows.len(),
        mean_season_log_loss: seasons.iter().map(|s| s.overall.log_loss).sum::<f64>() / seasons.len() as f64,
        mean_season_brier: seasons.iter().map(|s| s.overall.brier).sum::<f64>() / seasons.len() as f64,
        pooled: score(&rows).context("Empty evaluation")?,
        seasons,
        calibration: calibration(&rows, |row| (row.probabilities, &row.outcome)),
    })
}

fn mean_season_log_loss(evaluation: &Evaluation) -> f64 {
    evaluation.mean_season_log_loss
}

fn season_log_loss(evaluation: &Evaluation) -> Vec<f64> {
    evaluation.seasons.iter().map(|s| s.overall.log_loss).collect()
}

#[derive(Serialize)]
pub struct SeasonDifference {
    season: i32,
    baseline_minus_selected_log_loss: f64,
}

#[derive(Serialize)]
pub struct Comparison {
    baseline: Evaluation,
    selected: Evaluation,
    mean_season_log_loss_improvement: f64,
    paired_season_standard_error: Option<f64>,
    season_differences: Vec<SeasonDifference>,
}

fn compare(baseline: Evaluation, selected: Evaluation) -> Comparison {
    let delta: Vec<_> = season_log_loss(&baseline)
        .iter()
        .zip(season_log_loss(&selected))
        .map(|(a, b)| a - b)
        .collect();
    Comparison {
        mean_season_log_loss_improvement: baseline.mean_season_log_loss - selected.mean_season_log_loss,
        paired_season_standard_error: standard_error(&delta),
        season_differences: baseline
            .seasons
            .iter()
            .zip(delta)
            .map(|(s, d)| SeasonDifference {
                season: s.season,
                baseline_minus_selected_log_loss: d,
            })
            .collect(),
        baseline,
        selected,
    }
}

#[derive(Serialize)]
pub struct CandidateSummary {
    parameters: Parameters,
    mean_season_log_loss: f64,
    mean_season_brier: f64,
}

#[derive(Serialize)]
pub struct SearchReport {
    grid_source: &'static str,
    coarse_grid: Grid,
    refinement_centers: Vec<Parameters>,
    refinement_multipliers: [f64; 3],
    candidates_evaluated: usize,
    near_best_candidates: usize,
    minimum_log_loss_parameters: Parameters,
    top_candidates: Vec<CandidateSummary>,
    evaluated_ranges: Ranges,
    selected_on_boundary: Vec<&'static str>,
}

#[derive(Serialize)]
pub struct Report {
    pub run_at: String,
    pub league: String,
    config_sha256: String,
    history_sha256: String,
    method: &'static str,
    selection_rule: &'static str,
    split: Split,
    fixed_elo: EloSettings,
    baseline_parameters: Parameters,
    selected_parameters: Parameters,
    search: SearchReport,
    tuning: Comparison,
    holdout: Comparison,
    notes: Vec<String>,
}

pub fn run(history: &GameFile, cfg: &LeagueConfig, split: Split, config_sha256: String, history_sha256: String) -> Result<Report> {
    let run_at = chrono::Utc::now().to_rfc3339();
    validate(history, cfg, split)?;
    let baseline = Parameters::from_settings(&cfg.bayesian);
    eprintln!(
        "Searching Bayesian parameters using seasons {}–{} only...",
        split.tune_start, split.tune_end
    );
    let mut prepared = prepare(history, cfg, split.tune_start, split.tune_end)?;
    let (coarse_grid, grid_source) = Grid::starting(cfg.tuning_grids.as_ref().and_then(|g| g.bayesian.as_ref()));
    let mut candidates = Vec::new();
    let mut evaluated = 0_usize;
    let mut evaluate_tuning = |parameters: Parameters| {
        let evaluation = evaluate(&mut prepared, cfg, parameters).with_context(|| format!("Evaluate {parameters:?}"))?;
        evaluated += 1;
        if evaluated.is_multiple_of(10) {
            eprintln!("Evaluated {evaluated} Bayesian candidates...");
        }
        Ok(evaluation)
    };
    add_candidates(&mut candidates, [baseline], &mut evaluate_tuning, mean_season_log_loss)?;
    add_candidates(
        &mut candidates,
        coarse_grid.candidates(),
        &mut evaluate_tuning,
        mean_season_log_loss,
    )?;
    let centers: Vec<_> = candidates.iter().take(3).map(|c| c.parameters).collect();
    for &center in &centers {
        add_candidates(
            &mut candidates,
            Grid::around(center).candidates(),
            &mut evaluate_tuning,
            mean_season_log_loss,
        )?;
    }
    let (selected, near_best_candidates) = select(&candidates, baseline, mean_season_log_loss, season_log_loss);
    let evaluated_ranges = Ranges::of(candidates.iter().map(|c| c.parameters));
    let selected_on_boundary = evaluated_ranges.boundary(selected.parameters);
    let baseline_tuning = &candidates.iter().find(|c| c.parameters == baseline).unwrap().evaluation;
    eprintln!(
        "Selected from {} candidates; evaluating held-out seasons {}–{}...",
        candidates.len(),
        split.tune_end + 1,
        split.test_end
    );
    let mut heldout = prepare(history, cfg, split.tune_end + 1, split.test_end)?;
    let baseline_holdout = evaluate(&mut heldout, cfg, baseline)?;
    let selected_holdout = if selected.parameters == baseline {
        baseline_holdout.clone()
    } else {
        evaluate(&mut heldout, cfg, selected.parameters)?
    };
    let boundary = boundary_note(&selected_on_boundary);
    Ok(Report {
        run_at,
        league: cfg.id.clone(),
        config_sha256,
        history_sha256,
        method: "Fixed Elo; preseason priors and tie weight use earlier seasons only; Laplace posterior and Simpson predictive integration; earlier UTC dates only; minimize equally weighted season log loss",
        selection_rule: "Within one paired-season standard error of minimum log loss, choose closest to current settings by sum of squared log parameter ratios; break ties by log loss",
        split,
        fixed_elo: cfg.elo.clone(),
        baseline_parameters: baseline,
        selected_parameters: selected.parameters,
        search: SearchReport {
            grid_source,
            coarse_grid,
            refinement_centers: centers,
            refinement_multipliers: [1.0 / std::f64::consts::SQRT_2, 1.0, std::f64::consts::SQRT_2],
            candidates_evaluated: candidates.len(),
            near_best_candidates,
            minimum_log_loss_parameters: candidates[0].parameters,
            top_candidates: candidates
                .iter()
                .take(10)
                .map(|c| CandidateSummary {
                    parameters: c.parameters,
                    mean_season_log_loss: c.evaluation.mean_season_log_loss,
                    mean_season_brier: c.evaluation.mean_season_brier,
                })
                .collect(),
            evaluated_ranges,
            selected_on_boundary,
        },
        tuning: compare(baseline_tuning.clone(), selected.evaluation.clone()),
        holdout: compare(baseline_holdout, selected_holdout),
        notes: [
            "Offline: configuration and history are read once; published seeds are unused; inputs are never changed.",
            "Tie weight is reestimated for each season from earlier history and remains fixed within that season. Postseason follows the configured tie rules.",
            "Rare ties can leave tie smoothing weakly identified. The conservative selection is a stability heuristic, not a significance test.",
            "The grid and one local refinement are bounded; the minimum is not a guarantee of a global optimum.",
            "Log loss uses natural logs and a 1e-15 probability floor; Brier score sums squared errors across all three outcomes.",
            "First/second halves divide chronologically ordered games by count. Calibration bins are per outcome; expected/observed tie totals are also reported.",
            "Positive baseline-minus-selected log loss indicates improvement; paired-season standard errors are descriptive and seasons may be dependent.",
            "Only the frozen selection and current settings are evaluated on the holdout. Earlier holdout outcomes can train later holdout predictions, but never change selected parameters.",
            "Holdout validity also requires that these seasons did not influence Elo settings or previous search choices. Repeated search changes after inspecting holdout scores invalidate it.",
        ]
        .into_iter()
        .map(String::from)
        .chain(boundary)
        .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rating_core::tuning::Candidate;

    #[test]
    fn scores_all_three_outcomes_and_weights_seasons_equally() {
        let rows = [PredictionRow {
            probabilities: Probabilities {
                home_win: 0.5,
                away_win: 0.3,
                tie: 0.2,
            },
            outcome: Outcome::Tie,
        }];
        let s = score(&rows).unwrap();
        assert!((s.log_loss + 0.2_f64.ln()).abs() < 1e-12);
        assert!((s.brier - 0.98).abs() < 1e-12);
        assert_eq!(s.expected_ties, 0.2);
        assert_eq!(s.observed_ties, 1);
        let (cfg, mut history) = test_support::tuning_fixture();
        history.games.retain(|g| g.season != 2004 || g.round != 2);
        let mut prepared = prepare(&history, &cfg, 2003, 2004).unwrap();
        let result = evaluate(&mut prepared, &cfg, Parameters::from_settings(&cfg.bayesian)).unwrap();
        assert_eq!(result.games, 7);
        let a = result.seasons[0].overall.log_loss;
        let b = result.seasons[1].overall.log_loss;
        assert_eq!(result.mean_season_log_loss, (a + b) / 2.0);
        assert!((result.pooled.log_loss - (4.0 * a + 3.0 * b) / 7.0).abs() < 1e-12);
        assert_eq!(
            result
                .calibration
                .iter()
                .filter(|b| b.outcome == Outcome::Tie)
                .map(|b| b.games)
                .sum::<usize>(),
            7
        );
    }

    #[test]
    fn same_utc_day_and_future_results_cannot_leak_into_predictions_or_preseason_tie_weight() {
        let (cfg, mut history) = test_support::tuning_fixture();
        let mut games = history.games.iter_mut().filter(|g| g.season == 2003);
        games.next().unwrap().start_time_utc = "2003-09-07T23:30:00-04:00".parse().unwrap();
        games.next().unwrap().start_time_utc = "2003-09-08T22:00:00Z".parse().unwrap();
        let mut prepared = prepare(&history, &cfg, 2003, 2003).unwrap();
        let before = predict_season(&mut prepared[0], &cfg).unwrap();
        let tie_weight = prepared[0].preseason.seed.tie_weight;
        prepared[0].preseason.games[0].result = Some(Outcome::AwayWin);
        prepared[0].preseason.games[1].result = Some(Outcome::HomeWin);
        let after = predict_season(&mut prepared[0], &cfg).unwrap();
        assert_eq!(prepared[0].preseason.seed.tie_weight, tie_weight);
        for i in 0..2 {
            assert_eq!(before[i].probabilities.home_win, after[i].probabilities.home_win);
            assert_eq!(before[i].probabilities.tie, after[i].probabilities.tie);
        }
        assert_ne!(before[2].probabilities.home_win, after[2].probabilities.home_win);
        assert_eq!(before[3].probabilities.tie, 0.0);
        for g in history.games.iter_mut().filter(|g| g.season >= 2003) {
            g.result = Some(Outcome::AwayWin);
        }
        let changed = prepare(&history, &cfg, 2003, 2003).unwrap();
        assert_eq!(changed[0].preseason.seed.tie_weight, tie_weight);
        assert_eq!(
            serde_json::to_value(&changed[0].preseason.seed.ratings).unwrap(),
            serde_json::to_value(&prepared[0].preseason.seed.ratings).unwrap()
        );
    }

    #[test]
    fn selection_keeps_defaults_in_a_flat_region_and_moves_for_consistent_improvement() {
        let (cfg, history) = test_support::tuning_fixture();
        let baseline = Parameters::from_settings(&cfg.bayesian);
        let mut prepared = prepare(&history, &cfg, 2003, 2004).unwrap();
        let mut best = evaluate(&mut prepared, &cfg, baseline).unwrap();
        for s in &mut best.seasons {
            s.overall.log_loss = 0.5;
        }
        best.mean_season_log_loss = 0.5;
        let mut near = best.clone();
        near.seasons[1].overall.log_loss = 0.6;
        near.mean_season_log_loss = 0.55;
        let mut candidates = vec![
            Candidate {
                parameters: Parameters {
                    prior_sd_elo: 100.0,
                    ..baseline
                },
                evaluation: best,
            },
            Candidate {
                parameters: baseline,
                evaluation: near,
            },
        ];
        let selected = |candidates: &[Candidate<Parameters, Evaluation>]| {
            select(candidates, baseline, mean_season_log_loss, season_log_loss)
                .0
                .parameters
        };
        assert_eq!(selected(&candidates), baseline);
        for s in &mut candidates[1].evaluation.seasons {
            s.overall.log_loss = 0.55;
        }
        assert_eq!(selected(&candidates).prior_sd_elo, 100.0);
        assert_eq!(Grid::coarse().candidates().len(), 120);
        assert!(Grid::coarse().candidates().contains(&baseline));
    }

    #[test]
    fn default_starting_grid_keeps_the_previous_constants_and_a_configured_grid_replaces_it() {
        let (grid, source) = Grid::starting(None);
        assert_eq!(source, "default");
        assert_eq!(grid.prior_sd_elo, [50.0, 75.0, 100.0, 150.0, 200.0, 300.0]);
        assert_eq!(grid.tie_prior_games, [10.0, 30.0, 100.0, 300.0, 1000.0]);
        assert_eq!(grid.tie_prior_rate, [0.001, 0.0025, 0.005, 0.01]);
        assert_eq!(grid.candidates().len(), 120);
        let configured = BayesianGrid {
            prior_sd_elo: vec![60.0, 120.0],
            tie_prior_games: vec![20.0, 40.0, 80.0],
            tie_prior_rate: vec![0.002, 0.004],
        };
        let (grid, source) = Grid::starting(Some(&configured));
        assert_eq!(source, "config");
        assert_eq!(grid.prior_sd_elo, configured.prior_sd_elo);
        assert_eq!(grid.tie_prior_games, configured.tie_prior_games);
        assert_eq!(grid.tie_prior_rate, configured.tie_prior_rate);
        assert_eq!(grid.candidates().len(), 12);
    }

    #[test]
    fn search_names_and_values_follow_the_serialized_parameters() {
        let p = Parameters {
            prior_sd_elo: 150.0,
            tie_prior_games: 100.0,
            tie_prior_rate: 0.005,
        };
        let fields: Vec<_> = Parameters::NAMES
            .iter()
            .zip(p.values())
            .map(|(name, value)| format!("\"{name}\":{value:?}"))
            .collect();
        assert_eq!(serde_json::to_string(&p).unwrap(), format!("{{{}}}", fields.join(",")));
        assert_eq!(Parameters::from_values(p.values()), p);
    }
}
