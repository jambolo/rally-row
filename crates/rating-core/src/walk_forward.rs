//! Predicting a completed season one UTC date at a time from earlier information only, as Bayesian tuning, model
//! evaluation, and the held-out season simulation do.

use crate::{
    EloSeed, Game, GameFile, LeagueConfig, apply_game,
    bayesian::{Posterior, Probabilities, TieHistory, fit_posterior, outcome_probabilities},
    chronological, expected_home, seed_with_ties,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{cmp::Ordering, collections::BTreeMap};

/// A season to predict, prepared from the seasons before it.
pub struct Preseason {
    /// Preseason Elo priors and tie weight from the earlier seasons, without the replay audit.
    pub seed: EloSeed,
    /// Tie history of the earlier seasons; it re-estimates the tie weight under other Bayesian settings.
    pub ties: TieHistory,
    /// The season's games in start order.
    pub games: Vec<Game>,
}

/// Builds `season`'s preseason seed from the earlier seasons in `history`, as `build_seed` does for
/// `elo-ratings`, and collects the season's games in start order.
pub fn preseason(history: &GameFile, cfg: &LeagueConfig, season: i32) -> Result<Preseason> {
    let previous = GameFile {
        through_season: season - 1,
        games: history.games.iter().filter(|g| g.season < season).cloned().collect(),
        ..history.clone()
    };
    let (mut seed, ties) = seed_with_ties(&previous, b"", cfg, b"", season)?;
    seed.audit.clear();
    let mut games: Vec<_> = history.games.iter().filter(|g| g.season == season).cloned().collect();
    games.sort_by(chronological);
    Ok(Preseason { seed, ties, games })
}

/// Walks start-ordered `games` one UTC date at a time. Each item pairs a posterior fit to the earlier dates only
/// with the games of the next date, so no prediction sees a same-day outcome.
pub fn posteriors_by_utc_date<'a>(
    seed: &'a EloSeed,
    games: &'a [Game],
    cfg: &'a LeagueConfig,
) -> impl Iterator<Item = Result<(Posterior, &'a [Game])>> + 'a {
    let mut start = 0;
    games
        .chunk_by(|a, b| a.start_time_utc.date_naive() == b.start_time_utc.date_naive())
        .map(move |date_games| {
            let earlier = &games[..start];
            start += date_games.len();
            let date = date_games[0].start_time_utc.date_naive();
            let model =
                fit_posterior(seed, earlier, cfg).with_context(|| format!("Fit season {} before {date}", seed.target_season))?;
            Ok((model, date_games))
        })
}

/// One predictor's pregame forecast for a game.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Prediction {
    #[serde(flatten)]
    pub probabilities: Probabilities,
    /// The predictor's expected fractional home score: win 1, tie 1/2, loss 0.
    pub expected_home_score: f64,
}

impl Prediction {
    /// A forecast whose expected home score is `P(home win) + P(tie) / 2`.
    pub fn from_probabilities(probabilities: Probabilities) -> Self {
        Self {
            probabilities,
            expected_home_score: probabilities.home_win + probabilities.tie / 2.0,
        }
    }

    /// `Greater` when the home team is favored, `Less` when the away team is, `Equal` for a toss-up.
    pub fn favorite(&self) -> Ordering {
        self.probabilities.home_win.total_cmp(&self.probabilities.away_win)
    }
}

/// A completed game with both in-season predictors' pregame forecasts.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PredictedGame {
    #[serde(flatten)]
    pub game: Game,
    /// Laplace posterior fit to the season's earlier UTC dates.
    pub bayesian: Prediction,
    /// Elo ratings updated after every game of the season's earlier UTC dates.
    pub elo: Prediction,
}

/// Every game of one season, predicted before it was played.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SeasonPredictions {
    pub season: i32,
    /// Davidson tie weight estimated from the earlier seasons; both predictors apply it where ties are allowed.
    pub tie_weight: f64,
    /// The season's games in start order.
    pub games: Vec<PredictedGame>,
}

/// Simulates `season` as the app would have run it. Both predictors start from preseason Elo ratings and a tie weight
/// built from the earlier seasons, and each UTC date is predicted from the season's earlier dates only. Every game of
/// the season needs a result.
pub fn predict_season(history: &GameFile, cfg: &LeagueConfig, season: i32) -> Result<SeasonPredictions> {
    let Preseason { seed, games, .. } = preseason(history, cfg, season)?;
    let mut ratings: BTreeMap<_, _> = seed.ratings.iter().map(|r| (r.team.clone(), r.elo)).collect();
    let factor = std::f64::consts::LN_10 / cfg.elo.scale;
    let mut predicted = Vec::with_capacity(games.len());
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
            predicted.push(PredictedGame {
                game: g.clone(),
                bayesian: Prediction::from_probabilities(model.predict(&g.home_team, &g.away_team, g.neutral, &g.phase)?),
                elo: Prediction {
                    probabilities: outcome_probabilities(factor * (home - away + advantage), nu),
                    expected_home_score: expected_home(home, away, g.neutral, &cfg.elo),
                },
            });
        }
        // Elo learns from a date only after all of its games are predicted, matching the Bayesian information set.
        for g in date_games {
            apply_game(&mut ratings, g, &cfg.elo)?;
        }
    }
    Ok(SeasonPredictions {
        season,
        tie_weight: seed.tie_weight,
        games: predicted,
    })
}
