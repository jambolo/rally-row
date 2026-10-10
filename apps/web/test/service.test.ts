import { afterEach, it, expect, vi } from 'vitest';
import { PredictionService } from '../src/service.ts';
import { fitPosterior, predict } from '../src/model.ts';
import * as model from '../src/model.ts';
import * as postseason from '../src/postseason.ts';
import type { LeagueConfig } from '../src/contracts.ts';
import { readSnapshot, snapshotKey } from '../src/snapshot.ts';
import { digest, memoryStore, type Store } from '../src/storage.ts';
import { config, configHash, game, historyHashFile, seed } from './helpers.ts';
import { version as appVersion } from '../package.json';

const now = () => new Date('2026-09-19T18:00:00Z');
const dataBase = 'https://published.test/data';
const csv =
  'game_id,season,game_type,week,gameday,gametime,away_team,away_score,home_team,home_score,location\ng1,2026,REG,1,2026-09-01,13:00,SF,10,SEA,20,Home\ng2,2026,REG,2,2026-09-20,13:00,KC,,SEA,,Home\n';
const cacheKey = 'game-results-prediction:nfl:current-2026';

/** Stands in for the published static assets the Rust programs generate. */
function publish(files: Record<string, string> = {}) {
  const published: Record<string, string> = {
    [`${dataBase}/nfl/history.sha256`]: historyHashFile,
    [`${dataBase}/nfl/elo-2026.json`]: JSON.stringify(seed()),
    ...files,
  };
  const requests: { url: string; method: string; cache: RequestCache | undefined }[] = [];
  vi.stubGlobal('fetch', (input: string | URL | Request, init?: RequestInit) => {
    const url = String(input);
    requests.push({ url, method: init?.method ?? 'GET', cache: init?.cache });
    const body = published[url];
    return Promise.resolve(
      body === undefined ? new Response('missing', { status: 404 }) : new Response(new TextEncoder().encode(body), { status: 200 }),
    );
  });
  return { published, requests };
}
const service = (store: Store, fetchSource: () => Promise<string>) =>
  new PredictionService({ config, configHash, dataBase, season: 2026, postseasonSimulations: 200, now, store, fetchSource });

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

it('refreshes once per startup, uses each result once, and does not rewrite history or Elo', async () => {
  const { published, requests } = publish();
  const store = memoryStore();
  let sources = 0;
  const run = async () => {
    const s = service(store, async () => {
      sources++;
      return csv;
    });
    await s.initialize();
    return s;
  };
  const a = await run(),
    b = await run();
  expect(sources).toBe(2);
  expect(a.getState().status).toBe('ready');
  expect(a.getState().training_games).toBe(1);
  expect(a.getState().games[0]).toMatchObject({ round: 1, round_label: 'REG', start_time_utc: '2026-09-01T17:00:00Z' });
  expect(JSON.parse(store.getItem(cacheKey)!).schema_version).toBe(2);
  expect(a.predict('SEA', 'SF', true, 'regular')).toEqual(b.predict('SEA', 'SF', true, 'regular'));
  expect(requests.every((r) => r.method === 'GET')).toBe(true);
  expect(published[`${dataBase}/nfl/history.sha256`]).toBe(historyHashFile);
  expect(JSON.parse(published[`${dataBase}/nfl/elo-2026.json`]!)).toEqual(seed());
});

it('falls back visibly to valid cached data without overwriting it', async () => {
  publish();
  const store = memoryStore();
  await service(store, async () => csv).initialize();
  const cached = store.getItem(cacheKey);
  const second = service(store, () => Promise.reject(new Error('offline')));
  await second.initialize();
  expect(second.getState().status).toBe('ready');
  expect(second.getState().cached).toBe(true);
  expect(second.getState().warning).toContain('offline');
  expect(store.getItem(cacheKey)).toBe(cached);
});

it('keeps an obsolete cache until a successful refresh replaces it with the current format', async () => {
  publish();
  const store = memoryStore();
  await service(store, async () => csv).initialize();
  const old = JSON.parse(store.getItem(cacheKey)!);
  old.schema_version = 1;
  const saved = JSON.stringify(old);
  store.setItem(cacheKey, saved);
  store.removeItem(snapshotKey('nfl'));
  const offline = service(store, () => Promise.reject(new Error('offline')));
  await offline.initialize();
  expect(offline.getState().status).toBe('error');
  expect(offline.getState().error).toContain('No valid current-season cache');
  expect(store.getItem(cacheKey)).toBe(saved);
  const refreshed = service(store, async () => csv);
  await refreshed.initialize();
  expect(refreshed.getState().status).toBe('ready');
  expect(refreshed.getState().warning).toContain('Replaced it with a valid download');
  expect(JSON.parse(store.getItem(cacheKey)!).schema_version).toBe(2);
});

it('revalidates published inputs and skips fitting for unchanged normalized data', async () => {
  const { requests } = publish();
  const store = memoryStore();
  const first = service(store, async () => csv);
  await first.initialize();
  const fitted = vi.spyOn(model, 'fitPosterior');
  const progress = vi.fn();
  expect(readSnapshot(store, 'nfl')?.app_version).toBe(appVersion);
  requests.length = 0;
  const rows = csv.trim().split('\n');
  const second = new PredictionService({
    config,
    configHash,
    dataBase,
    season: 2026,
    postseasonSimulations: 200,
    store,
    now: () => new Date('2026-09-19T19:00:00Z'),
    fetchSource: async () => [rows[0], rows[2], rows[1].replace('SF,10', 'SF,11')].join('\n'),
    onProgress: progress,
  });
  await second.initialize();
  expect(fitted).not.toHaveBeenCalled();
  expect(requests).toEqual([
    { url: `${dataBase}/nfl/elo-2026.json`, method: 'GET', cache: 'no-cache' },
    { url: `${dataBase}/nfl/history.sha256`, method: 'GET', cache: 'no-cache' },
  ]);
  expect(progress.mock.calls).toEqual([['checking']]);
  expect(second.predict('SEA', 'SF', true, 'regular')).toEqual(first.predict('SEA', 'SF', true, 'regular'));
  expect(second.getState()).toMatchObject({
    status: 'ready',
    cached: false,
    refreshed_at: now().toISOString(),
    checked_at: '2026-09-19T19:00:00.000Z',
  });
});

it.each(['seed only', 'history and seed'])('rebuilds unchanged games after a published update: %s', async (change) => {
  const { published } = publish();
  const store = memoryStore();
  const first = service(store, async () => csv);
  await first.initialize();
  const updated = seed();
  if (change === 'seed only') {
    updated.ratings.find((t) => t.team === 'SEA')!.elo += 150;
    updated.ratings.find((t) => t.team === 'SF')!.elo -= 150;
  } else {
    updated.history_sha256 = await digest('history corrected');
    published[`${dataBase}/nfl/history.sha256`] = updated.history_sha256;
    updated.completed_games += 1;
  }
  const seedBytes = JSON.stringify(updated);
  published[`${dataBase}/nfl/elo-2026.json`] = seedBytes;
  const fitted = vi.spyOn(model, 'fitPosterior');
  const second = service(store, async () => csv);
  await second.initialize();
  expect(fitted).toHaveBeenCalled();
  expect(second.getState()).toMatchObject({ status: 'ready', cached: false, warning: null });
  expect(second.getModel()?.seed).toEqual(updated);
  if (change === 'seed only') {
    expect(second.getState().teams.find((t) => t.id === 'SEA')!.initial_elo).toBe(config.elo.initial + 150);
    expect(second.predict('SEA', 'SF', true, 'regular').home_win).toBeGreaterThan(
      first.predict('SEA', 'SF', true, 'regular').home_win,
    );
    for (const index of [0, 1]) {
      expect(second.getState().games[index].prediction!.home_win).toBeGreaterThan(
        first.getState().games[index].prediction!.home_win,
      );
    }
  }
  expect(second.getState().historical_games).toBe(updated.completed_games);
  expect(JSON.parse(store.getItem(snapshotKey('nfl'))!).seed_sha256).toBe(await digest(seedBytes));
});

it.each([
  ['history mismatch', 'History changed since Elo was calculated'],
  ['wrong league', 'wrong league or season'],
  ['wrong season', 'wrong league or season'],
  ['wrong configuration', 'Configuration changed since Elo was calculated'],
  ['failed fit', 'Elo seed does not match the configured league and teams'],
])('retains the snapshot when unchanged games accompany %s', async (failure, warning) => {
  const { published } = publish();
  const store = memoryStore();
  const first = service(store, async () => csv);
  await first.initialize();
  const saved = store.getItem(snapshotKey('nfl'));
  const cachedGames = store.getItem(cacheKey);
  const updated = seed();
  if (failure === 'history mismatch') published[`${dataBase}/nfl/history.sha256`] = await digest('different history');
  else if (failure === 'wrong league') updated.league = 'other';
  else if (failure === 'wrong season') updated.target_season -= 1;
  else if (failure === 'wrong configuration') updated.config_sha256 = '0'.repeat(64);
  else updated.ratings.pop();
  published[`${dataBase}/nfl/elo-2026.json`] = JSON.stringify(updated);
  const second = new PredictionService({
    config,
    configHash,
    dataBase,
    season: 2026,
    postseasonSimulations: 200,
    store,
    now: () => new Date('2026-09-19T19:00:00Z'),
    fetchSource: async () => csv,
  });
  await second.initialize();
  expect(second.getState()).toEqual({ ...first.getState(), cached: true, warning: expect.stringContaining(warning) });
  expect(second.getModel()).toEqual(first.getModel());
  expect(store.getItem(snapshotKey('nfl'))).toBe(saved);
  expect(store.getItem(cacheKey)).toBe(cachedGames);
});

it.each(['elo-2026.json', 'history.sha256'])('retains the snapshot if the freshness check cannot fetch %s', async (file) => {
  const { published } = publish();
  const store = memoryStore();
  const first = service(store, async () => csv);
  await first.initialize();
  const saved = store.getItem(snapshotKey('nfl'));
  delete published[`${dataBase}/nfl/${file}`];
  const second = service(store, async () => csv);
  await second.initialize();
  expect(second.getState()).toEqual({
    ...first.getState(),
    cached: true,
    warning: expect.stringContaining(`Not published: ${dataBase}/nfl/${file}`),
  });
  expect(second.getModel()).toEqual(first.getModel());
  expect(store.getItem(snapshotKey('nfl'))).toBe(saved);
});

it('rebuilds a compatible snapshot once to record its missing seed fingerprint', async () => {
  const { published } = publish();
  const store = memoryStore();
  await service(store, async () => csv).initialize();
  const legacy = JSON.parse(store.getItem(snapshotKey('nfl'))!);
  delete legacy.seed_sha256;
  store.setItem(snapshotKey('nfl'), JSON.stringify(legacy));
  expect(readSnapshot(store, 'nfl')).not.toBeNull();
  const fitted = vi.spyOn(model, 'fitPosterior');
  await service(store, async () => csv).initialize();
  expect(fitted).toHaveBeenCalled();
  expect(JSON.parse(store.getItem(snapshotKey('nfl'))!).seed_sha256).toBe(await digest(published[`${dataBase}/nfl/elo-2026.json`]));
  fitted.mockClear();
  await service(store, async () => csv).initialize();
  expect(fitted).not.toHaveBeenCalled();
});

it.each([undefined, '0.0.0', '999.0.0', 42])('rebuilds unchanged inputs when the saved app version is %s', async (version) => {
  publish();
  const store = memoryStore();
  await service(store, async () => csv).initialize();
  const saved = JSON.parse(store.getItem(snapshotKey('nfl'))!);
  saved.app_version = version;
  store.setItem(snapshotKey('nfl'), JSON.stringify(saved));
  expect(readSnapshot(store, 'nfl')).toBeNull();
  const fitted = vi.spyOn(model, 'fitPosterior');
  const progress = vi.fn(() => expect(second.getModel()).toBeNull());
  const second = new PredictionService({
    config,
    configHash,
    dataBase,
    season: 2026,
    postseasonSimulations: 200,
    now,
    store,
    fetchSource: async () => csv,
    onProgress: progress,
  });
  await second.initialize();
  expect(fitted).toHaveBeenCalled();
  expect(progress.mock.calls).toHaveLength(2);
  expect(second.getState()).toMatchObject({ status: 'ready', cached: false, warning: null });
  expect(readSnapshot(store, 'nfl')).toMatchObject({
    app_version: appVersion,
    file: { games: saved.file.games },
    model: second.getModel(),
  });
});

it.each([true, false])('validates raw game caches independently during an app upgrade; valid: %s', async (valid) => {
  publish();
  const store = memoryStore();
  await service(store, async () => csv).initialize();
  const saved = JSON.parse(store.getItem(snapshotKey('nfl'))!);
  saved.app_version = '0.0.0';
  store.setItem(snapshotKey('nfl'), JSON.stringify(saved));
  if (!valid) {
    const games = JSON.parse(store.getItem(cacheKey)!);
    games.schema_version = 1;
    store.setItem(cacheKey, JSON.stringify(games));
  }
  const second = service(store, async () => {
    throw new Error('offline');
  });
  await second.initialize();
  if (valid) {
    expect(second.getState()).toMatchObject({ status: 'ready', cached: true, warning: expect.stringContaining('offline') });
    expect(readSnapshot(store, 'nfl')?.app_version).toBe(appVersion);
  } else {
    expect(second.getState()).toMatchObject({ status: 'error', error: expect.stringContaining('No valid current-season cache') });
    expect(second.getModel()).toBeNull();
    expect(store.getItem(snapshotKey('nfl'))).toBe(JSON.stringify(saved));
  }
});

it.each(['fetch', 'validation', 'prediction'])(
  'preserves stored data after an app upgrade %s failure and recovers',
  async (failure) => {
    const { published } = publish();
    const store = memoryStore();
    await service(store, async () => csv).initialize();
    const legacy = JSON.parse(store.getItem(snapshotKey('nfl'))!);
    legacy.app_version = '0.0.0';
    const saved = JSON.stringify(legacy);
    store.setItem(snapshotKey('nfl'), saved);
    const savedGames = store.getItem(cacheKey);
    const seedUrl = `${dataBase}/nfl/elo-2026.json`;
    const seedBytes = published[seedUrl];
    if (failure === 'fetch') delete published[seedUrl];
    else if (failure === 'validation') published[seedUrl] = JSON.stringify({ ...seed(), config_sha256: '0'.repeat(64) });
    else
      vi.spyOn(model, 'predict').mockImplementationOnce(() => {
        throw new Error('prediction failed');
      });
    const changed = csv.replace('SF,10,SEA,20', 'SF,30,SEA,20');
    const second = service(store, async () => changed);
    await second.initialize();
    expect(second.getState()).toMatchObject({ status: 'error', error: expect.any(String) });
    expect(second.getModel()).toBeNull();
    expect(() => second.predict('SEA', 'SF', true, 'regular')).toThrow('Predictions are not ready');
    expect(store.getItem(snapshotKey('nfl'))).toBe(saved);
    expect(store.getItem(cacheKey)).toBe(savedGames);
    published[seedUrl] = seedBytes;
    vi.restoreAllMocks();
    const retry = service(store, async () => changed);
    await retry.initialize();
    expect(retry.getState()).toMatchObject({ status: 'ready', cached: false, warning: null });
    expect(readSnapshot(store, 'nfl')).toMatchObject({ app_version: appVersion, file: { games: [{ result: 'away_win' }, {}] } });
  },
);

it.each([true, false])('refreshes a changed baseline with unchanged games; updated seed available: %s', async (available) => {
  const updated = seed();
  updated.ratings.find((t) => t.team === 'SEA')!.elo += 100;
  updated.ratings.find((t) => t.team === 'SF')!.elo -= 100;
  const oldConfig = structuredClone(config);
  oldConfig.elo.initial += 500;
  const oldHash = await digest(JSON.stringify(oldConfig));
  const oldSeed = structuredClone(updated);
  oldSeed.config_sha256 = oldHash;
  oldSeed.settings = oldConfig.elo;
  oldSeed.ratings.forEach((r) => (r.elo += 500));
  const url = `${dataBase}/nfl/elo-2026.json`;
  const { published, requests } = publish({ [url]: JSON.stringify(oldSeed) });
  const store = memoryStore();
  const first = new PredictionService({
    config: oldConfig,
    configHash: oldHash,
    dataBase,
    season: 2026,
    postseasonSimulations: 200,
    now,
    store,
    fetchSource: async () => csv,
  });
  await first.initialize();
  expect(first.getState().status).toBe('ready');
  const saved = store.getItem(snapshotKey('nfl'));
  if (available) published[url] = JSON.stringify(updated);
  else delete published[url];
  requests.length = 0;
  const second = service(store, async () => csv);
  await second.initialize();
  expect(requests.some((r) => r.url === url)).toBe(true);
  expect(second.getState().status).toBe('ready');
  if (!available) {
    expect(second.getState().warning).toContain('initial ratings could not be loaded');
    expect(store.getItem(snapshotKey('nfl'))).toBe(saved);
    expect(second.getState().teams).toEqual(first.getState().teams);
    return;
  }
  expect(second.getState().warning).toBeNull();
  expect(readSnapshot(store, 'nfl')?.model.seed.config_sha256).toBe(configHash);
  for (const team of second.getState().teams) {
    const old = first.getState().teams.find((t) => t.id === team.id)!;
    expect(team.initial_elo).toBeCloseTo(old.initial_elo - 500, 10);
    expect(team.rating).toBeCloseTo(old.rating - 500, 10);
    expect(team.sd).toBeCloseTo(old.sd, 10);
  }
  for (const neutral of [true, false]) {
    for (const phase of ['regular', 'postseason'] as const) {
      const before = first.predict('SEA', 'SF', neutral, phase);
      const after = second.predict('SEA', 'SF', neutral, phase);
      for (const outcome of ['home_win', 'away_win', 'tie'] as const) expect(after[outcome]).toBeCloseTo(before[outcome], 12);
      after.home_probability_interval.forEach((p, i) => expect(p).toBeCloseTo(before.home_probability_interval[i], 12));
    }
  }
});

it.each([true, false])('updates configuration metadata only after a successful rebuild; seed available: %s', async (available) => {
  const { published } = publish();
  const store = memoryStore();
  const first = service(store, async () => csv);
  await first.initialize();
  const snapshot = readSnapshot(store, 'nfl')!;
  const saved = store.getItem(snapshotKey('nfl'));
  const updatedConfig = structuredClone(config);
  updatedConfig.name = 'Updated NFL';
  updatedConfig.history_start += 1;
  updatedConfig.source = { kind: 'canonical-json', url: 'https://updated.test/games.json' };
  const team = updatedConfig.teams.find((t) => t.id === 'SEA')!;
  team.name = 'Updated Seahawks';
  team.eras.at(-1)!.name = team.name;
  const updatedHash = await digest(JSON.stringify(updatedConfig));
  const updatedSeed = seed();
  updatedSeed.config_sha256 = updatedHash;
  const seedUrl = `${dataBase}/nfl/elo-2026.json`;
  if (available) published[seedUrl] = JSON.stringify(updatedSeed);
  else delete published[seedUrl];
  const second = new PredictionService({
    config: updatedConfig,
    configHash: updatedHash,
    dataBase,
    season: 2026,
    postseasonSimulations: 200,
    store,
    now: () => new Date('2026-09-19T19:00:00Z'),
    fetchSource: async () => JSON.stringify({ schema_version: 2, league: 'nfl', games: snapshot.file.games }),
    onProgress: () => expect(second.getState()).toEqual({ ...snapshot.state, cached: true }),
  });
  await second.initialize();
  if (!available) {
    expect(second.getState()).toEqual({
      ...snapshot.state,
      cached: true,
      warning: expect.stringContaining('initial ratings could not be loaded'),
    });
    expect(second.getModel()).toEqual(snapshot.model);
    expect(store.getItem(snapshotKey('nfl'))).toBe(saved);
    return;
  }
  const metadata = {
    league: updatedConfig.name,
    source: updatedConfig.source.url,
    team_history: updatedConfig.teams,
    history_start: updatedConfig.history_start,
    result_policy: 'The provider supplies completed game outcomes.',
  };
  expect(second.getState()).toMatchObject({ ...metadata, status: 'ready', cached: false, warning: null });
  expect(second.getModel()?.config).toEqual(updatedConfig);
  expect(readSnapshot(store, 'nfl')?.state).toMatchObject(metadata);
});

it('refreshes stale configuration metadata when reusing an unchanged model', async () => {
  publish();
  const store = memoryStore();
  const first = service(store, async () => csv);
  await first.initialize();
  const snapshot = readSnapshot(store, 'nfl')!;
  snapshot.state = {
    ...snapshot.state,
    league: 'Old NFL',
    source: 'https://previous.test/games.csv',
    team_history: snapshot.state.team_history.slice(1),
    history_start: 1999,
    result_policy: 'Old result policy',
  };
  store.setItem(snapshotKey('nfl'), JSON.stringify(snapshot));
  const fitted = vi.spyOn(model, 'fitPosterior');
  const second = service(store, async () => csv);
  await second.initialize();
  expect(fitted).not.toHaveBeenCalled();
  expect(second.getState()).toEqual(first.getState());
  expect(readSnapshot(store, 'nfl')?.state).toEqual(first.getState());
});

it('announces changed data while the old predictions are still available, then commits the new snapshot', async () => {
  publish();
  const store = memoryStore();
  const first = service(store, async () => csv);
  await first.initialize();
  const old = first.predict('SEA', 'SF', true, 'regular');
  const progress: string[] = [];
  const second = new PredictionService({
    config,
    configHash,
    dataBase,
    season: 2026,
    postseasonSimulations: 200,
    now,
    store,
    fetchSource: async () => csv.replace('SF,10,SEA,20', 'SF,30,SEA,20'),
    onProgress: (phase) => {
      progress.push(phase);
      if (phase === 'rebuilding') expect(second.predict('SEA', 'SF', true, 'regular')).toEqual(old);
    },
  });
  await second.initialize();
  expect(progress).toEqual(['checking', 'rebuilding']);
  expect(second.predict('SEA', 'SF', true, 'regular').home_win).toBeLessThan(old.home_win);
  expect(readSnapshot(store, 'nfl')?.file.games[0].result).toBe('away_win');
});

it('retains the complete previous model and source when a changed-data build fails', async () => {
  const { published } = publish();
  const store = memoryStore();
  const first = service(store, async () => csv);
  await first.initialize();
  const saved = store.getItem(snapshotKey('nfl'));
  const source = store.getItem(cacheKey);
  delete published[`${dataBase}/nfl/elo-2026.json`];
  const second = service(store, async () => csv.replace('SF,10,SEA,20', 'SF,30,SEA,20'));
  await second.initialize();
  expect(second.getState()).toMatchObject({ status: 'ready', cached: true });
  expect(second.getState().warning).toContain('initial ratings could not be loaded');
  expect(second.predict('SEA', 'SF', true, 'regular')).toEqual(first.predict('SEA', 'SF', true, 'regular'));
  expect(store.getItem(snapshotKey('nfl'))).toBe(saved);
  expect(store.getItem(cacheKey)).toBe(source);
});

it('rebuilds a corrupt saved model from validated games', async () => {
  publish();
  const store = memoryStore();
  await service(store, async () => csv).initialize();
  const saved = JSON.parse(store.getItem(snapshotKey('nfl'))!);
  saved.model.covariance = [];
  store.setItem(snapshotKey('nfl'), JSON.stringify(saved));
  const fitted = vi.spyOn(model, 'fitPosterior');
  const second = service(store, async () => csv);
  await second.initialize();
  expect(second.getState().status).toBe('ready');
  expect(fitted).toHaveBeenCalled();
  expect(readSnapshot(store, 'nfl')).not.toBeNull();
});

it.each([
  ['2026-09-01', '2026-09-02T04:00:00Z'],
  ['2026-12-01', '2026-12-02T05:00:00Z'],
])('rebuilds unchanged games when results from %s become eligible at Eastern midnight', async (date, midnight) => {
  publish();
  const store = memoryStore();
  const source = csv.replace('2026-09-01', date);
  const progress = vi.fn();
  const run = (at: Date) =>
    new PredictionService({
      config,
      configHash,
      dataBase,
      season: 2026,
      postseasonSimulations: 200,
      store,
      now: () => at,
      fetchSource: async () => source,
      onProgress: progress,
    });
  const first = run(new Date(`${date}T23:59:59Z`));
  await first.initialize();
  expect(first.getState()).toMatchObject({ training_games: 0, held_results: 1 });
  const fitted = vi.spyOn(model, 'fitPosterior');
  const beforeMidnight = run(new Date(Date.parse(midnight) - 1));
  await beforeMidnight.initialize();
  expect(beforeMidnight.getState()).toMatchObject({ training_games: 0, held_results: 1 });
  expect(fitted).not.toHaveBeenCalled();
  progress.mockClear();
  const second = run(new Date(midnight));
  await second.initialize();
  expect(second.getState()).toMatchObject({ training_games: 1, held_results: 0 });
  expect(second.getState().games.find((g) => g.id === 'g1')).toMatchObject({ status: 'completed', result: 'home_win' });
  expect(second.predict('SEA', 'SF', true, 'regular').home_win).toBeGreaterThan(
    first.predict('SEA', 'SF', true, 'regular').home_win,
  );
  expect(fitted).toHaveBeenCalled();
  expect(progress.mock.calls).toEqual([['checking'], ['rebuilding']]);
  expect(readSnapshot(store, 'nfl')?.state.training_games).toBe(1);

  fitted.mockClear();
  progress.mockClear();
  const nextDay = run(new Date(Date.parse(midnight) + 24 * 60 * 60 * 1000));
  await nextDay.initialize();
  expect(nextDay.getModel()).toEqual(second.getModel());
  expect(fitted).not.toHaveBeenCalled();
  expect(progress.mock.calls).toEqual([['checking']]);
});

it('refuses stale seeds and future leakage while still refreshing the current cache', async () => {
  const stale = seed();
  stale.target_season = 2025;
  publish({ [`${dataBase}/nfl/elo-2026.json`]: JSON.stringify(stale) });
  const store = memoryStore();
  const s = service(store, async () => csv);
  await s.initialize();
  expect(s.getState().status).toBe('error');
  expect(s.getState().error).toContain('wrong league or season');
  expect(JSON.parse(store.getItem(cacheKey)!).games).toHaveLength(2);
});

it('reports missing published ratings instead of inventing predictions', async () => {
  vi.stubGlobal('fetch', () => Promise.resolve(new Response('missing', { status: 404 })));
  const s = service(memoryStore(), async () => csv);
  await s.initialize();
  expect(s.getState().status).toBe('error');
  expect(s.getState().error).toContain('initial ratings could not be loaded');
});

it('applies a corrected result by rebuilding, and rejects a truncated refresh', async () => {
  publish();
  const store = memoryStore();
  const run = async (text: string) => {
    const s = service(store, async () => text);
    await s.initialize();
    return s;
  };
  const first = await run(csv),
    corrected = await run(csv.replace('SF,10,SEA,20', 'SF,30,SEA,20'));
  expect(corrected.getState().cached).toBe(false);
  expect(corrected.predict('SEA', 'SF', true, 'regular').home_win).toBeLessThan(
    first.predict('SEA', 'SF', true, 'regular').home_win,
  );
  const truncated = await run(
    csv
      .split('\n')
      .filter((line) => !line.startsWith('g1,'))
      .join('\n'),
  );
  expect(truncated.getState().cached).toBe(true);
  expect(truncated.getState().warning).toContain('lost a previously completed');
});

it('preserves pregame predictions when the game itself, same-day games, or later results change', async () => {
  publish();
  const rows = [
    csv.split('\n')[0],
    'first,2026,REG,1,2026-09-01,13:00,SF,10,SEA,20,Home',
    'prior,2026,REG,2,2026-09-08,13:00,SF,10,SEA,20,Home',
    'early,2026,REG,3,2026-09-15,13:00,SF,30,SEA,10,Home',
    'target,2026,REG,3,2026-09-15,20:20,SF,30,SEA,10,Neutral',
    'simultaneous,2026,REG,3,2026-09-15,20:20,KC,30,SEA,10,Home',
    'late,2026,REG,3,2026-09-15,23:00,SF,30,SEA,10,Home',
    'later,2026,REG,3,2026-09-16,13:00,SF,30,SEA,10,Home',
    'awaiting,2026,REG,3,2026-09-19,13:00,SF,,SEA,,Home',
    'upcoming,2026,REG,4,2026-09-20,13:00,SF,,SEA,,Home',
  ];
  const source = rows.join('\n');
  const run = async (text: string, at = now()) => {
    const s = new PredictionService({
      config,
      configHash,
      dataBase,
      season: 2026,
      postseasonSimulations: 200,
      now: () => at,
      store: memoryStore(),
      fetchSource: async () => text,
    });
    await s.initialize();
    expect(s.getState().status).toBe('ready');
    return s;
  };
  const atKickoff = await run(source, new Date('2026-09-16T00:20:00Z'));
  const after = await run(source);
  const target = after.getState().games.find((g) => g.id === 'target')!;
  expect(target.status).toBe('completed');
  expect(target.prediction).toEqual(atKickoff.predict('SEA', 'SF', true, 'regular'));
  expect(after.getState().games[0].prediction).toEqual(predict(fitPosterior(seed(), [], config), 'SEA', 'SF', false, 'regular'));

  const corrected = await run(source.replaceAll(',30,SEA,10', ',10,SEA,30'));
  expect(corrected.getState().games.find((g) => g.id === 'target')!.result).toBe('home_win');
  expect(corrected.getState().games.find((g) => g.id === 'target')!.prediction).toEqual(target.prediction);
  expect(corrected.predict('SEA', 'SF', true, 'regular')).not.toEqual(after.predict('SEA', 'SF', true, 'regular'));
  const changedPrior = await run(
    source.replace('prior,2026,REG,2,2026-09-08,13:00,SF,10,SEA,20', 'prior,2026,REG,2,2026-09-08,13:00,SF,20,SEA,10'),
  );
  expect(changedPrior.getState().games.find((g) => g.id === 'target')!.prediction!.home_win).toBeLessThan(
    target.prediction!.home_win,
  );

  for (const id of ['awaiting', 'upcoming']) {
    expect(after.getState().games.find((g) => g.id === id)!.prediction).toEqual(after.predict('SEA', 'SF', false, 'regular'));
  }
});

it('uses earlier UTC dates for canonical results across timezones and respects postseason rules', async () => {
  publish();
  const prior = game({ id: 'prior', date: '2026-09-01', time: '12:00', timezone: 'UTC' });
  const s = new PredictionService({
    config: { ...config, source: { ...config.source, kind: 'canonical-json' } },
    configHash,
    dataBase,
    season: 2026,
    postseasonSimulations: 200,
    now,
    store: memoryStore(),
    fetchSource: async () =>
      JSON.stringify({
        schema_version: 2,
        league: 'nfl',
        games: [
          game({ id: 'later', date: '2026-09-02', time: '16:00', timezone: 'America/New_York', result: 'away_win' }),
          game({ id: 'target', date: '2026-09-02', time: '12:00', timezone: 'UTC', phase: 'postseason', neutral: false }),
          game({ id: 'same-utc-day', date: '2026-09-01', time: '23:30', timezone: 'America/Los_Angeles', result: 'away_win' }),
          prior,
        ],
      }),
  });
  await s.initialize();
  expect(s.getState().status).toBe('ready');
  const prediction = s.getState().games.find((g) => g.id === 'target')!.prediction;
  expect(prediction).toEqual(predict(fitPosterior(seed(), [prior], config), 'SEA', 'SF', false, 'postseason'));
  expect(prediction!.tie).toBe(0);
});

it('publishes ready postseason odds in the state and the snapshot', async () => {
  publish();
  const store = memoryStore();
  const s = service(store, async () => csv);
  await s.initialize();
  expect(s.getState()).toMatchObject({ status: 'ready', postseason: { status: 'ready', mode: 'regular', simulations: 200 } });
  expect(readSnapshot(store, 'nfl')?.state.postseason?.status).toBe('ready');
  expect(readSnapshot(store, 'nfl')?.state.postseason).toEqual(s.getState().postseason);
});

it('reuses saved postseason odds without simulating for unchanged data', async () => {
  publish();
  const store = memoryStore();
  const first = service(store, async () => csv);
  await first.initialize();
  const simulated = vi.spyOn(postseason, 'simulatePostseason');
  const second = service(store, async () => csv);
  await second.initialize();
  expect(simulated).not.toHaveBeenCalled();
  expect(second.getState().postseason?.status).toBe('ready');
  expect(second.getState().postseason).toEqual(first.getState().postseason);
  await service(store, async () => csv.replace('SF,10,SEA,20', 'SF,30,SEA,20')).initialize();
  expect(simulated).toHaveBeenCalledOnce();
});

it('reports a failed postseason simulation without blocking predictions', async () => {
  publish();
  const store = memoryStore();
  vi.spyOn(postseason, 'simulatePostseason').mockImplementation(() => {
    throw new Error('simulation failed');
  });
  const s = service(store, async () => csv);
  await s.initialize();
  expect(s.getState()).toMatchObject({
    status: 'ready',
    warning: null,
    postseason: { status: 'error', error: 'simulation failed' },
  });
  expect(readSnapshot(store, 'nfl')?.state.postseason).toEqual({ status: 'error', error: 'simulation failed' });
});

it('publishes no postseason odds for a config without a postseason block', async () => {
  publish();
  const store = memoryStore();
  const plain: LeagueConfig = {
    ...config,
    postseason: undefined,
    teams: config.teams.map((t) => ({ ...t, eras: t.eras.map((era) => ({ ...era, division: undefined })) })),
  };
  const simulated = vi.spyOn(postseason, 'simulatePostseason');
  const s = new PredictionService({
    config: plain,
    configHash,
    dataBase,
    season: 2026,
    postseasonSimulations: 200,
    now,
    store,
    fetchSource: async () => csv,
  });
  await s.initialize();
  expect(s.getState()).toMatchObject({ status: 'ready', postseason: null });
  expect(simulated).not.toHaveBeenCalled();
  expect(readSnapshot(store, 'nfl')).not.toBeNull();
  expect(readSnapshot(store, 'nfl')?.state.postseason).toBeNull();
});
