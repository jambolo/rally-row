import { describe, it, expect } from 'vitest';
import {
  fitPosterior,
  formatTwoWayMoneyline,
  homeAdvantageLogit,
  outcomeInto,
  outcomeProbabilities,
  parseMoneyline,
  posteriorSampler,
  predict,
  teamEstimates,
  tieWeightIn,
  twoWayExpectedValue,
  twoWayMoneylines,
  type Posterior,
  type Probabilities,
} from '../src/model.ts';
import { SIMULATION_SEED, normalSource, sfc32 } from '../src/random.ts';
import { config, seed, game } from './helpers.ts';

describe('Bayesian model', () => {
  it('uses the Elo seed as the prior and preserves uncertainty without results', () => {
    const s = seed();
    s.ratings.find((t) => t.team === 'SEA')!.elo = config.elo.initial + 200;
    const model = fitPosterior(s, [], config),
      row = teamEstimates(model).find((t) => t.id === 'SEA')!;
    expect(row.rating).toBeCloseTo(config.elo.initial + 200, 10);
    expect(row.sd).toBeCloseTo(config.bayesian.prior_sd_elo, 10);
    const p = predict(model, 'SEA', 'SF', true, 'regular');
    expect(p.home_win).toBeGreaterThan(p.away_win);
    expect(p.home_win + p.away_win + p.tie).toBeCloseTo(1, 12);
    expect(p.home_probability_interval[0]).toBeLessThan(p.home_win);
    expect(p.home_probability_interval[1]).toBeGreaterThan(p.home_win);
  });
  it('updates both teams for a win and retains posterior correlation', () => {
    const model = fitPosterior(seed(), [game()], config),
      h = model.ids.indexOf('SEA'),
      a = model.ids.indexOf('SF');
    expect(model.means[h]).toBeGreaterThan(0);
    expect(model.means[a]).toBeLessThan(0);
    expect(model.covariance[h][a]).toBeGreaterThan(0);
    expect(teamEstimates(model).find((t) => t.id === 'SEA')!.sd).toBeLessThan(config.bayesian.prior_sd_elo);
  });
  it('treats ties as a proper third outcome and moves an overmatched favorite down', () => {
    const s = seed();
    s.ratings.find((t) => t.team === 'SEA')!.elo = config.elo.initial + 200;
    const model = fitPosterior(s, [game({ result: 'tie' })], config);
    expect(teamEstimates(model).find((t) => t.id === 'SEA')!.rating).toBeLessThan(config.elo.initial + 200);
    expect(predict(model, 'SEA', 'SF', true, 'regular').tie).toBeGreaterThan(0);
    expect(predict(model, 'SEA', 'SF', true, 'postseason').tie).toBe(0);
    expect(() => fitPosterior(s, [game({ result: 'tie', phase: 'postseason' })], config)).toThrow(/Tie/);
  });
  it('matches the analytic two-team Hessian for an equal-strength tie', () => {
    const model = fitPosterior(seed(), [game({ result: 'tie' })], config);
    const v = ((config.bayesian.prior_sd_elo * Math.LN10) / config.elo.scale) ** 2,
      p = 1 / v,
      h = 1 / (2 * (2 + 0.02));
    const determinant = p * p + 2 * p * h;
    const i = model.ids.indexOf('SEA'),
      j = model.ids.indexOf('SF');
    expect(model.means[i]).toBeCloseTo(0, 12);
    expect(model.covariance[i][i]).toBeCloseTo((p + h) / determinant, 12);
    expect(model.covariance[i][j]).toBeCloseTo(h / determinant, 12);
  });
  it('is symmetric under team reversal at neutral venues and handles home advantage', () => {
    const m = fitPosterior(seed(), [game()], config);
    const a = predict(m, 'SEA', 'SF', true, 'regular'),
      b = predict(m, 'SF', 'SEA', true, 'regular');
    expect(a.home_win).toBeCloseTo(b.away_win, 12);
    expect(a.tie).toBeCloseTo(b.tie, 12);
    expect(predict(m, 'SEA', 'SF', false, 'regular').home_win).toBeGreaterThan(a.home_win);
    expect(() => predict(m, 'SEA', 'SEA', true, 'regular')).toThrow();
  });
  it('is invariant to observation order and does not compound results on refits', () => {
    const games = [game(), game({ id: 'g2', result: 'away_win' }), game({ id: 'g3', away_team: 'KC', result: 'tie' })];
    const a = fitPosterior(seed(), games, config),
      b = fitPosterior(seed(), [...games].reverse(), config),
      c = fitPosterior(seed(), games, config);
    a.means.forEach((x, i) => {
      expect(x).toBeCloseTo(b.means[i], 10);
      expect(x).toBeCloseTo(c.means[i], 12);
    });
    expect(() => fitPosterior(seed(), [game(), game()], config)).toThrow(/duplicated/);
    expect(() => fitPosterior(seed(), [game({ season: 2025 })], config)).toThrow();
  });
  it('keeps extreme outcome probabilities finite and normalized', () => {
    for (const d of [-10000, -100, 0, 100, 10000]) {
      const p = outcomeProbabilities(d, 0.02);
      expect(p.home_win + p.away_win + p.tie).toBeCloseTo(1, 12);
      expect(Object.values(p).every((x) => Number.isFinite(x) && x >= 0 && x <= 1)).toBe(true);
    }
  });
});

describe('two-way moneyline', () => {
  const lines = (home_win: number, away_win: number) => twoWayMoneylines({ home_win, away_win, tie: 1 - home_win - away_win });
  it('prices favorites negative and underdogs positive', () => {
    expect(lines(0.6, 0.4)).toEqual({ home: -150, away: 150 });
    expect(lines(0.25, 0.75)).toEqual({ home: 300, away: -300 });
    expect(lines(0.9, 0.1)).toEqual({ home: -900, away: 900 });
  });
  it('shows even money as +100 on both sides', () => {
    expect(lines(0.5, 0.5)).toEqual({ home: 100, away: 100 });
    expect(lines(0.501, 0.499)).toEqual({ home: 100, away: 100 });
    expect(lines(0.4985, 0.4985)).toEqual({ home: 100, away: 100 });
  });
  it('rounds both sides to exact mirror images at half-point boundaries', () => {
    // 68/32 = 2.125, so the unrounded line is exactly 212.5.
    expect(lines(0.32, 0.68)).toEqual({ home: 213, away: -213 });
    expect(lines(0.68, 0.32)).toEqual({ home: -213, away: 213 });
  });
  it('treats a tie as a push by conditioning on a decisive game', () => {
    // 75.9% / 23.8% / 0.3% tie -> 76.13% / 23.87% once ties are removed.
    expect(lines(0.759, 0.238)).toEqual({ home: -319, away: 319 });
  });
  it('is symmetric whatever the tie probability', () => {
    for (const tie of [0, 0.003, 0.2]) {
      const decisive = 1 - tie;
      expect(lines(0.7 * decisive, 0.3 * decisive)).toEqual({ home: -233, away: 233 });
    }
  });
  it('returns null for degenerate probabilities', () => {
    expect(twoWayMoneylines({ home_win: 0, away_win: 0, tie: 1 })).toEqual({ home: null, away: null });
    expect(lines(1, 0)).toEqual({ home: null, away: null });
    expect(lines(0, 1)).toEqual({ home: null, away: null });
    expect(lines(1, Number.MIN_VALUE)).toEqual({ home: null, away: null });
  });
  it('formats with a plus sign for positive lines and a dash when undefined', () => {
    expect(formatTwoWayMoneyline(150)).toBe('+150');
    expect(formatTwoWayMoneyline(-150)).toBe('-150');
    expect(formatTwoWayMoneyline(null)).toBe('—');
  });
});

describe('book moneyline expected value', () => {
  it('parses American moneylines', () => {
    expect(parseMoneyline('-110')).toBe(-110);
    expect(parseMoneyline('+150')).toBe(150);
    expect(parseMoneyline(' 150 ')).toBe(150);
    expect(parseMoneyline('−120')).toBe(-120);
    expect(parseMoneyline('100')).toBe(100);
    expect(parseMoneyline('-100')).toBe(-100);
  });
  it('rejects text that is not a moneyline', () => {
    for (const text of ['', '99', '-99', '+0', 'abc', '-110.5', '+-110', '1e3', '9'.repeat(400)])
      expect(parseMoneyline(text)).toBeNull();
  });
  it('is zero when the book matches the fair line', () => {
    for (const line of [-319, -150, 100, 150, 319]) expect(twoWayExpectedValue(line, line, 0.003)).toBeCloseTo(0, 12);
    expect(twoWayExpectedValue(-100, 100, 0)).toBeCloseTo(0, 12);
  });
  it('prices favorites and underdogs at the book line', () => {
    // Fair -150 is a 60% decisive-game winner; +120 pays 1.2 units.
    expect(twoWayExpectedValue(-150, 120, 0)).toBeCloseTo(0.6 * 1.2 - 0.4, 12);
    // Fair +150 is a 40% winner; -110 pays 100/110 units.
    expect(twoWayExpectedValue(150, -110, 0)).toBeCloseTo(0.4 * (100 / 110) - 0.6, 12);
    // A coin flip at -110 loses the standard 4.55% vig.
    expect(twoWayExpectedValue(100, -110, 0)).toBeCloseTo(-1 / 22, 12);
  });
  it('charges the vig on both sides of a market priced at the fair odds', () => {
    // Fair -150 / +150 quoted by the book as -165 / +135.
    expect(twoWayExpectedValue(-150, -165, 0)).toBeCloseTo(0.6 * (100 / 165) - 0.4, 12);
    expect(twoWayExpectedValue(150, 135, 0)).toBeCloseTo(0.4 * 1.35 - 0.6, 12);
    expect(twoWayExpectedValue(-150, -165, 0)).toBeLessThan(0);
    expect(twoWayExpectedValue(150, 135, 0)).toBeLessThan(0);
  });
  it('scales by the chance the game is decisive because a tie is a push', () => {
    expect(twoWayExpectedValue(-150, 120, 0.25)).toBeCloseTo(0.75 * 0.32, 12);
  });
});

// Verbatim pre-change implementation, kept as the bit-identity reference.
function arraySoftmax(difference: number, tieWeight: number): Probabilities {
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

const differences = [
  -1e3,
  -37.5,
  -5,
  -1,
  -0.37,
  -1e-9,
  -0,
  0,
  1e-9,
  0.37,
  1,
  5,
  37.5,
  1e3,
  ...Array.from({ length: 81 }, (_, i) => (i - 40) * 0.173),
];
const tieWeights = [0, 1e-6, 0.003, 0.02, 0.25, 1, 3];
const fields = ['home_win', 'away_win', 'tie'] as const;

function mismatchesBetween(a: (d: number, nu: number) => Probabilities, b: (d: number, nu: number) => Probabilities) {
  const mismatches: string[] = [];
  for (const d of differences)
    for (const nu of tieWeights) {
      const x = a(d, nu),
        y = b(d, nu);
      for (const f of fields) if (!Object.is(x[f], y[f])) mismatches.push(`${d} ${nu} ${f}`);
    }
  return mismatches;
}

describe('simulation outcome math', () => {
  it('outcomeInto is bit-identical to the array-based softmax', () => {
    const mismatches = mismatchesBetween((d, nu) => outcomeInto(d, nu, { home_win: NaN, away_win: NaN, tie: NaN }), arraySoftmax);
    expect(mismatches).toEqual([]);
  });
  it('outcomeProbabilities returns the outcomeInto values', () => {
    const mismatches = mismatchesBetween(outcomeProbabilities, (d, nu) =>
      outcomeInto(d, nu, { home_win: NaN, away_win: NaN, tie: NaN }),
    );
    expect(mismatches).toEqual([]);
  });
  it('outcomeInto fills and returns the given object', () => {
    const out = { home_win: 0, away_win: 0, tie: 0 };
    expect(outcomeInto(1e3, 0, out)).toBe(out);
    expect(out).toEqual({ home_win: 1, away_win: 0, tie: 0 });
    outcomeInto(-1e3, 0, out);
    expect(out).toEqual({ home_win: 0, away_win: 1, tie: 0 });
    outcomeInto(0, 0.02, out);
    expect(out.home_win).toBe(out.away_win);
    expect(out.home_win + out.away_win + out.tie).toBeCloseTo(1, 12);
  });
  it('homeAdvantageLogit converts Elo home advantage to logit units', () => {
    expect(homeAdvantageLogit(config)).toBe(45 * (Math.LN10 / 400));
  });
  it('tieWeightIn allows ties only in configured phases', () => {
    const model = fitPosterior(seed(), [], config);
    expect(tieWeightIn(model, 'regular')).toBe(0.02);
    expect(tieWeightIn(model, 'postseason')).toBe(0);
  });
});

const posterior = (means: number[], covariance: number[][]): Posterior => ({
  ids: means.map((_, i) => 'T' + i),
  means,
  covariance,
  seed: seed(),
  config,
  games_used: 0,
  iterations: 0,
});

describe('posterior sampler', () => {
  it('posteriorSampler applies the Cholesky factor to normals in index order', () => {
    const sample = posteriorSampler(
      posterior(
        [10, 20],
        [
          [4, 2],
          [2, 5],
        ],
      ),
    );
    let calls = 0;
    const out = new Float64Array(2);
    expect(sample(() => ++calls, out)).toBe(out);
    expect(Array.from(out)).toEqual([12, 25]);
    expect(calls).toBe(2);
    sample(() => 0, out);
    expect(Array.from(out)).toEqual([10, 20]);
  });
  it('posteriorSampler matches the posterior mean and covariance', () => {
    const means = [0.5, -0.2, 0.1],
      covariance = [
        [0.04, 0.012, -0.008],
        [0.012, 0.09, 0.006],
        [-0.008, 0.006, 0.025],
      ];
    const sample = posteriorSampler(posterior(means, covariance)),
      normal = normalSource(sfc32(SIMULATION_SEED)),
      out = new Float64Array(3),
      N = 100_000,
      sum = [0, 0, 0],
      draws: number[][] = [];
    for (let s = 0; s < N; s++) {
      sample(normal, out);
      draws.push(Array.from(out));
      for (let i = 0; i < 3; i++) sum[i] += out[i];
    }
    const mean = sum.map((v) => v / N);
    for (let i = 0; i < 3; i++) {
      expect(Math.abs(mean[i] - means[i])).toBeLessThan(0.005);
      for (let j = 0; j < 3; j++) {
        let c = 0;
        for (const x of draws) c += (x[i] - mean[i]) * (x[j] - mean[j]);
        expect(Math.abs(c / N - covariance[i][j])).toBeLessThan(0.002);
      }
    }
  });
  it('posteriorSampler rejects a covariance that is not positive definite', () => {
    expect(() =>
      posteriorSampler(
        posterior(
          [0, 0],
          [
            [1, 2],
            [2, 1],
          ],
        ),
      ),
    ).toThrow('Posterior precision is not positive definite');
  });
});
