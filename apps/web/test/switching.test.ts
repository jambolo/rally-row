import { readFileSync } from 'node:fs';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { configSchema, type EloSeed } from '../src/contracts.ts';
import { indexedDbPersistence } from '../src/persistence.ts';
import * as postseason from '../src/postseason.ts';
import { refresh, type RefreshMessage, type RefreshRequest } from '../src/refresh-worker.ts';
import { hashChangeTarget, recordSwitch, startupLeague } from '../src/selection.ts';
import type { SessionView } from '../src/session.ts';
import { rememberedLeagueKey } from '../src/small-store.ts';
import { readSnapshot, snapshotKey } from '../src/snapshot.ts';
import { currentCacheKey, digest, memoryStore } from '../src/storage.ts';
import { fakeIndexedDb } from './fake-indexeddb.ts';
import { config as nflConfig, configBytes as nflConfigBytes, historyBytes as nflHistoryBytes, seed as nflSeed } from './helpers.ts';

// refresh() always runs the production simulation count; these multi-refresh tests only need the odds to exist.
const simulatePostseason = postseason.simulatePostseason;
vi.spyOn(postseason, 'simulatePostseason').mockImplementation((model, games) =>
  simulatePostseason(model, games, { simulations: 200 }),
);

const ids = ['nfl', 'mlb'];
const configBase = 'https://site.test/config';
const dataBase = 'https://site.test/data';
const mlbConfigBytes = readFileSync(new URL('../../../config/mlb.json', import.meta.url));
const mlbConfig = configSchema.parse(JSON.parse(mlbConfigBytes.toString()));
const mlbHistoryBytes = 'mlb history';
const mlbSeed: EloSeed = {
  schema_version: 1,
  league: 'mlb',
  target_season: 2026,
  through_season: 2025,
  generated_at: '2026-01-15T00:00:00Z',
  history_sha256: await digest(mlbHistoryBytes),
  config_sha256: await digest(mlbConfigBytes),
  settings: mlbConfig.elo,
  completed_games: 2400,
  tied_games: 0,
  tie_weight: 0.0003,
  ratings: mlbConfig.teams.map((t) => ({ team: t.id, elo: mlbConfig.elo.initial, games: 162 })),
};
const nflSchedule =
  'game_id,season,game_type,week,gameday,gametime,away_team,away_score,home_team,home_score,location\ng1,2026,REG,1,2026-09-01,13:00,SF,10,SEA,20,Home\ng2,2026,REG,2,2026-09-20,13:00,KC,,SEA,,Home\n';
// BOS 111 beat NYY 147 on September 18; TB 139 hosts TOR 141 on September 25.
const mlbSchedule = JSON.stringify({
  dates: [
    {
      date: '2026-09-18',
      games: [
        {
          gamePk: 1,
          gameType: 'R',
          season: '2026',
          gameDate: '2026-09-18T23:10:00Z',
          officialDate: '2026-09-18',
          status: { detailedState: 'Final' },
          teams: { home: { team: { id: 111 }, score: 5 }, away: { team: { id: 147 }, score: 3 } },
          isTie: false,
        },
        {
          gamePk: 2,
          gameType: 'R',
          season: '2026',
          gameDate: '2026-09-25T23:05:00Z',
          officialDate: '2026-09-25',
          status: { detailedState: 'Scheduled' },
          teams: { home: { team: { id: 139 } }, away: { team: { id: 141 } } },
        },
      ],
    },
  ],
});
const stops: (() => void)[] = [];

beforeEach(() => {
  vi.resetModules();
  vi.useFakeTimers({ now: new Date('2026-09-19T18:00:00Z') });
  const files = new Map([
    [`${configBase}/nfl.json`, nflConfigBytes.toString()],
    [`${dataBase}/nfl/elo-2026.json`, JSON.stringify(nflSeed())],
    [`${dataBase}/nfl/history.json`, nflHistoryBytes],
    [nflConfig.source.url, nflSchedule],
    [`${configBase}/mlb.json`, mlbConfigBytes.toString()],
    [`${dataBase}/mlb/elo-2026.json`, JSON.stringify(mlbSeed)],
    [`${dataBase}/mlb/history.json`, mlbHistoryBytes],
    [mlbConfig.source.url.replaceAll('{season}', '2026'), mlbSchedule],
  ]);
  vi.stubGlobal('fetch', async (url: string) => {
    const body = files.get(String(url));
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

it("switching updates the hash and the remembered league, and both leagues' saved snapshots remain in persistence after each session completes", async () => {
  const { startSession } = await import('../src/session.ts');
  const store = memoryStore();
  const location = { hash: '' };
  const idb = fakeIndexedDb();
  const persistence = indexedDbPersistence(idb.factory);
  const views: Record<string, SessionView[]> = { nfl: [], mlb: [] };
  const requests: RefreshRequest[] = [];
  const jobs: Promise<void>[] = [];
  // The worker runs `refresh` in-process.
  const createWorker = () => {
    const worker = {
      onmessage: null as ((event: MessageEvent<RefreshMessage>) => void) | null,
      terminate: vi.fn(),
      postMessage(request: RefreshRequest) {
        requests.push(structuredClone(request));
        jobs.push(refresh(structuredClone(request), (data) => worker.onmessage?.({ data: structuredClone(data) } as MessageEvent)));
      },
    };
    return worker as unknown as Worker;
  };
  // What the app does for its current league: stop the previous session, start this league's.
  let stop = () => {};
  const show = (league: string) => {
    stop();
    stop = startSession({
      league,
      configBase,
      dataBase,
      store,
      persistence,
      createWorker,
      locks: null,
      onChange: (view) => views[league].push(view),
    });
  };
  stops.push(() => stop());
  const switchTo = (league: string) => {
    recordSwitch(league, { location, store });
    show(league);
  };
  const settle = async () => {
    await vi.advanceTimersByTimeAsync(0);
    await Promise.all(jobs);
    await vi.advanceTimersByTimeAsync(0);
  };
  const savedKeys = async () => Object.keys(idb.contents()).sort();
  const both = [currentCacheKey('mlb', 2026), currentCacheKey('nfl', 2026), snapshotKey('mlb'), snapshotKey('nfl')].sort();
  const loadWindows = async () => [
    { id: 'nfl', windows: nflConfig.windows },
    { id: 'mlb', windows: mlbConfig.windows },
  ];

  // Startup with no hash and nothing remembered: the default rule picks nfl on December 1.
  const startup = await startupLeague({ ids, hash: location.hash, store, today: '12-01', loadWindows });
  expect(startup).toBe('nfl');
  show(startup);
  await settle();
  expect(location.hash).toBe('');
  expect(store.getItem(rememberedLeagueKey)).toBeNull();
  expect(views.nfl.at(-1)).toMatchObject({ phase: 'idle', error: '', state: { status: 'ready', league: 'NFL', cached: false } });
  expect(await savedKeys()).toEqual([currentCacheKey('nfl', 2026), snapshotKey('nfl')]);
  const nflSaved = await persistence.load('nfl');

  // The switcher selects mlb.
  switchTo('mlb');
  expect(location.hash).toBe('#mlb');
  expect(store.getItem(rememberedLeagueKey)).toBe('mlb');
  await settle();
  expect(views.mlb.at(-1)).toMatchObject({ phase: 'idle', error: '', state: { status: 'ready', league: 'MLB', cached: false } });
  expect(requests.map((r) => [r.league, Object.keys(r.cache).length])).toEqual([
    ['nfl', 0],
    ['mlb', 0],
  ]);
  expect(await savedKeys()).toEqual(both);
  expect(await persistence.load('nfl')).toEqual(nflSaved);
  const mlbSaved = await persistence.load('mlb');
  expect(readSnapshot(storeOf(mlbSaved), 'mlb')?.model).toEqual(views.mlb.at(-1)?.model);

  // Back to nfl through the location hash (the Back button): the hash already matches; the league is remembered.
  location.hash = '#nfl';
  const target = hashChangeTarget(location.hash, 'mlb', ids);
  expect(target).toBe('nfl');
  const shown = views.nfl.length;
  switchTo(target!);
  expect(location.hash).toBe('#nfl');
  expect(store.getItem(rememberedLeagueKey)).toBe('nfl');
  await settle();
  // nfl shows its saved model at once, then waits out its cooldown before refreshing.
  expect(views.nfl[shown]).toMatchObject({ state: { status: 'ready', league: 'NFL', cached: true } });
  expect(views.nfl[shown]?.model).toEqual(readSnapshot(storeOf(nflSaved), 'nfl')?.model);
  expect(views.nfl.at(-1)).toMatchObject({ phase: 'waiting', state: { status: 'ready', league: 'NFL', cached: true } });
  await vi.advanceTimersByTimeAsync(60_000);
  await settle();
  expect(views.nfl.at(-1)).toMatchObject({ phase: 'idle', error: '', state: { status: 'ready', league: 'NFL', cached: false } });
  expect(requests.map((r) => [r.league, Object.keys(r.cache).sort()])).toEqual([
    ['nfl', []],
    ['mlb', []],
    ['nfl', [currentCacheKey('nfl', 2026), snapshotKey('nfl')]],
  ]);
  expect(await savedKeys()).toEqual(both);
  expect(await persistence.load('mlb')).toEqual(mlbSaved);

  // A reload keeps the league the reader switched to, although the default rule picks mlb on June 15.
  expect(await startupLeague({ ids, hash: '', store: null, today: '06-15', loadWindows })).toBe('mlb');
  expect(await startupLeague({ ids, hash: location.hash, store, today: '06-15', loadWindows })).toBe('nfl');
  expect(await startupLeague({ ids, hash: '', store, today: '06-15', loadWindows })).toBe('nfl');
});
