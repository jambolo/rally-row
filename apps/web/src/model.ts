import { teamIdentity, type EloSeed, type Game, type LeagueConfig } from './contracts.ts';

export type Probabilities = { home_win: number; away_win: number; tie: number };
export type Prediction = Probabilities & {
  home_probability_interval: [number, number];
};
export type Posterior = {
  ids: string[];
  means: number[];
  covariance: number[][];
  seed: EloSeed;
  config: LeagueConfig;
  games_used: number;
  iterations: number;
};

/** Davidson extension of Bradley–Terry. Softmax keeps extreme matchups stable. */
export function outcomeProbabilities(difference: number, tieWeight: number): Probabilities {
  const logits = [difference / 2, -difference / 2, tieWeight > 0 ? Math.log(tieWeight) : -Infinity];
  const max = Math.max(...logits),
    weights = logits.map((v) => Math.exp(v - max));
  const total = weights.reduce((a, b) => a + b, 0);
  return {
    home_win: weights[0] / total,
    away_win: weights[1] / total,
    tie: weights[2] / total,
  };
}
/**
 * Fair (no-vig) two-way American moneylines: the favorite is negative, the underdog positive.
 * A tie is a push, so each side is priced as if the game is decisive. Conditioning on no tie
 * leaves the home:away odds ratio unchanged, so both lines come from that one ratio and are
 * exact mirror images. Even money is +100 on both sides.
 */
export function twoWayMoneylines(p: Probabilities): { home: number | null; away: number | null } {
  if (!(p.home_win > 0 && p.away_win > 0)) return { home: null, away: null };
  const magnitude = Math.round((100 * Math.max(p.home_win, p.away_win)) / Math.min(p.home_win, p.away_win));
  if (!Number.isFinite(magnitude)) return { home: null, away: null };
  if (magnitude === 100) return { home: 100, away: 100 };
  return p.home_win > p.away_win ? { home: -magnitude, away: magnitude } : { home: magnitude, away: -magnitude };
}
export function formatTwoWayMoneyline(line: number | null): string {
  return line === null ? '—' : line > 0 ? `+${line}` : `${line}`;
}
/** Parses an American moneyline such as `-110` or `+150`. Lines strictly between −100 and +100 do not exist. */
export function parseMoneyline(text: string): number | null {
  const normalized = text.trim().replace(/^−/, '-');
  if (!/^[+-]?\d+$/.test(normalized)) return null;
  const line = Number(normalized);
  return Number.isFinite(line) && Math.abs(line) >= 100 ? line : null;
}
/**
 * Expected profit per unit staked on one side of a two-way bet at the book's line, taking the fair line as the
 * true price of a decisive game. The book's line includes its vig, which is not removed: the bet pays at the quoted
 * price. A tie is a push that refunds the stake, so it scales the expectation by 1 − P(tie).
 */
export function twoWayExpectedValue(fairLine: number, bookLine: number, tie: number): number {
  const p = fairLine < 0 ? -fairLine / (100 - fairLine) : 100 / (100 + fairLine);
  const profit = bookLine < 0 ? -100 / bookLine : bookLine / 100;
  return (1 - tie) * (p * profit - (1 - p));
}
function zeros(n: number) {
  return Array.from({ length: n }, () => Array<number>(n).fill(0));
}

/** Cholesky factorization for the strictly positive-definite posterior precision. */
function cholesky(matrix: number[][]): number[][] {
  const n = matrix.length,
    l = zeros(n);
  for (let i = 0; i < n; i++)
    for (let j = 0; j <= i; j++) {
      let v = matrix[i][j];
      for (let k = 0; k < j; k++) v -= l[i][k] * l[j][k];
      if (i === j) {
        if (!(v > 0) || !Number.isFinite(v)) throw new Error('Posterior precision is not positive definite');
        l[i][j] = Math.sqrt(v);
      } else l[i][j] = v / l[j][j];
    }
  return l;
}
function solve(l: number[][], rhs: number[]): number[] {
  const n = l.length,
    y = Array<number>(n).fill(0),
    x = Array<number>(n).fill(0);
  for (let i = 0; i < n; i++) {
    let v = rhs[i];
    for (let j = 0; j < i; j++) v -= l[i][j] * y[j];
    y[i] = v / l[i][i];
  }
  for (let i = n - 1; i >= 0; i--) {
    let v = y[i];
    for (let j = i + 1; j < n; j++) v -= l[j][i] * x[j];
    x[i] = v / l[i][i];
  }
  return x;
}

/** Compensated (Neumaier) summation for the fit objective. Plain summation over thousands of games leaves rounding
 * noise above the line search's Armijo slack, so the optimizer could not tell a real decrease from noise.
 */
function compensatedSum() {
  let sum = 0,
    compensation = 0;
  return {
    add(x: number) {
      const t = sum + x;
      compensation += Math.abs(sum) >= Math.abs(x) ? sum - t + x : x - t + sum;
      sum = t;
    },
    total: () => sum + compensation,
  };
}

/** Refit from immutable preseason priors. Each result is used exactly once.
 * The posterior is approximated by a multivariate Gaussian at its MAP (Laplace).
 * Full covariance retains uncertainty shared between opponents.
 */
export function fitPosterior(seed: EloSeed, games: Game[], config: LeagueConfig): Posterior {
  const ids = config.teams.map((t) => t.id),
    index = new Map(ids.map((id, i) => [id, i]));
  const byTeam = new Map(seed.ratings.map((r) => [r.team, r.elo]));
  if (
    seed.league !== config.id ||
    seed.through_season !== seed.target_season - 1 ||
    byTeam.size !== ids.length ||
    seed.ratings.length !== ids.length ||
    ids.some((id) => !byTeam.has(id))
  )
    throw new Error('Elo seed does not match the configured league and teams');
  const factor = Math.LN10 / config.elo.scale;
  const prior = ids.map((id) => (byTeam.get(id)! - config.elo.initial) * factor);
  const variance = (config.bayesian.prior_sd_elo * factor) ** 2,
    precision = 1 / variance;
  const observations = games.filter((g) => g.result !== null);
  const seen = new Set<string>();
  for (const g of observations) {
    if (
      g.season !== seed.target_season ||
      g.league !== config.id ||
      !index.has(g.home_team) ||
      !index.has(g.away_team) ||
      g.home_team === g.away_team ||
      seen.has(g.id)
    )
      throw new Error('Invalid or duplicated current-season observation');
    if (g.result === 'tie' && !config.ties_allowed_in.includes(g.phase)) throw new Error('Tie not allowed in this phase');
    seen.add(g.id);
  }
  const evaluate = (theta: number[]) => {
    const gradient = theta.map((v, i) => (v - prior[i]) * precision),
      hessian = zeros(ids.length);
    const objective = compensatedSum();
    theta.forEach((v, i) => objective.add(((v - prior[i]) ** 2 * precision) / 2));
    for (let i = 0; i < ids.length; i++) hessian[i][i] = precision;
    for (const g of observations) {
      const h = index.get(g.home_team)!,
        a = index.get(g.away_team)!;
      const difference = theta[h] - theta[a] + (g.neutral ? 0 : config.elo.home_advantage * factor);
      const nu = config.ties_allowed_in.includes(g.phase) ? seed.tie_weight : 0;
      const p = outcomeProbabilities(difference, nu);
      const observed = g.result === 'home_win' ? 0.5 : g.result === 'away_win' ? -0.5 : 0;
      const expected = (p.home_win - p.away_win) / 2;
      const first = expected - observed;
      const second = (p.home_win + p.away_win) / 4 - expected ** 2;
      objective.add(-Math.log(Math.max(p[g.result!], Number.MIN_VALUE)));
      gradient[h] += first;
      gradient[a] -= first;
      hessian[h][h] += second;
      hessian[a][a] += second;
      hessian[h][a] -= second;
      hessian[a][h] -= second;
    }
    return { objective: objective.total(), gradient, hessian };
  };
  let theta = [...prior],
    iterations = 0,
    converged = false;
  for (; iterations < 80; iterations++) {
    const e = evaluate(theta);
    if (Math.max(...e.gradient.map(Math.abs)) < 1e-9) {
      converged = true;
      break;
    }
    const step = solve(cholesky(e.hessian), e.gradient);
    const descent = e.gradient.reduce((s, v, i) => s + v * step[i], 0);
    let rate = 1,
      accepted = false;
    for (let line = 0; line < 30; line++) {
      const candidate = theta.map((v, i) => v - rate * step[i]);
      if (evaluate(candidate).objective <= e.objective - 1e-4 * rate * descent + 1e-12) {
        theta = candidate;
        accepted = true;
        break;
      }
      rate /= 2;
    }
    if (!accepted) throw new Error('Bayesian optimizer line search failed');
  }
  if (!converged) throw new Error('Bayesian optimizer did not converge');
  const l = cholesky(evaluate(theta).hessian),
    covariance = zeros(ids.length);
  for (let j = 0; j < ids.length; j++) {
    const rhs = Array<number>(ids.length).fill(0);
    rhs[j] = 1;
    const column = solve(l, rhs);
    for (let i = 0; i < ids.length; i++) covariance[i][j] = column[i];
  }
  return {
    ids,
    means: theta,
    covariance,
    seed,
    config,
    games_used: observations.length,
    iterations,
  };
}

export function predict(model: Posterior, home: string, away: string, neutral: boolean, phase: Game['phase']): Prediction {
  const h = model.ids.indexOf(home),
    a = model.ids.indexOf(away);
  if (h < 0 || a < 0 || h === a) throw new Error('Choose two different known teams');
  const factor = Math.LN10 / model.config.elo.scale;
  const mean = model.means[h] - model.means[a] + (neutral ? 0 : model.config.elo.home_advantage * factor);
  const variance = Math.max(0, model.covariance[h][h] + model.covariance[a][a] - 2 * model.covariance[h][a]);
  const sd = Math.sqrt(variance),
    nu = model.config.ties_allowed_in.includes(phase) ? model.seed.tie_weight : 0;
  // Deterministic Simpson quadrature of the posterior predictive probabilities.
  // Integrate over ±8 normal standard deviations, then normalize the tiny tail loss.
  const result = { home_win: 0, away_win: 0, tie: 0 };
  let weightSum = 0;
  const intervals = 160;
  for (let i = 0; i <= intervals; i++) {
    const z = -8 + (16 * i) / intervals;
    const weight = Math.exp((-z * z) / 2) * (i === 0 || i === intervals ? 1 : i % 2 === 0 ? 2 : 4);
    const p = outcomeProbabilities(mean + sd * z, nu);
    result.home_win += weight * p.home_win;
    result.away_win += weight * p.away_win;
    result.tie += weight * p.tie;
    weightSum += weight;
  }
  return {
    home_win: result.home_win / weightSum,
    away_win: result.away_win / weightSum,
    tie: result.tie / weightSum,
    home_probability_interval: [
      outcomeProbabilities(mean - 1.96 * sd, nu).home_win,
      outcomeProbabilities(mean + 1.96 * sd, nu).home_win,
    ],
  };
}

export function teamEstimates(model: Posterior) {
  const factor = Math.LN10 / model.config.elo.scale;
  return model.ids
    .map((id, i) => {
      const era = teamIdentity(model.config, id, model.seed.target_season);
      return {
        id,
        name: era.name,
        location: era.location,
        abbreviation: era.abbreviation ?? era.source_ids[0],
        initial_elo: model.seed.ratings.find((r) => r.team === id)!.elo,
        rating: model.config.elo.initial + model.means[i] / factor,
        sd: Math.sqrt(model.covariance[i][i]) / factor,
      };
    })
    .sort((a, b) => b.rating - a.rating);
}
