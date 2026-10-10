use crate::{GameFile, HISTORY_SCHEMA_VERSION, LeagueConfig, TuningSettings, adapter_for, scoring::standard_error, validate_games};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize, Serializer};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Split {
    pub warmup_start: i32,
    pub tune_start: i32,
    pub tune_end: i32,
    pub test_end: i32,
}

impl Split {
    /// Seasons given on the command line override the first configured `defaults` entry. Each entry pairs a config
    /// field name with that field's settings; the names guide the error for a missing season.
    pub fn resolve(
        cfg: &LeagueConfig,
        tune_start: Option<i32>,
        tune_end: Option<i32>,
        test_end: Option<i32>,
        defaults: &[(&str, Option<&TuningSettings>)],
    ) -> Result<Self> {
        let configured = defaults.iter().find_map(|&(_, settings)| settings);
        let season = |given: Option<i32>, field: &str, value: fn(&TuningSettings) -> i32| {
            given.or(configured.map(value)).with_context(|| {
                let mut names = defaults.iter().map(|(name, _)| format!("{name}.{field}"));
                let first = names.next().unwrap_or_default();
                let others: String = names.map(|name| format!(" (or {name})")).collect();
                format!("Set {first}{others} in the config or pass --{}", field.replace('_', "-"))
            })
        };
        Ok(Self {
            warmup_start: cfg.history_start,
            tune_start: season(tune_start, "tune_start", |s| s.tune_start)?,
            tune_end: season(tune_end, "tune_end", |s| s.tune_end)?,
            test_end: season(test_end, "test_end", |s| s.test_end)?,
        })
    }
}

pub fn validate(history: &GameFile, cfg: &LeagueConfig, split: Split) -> Result<()> {
    cfg.validate()?;
    ensure!(
        split.warmup_start == cfg.history_start
            && split.warmup_start < split.tune_start
            && split.tune_start < split.tune_end
            && split.tune_end < split.test_end
            && split.test_end < cfg.current_season(),
        "Require warm-up history, at least two tuning seasons, and later completed held-out seasons"
    );
    ensure!(
        history.schema_version == HISTORY_SCHEMA_VERSION && history.league == cfg.id && history.teams == cfg.teams,
        "History schema, league, or franchise identities do not match configuration; rerun import-history"
    );
    ensure!(
        history.from_season == cfg.history_start && history.through_season >= split.test_end,
        "History must cover {} through {}; rerun import-history with --through-season {}",
        cfg.history_start,
        split.test_end,
        split.test_end
    );
    ensure!(
        history
            .games
            .iter()
            .all(|g| (history.from_season..=history.through_season).contains(&g.season)),
        "Game is outside the history file's declared seasons"
    );
    let mut games = history.games.clone();
    validate_games(&mut games, cfg)?;
    ensure!(
        games
            .iter()
            .filter(|g| g.season <= split.test_end)
            .all(|g| g.result.is_some()),
        "Evaluation history contains an unreported game; use completed historical seasons"
    );
    let adapter = adapter_for(&cfg.source.kind)?;
    for season in cfg.history_start..=split.test_end {
        ensure!(
            games.iter().any(|g| g.season == season),
            "Missing completed historical season {season}"
        );
        if let Some(reason) = adapter.season_incomplete(&games, season) {
            bail!("Season {season} has {reason}; refresh history before tuning");
        }
    }
    Ok(())
}

/// The settings a tuner searches: three parameters, named in report key order.
pub trait SearchParameters: Copy + PartialEq {
    const NAMES: [&'static str; 3];
    fn from_values(values: [f64; 3]) -> Self;
    fn values(self) -> [f64; 3];
    /// How far `self` moves from the current settings; the selection rule prefers small moves.
    fn distance(self, baseline: Self) -> f64;
}

/// Every combination of the axis values, with the last axis varying fastest.
pub fn grid_candidates<P: SearchParameters>(axes: [&[f64]; 3]) -> Vec<P> {
    let mut result = Vec::with_capacity(axes.iter().map(|axis| axis.len()).product());
    for &a in axes[0] {
        for &b in axes[1] {
            for &c in axes[2] {
                result.push(P::from_values([a, b, c]));
            }
        }
    }
    result
}

pub struct Candidate<P, E> {
    pub parameters: P,
    pub evaluation: E,
}

/// Evaluates each parameter set not already among `candidates`, then sorts every candidate by `score`, lowest first.
pub fn add_candidates<P: SearchParameters, E>(
    candidates: &mut Vec<Candidate<P, E>>,
    parameters: impl IntoIterator<Item = P>,
    mut evaluate: impl FnMut(P) -> Result<E>,
    score: impl Fn(&E) -> f64,
) -> Result<()> {
    for parameters in parameters {
        if !candidates.iter().any(|c| c.parameters == parameters) {
            let evaluation = evaluate(parameters)?;
            candidates.push(Candidate { parameters, evaluation });
        }
    }
    candidates.sort_by(|a, b| score(&a.evaluation).total_cmp(&score(&b.evaluation)));
    Ok(())
}

/// The selection rule. Among candidates whose `score` is within one paired-season standard error of the minimum,
/// choose the one closest to `baseline`, breaking ties by `score`. `candidates` must be sorted by `score`, and
/// `season_scores` lists an evaluation's per-season scores in season order. Returns the choice and the number of
/// candidates within the margin.
pub fn select<P: SearchParameters, E>(
    candidates: &[Candidate<P, E>],
    baseline: P,
    score: impl Fn(&E) -> f64,
    season_scores: impl Fn(&E) -> Vec<f64>,
) -> (&Candidate<P, E>, usize) {
    let best = &candidates[0];
    let best_seasons = season_scores(&best.evaluation);
    // Paired season variation is a stability heuristic, not a significance test after searching.
    let eligible: Vec<_> = candidates
        .iter()
        .filter(|c| {
            let differences: Vec<_> = season_scores(&c.evaluation)
                .iter()
                .zip(&best_seasons)
                .map(|(c, b)| c - b)
                .collect();
            score(&c.evaluation) - score(&best.evaluation) <= standard_error(&differences).unwrap_or(0.0) + 1e-12
        })
        .collect();
    let selected = eligible
        .iter()
        .copied()
        .min_by(|a, b| {
            a.parameters
                .distance(baseline)
                .total_cmp(&b.parameters.distance(baseline))
                .then_with(|| score(&a.evaluation).total_cmp(&score(&b.evaluation)))
        })
        .unwrap();
    (selected, eligible.len())
}

/// Inclusive `[min, max]` of each parameter over every evaluated candidate, serialized by parameter name.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ranges {
    names: [&'static str; 3],
    pub bounds: [[f64; 2]; 3],
}

impl Ranges {
    pub fn of<P: SearchParameters>(evaluated: impl IntoIterator<Item = P>) -> Self {
        let bounds = evaluated
            .into_iter()
            .fold([[f64::INFINITY, f64::NEG_INFINITY]; 3], |bounds, p| {
                let values = p.values();
                std::array::from_fn(|i| [bounds[i][0].min(values[i]), bounds[i][1].max(values[i])])
            });
        Self { names: P::NAMES, bounds }
    }

    /// Parameter names, in report key order, whose selected value equals its evaluated minimum or maximum.
    pub fn boundary<P: SearchParameters>(&self, selected: P) -> Vec<&'static str> {
        self.names
            .into_iter()
            .zip(selected.values())
            .zip(self.bounds)
            .filter(|&((_, value), [lo, hi])| value == lo || value == hi)
            .map(|((name, _), _)| name)
            .collect()
    }
}

impl Serialize for Ranges {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_map(self.names.iter().zip(&self.bounds))
    }
}

pub fn boundary_note(names: &[&str]) -> Option<String> {
    (!names.is_empty()).then(|| {
        format!(
            "Selected parameters lie on a searched boundary ({}); extend tuning_grids in the league configuration and rerun before adopting.",
            names.join(", ")
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Point([f64; 3]);

    impl SearchParameters for Point {
        const NAMES: [&'static str; 3] = ["a", "b", "c"];
        fn from_values(values: [f64; 3]) -> Self {
            Self(values)
        }
        fn values(self) -> [f64; 3] {
            self.0
        }
        fn distance(self, baseline: Self) -> f64 {
            self.0.iter().zip(baseline.0).map(|(v, b)| (v - b).powi(2)).sum()
        }
    }

    /// Mean season score and per-season scores.
    type Scores = (f64, Vec<f64>);

    fn candidate(values: [f64; 3], seasons: &[f64]) -> Candidate<Point, Scores> {
        Candidate {
            parameters: Point(values),
            evaluation: (seasons.iter().sum::<f64>() / seasons.len() as f64, seasons.to_vec()),
        }
    }

    fn select_from(candidates: &[Candidate<Point, Scores>], baseline: Point) -> (Point, usize) {
        let (selected, eligible) = select(candidates, baseline, |e| e.0, |e| e.1.clone());
        (selected.parameters, eligible)
    }

    #[test]
    fn grid_varies_the_last_axis_fastest() {
        let points: Vec<Point> = grid_candidates([&[1.0, 2.0], &[3.0], &[4.0, 5.0]]);
        let values: Vec<_> = points.iter().map(|p| p.0).collect();
        assert_eq!(values, [[1.0, 3.0, 4.0], [1.0, 3.0, 5.0], [2.0, 3.0, 4.0], [2.0, 3.0, 5.0]]);
    }

    #[test]
    fn adding_skips_evaluated_parameters_and_sorts_by_score() {
        let mut candidates = Vec::new();
        let mut evaluated = 0;
        let mut evaluate = |p: Point| {
            evaluated += 1;
            Ok((p.0[0], vec![p.0[0]]))
        };
        add_candidates(&mut candidates, [Point([2.0, 0.0, 0.0])], &mut evaluate, |e| e.0).unwrap();
        add_candidates(
            &mut candidates,
            [Point([3.0, 0.0, 0.0]), Point([2.0, 0.0, 0.0]), Point([1.0, 0.0, 0.0])],
            &mut evaluate,
            |e| e.0,
        )
        .unwrap();
        assert_eq!(evaluated, 3);
        let order: Vec<_> = candidates.iter().map(|c| c.parameters.0[0]).collect();
        assert_eq!(order, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn selection_prefers_the_baseline_on_a_flat_region_but_not_for_consistently_worse_scores() {
        let baseline = Point([20.0, 55.0, 1.0 / 3.0]);
        let mut candidates = vec![
            candidate([30.0, 55.0, 1.0 / 3.0], &[0.0, 0.0]),
            candidate(baseline.0, &[0.0, 0.01]),
        ];
        assert_eq!(select_from(&candidates, baseline), (baseline, 2));
        candidates[1] = candidate(baseline.0, &[0.01, 0.01]);
        assert_eq!(select_from(&candidates, baseline), (Point([30.0, 55.0, 1.0 / 3.0]), 1));
    }

    #[test]
    fn boundary_detection_flags_minimum_and_maximum_and_ignores_interior_values() {
        let ranges = Ranges::of([Point([10.0, 40.0, 0.5]), Point([20.0, 0.0, 0.25]), Point([5.0, 70.0, 1.0])]);
        assert_eq!(ranges.bounds, [[5.0, 20.0], [0.0, 70.0], [0.25, 1.0]]);
        assert_eq!(
            serde_json::to_string(&ranges).unwrap(),
            r#"{"a":[5.0,20.0],"b":[0.0,70.0],"c":[0.25,1.0]}"#
        );
        assert!(ranges.boundary(Point([10.0, 40.0, 0.5])).is_empty());
        assert_eq!(ranges.boundary(Point([5.0, 40.0, 1.0])), ["a", "c"]);
        assert_eq!(ranges.boundary(Point([20.0, 0.0, 0.5])), ["a", "b"]);
        assert_eq!(boundary_note(&[]), None);
        assert_eq!(
            boundary_note(&["a", "c"]).unwrap(),
            "Selected parameters lie on a searched boundary (a, c); extend tuning_grids in the league configuration and rerun before adopting."
        );
    }
}
