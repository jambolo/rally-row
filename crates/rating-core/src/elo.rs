use crate::{Audit, EloSettings, Game, LeagueConfig, Rating, validate_games};
use anyhow::{Context, Result, ensure};
use std::collections::BTreeMap;

pub struct EloReplay {
    pub ratings: Vec<Rating>,
    pub audit: Vec<Audit>,
}

pub fn expected_home(home: f64, away: f64, neutral: bool, cfg: &EloSettings) -> f64 {
    1.0 / (1.0 + 10_f64.powf((away - home - if neutral { 0.0 } else { cfg.home_advantage }) / cfg.scale))
}

pub fn regress_rating(rating: f64, cfg: &EloSettings) -> f64 {
    cfg.initial + (rating - cfg.initial) * (1.0 - cfg.offseason_regression)
}

/// One game's rating update: the pregame ratings, the expected and observed home scores, and the home team's change.
#[derive(Debug, Clone, Copy)]
pub struct Update {
    pub home_before: f64,
    pub away_before: f64,
    pub expected_home_score: f64,
    pub observed_home_score: f64,
    pub delta: f64,
}

impl Update {
    pub fn audit(&self, game: &Game) -> Audit {
        Audit {
            game_id: game.id.clone(),
            season: game.season,
            home_before: self.home_before,
            away_before: self.away_before,
            expected_home_score: self.expected_home_score,
            observed_home_score: self.observed_home_score,
            home_after: self.home_before + self.delta,
            away_after: self.away_before - self.delta,
        }
    }
}

/// Scores a completed game with the pregame ratings, then moves both ratings by K times the observed surprise.
pub fn apply_game(ratings: &mut BTreeMap<String, f64>, game: &Game, cfg: &EloSettings) -> Result<Update> {
    let outcome = game.result.as_ref().context("Cannot score an unreported game")?;
    let home = ratings[&game.home_team];
    let away = ratings[&game.away_team];
    let expected = expected_home(home, away, game.neutral, cfg);
    let observed = outcome.home_score();
    let delta = cfg.k * (observed - expected);
    *ratings.get_mut(&game.home_team).unwrap() = home + delta;
    *ratings.get_mut(&game.away_team).unwrap() = away - delta;
    Ok(Update {
        home_before: home,
        away_before: away,
        expected_home_score: expected,
        observed_home_score: observed,
        delta,
    })
}

/// Ratings and game counts, replayed game by game from the configured historical start.
pub(crate) struct Elo<'a> {
    settings: &'a EloSettings,
    season: i32,
    ratings: BTreeMap<String, f64>,
    counts: BTreeMap<String, usize>,
}

impl<'a> Elo<'a> {
    pub(crate) fn new(cfg: &'a LeagueConfig) -> Self {
        let ratings: BTreeMap<String, f64> = cfg.teams.iter().map(|t| (t.id.clone(), cfg.elo.initial)).collect();
        let counts = ratings.keys().map(|k| (k.clone(), 0)).collect();
        Self {
            settings: &cfg.elo,
            season: cfg.history_start,
            ratings,
            counts,
        }
    }

    /// Regresses once for each season boundary since the previous game, then applies the completed `game`.
    pub(crate) fn apply(&mut self, game: &Game) -> Result<Update> {
        while self.season < game.season {
            for r in self.ratings.values_mut() {
                *r = regress_rating(*r, self.settings);
            }
            self.season += 1;
        }
        let update = apply_game(&mut self.ratings, game, self.settings)?;
        *self.counts.get_mut(&game.home_team).unwrap() += 1;
        *self.counts.get_mut(&game.away_team).unwrap() += 1;
        Ok(update)
    }

    /// The ratings after the latest game, without the offseason regression that follows it.
    pub(crate) fn ratings(&self) -> Vec<Rating> {
        self.ratings
            .iter()
            .map(|(team, &elo)| Rating {
                team: team.clone(),
                elo,
                games: self.counts[team],
            })
            .collect()
    }
}

/// Replay through the end of a season; callers explicitly regress again for next-season priors.
pub fn replay_elo(games: &[Game], cfg: &LeagueConfig, through_season: i32) -> Result<EloReplay> {
    let games = replay_games(games, cfg, through_season)?;
    let mut elo = Elo::new(cfg);
    let audit = games
        .iter()
        .filter(|g| g.result.is_some())
        .map(|g| Ok(elo.apply(g)?.audit(g)))
        .collect::<Result<_>>()?;
    Ok(EloReplay {
        ratings: elo.ratings(),
        audit,
    })
}

/// Validates `games` through `through_season` and returns them in replay order. Every season from the historical start
/// through `through_season` needs a completed game.
pub(crate) fn replay_games(games: &[Game], cfg: &LeagueConfig, through_season: i32) -> Result<Vec<Game>> {
    cfg.validate()?;
    ensure!(
        (cfg.history_start..=2200).contains(&through_season),
        "Invalid Elo replay end season"
    );
    let mut games: Vec<_> = games.iter().filter(|g| g.season <= through_season).cloned().collect();
    validate_games(&mut games, cfg)?;
    ensure!(
        games.iter().all(|g| g.season >= cfg.history_start),
        "Game precedes the configured historical start"
    );
    for season in cfg.history_start..=through_season {
        ensure!(
            games.iter().any(|g| g.season == season && g.result.is_some()),
            "Missing completed historical season {season}"
        );
    }
    Ok(games)
}
