import { readFileSync } from 'node:fs';
import { configSchema, type EloSeed, type Game } from '../src/contracts.ts';
import { digest } from '../src/storage.ts';

export const configBytes = readFileSync(new URL('../../../config/nfl.json', import.meta.url));
export const config = configSchema.parse(JSON.parse(configBytes.toString()));
// Hashing is asynchronous in the browser, so the fixtures resolve it once at module load.
export const configHash = await digest(configBytes);
export const historyHash = await digest('history');
/** The published `history.sha256`, as the site build writes it. */
export const historyHashFile = `${historyHash}
`;

export function seed(): EloSeed {
  return {
    schema_version: 1,
    league: 'nfl',
    target_season: 2026,
    through_season: 2025,
    generated_at: '2026-09-01T00:00:00Z',
    history_sha256: historyHash,
    config_sha256: configHash,
    settings: config.elo,
    completed_games: 100,
    tied_games: 1,
    tie_weight: 0.02,
    ratings: config.teams.map((t) => ({ team: t.id, elo: config.elo.initial, games: 1 })),
  };
}
export function game(overrides: Partial<Game> = {}): Game {
  return {
    id: 'g1',
    league: 'nfl',
    season: 2026,
    date: '2026-09-01',
    time: '13:00',
    timezone: 'America/New_York',
    phase: 'regular',
    round_label: 'REG',
    round: 1,
    home_team: 'SEA',
    away_team: 'SF',
    home_source_id: overrides.home_team ?? 'SEA',
    away_source_id: overrides.away_team ?? 'SF',
    neutral: true,
    result: 'home_win',
    ...overrides,
  };
}
