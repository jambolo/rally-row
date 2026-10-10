import { afterEach, expect, it, vi } from 'vitest';
import * as model from '../src/model.ts';
import { PredictionService } from '../src/service.ts';
import { readSnapshot, snapshotKey } from '../src/snapshot.ts';
import { memoryStore, type Store } from '../src/storage.ts';
import type { LeagueConfig } from '../src/contracts.ts';
import { config, configHash, game, historyHashFile, seed } from './helpers.ts';

const dataBase = 'https://published.test/data';
const now = () => new Date('2026-09-19T18:00:00Z');
const csv =
  'game_id,season,game_type,week,gameday,gametime,away_team,away_score,home_team,home_score,location\ng1,2026,REG,1,2026-09-01,13:00,SF,10,SEA,20,Home\ng2,2026,REG,2,2026-09-20,13:00,KC,,SEA,,Home\n';
const canonicalConfig: LeagueConfig = { ...config, source: { ...config.source, kind: 'canonical-json' } };
const envelope = JSON.stringify({ schema_version: 2, league: 'nfl', games: [game()] });
const nflverse = {
  summary: "Today's results enter the picture the next day, Eastern Time.",
  detail: 'Results from today are held until the next calendar day in Eastern Time because the source has no live/final flag.',
};
const canonical = {
  summary: 'Results enter the picture as soon as the provider reports them.',
  detail: 'The provider supplies completed game outcomes.',
};

function publish() {
  const published: Record<string, string> = {
    [`${dataBase}/nfl/history.sha256`]: historyHashFile,
    [`${dataBase}/nfl/elo-2026.json`]: JSON.stringify(seed()),
  };
  vi.stubGlobal('fetch', (input: string | URL | Request) => {
    const body = published[String(input)];
    return Promise.resolve(body === undefined ? new Response('missing', { status: 404 }) : new Response(body, { status: 200 }));
  });
}
const service = (store: Store, leagueConfig: LeagueConfig, fetchSource: (url: string) => Promise<string>) =>
  new PredictionService({
    config: leagueConfig,
    configHash,
    dataBase,
    season: 2026,
    postseasonSimulations: 200,
    now,
    store,
    fetchSource,
  });

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

it.each([
  ['nflverse-csv', config, csv, nflverse],
  ['canonical-json', canonicalConfig, envelope, canonical],
] as const)('publishes the %s result policy in state and snapshot', async (_kind, leagueConfig, body, policy) => {
  publish();
  const store = memoryStore();
  const s = service(store, leagueConfig, async () => body);
  await s.initialize();
  expect(s.getState()).toMatchObject({ status: 'ready', result_policy: policy.detail, result_policy_summary: policy.summary });
  expect(readSnapshot(store, 'nfl')?.state).toMatchObject({ result_policy: policy.detail, result_policy_summary: policy.summary });
});

it('downloads each season URL from the source adapter through fetchSource', async () => {
  publish();
  const fetchSource = vi.fn(async () => csv);
  await service(memoryStore(), config, fetchSource).initialize();
  expect(fetchSource.mock.calls).toEqual([[config.source.url]]);
});

it('refreshes a stale result_policy_summary when reusing an unchanged model', async () => {
  publish();
  const store = memoryStore();
  const first = service(store, config, async () => csv);
  await first.initialize();
  const saved = JSON.parse(store.getItem(snapshotKey('nfl'))!);
  saved.state.result_policy_summary = 'Old summary';
  store.setItem(snapshotKey('nfl'), JSON.stringify(saved));
  const fitted = vi.spyOn(model, 'fitPosterior');
  const second = service(store, config, async () => csv);
  await second.initialize();
  expect(fitted).not.toHaveBeenCalled();
  expect(second.getState().result_policy_summary).toBe(nflverse.summary);
  expect(readSnapshot(store, 'nfl')?.state.result_policy_summary).toBe(nflverse.summary);
});

it('discards a snapshot saved without result_policy_summary', async () => {
  publish();
  const store = memoryStore();
  await service(store, config, async () => csv).initialize();
  const saved = JSON.parse(store.getItem(snapshotKey('nfl'))!);
  delete saved.state.result_policy_summary;
  store.setItem(snapshotKey('nfl'), JSON.stringify(saved));
  expect(readSnapshot(store, 'nfl')).toBeNull();
});
