import { readFileSync } from 'node:fs';
import { expect, it } from 'vitest';
import { configSchema, monthDayOrdinal, teamIdentity, windowsSchema, type LeagueConfig } from '../src/contracts.ts';
import { configBytes } from './helpers.ts';

const rawConfig = () => JSON.parse(configBytes.toString()) as Record<string, unknown>;

const windows = (season: [string, string], postseason: [string, string]) => ({
  season: { start: season[0], end: season[1] },
  postseason: { start: postseason[0], end: postseason[1] },
});

const messages = (value: unknown) => windowsSchema.safeParse(value).error?.issues.map((i) => i.message);

it('accepts the NFL config with year-wrapping windows', () => {
  expect(configSchema.safeParse(rawConfig()).success).toBe(true);
});

it('accepts non-wrapping windows', () => {
  expect(messages(windows(['03-20', '11-05'], ['09-30', '11-05']))).toBeUndefined();
});

it('computes non-leap day-of-year ordinals', () => {
  expect(['01-01', '02-15', '02-28', '09-01', '12-31', '02-29'].map(monthDayOrdinal)).toEqual([1, 46, 59, 244, 365, null]);
});

it.each(['02-29', '13-01', '04-31', '00-10', '01-00', '9-01', '09-1'])('rejects invalid month-day %s', (v) => {
  expect(messages(windows([v, '02-15'], ['01-08', '02-15']))).toEqual(['Invalid month-day']);
});

it('rejects a postseason window outside the season window', () => {
  for (const post of [
    ['01-08', '02-20'],
    ['08-15', '02-15'],
    ['02-15', '01-08'],
  ] as [string, string][]) {
    expect(messages(windows(['09-01', '02-15'], post))).toEqual(['Postseason window must lie within the season window']);
  }
});

it('rejects equal season start and end', () => {
  expect(messages(windows(['09-01', '09-01'], ['09-01', '09-01']))).toEqual(['Season window start and end must be different']);
});

it('rejects schema_version 1', () => {
  expect(configSchema.safeParse({ ...rawConfig(), schema_version: 1 }).success).toBe(false);
});

it('rejects an unknown schedule_filter unit', () => {
  const raw = rawConfig() as { display: { schedule_filter: { unit: string } } };
  raw.display.schedule_filter.unit = 'week';
  expect(configSchema.safeParse(raw).success).toBe(false);
});

it('rejects an unknown source kind', () => {
  const raw = { ...rawConfig(), source: { kind: 'bogus', url: 'https://example.com/games.json' } };
  expect(configSchema.safeParse(raw).success).toBe(false);
});

it('accepts a null round_name', () => {
  const raw = rawConfig() as { display: { round_name: string | null } };
  raw.display.round_name = null;
  expect(configSchema.safeParse(raw).success).toBe(true);
});

const mlbBytes = readFileSync(new URL('../../../config/mlb.json', import.meta.url));
const nfl = (): LeagueConfig => JSON.parse(configBytes.toString());
const mlb = (): LeagueConfig => JSON.parse(mlbBytes.toString());
const team = (raw: LeagueConfig, id: string) => raw.teams.find((t) => t.id === id)!;
const round = (label: string) => ({ name: label, short: label, round_label: label, pattern: 'H', home: 'higher_seed' as const });
const firstMessage = (raw: LeagueConfig) => configSchema.safeParse(raw).error?.issues[0]?.message;

const rejections: [string, (r: LeagueConfig) => void][] = [
  ['ARI: Team division needs a postseason format', (r) => delete r.postseason],
  ['Postseason format requires postseason ties to be disallowed', (r) => (r.ties_allowed_in = ['regular', 'postseason'])],
  [
    'Conference count must be a power of two',
    (r) => r.postseason!.conferences.push({ id: 'X', name: 'X', divisions: [{ id: 'X-1', name: 'X 1' }] }),
  ],
  ['Duplicate conference or division id', (r) => (r.postseason!.conferences[1].divisions[0].id = 'AFC-E')],
  ['Invalid playoff field size', (r) => (r.postseason!.teams_per_conference = 1)],
  ['More divisions than playoff spots', (r) => (r.postseason!.teams_per_conference = 3)],
  ['Round count does not match the bracket', (r) => r.postseason!.rounds.pop()],
  ['Duplicate round label', (r) => (r.postseason!.rounds[1].round_label = 'WC')],
  ['Invalid series pattern', (r) => (r.postseason!.rounds[0].pattern = 'HH')],
  [
    'Rounds between conferences cannot give home advantage by seed',
    (r) => Object.assign(r.postseason!.rounds[3], { pattern: 'H', home: 'higher_seed' }),
  ],
  ['min_games applies only to common_games', (r) => (r.postseason!.tiebreakers.division[0].min_games = 2)],
  ['min_games must be positive', (r) => (r.postseason!.tiebreakers.conference[2].min_games = 0)],
  ['Duplicate tiebreaker', (r) => r.postseason!.tiebreakers.division.push({ rule: 'head_to_head' })],
  ['ARI: Unknown division', (r) => (team(r, 'ARI').eras[0].division = 'NOPE')],
  ['ARI: Current division missing', (r) => delete team(r, 'ARI').eras[0].division],
  ['AFC-X: Division has no current teams', (r) => r.postseason!.conferences[0].divisions.push({ id: 'AFC-X', name: 'AFC X' })],
  [
    'AFC: Conference has fewer teams than playoff spots',
    (r) => {
      r.postseason!.teams_per_conference = 17;
      r.postseason!.rounds.unshift(round('R1'), round('R2'));
    },
  ],
];

it.each(rejections)('rejects: %s', (message, mutate) => {
  const raw = nfl();
  mutate(raw);
  expect(firstMessage(raw)).toBe(message);
});

const alignment = (c: LeagueConfig) =>
  c.postseason!.conferences.map((conf) =>
    conf.divisions.map((d) => c.teams.filter((t) => t.eras.at(-1)!.division === d.id).length),
  );

it('accepts the MLB config', () => {
  expect(configSchema.safeParse(mlb()).success).toBe(true);
});

it('accepts older eras without a division', () => {
  const raw = nfl();
  delete team(raw, 'LV').eras[0].division;
  expect(configSchema.safeParse(raw).success).toBe(true);
});

it('accepts a config without postseason or divisions', () => {
  const raw = nfl();
  delete raw.postseason;
  for (const t of raw.teams) for (const era of t.eras) delete era.division;
  expect(configSchema.safeParse(raw).success).toBe(true);
});

it('aligns NFL current teams 2 x 4 x 4', () => {
  expect(alignment(configSchema.parse(nfl()))).toEqual([
    [4, 4, 4, 4],
    [4, 4, 4, 4],
  ]);
});

it('aligns MLB current teams 2 x 3 x 5', () => {
  expect(alignment(configSchema.parse(mlb()))).toEqual([
    [5, 5, 5],
    [5, 5, 5],
  ]);
});

it('moves HOU from NL-C to AL-W in 2013', () => {
  const cfg = configSchema.parse(mlb());
  expect(teamIdentity(cfg, 'HOU', 2012).division).toBe('NL-C');
  expect(teamIdentity(cfg, 'HOU', 2013).division).toBe('AL-W');
});

it('rejects an empty era division', () => {
  const raw = nfl();
  team(raw, 'ARI').eras[0].division = '';
  expect(configSchema.safeParse(raw).error?.issues[0]?.path).toEqual([
    'teams',
    raw.teams.findIndex((t) => t.id === 'ARI'),
    'eras',
    0,
    'division',
  ]);
});

it('parses postseason between windows and elo', () => {
  const keys = Object.keys(configSchema.parse(nfl()));
  expect(keys.slice(keys.indexOf('windows'), keys.indexOf('elo') + 1)).toEqual(['windows', 'postseason', 'elo']);
});
