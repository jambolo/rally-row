import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { Posterior } from '../src/model.ts';
import { indexedDbPersistence } from '../src/persistence.ts';
import { PredictionService } from '../src/service.ts';
import { memoryStore } from '../src/storage.ts';
import { readSnapshot, snapshotKey } from '../src/snapshot.ts';
import type { SessionView } from '../src/session.ts';
import type { RefreshMessage } from '../src/refresh-worker.ts';
import { fakeIndexedDb } from './fake-indexeddb.ts';
import { config, configHash, game, historyBytes, seed } from './helpers.ts';

class BackgroundWorker {
  onmessage: ((event: MessageEvent<RefreshMessage>) => void) | null = null;
  onerror: ((event: ErrorEvent) => void) | null = null;
  postMessage = vi.fn();
  terminate = vi.fn();
  send(data: RefreshMessage) {
    this.onmessage?.({ data } as MessageEvent<RefreshMessage>);
  }
}

beforeEach(() => {
  vi.resetModules();
  vi.useFakeTimers({ now: new Date('2026-09-19T18:00:00Z') });
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

/** A Store holding saved entries, for readSnapshot. */
function storeOf(entries: Record<string, string>) {
  const store = memoryStore();
  for (const [key, value] of Object.entries(entries)) store.setItem(key, value);
  return store;
}

/** A session whose persistence holds the built current-season cache and, unless `withSnapshot` is false, snapshot. */
async function setup({ withSnapshot = true } = {}) {
  const built = memoryStore();
  vi.stubGlobal('fetch', (url: string) =>
    Promise.resolve(new Response(url.endsWith('history.json') ? historyBytes : JSON.stringify(seed()))),
  );
  const service = new PredictionService({
    config: { ...config, source: { ...config.source, kind: 'canonical-json' } },
    configHash,
    dataBase: 'https://test/data',
    season: 2026,
    postseasonSimulations: 200,
    store: built,
    fetchSource: async () => JSON.stringify({ schema_version: 2, league: 'nfl', games: [game()] }),
  });
  await service.initialize();
  const snapshot = readSnapshot(built, 'nfl')!;
  expect(snapshot).not.toBeNull();
  const persistence = indexedDbPersistence(fakeIndexedDb().factory);
  const keys = built.keys().filter((key) => withSnapshot || key !== snapshotKey('nfl'));
  await persistence.save(Object.fromEntries(keys.map((key) => [key, built.getItem(key)!])));
  // The small store holds only the refresh cooldown.
  const store = memoryStore();
  const worker = new BackgroundWorker();
  const changes: SessionView[] = [];
  const modelsAtWorkerStart: (Posterior | null)[] = [];
  const createWorker = vi.fn(() => {
    modelsAtWorkerStart.push(changes.at(-1)?.model ?? null);
    return worker as unknown as Worker;
  });
  const options = {
    league: 'nfl',
    configBase: 'https://test/config',
    dataBase: 'https://test/data',
    store,
    persistence,
    createWorker,
    locks: null,
    onChange: (view: SessionView) => changes.push(view),
  };
  const session = await import('../src/session.ts');
  const saved = () => persistence.load('nfl');
  return { ...session, options, store, persistence, saved, snapshot, worker, changes, createWorker, modelsAtWorkerStart };
}

it('shows the cached model before starting a worker and keeps it visible during rebuilding', async () => {
  const { startSession, options, snapshot, worker, createWorker, changes, store, attemptKey, modelsAtWorkerStart } = await setup();
  const stop = startSession(options);
  expect(createWorker).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(0);
  expect(changes[0]).toMatchObject({ model: snapshot.model, state: { ...snapshot.state, cached: true } });
  expect(createWorker).toHaveBeenCalledOnce();
  expect(modelsAtWorkerStart).toEqual([snapshot.model]);
  expect(Number(store.getItem(attemptKey('nfl')))).toBe(Date.now());
  worker.send({ type: 'progress', phase: 'rebuilding' });
  expect(changes.at(-1)).toMatchObject({ phase: 'rebuilding', model: snapshot.model, state: { status: 'ready' } });
  worker.send({ type: 'complete', state: { ...snapshot.state, cached: false }, model: snapshot.model, cache: {} });
  expect(changes.at(-1)?.phase).toBe('idle');
  expect(worker.terminate).toHaveBeenCalledOnce();
  stop();
});

it.each([undefined, '0.0.0', '999.0.0', 42])(
  'does not expose a snapshot with app version %s on restoration or worker failure',
  async (version) => {
    const f = await setup();
    const saved = JSON.stringify({ ...f.snapshot, app_version: version });
    await f.persistence.save({ [snapshotKey('nfl')]: saved });
    const stop = f.startSession(f.options);
    expect(f.changes.some((view) => view.model !== null)).toBe(false);
    await vi.advanceTimersByTimeAsync(0);
    expect(f.changes.some((view) => view.model !== null)).toBe(false);
    expect(f.changes.at(-1)).toMatchObject({ model: null, state: null, phase: 'checking' });
    f.worker.send({ type: 'error', error: 'offline' });
    expect(f.changes.at(-1)).toMatchObject({ model: null, state: null, phase: 'idle', error: 'offline' });
    expect((await f.saved())[snapshotKey('nfl')]).toBe(saved);
    stop();
  },
);

it.each([true, false])(
  'checks the version of a snapshot published while waiting for the lock; compatible: %s',
  async (compatible) => {
    const f = await setup({ withSnapshot: false });
    const gate = Promise.withResolvers<void>();
    const locks = {
      request: vi.fn(async (_name: string, _options: LockOptions, callback: () => Promise<void>) => {
        await gate.promise;
        await callback();
      }),
    } as unknown as Pick<LockManager, 'request'>;
    const stop = f.startSession({ ...f.options, locks });
    await vi.advanceTimersByTimeAsync(0);
    expect(f.createWorker).not.toHaveBeenCalled();
    expect(f.changes.some((view) => view.model !== null)).toBe(false);
    await f.persistence.save({
      [snapshotKey('nfl')]: JSON.stringify({ ...f.snapshot, ...(compatible ? {} : { app_version: '0.0.0' }) }),
    });
    gate.resolve();
    await vi.advanceTimersByTimeAsync(0);
    expect(f.createWorker).toHaveBeenCalledOnce();
    expect(f.changes.at(-1)?.model).toEqual(compatible ? f.snapshot.model : null);
    f.worker.send({ type: 'error', error: 'offline' });
    stop();
  },
);

it('defers work across a reload that interrupted the previous worker, then retries at the deadline', async () => {
  const { startSession, options, worker, createWorker, changes, cooldownMs } = await setup();
  const stop = startSession(options);
  await vi.advanceTimersByTimeAsync(0);
  stop();
  expect(worker.terminate).toHaveBeenCalledOnce();
  await vi.advanceTimersByTimeAsync(10_000);
  // A fresh module simulates losing all in-memory guards during a full page reload.
  vi.resetModules();
  const { startSession: reload } = await import('../src/session.ts');
  const stopReload = reload(options);
  await vi.advanceTimersByTimeAsync(0);
  expect(changes.at(-1)?.phase).toBe('waiting');
  expect(createWorker).toHaveBeenCalledOnce();
  await vi.advanceTimersByTimeAsync(cooldownMs - 10_001);
  expect(createWorker).toHaveBeenCalledOnce();
  await vi.advanceTimersByTimeAsync(1);
  expect(createWorker).toHaveBeenCalledTimes(2);
  stopReload();
});

it('does not poll after completing a check, and a rapid reload reuses the saved result', async () => {
  const { startSession, options, worker, snapshot, createWorker, cooldownMs, changes, saved } = await setup({
    withSnapshot: false,
  });
  const stop = startSession(options);
  await vi.advanceTimersByTimeAsync(0);
  worker.send({
    type: 'complete',
    state: snapshot.state,
    model: snapshot.model,
    cache: { [snapshotKey('nfl')]: JSON.stringify(snapshot) },
  });
  stop();
  expect((await saved())[snapshotKey('nfl')]).toBe(JSON.stringify(snapshot));
  const reloadStart = changes.length;
  const stopReload = startSession(options);
  await vi.advanceTimersByTimeAsync(0);
  expect(changes[reloadStart]).toMatchObject({ model: snapshot.model, state: { ...snapshot.state, cached: true } });
  expect(changes.at(-1)?.phase).toBe('waiting');
  expect(createWorker).toHaveBeenCalledOnce();
  await vi.advanceTimersByTimeAsync(cooldownMs);
  worker.send({ type: 'complete', state: snapshot.state, model: snapshot.model, cache: {} });
  await vi.advanceTimersByTimeAsync(cooldownMs * 3);
  expect(createWorker).toHaveBeenCalledTimes(2);
  stopReload();
});

it('rechecks the cache and cooldown after another tab releases the lock', async () => {
  const { startSession, options, worker, snapshot, createWorker, changes } = await setup();
  let tail = Promise.resolve();
  const locks = {
    request: vi.fn((_name: string, _options: LockOptions, callback: () => Promise<void>) => {
      tail = tail.then(callback);
      return tail;
    }),
  } as unknown as Pick<LockManager, 'request'>;
  const stopFirst = startSession({ ...options, locks });
  await vi.advanceTimersByTimeAsync(0);
  const secondChanges: SessionView[] = [];
  const stopSecond = startSession({ ...options, locks, onChange: (view) => secondChanges.push(view) });
  await vi.advanceTimersByTimeAsync(0);
  expect(createWorker).toHaveBeenCalledOnce();
  const updated = { ...snapshot, state: { ...snapshot.state, refreshed_at: '2026-09-19T18:00:01.000Z' } };
  worker.send({
    type: 'complete',
    state: updated.state,
    model: updated.model,
    cache: { [snapshotKey('nfl')]: JSON.stringify(updated) },
  });
  await tail;
  expect(secondChanges.at(-1)?.state?.refreshed_at).toBe(updated.state.refreshed_at);
  expect(secondChanges.at(-1)?.phase).toBe('waiting');
  expect(changes.at(-1)?.phase).toBe('idle');
  expect(createWorker).toHaveBeenCalledOnce();
  stopFirst();
  stopSecond();
});

it('ignores stale worker messages after unmount and preserves saved results on worker failure', async () => {
  const { startSession, options, worker, saved, changes, snapshot } = await setup();
  const stop = startSession(options);
  await vi.advanceTimersByTimeAsync(0);
  worker.send({ type: 'error', error: 'offline' });
  expect(changes.at(-1)).toMatchObject({
    phase: 'idle',
    model: snapshot.model,
    state: { status: 'ready', cached: true, warning: 'Update failed. offline' },
  });
  stop();
  const count = changes.length;
  const before = await saved();
  worker.send({ type: 'complete', state: snapshot.state, model: snapshot.model, cache: { [snapshotKey('nfl')]: 'broken' } });
  await vi.advanceTimersByTimeAsync(0);
  expect(changes).toHaveLength(count);
  expect(await saved()).toEqual(before);
  expect(readSnapshot(storeOf(await saved()), 'nfl')).not.toBeNull();
});

it('does not record a refresh during a development-mode mount immediately followed by cleanup', async () => {
  const { startSession, options, createWorker, store, attemptKey } = await setup();
  startSession(options)();
  await vi.advanceTimersByTimeAsync(0);
  expect(store.getItem(attemptKey('nfl'))).toBeNull();
  expect(createWorker).not.toHaveBeenCalled();
  const stop = startSession(options);
  await vi.advanceTimersByTimeAsync(0);
  expect(createWorker).toHaveBeenCalledOnce();
  stop();
});
