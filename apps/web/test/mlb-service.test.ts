import { readFileSync } from 'node:fs';
import { afterEach, expect, it, vi } from 'vitest';
import { mlbStatsApi } from '../src/adapters/mlb-statsapi.ts';
import { configSchema, type EloSeed } from '../src/contracts.ts';
import * as model from '../src/model.ts';
import { parseSource, usableResults } from '../src/provider.ts';
import { PredictionService } from '../src/service.ts';
import { readSnapshot } from '../src/snapshot.ts';
import { digest, memoryStore, type Store } from '../src/storage.ts';

const configBytes = readFileSync(new URL('../../../config/mlb.json', import.meta.url));
const config = configSchema.parse(JSON.parse(configBytes.toString()));
const configHash = await digest(configBytes);
const historyHash = await digest('mlb history');
const dataBase = 'https://published.test/data';
const seasonUrl = 'https://statsapi.mlb.com/api/v1/schedule?sportId=1&season=2026&gameType=R,F,D,L,W';

function seed(): EloSeed {
  return {
    schema_version: 1,
    league: 'mlb',
    target_season: 2026,
    through_season: 2025,
    generated_at: '2026-01-15T00:00:00Z',
    history_sha256: historyHash,
    config_sha256: configHash,
    settings: config.elo,
    completed_games: 2400,
    tied_games: 0,
    tie_weight: 0.0003,
    ratings: config.teams.map((t) => ({ team: t.id, elo: config.elo.initial, games: 162 })),
  };
}
function publish() {
  const published: Record<string, string> = {
    [`${dataBase}/mlb/history.sha256`]: historyHash,
    [`${dataBase}/mlb/elo-2026.json`]: JSON.stringify(seed()),
  };
  vi.stubGlobal('fetch', (input: string | URL | Request) => {
    const body = published[String(input)];
    return Promise.resolve(body === undefined ? new Response('missing', { status: 404 }) : new Response(body, { status: 200 }));
  });
}
/** One 2026 regular-season listing; scores only when given. */
function entry(
  gamePk: number,
  officialDate: string,
  gameDate: string,
  detailedState: string,
  home: [number, number?],
  away: [number, number?],
) {
  const side = ([id, score]: [number, number?]) => ({ team: { id }, ...(score === undefined ? {} : { score }) });
  return {
    gamePk,
    gameType: 'R',
    season: '2026',
    gameDate,
    officialDate,
    status: { detailedState },
    teams: { home: side(home), away: side(away) },
    isTie: false,
  };
}
const schedule = (entries: unknown[]) => JSON.stringify({ dates: [{ date: '2026-05-01', games: entries }] });
// BOS 111, NYY 147, TB 139, TOR 141.
const games = [
  // Evening game on May 1 local time; its UTC start date is May 2.
  entry(1, '2026-05-01', '2026-05-02T01:10:00Z', 'Final', [111, 5], [147, 3]),
  // A doubleheader on May 2.
  entry(2, '2026-05-02', '2026-05-02T17:05:00Z', 'Final', [139, 4], [141, 2]),
  entry(3, '2026-05-02', '2026-05-02T23:05:00Z', 'Final', [139, 1], [141, 6]),
  // Started, scored, not final.
  entry(4, '2026-05-02', '2026-05-02T23:10:00Z', 'In Progress', [111, 2], [147, 1]),
  entry(5, '2026-05-20', '2026-05-20T23:05:00Z', 'Scheduled', [147], [111]),
];
const service = (now: string, store: Store | null = memoryStore(), fetchSource = async () => schedule(games)) =>
  new PredictionService({
    config,
    configHash,
    dataBase,
    season: 2026,
    postseasonSimulations: 200,
    now: () => new Date(now),
    store,
    fetchSource,
  });

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

it('counts a final result at its own start instant and never trains on a non-final game', async () => {
  const parsed = parseSource([schedule(games)], config);
  const first = parsed.find((g) => g.id === '1')!;
  expect(mlbStatsApi.isResultEligible(first, new Date(first.start_time_utc!))).toBe(true);
  expect(usableResults(parsed, config, new Date(first.start_time_utc!)).map((g) => g.id)).toEqual(['1', '2', '3']);
  expect(parsed.find((g) => g.id === '4')?.result).toBeNull();
  publish();
  const s = service('2026-05-02T23:30:00Z');
  await s.initialize();
  expect(s.getState()).toMatchObject({ status: 'ready', training_games: 3, held_results: 0 });
  expect(s.getState().games.map((g) => [g.id, g.status])).toEqual([
    ['1', 'completed'],
    ['2', 'completed'],
    ['3', 'completed'],
    ['4', 'awaiting_result'],
    ['5', 'scheduled'],
  ]);
});

it('excludes same-official-date doubleheader games and includes earlier official dates in pregame models', async () => {
  publish();
  const fitted = vi.spyOn(model, 'fitPosterior');
  const s = service('2026-05-03T12:00:00Z');
  await s.initialize();
  const byId = (id: string) => s.getState().games.find((g) => g.id === id)!;
  // Game 1 starts on the same UTC date as game 3 but on an earlier official date.
  expect(byId('1').start_time_utc!.slice(0, 10)).toBe(byId('3').start_time_utc!.slice(0, 10));
  // Full fit, then one pregame fit per official date: May 1 (no earlier results), May 2 (game 1 only).
  expect(fitted.mock.calls.map(([, results]) => results.map((g) => g.id))).toEqual([['1', '2', '3'], [], ['1']]);
  const pregame = model.fitPosterior(seed(), [byId('1')], config);
  expect(byId('3').prediction).toEqual(model.predict(pregame, 'TB', 'TOR', false, 'regular'));
  expect(byId('2').prediction).toEqual(byId('3').prediction);
});

it('abbreviates teams by era rather than by numeric source id', () => {
  const renamed = (target: number) =>
    Object.fromEntries(
      model
        .teamEstimates(model.fitPosterior({ ...seed(), target_season: target, through_season: target - 1 }, [], config))
        .filter((t) => t.abbreviation !== t.id)
        .map((t) => [t.id, t.abbreviation]),
    );
  expect(renamed(2026)).toEqual({});
  expect(renamed(2004)).toEqual({ ATH: 'OAK', LAA: 'ANA', MIA: 'FLA', WSH: 'MON' });
});

it('publishes a season-resolved source link and the MLB result policy', async () => {
  publish();
  const store = memoryStore();
  const fetchSource = vi.fn(async () => schedule(games));
  const s = service('2026-05-03T12:00:00Z', store, fetchSource);
  expect(s.getState().source).toBe(seasonUrl);
  await s.initialize();
  expect(fetchSource.mock.calls).toEqual([[seasonUrl]]);
  const policy = {
    source: seasonUrl,
    result_policy_summary: 'Results enter the picture as soon as the provider marks a game final.',
    result_policy:
      'Results count as soon as the provider marks a game final; postponed, suspended, and cancelled games are not results.',
  };
  expect(s.getState()).toMatchObject({ status: 'ready', ...policy });
  expect(s.getState().source).not.toContain('{season}');
  expect(readSnapshot(store, 'mlb')?.state).toMatchObject(policy);
});

it('persists nothing when the store option is omitted', async () => {
  publish();
  const storage = { getItem: vi.fn(), setItem: vi.fn(), removeItem: vi.fn(), key: vi.fn(), clear: vi.fn(), length: 0 };
  vi.stubGlobal('localStorage', storage);
  const s = new PredictionService({
    config,
    configHash,
    dataBase,
    season: 2026,
    postseasonSimulations: 200,
    now: () => new Date('2026-05-03T12:00:00Z'),
    fetchSource: async () => schedule(games),
  });
  await s.initialize();
  expect(s.getState().status).toBe('ready');
  for (const method of [storage.getItem, storage.setItem, storage.removeItem, storage.key, storage.clear])
    expect(method).not.toHaveBeenCalled();
});

it('publishes postseason odds in the MLB service state', async () => {
  publish();
  const s = service('2026-05-03T12:00:00Z');
  await s.initialize();
  expect(s.getState()).toMatchObject({ status: 'ready', postseason: { status: 'ready', mode: 'regular' } });
});
