//! Predicting a completed season one UTC date at a time from earlier information only, as Bayesian tuning and
//! model evaluation do.

use crate::{
    EloSeed, Game, GameFile, LeagueConfig,
    bayesian::{Posterior, TieHistory, fit_posterior},
    chronological, seed_with_ties,
};
use anyhow::{Context, Result};

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
