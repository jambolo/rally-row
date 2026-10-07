import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { LeagueConfig } from '../src/contracts.ts';
import { indexedDbPersistence } from '../src/persistence.ts';
import * as postseason from '../src/postseason.ts';
import { refresh, type RefreshMessage, type RefreshRequest } from '../src/refresh-worker.ts';
import type { SessionView } from '../src/session.ts';
import { readSnapshot, snapshotKey } from '../src/snapshot.ts';
import { digest, memoryStore } from '../src/storage.ts';
import { fakeIndexedDb } from './fake-indexeddb.ts';
import { config, configBytes, historyBytes, seed } from './helpers.ts';

// refresh() always runs the production simulation count; these multi-refresh tests only need the odds to exist.
const simulatePostseason = postseason.simulatePostseason;
vi.spyOn(postseason, 'simulatePostseason').mockImplementation((model, games) =>
  simulatePostseason(model, games, { simulations: 200 }),
);

const stops: (() => void)[] = [];
const csv =
  'game_id,season,game_type,week,gameday,gametime,away_team,away_score,home_team,home_score,location\ng1,2026,REG,1,2026-09-01,13:00,SF,10,SEA,20,Home\ng2,2026,REG,2,2026-09-20,13:00,KC,,SEA,,Home\n';

beforeEach(() => {
  vi.resetModules();
  vi.useFakeTimers({ now: new Date('2026-09-19T18:00:00Z') });
});

afterEach(() => {
  for (const stop of stops.splice(0)) stop();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

async function setup() {
  const configUrl = 'https://test/config/nfl.json';
  const dataBase = 'https://test/data';
  const seedUrl = `${dataBase}/nfl/elo-2026.json`;
  const files = new Map([
    [configUrl, configBytes.toString()],
    [seedUrl, JSON.stringify(seed())],
    [`${dataBase}/nfl/history.json`, historyBytes],
    [config.source.url, csv],
  ]);
  const fetcher = vi.fn<(url: string, init?: RequestInit) => Promise<Response>>(async (url) => {
    const body = files.get(url);
    return body === undefined ? new Response('missing', { status: 404 }) : new Response(body);
  });
  vi.stubGlobal('fetch', fetcher);
  // The small store holds only the refresh cooldown; snapshots and caches persist in (fake) IndexedDB.
  const store = memoryStore();
  const persistence = indexedDbPersistence(fakeIndexedDb().factory);
  let built: Record<string, string> = {};
  await refresh({ league: 'nfl', configUrl, dataBase, cache: {} }, (message) => {
    if (message.type === 'complete') built = message.cache;
  });
  await persistence.save(built);
  const entry = async (key: string) => (await persistence.load('nfl'))[key] ?? null;
  const saved = async () => {
    const entries = memoryStore();
    for (const [key, value] of Object.entries(await persistence.load('nfl'))) entries.setItem(key, value);
    return readSnapshot(entries, 'nfl');
  };
  const snapshot = (await saved())!;
  expect(snapshot).not.toBeNull();
  const changes: SessionView[] = [];
  const messages: RefreshMessage[] = [];
  const jobs: Promise<void>[] = [];
  const createWorker = vi.fn(() => {
    const worker = {
      onmessage: null as ((event: MessageEvent<RefreshMessage>) => void) | null,
      terminate: vi.fn(),
      postMessage(request: RefreshRequest) {
        jobs.push(
          refresh(structuredClone(request), (message) => {
            const delivered = structuredClone(message);
            messages.push(delivered);
            worker.onmessage?.({ data: delivered } as MessageEvent<RefreshMessage>);
          }),
        );
      },
    };
    return worker as unknown as Worker;
  });
  const { startSession, cooldownMs } = await import('../src/session.ts');
  fetcher.mockClear();
  return {
    configUrl,
    seedUrl,
    files,
    fetcher,
    store,
    persistence,
    entry,
    saved,
    snapshot,
    changes,
    messages,
    createWorker,
    cooldownMs,
    settle: () => Promise.all(jobs),
    start() {
      const stop = startSession({
        league: 'nfl',
        configBase: 'https://test/config',
        dataBase,
        store,
        persistence,
        createWorker,
        locks: null,
        onChange: (view) => changes.push(view),
      });
      stops.push(stop);
      return stop;
    },
    async publishConfig(updated: LeagueConfig) {
      const bytes = JSON.stringify(updated);
      const hash = await digest(bytes);
      const updatedSeed = { ...seed(), config_sha256: hash, settings: updated.elo };
      files.set(configUrl, bytes);
      files.set(seedUrl, JSON.stringify(updatedSeed));
      files.set(
        updated.source.url,
        updated.source.kind === 'nflverse-csv'
          ? csv
          : JSON.stringify({ schema_version: 2, league: updated.id, games: snapshot.file.games }),
      );
      return { hash, updatedSeed };
    },
  };
}

it.each(['Bayesian settings', 'source and metadata'])(
  'publishes a configuration transition through the worker and session: %s',
  async (change) => {
    const f = await setup();
    const updated = structuredClone(config);
    if (change === 'Bayesian settings') updated.bayesian.prior_sd_elo *= 2;
    else {
      updated.name = 'Updated NFL';
      updated.history_start += 1;
      updated.source = { kind: 'canonical-json', url: 'https://updated.test/games.json' };
      const team = updated.teams.find((t) => t.id === 'SEA')!;
      team.name = 'Updated Seahawks';
      team.eras.at(-1)!.name = team.name;
    }
    const { hash } = await f.publishConfig(updated);
    const seedRequested = Promise.withResolvers<void>();
    const releaseSeed = Promise.withResolvers<void>();
    const fetchPublished = f.fetcher.getMockImplementation()!;
    f.fetcher.mockImplementation(async (url, init) => {
      if (url === f.seedUrl) {
        seedRequested.resolve();
        await releaseSeed.promise;
      }
      return fetchPublished(url, init);
    });
    const saved = await f.entry(snapshotKey('nfl'));
    const stop = f.start();
    await vi.advanceTimersByTimeAsync(0);
    await seedRequested.promise;
    expect(f.changes[0]?.state).toEqual({ ...f.snapshot.state, cached: true });
    expect(f.changes.at(-1)).toMatchObject({
      phase: 'rebuilding',
      model: f.snapshot.model,
      state: { ...f.snapshot.state, cached: true },
    });
    expect(await f.entry(snapshotKey('nfl'))).toBe(saved);
    releaseSeed.resolve();
    await f.settle();
    const next = (await f.saved())!;
    expect(next.file.games).toEqual(f.snapshot.file.games);
    expect(next.model.config).toEqual(updated);
    expect(next.model.seed.config_sha256).toBe(hash);
    expect(next.state).toMatchObject({
      status: 'ready',
      cached: false,
      warning: null,
      league: updated.name,
      source: updated.source.url,
      team_history: updated.teams,
      history_start: updated.history_start,
      result_policy:
        updated.source.kind === 'canonical-json'
          ? 'The provider supplies completed game outcomes.'
          : f.snapshot.state.result_policy,
    });
    expect(f.changes.at(-1)).toMatchObject({ phase: 'idle', state: next.state, model: next.model, error: '' });
    expect(f.fetcher).toHaveBeenCalledWith(f.configUrl, expect.objectContaining({ cache: 'no-store' }));
    expect(f.fetcher).toHaveBeenCalledWith(updated.source.url, expect.objectContaining({ cache: 'no-cache' }));
    if (change === 'Bayesian settings') {
      expect(next.model.covariance).not.toEqual(f.snapshot.model.covariance);
      expect(next.state.games[1].prediction).not.toEqual(f.snapshot.state.games[1].prediction);
    }
    stop();
    f.fetcher.mockClear();
    f.messages.length = 0;
    f.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(f.changes.at(-1)?.phase).toBe('waiting');
    expect(f.createWorker).toHaveBeenCalledOnce();
    await vi.advanceTimersByTimeAsync(f.cooldownMs);
    await f.settle();
    expect(f.createWorker).toHaveBeenCalledTimes(2);
    expect(f.messages.filter((message) => message.type === 'progress')).toEqual([{ type: 'progress', phase: 'checking' }]);
    expect(f.fetcher).toHaveBeenCalledWith(f.seedUrl, expect.objectContaining({ cache: 'no-cache' }));
    expect(f.fetcher).toHaveBeenCalledWith('https://test/data/nfl/history.json', expect.objectContaining({ cache: 'no-cache' }));
    expect((await f.saved())?.model).toEqual(next.model);
    expect(f.changes.at(-1)?.state).toMatchObject({ ...next.state, checked_at: new Date().toISOString() });
  },
);

it.each([
  ['missing seed', 'Not published'],
  ['mismatched seed', 'Configuration changed since Elo was calculated'],
  ['invalid configuration', 'Too small'],
  ['failed fit', 'Elo seed does not match the configured league and teams'],
])('preserves the snapshot after %s during a configuration transition and recovers on reload', async (failure, warning) => {
  const f = await setup();
  const updated = structuredClone(config);
  updated.name = 'Updated NFL';
  updated.bayesian.prior_sd_elo *= 2;
  const { hash, updatedSeed } = await f.publishConfig(updated);
  if (failure === 'missing seed') f.files.delete(f.seedUrl);
  else if (failure === 'mismatched seed') f.files.set(f.seedUrl, JSON.stringify(seed()));
  else if (failure === 'invalid configuration')
    f.files.set(f.configUrl, JSON.stringify({ ...updated, bayesian: { ...updated.bayesian, prior_sd_elo: 0 } }));
  else f.files.set(f.seedUrl, JSON.stringify({ ...updatedSeed, ratings: updatedSeed.ratings.slice(1) }));
  const saved = await f.entry(snapshotKey('nfl'));
  const savedGames = await f.entry('game-results-prediction:nfl:current-2026');
  expect(savedGames).not.toBeNull();
  await vi.advanceTimersByTimeAsync(1_000);
  const stop = f.start();
  await vi.advanceTimersByTimeAsync(0);
  await f.settle();
  expect(f.changes.at(-1)).toMatchObject({
    phase: 'idle',
    model: f.snapshot.model,
    state: { ...f.snapshot.state, cached: true, warning: expect.stringContaining(warning) },
  });
  expect(await f.entry(snapshotKey('nfl'))).toBe(saved);
  expect(await f.entry('game-results-prediction:nfl:current-2026')).toBe(savedGames);
  expect(f.messages.at(-1)?.type).toBe(failure === 'invalid configuration' ? 'error' : 'complete');
  stop();
  await f.publishConfig(updated);
  f.start();
  await vi.advanceTimersByTimeAsync(f.cooldownMs);
  await f.settle();
  const next = (await f.saved())!;
  expect(next.model.config).toEqual(updated);
  expect(next.model.seed.config_sha256).toBe(hash);
  expect(next.file.games).toEqual(f.snapshot.file.games);
  expect(f.changes.at(-1)).toMatchObject({
    phase: 'idle',
    model: next.model,
    state: { ...next.state, league: updated.name, warning: null, cached: false, checked_at: new Date().toISOString() },
  });
});
