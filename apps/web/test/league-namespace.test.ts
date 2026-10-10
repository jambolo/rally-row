import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { indexedDbPersistence, type Persistence } from '../src/persistence.ts';
import { refresh, type RefreshMessage } from '../src/refresh-worker.ts';
import { PredictionService } from '../src/service.ts';
import type { SessionView } from '../src/session.ts';
import { readSnapshot, snapshotKey } from '../src/snapshot.ts';
import { currentCacheKey, memoryStore, type Store } from '../src/storage.ts';
import { fakeIndexedDb } from './fake-indexeddb.ts';
import { config, configBytes, configHash, game, historyHashFile, seed } from './helpers.ts';

beforeEach(() => {
  vi.resetModules();
  vi.useFakeTimers({ now: new Date('2026-09-19T18:00:00Z') });
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

function fakeWorker() {
  return { onmessage: null, onerror: null, onmessageerror: null, postMessage: vi.fn(), terminate: vi.fn() };
}

async function session(
  league: string,
  store: Store,
  locks: Pick<LockManager, 'request'> | null = null,
  persistence: Persistence | null = null,
) {
  const { startSession } = await import('../src/session.ts');
  const worker = fakeWorker();
  const changes: SessionView[] = [];
  const stop = startSession({
    league,
    configBase: 'https://test/config/',
    dataBase: 'https://test/data',
    store,
    persistence,
    createWorker: () => worker as unknown as Worker,
    locks,
    onChange: (view) => changes.push(view),
  });
  await vi.advanceTimersByTimeAsync(0);
  return { worker, changes, stop };
}

it('builds the same storage keys and lock name as before for nfl', async () => {
  const { attemptKey, lockName } = await import('../src/session.ts');
  expect(snapshotKey('nfl')).toBe('game-results-prediction:nfl:model-v1');
  expect(attemptKey('nfl')).toBe('game-results-prediction:nfl:last-attempt');
  expect(lockName('nfl')).toBe('game-results-prediction:nfl:refresh');
  expect(currentCacheKey('nfl', 2026)).toBe('game-results-prediction:nfl:current-2026');
});

it('ignores a snapshot saved for another league', async () => {
  vi.stubGlobal('fetch', (url: string) =>
    Promise.resolve(new Response(url.endsWith('history.sha256') ? historyHashFile : JSON.stringify(seed()))),
  );
  const store = memoryStore();
  await new PredictionService({
    config: { ...config, source: { ...config.source, kind: 'canonical-json' } },
    configHash,
    dataBase: 'https://test/data',
    season: 2026,
    postseasonSimulations: 200,
    store,
    fetchSource: async () => JSON.stringify({ schema_version: 2, league: 'nfl', games: [game()] }),
  }).initialize();
  expect(readSnapshot(store, 'nfl')).not.toBeNull();
  expect(readSnapshot(store, 'other')).toBeNull();
  store.setItem(snapshotKey('other'), store.getItem(snapshotKey('nfl'))!);
  expect(readSnapshot(store, 'other')).toBeNull();
});

it("hands the worker only the requested league's snapshot and current-season caches", async () => {
  const entries: Record<string, string> = {
    [snapshotKey('nfl')]: 'nfl model',
    [currentCacheKey('nfl', 2025)]: 'nfl 2025 games',
    [currentCacheKey('nfl', 2026)]: 'nfl 2026 games',
    [snapshotKey('other')]: 'other model',
    [currentCacheKey('other', 2026)]: 'other games',
    [currentCacheKey('nfl-x', 2026)]: 'prefixed league games',
    'game-results-prediction:nfl:current-x': 'malformed season',
    'game-results-prediction:other:last-attempt': '1',
    unrelated: 'value',
  };
  const expected = {
    league: 'nfl',
    configUrl: 'https://test/config/nfl.json',
    dataBase: 'https://test/data',
    cache: {
      [snapshotKey('nfl')]: 'nfl model',
      [currentCacheKey('nfl', 2025)]: 'nfl 2025 games',
      [currentCacheKey('nfl', 2026)]: 'nfl 2026 games',
    },
  };
  const persistence = indexedDbPersistence(fakeIndexedDb().factory);
  await persistence.save(entries);
  // Entries an earlier version left in the small store never reach the worker.
  const store = memoryStore();
  store.setItem(snapshotKey('nfl'), 'legacy nfl model');
  store.setItem(currentCacheKey('nfl', 2026), 'legacy nfl games');
  const first = await session('nfl', store, null, persistence);
  expect(first.worker.postMessage).toHaveBeenCalledExactlyOnceWith(expected);
  first.stop();
  // The session filters even when a persistence returns every entry it holds.
  vi.resetModules();
  const everything: Persistence = { load: async () => entries, save: async () => {} };
  const second = await session('nfl', memoryStore(), null, everything);
  expect(second.worker.postMessage).toHaveBeenCalledExactlyOnceWith(expected);
  second.stop();
});

it('locks and cools down each league independently', async () => {
  const names: string[] = [];
  const locks = {
    request: vi.fn(async (name: string, _options: LockOptions, callback: () => Promise<void>) => {
      names.push(name);
      await callback();
    }),
  } as unknown as Pick<LockManager, 'request'>;
  const store = memoryStore();
  const nfl = await session('nfl', store, locks);
  expect(nfl.worker.postMessage).toHaveBeenCalledOnce();
  const other = await session('other', store, locks);
  expect(other.worker.postMessage).toHaveBeenCalledOnce();
  expect(names).toEqual(['game-results-prediction:nfl:refresh', 'game-results-prediction:other:refresh']);
  expect(store.getItem('game-results-prediction:other:last-attempt')).toBe(String(Date.now()));
  const again = await session('nfl', store, locks);
  expect(again.worker.postMessage).not.toHaveBeenCalled();
  expect(again.changes.at(-1)?.phase).toBe('waiting');
  for (const s of [nfl, other, again]) s.stop();
});

it('refresh reports an error when the configuration id differs from the requested league', async () => {
  const configUrl = 'https://test/config/other.json';
  vi.stubGlobal('fetch', async (url: string) =>
    url === configUrl ? new Response(configBytes) : new Response('missing', { status: 404 }),
  );
  const messages: RefreshMessage[] = [];
  await refresh({ league: 'other', configUrl, dataBase: 'https://test/data', cache: {} }, (message) => messages.push(message));
  expect(messages).toEqual([{ type: 'error', error: 'Configuration id nfl does not match requested league other' }]);
});
