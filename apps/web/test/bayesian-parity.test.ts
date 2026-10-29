import { readFileSync } from 'node:fs';
import { expect, it } from 'vitest';
import { fitPosterior, predict } from '../src/model.ts';
import { SEED_SCHEMA_VERSION, type EloSeed, type Game, type LeagueConfig } from '../src/contracts.ts';

// Both implementations consume this fixture, so model changes cannot silently invalidate Rust tuning.
const fixture = JSON.parse(
  readFileSync(new URL('../../../crates/rating-core/tests/fixtures/bayesian-parity.json', import.meta.url), 'utf8'),
) as {
  config: LeagueConfig;
  seed: EloSeed;
  games: Game[];
  cases: {
    training_count: number;
    means: number[];
    covariance: number[][];
    predictions: {
      home: string;
      away: string;
      neutral: boolean;
      phase: Game['phase'];
      home_win: number;
      away_win: number;
      tie: number;
    }[];
  }[];
};

it('matches the shared Rust Bayesian parity fixture', () => {
  expect(fixture.seed.schema_version).toBe(SEED_SCHEMA_VERSION);
  for (const sample of fixture.cases) {
    const model = fitPosterior(fixture.seed, fixture.games.slice(0, sample.training_count), fixture.config);
    model.means.forEach((v, i) => expect(v).toBeCloseTo(sample.means[i], 9));
    model.covariance.forEach((row, i) => row.forEach((v, j) => expect(v).toBeCloseTo(sample.covariance[i][j], 9)));
    for (const q of sample.predictions) {
      const p = predict(model, q.home, q.away, q.neutral, q.phase);
      expect(p.home_win).toBeCloseTo(q.home_win, 9);
      expect(p.away_win).toBeCloseTo(q.away_win, 9);
      expect(p.tie).toBeCloseTo(q.tie, 9);
    }
  }
});
