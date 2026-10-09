//! Bradley–Terry–Davidson likelihood and Laplace approximation, matching apps/web/src/model.ts.
use crate::{Audit, BayesianSettings, EloSeed, Game, LeagueConfig, Outcome};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub struct TieHistory {
    denominators: Vec<f64>,
    pub tied_games: usize,
}

impl TieHistory {
    pub fn from_audit(audit: &[Audit], games: &[Game], cfg: &LeagueConfig) -> Result<Self> {
        let games: BTreeMap<_, _> = games.iter().map(|g| (g.id.as_str(), g)).collect();
        let mut denominators = Vec::new();
        let mut tied_games = 0;
        for entry in audit {
            let g = games
                .get(entry.game_id.as_str())
                .context("Elo audit game missing from history")?;
            if cfg.ties_allowed_in.contains(&g.phase) {
                let advantage = if g.neutral { 0.0 } else { cfg.elo.home_advantage };
                let difference = std::f64::consts::LN_10 * (entry.home_before - entry.away_before + advantage) / cfg.elo.scale;
                denominators.push(2.0 * (difference / 2.0).cosh());
                tied_games += usize::from(g.result == Some(Outcome::Tie));
            }
        }
        Ok(Self {
            denominators,
            tied_games,
        })
    }

    pub fn estimate(&self, settings: &BayesianSettings) -> f64 {
        let desired = self.tied_games as f64 + settings.tie_prior_games * settings.tie_prior_rate;
        let (mut low, mut high) = (-25.0_f64, 25.0_f64);
        for _ in 0..100 {
            let mid = (low + high) / 2.0;
            let nu = mid.exp();
            let expected =
                self.denominators.iter().map(|d| nu / (d + nu)).sum::<f64>() + settings.tie_prior_games * nu / (2.0 + nu);
            if expected < desired {
                low = mid;
            } else {
                high = mid;
            }
        }
        ((low + high) / 2.0).exp()
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Probabilities {
    pub home_win: f64,
    pub away_win: f64,
    pub tie: f64,
}

impl Probabilities {
    pub fn for_outcome(self, outcome: &Outcome) -> f64 {
        match outcome {
            Outcome::HomeWin => self.home_win,
            Outcome::AwayWin => self.away_win,
            Outcome::Tie => self.tie,
        }
    }
}

pub fn outcome_probabilities(difference: f64, tie_weight: f64) -> Probabilities {
    let logits = [difference / 2.0, -difference / 2.0, tie_weight.ln()];
    let max = logits.into_iter().fold(f64::NEG_INFINITY, f64::max);
    let weights = logits.map(|v| (v - max).exp());
    let total: f64 = weights.iter().sum();
    Probabilities {
        home_win: weights[0] / total,
        away_win: weights[1] / total,
        tie: weights[2] / total,
    }
}

pub struct Posterior {
    pub ids: Vec<String>,
    pub means: Vec<f64>,
    pub covariance: Vec<Vec<f64>>,
    home_advantage: f64,
    tie_weight: f64,
    ties_allowed_in: Vec<String>,
}

impl Posterior {
    pub fn predict(&self, home: &str, away: &str, neutral: bool, phase: &str) -> Result<Probabilities> {
        let h = self.ids.iter().position(|id| id == home).context("Unknown home team")?;
        let a = self.ids.iter().position(|id| id == away).context("Unknown away team")?;
        ensure!(h != a, "Choose two different teams");
        let mean = self.means[h] - self.means[a] + if neutral { 0.0 } else { self.home_advantage };
        let sd = (self.covariance[h][h] + self.covariance[a][a] - 2.0 * self.covariance[h][a])
            .max(0.0)
            .sqrt();
        let nu = if self.ties_allowed_in.iter().any(|p| p == phase) {
            self.tie_weight
        } else {
            0.0
        };
        let mut total = [0.0; 3];
        let mut weight_sum = 0.0;
        for i in 0..=160 {
            let z = -8.0 + 16.0 * f64::from(i) / 160.0;
            let weight = (-z * z / 2.0).exp()
                * if i == 0 || i == 160 {
                    1.0
                } else if i % 2 == 0 {
                    2.0
                } else {
                    4.0
                };
            let p = outcome_probabilities(mean + sd * z, nu);
            for (sum, probability) in total.iter_mut().zip([p.home_win, p.away_win, p.tie]) {
                *sum += weight * probability;
            }
            weight_sum += weight;
        }
        Ok(Probabilities {
            home_win: total[0] / weight_sum,
            away_win: total[1] / weight_sum,
            tie: total[2] / weight_sum,
        })
    }
}

#[allow(clippy::needless_range_loop)]
fn cholesky(matrix: &[Vec<f64>]) -> Result<Vec<Vec<f64>>> {
    let n = matrix.len();
    let mut l = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in 0..=i {
            let mut v = matrix[i][j];
            for k in 0..j {
                v -= l[i][k] * l[j][k];
            }
            if i == j {
                ensure!(v > 0.0 && v.is_finite(), "Posterior precision is not positive definite");
                l[i][j] = v.sqrt();
            } else {
                l[i][j] = v / l[j][j];
            }
        }
    }
    Ok(l)
}

fn solve(l: &[Vec<f64>], rhs: &[f64]) -> Vec<f64> {
    let n = l.len();
    let mut y = vec![0.0; n];
    let mut x = vec![0.0; n];
    for i in 0..n {
        y[i] = (rhs[i] - (0..i).map(|j| l[i][j] * y[j]).sum::<f64>()) / l[i][i];
    }
    for i in (0..n).rev() {
        x[i] = (y[i] - (i + 1..n).map(|j| l[j][i] * x[j]).sum::<f64>()) / l[i][i];
    }
    x
}

/// Compensated (Neumaier) summation for the fit objective. Plain summation over thousands of games leaves rounding
/// noise above the line search's Armijo slack, so the optimizer could not tell a real decrease from noise.
#[derive(Default)]
struct CompensatedSum {
    sum: f64,
    compensation: f64,
}

impl CompensatedSum {
    fn add(&mut self, x: f64) {
        let t = self.sum + x;
        if self.sum.abs() >= x.abs() {
            self.compensation += (self.sum - t) + x;
        } else {
            self.compensation += (x - t) + self.sum;
        }
        self.sum = t;
    }

    fn total(&self) -> f64 {
        self.sum + self.compensation
    }
}

/// Each fit starts from the immutable preseason prior, never the preceding posterior.
pub fn fit_posterior(seed: &EloSeed, games: &[Game], cfg: &LeagueConfig) -> Result<Posterior> {
    cfg.validate()?;
    let ids: Vec<_> = cfg.teams.iter().map(|t| t.id.clone()).collect();
    let index: BTreeMap<_, _> = ids.iter().enumerate().map(|(i, id)| (id.as_str(), i)).collect();
    let ratings: BTreeMap<_, _> = seed.ratings.iter().map(|r| (r.team.as_str(), r.elo)).collect();
    ensure!(
        seed.league == cfg.id
            && seed.through_season == seed.target_season - 1
            && ratings.len() == ids.len()
            && seed.ratings.len() == ids.len()
            && ids.iter().all(|id| ratings.get(id.as_str()).is_some_and(|r| r.is_finite()))
            && seed.tie_weight.is_finite()
            && seed.tie_weight > 0.0,
        "Elo seed does not match the configured league and teams"
    );
    let factor = std::f64::consts::LN_10 / cfg.elo.scale;
    let prior: Vec<_> = ids
        .iter()
        .map(|id| (ratings[id.as_str()] - cfg.elo.initial) * factor)
        .collect();
    let precision = 1.0 / (cfg.bayesian.prior_sd_elo * factor).powi(2);
    let mut seen = BTreeSet::new();
    let mut observations = Vec::new();
    for g in games.iter().filter(|g| g.result.is_some()) {
        ensure!(
            g.season == seed.target_season
                && g.league == cfg.id
                && g.home_team != g.away_team
                && index.contains_key(g.home_team.as_str())
                && index.contains_key(g.away_team.as_str())
                && seen.insert(&g.id),
            "Invalid or duplicated current-season observation"
        );
        let nu = if cfg.ties_allowed_in.contains(&g.phase) {
            seed.tie_weight
        } else {
            0.0
        };
        ensure!(g.result != Some(Outcome::Tie) || nu > 0.0, "Tie not allowed in this phase");
        observations.push((
            index[g.home_team.as_str()],
            index[g.away_team.as_str()],
            if g.neutral { 0.0 } else { cfg.elo.home_advantage * factor },
            nu,
            g.result.as_ref().unwrap(),
        ));
    }
    let evaluate = |theta: &[f64]| {
        let mut gradient: Vec<_> = theta.iter().zip(&prior).map(|(v, p)| (v - p) * precision).collect();
        let mut hessian = vec![vec![0.0; ids.len()]; ids.len()];
        for (i, row) in hessian.iter_mut().enumerate() {
            row[i] = precision;
        }
        let mut objective = CompensatedSum::default();
        for (v, p) in theta.iter().zip(&prior) {
            objective.add((v - p).powi(2) * precision / 2.0);
        }
        for &(h, a, advantage, nu, outcome) in &observations {
            let p = outcome_probabilities(theta[h] - theta[a] + advantage, nu);
            let observed = match outcome {
                Outcome::HomeWin => 0.5,
                Outcome::AwayWin => -0.5,
                Outcome::Tie => 0.0,
            };
            let expected = (p.home_win - p.away_win) / 2.0;
            let second = (p.home_win + p.away_win) / 4.0 - expected.powi(2);
            objective.add(-p.for_outcome(outcome).max(f64::from_bits(1)).ln());
            gradient[h] += expected - observed;
            gradient[a] -= expected - observed;
            hessian[h][h] += second;
            hessian[a][a] += second;
            hessian[h][a] -= second;
            hessian[a][h] -= second;
        }
        (objective.total(), gradient, hessian)
    };
    let mut theta = prior.clone();
    let mut converged = false;
    for _ in 0..80 {
        let (objective, gradient, hessian) = evaluate(&theta);
        if gradient.iter().map(|v| v.abs()).fold(0.0, f64::max) < 1e-9 {
            converged = true;
            break;
        }
        let step = solve(&cholesky(&hessian)?, &gradient);
        let descent = gradient.iter().zip(&step).map(|(g, s)| g * s).sum::<f64>();
        let mut rate = 1.0;
        let mut accepted = false;
        for _ in 0..30 {
            let candidate: Vec<_> = theta.iter().zip(&step).map(|(v, s)| v - rate * s).collect();
            if evaluate(&candidate).0 <= objective - 1e-4 * rate * descent + 1e-12 {
                theta = candidate;
                accepted = true;
                break;
            }
            rate /= 2.0;
        }
        ensure!(accepted, "Bayesian optimizer line search failed");
    }
    ensure!(converged, "Bayesian optimizer did not converge");
    let l = cholesky(&evaluate(&theta).2)?;
    let mut covariance = vec![vec![0.0; ids.len()]; ids.len()];
    for j in 0..ids.len() {
        let mut rhs = vec![0.0; ids.len()];
        rhs[j] = 1.0;
        for (row, v) in covariance.iter_mut().zip(solve(&l, &rhs)) {
            row[j] = v;
        }
    }
    Ok(Posterior {
        ids,
        means: theta,
        covariance,
        home_advantage: cfg.elo.home_advantage * factor,
        tie_weight: seed.tie_weight,
        ties_allowed_in: cfg.ties_allowed_in.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tie_smoothing_matches_pseudocounts_for_equal_strength_games() {
        let mut history = TieHistory {
            denominators: vec![2.0; 100],
            tied_games: 1,
        };
        let mut settings = BayesianSettings {
            prior_sd_elo: 150.0,
            tie_prior_games: 100.0,
            tie_prior_rate: 0.005,
        };
        let q = (1.0 + 100.0 * 0.005) / 200.0;
        assert!((history.estimate(&settings) - 2.0 * q / (1.0 - q)).abs() < 1e-12);
        history.tied_games = 0;
        let small = history.estimate(&settings);
        assert!(small > 0.0);
        settings.tie_prior_rate = 0.01;
        assert!(history.estimate(&settings) > small);
        history.denominators.clear();
        assert!((history.estimate(&settings) - 2.0 * 0.01 / 0.99).abs() < 1e-12);
    }
}
