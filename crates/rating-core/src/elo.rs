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

/// Scores a completed game with the pregame ratings, then moves both ratings by K times the observed surprise.
pub fn apply_game(ratings: &mut BTreeMap<String, f64>, game: &Game, cfg: &EloSettings) -> Result<Audit> {
    let outcome = game.result.as_ref().context("Cannot score an unreported game")?;
    let home = ratings[&game.home_team];
    let away = ratings[&game.away_team];
    let expected = expected_home(home, away, game.neutral, cfg);
    let observed = outcome.home_score();
    let delta = cfg.k * (observed - expected);
    ratings.insert(game.home_team.clone(), home + delta);
    ratings.insert(game.away_team.clone(), away - delta);
    Ok(Audit {
        game_id: game.id.clone(),
        season: game.season,
        home_before: home,
        away_before: away,
        expected_home_score: expected,
        observed_home_score: observed,
        home_after: home + delta,
        away_after: away - delta,
    })
}

/// Replay through the end of a season; callers explicitly regress again for next-season priors.
pub fn replay_elo(games: &[Game], cfg: &LeagueConfig, through_season: i32) -> Result<EloReplay> {
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
    let mut ratings: BTreeMap<String, f64> = cfg.teams.iter().map(|t| (t.id.clone(), cfg.elo.initial)).collect();
    let mut counts: BTreeMap<String, usize> = ratings.keys().map(|k| (k.clone(), 0)).collect();
    let mut audit = Vec::new();
    let mut previous = cfg.history_start;
    for g in games.iter().filter(|g| g.result.is_some()) {
        while previous < g.season {
            for r in ratings.values_mut() {
                *r = regress_rating(*r, &cfg.elo);
            }
            previous += 1;
        }
        audit.push(apply_game(&mut ratings, g, &cfg.elo)?);
        *counts.get_mut(&g.home_team).unwrap() += 1;
        *counts.get_mut(&g.away_team).unwrap() += 1;
    }
    Ok(EloReplay {
        ratings: ratings
            .into_iter()
            .map(|(team, elo)| Rating {
                games: counts[&team],
                team,
                elo,
            })
            .collect(),
        audit,
    })
}
