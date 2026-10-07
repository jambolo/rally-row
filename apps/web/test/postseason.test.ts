import { describe, expect, it } from 'vitest';
import type { LeagueConfig } from '../src/contracts.ts';
import {
  bracketOrder,
  formatOdds,
  formatRecord,
  isListed,
  postseasonState,
  POSTSEASON_SIMULATIONS,
  simulatePostseason,
  statusText,
  type PostseasonOdds,
  type TeamOdds,
} from '../src/postseason.ts';
import { mlb, nfl, posterior, postseason, regular, roundRobin, transitive, withHomeAdvantage } from './postseason-helpers.ts';

describe('bracket order and formatters', () => {
  it('POSTSEASON_SIMULATIONS is 10,000', () => {
    expect(POSTSEASON_SIMULATIONS).toBe(10_000);
  });

  it('bracketOrder lists seeds in bracket slot order', () => {
    expect(bracketOrder(1)).toEqual([1]);
    expect(bracketOrder(2)).toEqual([1, 2]);
    expect(bracketOrder(4)).toEqual([1, 4, 2, 3]);
    expect(bracketOrder(8)).toEqual([1, 8, 4, 5, 2, 7, 3, 6]);
    expect(bracketOrder(16)).toEqual([1, 16, 8, 9, 4, 13, 5, 12, 2, 15, 7, 10, 3, 14, 6, 11]);
  });

  it('formatOdds clamps, rounds and marks the extremes', () => {
    const cases: [number, string][] = [
      [0, '0%'],
      [-0.5, '0%'],
      [1, '100%'],
      [1.5, '100%'],
      [1e-9, '<1%'],
      [0.004, '<1%'],
      [0.005, '1%'],
      [0.123, '12%'],
      [0.5, '50%'],
      [0.994, '99%'],
      [0.995, '>99%'],
      [0.999999, '>99%'],
    ];
    for (const [p, out] of cases) expect(formatOdds(p)).toBe(out);
  });

  it('formatRecord shows ties only when there are some', () => {
    expect(formatRecord({ wins: 10, losses: 6, ties: 1 })).toBe('10-6-1');
    expect(formatRecord({ wins: 10, losses: 7, ties: 0 })).toBe('10-7');
    expect(formatRecord({ wins: 0, losses: 0, ties: 0 })).toBe('0-0');
  });

  it('statusText describes every status kind', () => {
    const rounds = [
      { name: 'Wild Card Series', short: 'WC', games: 3 },
      { name: 'Division Series', short: 'DS', games: 5 },
    ];
    expect(statusText(null, rounds)).toBe('');
    expect(statusText({ kind: 'out' }, rounds)).toBe('Out');
    expect(statusText({ kind: 'pending' }, rounds)).toBe('Pending');
    expect(statusText({ kind: 'qualified' }, rounds)).toBe('Qualified');
    expect(statusText({ kind: 'bye' }, rounds)).toBe('Bye');
    expect(statusText({ kind: 'champion' }, rounds)).toBe('Champion');
    expect(statusText({ kind: 'series', round: 1, wins: 2, losses: 1, opponent: 'NYY' }, rounds)).toBe('DS 2-1 vs NYY');
    expect(statusText({ kind: 'advanced', round: 0 }, rounds)).toBe('Won WC');
    expect(statusText({ kind: 'eliminated', round: 1 }, rounds)).toBe('Lost DS');
  });

  it('isListed keeps playoff contenders in the regular season and teams still playing afterward', () => {
    const team = (playoffs: number, status: TeamOdds['status']) => ({ playoffs, status }) as TeamOdds;
    expect(isListed(team(10 / POSTSEASON_SIMULATIONS, null), 'regular')).toBe(true);
    expect(isListed(team(9 / POSTSEASON_SIMULATIONS, null), 'regular')).toBe(false);
    expect(isListed(team(0, null), 'regular')).toBe(false);
    const playing: TeamOdds['status'][] = [
      { kind: 'pending' },
      { kind: 'qualified' },
      { kind: 'bye' },
      { kind: 'champion' },
      { kind: 'series', round: 1, wins: 0, losses: 2, opponent: 'NYY' },
      { kind: 'advanced', round: 0 },
    ];
    for (const status of playing) expect(isListed(team(1, status), 'postseason')).toBe(true);
    expect(isListed(team(0, { kind: 'out' }), 'postseason')).toBe(false);
    expect(isListed(team(1, { kind: 'eliminated', round: 1 }), 'postseason')).toBe(false);
  });
});

function row(odds: PostseasonOdds, id: string): TeamOdds {
  const team = odds.conferences.flatMap((c) => c.teams).find((t) => t.id === id);
  if (!team) throw new Error(`No row for ${id}`);
  return team;
}

function expectInvariants(odds: PostseasonOdds): void {
  const sum = (values: number[]) => values.reduce((a, b) => a + b, 0);
  for (const c of odds.conferences) {
    expect(sum(c.teams.map((t) => t.playoffs))).toBeCloseTo(odds.playoff_spots, 9);
    expect(sum(c.teams.map((t) => t.bye))).toBeCloseTo(odds.byes, 9);
    for (const division of new Set(c.teams.map((t) => t.division)))
      expect(sum(c.teams.filter((t) => t.division === division).map((t) => t.win_division))).toBeCloseTo(1, 9);
  }
  const teams = odds.conferences.flatMap((c) => c.teams);
  expect(sum(teams.map((t) => t.title))).toBeCloseTo(1, 9);
  for (const t of teams) {
    const chain = [t.playoffs, ...t.reach, t.title];
    for (let i = 1; i < chain.length; i++) expect(chain[i - 1]).toBeGreaterThanOrEqual(chain[i]);
  }
}

describe('regular season', () => {
  it('NFL odds satisfy the sum and order invariants', () => {
    const odds = simulatePostseason(posterior(nfl), roundRobin(nfl, 17), { simulations: 2000 });
    expect(odds.status).toBe('ready');
    expect(odds.mode).toBe('regular');
    expect(odds.simulations).toBe(2000);
    expect(odds.playoff_spots).toBe(7);
    expect(odds.byes).toBe(1);
    expect(odds.rounds).toEqual([
      { name: 'Wild Card', short: 'WC', games: 1 },
      { name: 'Divisional', short: 'DIV', games: 1 },
      { name: 'Conference Championship', short: 'CON', games: 1 },
      { name: 'Super Bowl', short: 'SB', games: 1 },
    ]);
    expect(odds.conferences.map((c) => c.id)).toEqual(['AFC', 'NFC']);
    expect(odds.conferences.map((c) => c.teams.length)).toEqual([16, 16]);
    expect(odds.notes).toEqual([]);
    for (const c of odds.conferences)
      for (const t of c.teams) {
        expect(t.status).toBeNull();
        expect(t.reach).toHaveLength(3);
      }
    expectInvariants(odds);
  });

  it('MLB odds satisfy the sum and order invariants', () => {
    const odds = simulatePostseason(posterior(mlb), roundRobin(mlb, 162), { simulations: 2000 });
    expect(odds.mode).toBe('regular');
    expect(odds.playoff_spots).toBe(6);
    expect(odds.byes).toBe(2);
    expect(odds.rounds.map((r) => r.games)).toEqual([3, 5, 7, 7]);
    expect(odds.conferences.map((c) => c.id)).toEqual(['AL', 'NL']);
    expectInvariants(odds);
  });

  it('two runs with the same options are deep-equal', () => {
    const games = roundRobin(nfl, 17);
    const a = simulatePostseason(posterior(nfl), games, { simulations: 500 });
    const b = simulatePostseason(posterior(nfl), games, { simulations: 500 });
    expect(a).toEqual(b);
    expect(simulatePostseason(posterior(nfl), games, { simulations: 500, seed: 7 })).not.toEqual(a);
  });

  it('a dominant team wins the title with probability near 1', () => {
    const odds = simulatePostseason(posterior(nfl, { KC: 10 }), roundRobin(nfl, 17), { simulations: 1000 });
    expect(row(odds, 'KC').playoffs).toBe(1);
    expect(row(odds, 'KC').title).toBeGreaterThan(0.99);
  });

  it('builds rows with records, divisions and the documented sort order', () => {
    const games = [
      ...transitive(nfl).map((g) => (g.home_team === 'PIT' && g.away_team === 'TEN' ? { ...g, result: 'tie' as const } : g)),
      ...regular([['TEN', 'NYJ', null]]),
    ];
    const odds = simulatePostseason(posterior(nfl), games, { simulations: 500 });
    expect(odds.mode).toBe('regular');
    expect(row(odds, 'BAL')).toMatchObject({
      division: 'AFC-N',
      division_label: 'North',
      record: { wins: 15, losses: 0, ties: 0 },
      mean_seed: 1,
      playoffs: 1,
      win_division: 1,
      bye: 1,
      status: null,
    });
    expect(row(odds, 'CIN')).toMatchObject({ mean_seed: 5, playoffs: 1, win_division: 0, bye: 0 });
    expect(row(odds, 'JAX')).toMatchObject({ record: { wins: 8, losses: 7, ties: 0 }, mean_seed: null, playoffs: 0 });
    expect(row(odds, 'PIT').record).toEqual({ wins: 0, losses: 14, ties: 1 });
    expect(row(odds, 'TEN').record).toEqual({ wins: 0, losses: 14, ties: 1 });
    const afc = odds.conferences[0]!.teams.map((t) => t.id);
    expect(afc.slice(0, 7).sort()).toEqual(['BAL', 'BUF', 'CIN', 'CLE', 'DEN', 'HOU', 'IND']);
    expect(afc.slice(7)).toEqual(['JAX', 'KC', 'LAC', 'LV', 'MIA', 'NE', 'NYJ', 'PIT', 'TEN']);
  });

  it('postseasonState reports a league without a postseason format', () => {
    const config: LeagueConfig = { ...mlb };
    delete config.postseason;
    expect(postseasonState(posterior(config), roundRobin(mlb, 2))).toEqual({
      status: 'error',
      error: 'League has no postseason format',
    });
    expect(() => simulatePostseason(posterior(config), roundRobin(mlb, 2))).toThrow('League has no postseason format');
  });

  it('simulates a full MLB season 10,000 times within 15 seconds', () => {
    const games = roundRobin(mlb, 162);
    const start = performance.now();
    const odds = simulatePostseason(posterior(mlb), games);
    const elapsed = performance.now() - start;
    expect(odds.simulations).toBe(10_000);
    expect(elapsed).toBeLessThan(15_000);
  }, 60_000);
});

const UNFIT = "The listed postseason games don't fit the configured playoff format.";
const NL = ['ATL', 'AZ', 'CHC', 'CIN', 'COL', 'LAD', 'MIA', 'MIL', 'NYM', 'PHI', 'PIT', 'SD', 'SF', 'STL', 'WSH'];

describe('postseason mode', () => {
  it('Division Series venues follow the HHAAH pattern', () => {
    const T = transitive(mlb);
    const games = [
      ...T,
      ...postseason('D', [
        ['ATH', 'BOS', 'A'],
        ['ATH', 'BOS', 'A'],
      ]),
    ];
    const odds = simulatePostseason(posterior(withHomeAdvantage(mlb, 2e5), {}, 1e-6), games, { simulations: 200 });
    expect(odds.mode).toBe('postseason');
    expect(odds.notes).toEqual([]);
    expect(row(odds, 'BOS').reach).toEqual([1, 1, 0]);
    expect(row(odds, 'ATH').reach).toEqual([1, 0, 0]);
    expect(row(odds, 'BAL').reach).toEqual([1, 1, 1]);
    expect(row(odds, 'ATL').title).toBe(1);
    expect(row(odds, 'BOS').status).toEqual({ kind: 'series', round: 1, wins: 2, losses: 0, opponent: 'ATH' });
    expect(row(odds, 'ATH').status).toEqual({ kind: 'series', round: 1, wins: 0, losses: 2, opponent: 'BOS' });
  });

  it('NFL teams listed in a later round are forced through earlier rounds', () => {
    const TN = transitive(nfl);
    const games = [...TN, ...postseason('WC', [['BUF', 'IND', 'H']]), ...postseason('CON', [['HOU', 'CLE', 'H']])];
    const odds = simulatePostseason(posterior(nfl), games, { simulations: 500 });
    expect(odds.notes).toEqual([]);
    expect(row(odds, 'HOU').reach).toEqual([1, 1, 1]);
    expect(row(odds, 'CLE').reach).toEqual([1, 1, 0]);
    expect(row(odds, 'BAL').reach).toEqual([1, 0, 0]);
    expect(row(odds, 'BUF').reach).toEqual([1, 0, 0]);
    expect(row(odds, 'IND').reach).toEqual([0, 0, 0]);
    expect(row(odds, 'DEN').reach[0]).toBe(0);
    expect(row(odds, 'CIN').reach[0]).toBe(0);
    expect(row(odds, 'HOU').status).toEqual({ kind: 'advanced', round: 2 });
    expect(row(odds, 'CLE').status).toEqual({ kind: 'eliminated', round: 2 });
    expect(row(odds, 'BUF').status).toEqual({ kind: 'advanced', round: 0 });
    expect(row(odds, 'IND').status).toEqual({ kind: 'eliminated', round: 0 });
  });

  it('mid-postseason MLB statuses cover series, advanced, eliminated, bye, qualified and out', () => {
    const T = transitive(mlb);
    const games = [
      ...T,
      ...postseason('F', [
        ['BOS', 'CWS', 'H'],
        ['BOS', 'CWS', 'H'],
        ['CLE', 'DET', 'H'],
        ['CLE', 'DET', null],
      ]),
    ];
    const odds = simulatePostseason(posterior(mlb), games, { simulations: 500 });
    expect(odds.mode).toBe('postseason');
    expect(odds.notes).toEqual([]);
    expect(row(odds, 'BOS').status).toEqual({ kind: 'advanced', round: 0 });
    expect(row(odds, 'CWS').status).toEqual({ kind: 'eliminated', round: 0 });
    expect(row(odds, 'CLE').status).toEqual({ kind: 'series', round: 0, wins: 1, losses: 0, opponent: 'DET' });
    expect(row(odds, 'DET').status).toEqual({ kind: 'series', round: 0, wins: 0, losses: 1, opponent: 'CLE' });
    expect(row(odds, 'ATH').status).toEqual({ kind: 'bye' });
    expect(row(odds, 'BAL').status).toEqual({ kind: 'bye' });
    expect(row(odds, 'CHC').status).toEqual({ kind: 'qualified' });
    expect(row(odds, 'HOU').status).toEqual({ kind: 'out' });
    expect(statusText(row(odds, 'BOS').status, odds.rounds)).toBe('Won WC');
    expect(statusText(row(odds, 'CLE').status, odds.rounds)).toBe('WC 1-0 vs DET');
    expect(row(odds, 'CWS').reach).toEqual([0, 0, 0]);
  });

  it('a clinched World Series makes a champion', () => {
    const T = transitive(mlb);
    const games = [
      ...T,
      ...postseason('W', [
        ['ATH', 'ATL', 'H'],
        ['ATH', 'ATL', 'H'],
        ['ATL', 'ATH', 'A'],
        ['ATL', 'ATH', 'A'],
      ]),
    ];
    const odds = simulatePostseason(posterior(mlb), games, { simulations: 300 });
    expect(row(odds, 'ATH').status).toEqual({ kind: 'champion' });
    expect(row(odds, 'ATH').title).toBe(1);
    expect(row(odds, 'ATL').status).toEqual({ kind: 'eliminated', round: 3 });
    expect(row(odds, 'ATL').reach).toEqual([1, 1, 1]);
    expect(row(odds, 'ATL').title).toBe(0);
  });

  it('random seeds before the Wild Card give pending and note N4', () => {
    const games = transitive(mlb).filter((g) => !NL.includes(g.home_team));
    const odds = simulatePostseason(posterior(mlb), games, { simulations: 500 });
    expect(odds.mode).toBe('postseason');
    expect(odds.notes).toEqual(['Some seeds depend on random tiebreak draws.']);
    expect(row(odds, 'ATH').status).toEqual({ kind: 'bye' });
    expect(row(odds, 'CLE').status).toEqual({ kind: 'qualified' });
    expect(row(odds, 'HOU').status).toEqual({ kind: 'out' });
    for (const id of NL) expect(row(odds, id).status).toEqual({ kind: 'pending' });
    expectInvariants(odds);
  });

  it('a seeding mismatch relaxes the tiebreakers with note N3', () => {
    const games = [
      ...transitive(mlb).map((g) => (g.home_team === 'DET' && g.away_team === 'HOU' ? { ...g, result: 'tie' as const } : g)),
      ...postseason('F', [['CLE', 'DET', 'H']]),
    ];
    const odds = simulatePostseason(posterior(mlb), games, { simulations: 300 });
    expect(odds.notes).toEqual(['Computed tiebreakers disagree with the listed postseason games; seeds follow the listed games.']);
    expect(row(odds, 'DET').playoffs).toBe(1);
    expect(row(odds, 'HOU').playoffs).toBe(0);
    expect(row(odds, 'DET').status).toEqual({ kind: 'series', round: 0, wins: 0, losses: 1, opponent: 'CLE' });
    expect(row(odds, 'HOU').status).toEqual({ kind: 'out' });
  });

  it('an impossible listing returns the E1 error state', () => {
    const games = [...transitive(mlb), ...postseason('F', [['ATH', 'BAL', 'H']])];
    expect(postseasonState(posterior(mlb), games, { simulations: 50 })).toEqual({ status: 'error', error: UNFIT });
    expect(() => simulatePostseason(posterior(mlb), games, { simulations: 50 })).toThrow(UNFIT);
  });

  it('unknown round labels add note N2 once per label', () => {
    const games = [
      ...transitive(mlb),
      ...postseason('X', [
        ['ATH', 'BAL', 'H'],
        ['ATH', 'BAL', null],
      ]),
      ...postseason('Y', [['NYY', 'TB', 'H']]),
    ];
    const odds = simulatePostseason(posterior(mlb), games, { simulations: 50 });
    expect(odds.mode).toBe('postseason');
    expect(odds.notes).toEqual([
      'Ignored postseason games with unknown round label "X".',
      'Ignored postseason games with unknown round label "Y".',
    ]);
    expect(row(odds, 'ATH').status).toEqual({ kind: 'bye' });
  });

  it('drops unplayed regular games once the postseason starts with note N1', () => {
    const started = postseason('F', [['BOS', 'CWS', 'H']]);
    const a = simulatePostseason(posterior(mlb), [...transitive(mlb), ...regular([['NYY', 'TB', null]]), ...started], {
      simulations: 50,
    });
    expect(a.mode).toBe('postseason');
    expect(a.notes).toEqual(['Ignored 1 unplayed regular-season game because the postseason has started.']);
    expect(row(a, 'NYY').record).toEqual({ wins: 4, losses: 10, ties: 0 });
    const b = simulatePostseason(
      posterior(mlb),
      [
        ...transitive(mlb),
        ...regular([
          ['NYY', 'TB', null],
          ['TB', 'NYY', null],
        ]),
        ...started,
      ],
      { simulations: 50 },
    );
    expect(b.notes).toEqual(['Ignored 2 unplayed regular-season games because the postseason has started.']);
  });

  it('rejects inconsistent listed series with errors E2, E3 and E4', () => {
    const run = (rows: Parameters<typeof postseason>[1]) =>
      postseasonState(posterior(mlb), [...transitive(mlb), ...postseason('F', rows)], { simulations: 50 });
    expect(
      run([
        ['BOS', 'CWS', 'H'],
        ['BOS', 'DET', 'H'],
      ]),
    ).toEqual({
      status: 'error',
      error: 'Conflicting Wild Card Series series for BOS',
    });
    expect(
      run([
        ['CWS', 'BOS', 'A'],
        ['CWS', 'BOS', 'A'],
        ['CWS', 'BOS', 'A'],
      ]),
    ).toEqual({
      status: 'error',
      error: 'Too many wins in the Wild Card Series series between BOS and CWS',
    });
    expect(
      run([
        ['CWS', 'BOS', 'H'],
        ['CWS', 'BOS', 'H'],
        ['CWS', 'BOS', 'A'],
        ['CWS', 'BOS', 'A'],
      ]),
    ).toEqual({
      status: 'error',
      error: 'Both teams clinched the Wild Card Series series between BOS and CWS',
    });
  });

  it('regular mode ignores listed postseason games', () => {
    const games = [
      ...transitive(mlb),
      ...regular([['NYY', 'TB', null]]),
      ...postseason('F', [['BOS', 'CWS', null]]),
      ...postseason('X', [['ATH', 'BAL', null]]),
    ];
    const odds = simulatePostseason(posterior(mlb), games, { simulations: 50 });
    expect(odds.mode).toBe('regular');
    expect(odds.notes).toEqual([]);
    for (const c of odds.conferences) for (const t of c.teams) expect(t.status).toBeNull();
  });
});
