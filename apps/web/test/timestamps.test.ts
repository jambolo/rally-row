import { expect, it } from 'vitest';
import { validateGames } from '../src/contracts.ts';
import { config, game } from './helpers.ts';

it.each([
  ['2002-09-01', '13:00', 'America/New_York', '2002-09-01T17:00:00Z'],
  ['2002-01-01', '13:00', 'America/New_York', '2002-01-01T18:00:00Z'],
  ['2002-09-01', '23:30', 'America/Los_Angeles', '2002-09-02T06:30:00Z'],
  ['2002-09-02', '08:00', 'Asia/Tokyo', '2002-09-01T23:00:00Z'],
  ['2002-09-01', null, 'America/New_York', '2002-09-01T04:00:00Z'],
  ['2002-09-01', '', 'America/New_York', '2002-09-01T04:00:00Z'],
  ['2002-09-01', '  ', 'America/New_York', '2002-09-01T04:00:00Z'],
  ['2002-10-27', '01:30', 'America/New_York', '2002-10-27T05:30:00Z'],
  ['2002-04-07', '02:30', 'America/New_York', '2002-04-07T05:00:00Z'],
] as const)('normalizes %s %s in %s consistently with Rust', (date, time, timezone, expected) => {
  const [g] = validateGames([game({ date, time, timezone })], config);
  expect(g.start_time_utc).toBe(expected);
  expect([g.date, g.time, g.timezone]).toEqual([date, time, timezone]);
});

it('sorts by season, UTC start time, then byte-ordered ID across timezones', () => {
  const games = validateGames(
    [
      game({ id: 'later', date: '2026-09-01', time: '23:30', timezone: 'America/Los_Angeles' }),
      game({ id: 'earlier', date: '2026-09-02', time: '08:00', timezone: 'Asia/Tokyo' }),
      game({ id: 'z', date: '2026-09-01', time: '09:00', timezone: 'America/New_York' }),
      game({ id: 'A', date: '2026-09-01', time: '14:00', timezone: 'Europe/London' }),
      game({ id: 'prior-season', season: 2025 }),
    ],
    config,
  );
  expect(games.map((g) => g.id)).toEqual(['prior-season', 'A', 'z', 'earlier', 'later']);
  games.find((g) => g.id === 'earlier')!.date = '2026-09-03';
  expect(validateGames(games, config).at(-1)!.id).toBe('earlier');
});

it('rejects missing dates, unknown zones, and dates without local midnight', () => {
  expect(() => validateGames([{ ...game(), date: undefined }], config)).toThrow();
  expect(() => validateGames([game({ timezone: 'Mars/Olympus' })], config)).toThrow();
  expect(() => validateGames([game({ date: '2011-12-30', time: null, timezone: 'Pacific/Apia' })], config)).toThrow(/midnight/);
  expect(validateGames([{ ...game(), time: undefined }], config)[0].start_time_utc).toBe('2026-09-01T04:00:00Z');
});
