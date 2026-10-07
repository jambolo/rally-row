import { readFileSync } from 'node:fs';
import { it, expect } from 'vitest';
import { configSchema, identityRuns, teamIdentity } from '../src/contracts.ts';
import { parseSource } from '../src/provider.ts';
import { config, seed } from './helpers.ts';
import { fitPosterior, teamEstimates } from '../src/model.ts';

it('resolves identity by season without changing franchise ids', () => {
  expect(teamIdentity(config, 'LV', 2019).name).toBe('Oakland Raiders');
  expect(teamIdentity(config, 'LV', 2020).name).toBe('Las Vegas Raiders');
  expect(teamIdentity(config, 'LAR', 2015).location).toBe('St. Louis');
  expect(teamIdentity(config, 'LAR', 2016).location).toBe('Los Angeles');
  expect(teamIdentity(config, 'LAC', 2016).name).toBe('San Diego Chargers');
  expect(teamIdentity(config, 'LAC', 2017).name).toBe('Los Angeles Chargers');
  expect(teamIdentity(config, 'WAS', 2019).name).toBe('Washington Redskins');
  expect(teamIdentity(config, 'WAS', 2020).name).toBe('Washington Football Team');
  expect(teamIdentity(config, 'WAS', 2021).name).toBe('Washington Football Team');
  expect(teamIdentity(config, 'WAS', 2022).name).toBe('Washington Commanders');
});
it('preserves original abbreviations and rejects an anachronistic source code', () => {
  const header = 'game_id,season,game_type,week,gameday,gametime,away_team,away_score,home_team,home_score,location\n';
  const raw = header + 'a,2019,REG,1,2019-09-01,13:00,SEA,10,OAK,20,Home\nb,2020,REG,1,2020-09-01,13:00,SEA,10,LV,20,Home\n';
  const games = parseSource(raw, config);
  expect(games.map((g) => g.home_team)).toEqual(['LV', 'LV']);
  expect(games.map((g) => g.home_source_id)).toEqual(['OAK', 'LV']);
  expect(() => parseSource(raw.replace('OAK', 'LV'), config)).toThrow(/invalid for season/);
});
it('rejects gaps and overlaps in historical identity ranges', () => {
  const cfg = structuredClone(config);
  cfg.teams.find((t) => t.id === 'LV')!.eras[0].through_season = 2020;
  expect(() => configSchema.parse(cfg)).toThrow(/overlap/);
  cfg.teams.find((t) => t.id === 'LV')!.eras[0].through_season = 2018;
  expect(() => configSchema.parse(cfg)).toThrow(/gap/);
});
it('uses historical display names when evaluating an old season', () => {
  const s = seed();
  s.target_season = 2019;
  s.through_season = 2018;
  const row = teamEstimates(fitPosterior(s, [], config)).find((t) => t.id === 'LV')!;
  expect(row.name).toBe('Oakland Raiders');
  expect(row.abbreviation).toBe('OAK');
});

const team = (id: string) => config.teams.find((t) => t.id === id)!;

it('merges HOU eras that differ only in division into one run', () => {
  const mlb = configSchema.parse(JSON.parse(readFileSync(new URL('../../../config/mlb.json', import.meta.url)).toString()));
  const hou = mlb.teams.find((t) => t.id === 'HOU')!;
  expect(identityRuns(hou)).toEqual([{ from_season: 1998, through_season: null, name: 'Houston Astros', location: 'Houston' }]);
});

it('merges eras that differ only in division', () => {
  const ari = structuredClone(team('ARI'));
  const era = ari.eras[0]!;
  ari.eras = [
    { ...era, through_season: 2010, division: 'D1' },
    { ...era, from_season: 2011, division: 'D2' },
  ];
  expect(identityRuns(ari)).toEqual([
    { from_season: era.from_season, through_season: null, name: era.name, location: era.location },
  ]);
});

it('keeps WAS renames as three runs', () => {
  expect(identityRuns(team('WAS')).map((r) => r.name)).toEqual([
    'Washington Redskins',
    'Washington Football Team',
    'Washington Commanders',
  ]);
});

it('keeps LV relocation as two runs', () => {
  expect(identityRuns(team('LV')).map((r) => [r.from_season, r.through_season, r.location])).toEqual([
    [2002, 2019, 'Oakland'],
    [2020, null, 'Las Vegas'],
  ]);
});
