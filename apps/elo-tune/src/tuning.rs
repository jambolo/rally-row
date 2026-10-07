use anyhow::Result;
use rating_core::{
    Audit, EloGrid, EloSettings, GameFile, LeagueConfig, replay_elo,
    scoring::standard_error,
    tuning::{Ranges, SearchParameters, Split, add_candidates, boundary_note, grid_candidates, select, validate},
};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Parameters {
    pub k: f64,
    pub home_advantage: f64,
    pub offseason_regression: f64,
}

impl Parameters {
    fn from_settings(settings: &EloSettings) -> Self {
        Self {
            k: settings.k,
            home_advantage: settings.home_advantage,
            offseason_regression: settings.offseason_regression,
        }
    }
}

impl SearchParameters for Parameters {
    const NAMES: [&'static str; 3] = ["k", "home_advantage", "offseason_regression"];

    fn from_values([k, home_advantage, offseason_regression]: [f64; 3]) -> Self {
        Self {
            k,
            home_advantage,
            offseason_regression,
        }
    }

    fn values(self) -> [f64; 3] {
        [self.k, self.home_advantage, self.offseason_regression]
    }

    fn distance(self, baseline: Self) -> f64 {
        ((self.k - baseline.k) / baseline.k).powi(2)
            + ((self.home_advantage - baseline.home_advantage) / 25.0).powi(2)
            + ((self.offseason_regression - baseline.offseason_regression) / 0.25).powi(2)
    }
}

#[derive(Clone, Serialize)]
pub struct Score {
    pub games: usize,
    pub mse: f64,
}

#[derive(Clone, Serialize)]
pub struct SeasonScore {
    pub season: i32,
    pub overall: Score,
    pub first_half: Option<Score>,
    pub second_half: Option<Score>,
}

#[derive(Clone, Serialize)]
pub struct CalibrationBin {
    pub lower: f64,
    pub upper: f64,
    pub games: usize,
    pub mean_expected_score: f64,
    pub mean_observed_score: f64,
}

#[derive(Clone, Serialize)]
pub struct Evaluation {
    pub games: usize,
    pub mean_season_mse: f64,
    pub pooled_mse: f64,
    pub seasons: Vec<SeasonScore>,
    pub calibration: Vec<CalibrationBin>,
}

fn mean_season_mse(evaluation: &Evaluation) -> f64 {
    evaluation.mean_season_mse
}

fn season_mse(evaluation: &Evaluation) -> Vec<f64> {
    evaluation.seasons.iter().map(|s| s.overall.mse).collect()
}

#[derive(Serialize)]
pub struct SeasonDifference {
    pub season: i32,
    pub baseline_minus_selected_mse: f64,
}

#[derive(Serialize)]
pub struct Comparison {
    pub baseline: Evaluation,
    pub selected: Evaluation,
    pub mean_season_mse_improvement: f64,
    pub relative_improvement_percent: Option<f64>,
    pub paired_season_standard_error: Option<f64>,
    pub season_differences: Vec<SeasonDifference>,
}

#[derive(Clone, Serialize)]
pub struct Grid {
    k: Vec<f64>,
    home_advantage: Vec<f64>,
    offseason_regression: Vec<f64>,
}

impl Grid {
    fn coarse() -> Self {
        Self {
            k: vec![10.0, 15.0, 20.0, 30.0, 40.0],
            home_advantage: vec![0.0, 25.0, 40.0, 55.0, 70.0],
            offseason_regression: vec![0.0, 0.15, 1.0 / 3.0, 0.5, 0.75],
        }
    }

    /// The league's configured starting grid if present, else the default coarse grid, with its source label.
    fn starting(configured: Option<&EloGrid>) -> (Self, &'static str) {
        match configured {
            Some(g) => (
                Self {
                    k: g.k.clone(),
                    home_advantage: g.home_advantage.clone(),
                    offseason_regression: g.offseason_regression.clone(),
                },
                "config",
            ),
            None => (Self::coarse(), "default"),
        }
    }

    fn candidates(&self) -> Vec<Parameters> {
        grid_candidates([&self.k, &self.home_advantage, &self.offseason_regression])
    }

    fn expand(&mut self, best: Parameters) -> bool {
        let mut changed = false;
        if best.k == self.k[0] {
            self.k.insert(0, best.k / 2.0);
            changed = true;
        } else if best.k == *self.k.last().unwrap() {
            self.k.push(best.k * 2.0);
            changed = true;
        }
        if best.home_advantage == *self.home_advantage.last().unwrap() {
            self.home_advantage.push(best.home_advantage + 25.0);
            changed = true;
        }
        if best.offseason_regression == *self.offseason_regression.last().unwrap() && best.offseason_regression < 1.0 {
            self.offseason_regression.push(1.0);
            changed = true;
        }
        changed
    }
}

#[derive(Serialize)]
pub struct CandidateSummary {
    pub parameters: Parameters,
    pub mean_season_mse: f64,
}

#[derive(Serialize)]
pub struct SearchReport {
    pub grid_source: &'static str,
    pub coarse_grid: Grid,
    pub expanded_grid: Grid,
    pub expansion_rounds: usize,
    pub refinement_steps: Parameters,
    pub candidates_evaluated: usize,
    pub near_best_candidates: usize,
    pub minimum_mse_parameters: Parameters,
    pub top_candidates: Vec<CandidateSummary>,
    pub evaluated_ranges: Ranges,
    pub selected_on_boundary: Vec<&'static str>,
}

#[derive(Serialize)]
pub struct Report {
    pub run_at: String,
    pub league: String,
    pub config_sha256: String,
    pub history_sha256: String,
    pub method: &'static str,
    pub selection_rule: &'static str,
    pub split: Split,
    pub initial_elo: f64,
    pub elo_scale: f64,
    pub baseline_parameters: Parameters,
    pub selected_parameters: Parameters,
    pub search: SearchReport,
    pub tuning: Comparison,
    pub holdout: Comparison,
    pub notes: Vec<String>,
}

fn score(rows: &[&Audit]) -> Option<Score> {
    (!rows.is_empty()).then(|| Score {
        games: rows.len(),
        mse: rows
            .iter()
            .map(|g| (g.expected_home_score - g.observed_home_score).powi(2))
            .sum::<f64>()
            / rows.len() as f64,
    })
}

fn evaluate(audit: &[Audit], start: i32, end: i32) -> Evaluation {
    let rows: Vec<_> = audit.iter().filter(|g| (start..=end).contains(&g.season)).collect();
    let mut by_season: BTreeMap<i32, Vec<&Audit>> = BTreeMap::new();
    let mut bins: [Vec<&Audit>; 10] = std::array::from_fn(|_| Vec::new());
    for &g in &rows {
        by_season.entry(g.season).or_default().push(g);
        bins[((g.expected_home_score * 10.0) as usize).min(9)].push(g);
    }
    let seasons: Vec<_> = by_season
        .into_iter()
        .map(|(season, games)| {
            let midpoint = games.len() / 2;
            SeasonScore {
                season,
                overall: score(&games).unwrap(),
                first_half: score(&games[..midpoint]),
                second_half: score(&games[midpoint..]),
            }
        })
        .collect();
    Evaluation {
        games: rows.len(),
        mean_season_mse: seasons.iter().map(|s| s.overall.mse).sum::<f64>() / seasons.len() as f64,
        pooled_mse: score(&rows).unwrap().mse,
        seasons,
        calibration: bins
            .iter()
            .enumerate()
            .filter(|(_, games)| !games.is_empty())
            .map(|(index, games)| CalibrationBin {
                lower: index as f64 / 10.0,
                upper: (index + 1) as f64 / 10.0,
                games: games.len(),
                mean_expected_score: games.iter().map(|g| g.expected_home_score).sum::<f64>() / games.len() as f64,
                mean_observed_score: games.iter().map(|g| g.observed_home_score).sum::<f64>() / games.len() as f64,
            })
            .collect(),
    }
}

fn evaluate_parameters(history: &GameFile, cfg: &LeagueConfig, parameters: Parameters, start: i32, end: i32) -> Result<Evaluation> {
    let mut candidate = cfg.clone();
    candidate.elo.k = parameters.k;
    candidate.elo.home_advantage = parameters.home_advantage;
    candidate.elo.offseason_regression = parameters.offseason_regression;
    let replay = replay_elo(&history.games, &candidate, end)?;
    Ok(evaluate(&replay.audit, start, end))
}

fn compare(baseline: Evaluation, selected: Evaluation) -> Comparison {
    let differences: Vec<_> = season_mse(&baseline)
        .iter()
        .zip(season_mse(&selected))
        .map(|(a, b)| a - b)
        .collect();
    let improvement = baseline.mean_season_mse - selected.mean_season_mse;
    Comparison {
        mean_season_mse_improvement: improvement,
        relative_improvement_percent: (baseline.mean_season_mse > 0.0).then(|| 100.0 * improvement / baseline.mean_season_mse),
        paired_season_standard_error: standard_error(&differences),
        season_differences: baseline
            .seasons
            .iter()
            .zip(differences)
            .map(|(s, difference)| SeasonDifference {
                season: s.season,
                baseline_minus_selected_mse: difference,
            })
            .collect(),
        baseline,
        selected,
    }
}

pub fn run(history: &GameFile, cfg: &LeagueConfig, split: Split, config_sha256: String, history_sha256: String) -> Result<Report> {
    let run_at = chrono::Utc::now().to_rfc3339();
    validate(history, cfg, split)?;
    let baseline = Parameters::from_settings(&cfg.elo);
    let (mut grid, grid_source) = Grid::starting(cfg.tuning_grids.as_ref().and_then(|g| g.elo.as_ref()));
    let coarse_grid = grid.clone();
    let mut candidates = Vec::new();
    let evaluate_tuning = |parameters| evaluate_parameters(history, cfg, parameters, split.tune_start, split.tune_end);
    eprintln!(
        "Searching Elo parameters using seasons {}–{} only...",
        split.tune_start, split.tune_end
    );
    add_candidates(&mut candidates, [baseline], evaluate_tuning, mean_season_mse)?;
    add_candidates(&mut candidates, grid.candidates(), evaluate_tuning, mean_season_mse)?;
    let mut expansion_rounds = 0;
    while expansion_rounds < 4 && grid.expand(candidates[0].parameters) {
        expansion_rounds += 1;
        add_candidates(&mut candidates, grid.candidates(), evaluate_tuning, mean_season_mse)?;
    }
    let boundary_limited = grid.clone().expand(candidates[0].parameters);
    let steps = Parameters {
        k: 2.5,
        home_advantage: 5.0,
        offseason_regression: 0.05,
    };
    let centers: Vec<_> = candidates.iter().take(3).map(|c| c.parameters).collect();
    for center in centers {
        let local = Grid {
            k: (-2..=2)
                .map(|i| center.k + f64::from(i) * steps.k)
                .filter(|&k| k > 0.0)
                .collect(),
            home_advantage: (-2..=2)
                .map(|i| center.home_advantage + f64::from(i) * steps.home_advantage)
                .filter(|&h| h >= 0.0)
                .collect(),
            offseason_regression: (-2..=2)
                .map(|i| center.offseason_regression + f64::from(i) * steps.offseason_regression)
                .filter(|r| (0.0..=1.0).contains(r))
                .collect(),
        };
        add_candidates(&mut candidates, local.candidates(), evaluate_tuning, mean_season_mse)?;
    }
    let (selected, near_best_candidates) = select(&candidates, baseline, mean_season_mse, season_mse);
    let evaluated_ranges = Ranges::of(candidates.iter().map(|c| c.parameters));
    let selected_on_boundary = evaluated_ranges.boundary(selected.parameters);
    let baseline_tuning = &candidates.iter().find(|c| c.parameters == baseline).unwrap().evaluation;
    eprintln!(
        "Selected parameters from {} candidates; evaluating held-out seasons {}–{}...",
        candidates.len(),
        split.tune_end + 1,
        split.test_end
    );
    // Only the frozen selection and existing settings ever reach the held-out evaluator.
    let baseline_holdout = evaluate_parameters(history, cfg, baseline, split.tune_end + 1, split.test_end)?;
    let selected_holdout = if selected.parameters == baseline {
        baseline_holdout.clone()
    } else {
        evaluate_parameters(history, cfg, selected.parameters, split.tune_end + 1, split.test_end)?
    };
    let mut notes = vec![
        "No Bayesian fitting or tie-probability estimation. Elo ties score 0.5; this MSE is not multiclass Brier score or log loss.".into(),
        "The initial rating and Elo scale remain fixed. Inputs are read once; there are no downloads or configuration/data writes.".into(),
        "First/second halves split each season's chronologically ordered games by count, including postseason games.".into(),
        "Calibration bins compare mean expected fractional score with mean observed fractional score, not win frequency.".into(),
        "Positive baseline-minus-selected MSE indicates improvement. Paired season standard errors are descriptive; few held-out seasons and dependent seasons limit inference.".into(),
        "Held-out results do not change the selection. Repeatedly revising the search after inspecting them would invalidate the holdout.".into(),
    ];
    if boundary_limited {
        notes.push("The coarse minimum still touched an expandable boundary after four expansions; the search is bounded, not a global optimum guarantee.".into());
    }
    notes.extend(boundary_note(&selected_on_boundary));
    Ok(Report {
        run_at,
        league: cfg.id.clone(),
        config_sha256,
        history_sha256,
        method: "Chronological Elo replay; score before updating; regress once per season boundary; minimize equally weighted season mean squared errors",
        selection_rule: "Among candidates within one paired-season standard error of the minimum MSE, choose closest to current settings by squared distance scaled by current K, 25 home Elo, and 0.25 regression; break ties by MSE",
        split,
        initial_elo: cfg.elo.initial,
        elo_scale: cfg.elo.scale,
        baseline_parameters: baseline,
        selected_parameters: selected.parameters,
        search: SearchReport {
            grid_source,
            coarse_grid,
            expanded_grid: grid,
            expansion_rounds,
            refinement_steps: steps,
            candidates_evaluated: candidates.len(),
            near_best_candidates,
            minimum_mse_parameters: candidates[0].parameters,
            top_candidates: candidates
                .iter()
                .take(10)
                .map(|c| CandidateSummary {
                    parameters: c.parameters,
                    mean_season_mse: c.evaluation.mean_season_mse,
                })
                .collect(),
            evaluated_ranges,
            selected_on_boundary,
        },
        tuning: compare(baseline_tuning.clone(), selected.evaluation.clone()),
        holdout: compare(baseline_holdout, selected_holdout),
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rating_core::tuning::Candidate;

    fn row(season: i32, expected: f64, observed: f64) -> Audit {
        Audit {
            game_id: String::new(),
            season,
            home_before: 1500.0,
            away_before: 1500.0,
            expected_home_score: expected,
            observed_home_score: observed,
            home_after: 1500.0,
            away_after: 1500.0,
        }
    }

    #[test]
    fn mse_uses_fractional_ties_excludes_warmup_and_weights_seasons_equally() {
        let audit = [
            row(2009, 0.0, 1.0),
            row(2010, 0.5, 0.5),
            row(2011, 0.0, 1.0),
            row(2011, 0.0, 1.0),
        ];
        let report = evaluate(&audit, 2010, 2011);
        assert_eq!(report.games, 3);
        assert_eq!(report.mean_season_mse, 0.5);
        assert!((report.pooled_mse - 2.0 / 3.0).abs() < 1e-12);
        assert_eq!(report.seasons[0].overall.mse, 0.0);
        assert!(report.seasons[0].first_half.is_none());
        assert_eq!(report.seasons[1].first_half.as_ref().unwrap().mse, 1.0);
        assert_eq!(report.calibration[1].mean_observed_score, 0.5);
        assert_eq!(report.calibration[0].games, 2);
    }

    #[test]
    fn selection_prefers_defaults_on_a_flat_region_but_not_for_consistently_worse_scores() {
        let baseline = Parameters {
            k: 20.0,
            home_advantage: 55.0,
            offseason_regression: 1.0 / 3.0,
        };
        let mut candidates = vec![
            Candidate {
                parameters: Parameters { k: 30.0, ..baseline },
                evaluation: evaluate(&[row(2010, 0.0, 0.0), row(2011, 0.0, 0.0)], 2010, 2011),
            },
            Candidate {
                parameters: baseline,
                evaluation: evaluate(&[row(2010, 0.0, 0.0), row(2011, 0.1, 0.0)], 2010, 2011),
            },
        ];
        let selected = |candidates: &[Candidate<Parameters, Evaluation>]| {
            let (selected, eligible) = select(candidates, baseline, mean_season_mse, season_mse);
            (selected.parameters, eligible)
        };
        assert_eq!(selected(&candidates), (baseline, 2));
        candidates[1].evaluation = evaluate(&[row(2010, 0.1, 0.0), row(2011, 0.1, 0.0)], 2010, 2011);
        assert_eq!(selected(&candidates).0.k, 30.0);
    }

    #[test]
    fn coarse_grid_and_boundary_expansion_cover_the_proposed_search() {
        let mut grid = Grid::coarse();
        assert_eq!(grid.candidates().len(), 125);
        assert!(grid.expand(Parameters {
            k: 10.0,
            home_advantage: 70.0,
            offseason_regression: 0.75,
        }));
        assert_eq!(grid.k[0], 5.0);
        assert_eq!(*grid.home_advantage.last().unwrap(), 95.0);
        assert_eq!(*grid.offseason_regression.last().unwrap(), 1.0);
    }

    #[test]
    fn default_starting_grid_keeps_the_previous_constants_and_a_configured_grid_replaces_it() {
        let (grid, source) = Grid::starting(None);
        assert_eq!(source, "default");
        assert_eq!(grid.k, [10.0, 15.0, 20.0, 30.0, 40.0]);
        assert_eq!(grid.home_advantage, [0.0, 25.0, 40.0, 55.0, 70.0]);
        assert_eq!(grid.offseason_regression, [0.0, 0.15, 1.0 / 3.0, 0.5, 0.75]);
        assert_eq!(grid.candidates().len(), 125);
        let configured = EloGrid {
            k: vec![2.0, 4.0, 8.0],
            home_advantage: vec![12.0, 24.0],
            offseason_regression: vec![0.25, 0.5],
        };
        let (grid, source) = Grid::starting(Some(&configured));
        assert_eq!(source, "config");
        assert_eq!(grid.k, configured.k);
        assert_eq!(grid.home_advantage, configured.home_advantage);
        assert_eq!(grid.offseason_regression, configured.offseason_regression);
        assert_eq!(grid.candidates().len(), 12);
    }

    #[test]
    fn search_names_and_values_follow_the_serialized_parameters() {
        let p = Parameters {
            k: 20.0,
            home_advantage: 55.0,
            offseason_regression: 0.25,
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
