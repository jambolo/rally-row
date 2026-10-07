import { resolve } from 'node:path';
import { createContext, runInContext } from 'node:vm';
import { build } from 'vite';
import { afterEach, expect, it, vi } from 'vitest';
import type { RefreshMessage } from '../src/refresh-worker.ts';
import { snapshotKey } from '../src/snapshot.ts';
import { version as appVersion } from '../package.json';
import { config, configBytes, historyBytes, seed } from './helpers.ts';

afterEach(() => vi.useRealTimers());

it('builds, reuses, and upgrades snapshots through the bundled worker without window, localStorage, or Node globals', async () => {
  vi.useFakeTimers({ toFake: ['Date'], now: new Date('2026-09-19T18:00:00Z') });
  const bundle = await build({
    configFile: false,
    logLevel: 'silent',
    // The bundle cannot take the service option, and this test checks worker plumbing, not odds precision.
    plugins: [
      {
        name: 'fewer-postseason-simulations',
        enforce: 'pre',
        transform(code, id) {
          if (!id.endsWith('/src/postseason.ts')) return null;
          const lowered = code.replace('POSTSEASON_SIMULATIONS = 10_000', 'POSTSEASON_SIMULATIONS = 200');
          if (lowered === code) throw new Error('POSTSEASON_SIMULATIONS = 10_000 not found');
          return lowered;
        },
      },
    ],
    build: {
      write: false,
      minify: false,
      lib: { entry: resolve(import.meta.dirname, '../src/refresh.worker.ts'), formats: ['iife'], name: 'refreshWorker' },
    },
  });
  const output = Array.isArray(bundle) ? bundle[0] : bundle;
  if (!('output' in output)) throw new Error('Expected a browser bundle');
  const chunk = output.output.find((file) => file.type === 'chunk')!;
  const messages: RefreshMessage[] = [];
  const requests: { url: string; cache: RequestCache | undefined }[] = [];
  let complete!: () => void;
  let seedAvailable = true;
  const context = createContext({
    self: {
      postMessage: (message: RefreshMessage) => {
        messages.push(message);
        if (message.type !== 'progress') complete();
      },
    },
    request: { league: 'nfl', configUrl: 'https://test/config/nfl.json', dataBase: 'https://test/data', cache: {} },
    TextEncoder,
    TextDecoder,
    crypto,
    AbortSignal,
    setTimeout,
    URL,
    Date,
    fetch: async (url: string, init?: RequestInit) => {
      requests.push({ url, cache: init?.cache });
      if (url.endsWith('/config/nfl.json')) return new Response(configBytes);
      if (url.endsWith('/history.json')) return new Response(historyBytes);
      if (url.endsWith('/elo-2026.json'))
        return seedAvailable ? new Response(JSON.stringify(seed())) : new Response('missing', { status: 404 });
      if (url === config.source.url)
        return new Response(
          'game_id,season,game_type,week,gameday,gametime,away_team,away_score,home_team,home_score,location\ng1,2026,REG,1,2026-09-01,13:00,SF,10,SEA,20,Home\n',
        );
      throw new Error(`Unexpected URL ${url}`);
    },
  });
  runInContext(chunk.code, context);
  const run = async (cache: Record<string, string>) => {
    messages.length = 0;
    context.request.cache = cache;
    const done = new Promise<void>((resolve) => {
      complete = resolve;
    });
    runInContext('self.onmessage({ data: request })', context);
    await done;
    const result = messages.at(-1)!;
    if (result.type !== 'complete') throw new Error(result.type === 'error' ? result.error : 'Worker did not complete');
    return result;
  };
  const result = await run({});
  expect(messages.at(-1)).toMatchObject({
    type: 'complete',
    state: { status: 'ready', training_games: 1, postseason: { status: 'ready', simulations: 200 } },
    model: { games_used: 1 },
  });
  expect(messages).toContainEqual({ type: 'progress', phase: 'building' });
  expect(requests.find((r) => r.url === config.source.url)?.cache).toBe('no-cache');
  expect(requests.find((r) => r.url.endsWith('/elo-2026.json'))?.cache).toBe('no-cache');
  expect(requests.find((r) => r.url.endsWith('/history.json'))?.cache).toBe('no-cache');
  expect(JSON.parse(result.cache[snapshotKey('nfl')]).app_version).toBe(appVersion);

  const reused = await run(result.cache);
  expect(messages.filter((message) => message.type === 'progress')).toEqual([{ type: 'progress', phase: 'checking' }]);
  expect(reused.model).toEqual(result.model);

  const legacy = JSON.parse(result.cache[snapshotKey('nfl')]);
  legacy.app_version = '0.0.0';
  const oldCache = { ...result.cache, [snapshotKey('nfl')]: JSON.stringify(legacy) };
  seedAvailable = false;
  const failed = await run(oldCache);
  expect(failed).toMatchObject({ model: null, state: { status: 'error', error: expect.stringContaining('Not published') } });
  expect(failed.cache).toEqual(oldCache);

  seedAvailable = true;
  const upgraded = await run(failed.cache);
  expect(messages).toContainEqual({ type: 'progress', phase: 'building' });
  expect(upgraded).toMatchObject({ state: { status: 'ready', warning: null }, model: { games_used: 1 } });
  expect(JSON.parse(upgraded.cache[snapshotKey('nfl')]).app_version).toBe(appVersion);
});
