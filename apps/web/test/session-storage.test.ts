import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { indexedDbPersistence, type Persistence } from '../src/persistence.ts';
import * as postseason from '../src/postseason.ts';
import { refresh, type RefreshMessage, type RefreshRequest } from '../src/refresh-worker.ts';
import type { SessionView } from '../src/session.ts';
import { readSnapshot, snapshotKey } from '../src/snapshot.ts';
import { currentCacheKey, memoryStore } from '../src/storage.ts';
import { fakeIndexedDb } from './fake-indexeddb.ts';
import { config, configBytes, historyHashFile, seed } from './helpers.ts';

// refresh() always runs the production simulation count; these multi-refresh tests only need the odds to exist.
const simulatePostseason = postseason.simulatePostseason;
vi.spyOn(postseason, 'simulatePostseason').mockImplementation((model, games) =>
  simulatePostseason(model, games, { simulations: 200 }),
);

const dataBase = 'https://test/data';
const csv =
  'game_id,season,game_type,week,gameday,gametime,away_team,away_score,home_team,home_score,location\ng1,2026,REG,1,2026-09-01,13:00,SF,10,SEA,20,Home\ng2,2026,REG,2,2026-09-20,13:00,KC,,SEA,,Home\n';
const stops: (() => void)[] = [];

beforeEach(() => {
  vi.resetModules();
  vi.useFakeTimers({ now: new Date('2026-09-19T18:00:00Z') });
  const files = new Map([
    ['https://test/config/nfl.json', configBytes.toString()],
    [`${dataBase}/nfl/elo-2026.json`, JSON.stringify(seed())],
    [`${dataBase}/nfl/history.sha256`, historyHashFile],
    [config.source.url, csv],
  ]);
  vi.stubGlobal('fetch', async (url: string) => {
    const body = files.get(url);
    return body === undefined ? new Response('missing', { status: 404 }) : new Response(body);
  });
});
afterEach(() => {
  for (const stop of stops.splice(0)) stop();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

/** A Store holding saved entries, for readSnapshot. */
function storeOf(entries: Record<string, string>) {
  const store = memoryStore();
  for (const [key, value] of Object.entries(entries)) store.setItem(key, value);
  return store;
}

/** The entries a completed NFL refresh returns. */
async function builtEntries() {
  let cache: Record<string, string> = {};
  await refresh({ league: 'nfl', configUrl: 'https://test/config/nfl.json', dataBase, cache: {} }, (message) => {
    if (message.type === 'complete') cache = message.cache;
  });
  expect(Object.keys(cache).sort()).toEqual([currentCacheKey('nfl', 2026), snapshotKey('nfl')]);
  return cache;
}

/** Starts an NFL session whose worker runs `refresh` in-process and adds `extraCache` to a completed result;
 * `persistence: undefined` uses the default.
 */
async function run(options: {
  persistence?: Persistence | null | undefined;
  locks?: Pick<LockManager, 'request'> | null | undefined;
  extraCache?: Record<string, string> | undefined;
}) {
  const store = memoryStore();
  const changes: SessionView[] = [];
  const requests: RefreshRequest[] = [];
  const jobs: Promise<void>[] = [];
  const createWorker = vi.fn(() => {
    const worker = {
      onmessage: null as ((event: MessageEvent<RefreshMessage>) => void) | null,
      terminate: vi.fn(),
      postMessage(request: RefreshRequest) {
        requests.push(structuredClone(request));
        jobs.push(
          refresh(structuredClone(request), (message) => {
            const data = message.type === 'complete' ? { ...message, cache: { ...message.cache, ...options.extraCache } } : message;
            worker.onmessage?.({ data: structuredClone(data) } as MessageEvent<RefreshMessage>);
          }),
        );
      },
    };
    return worker as unknown as Worker;
  });
  const { startSession } = await import('../src/session.ts');
  stops.push(
    startSession({
      league: 'nfl',
      configBase: 'https://test/config',
      dataBase,
      store,
      ...(options.persistence === undefined ? {} : { persistence: options.persistence }),
      createWorker,
      locks: options.locks ?? null,
      onChange: (view) => changes.push(view),
    }),
  );
  const settle = async () => {
    await vi.advanceTimersByTimeAsync(0);
    await Promise.all(jobs);
    await vi.advanceTimersByTimeAsync(0);
  };
  return { store, changes, requests, createWorker, settle };
}

it('after a full session run the small store holds no snapshot or cache key and persistence holds the snapshot', async () => {
  const idb = fakeIndexedDb();
  const persistence = indexedDbPersistence(idb.factory);
  const { store, changes, settle } = await run({ persistence });
  await settle();
  expect(changes.at(-1)).toMatchObject({ phase: 'idle', error: '', state: { status: 'ready', training_games: 1 } });
  expect(store.keys()).toEqual(['game-results-prediction:nfl:last-attempt']);
  const saved = await persistence.load('nfl');
  expect(Object.keys(saved).sort()).toEqual([currentCacheKey('nfl', 2026), snapshotKey('nfl')]);
  expect(Object.keys(idb.contents()).sort()).toEqual([currentCacheKey('nfl', 2026), snapshotKey('nfl')]);
  expect(readSnapshot(storeOf(saved), 'nfl')?.model).toEqual(changes.at(-1)?.model);
});

it('a failing persistence save still shows the fitted model and releases the lock', async () => {
  const idb = fakeIndexedDb({ writesFail: true });
  const persistence = indexedDbPersistence(idb.factory);
  const held: Promise<void>[] = [];
  const locks = {
    request: vi.fn((_name: string, _options: LockOptions, callback: () => Promise<void>) => {
      const done = callback();
      held.push(done);
      return done;
    }),
  } as unknown as Pick<LockManager, 'request'>;
  const { changes, settle } = await run({ persistence, locks });
  await settle();
  expect(held).toHaveLength(1);
  await expect(held[0]).resolves.toBeUndefined();
  expect(changes.at(-1)).toMatchObject({ phase: 'idle', error: '', state: { status: 'ready', training_games: 1 } });
  expect(changes.at(-1)?.model).not.toBeNull();
  expect(idb.contents()).toEqual({});
});

it.each([
  ['no persistence', () => null],
  ['the default without IndexedDB', () => undefined],
  ['IndexedDB that fails to open', () => indexedDbPersistence(fakeIndexedDb({ openFails: true }).factory)],
])('persistence unavailable (%s): the session still shows results', async (_label, persistence) => {
  const { changes, requests, settle } = await run({ persistence: persistence() });
  await settle();
  expect(requests).toHaveLength(1);
  expect(requests[0].cache).toEqual({});
  expect(changes.at(-1)).toMatchObject({ phase: 'idle', error: '', state: { status: 'ready', training_games: 1 } });
  expect(changes.at(-1)?.model).not.toBeNull();
});

it('holds the refresh lock until the save completes', async () => {
  const idb = fakeIndexedDb();
  const inner = indexedDbPersistence(idb.factory);
  const gate = Promise.withResolvers<void>();
  const persistence: Persistence = {
    load: (league) => inner.load(league),
    save: async (entries) => {
      await gate.promise;
      await inner.save(entries);
    },
  };
  let released = false;
  const locks = {
    request: vi.fn(async (_name: string, _options: LockOptions, callback: () => Promise<void>) => {
      await callback();
      released = true;
    }),
  } as unknown as Pick<LockManager, 'request'>;
  const { changes, settle } = await run({ persistence, locks });
  await settle();
  // The result shows at once; the lock waits for the save.
  expect(changes.at(-1)).toMatchObject({ phase: 'idle', state: { status: 'ready' } });
  expect(released).toBe(false);
  expect(idb.contents()).toEqual({});
  gate.resolve();
  await vi.advanceTimersByTimeAsync(0);
  expect(released).toBe(true);
  expect(Object.keys(await inner.load('nfl')).sort()).toEqual([currentCacheKey('nfl', 2026), snapshotKey('nfl')]);
});

it('restores the saved snapshot before creating the worker even when loading is slow', async () => {
  const entries = await builtEntries();
  const inner = indexedDbPersistence(fakeIndexedDb().factory);
  await inner.save(entries);
  // Only the first load (the startup restore) is slow.
  const gate = Promise.withResolvers<void>();
  let loads = 0;
  const persistence: Persistence = {
    load: async (league) => {
      if (loads++ === 0) await gate.promise;
      return inner.load(league);
    },
    save: (saved) => inner.save(saved),
  };
  const { changes, createWorker, settle } = await run({ persistence });
  await vi.advanceTimersByTimeAsync(0);
  expect(createWorker).not.toHaveBeenCalled();
  expect(changes).toEqual([]);
  const modelsAtWorkerStart: unknown[] = [];
  const create = createWorker.getMockImplementation()!;
  createWorker.mockImplementation(() => {
    modelsAtWorkerStart.push(changes.at(-1)?.model ?? null);
    return create();
  });
  gate.resolve();
  await settle();
  const snapshot = readSnapshot(storeOf(entries), 'nfl')!;
  expect(changes[0]).toMatchObject({ model: snapshot.model, state: { ...snapshot.state, cached: true } });
  expect(createWorker).toHaveBeenCalledOnce();
  expect(modelsAtWorkerStart).toEqual([snapshot.model]);
});

it("never reads, writes, or deletes another league's entries", async () => {
  const other = {
    [snapshotKey('other')]: 'other model',
    [currentCacheKey('other', 2026)]: 'other games',
    [currentCacheKey('nfl-x', 2026)]: 'prefixed league games',
  };
  const idb = fakeIndexedDb();
  const persistence = indexedDbPersistence(idb.factory);
  await persistence.save(other);
  const { requests, changes, settle } = await run({
    persistence,
    extraCache: { [snapshotKey('other')]: 'overwritten', 'game-results-prediction:other:last-attempt': '1' },
  });
  await settle();
  expect(requests[0].cache).toEqual({});
  expect(changes.at(-1)).toMatchObject({ phase: 'idle', state: { status: 'ready' } });
  expect(idb.contents()).toMatchObject(other);
  expect(Object.keys(idb.contents()).sort()).toEqual(
    [
      currentCacheKey('nfl', 2026),
      currentCacheKey('nfl-x', 2026),
      snapshotKey('nfl'),
      currentCacheKey('other', 2026),
      snapshotKey('other'),
    ].sort(),
  );
});
