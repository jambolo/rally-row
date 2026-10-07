import { afterEach, expect, it, vi } from 'vitest';
import * as model from '../src/model.ts';
import { PredictionService } from '../src/service.ts';
import { readSnapshot, snapshotKey } from '../src/snapshot.ts';
import { memoryStore, type Store } from '../src/storage.ts';
import { config, configHash, historyBytes, seed } from './helpers.ts';

const dataBase = 'https://published.test/data';
const now = () => new Date('2026-09-19T18:00:00Z');
const csv =
  'game_id,season,game_type,week,gameday,gametime,away_team,away_score,home_team,home_score,location\ng1,2026,REG,1,2026-09-01,13:00,SF,10,SEA,20,Home\ng2,2026,REG,2,2026-09-20,13:00,KC,,SEA,,Home\n';

function publish() {
  const published: Record<string, string> = {
    [`${dataBase}/nfl/history.json`]: historyBytes,
    [`${dataBase}/nfl/elo-2026.json`]: JSON.stringify(seed()),
  };
  vi.stubGlobal('fetch', (input: string | URL | Request) => {
    const body = published[String(input)];
    return Promise.resolve(body === undefined ? new Response('missing', { status: 404 }) : new Response(body, { status: 200 }));
  });
}
const service = (store: Store) =>
  new PredictionService({
    config,
    configHash,
    dataBase,
    season: 2026,
    postseasonSimulations: 200,
    now,
    store,
    fetchSource: async () => csv,
  });

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

it('publishes the display vocabulary and tie rules in state and snapshot', async () => {
  publish();
  const store = memoryStore();
  const s = service(store);
  expect(s.getState()).toMatchObject({ display: config.display, ties_allowed_in: ['regular'] });
  await s.initialize();
  expect(s.getState()).toMatchObject({ status: 'ready', display: config.display, ties_allowed_in: ['regular'] });
  expect(readSnapshot(store, 'nfl')?.state).toMatchObject({ display: config.display, ties_allowed_in: ['regular'] });
});

it('refreshes a stale display vocabulary when reusing an unchanged model', async () => {
  publish();
  const store = memoryStore();
  await service(store).initialize();
  const saved = JSON.parse(store.getItem(snapshotKey('nfl'))!);
  saved.state.display = { ...saved.state.display, start_time_label: 'Old label' };
  saved.state.ties_allowed_in = ['regular', 'postseason'];
  store.setItem(snapshotKey('nfl'), JSON.stringify(saved));
  const fitted = vi.spyOn(model, 'fitPosterior');
  const second = service(store);
  await second.initialize();
  expect(fitted).not.toHaveBeenCalled();
  expect(second.getState()).toMatchObject({ display: config.display, ties_allowed_in: ['regular'] });
  expect(readSnapshot(store, 'nfl')?.state).toMatchObject({ display: config.display, ties_allowed_in: ['regular'] });
});

it.each(['display', 'ties_allowed_in', 'postseason'])('discards a snapshot saved without %s', async (field) => {
  publish();
  const store = memoryStore();
  await service(store).initialize();
  const saved = JSON.parse(store.getItem(snapshotKey('nfl'))!);
  delete saved.state[field];
  store.setItem(snapshotKey('nfl'), JSON.stringify(saved));
  expect(readSnapshot(store, 'nfl')).toBeNull();
});
